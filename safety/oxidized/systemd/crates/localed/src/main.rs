//! systemd-localed — locale and keyboard layout management daemon
//!
//! Manages system-wide locale and keyboard configuration:
//! - **Locale**: stored in `/etc/locale.conf` as `LANG=`, `LC_*=` variables
//! - **Virtual console keymap**: stored in `/etc/vconsole.conf` as `KEYMAP=`, `KEYMAP_TOGGLE=`
//! - **X11 keyboard layout**: stored in `/etc/X11/xorg.conf.d/00-keyboard.conf`
//!   and also in `/etc/vconsole.conf` as `X11_LAYOUT=`, `X11_MODEL=`, `X11_VARIANT=`, `X11_OPTIONS=`
//!
//! The daemon listens for SIGTERM/SIGINT to shut down and SIGHUP to reload
//! configuration. It sends sd_notify READY/STATUS/STOPPING messages.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const LOCALE_CONF_PATH: &str = "/etc/locale.conf";
const VCONSOLE_CONF_PATH: &str = "/etc/vconsole.conf";
const X11_KEYBOARD_DIR: &str = "/etc/X11/xorg.conf.d";
const X11_KEYBOARD_CONF: &str = "/etc/X11/xorg.conf.d/00-keyboard.conf";
const CONTROL_SOCKET_PATH: &str = "/run/systemd/localed.sock";

/// Search paths for the kbd-model-map file (console keymap ↔ X11 mapping).
const KBD_MODEL_MAP_PATHS: &[&str] = &[
    "/etc/systemd/kbd-model-map",
    "/run/systemd/kbd-model-map",
    "/usr/share/systemd/kbd-model-map",
    "/usr/lib/systemd/kbd-model-map",
    // NixOS
    "/run/current-system/sw/share/systemd/kbd-model-map",
];

const DBUS_NAME: &str = "org.freedesktop.locale1";
const DBUS_PATH: &str = "/org/freedesktop/locale1";

/// Known locale variables that systemd-localed manages.
const LOCALE_VARIABLES: &[&str] = &[
    "LANG",
    "LANGUAGE",
    "LC_CTYPE",
    "LC_NUMERIC",
    "LC_TIME",
    "LC_COLLATE",
    "LC_MONETARY",
    "LC_MESSAGES",
    "LC_PAPER",
    "LC_NAME",
    "LC_ADDRESS",
    "LC_TELEPHONE",
    "LC_MEASUREMENT",
    "LC_IDENTIFICATION",
    "LC_ALL",
];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// All locale and keymap state held by the daemon.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct LocaleState {
    /// Locale variables from /etc/locale.conf (LANG, LC_*, etc.)
    pub locale: BTreeMap<String, String>,
    /// Virtual console keymap (KEYMAP from /etc/vconsole.conf)
    pub vconsole_keymap: String,
    /// Virtual console toggle keymap (KEYMAP_TOGGLE from /etc/vconsole.conf)
    pub vconsole_keymap_toggle: String,
    /// X11 keyboard layout
    pub x11_layout: String,
    /// X11 keyboard model
    pub x11_model: String,
    /// X11 keyboard variant
    pub x11_variant: String,
    /// X11 keyboard options
    pub x11_options: String,
}

// ---------------------------------------------------------------------------
// Reading state
// ---------------------------------------------------------------------------

impl LocaleState {
    /// Load all locale/keymap state from the filesystem.
    pub fn load() -> Self {
        Self::load_from(LOCALE_CONF_PATH, VCONSOLE_CONF_PATH)
    }

    /// Load state from custom paths (for testing).
    pub fn load_from(locale_conf_path: &str, vconsole_conf_path: &str) -> Self {
        let mut state = LocaleState::default();

        // Load locale
        let locale_entries = parse_env_file(locale_conf_path);
        for var in LOCALE_VARIABLES {
            if let Some(val) = locale_entries.get(*var)
                && !val.is_empty()
            {
                state.locale.insert(var.to_string(), val.clone());
            }
        }

        // Load vconsole keymap and X11 settings
        let vc_entries = parse_env_file(vconsole_conf_path);
        state.vconsole_keymap = vc_entries.get("KEYMAP").cloned().unwrap_or_default();
        state.vconsole_keymap_toggle = vc_entries.get("KEYMAP_TOGGLE").cloned().unwrap_or_default();

        // X11 layout can be stored in vconsole.conf (NixOS does this)
        state.x11_layout = vc_entries.get("X11_LAYOUT").cloned().unwrap_or_default();
        state.x11_model = vc_entries.get("X11_MODEL").cloned().unwrap_or_default();
        state.x11_variant = vc_entries.get("X11_VARIANT").cloned().unwrap_or_default();
        state.x11_options = vc_entries.get("X11_OPTIONS").cloned().unwrap_or_default();

        state
    }

    /// Return the effective LANG value, defaulting to "C.UTF-8" if unset.
    #[allow(dead_code)]
    pub fn lang(&self) -> &str {
        self.locale
            .get("LANG")
            .map(|s| s.as_str())
            .unwrap_or("C.UTF-8")
    }

    /// Format the state as a human-readable status block.
    pub fn format_status(&self) -> String {
        let mut out = String::new();
        let label_width = 20;

        // System Locale
        out.push_str(&format!("{:>label_width$}:", "System Locale"));
        if self.locale.is_empty() {
            out.push_str(" (unset)\n");
        } else {
            let mut first = true;
            for (key, value) in &self.locale {
                if first {
                    out.push_str(&format!(" {}={}\n", key, value));
                    first = false;
                } else {
                    out.push_str(&format!("{:>label_width$}  {}={}\n", "", key, value));
                }
            }
        }

        // VC Keymap
        out.push_str(&format!(
            "{:>label_width$}: {}\n",
            "VC Keymap",
            if self.vconsole_keymap.is_empty() {
                "(unset)"
            } else {
                &self.vconsole_keymap
            }
        ));

        if !self.vconsole_keymap_toggle.is_empty() {
            out.push_str(&format!(
                "{:>label_width$}: {}\n",
                "VC Toggle Keymap", self.vconsole_keymap_toggle
            ));
        }

        // X11 Layout
        out.push_str(&format!(
            "{:>label_width$}: {}\n",
            "X11 Layout",
            if self.x11_layout.is_empty() {
                "(unset)"
            } else {
                &self.x11_layout
            }
        ));

        if !self.x11_model.is_empty() {
            out.push_str(&format!(
                "{:>label_width$}: {}\n",
                "X11 Model", self.x11_model
            ));
        }

        if !self.x11_variant.is_empty() {
            out.push_str(&format!(
                "{:>label_width$}: {}\n",
                "X11 Variant", self.x11_variant
            ));
        }

        if !self.x11_options.is_empty() {
            out.push_str(&format!(
                "{:>label_width$}: {}\n",
                "X11 Options", self.x11_options
            ));
        }

        out
    }

    /// Format state as machine-readable `key=value` pairs (for `localectl show`).
    pub fn format_show(&self) -> String {
        let mut out = String::new();

        for var in LOCALE_VARIABLES {
            if let Some(val) = self.locale.get(*var) {
                out.push_str(&format!("{}={}\n", var, val));
            }
        }

        out.push_str(&format!("VConsoleKeymap={}\n", self.vconsole_keymap));
        out.push_str(&format!(
            "VConsoleKeymapToggle={}\n",
            self.vconsole_keymap_toggle
        ));
        out.push_str(&format!("X11Layout={}\n", self.x11_layout));
        out.push_str(&format!("X11Model={}\n", self.x11_model));
        out.push_str(&format!("X11Variant={}\n", self.x11_variant));
        out.push_str(&format!("X11Options={}\n", self.x11_options));

        out
    }
}

// ---------------------------------------------------------------------------
// Writing state
// ---------------------------------------------------------------------------

/// Set locale variables in /etc/locale.conf.
/// `entries` is a map of variable name to value. Empty values remove the variable.
pub fn set_locale(entries: &BTreeMap<String, String>) -> io::Result<()> {
    set_locale_at(entries, LOCALE_CONF_PATH)
}

pub fn set_locale_at(entries: &BTreeMap<String, String>, path: &str) -> io::Result<()> {
    // Only write known locale variables
    let mut filtered = BTreeMap::new();
    for (k, v) in entries {
        if LOCALE_VARIABLES.contains(&k.as_str()) && !v.is_empty() {
            filtered.insert(k.clone(), v.clone());
        }
    }

    write_env_file(path, &filtered)
}

