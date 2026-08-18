//! systemd-binfmt — Register binary formats via binfmt_misc from static configuration
//!
//! Reads configuration from binfmt.d/*.conf files and registers binary formats
//! with the kernel's binfmt_misc facility. This is a drop-in replacement for
//! systemd-binfmt.
//!
//! Configuration is read from (in order of priority):
//!   /etc/binfmt.d/*.conf
//!   /run/binfmt.d/*.conf
//!   /usr/lib/binfmt.d/*.conf
//!   /lib/binfmt.d/*.conf
//!
//! Each .conf file contains binfmt_misc registration strings, one per line.
//! Lines beginning with '#' or ';' are comments. Empty lines are ignored.
//!
//! The format of each registration line is:
//!   :name:type:offset:magic:mask:interpreter:flags
//!
//! See Documentation/admin-guide/binfmt-misc.rst in the Linux kernel source
//! for the full specification.

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Directories to search for binfmt.d configuration, in priority order.
/// Earlier directories take precedence when the same filename exists in multiple.
const CONFIG_DIRS: &[&str] = &[
    "/etc/binfmt.d",
    "/run/binfmt.d",
    "/usr/lib/binfmt.d",
    "/lib/binfmt.d",
];

/// Path to the binfmt_misc register file.
const BINFMT_REGISTER: &str = "/proc/sys/fs/binfmt_misc/register";

/// Path to the binfmt_misc status file.
const BINFMT_STATUS: &str = "/proc/sys/fs/binfmt_misc/status";

/// Path to the binfmt_misc directory (for checking existing registrations).
const BINFMT_DIR: &str = "/proc/sys/fs/binfmt_misc";

/// systemd-binfmt — Register binary formats via binfmt_misc from static configuration
#[derive(Parser, Debug)]
#[command(name = "systemd-binfmt", version, about)]
struct Cli {
    /// Unregister all binary formats before registering new ones
    #[arg(long)]
    unregister: bool,

    /// Specific binfmt.d config files to read (instead of scanning directories)
    files: Vec<PathBuf>,
}

/// A binfmt_misc registration entry parsed from configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct BinfmtEntry {
    /// The raw registration string (e.g. ":name:type:offset:magic:mask:interpreter:flags")
    raw: String,
    /// The name extracted from the registration string (first field after leading ':')
    name: String,
    /// Source file for diagnostic messages
    source: PathBuf,
    /// Line number in source file
    line_number: usize,
}

impl BinfmtEntry {
    /// Parse a binfmt_misc registration string.
    /// Format: :name:type:offset:magic:mask:interpreter:flags
    fn parse(raw: &str, source: &Path, line_number: usize) -> Option<Self> {
        let trimmed = raw.trim();

        if trimmed.is_empty() {
            return None;
        }

        // The registration string must start with ':'
        if !trimmed.starts_with(':') {
            eprintln!(
                "systemd-binfmt: {}:{}: registration string must start with ':', ignoring: {}",
                source.display(),
                line_number,
                trimmed
            );
            return None;
        }

        // Extract the name (second field, between first and second ':')
        let parts: Vec<&str> = trimmed.splitn(3, ':').collect();
        if parts.len() < 3 {
            eprintln!(
                "systemd-binfmt: {}:{}: malformed registration string, ignoring: {}",
                source.display(),
                line_number,
                trimmed
            );
            return None;
        }

        let name = parts[1].to_string();
        if name.is_empty() {
            eprintln!(
                "systemd-binfmt: {}:{}: empty name in registration string, ignoring: {}",
                source.display(),
                line_number,
                trimmed
            );
            return None;
        }

        // Validate field count: :name:type:offset:magic:mask:interpreter:flags
        // That's 7 colons separating 7 fields (with leading empty field before first colon)
        let colon_count = trimmed.chars().filter(|&c| c == ':').count();
        if colon_count < 6 {
            eprintln!(
                "systemd-binfmt: {}:{}: registration string has too few fields (expected at least 7 colon-separated fields), ignoring: {}",
                source.display(),
                line_number,
                trimmed
            );
            return None;
        }

        Some(BinfmtEntry {
            raw: trimmed.to_string(),
            name,
            source: source.to_path_buf(),
            line_number,
        })
    }
}

