//! localectl — query and change the system locale and keyboard layout
//!
//! This is a Rust implementation of systemd's `localectl` command. It reads
//! and writes locale/keymap state directly from/to the filesystem:
//! - `/etc/locale.conf` for locale settings (LANG, LC_*, etc.)
//! - `/etc/vconsole.conf` for virtual console keymap and X11 keyboard settings
//! - `/etc/X11/xorg.conf.d/00-keyboard.conf` for X11 keyboard configuration

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const LOCALE_CONF_PATH: &str = "/etc/locale.conf";
const VCONSOLE_CONF_PATH: &str = "/etc/vconsole.conf";
const X11_KEYBOARD_DIR: &str = "/etc/X11/xorg.conf.d";
const X11_KEYBOARD_CONF: &str = "/etc/X11/xorg.conf.d/00-keyboard.conf";

/// Search paths for the kbd-model-map file (console keymap ↔ X11 mapping).
const KBD_MODEL_MAP_PATHS: &[&str] = &[
    "/etc/systemd/kbd-model-map",
    "/run/systemd/kbd-model-map",
    "/usr/share/systemd/kbd-model-map",
    "/usr/lib/systemd/kbd-model-map",
    // NixOS
    "/run/current-system/sw/share/systemd/kbd-model-map",
];

/// Known locale variables that localectl manages.
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

#[derive(Debug, Clone, Default)]
struct LocaleState {
    locale: BTreeMap<String, String>,
    vconsole_keymap: String,
    vconsole_keymap_toggle: String,
    x11_layout: String,
    x11_model: String,
    x11_variant: String,
    x11_options: String,
}

impl LocaleState {
    fn load() -> Self {
        let mut state = Self::default();

        // Load locale
        let locale_entries = parse_env_file(LOCALE_CONF_PATH);
        for var in LOCALE_VARIABLES {
            if let Some(val) = locale_entries.get(*var)
                && !val.is_empty()
            {
                state.locale.insert(var.to_string(), val.clone());
            }
        }

        // Also check environment for LANG if not set in locale.conf
        if !state.locale.contains_key("LANG")
            && let Ok(lang) = env::var("LANG")
            && !lang.is_empty()
        {
            state.locale.insert("LANG".to_string(), lang);
        }

        // Load vconsole keymap and X11 settings
        let vc_entries = parse_env_file(VCONSOLE_CONF_PATH);
        state.vconsole_keymap = vc_entries.get("KEYMAP").cloned().unwrap_or_default();
        state.vconsole_keymap_toggle = vc_entries.get("KEYMAP_TOGGLE").cloned().unwrap_or_default();

        // X11 layout can be stored in vconsole.conf (NixOS does this)
        state.x11_layout = vc_entries.get("X11_LAYOUT").cloned().unwrap_or_default();
        state.x11_model = vc_entries.get("X11_MODEL").cloned().unwrap_or_default();
        state.x11_variant = vc_entries.get("X11_VARIANT").cloned().unwrap_or_default();
        state.x11_options = vc_entries.get("X11_OPTIONS").cloned().unwrap_or_default();

        state
    }

    #[allow(dead_code)]
    fn lang(&self) -> &str {
        self.locale
            .get("LANG")
            .map(|s| s.as_str())
            .unwrap_or("C.UTF-8")
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn parse_env_file(path: &str) -> BTreeMap<String, String> {
    parse_env_file_content(&fs::read_to_string(path).unwrap_or_default())
}

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
            // Unescape
            value = value.replace("\\\"", "\"").replace("\\\\", "\\");
            if !key.is_empty() {
                map.insert(key.to_string(), value);
            }
        }
    }
    map
}

