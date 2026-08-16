//! systemd-vconsole-setup — Configure virtual console font and keymap
//!
//! Reads configuration from /etc/vconsole.conf and applies the configured
//! keyboard layout and console font to virtual consoles. This is a drop-in
//! replacement for systemd-vconsole-setup.
//!
//! Configuration file format (/etc/vconsole.conf):
//!   KEYMAP=us
//!   KEYMAP_TOGGLE=
//!   FONT=latarcyrheb-sun16
//!   FONT_MAP=
//!   FONT_UNIMAP=
//!
//! The tool applies the keymap using loadkeys(1) and the font using setfont(1).
//! If called with a TTY device argument, it applies settings only to that TTY.
//! Otherwise it applies to the current console.

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Default configuration file path.
const VCONSOLE_CONF: &str = "/etc/vconsole.conf";

/// Paths to search for loadkeys.
const LOADKEYS_PATHS: &[&str] = &["/usr/bin/loadkeys", "/bin/loadkeys", "/usr/sbin/loadkeys"];

/// Paths to search for setfont.
const SETFONT_PATHS: &[&str] = &["/usr/bin/setfont", "/bin/setfont", "/usr/sbin/setfont"];

/// systemd-vconsole-setup — Configure virtual console font and keymap
#[derive(Parser, Debug)]
#[command(name = "systemd-vconsole-setup", version, about)]
struct Cli {
    /// TTY device to configure (e.g. /dev/tty1). If omitted, configures the
    /// current console.
    tty: Option<String>,
}

/// Configuration parsed from /etc/vconsole.conf.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct VconsoleConfig {
    /// Console keyboard mapping (e.g. "us", "de", "uk")
    keymap: Option<String>,
    /// Toggle keymap for switching between two layouts
    keymap_toggle: Option<String>,
    /// Console font name (e.g. "latarcyrheb-sun16", "Lat2-Terminus16")
    font: Option<String>,
    /// Console font map file
    font_map: Option<String>,
    /// Console font unicode map file
    font_unimap: Option<String>,
}

impl VconsoleConfig {
    /// Load configuration from /etc/vconsole.conf.
    fn load() -> Self {
        Self::load_from(Path::new(VCONSOLE_CONF))
    }

    /// Load configuration from a specific path.
    fn load_from(path: &Path) -> Self {
        let mut config = VconsoleConfig::default();

        let file = match fs::File::open(path) {
            Ok(f) => f,
            Err(e) => {
                if e.kind() != io::ErrorKind::NotFound {
                    eprintln!(
                        "systemd-vconsole-setup: Failed to read {}: {}",
                        path.display(),
                        e
                    );
                }
                return config;
            }
        };

        let reader = io::BufReader::new(file);
        let mut vars: HashMap<String, String> = HashMap::new();

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let trimmed = line.trim();

            // Skip empty lines and comments
            if trimmed.is_empty() || trimmed.starts_with('#') {
                continue;
            }

            // Parse KEY=VALUE or KEY="VALUE" or KEY='VALUE'
            if let Some(pos) = trimmed.find('=') {
                let key = trimmed[..pos].trim().to_string();
                let mut value = trimmed[pos + 1..].trim().to_string();

                // Strip surrounding quotes
                if ((value.starts_with('"') && value.ends_with('"'))
                    || (value.starts_with('\'') && value.ends_with('\'')))
                    && value.len() >= 2
                {
                    value = value[1..value.len() - 1].to_string();
                }

                if !value.is_empty() {
                    vars.insert(key, value);
                }
            }
        }

        config.keymap = vars.remove("KEYMAP");
        config.keymap_toggle = vars.remove("KEYMAP_TOGGLE");
        config.font = vars.remove("FONT");
        config.font_map = vars.remove("FONT_MAP");
        config.font_unimap = vars.remove("FONT_UNIMAP");

        config
    }

    /// Check if any configuration is set.
    fn has_keymap(&self) -> bool {
        self.keymap.is_some()
    }

    fn has_font(&self) -> bool {
        self.font.is_some()
    }
}

/// Find an executable in the given list of paths, falling back to PATH lookup.
fn find_executable(paths: &[&str], name: &str) -> Option<PathBuf> {
    // First, check explicit paths
    for path in paths {
        let p = Path::new(path);
        if p.exists() {
            return Some(p.to_path_buf());
        }
    }

    // Fall back to searching PATH via `which`-style lookup
    if let Ok(output) = Command::new("which").arg(name).output()
        && output.status.success()
    {
        let path_str = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if !path_str.is_empty() {
            return Some(PathBuf::from(path_str));
        }
    }

    None
}