/// Discover all .conf files across the config directories, respecting priority.
/// Files in earlier directories shadow files with the same name in later directories.
fn discover_config_files() -> Vec<PathBuf> {
    let mut seen_names: BTreeSet<String> = BTreeSet::new();
    let mut result = Vec::new();

    for dir in CONFIG_DIRS {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let mut entries: Vec<PathBuf> = match fs::read_dir(dir_path) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|ext| ext == "conf").unwrap_or(false))
                .collect(),
            Err(e) => {
                eprintln!("systemd-binfmt: Failed to read directory {}: {}", dir, e);
                continue;
            }
        };
        entries.sort();

        for path in entries {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let name = file_name.to_string();
                if seen_names.contains(&name) {
                    // Shadowed by a higher-priority directory
                    continue;
                }
                seen_names.insert(name);
                result.push(path);
            }
        }
    }

    result
}

/// Parse a single binfmt.d config file and return binfmt entries.
fn parse_config_file(path: &Path) -> io::Result<Vec<BinfmtEntry>> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut entries = Vec::new();

    for (line_idx, line) in reader.lines().enumerate() {
        let line = line?;
        let line_number = line_idx + 1;
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        if let Some(entry) = BinfmtEntry::parse(trimmed, path, line_number) {
            entries.push(entry);
        }
    }

    Ok(entries)
}

/// Check if binfmt_misc is available (mounted and status file exists).
fn is_binfmt_misc_available() -> bool {
    Path::new(BINFMT_STATUS).exists()
}

/// Check if a binfmt with the given name is already registered.
fn is_binfmt_registered(name: &str) -> bool {
    let entry_path = PathBuf::from(BINFMT_DIR).join(name);
    entry_path.exists()
}

/// Unregister a binfmt entry by writing -1 to its control file.
fn unregister_binfmt(name: &str) -> io::Result<()> {
    let entry_path = PathBuf::from(BINFMT_DIR).join(name);
    if entry_path.exists() {
        fs::write(&entry_path, "-1\n")?;
    }
    Ok(())
}

/// Unregister all currently registered binfmt entries.
fn unregister_all(verbose: bool) -> bool {
    let binfmt_dir = Path::new(BINFMT_DIR);
    if !binfmt_dir.is_dir() {
        return true;
    }

    let entries = match fs::read_dir(binfmt_dir) {
        Ok(rd) => rd,
        Err(e) => {
            eprintln!("systemd-binfmt: Failed to read {}: {}", BINFMT_DIR, e);
            return false;
        }
    };

    let mut all_ok = true;
    for entry in entries.filter_map(|e| e.ok()) {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        // Skip the special "register" and "status" files
        if name_str == "register" || name_str == "status" {
            continue;
        }

        if verbose {
            eprintln!("systemd-binfmt: Unregistering '{}'...", name_str);
        }

        if let Err(e) = unregister_binfmt(&name_str) {
            eprintln!("systemd-binfmt: Failed to unregister '{}': {}", name_str, e);
            all_ok = false;
        }
    }

    all_ok
}

