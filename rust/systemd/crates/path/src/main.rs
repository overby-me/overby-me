//! systemd-path — List and query well-known system and user runtime paths.
//!
//! A drop-in replacement for `systemd-path(1)`. When invoked without
//! arguments it prints all known path names and their current values, one
//! per line.  When invoked with one or more path names it prints only the
//! requested paths.
//!
//! Supported options:
//!
//! - `--suffix=SUFFIX` — Append the given suffix to each printed path.
//!
//! The tool mirrors the output format of the real `systemd-path`:
//!
//!     <name>: <path>
//!
//! or, when a single path is queried, just the bare path.

use clap::Parser;
use std::collections::BTreeMap;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-path",
    about = "List and query well-known system and user runtime paths",
    version
)]
struct Cli {
    /// Append SUFFIX to all printed paths.
    #[arg(long, value_name = "SUFFIX")]
    suffix: Option<String>,

    /// Path names to query.  If none are given, all known paths are printed.
    names: Vec<String>,
}

/// Get HOME directory, validated to be an absolute path.
/// If HOME is not set or not absolute, falls back to the passwd entry.
fn get_home() -> String {
    if let Ok(home) = std::env::var("HOME")
        && home.starts_with('/')
    {
        return home;
    }
    // Fallback: query passwd
    let uid = unsafe { libc::getuid() };
    let pw = unsafe { libc::getpwuid(uid) };
    if !pw.is_null() {
        let home_cstr = unsafe { std::ffi::CStr::from_ptr((*pw).pw_dir) };
        if let Ok(s) = home_cstr.to_str() {
            return s.to_string();
        }
    }
    "/root".to_string()
}

/// Get an XDG environment variable, but only if it's an absolute path.
fn get_xdg_var(name: &str) -> Option<String> {
    std::env::var(name).ok().filter(|v| v.starts_with('/'))
}

/// Parse XDG user-dirs.dirs file to get user directory overrides.
/// Format: XDG_DESKTOP_DIR="$HOME/Desktop" or XDG_DESKTOP_DIR="/absolute/path"
fn parse_user_dirs(home: &str) -> BTreeMap<String, String> {
    let mut dirs = BTreeMap::new();
    let config_home = get_xdg_var("XDG_CONFIG_HOME").unwrap_or_else(|| format!("{home}/.config"));
    let path = PathBuf::from(&config_home).join("user-dirs.dirs");

    let file = match std::fs::File::open(&path) {
        Ok(f) => f,
        Err(_) => return dirs,
    };

    for line in io::BufReader::new(file).lines() {
        let line = match line {
            Ok(l) => l,
            Err(_) => continue,
        };
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') {
            continue;
        }

        // Parse KEY="VALUE" format
        let Some(eq_pos) = trimmed.find('=') else {
            continue;
        };
        let key = trimmed[..eq_pos].trim();
        let val_part = trimmed[eq_pos + 1..].trim();

        // Must start and end with double quotes
        if !val_part.starts_with('"') || !val_part.ends_with('"') {
            continue;
        }
        let inner = &val_part[1..val_part.len() - 1];

        // Must be empty or an absolute path (possibly starting with $HOME)
        if inner.is_empty() {
            continue;
        }

        let resolved = if let Some(rest) = inner.strip_prefix("$HOME") {
            if rest.is_empty() {
                home.to_string()
            } else if rest.starts_with('/') {
                format!("{home}{rest}")
            } else {
                continue; // Invalid: not followed by path separator
            }
        } else if inner.starts_with('/') {
            inner.to_string()
        } else {
            continue; // Not absolute
        };

        // Relative paths (containing ..) are rejected
        if resolved.contains("../") {
            continue;
        }

        // Map XDG key names to our path identifiers
        let path_name = match key {
            "XDG_DESKTOP_DIR" => "user-desktop",
            "XDG_DOWNLOAD_DIR" => "user-download",
            "XDG_TEMPLATES_DIR" => "user-templates",
            "XDG_PUBLICSHARE_DIR" => "user-public",
            "XDG_DOCUMENTS_DIR" => "user-documents",
            "XDG_MUSIC_DIR" => "user-music",
            "XDG_PICTURES_DIR" => "user-pictures",
            "XDG_VIDEOS_DIR" => "user-videos",
            _ => continue,
        };

        dirs.insert(path_name.to_string(), resolved);
    }

    dirs
}