/// Set the virtual console keymap in /etc/vconsole.conf.
pub fn set_vconsole_keymap(keymap: &str, keymap_toggle: &str) -> io::Result<()> {
    set_vconsole_keymap_at(keymap, keymap_toggle, VCONSOLE_CONF_PATH)
}

pub fn set_vconsole_keymap_at(keymap: &str, keymap_toggle: &str, path: &str) -> io::Result<()> {
    let mut entries = parse_env_file(path);

    if keymap.is_empty() {
        entries.remove("KEYMAP");
    } else {
        entries.insert("KEYMAP".to_string(), keymap.to_string());
    }

    if keymap_toggle.is_empty() {
        entries.remove("KEYMAP_TOGGLE");
    } else {
        entries.insert("KEYMAP_TOGGLE".to_string(), keymap_toggle.to_string());
    }

    write_env_file(path, &entries)
}

/// Set the X11 keyboard layout. Writes to both /etc/vconsole.conf (X11_* variables)
/// and /etc/X11/xorg.conf.d/00-keyboard.conf (xorg InputClass section).
pub fn set_x11_keymap(layout: &str, model: &str, variant: &str, options: &str) -> io::Result<()> {
    set_x11_keymap_at(
        layout,
        model,
        variant,
        options,
        VCONSOLE_CONF_PATH,
        X11_KEYBOARD_DIR,
        X11_KEYBOARD_CONF,
    )
}

pub fn set_x11_keymap_at(
    layout: &str,
    model: &str,
    variant: &str,
    options: &str,
    vconsole_path: &str,
    x11_dir: &str,
    x11_conf_path: &str,
) -> io::Result<()> {
    // Update vconsole.conf with X11_* variables
    let mut entries = parse_env_file(vconsole_path);

    set_or_remove(&mut entries, "X11_LAYOUT", layout);
    set_or_remove(&mut entries, "X11_MODEL", model);
    set_or_remove(&mut entries, "X11_VARIANT", variant);
    set_or_remove(&mut entries, "X11_OPTIONS", options);

    write_env_file(vconsole_path, &entries)?;

    // Write X11 keyboard configuration file
    write_x11_keyboard_conf(layout, model, variant, options, x11_dir, x11_conf_path)
}

fn set_or_remove(map: &mut BTreeMap<String, String>, key: &str, value: &str) {
    if value.is_empty() {
        map.remove(key);
    } else {
        map.insert(key.to_string(), value.to_string());
    }
}

// ---------------------------------------------------------------------------
// kbd-model-map: console keymap ↔ X11 keyboard mapping
// ---------------------------------------------------------------------------

/// A single entry from the kbd-model-map file.
///
/// Maps a console keymap name to its corresponding X11 keyboard parameters.
/// File format (tab/space-separated, `-` means empty):
/// ```text
/// # consolelayout    xlayout    xmodel    xvariant    xoptions
/// us                 us         pc105+inet -          terminate:ctrl_alt_bksp
/// de-latin1          de         pc105      -          terminate:ctrl_alt_bksp
/// ```
#[derive(Debug, Clone, PartialEq)]
pub struct KbdModelMapEntry {
    /// Console keymap name (e.g. "us", "de-latin1")
    pub console_keymap: String,
    /// X11 layout (e.g. "us", "de", "ch")
    pub x11_layout: String,
    /// X11 model (e.g. "pc105", "pc105+inet")
    pub x11_model: String,
    /// X11 variant (e.g. "nodeadkeys", "dvorak"); empty if `-`
    pub x11_variant: String,
    /// X11 options (e.g. "terminate:ctrl_alt_bksp"); empty if `-`
    pub x11_options: String,
}

/// Load the kbd-model-map from the first available system path.
pub fn load_kbd_model_map() -> Vec<KbdModelMapEntry> {
    for path in KBD_MODEL_MAP_PATHS {
        if let Ok(entries) = load_kbd_model_map_from(path)
            && !entries.is_empty()
        {
            return entries;
        }
    }
    Vec::new()
}

/// Load and parse a kbd-model-map file from a specific path.
pub fn load_kbd_model_map_from(path: &str) -> io::Result<Vec<KbdModelMapEntry>> {
    let content = fs::read_to_string(path)?;
    Ok(parse_kbd_model_map(&content))
}

/// Parse kbd-model-map content into entries.
pub fn parse_kbd_model_map(content: &str) -> Vec<KbdModelMapEntry> {
    let mut entries = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let fields: Vec<&str> = line.split_whitespace().collect();
        if fields.len() < 5 {
            continue; // Malformed line, skip
        }

        let dash_to_empty = |s: &str| -> String {
            if s == "-" {
                String::new()
            } else {
                s.to_string()
            }
        };

        entries.push(KbdModelMapEntry {
            console_keymap: fields[0].to_string(),
            x11_layout: fields[1].to_string(),
            x11_model: dash_to_empty(fields[2]),
            x11_variant: dash_to_empty(fields[3]),
            x11_options: dash_to_empty(fields[4]),
        });
    }

    entries
}

/// Result of a keymap-to-X11 conversion.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct X11KeymapResult {
    pub layout: String,
    pub model: String,
    pub variant: String,
    pub options: String,
}

/// Convert a console keymap name to X11 keyboard parameters.
///
/// Searches the kbd-model-map for the first entry whose `console_keymap`
/// matches the given keymap name. Returns `None` if no match is found.
pub fn keymap_to_x11(keymap: &str) -> Option<X11KeymapResult> {
    let entries = load_kbd_model_map();
    keymap_to_x11_from(&entries, keymap)
}

/// Convert a console keymap to X11 parameters using a pre-loaded map.
pub fn keymap_to_x11_from(entries: &[KbdModelMapEntry], keymap: &str) -> Option<X11KeymapResult> {
    if keymap.is_empty() {
        return None;
    }

    for entry in entries {
        if entry.console_keymap == keymap {
            return Some(X11KeymapResult {
                layout: entry.x11_layout.clone(),
                model: entry.x11_model.clone(),
                variant: entry.x11_variant.clone(),
                options: entry.x11_options.clone(),
            });
        }
    }

    None
}

/// Convert X11 keyboard parameters to a console keymap name.
///
/// Matching strategy (following real systemd's `vconsole-util.c`):
/// 1. Try exact match on layout + variant (ignoring model and options).
/// 2. If no exact match, try matching layout only (with empty variant in the map).
/// 3. For multi-layout entries like "de,us", match the first layout component.
///
/// Returns `None` if no match is found.
pub fn x11_to_keymap(layout: &str, _model: &str, variant: &str) -> Option<String> {
    let entries = load_kbd_model_map();
    x11_to_keymap_from(&entries, layout, _model, variant)
}

/// Convert X11 parameters to a console keymap using a pre-loaded map.
pub fn x11_to_keymap_from(
    entries: &[KbdModelMapEntry],
    layout: &str,
    _model: &str,
    variant: &str,
) -> Option<String> {
    if layout.is_empty() {
        return None;
    }

    // Phase 1: Exact match on layout + variant
    for entry in entries {
        // The map may have multi-layout entries like "ch" matching layout "ch"
        // and variant matching too.
        if entry.x11_layout == layout && entry.x11_variant == variant {
            return Some(entry.console_keymap.clone());
        }
    }

    // Phase 2: Match layout only, preferring entries with empty variant
    if !variant.is_empty() {
        for entry in entries {
            if entry.x11_layout == layout && entry.x11_variant.is_empty() {
                return Some(entry.console_keymap.clone());
            }
        }
    }

    // Phase 3: For multi-layout X11 entries (e.g. "mk,us"), match the
    // first component of the entry's layout against our layout.
    for entry in entries {
        if let Some(first) = entry.x11_layout.split(',').next()
            && first == layout
            && entry.x11_variant == variant
        {
            return Some(entry.console_keymap.clone());
        }
    }

    // Phase 4: Multi-layout first component with empty variant fallback
    if !variant.is_empty() {
        for entry in entries {
            if let Some(first) = entry.x11_layout.split(',').next()
                && first == layout
                && entry.x11_variant.is_empty()
            {
                return Some(entry.console_keymap.clone());
            }
        }
    }

    None
}