/// Register a binfmt entry by writing the registration string to the register file.
fn register_binfmt(entry: &BinfmtEntry, verbose: bool) -> bool {
    // If already registered, unregister first so we can re-register with
    // potentially updated settings
    if is_binfmt_registered(&entry.name) {
        if verbose {
            eprintln!(
                "systemd-binfmt: '{}' already registered, re-registering...",
                entry.name
            );
        }
        if let Err(e) = unregister_binfmt(&entry.name) {
            eprintln!(
                "systemd-binfmt: Failed to unregister '{}' for re-registration: {}",
                entry.name, e
            );
            return false;
        }
    }

    if verbose {
        eprintln!(
            "systemd-binfmt: Registering '{}' from {}:{}",
            entry.name,
            entry.source.display(),
            entry.line_number
        );
    }

    match fs::write(BINFMT_REGISTER, &entry.raw) {
        Ok(()) => {
            if verbose {
                eprintln!("systemd-binfmt: Successfully registered '{}'.", entry.name);
            }
            true
        }
        Err(e) => {
            eprintln!(
                "systemd-binfmt: {}:{}: Failed to register '{}': {}",
                entry.source.display(),
                entry.line_number,
                entry.name,
                e
            );
            false
        }
    }
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    // Check that binfmt_misc is available
    if !is_binfmt_misc_available() {
        eprintln!(
            "systemd-binfmt: binfmt_misc is not available ({}  not found). \
             Is binfmt_misc mounted?",
            BINFMT_STATUS
        );
        // If only unregistering, that's fine — nothing to do
        if cli.unregister {
            return EXIT_SUCCESS;
        }
        return EXIT_FAILURE;
    }

    // Handle --unregister: remove all existing registrations
    if cli.unregister {
        if verbose {
            eprintln!("systemd-binfmt: Unregistering all binfmt entries...");
        }
        if unregister_all(verbose) {
            return EXIT_SUCCESS;
        } else {
            return EXIT_FAILURE;
        }
    }

    // Collect config files to read
    let config_files = if !cli.files.is_empty() {
        cli.files.clone()
    } else {
        discover_config_files()
    };

    if verbose {
        eprintln!(
            "systemd-binfmt: Found {} configuration file(s).",
            config_files.len()
        );
    }

    // Parse all files. Later entries for the same name override earlier ones
    // (last writer wins), matching systemd behavior.
    let mut registrations: BTreeMap<String, BinfmtEntry> = BTreeMap::new();

    for path in &config_files {
        match parse_config_file(path) {
            Ok(entries) => {
                if verbose {
                    eprintln!(
                        "systemd-binfmt: Read {} entry/entries from {}",
                        entries.len(),
                        path.display()
                    );
                }
                for entry in entries {
                    registrations.insert(entry.name.clone(), entry);
                }
            }
            Err(e) => {
                eprintln!("systemd-binfmt: Failed to read {}: {}", path.display(), e);
            }
        }
    }

    if registrations.is_empty() {
        if verbose {
            eprintln!("systemd-binfmt: No binary formats to register.");
        }
        return EXIT_SUCCESS;
    }

    if verbose {
        eprintln!(
            "systemd-binfmt: Registering {} binary format(s)...",
            registrations.len()
        );
    }

    let mut any_failed = false;
    for entry in registrations.values() {
        if !register_binfmt(entry, verbose) {
            any_failed = true;
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
    fn test_parse_entry_basic() {
        let path = PathBuf::from("test.conf");
        let raw = ":qemu-arm:M::magic:mask:/usr/bin/qemu-arm:OC";
        let entry = BinfmtEntry::parse(raw, &path, 1).unwrap();
        assert_eq!(entry.name, "qemu-arm");
        assert_eq!(entry.raw, raw);
        assert_eq!(entry.line_number, 1);
    }

    #[test]
    fn test_parse_entry_no_leading_colon() {
        let path = PathBuf::from("test.conf");
        let raw = "qemu-arm:M::magic:mask:/usr/bin/qemu-arm:OC";
        let entry = BinfmtEntry::parse(raw, &path, 1);
        assert!(entry.is_none());
    }

    #[test]
    fn test_parse_entry_empty_name() {
        let path = PathBuf::from("test.conf");
        let raw = "::M::magic:mask:/usr/bin/qemu-arm:OC";
        let entry = BinfmtEntry::parse(raw, &path, 1);
        assert!(entry.is_none());
    }

    #[test]
    fn test_parse_entry_too_few_fields() {
        let path = PathBuf::from("test.conf");
        let raw = ":name:M";
        let entry = BinfmtEntry::parse(raw, &path, 1);
        assert!(entry.is_none());
    }

    #[test]
    fn test_parse_entry_empty() {
        let path = PathBuf::from("test.conf");
        let entry = BinfmtEntry::parse("", &path, 1);
        assert!(entry.is_none());
    }

    #[test]
    fn test_parse_entry_whitespace() {
        let path = PathBuf::from("test.conf");
        let entry = BinfmtEntry::parse("   ", &path, 1);
        assert!(entry.is_none());
    }

    #[test]
    fn test_parse_config_file_basic() {
        let dir = std::env::temp_dir().join("systemd-binfmt-test-basic");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# This is a comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, ":myformat:E::py::/usr/bin/python3:").unwrap();
        writeln!(f, "; Another comment").unwrap();
        writeln!(f, ":another:M::magic:mask:/usr/bin/handler:OC").unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].name, "myformat");
        assert_eq!(entries[1].name, "another");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_file_empty() {
        let dir = std::env::temp_dir().join("systemd-binfmt-test-empty");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("empty.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# Only comments here").unwrap();
        writeln!(f).unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert!(entries.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_file_missing() {
        let result = parse_config_file(Path::new("/nonexistent/binfmt.conf"));
        assert!(result.is_err());
    }

    #[test]
    fn test_is_binfmt_registered_nonexistent() {
        assert!(!is_binfmt_registered("zzzz_nonexistent_format_99999"));
    }

    #[test]
    fn test_discover_config_files_no_crash() {
        let _files = discover_config_files();
    }

    #[test]
    fn test_binfmt_entry_equality() {
        let path = PathBuf::from("test.conf");
        let a = BinfmtEntry::parse(":fmt1:E::py::/usr/bin/python:OC", &path, 1).unwrap();
        let b = BinfmtEntry::parse(":fmt1:E::py::/usr/bin/python:OC", &path, 1).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn test_binfmt_entry_inequality() {
        let path = PathBuf::from("test.conf");
        let a = BinfmtEntry::parse(":fmt1:E::py::/usr/bin/python:OC", &path, 1).unwrap();
        let b = BinfmtEntry::parse(":fmt2:E::rb::/usr/bin/ruby:OC", &path, 2).unwrap();
        assert_ne!(a.name, b.name);
    }

    #[test]
    fn test_last_entry_wins() {
        let dir = std::env::temp_dir().join("systemd-binfmt-test-override");
        let _ = fs::create_dir_all(&dir);

        let path1 = dir.join("a.conf");
        let path2 = dir.join("b.conf");

        let mut f1 = fs::File::create(&path1).unwrap();
        writeln!(f1, ":myfmt:E::py::/usr/bin/python2:").unwrap();
        drop(f1);

        let mut f2 = fs::File::create(&path2).unwrap();
        writeln!(f2, ":myfmt:E::py::/usr/bin/python3:").unwrap();
        drop(f2);

        let e1 = parse_config_file(&path1).unwrap();
        let e2 = parse_config_file(&path2).unwrap();

        let mut registrations = std::collections::BTreeMap::new();
        for entry in e1 {
            registrations.insert(entry.name.clone(), entry);
        }
        for entry in e2 {
            registrations.insert(entry.name.clone(), entry);
        }

        assert_eq!(registrations.len(), 1);
        assert!(registrations["myfmt"].raw.contains("python3"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_entry_extension_type() {
        let path = PathBuf::from("test.conf");
        let raw = ":python:E::py::/usr/bin/python3:OC";
        let entry = BinfmtEntry::parse(raw, &path, 1).unwrap();
        assert_eq!(entry.name, "python");
    }

    #[test]
    fn test_parse_entry_magic_type() {
        let path = PathBuf::from("test.conf");
        let raw = ":java:M:0:cafebabe:ffffffff:/usr/bin/java:";
        let entry = BinfmtEntry::parse(raw, &path, 1).unwrap();
        assert_eq!(entry.name, "java");
    }
}