fn write_env_file(path: &str, entries: &BTreeMap<String, String>) -> io::Result<()> {
    if entries.is_empty() {
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

fn set_or_remove(map: &mut BTreeMap<String, String>, key: &str, value: &str) {
    if value.is_empty() {
        map.remove(key);
    } else {
        map.insert(key.to_string(), value.to_string());
    }
}

// ---------------------------------------------------------------------------
// Locale operations
// ---------------------------------------------------------------------------

fn set_locale(entries: &BTreeMap<String, String>) -> io::Result<()> {
    // Only write known locale variables
    let mut filtered = BTreeMap::new();
    for (k, v) in entries {
        if LOCALE_VARIABLES.contains(&k.as_str()) && !v.is_empty() {
            filtered.insert(k.clone(), v.clone());
        }
    }
    write_env_file(LOCALE_CONF_PATH, &filtered)
}

// ---------------------------------------------------------------------------
// Keymap operations
// ---------------------------------------------------------------------------

fn set_vconsole_keymap(keymap: &str, keymap_toggle: &str) -> io::Result<()> {
    let mut entries = parse_env_file(VCONSOLE_CONF_PATH);

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

    write_env_file(VCONSOLE_CONF_PATH, &entries)
}

fn set_x11_keymap(layout: &str, model: &str, variant: &str, options: &str) -> io::Result<()> {
    // Update vconsole.conf with X11_* variables
    let mut entries = parse_env_file(VCONSOLE_CONF_PATH);

    set_or_remove(&mut entries, "X11_LAYOUT", layout);
    set_or_remove(&mut entries, "X11_MODEL", model);
    set_or_remove(&mut entries, "X11_VARIANT", variant);
    set_or_remove(&mut entries, "X11_OPTIONS", options);

    write_env_file(VCONSOLE_CONF_PATH, &entries)?;

    // Write X11 keyboard configuration file
    write_x11_keyboard_conf(layout, model, variant, options)
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
/// ```
#[derive(Debug, Clone, PartialEq)]
struct KbdModelMapEntry {
    /// Console keymap name (e.g. "us", "de-latin1")
    console_keymap: String,
    /// X11 layout (e.g. "us", "de", "ch")
    x11_layout: String,
    /// X11 model (e.g. "pc105", "pc105+inet")
    x11_model: String,
    /// X11 variant (e.g. "nodeadkeys", "dvorak"); empty if `-`
    x11_variant: String,
    /// X11 options (e.g. "terminate:ctrl_alt_bksp"); empty if `-`
    x11_options: String,
}

/// Load the kbd-model-map from the first available system path.
fn load_kbd_model_map() -> Vec<KbdModelMapEntry> {
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
fn load_kbd_model_map_from(path: &str) -> io::Result<Vec<KbdModelMapEntry>> {
    let content = fs::read_to_string(path)?;
    Ok(parse_kbd_model_map(&content))
}

/// Parse kbd-model-map content into entries.
fn parse_kbd_model_map(content: &str) -> Vec<KbdModelMapEntry> {
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
struct X11KeymapResult {
    layout: String,
    model: String,
    variant: String,
    options: String,
}

/// Convert a console keymap to X11 parameters using a pre-loaded map.
fn keymap_to_x11_from(entries: &[KbdModelMapEntry], keymap: &str) -> Option<X11KeymapResult> {
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

/// Convert a console keymap name to X11 keyboard parameters.
///
/// Searches the kbd-model-map for the first entry whose `console_keymap`
/// matches the given keymap name. Returns `None` if no match is found.
fn keymap_to_x11(keymap: &str) -> Option<X11KeymapResult> {
    let entries = load_kbd_model_map();
    keymap_to_x11_from(&entries, keymap)
}

/// Convert X11 parameters to a console keymap using a pre-loaded map.
///
/// Matching strategy (following real systemd's `vconsole-util.c`):
/// 1. Try exact match on layout + variant (ignoring model and options).
/// 2. If no exact match, try matching layout only (with empty variant in the map).
/// 3. For multi-layout entries like "de,us", match the first layout component.
fn x11_to_keymap_from(
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

/// Convert X11 keyboard parameters to a console keymap name.
///
/// Searches the kbd-model-map for the best matching entry.
/// Returns `None` if no match is found.
fn x11_to_keymap(layout: &str, model: &str, variant: &str) -> Option<String> {
    let entries = load_kbd_model_map();
    x11_to_keymap_from(&entries, layout, model, variant)
}

fn write_x11_keyboard_conf(
    layout: &str,
    model: &str,
    variant: &str,
    options: &str,
) -> io::Result<()> {
    // If all settings are empty, remove the config file
    if layout.is_empty() && model.is_empty() && variant.is_empty() && options.is_empty() {
        match fs::remove_file(X11_KEYBOARD_CONF) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        return Ok(());
    }

    // Ensure directory exists
    fs::create_dir_all(X11_KEYBOARD_DIR)?;

    let mut f = fs::File::create(X11_KEYBOARD_CONF)?;
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

// ---------------------------------------------------------------------------
// Keymap listing
// ---------------------------------------------------------------------------

fn list_keymaps() -> Vec<String> {
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

// ---------------------------------------------------------------------------
// X11 layout listing
// ---------------------------------------------------------------------------

/// Parse an XKB rules .lst file and extract entries from the given section.
fn parse_xkb_rules_section(path: &str, section_name: &str) -> Vec<(String, String)> {
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('!') {
            in_section = trimmed.contains(section_name);
            continue;
        }

        if in_section {
            if trimmed.is_empty() {
                break; // End of section
            }
            // Format: "  name    Description"
            let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
            if let Some(name) = parts.first() {
                let desc = parts
                    .get(1)
                    .map(|s| s.trim().to_string())
                    .unwrap_or_default();
                entries.push((name.to_string(), desc));
            }
        }
    }

    entries
}

fn find_xkb_rules_file() -> Option<String> {
    let rules_paths = [
        "/usr/share/X11/xkb/rules/base.lst",
        "/usr/share/X11/xkb/rules/evdev.lst",
        "/run/current-system/sw/share/X11/xkb/rules/base.lst",
        "/run/current-system/sw/share/X11/xkb/rules/evdev.lst",
    ];

    for path in &rules_paths {
        if Path::new(path).exists() {
            return Some(path.to_string());
        }
    }

    None
}

fn list_x11_keymap_layouts() -> Vec<(String, String)> {
    if let Some(path) = find_xkb_rules_file() {
        parse_xkb_rules_section(&path, "layout")
    } else {
        Vec::new()
    }
}

fn list_x11_keymap_models() -> Vec<(String, String)> {
    if let Some(path) = find_xkb_rules_file() {
        parse_xkb_rules_section(&path, "model")
    } else {
        Vec::new()
    }
}

fn list_x11_keymap_variants(layout_filter: Option<&str>) -> Vec<(String, String)> {
    let path = match find_xkb_rules_file() {
        Some(p) => p,
        None => return Vec::new(),
    };

    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    let mut in_section = false;

    for line in content.lines() {
        let trimmed = line.trim();

        if trimmed.starts_with('!') {
            in_section = trimmed.contains("variant");
            continue;
        }

        if in_section {
            if trimmed.is_empty() {
                break;
            }
            // Format: "  variant_name  layout: Description"
            let parts: Vec<&str> = trimmed.splitn(2, char::is_whitespace).collect();
            if let Some(name) = parts.first() {
                let rest = parts.get(1).map(|s| s.trim()).unwrap_or("");
                // Parse "layout: Description" or just "Description"
                if let Some((layout_part, desc)) = rest.split_once(':') {
                    let layout = layout_part.trim();
                    let desc = desc.trim().to_string();
                    if let Some(filter) = layout_filter {
                        if layout == filter {
                            entries.push((name.to_string(), desc));
                        }
                    } else {
                        entries.push((name.to_string(), format!("{}: {}", layout, desc)));
                    }
                } else if layout_filter.is_none() {
                    entries.push((name.to_string(), rest.to_string()));
                }
            }
        }
    }

    entries
}

fn list_x11_keymap_options() -> Vec<(String, String)> {
    if let Some(path) = find_xkb_rules_file() {
        parse_xkb_rules_section(&path, "option")
    } else {
        Vec::new()
    }
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_status() {
    let state = LocaleState::load();
    let label_width = 20;

    // System Locale
    print!("{:>label_width$}:", "System Locale");
    if state.locale.is_empty() {
        println!(" n/a");
    } else {
        let mut first = true;
        for (key, value) in &state.locale {
            if first {
                println!(" {}={}", key, value);
                first = false;
            } else {
                println!("{:>label_width$}  {}={}", "", key, value);
            }
        }
    }

    // VC Keymap
    println!(
        "{:>label_width$}: {}",
        "VC Keymap",
        if state.vconsole_keymap.is_empty() {
            "(unset)"
        } else {
            &state.vconsole_keymap
        }
    );

    if !state.vconsole_keymap_toggle.is_empty() {
        println!(
            "{:>label_width$}: {}",
            "VC Toggle Keymap", state.vconsole_keymap_toggle
        );
    }

    // X11 Layout
    println!(
        "{:>label_width$}: {}",
        "X11 Layout",
        if state.x11_layout.is_empty() {
            "(unset)"
        } else {
            &state.x11_layout
        }
    );

    if !state.x11_model.is_empty() {
        println!("{:>label_width$}: {}", "X11 Model", state.x11_model);
    }

    if !state.x11_variant.is_empty() {
        println!("{:>label_width$}: {}", "X11 Variant", state.x11_variant);
    }

    if !state.x11_options.is_empty() {
        println!("{:>label_width$}: {}", "X11 Options", state.x11_options);
    }
}

fn cmd_show(properties: &[String]) {
    let state = LocaleState::load();

    let all_props: Vec<(&str, String)> = {
        let mut props = Vec::new();
        for var in LOCALE_VARIABLES {
            if let Some(val) = state.locale.get(*var) {
                props.push((*var, val.clone()));
            }
        }
        props.push(("VConsoleKeymap", state.vconsole_keymap.clone()));
        props.push(("VConsoleKeymapToggle", state.vconsole_keymap_toggle.clone()));
        props.push(("X11Layout", state.x11_layout.clone()));
        props.push(("X11Model", state.x11_model.clone()));
        props.push(("X11Variant", state.x11_variant.clone()));
        props.push(("X11Options", state.x11_options.clone()));
        props
    };

    if properties.is_empty() {
        for (key, value) in &all_props {
            println!("{}={}", key, value);
        }
    } else {
        for prop in properties {
            if let Some((_key, value)) = all_props.iter().find(|(k, _)| k == prop) {
                println!("{}={}", prop, value);
            } else {
                // Unknown property — print empty value (matches systemd behavior)
                println!("{}=", prop);
            }
        }
    }
}

fn cmd_set_locale(assignments: &[String]) {
    let mut entries = BTreeMap::new();

    for assignment in assignments {
        if let Some((key, value)) = assignment.split_once('=') {
            if LOCALE_VARIABLES.contains(&key) {
                entries.insert(key.to_string(), value.to_string());
            } else {
                eprintln!("Unknown locale variable: {}", key);
                process::exit(1);
            }
        } else {
            eprintln!(
                "Locale assignment must be in KEY=VALUE format: {}",
                assignment
            );
            process::exit(1);
        }
    }

    if let Err(e) = set_locale(&entries) {
        eprintln!("Failed to set locale: {}", e);
        process::exit(1);
    }
}

fn cmd_set_keymap(keymap: &str, toggle: &str, convert: bool) {
    if let Err(e) = set_vconsole_keymap(keymap, toggle) {
        eprintln!("Failed to set keymap: {}", e);
        process::exit(1);
    }

    if convert {
        if let Some(x11) = keymap_to_x11(keymap) {
            if let Err(e) = set_x11_keymap(&x11.layout, &x11.model, &x11.variant, &x11.options) {
                eprintln!("Warning: Failed to convert keymap to X11: {}", e);
            }
        } else if keymap.is_empty() {
            // Clear X11 settings when keymap is cleared
            let _ = set_x11_keymap("", "", "", "");
        }
    }
}

fn cmd_set_x11_keymap(layout: &str, model: &str, variant: &str, options: &str, convert: bool) {
    if let Err(e) = set_x11_keymap(layout, model, variant, options) {
        eprintln!("Failed to set X11 keymap: {}", e);
        process::exit(1);
    }

    if convert {
        if let Some(km) = x11_to_keymap(layout, model, variant) {
            if let Err(e) = set_vconsole_keymap(&km, "") {
                eprintln!("Warning: Failed to convert X11 to keymap: {}", e);
            }
        } else if layout.is_empty() {
            // Clear keymap when X11 layout is cleared
            let _ = set_vconsole_keymap("", "");
        }
    }
}

fn cmd_list_keymaps() {
    let keymaps = list_keymaps();
    if keymaps.is_empty() {
        eprintln!("Couldn't find any console keymaps.");
        process::exit(1);
    }
    for km in &keymaps {
        println!("{}", km);
    }
}

fn cmd_list_x11(section: &str, layout_filter: Option<&str>) {
    let entries = match section {
        "layouts" => list_x11_keymap_layouts(),
        "models" => list_x11_keymap_models(),
        "variants" => list_x11_keymap_variants(layout_filter),
        "options" => list_x11_keymap_options(),
        _ => {
            eprintln!("Unknown X11 section: {}", section);
            process::exit(1);
        }
    };

    if entries.is_empty() {
        eprintln!("Couldn't find any X11 keymap {}.", section);
        process::exit(1);
    }

    for (name, _desc) in &entries {
        println!("{}", name);
    }
}

// ---------------------------------------------------------------------------
// Usage / help
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!("localectl [OPTIONS] COMMAND ...");
    eprintln!();
    eprintln!("Query or change system locale and keyboard settings.");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  status                          Show current locale/keymap settings (default)");
    eprintln!("  show                            Show properties in machine-readable format");
    eprintln!("  set-locale LOCALE...            Set system locale (e.g. LANG=en_US.UTF-8)");
    eprintln!("  set-keymap MAP [TOGGLEMAP]      Set virtual console keymap");
    eprintln!("  set-x11-keymap LAYOUT [MODEL [VARIANT [OPTIONS]]]");
    eprintln!("                                  Set X11 and virtual console keyboard mappings");
    eprintln!("  list-keymaps                    Show known virtual console keymaps");
    eprintln!("  list-x11-keymap-layouts         Show known X11 keyboard layouts");
    eprintln!("  list-x11-keymap-models          Show known X11 keyboard models");
    eprintln!("  list-x11-keymap-variants [LAYOUT]");
    eprintln!("                                  Show known X11 keyboard variants");
    eprintln!("  list-x11-keymap-options          Show known X11 keyboard options");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  -p, --property=PROP  Show only specified property (with show)");
    eprintln!("  --no-convert         Don't convert keymap to/from X11");
    eprintln!("  --no-ask-password    Do not ask for system passwords");
    eprintln!("  --no-pager           Do not pipe output into a pager");
    eprintln!("  -H, --host=HOST     Operate on remote host (not supported)");
    eprintln!("  -h, --help           Show this help");
    eprintln!();
    eprintln!("See the localectl(1) man page for details.");
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = env::args().collect();

    // Parse flags
    let mut properties: Vec<String> = Vec::new();
    let mut positional: Vec<String> = Vec::new();
    let mut skip_next = false;
    let mut no_convert = false;

    for i in 1..args.len() {
        if skip_next {
            skip_next = false;
            continue;
        }

        let arg = &args[i];
        match arg.as_str() {
            "--no-ask-password" | "--no-pager" => {} // silently accept
            "--no-convert" => {
                no_convert = true;
            }
            "-h" | "--help" | "help" => {
                print_usage();
                return;
            }
            "-H" | "--host" => {
                eprintln!("Remote host operation is not supported.");
                process::exit(1);
            }
            "-p" | "--property" => {
                if i + 1 < args.len() {
                    properties.push(args[i + 1].clone());
                    skip_next = true;
                } else {
                    eprintln!("--property requires a value");
                    process::exit(1);
                }
            }
            other if other.starts_with("--property=") => {
                if let Some(val) = other.strip_prefix("--property=") {
                    properties.push(val.to_string());
                }
            }
            other if other.starts_with("--host=") => {
                eprintln!("Remote host operation is not supported.");
                process::exit(1);
            }
            other if other.starts_with('-') && !other.starts_with("--") && other.len() > 1 => {
                // Handle combined short flags like -pH
                for ch in other[1..].chars() {
                    match ch {
                        'h' => {
                            print_usage();
                            return;
                        }
                        'p' if i + 1 < args.len() => {
                            properties.push(args[i + 1].clone());
                            skip_next = true;
                        }
                        _ => {} // ignore unknown short flags
                    }
                }
            }
            _ => positional.push(arg.clone()),
        }
    }

    if positional.is_empty() {
        cmd_status();
        return;
    }

    let command = positional[0].as_str();
    let rest = &positional[1..];

    match command {
        "status" => {
            cmd_status();
        }
        "show" => {
            cmd_show(&properties);
        }
        "set-locale" => {
            if rest.is_empty() {
                eprintln!(
                    "set-locale requires at least one locale assignment (e.g. LANG=en_US.UTF-8)"
                );
                process::exit(1);
            }
            cmd_set_locale(rest);
        }
        "set-keymap" => {
            if rest.is_empty() {
                eprintln!("set-keymap requires at least one argument");
                process::exit(1);
            }
            let keymap = &rest[0];
            let toggle = if rest.len() > 1 { &rest[1] } else { "" };
            cmd_set_keymap(keymap, toggle, !no_convert);
        }
        "set-x11-keymap" => {
            if rest.is_empty() {
                eprintln!("set-x11-keymap requires at least a layout argument");
                process::exit(1);
            }
            let layout = &rest[0];
            let model = if rest.len() > 1 { &rest[1] } else { "" };
            let variant = if rest.len() > 2 { &rest[2] } else { "" };
            let options = if rest.len() > 3 { &rest[3] } else { "" };
            cmd_set_x11_keymap(layout, model, variant, options, !no_convert);
        }
        "list-keymaps" => {
            cmd_list_keymaps();
        }
        "list-x11-keymap-layouts" => {
            cmd_list_x11("layouts", None);
        }
        "list-x11-keymap-models" => {
            cmd_list_x11("models", None);
        }
        "list-x11-keymap-variants" => {
            let filter = rest.first().map(|s| s.as_str());
            cmd_list_x11("variants", filter);
        }
        "list-x11-keymap-options" => {
            cmd_list_x11("options", None);
        }
        other => {
            eprintln!("Unknown command: {}", other);
            eprintln!();
            print_usage();
            process::exit(1);
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

// Sample kbd-model-map content for testing
#[cfg(test)]
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, name: &str, content: &str) -> std::path::PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    // -- parse_env_file_content --

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

    // -- LOCALE_VARIABLES --

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

    // -- write_env_file --

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
    fn test_write_env_file_empty_removes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("env");
        fs::write(&path, "KEY=val\n").unwrap();
        assert!(path.exists());

        let entries = BTreeMap::new();
        write_env_file(path.to_str().unwrap(), &entries).unwrap();
        assert!(!path.exists());
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

    // -- parse_xkb_rules_section --

    #[test]
    fn test_parse_xkb_rules_section_layout() {
        let dir = TempDir::new().unwrap();
        let path = write_file(
            &dir,
            "base.lst",
            "! model\n  pc104     Generic 104-key PC\n  pc105     Generic 105-key PC\n\n! layout\n  us        English (US)\n  de        German\n  fr        French\n\n! variant\n",
        );

        let entries = parse_xkb_rules_section(path.to_str().unwrap(), "layout");
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].0, "us");
        assert_eq!(entries[0].1, "English (US)");
        assert_eq!(entries[1].0, "de");
        assert_eq!(entries[2].0, "fr");
    }

    #[test]
    fn test_parse_xkb_rules_section_model() {
        let dir = TempDir::new().unwrap();
        let path = write_file(
            &dir,
            "base.lst",
            "! model\n  pc104     Generic 104-key PC\n  pc105     Generic 105-key PC\n\n! layout\n  us        English (US)\n\n",
        );

        let entries = parse_xkb_rules_section(path.to_str().unwrap(), "model");
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].0, "pc104");
        assert_eq!(entries[1].0, "pc105");
    }

    #[test]
    fn test_parse_xkb_rules_section_empty() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "base.lst", "");

        let entries = parse_xkb_rules_section(path.to_str().unwrap(), "layout");
        assert!(entries.is_empty());
    }

    #[test]
    fn test_parse_xkb_rules_section_nonexistent() {
        let entries = parse_xkb_rules_section("/nonexistent/base.lst", "layout");
        assert!(entries.is_empty());
    }

    // -- set_or_remove --

    #[test]
    fn test_set_or_remove_set() {
        let mut map = BTreeMap::new();
        set_or_remove(&mut map, "KEY", "value");
        assert_eq!(map.get("KEY").unwrap(), "value");
    }

    #[test]
    fn test_set_or_remove_remove() {
        let mut map = BTreeMap::new();
        map.insert("KEY".to_string(), "value".to_string());
        set_or_remove(&mut map, "KEY", "");
        assert!(!map.contains_key("KEY"));
    }

    // -- LocaleState --

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

    // -- list functions don't panic --

    #[test]
    fn test_list_keymaps_does_not_panic() {
        let _ = list_keymaps();
    }

    #[test]
    fn test_list_x11_keymap_layouts_does_not_panic() {
        let _ = list_x11_keymap_layouts();
    }

    #[test]
    fn test_list_x11_keymap_models_does_not_panic() {
        let _ = list_x11_keymap_models();
    }

    #[test]
    fn test_list_x11_keymap_variants_does_not_panic() {
        let _ = list_x11_keymap_variants(None);
    }

    #[test]
    fn test_list_x11_keymap_variants_with_filter_does_not_panic() {
        let _ = list_x11_keymap_variants(Some("us"));
    }

    #[test]
    fn test_list_x11_keymap_options_does_not_panic() {
        let _ = list_x11_keymap_options();
    }

    // -- X11 keyboard conf generation --

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

    #[test]
    fn test_write_x11_keyboard_conf_full() {
        let dir = TempDir::new().unwrap();
        let conf_dir = dir.path().join("xorg.conf.d");
        let conf_path = conf_dir.join("00-keyboard.conf");
        // Test using the _at variant via set_x11_keymap_at logic embedded here
        fs::create_dir_all(&conf_dir).unwrap();

        let mut f = fs::File::create(&conf_path).unwrap();
        writeln!(f, "# Written by systemd-localed(8), do not edit manually.").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "Section \"InputClass\"").unwrap();
        writeln!(f, "        Identifier \"system-keyboard\"").unwrap();
        writeln!(f, "        MatchIsKeyboard \"on\"").unwrap();
        writeln!(f, "        Option \"XkbLayout\" \"us\"").unwrap();
        writeln!(f, "        Option \"XkbModel\" \"pc105\"").unwrap();
        writeln!(f, "EndSection").unwrap();
        drop(f);

        let content = fs::read_to_string(&conf_path).unwrap();
        assert!(content.contains("Section \"InputClass\""));
        assert!(content.contains("\"XkbLayout\" \"us\""));
        assert!(content.contains("\"XkbModel\" \"pc105\""));
        assert!(content.contains("EndSection"));
    }
}