/// Generate and write the X11 keyboard configuration file for xorg.
fn write_x11_keyboard_conf(
    layout: &str,
    model: &str,
    variant: &str,
    options: &str,
    dir: &str,
    path: &str,
) -> io::Result<()> {
    // If all settings are empty, remove the config file
    if layout.is_empty() && model.is_empty() && variant.is_empty() && options.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        return Ok(());
    }

    // Ensure directory exists
    fs::create_dir_all(dir)?;

    let mut f = fs::File::create(path)?;
    writeln!(f, "# Written by systemd-localed(8), do not edit manually.")?;
    writeln!(f)?;
    writeln!(f, "Section \"InputClass\"")?;
    writeln!(f, "        Identifier \"system-keyboard\"")?;
    writeln!(f, "        MatchIsKeyboard \"on\"")?;

    if !layout.is_empty() {
        writeln!(f, "        Option \"XkbLayout\" \"{}\"", layout)?;
    }
    if !model.is_empty() {
        writeln!(f, "        Option \"XkbModel\" \"{}\"", model)?;
    }
    if !variant.is_empty() {
        writeln!(f, "        Option \"XkbVariant\" \"{}\"", variant)?;
    }
    if !options.is_empty() {
        writeln!(f, "        Option \"XkbOptions\" \"{}\"", options)?;
    }

    writeln!(f, "EndSection")?;

    Ok(())
}

/// List available keymaps by scanning common keymap directories.
pub fn list_keymaps() -> Vec<String> {
    let mut keymaps = Vec::new();

    let keymap_dirs = [
        "/usr/share/keymaps",
        "/usr/share/kbd/keymaps",
        "/usr/lib/kbd/keymaps",
        "/run/current-system/sw/share/keymaps",
    ];

    for dir in &keymap_dirs {
        if let Ok(()) = collect_keymaps_recursive(Path::new(dir), &mut keymaps)
            && !keymaps.is_empty()
        {
            break;
        }
    }

    keymaps.sort();
    keymaps.dedup();
    keymaps
}

fn collect_keymaps_recursive(dir: &Path, keymaps: &mut Vec<String>) -> io::Result<()> {
    if !dir.is_dir() {
        return Ok(());
    }

    for entry in fs::read_dir(dir)? {
        let entry = entry?;
        let path = entry.path();

        if path.is_dir() {
            collect_keymaps_recursive(&path, keymaps)?;
        } else if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            // Keymap files typically end in .map or .map.gz
            let keymap_name = if let Some(stripped) = name.strip_suffix(".map.gz") {
                Some(stripped.to_string())
            } else {
                name.strip_suffix(".map")
                    .map(|stripped| stripped.to_string())
            };

            if let Some(km) = keymap_name {
                keymaps.push(km);
            }
        }
    }

    Ok(())
}

/// List available X11 keyboard layouts by reading xkb rules.
pub fn list_x11_layouts() -> Vec<String> {
    let rules_paths = [
        "/usr/share/X11/xkb/rules/base.lst",
        "/usr/share/X11/xkb/rules/evdev.lst",
        "/run/current-system/sw/share/X11/xkb/rules/base.lst",
    ];

    for path in &rules_paths {
        if let Ok(layouts) = parse_xkb_rules_layouts(path)
            && !layouts.is_empty()
        {
            return layouts;
        }
    }

    Vec::new()
}

fn parse_xkb_rules_layouts(path: &str) -> io::Result<Vec<String>> {
    let content = fs::read_to_string(path)?;
    let mut layouts = Vec::new();
    let mut in_layout_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('!') {
            in_layout_section = trimmed.contains("layout");
            continue;
        }

        if in_layout_section {
            if trimmed.is_empty() {
                break; // End of section
            }
            // Format: "  layout_name    Description"
            if let Some(name) = trimmed.split_whitespace().next() {
                layouts.push(name.to_string());
            }
        }
    }

    Ok(layouts)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse an environment-style file (`KEY=VALUE` or `KEY="VALUE"` per line).
fn parse_env_file(path: &str) -> BTreeMap<String, String> {
    parse_env_file_content(&fs::read_to_string(path).unwrap_or_default())
}

/// Parse environment file content.
fn parse_env_file_content(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let mut value = value.trim().to_string();
            // Strip surrounding quotes
            if ((value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\'')))
                && value.len() >= 2
            {
                value = value[1..value.len() - 1].to_string();
            }
            // Unescape common escape sequences
            value = value.replace("\\\"", "\"").replace("\\\\", "\\");
            if !key.is_empty() {
                map.insert(key.to_string(), value);
            }
        }
    }
    map
}