/// Apply the keyboard mapping using loadkeys.
fn apply_keymap(config: &VconsoleConfig, tty: Option<&str>, verbose: bool) -> bool {
    let keymap = match &config.keymap {
        Some(k) => k,
        None => return true,
    };

    let loadkeys = match find_executable(LOADKEYS_PATHS, "loadkeys") {
        Some(p) => p,
        None => {
            eprintln!(
                "systemd-vconsole-setup: loadkeys not found, cannot set keymap '{}'.",
                keymap
            );
            return false;
        }
    };

    if verbose {
        eprintln!(
            "systemd-vconsole-setup: Setting keymap to '{}' using {}",
            keymap,
            loadkeys.display()
        );
    }

    let mut cmd = Command::new(&loadkeys);

    // loadkeys -q for quiet operation
    cmd.arg("-q");

    // If a TTY is specified, use -C to target that console
    if let Some(tty_dev) = tty {
        cmd.arg("-C");
        cmd.arg(tty_dev);
    }

    // Add the keymap
    cmd.arg(keymap);

    // If there's a toggle keymap, add it too
    if let Some(toggle) = &config.keymap_toggle {
        cmd.arg(toggle);
    }

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                if verbose {
                    eprintln!(
                        "systemd-vconsole-setup: Successfully set keymap '{}'.",
                        keymap
                    );
                }
                true
            } else {
                eprintln!(
                    "systemd-vconsole-setup: loadkeys failed with exit code {} for keymap '{}'.",
                    status.code().unwrap_or(-1),
                    keymap
                );
                false
            }
        }
        Err(e) => {
            eprintln!("systemd-vconsole-setup: Failed to execute loadkeys: {}", e);
            false
        }
    }
}

/// Apply the console font using setfont.
fn apply_font(config: &VconsoleConfig, tty: Option<&str>, verbose: bool) -> bool {
    let font = match &config.font {
        Some(f) => f,
        None => return true,
    };

    let setfont = match find_executable(SETFONT_PATHS, "setfont") {
        Some(p) => p,
        None => {
            eprintln!(
                "systemd-vconsole-setup: setfont not found, cannot set font '{}'.",
                font
            );
            return false;
        }
    };

    if verbose {
        eprintln!(
            "systemd-vconsole-setup: Setting font to '{}' using {}",
            font,
            setfont.display()
        );
    }

    let mut cmd = Command::new(&setfont);

    // The font name
    cmd.arg(font);

    // If a TTY is specified, use -C to target that console
    if let Some(tty_dev) = tty {
        cmd.arg("-C");
        cmd.arg(tty_dev);
    }

    // Add font map if specified
    if let Some(font_map) = &config.font_map {
        cmd.arg("-m");
        cmd.arg(font_map);
    }

    // Add unicode map if specified
    if let Some(font_unimap) = &config.font_unimap {
        cmd.arg("-u");
        cmd.arg(font_unimap);
    }

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                if verbose {
                    eprintln!("systemd-vconsole-setup: Successfully set font '{}'.", font);
                }
                true
            } else {
                eprintln!(
                    "systemd-vconsole-setup: setfont failed with exit code {} for font '{}'.",
                    status.code().unwrap_or(-1),
                    font
                );
                false
            }
        }
        Err(e) => {
            eprintln!("systemd-vconsole-setup: Failed to execute setfont: {}", e);
            false
        }
    }
}

/// Get a list of all active VT (virtual terminal) devices.
fn get_active_vts() -> Vec<String> {
    let mut vts = Vec::new();

    // Check /sys/class/tty for active virtual terminals
    let tty_dir = Path::new("/sys/class/tty");
    if let Ok(entries) = fs::read_dir(tty_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();

            // Virtual terminals are named tty1, tty2, ..., tty63
            if name_str.starts_with("tty")
                && let Some(num_str) = name_str.strip_prefix("tty")
                && let Ok(num) = num_str.parse::<u32>()
                && (1..=63).contains(&num)
            {
                vts.push(format!("/dev/{}", name_str));
            }
        }
    }

    vts.sort();
    vts
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    let config = VconsoleConfig::load();

    if !config.has_keymap() && !config.has_font() {
        if verbose {
            eprintln!(
                "systemd-vconsole-setup: No keymap or font configured in {}.",
                VCONSOLE_CONF
            );
        }
        return EXIT_SUCCESS;
    }

    if verbose {
        eprintln!(
            "systemd-vconsole-setup: Configuration loaded from {}:",
            VCONSOLE_CONF
        );
        if let Some(ref k) = config.keymap {
            eprintln!("  KEYMAP={}", k);
        }
        if let Some(ref k) = config.keymap_toggle {
            eprintln!("  KEYMAP_TOGGLE={}", k);
        }
        if let Some(ref f) = config.font {
            eprintln!("  FONT={}", f);
        }
        if let Some(ref m) = config.font_map {
            eprintln!("  FONT_MAP={}", m);
        }
        if let Some(ref u) = config.font_unimap {
            eprintln!("  FONT_UNIMAP={}", u);
        }
    }

    let tty = cli.tty.as_deref();
    let mut any_failed = false;

    // Apply keymap (only needs to be done once, not per-TTY, since loadkeys
    // affects the kernel console driver globally — unless a specific TTY is given)
    if config.has_keymap() && !apply_keymap(&config, tty, verbose) {
        any_failed = true;
    }

    // Apply font
    if config.has_font() {
        if let Some(tty_dev) = tty {
            // Apply to specified TTY only
            if !apply_font(&config, Some(tty_dev), verbose) {
                any_failed = true;
            }
        } else {
            // Apply to all active VTs
            let vts = get_active_vts();
            if vts.is_empty() {
                // No VTs found, try without specifying a TTY
                if !apply_font(&config, None, verbose) {
                    any_failed = true;
                }
            } else {
                for vt in &vts {
                    if !apply_font(&config, Some(vt), verbose) {
                        // Font application failure on individual VTs is not fatal;
                        // the VT might not be allocatable
                        if verbose {
                            eprintln!(
                                "systemd-vconsole-setup: Warning: failed to set font on {}",
                                vt
                            );
                        }
                    }
                }
            }
        }
    }

    if any_failed {
        EXIT_FAILURE
    } else {
        EXIT_SUCCESS
    }
}