/// Build a map of all well-known paths.
fn known_paths() -> BTreeMap<&'static str, PathBuf> {
    let mut m = BTreeMap::new();

    // --- System paths ---
    m.insert(
        "temporary",
        PathBuf::from(std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into())),
    );
    m.insert("temporary-large", PathBuf::from("/var/tmp"));
    m.insert("system-binaries", PathBuf::from("/usr/bin"));
    m.insert("system-include", PathBuf::from("/usr/include"));
    m.insert("system-library-private", PathBuf::from("/usr/lib"));
    m.insert("system-library-arch", PathBuf::from("/usr/lib"));
    m.insert("system-shared", PathBuf::from("/usr/share"));
    m.insert(
        "system-configuration-factory",
        PathBuf::from("/usr/share/factory/etc"),
    );
    m.insert(
        "system-state-factory",
        PathBuf::from("/usr/share/factory/var"),
    );
    m.insert("system-configuration", PathBuf::from("/etc"));
    m.insert("system-runtime", PathBuf::from("/run"));
    m.insert("system-runtime-logs", PathBuf::from("/run/log"));
    m.insert("system-state-private", PathBuf::from("/var/lib"));
    m.insert("system-state-logs", PathBuf::from("/var/log"));
    m.insert("system-state-cache", PathBuf::from("/var/cache"));
    m.insert("system-state-spool", PathBuf::from("/var/spool"));

    // --- Credential paths ---
    m.insert("system-credential-store", PathBuf::from("/etc/credstore"));
    m.insert(
        "system-credential-store-encrypted",
        PathBuf::from("/etc/credstore.encrypted"),
    );

    // --- systemd auxiliary data directories ---
    m.insert("binfmt", PathBuf::from("/usr/lib/binfmt.d"));
    m.insert("sysctl", PathBuf::from("/usr/lib/sysctl.d"));
    m.insert("sysusers", PathBuf::from("/usr/lib/sysusers.d"));
    m.insert("tmpfiles", PathBuf::from("/usr/lib/tmpfiles.d"));
    m.insert("modules-load", PathBuf::from("/usr/lib/modules-load.d"));
    m.insert("catalog", PathBuf::from("/usr/lib/systemd/catalog"));

    // --- systemd search paths ---
    m.insert("systemd-util", PathBuf::from("/usr/lib/systemd"));
    m.insert(
        "system-generator",
        PathBuf::from("/usr/lib/systemd/system-generators"),
    );
    m.insert(
        "system-generator-early",
        PathBuf::from("/run/systemd/system-generators"),
    );
    m.insert(
        "system-generator-late",
        PathBuf::from("/usr/lib/systemd/system-generators"),
    );
    m.insert(
        "user-generator",
        PathBuf::from("/usr/lib/systemd/user-generators"),
    );
    m.insert(
        "user-generator-early",
        PathBuf::from("/run/systemd/user-generators"),
    );
    m.insert(
        "user-generator-late",
        PathBuf::from("/usr/lib/systemd/user-generators"),
    );
    m.insert(
        "systemd-system-unit",
        PathBuf::from("/usr/lib/systemd/system"),
    );
    m.insert("systemd-user-unit", PathBuf::from("/usr/lib/systemd/user"));
    m.insert("systemd-system-conf", PathBuf::from("/etc/systemd/system"));
    m.insert("systemd-user-conf", PathBuf::from("/etc/systemd/user"));
    m.insert(
        "systemd-search-system-unit",
        PathBuf::from("/etc/systemd/system"),
    );
    m.insert(
        "systemd-search-user-unit",
        PathBuf::from("/etc/systemd/user"),
    );
    m.insert(
        "systemd-system-preset",
        PathBuf::from("/usr/lib/systemd/system-preset"),
    );
    m.insert(
        "systemd-user-preset",
        PathBuf::from("/usr/lib/systemd/user-preset"),
    );
    m.insert(
        "systemd-system-environment-generator",
        PathBuf::from("/usr/lib/systemd/system-environment-generators"),
    );
    m.insert(
        "systemd-user-environment-generator",
        PathBuf::from("/usr/lib/systemd/user-environment-generators"),
    );
    m.insert(
        "systemd-system-generator",
        PathBuf::from("/usr/lib/systemd/system-generators"),
    );
    m.insert(
        "systemd-user-generator",
        PathBuf::from("/usr/lib/systemd/user-generators"),
    );
    m.insert(
        "systemd-shutdown",
        PathBuf::from("/usr/lib/systemd/system-shutdown"),
    );
    m.insert(
        "systemd-sleep",
        PathBuf::from("/usr/lib/systemd/system-sleep"),
    );
    m.insert(
        "systemd-initrd-preset",
        PathBuf::from("/usr/lib/systemd/initrd-preset"),
    );

    // --- User paths (XDG) ---
    let home = get_home();
    let home_path = Path::new(&home);

    let xdg_runtime = get_xdg_var("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from(format!("/run/user/{}", unsafe { libc::getuid() })));

    let xdg_config_home = get_xdg_var("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path.join(".config"));

    let xdg_cache_home = get_xdg_var("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path.join(".cache"));

    let xdg_data_home = get_xdg_var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path.join(".local/share"));

    let xdg_state_home = get_xdg_var("XDG_STATE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home_path.join(".local/state"));

    m.insert("user", home_path.to_path_buf());
    m.insert("user-runtime", xdg_runtime.clone());
    m.insert("user-configuration", xdg_config_home.clone());
    m.insert("user-cache", xdg_cache_home);
    m.insert("user-shared", xdg_data_home);
    m.insert("user-state", xdg_state_home);
    m.insert("user-library-private", home_path.join(".local/lib"));
    m.insert("user-binaries", home_path.join(".local/bin"));
    m.insert("user-credential-store", xdg_config_home.join("credstore"));
    m.insert(
        "user-credential-store-encrypted",
        xdg_config_home.join("credstore.encrypted"),
    );

    // User directories: defaults, then overridden by user-dirs.dirs
    // Default: all user dirs except desktop default to $HOME
    m.insert("user-desktop", home_path.join("Desktop"));
    m.insert("user-documents", home_path.to_path_buf());
    m.insert("user-download", home_path.to_path_buf());
    m.insert("user-music", home_path.to_path_buf());
    m.insert("user-pictures", home_path.to_path_buf());
    m.insert("user-videos", home_path.to_path_buf());
    m.insert("user-templates", home_path.to_path_buf());
    m.insert("user-public", home_path.to_path_buf());

    // Override with user-dirs.dirs values
    let user_dirs = parse_user_dirs(&home);
    for (name, path) in &user_dirs {
        match name.as_str() {
            "user-desktop" => {
                m.insert("user-desktop", PathBuf::from(path));
            }
            "user-documents" => {
                m.insert("user-documents", PathBuf::from(path));
            }
            "user-download" => {
                m.insert("user-download", PathBuf::from(path));
            }
            "user-music" => {
                m.insert("user-music", PathBuf::from(path));
            }
            "user-pictures" => {
                m.insert("user-pictures", PathBuf::from(path));
            }
            "user-videos" => {
                m.insert("user-videos", PathBuf::from(path));
            }
            "user-templates" => {
                m.insert("user-templates", PathBuf::from(path));
            }
            "user-public" => {
                m.insert("user-public", PathBuf::from(path));
            }
            _ => {}
        }
    }

    // --- Derived search paths ---
    m.insert(
        "search-binaries",
        PathBuf::from("/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin"),
    );
    m.insert("search-binaries-default", PathBuf::from("/usr/bin/"));
    m.insert(
        "search-library-private",
        PathBuf::from("/usr/local/lib:/usr/lib"),
    );
    m.insert(
        "search-library-arch",
        PathBuf::from("/usr/local/lib:/usr/lib"),
    );
    m.insert(
        "search-shared",
        PathBuf::from("/usr/local/share:/usr/share"),
    );
    m.insert(
        "search-configuration-factory",
        PathBuf::from("/usr/local/share/factory/etc:/usr/share/factory/etc"),
    );
    m.insert(
        "search-state-factory",
        PathBuf::from("/usr/local/share/factory/var:/usr/share/factory/var"),
    );

    // search-configuration: XDG_CONFIG_DIRS, /etc
    let xdg_config_dirs = get_xdg_var("XDG_CONFIG_DIRS").unwrap_or_else(|| "/etc".to_string());
    let search_config = format!("{}:{}", xdg_config_home.display(), xdg_config_dirs);
    m.insert("search-configuration", PathBuf::from(search_config));

    // Search credential stores
    m.insert(
        "system-search-credential-store",
        PathBuf::from("/etc/credstore:/run/credstore:/usr/local/lib/credstore:/usr/lib/credstore"),
    );
    m.insert(
        "system-search-credential-store-encrypted",
        PathBuf::from("/etc/credstore.encrypted:/run/credstore.encrypted:/usr/local/lib/credstore.encrypted:/usr/lib/credstore.encrypted"),
    );

    // Search network
    m.insert(
        "systemd-search-network",
        PathBuf::from("/etc/systemd/network:/run/systemd/network:/usr/local/lib/systemd/network:/usr/lib/systemd/network"),
    );

    // Search generators and environment generators
    m.insert(
        "systemd-search-system-generator",
        PathBuf::from("/run/systemd/system-generators:/etc/systemd/system-generators:/usr/lib/systemd/system-generators"),
    );
    m.insert(
        "systemd-search-user-generator",
        PathBuf::from("/run/systemd/user-generators:/etc/systemd/user-generators:/usr/lib/systemd/user-generators"),
    );
    m.insert(
        "systemd-search-system-environment-generator",
        PathBuf::from("/run/systemd/system-environment-generators:/etc/systemd/system-environment-generators:/usr/lib/systemd/system-environment-generators"),
    );
    m.insert(
        "systemd-search-user-environment-generator",
        PathBuf::from("/run/systemd/user-environment-generators:/etc/systemd/user-environment-generators:/usr/lib/systemd/user-environment-generators"),
    );

    m
}

fn apply_suffix(path: &Path, suffix: Option<&str>) -> PathBuf {
    match suffix {
        Some(s) if !s.is_empty() => {
            let path_str = path.to_string_lossy();
            // For search paths (colon-separated), append suffix to each component
            if path_str.contains(':') {
                let result: String = path_str
                    .split(':')
                    .map(|component| Path::new(component).join(s).to_string_lossy().into_owned())
                    .collect::<Vec<_>>()
                    .join(":");
                PathBuf::from(result)
            } else {
                path.join(s)
            }
        }
        _ => path.to_path_buf(),
    }
}

fn main() {
    let cli = Cli::parse();
    let paths = known_paths();
    let suffix = cli.suffix.as_deref();

    if cli.names.is_empty() {
        // Print all known paths
        for (name, path) in &paths {
            let display_path = apply_suffix(path, suffix);
            println!("{name}: {}", display_path.display());
        }
    } else if cli.names.len() == 1 {
        // Single query: print just the path (no name prefix)
        let name = &cli.names[0];
        match paths.get(name.as_str()) {
            Some(path) => {
                let display_path = apply_suffix(path, suffix);
                println!("{}", display_path.display());
            }
            None => {
                eprintln!("Unknown path: {name}");
                process::exit(1);
            }
        }
    } else {
        // Multiple queries: print name: path for each
        let mut exit_code = 0;
        for name in &cli.names {
            match paths.get(name.as_str()) {
                Some(path) => {
                    let display_path = apply_suffix(path, suffix);
                    println!("{name}: {}", display_path.display());
                }
                None => {
                    eprintln!("Unknown path: {name}");
                    exit_code = 1;
                }
            }
        }
        if exit_code != 0 {
            process::exit(exit_code);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_known_paths_not_empty() {
        let paths = known_paths();
        assert!(!paths.is_empty());
    }

    #[test]
    fn test_known_paths_has_system_configuration() {
        let paths = known_paths();
        assert_eq!(
            paths.get("system-configuration").unwrap().as_os_str(),
            "/etc"
        );
    }

    #[test]
    fn test_known_paths_has_temporary() {
        let paths = known_paths();
        assert!(paths.contains_key("temporary"));
    }

    #[test]
    fn test_known_paths_has_binfmt() {
        let paths = known_paths();
        assert!(paths.contains_key("binfmt"));
    }

    #[test]
    fn test_apply_suffix_none() {
        let p = PathBuf::from("/etc");
        assert_eq!(apply_suffix(&p, None), PathBuf::from("/etc"));
    }

    #[test]
    fn test_apply_suffix_empty() {
        let p = PathBuf::from("/run");
        assert_eq!(apply_suffix(&p, Some("")), PathBuf::from("/run"));
    }

    #[test]
    fn test_apply_suffix_some() {
        let p = PathBuf::from("/etc");
        assert_eq!(
            apply_suffix(&p, Some("systemd")),
            PathBuf::from("/etc/systemd")
        );
    }

    #[test]
    fn test_known_paths_has_user_runtime() {
        let paths = known_paths();
        assert!(paths.contains_key("user-runtime"));
    }

    #[test]
    fn test_known_paths_has_system_runtime() {
        let paths = known_paths();
        assert_eq!(paths.get("system-runtime").unwrap().as_os_str(), "/run");
    }

    #[test]
    fn test_get_home_absolute() {
        // If HOME is set to an absolute path, it should be used
        std::env::set_var("HOME", "/test/home");
        assert_eq!(get_home(), "/test/home");
    }
}