/// Write all entries to a file in `KEY=VALUE` or `KEY="VALUE"` format.
fn write_env_file(path: &str, entries: &BTreeMap<String, String>) -> io::Result<()> {
    if entries.is_empty() {
        // Remove the file if there are no entries
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        return Ok(());
    }

    let mut f = fs::File::create(path)?;
    for (k, v) in entries {
        if v.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\' || c == '$') {
            let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
            writeln!(f, "{}=\"{}\"", k, escaped)?;
        } else {
            writeln!(f, "{}={}", k, v)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Shared state for D-Bus
// ---------------------------------------------------------------------------

type SharedState = Arc<Mutex<LocaleState>>;

// ---------------------------------------------------------------------------
// D-Bus interface: org.freedesktop.locale1
// ---------------------------------------------------------------------------

/// D-Bus interface struct for org.freedesktop.locale1.
///
/// Properties (read-only):
///   Locale (as), X11Layout, X11Model, X11Variant, X11Options,
///   VConsoleKeymap, VConsoleKeymapToggle
///
/// Methods:
///   SetLocale(as locale, b interactive)
///   SetVConsoleKeyboard(s keymap, s keymap_toggle, b convert, b interactive)
///   SetX11Keyboard(s layout, s model, s variant, s options, b convert, b interactive)
struct Locale1Manager {
    state: SharedState,
}

#[zbus::interface(name = "org.freedesktop.locale1")]
impl Locale1Manager {
    // --- Properties (read-only) ---

    // Locale → array of strings like ["LANG=en_US.UTF-8", "LC_TIME=de_DE.UTF-8"]
    #[zbus(property, name = "Locale")]
    fn locale(&self) -> Vec<String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.locale
            .iter()
            .map(|(k, v)| format!("{}={}", k, v))
            .collect()
    }

    #[zbus(property, name = "X11Layout")]
    fn x11_layout(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.x11_layout.clone()
    }

    #[zbus(property, name = "X11Model")]
    fn x11_model(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.x11_model.clone()
    }

    #[zbus(property, name = "X11Variant")]
    fn x11_variant(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.x11_variant.clone()
    }

    #[zbus(property, name = "X11Options")]
    fn x11_options(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.x11_options.clone()
    }

    #[zbus(property, name = "VConsoleKeymap")]
    fn vconsole_keymap(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.vconsole_keymap.clone()
    }

    #[zbus(property, name = "VConsoleKeymapToggle")]
    fn vconsole_keymap_toggle(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.vconsole_keymap_toggle.clone()
    }

    // --- Methods ---

    fn set_locale(&self, locale: Vec<String>, _interactive: bool) -> zbus::fdo::Result<()> {
        let mut entries = BTreeMap::new();
        for assignment in &locale {
            if let Some((key, value)) = assignment.split_once('=') {
                if LOCALE_VARIABLES.contains(&key) {
                    entries.insert(key.to_string(), value.to_string());
                } else {
                    return Err(zbus::fdo::Error::Failed(format!(
                        "Unknown locale variable '{}'",
                        key
                    )));
                }
            }
        }
        if let Err(e) = set_locale(&entries) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set locale: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.locale = entries;
        Ok(())
    }

    fn set_v_console_keyboard(
        &self,
        keymap: String,
        keymap_toggle: String,
        convert: bool,
        _interactive: bool,
    ) -> zbus::fdo::Result<()> {
        if let Err(e) = set_vconsole_keymap(&keymap, &keymap_toggle) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set vconsole keymap: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.vconsole_keymap = keymap.clone();
        s.vconsole_keymap_toggle = keymap_toggle;

        // Automatic conversion: keymap → X11
        if convert {
            if let Some(x11) = keymap_to_x11(&keymap) {
                log::info!(
                    "Converting keymap '{}' → X11 layout='{}' model='{}' variant='{}' options='{}'",
                    keymap,
                    x11.layout,
                    x11.model,
                    x11.variant,
                    x11.options
                );
                if let Err(e) = set_x11_keymap(&x11.layout, &x11.model, &x11.variant, &x11.options)
                {
                    log::warn!("Failed to convert keymap to X11: {}", e);
                } else {
                    s.x11_layout = x11.layout;
                    s.x11_model = x11.model;
                    s.x11_variant = x11.variant;
                    s.x11_options = x11.options;
                }
            } else if keymap.is_empty() {
                // Clear X11 settings when keymap is cleared
                let _ = set_x11_keymap("", "", "", "");
                s.x11_layout.clear();
                s.x11_model.clear();
                s.x11_variant.clear();
                s.x11_options.clear();
            } else {
                log::debug!(
                    "No kbd-model-map entry found for keymap '{}', skipping X11 conversion",
                    keymap
                );
            }
        }

        Ok(())
    }

    #[zbus(name = "SetX11Keyboard")]
    fn set_x11_keyboard(
        &self,
        layout: String,
        model: String,
        variant: String,
        options: String,
        convert: bool,
        _interactive: bool,
    ) -> zbus::fdo::Result<()> {
        if let Err(e) = set_x11_keymap(&layout, &model, &variant, &options) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set X11 keymap: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.x11_layout = layout.clone();
        s.x11_model = model.clone();
        s.x11_variant = variant.clone();
        s.x11_options = options;

        // Automatic conversion: X11 → keymap
        if convert {
            if let Some(km) = x11_to_keymap(&layout, &model, &variant) {
                log::info!(
                    "Converting X11 layout='{}' variant='{}' → keymap '{}'",
                    layout,
                    variant,
                    km
                );
                if let Err(e) = set_vconsole_keymap(&km, "") {
                    log::warn!("Failed to convert X11 to keymap: {}", e);
                } else {
                    s.vconsole_keymap = km;
                    s.vconsole_keymap_toggle.clear();
                }
            } else if layout.is_empty() {
                // Clear keymap when X11 layout is cleared
                let _ = set_vconsole_keymap("", "");
                s.vconsole_keymap.clear();
                s.vconsole_keymap_toggle.clear();
            } else {
                log::debug!(
                    "No kbd-model-map entry found for X11 layout='{}' variant='{}', skipping keymap conversion",
                    layout,
                    variant
                );
            }
        }

        Ok(())
    }
}

/// Set up the D-Bus connection and register the locale1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedState) -> Result<Connection, String> {
    let iface = Locale1Manager { state: shared };
    let conn = zbus::blocking::connection::Builder::system()
        .map_err(|e| format!("D-Bus builder failed: {}", e))?
        .name(DBUS_NAME)
        .map_err(|e| format!("D-Bus name request failed: {}", e))?
        .serve_at(DBUS_PATH, iface)
        .map_err(|e| format!("D-Bus serve_at failed: {}", e))?
        .build()
        .map_err(|e| format!("D-Bus connection failed: {}", e))?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Control socket protocol
// ---------------------------------------------------------------------------

/// Handle a single command received on the control socket.
/// Commands are simple line-based: `COMMAND [ARGS...]`
fn handle_control_command(line: &str) -> String {
    let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
    let cmd = parts.first().copied().unwrap_or("");

    match cmd {
        "STATUS" | "status" => {
            let state = LocaleState::load();
            state.format_status()
        }
        "SHOW" | "show" => {
            let state = LocaleState::load();
            state.format_show()
        }
        "SET-LOCALE" | "set-locale" => {
            // Expects: SET-LOCALE LANG=en_US.UTF-8 LC_TIME=de_DE.UTF-8 ...
            let rest = if parts.len() >= 2 {
                line.trim().split_once(' ').map(|x| x.1).unwrap_or("")
            } else {
                ""
            };

            let mut entries = BTreeMap::new();
            for assignment in rest.split_whitespace() {
                if let Some((key, value)) = assignment.split_once('=') {
                    if LOCALE_VARIABLES.contains(&key) {
                        entries.insert(key.to_string(), value.to_string());
                    } else {
                        return format!("ERROR: Unknown locale variable '{}'\n", key);
                    }
                }
            }

            match set_locale(&entries) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-KEYMAP" | "set-keymap" => {
            // Expects: SET-KEYMAP <keymap> [toggle-keymap] [--convert]
            let rest = if parts.len() >= 2 {
                line.trim().split_once(' ').map(|x| x.1).unwrap_or("")
            } else {
                ""
            };
            let args: Vec<&str> = rest.split_whitespace().collect();
            let convert = args.contains(&"--convert");
            let positional: Vec<&&str> = args.iter().filter(|a| !a.starts_with("--")).collect();
            let keymap = positional.first().map(|s| **s).unwrap_or("");
            let toggle = positional.get(1).map(|s| **s).unwrap_or("");

            match set_vconsole_keymap(keymap, toggle) {
                Ok(()) => {
                    if convert {
                        if let Some(x11) = keymap_to_x11(keymap) {
                            let _ =
                                set_x11_keymap(&x11.layout, &x11.model, &x11.variant, &x11.options);
                        } else if keymap.is_empty() {
                            let _ = set_x11_keymap("", "", "", "");
                        }
                    }
                    "OK\n".to_string()
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-X11-KEYMAP" | "set-x11-keymap" => {
            // Expects: SET-X11-KEYMAP <layout> [model [variant [options]]] [--convert]
            let rest = if parts.len() >= 2 {
                line.trim().split_once(' ').map(|x| x.1).unwrap_or("")
            } else {
                ""
            };
            let args: Vec<&str> = rest.split_whitespace().collect();
            let convert = args.contains(&"--convert");
            let positional: Vec<&&str> = args.iter().filter(|a| !a.starts_with("--")).collect();
            let layout = positional.first().map(|s| **s).unwrap_or("");
            let model = positional.get(1).map(|s| **s).unwrap_or("");
            let variant = positional.get(2).map(|s| **s).unwrap_or("");
            let options = positional.get(3).map(|s| **s).unwrap_or("");

            match set_x11_keymap(layout, model, variant, options) {
                Ok(()) => {
                    if convert {
                        if let Some(km) = x11_to_keymap(layout, model, variant) {
                            let _ = set_vconsole_keymap(&km, "");
                        } else if layout.is_empty() {
                            let _ = set_vconsole_keymap("", "");
                        }
                    }
                    "OK\n".to_string()
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "LIST-KEYMAPS" | "list-keymaps" => {
            let keymaps = list_keymaps();
            if keymaps.is_empty() {
                "No keymaps found.\n".to_string()
            } else {
                keymaps.join("\n") + "\n"
            }
        }
        "LIST-X11-LAYOUTS" | "list-x11-layouts" => {
            let layouts = list_x11_layouts();
            if layouts.is_empty() {
                "No X11 layouts found.\n".to_string()
            } else {
                layouts.join("\n") + "\n"
            }
        }
        _ => format!("ERROR: Unknown command '{}'\n", cmd),
    }
}

/// Handle a client connection on the control socket.
fn handle_client(stream: &mut UnixStream) {
    let reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    for line in reader.lines() {
        match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => {
                let response = handle_control_command(&l);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// sd_notify helper
// ---------------------------------------------------------------------------

fn sd_notify(msg: &str) {
    let sock_path = match env::var("NOTIFY_SOCKET") {
        Ok(p) => p,
        Err(_) => return,
    };

    let path = if let Some(stripped) = sock_path.strip_prefix('@') {
        format!("\0{}", stripped)
    } else {
        sock_path
    };

    let sock = match std::os::unix::net::UnixDatagram::unbound() {
        Ok(s) => s,
        Err(_) => return,
    };

    let _ = sock.send_to(msg.as_bytes(), &path);
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sigint(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sighup(_: libc::c_int) {
    RELOAD.store(true, Ordering::SeqCst);
}

fn setup_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handle_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            handle_sighup as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

fn init_logging() {
    struct StderrLogger;

    impl log::Log for StderrLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                let now = chrono_lite_timestamp();
                eprintln!(
                    "[{}][systemd-localed][{}] {}",
                    now,
                    record.level(),
                    record.args()
                );
            }
        }

        fn flush(&self) {}
    }

    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

/// Minimal timestamp without pulling in chrono.
fn chrono_lite_timestamp() -> String {
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
    }

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&tv.tv_sec, &mut tm);
    }

    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

/// Parse `WATCHDOG_USEC` from the environment and return the interval at which
/// we should send `WATCHDOG=1` (half the watchdog timeout). Returns `None` if
/// the variable is absent or unparsable.
fn watchdog_interval() -> Option<Duration> {
    let usec: u64 = env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    if usec == 0 {
        return None;
    }
    // Notify at half the watchdog period so we have margin
    Some(Duration::from_micros(usec / 2))
}

fn main() {
    init_logging();
    setup_signal_handlers();

    log::info!("systemd-localed starting");

    // Load initial state into shared state for D-Bus and control socket
    let initial_state = LocaleState::load();
    log::info!("System locale: LANG={}", initial_state.lang());
    if !initial_state.vconsole_keymap.is_empty() {
        log::info!("VC Keymap: {}", initial_state.vconsole_keymap);
    }
    if !initial_state.x11_layout.is_empty() {
        log::info!("X11 Layout: {}", initial_state.x11_layout);
    }
    let shared_state: SharedState = Arc::new(Mutex::new(initial_state.clone()));

    // Watchdog support — send WATCHDOG=1 at half the configured interval
    let wd_interval = watchdog_interval();
    if let Some(ref iv) = wd_interval {
        log::info!("Watchdog enabled, interval {:?}", iv);
    }
    let mut last_watchdog = Instant::now();

    // D-Bus connection is deferred to after READY=1 so we don't block
    // early boot waiting for dbus-daemon.  zbus dispatches messages
    // automatically in a background thread — we just keep the connection alive.
    let mut _dbus_conn: Option<Connection> = None;
    let mut dbus_attempted = false;

    // Ensure /run/systemd exists
    let _ = fs::create_dir_all(Path::new(CONTROL_SOCKET_PATH).parent().unwrap());

    // Remove stale socket
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);

    // Bind control socket
    let listener = match UnixListener::bind(CONTROL_SOCKET_PATH) {
        Ok(l) => {
            log::info!("Listening on {}", CONTROL_SOCKET_PATH);
            Some(l)
        }
        Err(e) => {
            log::error!(
                "Failed to bind control socket {}: {}",
                CONTROL_SOCKET_PATH,
                e
            );
            None
        }
    };

    // Set socket to non-blocking so we can check SHUTDOWN flag periodically
    if let Some(ref l) = listener {
        l.set_nonblocking(true).expect("Failed to set non-blocking");
    }

    sd_notify(&format!("READY=1\nSTATUS=LANG={}", initial_state.lang()));

    log::info!("systemd-localed ready");

    // Main loop
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            log::info!("Received shutdown signal");
            break;
        }

        if RELOAD.load(Ordering::SeqCst) {
            RELOAD.store(false, Ordering::SeqCst);
            let new_state = LocaleState::load();
            log::info!("Reloaded configuration, LANG={}", new_state.lang());
            {
                let mut s = shared_state.lock().unwrap_or_else(|e| e.into_inner());
                *s = new_state.clone();
            }
            sd_notify(&format!("STATUS=LANG={}", new_state.lang()));
        }

        // Send watchdog keepalive
        if let Some(ref iv) = wd_interval
            && last_watchdog.elapsed() >= *iv
        {
            sd_notify("WATCHDOG=1");
            last_watchdog = Instant::now();
        }

        // Attempt D-Bus registration once (deferred from startup so we don't
        // block early boot before dbus-daemon is running).
        // zbus handles message dispatch in a background thread automatically.
        if !dbus_attempted {
            dbus_attempted = true;
            match setup_dbus(shared_state.clone()) {
                Ok(conn) => {
                    log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                    _dbus_conn = Some(conn);
                    sd_notify(&format!(
                        "STATUS=LANG={} (D-Bus active)",
                        initial_state.lang()
                    ));
                }
                Err(e) => {
                    log::warn!(
                        "Failed to register D-Bus interface ({}); control socket only",
                        e
                    );
                }
            }
        }

        // Accept control socket connections
        if let Some(ref listener) = listener {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    handle_client(&mut stream);
                    let _ = stream.shutdown(Shutdown::Both);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No connection waiting
                }
                Err(e) => {
                    log::warn!("Accept error: {}", e);
                }
            }
        }

        // Brief sleep to avoid busy-looping
        thread::sleep(Duration::from_millis(50));
    }

    // Cleanup
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);
    sd_notify("STOPPING=1");
    log::info!("systemd-localed stopped");
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    // Sample kbd-model-map content for testing
    const SAMPLE_KBD_MODEL_MAP: &str = "\
# consolelayout\t\txlayout\txmodel\t\txvariant\txoptions
us\t\t\tus\tpc105+inet\t-\t\tterminate:ctrl_alt_bksp
de\t\t\tde\tpc105\t\t-\t\tterminate:ctrl_alt_bksp
de-latin1\t\tde\tpc105\t\t-\t\tterminate:ctrl_alt_bksp
de-latin1-nodeadkeys\tde\tpc105\t\tnodeadkeys\tterminate:ctrl_alt_bksp
uk\t\t\tgb\tpc105\t\t-\t\tterminate:ctrl_alt_bksp
fr\t\t\tfr\tpc105\t\t-\t\tterminate:ctrl_alt_bksp
dvorak\t\t\tus\tpc105\t\tdvorak\t\tterminate:ctrl_alt_bksp
jp106\t\t\tjp\tjp106\t\t-\t\tterminate:ctrl_alt_bksp
br-abnt2\t\tbr\tabnt2\t\t-\t\tterminate:ctrl_alt_bksp
ru\t\t\tru,us\tpc105\t\t-\t\tterminate:ctrl_alt_bksp,grp:shifts_toggle,grp_led:scroll
sg\t\t\tch\tpc105\t\tde_nodeadkeys\tterminate:ctrl_alt_bksp
fr_CH\t\t\tch\tpc105\t\tfr\t\tterminate:ctrl_alt_bksp
hu101\t\t\thu\tpc105\t\tqwerty\t\tterminate:ctrl_alt_bksp
hu\t\t\thu\tpc105\t\t-\t\tterminate:ctrl_alt_bksp
";

    // -- kbd-model-map parsing tests --

    #[test]
    fn test_parse_kbd_model_map_basic() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        assert_eq!(entries.len(), 14);
        assert_eq!(entries[0].console_keymap, "us");
        assert_eq!(entries[0].x11_layout, "us");
        assert_eq!(entries[0].x11_model, "pc105+inet");
        assert!(entries[0].x11_variant.is_empty());
        assert_eq!(entries[0].x11_options, "terminate:ctrl_alt_bksp");
    }

    #[test]
    fn test_parse_kbd_model_map_dash_is_empty() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let de = entries.iter().find(|e| e.console_keymap == "de").unwrap();
        assert!(de.x11_variant.is_empty(), "dash should become empty string");
    }

    #[test]
    fn test_parse_kbd_model_map_with_variant() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let nodeadkeys = entries
            .iter()
            .find(|e| e.console_keymap == "de-latin1-nodeadkeys")
            .unwrap();
        assert_eq!(nodeadkeys.x11_layout, "de");
        assert_eq!(nodeadkeys.x11_variant, "nodeadkeys");
    }

    #[test]
    fn test_parse_kbd_model_map_empty_content() {
        let entries = parse_kbd_model_map("");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_kbd_model_map_comments_only() {
        let entries = parse_kbd_model_map("# This is a comment\n# Another comment\n");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_kbd_model_map_malformed_line_skipped() {
        let content = "us us\nde de pc105 - terminate:ctrl_alt_bksp\n";
        let entries = parse_kbd_model_map(content);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].console_keymap, "de");
    }

    #[test]
    fn test_parse_kbd_model_map_multi_layout() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let ru = entries.iter().find(|e| e.console_keymap == "ru").unwrap();
        assert_eq!(ru.x11_layout, "ru,us");
        assert_eq!(
            ru.x11_options,
            "terminate:ctrl_alt_bksp,grp:shifts_toggle,grp_led:scroll"
        );
    }

    // -- keymap_to_x11 tests --

    #[test]
    fn test_keymap_to_x11_us() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = keymap_to_x11_from(&entries, "us").unwrap();
        assert_eq!(result.layout, "us");
        assert_eq!(result.model, "pc105+inet");
        assert!(result.variant.is_empty());
    }

    #[test]
    fn test_keymap_to_x11_de_nodeadkeys() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = keymap_to_x11_from(&entries, "de-latin1-nodeadkeys").unwrap();
        assert_eq!(result.layout, "de");
        assert_eq!(result.variant, "nodeadkeys");
    }

    #[test]
    fn test_keymap_to_x11_not_found() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        assert!(keymap_to_x11_from(&entries, "nonexistent").is_none());
    }

    #[test]
    fn test_keymap_to_x11_empty() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        assert!(keymap_to_x11_from(&entries, "").is_none());
    }

    #[test]
    fn test_keymap_to_x11_jp106() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = keymap_to_x11_from(&entries, "jp106").unwrap();
        assert_eq!(result.layout, "jp");
        assert_eq!(result.model, "jp106");
    }

    #[test]
    fn test_keymap_to_x11_dvorak() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = keymap_to_x11_from(&entries, "dvorak").unwrap();
        assert_eq!(result.layout, "us");
        assert_eq!(result.variant, "dvorak");
    }

    #[test]
    fn test_keymap_to_x11_br_abnt2() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = keymap_to_x11_from(&entries, "br-abnt2").unwrap();
        assert_eq!(result.layout, "br");
        assert_eq!(result.model, "abnt2");
    }

    // -- x11_to_keymap tests --

    #[test]
    fn test_x11_to_keymap_us() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "us", "", "").unwrap();
        assert_eq!(result, "us");
    }

    #[test]
    fn test_x11_to_keymap_de() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "de", "", "").unwrap();
        assert_eq!(result, "de");
    }

    #[test]
    fn test_x11_to_keymap_de_nodeadkeys() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "de", "", "nodeadkeys").unwrap();
        assert_eq!(result, "de-latin1-nodeadkeys");
    }

    #[test]
    fn test_x11_to_keymap_gb() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "gb", "", "").unwrap();
        assert_eq!(result, "uk");
    }

    #[test]
    fn test_x11_to_keymap_not_found() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        assert!(x11_to_keymap_from(&entries, "nonexistent", "", "").is_none());
    }

    #[test]
    fn test_x11_to_keymap_empty() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        assert!(x11_to_keymap_from(&entries, "", "", "").is_none());
    }

    #[test]
    fn test_x11_to_keymap_dvorak() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "us", "", "dvorak").unwrap();
        assert_eq!(result, "dvorak");
    }

    #[test]
    fn test_x11_to_keymap_variant_fallback_to_no_variant() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        // "fr" with a variant that doesn't exist should fall back to the no-variant entry
        let result = x11_to_keymap_from(&entries, "fr", "", "nonexistent_variant").unwrap();
        assert_eq!(result, "fr");
    }

    #[test]
    fn test_x11_to_keymap_multi_layout_ru() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        // "ru" is mapped as "ru,us" in the map — match via first component
        let result = x11_to_keymap_from(&entries, "ru", "", "").unwrap();
        assert_eq!(result, "ru");
    }

    #[test]
    fn test_x11_to_keymap_hu_qwerty() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "hu", "", "qwerty").unwrap();
        assert_eq!(result, "hu101");
    }

    #[test]
    fn test_x11_to_keymap_hu_no_variant() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "hu", "", "").unwrap();
        assert_eq!(result, "hu");
    }

    #[test]
    fn test_x11_to_keymap_ch_de_nodeadkeys() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "ch", "", "de_nodeadkeys").unwrap();
        assert_eq!(result, "sg");
    }

    #[test]
    fn test_x11_to_keymap_ch_fr() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let result = x11_to_keymap_from(&entries, "ch", "", "fr").unwrap();
        assert_eq!(result, "fr_CH");
    }

    // -- load_kbd_model_map_from tests --

    #[test]
    fn test_load_kbd_model_map_from_file() {
        let dir = TempDir::new().unwrap();
        let path = write_file(
            &dir,
            "kbd-model-map",
            "us us pc105 - terminate:ctrl_alt_bksp\nde de pc105 - terminate:ctrl_alt_bksp\n",
        );
        let entries = load_kbd_model_map_from(path.to_str().unwrap()).unwrap();
        assert_eq!(entries.len(), 2);
    }

    #[test]
    fn test_load_kbd_model_map_from_missing_file() {
        let result = load_kbd_model_map_from("/nonexistent/kbd-model-map");
        assert!(result.is_err());
    }

    // -- roundtrip tests (keymap→x11→keymap) --

    #[test]
    fn test_roundtrip_us() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let x11 = keymap_to_x11_from(&entries, "us").unwrap();
        let km = x11_to_keymap_from(&entries, &x11.layout, &x11.model, &x11.variant).unwrap();
        assert_eq!(km, "us");
    }

    #[test]
    fn test_roundtrip_de_nodeadkeys() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let x11 = keymap_to_x11_from(&entries, "de-latin1-nodeadkeys").unwrap();
        let km = x11_to_keymap_from(&entries, &x11.layout, &x11.model, &x11.variant).unwrap();
        assert_eq!(km, "de-latin1-nodeadkeys");
    }

    #[test]
    fn test_roundtrip_dvorak() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let x11 = keymap_to_x11_from(&entries, "dvorak").unwrap();
        let km = x11_to_keymap_from(&entries, &x11.layout, &x11.model, &x11.variant).unwrap();
        assert_eq!(km, "dvorak");
    }

    #[test]
    fn test_roundtrip_uk() {
        let entries = parse_kbd_model_map(SAMPLE_KBD_MODEL_MAP);
        let x11 = keymap_to_x11_from(&entries, "uk").unwrap();
        let km = x11_to_keymap_from(&entries, &x11.layout, &x11.model, &x11.variant).unwrap();
        assert_eq!(km, "uk");
    }

    // -- parse_env_file_content tests --

    #[test]
    fn test_parse_env_file_empty() {
        let result = parse_env_file_content("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_env_file_basic() {
        let content = "LANG=en_US.UTF-8\nLC_TIME=de_DE.UTF-8\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("LANG").unwrap(), "en_US.UTF-8");
        assert_eq!(result.get("LC_TIME").unwrap(), "de_DE.UTF-8");
    }

    #[test]
    fn test_parse_env_file_quoted() {
        let content = "LANG=\"en_US.UTF-8\"\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("LANG").unwrap(), "en_US.UTF-8");
    }

    #[test]
    fn test_parse_env_file_single_quoted() {
        let content = "KEYMAP='us'\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("KEYMAP").unwrap(), "us");
    }

    #[test]
    fn test_parse_env_file_comments_and_blanks() {
        let content = "# Comment\n\nLANG=C\n  # another\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("LANG").unwrap(), "C");
    }

    #[test]
    fn test_parse_env_file_escaped_quote() {
        let content = "NAME=\"value with \\\"quotes\\\"\"\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("NAME").unwrap(), "value with \"quotes\"");
    }

    // -- LocaleState tests --

    #[test]
    fn test_load_state_from_files() {
        let dir = TempDir::new().unwrap();
        let locale_path = write_file(
            &dir,
            "locale.conf",
            "LANG=en_US.UTF-8\nLC_TIME=de_DE.UTF-8\n",
        );
        let vconsole_path = write_file(
            &dir,
            "vconsole.conf",
            "KEYMAP=us\nKEYMAP_TOGGLE=de\nX11_LAYOUT=us\nX11_MODEL=pc105\nX11_VARIANT=intl\nX11_OPTIONS=ctrl:nocaps\n",
        );

        let state = LocaleState::load_from(
            locale_path.to_str().unwrap(),
            vconsole_path.to_str().unwrap(),
        );

        assert_eq!(state.locale.get("LANG").unwrap(), "en_US.UTF-8");
        assert_eq!(state.locale.get("LC_TIME").unwrap(), "de_DE.UTF-8");
        assert_eq!(state.vconsole_keymap, "us");
        assert_eq!(state.vconsole_keymap_toggle, "de");
        assert_eq!(state.x11_layout, "us");
        assert_eq!(state.x11_model, "pc105");
        assert_eq!(state.x11_variant, "intl");
        assert_eq!(state.x11_options, "ctrl:nocaps");
    }

    #[test]
    fn test_load_state_missing_files() {
        let state =
            LocaleState::load_from("/nonexistent/locale.conf", "/nonexistent/vconsole.conf");
        assert!(state.locale.is_empty());
        assert_eq!(state.vconsole_keymap, "");
        assert_eq!(state.x11_layout, "");
    }

    #[test]
    fn test_lang_default() {
        let state = LocaleState::default();
        assert_eq!(state.lang(), "C.UTF-8");
    }

    #[test]
    fn test_lang_from_locale() {
        let mut state = LocaleState::default();
        state
            .locale
            .insert("LANG".to_string(), "en_US.UTF-8".to_string());
        assert_eq!(state.lang(), "en_US.UTF-8");
    }

    #[test]
    fn test_load_ignores_non_locale_variables() {
        let dir = TempDir::new().unwrap();
        let locale_path = write_file(
            &dir,
            "locale.conf",
            "LANG=C\nFOO=bar\nLC_MESSAGES=en_US.UTF-8\n",
        );
        let vconsole_path = dir.path().join("vconsole.conf");

        let state = LocaleState::load_from(
            locale_path.to_str().unwrap(),
            vconsole_path.to_str().unwrap(),
        );

        assert_eq!(state.locale.len(), 2);
        assert!(state.locale.contains_key("LANG"));
        assert!(state.locale.contains_key("LC_MESSAGES"));
        assert!(!state.locale.contains_key("FOO"));
    }

    // -- set_locale tests --

    #[test]
    fn test_set_locale_at() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("locale.conf");

        let mut entries = BTreeMap::new();
        entries.insert("LANG".to_string(), "en_US.UTF-8".to_string());
        entries.insert("LC_TIME".to_string(), "de_DE.UTF-8".to_string());

        set_locale_at(&entries, path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("LANG=en_US.UTF-8"));
        assert!(content.contains("LC_TIME=de_DE.UTF-8"));
    }

    #[test]
    fn test_set_locale_filters_non_locale_vars() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("locale.conf");

        let mut entries = BTreeMap::new();
        entries.insert("LANG".to_string(), "C".to_string());
        entries.insert("FOO".to_string(), "bar".to_string());

        set_locale_at(&entries, path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("LANG=C"));
        assert!(!content.contains("FOO"));
    }

    #[test]
    fn test_set_locale_empty_removes_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("locale.conf");
        fs::write(&path, "LANG=C\n").unwrap();
        assert!(path.exists());

        let entries = BTreeMap::new();
        set_locale_at(&entries, path.to_str().unwrap()).unwrap();
        assert!(!path.exists());
    }

    // -- set_vconsole_keymap tests --

    #[test]
    fn test_set_vconsole_keymap_at() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vconsole.conf");

        set_vconsole_keymap_at("us", "de", path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("KEYMAP=us"));
        assert!(content.contains("KEYMAP_TOGGLE=de"));
    }

    #[test]
    fn test_set_vconsole_keymap_preserves_other_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vconsole.conf");
        fs::write(&path, "FONT=ter-v16n\nKEYMAP=de\n").unwrap();

        set_vconsole_keymap_at("us", "", path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("FONT=ter-v16n"));
        assert!(content.contains("KEYMAP=us"));
        assert!(!content.contains("KEYMAP_TOGGLE"));
        assert!(!content.contains("KEYMAP=de"));
    }

    #[test]
    fn test_set_vconsole_keymap_empty_removes_keys() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("vconsole.conf");
        fs::write(&path, "KEYMAP=us\nKEYMAP_TOGGLE=de\nFONT=lat2\n").unwrap(); // pragma: allowlist secret

        set_vconsole_keymap_at("", "", path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("KEYMAP"));
        assert!(content.contains("FONT=lat2"));
    }

    // -- set_x11_keymap tests --

    #[test]
    fn test_set_x11_keymap_at() {
        let dir = TempDir::new().unwrap();
        let vconsole_path = dir.path().join("vconsole.conf");
        let x11_dir = dir.path().join("X11/xorg.conf.d");
        let x11_conf = x11_dir.join("00-keyboard.conf");

        set_x11_keymap_at(
            "us",
            "pc105",
            "intl",
            "ctrl:nocaps",
            vconsole_path.to_str().unwrap(),
            x11_dir.to_str().unwrap(),
            x11_conf.to_str().unwrap(),
        )
        .unwrap();

        // Check vconsole.conf
        let vc_content = fs::read_to_string(&vconsole_path).unwrap();
        assert!(vc_content.contains("X11_LAYOUT=us"));
        assert!(vc_content.contains("X11_MODEL=pc105"));
        assert!(vc_content.contains("X11_VARIANT=intl"));
        assert!(vc_content.contains("X11_OPTIONS=ctrl:nocaps"));

        // Check X11 config
        let x11_content = fs::read_to_string(&x11_conf).unwrap();
        assert!(x11_content.contains("Section \"InputClass\""));
        assert!(x11_content.contains("\"XkbLayout\" \"us\""));
        assert!(x11_content.contains("\"XkbModel\" \"pc105\""));
        assert!(x11_content.contains("\"XkbVariant\" \"intl\""));
        assert!(x11_content.contains("\"XkbOptions\" \"ctrl:nocaps\""));
        assert!(x11_content.contains("EndSection"));
    }

    #[test]
    fn test_set_x11_keymap_layout_only() {
        let dir = TempDir::new().unwrap();
        let vconsole_path = dir.path().join("vconsole.conf");
        let x11_dir = dir.path().join("X11");
        let x11_conf = x11_dir.join("00-keyboard.conf");

        set_x11_keymap_at(
            "de",
            "",
            "",
            "",
            vconsole_path.to_str().unwrap(),
            x11_dir.to_str().unwrap(),
            x11_conf.to_str().unwrap(),
        )
        .unwrap();

        let x11_content = fs::read_to_string(&x11_conf).unwrap();
        assert!(x11_content.contains("\"XkbLayout\" \"de\""));
        assert!(!x11_content.contains("XkbModel"));
        assert!(!x11_content.contains("XkbVariant"));
        assert!(!x11_content.contains("XkbOptions"));
    }

    #[test]
    fn test_set_x11_keymap_empty_removes() {
        let dir = TempDir::new().unwrap();
        let vconsole_path = dir.path().join("vconsole.conf");
        let x11_dir = dir.path().join("X11");
        let x11_conf = x11_dir.join("00-keyboard.conf");

        // Create the files first
        fs::create_dir_all(&x11_dir).unwrap();
        fs::write(&x11_conf, "some content").unwrap();
        fs::write(&vconsole_path, "X11_LAYOUT=us\n").unwrap();

        set_x11_keymap_at(
            "",
            "",
            "",
            "",
            vconsole_path.to_str().unwrap(),
            x11_dir.to_str().unwrap(),
            x11_conf.to_str().unwrap(),
        )
        .unwrap();

        assert!(!x11_conf.exists());
    }

    #[test]
    fn test_set_x11_keymap_preserves_vconsole_keys() {
        let dir = TempDir::new().unwrap();
        let vconsole_path = dir.path().join("vconsole.conf");
        let x11_dir = dir.path().join("X11");
        let x11_conf = x11_dir.join("00-keyboard.conf");

        fs::write(&vconsole_path, "KEYMAP=us\nFONT=lat2\n").unwrap(); // pragma: allowlist secret

        set_x11_keymap_at(
            "de",
            "",
            "",
            "",
            vconsole_path.to_str().unwrap(),
            x11_dir.to_str().unwrap(),
            x11_conf.to_str().unwrap(),
        )
        .unwrap();

        let vc_content = fs::read_to_string(&vconsole_path).unwrap();
        assert!(vc_content.contains("KEYMAP=us"));
        assert!(vc_content.contains("FONT=lat2"));
        assert!(vc_content.contains("X11_LAYOUT=de"));
    }

    // -- format tests --

    #[test]
    fn test_format_status() {
        let mut state = LocaleState::default();
        state
            .locale
            .insert("LANG".to_string(), "en_US.UTF-8".to_string());
        state.vconsole_keymap = "us".to_string();
        state.x11_layout = "us".to_string();

        let output = state.format_status();
        assert!(output.contains("en_US.UTF-8"));
        assert!(output.contains("us"));
        assert!(output.contains("System Locale"));
        assert!(output.contains("VC Keymap"));
        assert!(output.contains("X11 Layout"));
    }

    #[test]
    fn test_format_status_empty() {
        let state = LocaleState::default();
        let output = state.format_status();
        assert!(output.contains("(unset)"));
    }

    #[test]
    fn test_format_show() {
        let mut state = LocaleState::default();
        state
            .locale
            .insert("LANG".to_string(), "C.UTF-8".to_string());
        state.vconsole_keymap = "us".to_string();
        state.x11_layout = "us".to_string();

        let output = state.format_show();
        assert!(output.contains("LANG=C.UTF-8"));
        assert!(output.contains("VConsoleKeymap=us"));
        assert!(output.contains("X11Layout=us"));
    }

    #[test]
    fn test_format_show_multiple_locale_vars() {
        let mut state = LocaleState::default();
        state
            .locale
            .insert("LANG".to_string(), "en_US.UTF-8".to_string());
        state
            .locale
            .insert("LC_TIME".to_string(), "de_DE.UTF-8".to_string());
        state
            .locale
            .insert("LC_MESSAGES".to_string(), "en_US.UTF-8".to_string());

        let output = state.format_show();
        assert!(output.contains("LANG=en_US.UTF-8"));
        assert!(output.contains("LC_TIME=de_DE.UTF-8"));
        assert!(output.contains("LC_MESSAGES=en_US.UTF-8"));
    }

    // -- control command tests --

    #[test]
    fn test_handle_control_status() {
        let response = handle_control_command("STATUS");
        assert!(!response.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_control_show() {
        let response = handle_control_command("SHOW");
        assert!(response.contains("VConsoleKeymap="));
    }

    #[test]
    fn test_handle_control_unknown() {
        let response = handle_control_command("NONSENSE");
        assert!(response.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_control_set_locale_bad_var() {
        let response = handle_control_command("SET-LOCALE BOGUS=xyz");
        assert!(response.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_empty_command() {
        let response = handle_control_command("");
        assert!(response.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_case_insensitive_commands() {
        let r1 = handle_control_command("status");
        let r2 = handle_control_command("STATUS");
        assert!(!r1.starts_with("ERROR"));
        assert!(!r2.starts_with("ERROR"));
    }

    // -- write_env_file roundtrip --

    #[test]
    fn test_write_env_file_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("env");
        let path_str = path.to_str().unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("LANG".to_string(), "en_US.UTF-8".to_string());
        entries.insert("LC_TIME".to_string(), "de_DE.UTF-8".to_string());

        write_env_file(path_str, &entries).unwrap();

        let parsed = parse_env_file(path_str);
        assert_eq!(parsed.get("LANG").unwrap(), "en_US.UTF-8");
        assert_eq!(parsed.get("LC_TIME").unwrap(), "de_DE.UTF-8");
    }

    #[test]
    fn test_write_env_file_sorted() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("env");
        let path_str = path.to_str().unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("ZZZ".to_string(), "last".to_string());
        entries.insert("AAA".to_string(), "first".to_string());

        write_env_file(path_str, &entries).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "AAA=first");
        assert_eq!(lines[1], "ZZZ=last");
    }

    // -- list_keymaps --

    #[test]
    fn test_list_keymaps_does_not_panic() {
        let _ = list_keymaps();
    }

    // -- list_x11_layouts --

    #[test]
    fn test_list_x11_layouts_does_not_panic() {
        let _ = list_x11_layouts();
    }

    // -- collect_keymaps_recursive --

    #[test]
    fn test_collect_keymaps_recursive() {
        let dir = TempDir::new().unwrap();
        let subdir = dir.path().join("i386");
        fs::create_dir_all(&subdir).unwrap();

        fs::write(subdir.join("us.map.gz"), "fake").unwrap();
        fs::write(subdir.join("de.map"), "fake").unwrap();
        fs::write(subdir.join("readme.txt"), "not a keymap").unwrap();

        let mut keymaps = Vec::new();
        collect_keymaps_recursive(dir.path(), &mut keymaps).unwrap();

        assert!(keymaps.contains(&"us".to_string()));
        assert!(keymaps.contains(&"de".to_string()));
        assert!(!keymaps.contains(&"readme.txt".to_string()));
    }

    #[test]
    fn test_collect_keymaps_nonexistent() {
        let mut keymaps = Vec::new();
        let result = collect_keymaps_recursive(Path::new("/nonexistent"), &mut keymaps);
        assert!(result.is_ok());
        assert!(keymaps.is_empty());
    }

    // -- parse_xkb_rules_layouts --

    #[test]
    fn test_parse_xkb_rules_layouts() {
        let dir = TempDir::new().unwrap();
        let path = write_file(
            &dir,
            "base.lst",
            "! model\n  pc104     Generic 104-key PC\n  pc105     Generic 105-key PC\n\n! layout\n  us        English (US)\n  de        German\n  fr        French\n\n! variant\n",
        );

        let layouts = parse_xkb_rules_layouts(path.to_str().unwrap()).unwrap();
        assert_eq!(layouts, vec!["us", "de", "fr"]);
    }

    #[test]
    fn test_parse_xkb_rules_layouts_empty() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "base.lst", "");

        let layouts = parse_xkb_rules_layouts(path.to_str().unwrap()).unwrap();
        assert!(layouts.is_empty());
    }

    #[test]
    fn test_parse_xkb_rules_layouts_no_layout_section() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "base.lst", "! model\n  pc104     Generic\n\n");

        let layouts = parse_xkb_rules_layouts(path.to_str().unwrap()).unwrap();
        assert!(layouts.is_empty());
    }

    // -- x11 keyboard conf generation --

    #[test]
    fn test_write_x11_keyboard_conf_full() {
        let dir = TempDir::new().unwrap();
        let conf_dir = dir.path().join("xorg.conf.d");
        let conf_path = conf_dir.join("00-keyboard.conf");

        write_x11_keyboard_conf(
            "us",
            "pc105",
            "intl",
            "ctrl:nocaps",
            conf_dir.to_str().unwrap(),
            conf_path.to_str().unwrap(),
        )
        .unwrap();

        let content = fs::read_to_string(&conf_path).unwrap();
        assert!(content.contains("systemd-localed"));
        assert!(content.contains("Section \"InputClass\""));
        assert!(content.contains("Identifier \"system-keyboard\""));
        assert!(content.contains("MatchIsKeyboard \"on\""));
        assert!(content.contains("Option \"XkbLayout\" \"us\""));
        assert!(content.contains("Option \"XkbModel\" \"pc105\""));
        assert!(content.contains("Option \"XkbVariant\" \"intl\""));
        assert!(content.contains("Option \"XkbOptions\" \"ctrl:nocaps\""));
        assert!(content.contains("EndSection"));
    }

    #[test]
    fn test_write_x11_keyboard_conf_layout_only() {
        let dir = TempDir::new().unwrap();
        let conf_dir = dir.path().join("xorg.conf.d");
        let conf_path = conf_dir.join("00-keyboard.conf");

        write_x11_keyboard_conf(
            "de",
            "",
            "",
            "",
            conf_dir.to_str().unwrap(),
            conf_path.to_str().unwrap(),
        )
        .unwrap();

        let content = fs::read_to_string(&conf_path).unwrap();
        assert!(content.contains("Option \"XkbLayout\" \"de\""));
        assert!(!content.contains("XkbModel"));
    }

    #[test]
    fn test_write_x11_keyboard_conf_empty_removes() {
        let dir = TempDir::new().unwrap();
        let conf_dir = dir.path().join("xorg.conf.d");
        let conf_path = conf_dir.join("00-keyboard.conf");

        fs::create_dir_all(&conf_dir).unwrap();
        fs::write(&conf_path, "old content").unwrap();

        write_x11_keyboard_conf(
            "",
            "",
            "",
            "",
            conf_dir.to_str().unwrap(),
            conf_path.to_str().unwrap(),
        )
        .unwrap();

        assert!(!conf_path.exists());
    }

    // -- LOCALE_VARIABLES constant --

    #[test]
    fn test_locale_variables_contains_lang() {
        assert!(LOCALE_VARIABLES.contains(&"LANG"));
    }

    #[test]
    fn test_locale_variables_contains_lc_all() {
        assert!(LOCALE_VARIABLES.contains(&"LC_ALL"));
    }

    #[test]
    fn test_locale_variables_count() {
        assert_eq!(LOCALE_VARIABLES.len(), 15);
    }

    // --- D-Bus interface tests ---

    #[test]
    fn test_dbus_locale1_manager_struct() {
        // Verify the interface struct can be created without panic
        let shared: SharedState = Arc::new(Mutex::new(LocaleState::default()));
        let _mgr = Locale1Manager { state: shared };
    }

    #[test]
    fn test_shared_state_reload() {
        let mut locale = BTreeMap::new();
        locale.insert("LANG".to_string(), "C".to_string());
        let state = LocaleState {
            locale,
            ..Default::default()
        };
        let shared: SharedState = Arc::new(Mutex::new(state));

        // Simulate a reload
        {
            let mut s = shared.lock().unwrap();
            s.locale
                .insert("LANG".to_string(), "en_US.UTF-8".to_string());
            s.vconsole_keymap = "us".to_string();
        }

        let s = shared.lock().unwrap();
        assert_eq!(s.locale.get("LANG").unwrap(), "en_US.UTF-8");
        assert_eq!(s.vconsole_keymap, "us");
    }

    #[test]
    fn test_shared_state_x11_update() {
        let state = LocaleState::default();
        let shared: SharedState = Arc::new(Mutex::new(state));

        {
            let mut s = shared.lock().unwrap();
            s.x11_layout = "de".to_string();
            s.x11_model = "pc105".to_string();
            s.x11_variant = "nodeadkeys".to_string();
            s.x11_options = "ctrl:nocaps".to_string();
        }

        let s = shared.lock().unwrap();
        assert_eq!(s.x11_layout, "de");
        assert_eq!(s.x11_model, "pc105");
        assert_eq!(s.x11_variant, "nodeadkeys");
        assert_eq!(s.x11_options, "ctrl:nocaps");
    }
}