fn main() -> ExitCode {
    ExitCode::from(run())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn test_parse_config_basic() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-basic");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "KEYMAP=us").unwrap();
        writeln!(f, "FONT=latarcyrheb-sun16").unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        assert_eq!(config.keymap.as_deref(), Some("us"));
        assert_eq!(config.font.as_deref(), Some("latarcyrheb-sun16"));
        assert!(config.keymap_toggle.is_none());
        assert!(config.font_map.is_none());
        assert!(config.font_unimap.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_quoted_values() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-quoted");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "KEYMAP=\"de-latin1\"").unwrap();
        writeln!(f, "FONT='Lat2-Terminus16'").unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        assert_eq!(config.keymap.as_deref(), Some("de-latin1"));
        assert_eq!(config.font.as_deref(), Some("Lat2-Terminus16"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_with_comments() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-comments");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# This is a comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "KEYMAP=uk").unwrap();
        writeln!(f, "# FONT=should-be-ignored").unwrap();
        writeln!(f, "FONT=ter-v16n").unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        assert_eq!(config.keymap.as_deref(), Some("uk"));
        assert_eq!(config.font.as_deref(), Some("ter-v16n"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_all_fields() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-all");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "KEYMAP=us").unwrap();
        writeln!(f, "KEYMAP_TOGGLE=ru").unwrap();
        writeln!(f, "FONT=latarcyrheb-sun16").unwrap();
        writeln!(f, "FONT_MAP=8859-1").unwrap();
        writeln!(f, "FONT_UNIMAP=lat1u.uni").unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        assert_eq!(config.keymap.as_deref(), Some("us"));
        assert_eq!(config.keymap_toggle.as_deref(), Some("ru"));
        assert_eq!(config.font.as_deref(), Some("latarcyrheb-sun16"));
        assert_eq!(config.font_map.as_deref(), Some("8859-1"));
        assert_eq!(config.font_unimap.as_deref(), Some("lat1u.uni"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_empty_file() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-empty");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# Only comments here").unwrap();
        writeln!(f).unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        assert!(config.keymap.is_none());
        assert!(config.font.is_none());
        assert!(!config.has_keymap());
        assert!(!config.has_font());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_missing_file() {
        let config = VconsoleConfig::load_from(Path::new("/nonexistent/vconsole.conf"));
        assert!(config.keymap.is_none());
        assert!(config.font.is_none());
    }

    #[test]
    fn test_parse_config_spaces_around_equals() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-spaces");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "KEYMAP = us").unwrap();
        writeln!(f, "FONT = Lat2-Terminus16").unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        assert_eq!(config.keymap.as_deref(), Some("us"));
        assert_eq!(config.font.as_deref(), Some("Lat2-Terminus16"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_has_keymap_and_font() {
        let mut config = VconsoleConfig::default();
        assert!(!config.has_keymap());
        assert!(!config.has_font());

        config.keymap = Some("us".to_string());
        assert!(config.has_keymap());
        assert!(!config.has_font());

        config.font = Some("default8x16".to_string());
        assert!(config.has_keymap());
        assert!(config.has_font());
    }

    #[test]
    fn test_default_config() {
        let config = VconsoleConfig::default();
        assert!(config.keymap.is_none());
        assert!(config.keymap_toggle.is_none());
        assert!(config.font.is_none());
        assert!(config.font_map.is_none());
        assert!(config.font_unimap.is_none());
    }

    #[test]
    fn test_parse_config_empty_values_ignored() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-emptyvals");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "KEYMAP=").unwrap();
        writeln!(f, "FONT=").unwrap();
        writeln!(f, "KEYMAP_TOGGLE=\"\"").unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        // Empty values should not be set
        assert!(config.keymap.is_none());
        assert!(config.font.is_none());
        assert!(config.keymap_toggle.is_none());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_last_value_wins() {
        let dir = std::env::temp_dir().join("systemd-vconsole-test-override");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("vconsole.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "KEYMAP=us").unwrap();
        writeln!(f, "KEYMAP=de").unwrap();
        drop(f);

        let config = VconsoleConfig::load_from(&path);
        assert_eq!(config.keymap.as_deref(), Some("de"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_find_executable_nonexistent() {
        let result = find_executable(
            &["/nonexistent/path/xyz_no_such_bin"],
            "xyz_no_such_bin_99999",
        );
        assert!(result.is_none());
    }

    #[test]
    fn test_get_active_vts_no_crash() {
        // Just ensure this doesn't panic
        let _vts = get_active_vts();
    }
}
