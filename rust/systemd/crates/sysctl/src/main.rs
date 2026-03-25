//! systemd-sysctl — Apply kernel sysctl parameters from static configuration
//!
//! Reads configuration from sysctl.d/*.conf files and applies kernel parameters
//! by writing to /proc/sys/. This is a drop-in replacement for systemd-sysctl.
//!
//! Configuration is read from (in order of priority):
//!   /etc/sysctl.d/*.conf
//!   /run/sysctl.d/*.conf
//!   /usr/lib/sysctl.d/*.conf
//!   /lib/sysctl.d/*.conf
//!   /etc/sysctl.conf (legacy, lowest priority)
//!
//! Each .conf file contains key=value pairs of sysctl parameters.
//! Lines beginning with '#' or ';' are comments. Empty lines are ignored.
//! Keys use dotted notation (e.g. net.ipv4.ip_forward) which maps to
//! /proc/sys/net/ipv4/ip_forward.
//!
//! A key prefixed with '-' means errors applying that setting are ignored.
//! Glob patterns in keys are supported (e.g. net.ipv4.conf.*.rp_filter).

use std::collections::BTreeMap;
use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Directories to search for sysctl.d configuration, in priority order.
/// Earlier directories take precedence when the same filename exists in multiple.
const CONFIG_DIRS: &[&str] = &[
    "/etc/sysctl.d",
    "/run/sysctl.d",
    "/usr/lib/sysctl.d",
    "/lib/sysctl.d",
];

/// Legacy sysctl.conf path (lowest priority).
const LEGACY_CONF: &str = "/etc/sysctl.conf";

/// Base path for sysctl parameters in procfs.
const PROC_SYS: &str = "/proc/sys";

/// systemd-sysctl — Apply kernel sysctl parameters from static configuration
#[derive(Parser, Debug)]
#[command(name = "systemd-sysctl", version, about)]
struct Cli {
    /// Only apply settings for the given prefix(es)
    /// (e.g. "net.ipv4" or "/net/ipv4" to only apply net.ipv4.* settings)
    #[arg(long = "prefix")]
    prefixes: Vec<String>,

    /// Return non-zero exit code on failure even for unknown sysctl parameters
    #[arg(long)]
    strict: bool,

    /// Specific sysctl.d config files to read (instead of scanning directories)
    files: Vec<PathBuf>,
}

/// A sysctl setting parsed from configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SysctlEntry {
    /// The sysctl key in dotted notation (e.g. "net.ipv4.ip_forward").
    key: String,
    /// The value to set.
    value: String,
    /// If true, errors applying this setting are silently ignored (key was prefixed with '-').
    ignore_error: bool,
    /// Source file for diagnostic messages.
    source: PathBuf,
    /// Line number in source file.
    line_number: usize,
}

/// Convert a dotted sysctl key to a /proc/sys path.
/// e.g. "net.ipv4.ip_forward" -> "/proc/sys/net/ipv4/ip_forward"
fn key_to_proc_path(key: &str) -> PathBuf {
    let path_part = key.replace('.', "/");
    PathBuf::from(PROC_SYS).join(path_part)
}

/// Convert a /proc/sys path back to a dotted sysctl key.
/// e.g. "/proc/sys/net/ipv4/ip_forward" -> "net.ipv4.ip_forward"
fn proc_path_to_key(path: &Path) -> Option<String> {
    let stripped = path.strip_prefix(PROC_SYS).ok()?;
    Some(
        stripped
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect::<Vec<_>>()
            .join("."),
    )
}

/// Match a glob pattern against a string. Supports '*' as a wildcard
/// that matches any sequence of characters within a single path segment.
#[cfg(test)]
fn glob_match(pattern: &str, text: &str) -> bool {
    let pattern_parts: Vec<&str> = pattern.split('.').collect();
    let text_parts: Vec<&str> = text.split('.').collect();

    if pattern_parts.len() != text_parts.len() {
        return false;
    }

    for (p, t) in pattern_parts.iter().zip(text_parts.iter()) {
        if *p == "*" {
            continue;
        }
        if !simple_glob_match(p, t) {
            return false;
        }
    }

    true
}

/// Simple glob matching for a single segment: supports '*' and '?' wildcards.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    simple_glob_match_inner(&p, &t)
}

fn simple_glob_match_inner(pattern: &[char], text: &[char]) -> bool {
    if pattern.is_empty() {
        return text.is_empty();
    }

    match pattern[0] {
        '*' => {
            // '*' matches zero or more characters
            for i in 0..=text.len() {
                if simple_glob_match_inner(&pattern[1..], &text[i..]) {
                    return true;
                }
            }
            false
        }
        '?' => {
            // '?' matches exactly one character
            if text.is_empty() {
                false
            } else {
                simple_glob_match_inner(&pattern[1..], &text[1..])
            }
        }
        c => {
            if text.is_empty() || text[0] != c {
                false
            } else {
                simple_glob_match_inner(&pattern[1..], &text[1..])
            }
        }
    }
}

/// Check if a key contains glob characters.
fn is_glob_pattern(key: &str) -> bool {
    key.contains('*') || key.contains('?')
}

/// Expand a glob pattern against actual /proc/sys entries.
/// Returns a list of concrete sysctl keys that match the pattern.
fn expand_glob(pattern: &str) -> Vec<String> {
    let parts: Vec<&str> = pattern.split('.').collect();
    let mut results = Vec::new();
    let base = PathBuf::from(PROC_SYS);

    expand_glob_recursive(&base, &parts, 0, &mut results);
    results
}

fn expand_glob_recursive(
    current_path: &Path,
    parts: &[&str],
    index: usize,
    results: &mut Vec<String>,
) {
    if index >= parts.len() {
        // We've matched all parts; check if the path exists and is a file
        if current_path.is_file()
            && let Some(key) = proc_path_to_key(current_path)
        {
            results.push(key);
        }
        return;
    }

    let part = parts[index];

    if part.contains('*') || part.contains('?') {
        // Glob: enumerate the directory and match
        if let Ok(entries) = fs::read_dir(current_path) {
            let mut names: Vec<String> = entries
                .filter_map(|e| e.ok())
                .filter_map(|e| e.file_name().into_string().ok())
                .collect();
            names.sort();

            for name in names {
                if simple_glob_match(part, &name) {
                    let next_path = current_path.join(&name);
                    expand_glob_recursive(&next_path, parts, index + 1, results);
                }
            }
        }
    } else {
        let next_path = current_path.join(part);
        if next_path.exists() {
            expand_glob_recursive(&next_path, parts, index + 1, results);
        }
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
                eprintln!("systemd-sysctl: Failed to read directory {}: {}", dir, e);
                continue;
            }
        };
        entries.sort();

        for path in entries {
            if let Some(file_name) = path.file_name().and_then(|n| n.to_str()) {
                let name = file_name.to_string();
                if seen_names.contains(&name) {
                    continue;
                }
                seen_names.insert(name);
                result.push(path);
            }
        }
    }

    // Also check the legacy /etc/sysctl.conf
    let legacy_path = Path::new(LEGACY_CONF);
    if legacy_path.is_file() {
        result.push(legacy_path.to_path_buf());
    }

    result
}

/// Parse a single sysctl.d config file and return sysctl entries.
fn parse_config_file(path: &Path) -> io::Result<Vec<SysctlEntry>> {
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

        // Parse key=value or key = value
        let (key_part, value_part) = if let Some(pos) = trimmed.find('=') {
            let k = trimmed[..pos].trim();
            let v = trimmed[pos + 1..].trim();
            (k, v)
        } else {
            eprintln!(
                "systemd-sysctl: {}:{}: line is not a valid key=value pair, ignoring: {}",
                path.display(),
                line_number,
                trimmed
            );
            continue;
        };

        if key_part.is_empty() {
            eprintln!(
                "systemd-sysctl: {}:{}: empty key, ignoring.",
                path.display(),
                line_number,
            );
            continue;
        }

        // Check for '-' prefix (ignore errors)
        let (key, ignore_error) = if let Some(stripped) = key_part.strip_prefix('-') {
            (stripped.trim(), true)
        } else {
            (key_part, false)
        };

        if key.is_empty() {
            eprintln!(
                "systemd-sysctl: {}:{}: empty key after prefix, ignoring.",
                path.display(),
                line_number,
            );
            continue;
        }

        entries.push(SysctlEntry {
            key: key.to_string(),
            value: value_part.to_string(),
            ignore_error,
            source: path.to_path_buf(),
            line_number,
        });
    }

    Ok(entries)
}

/// Normalize a prefix from /path/style to dot.style.
/// e.g. "/net/ipv4/conf/hoge" -> "net.ipv4.conf.hoge"
fn normalize_prefix(prefix: &str) -> String {
    let stripped = prefix.strip_prefix('/').unwrap_or(prefix);
    stripped.replace('/', ".")
}

/// Apply a single sysctl setting by writing to /proc/sys.
/// When `strict` is false, write errors for non-existent parameters are
/// treated as warnings (return true). When `strict` is true, they cause
/// failure (return false), unless the entry has `ignore_error` set.
fn apply_sysctl(entry: &SysctlEntry, verbose: bool, strict: bool, prefixes: &[String]) -> bool {
    let keys_to_apply: Vec<String> = if is_glob_pattern(&entry.key) {
        let expanded = expand_glob(&entry.key);
        if expanded.is_empty() {
            if !entry.ignore_error && strict {
                eprintln!(
                    "systemd-sysctl: {}:{}: glob pattern '{}' matched no sysctl parameters.",
                    entry.source.display(),
                    entry.line_number,
                    entry.key,
                );
            }
            return true;
        }
        expanded
    } else {
        vec![entry.key.clone()]
    };

    let mut all_ok = true;

    for key in &keys_to_apply {
        // Apply prefix filter after glob expansion
        if !prefixes.is_empty() && !prefixes.iter().any(|p| key.starts_with(p.as_str())) {
            continue;
        }

        let proc_path = key_to_proc_path(key);

        if verbose {
            eprintln!("systemd-sysctl: Setting '{}' to '{}'", key, entry.value);
        }

        match fs::write(&proc_path, format!("{}\n", entry.value)) {
            Ok(()) => {
                if verbose {
                    eprintln!("systemd-sysctl: Successfully set '{}'.", key);
                }
            }
            Err(e) => {
                if entry.ignore_error {
                    if verbose {
                        eprintln!("systemd-sysctl: Failed to set '{}' (ignored): {}", key, e);
                    }
                } else if strict {
                    eprintln!(
                        "systemd-sysctl: {}:{}: Failed to set '{}' to '{}': {}",
                        entry.source.display(),
                        entry.line_number,
                        key,
                        entry.value,
                        e
                    );
                    all_ok = false;
                }
            }
        }
    }

    all_ok
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    // Collect config files to read
    let config_files = if !cli.files.is_empty() {
        cli.files.clone()
    } else {
        discover_config_files()
    };

    if verbose {
        eprintln!(
            "systemd-sysctl: Found {} configuration file(s).",
            config_files.len()
        );
    }

    // Parse all files and collect entries. Later entries for the same key
    // override earlier ones (last writer wins), matching systemd behavior.
    let mut settings: BTreeMap<String, SysctlEntry> = BTreeMap::new();

    for path in &config_files {
        match parse_config_file(path) {
            Ok(entries) => {
                if verbose {
                    eprintln!(
                        "systemd-sysctl: Read {} setting(s) from {}",
                        entries.len(),
                        path.display()
                    );
                }
                for entry in entries {
                    // For glob patterns, use the pattern itself as the key
                    // (they'll be expanded at apply time)
                    settings.insert(entry.key.clone(), entry);
                }
            }
            Err(e) => {
                eprintln!("systemd-sysctl: Failed to read {}: {}", path.display(), e);
            }
        }
    }

    if settings.is_empty() {
        if verbose {
            eprintln!("systemd-sysctl: No sysctl settings to apply.");
        }
        return EXIT_SUCCESS;
    }

    // Normalize prefixes: convert /path/style to dot.style
    let normalized_prefixes: Vec<String> =
        cli.prefixes.iter().map(|p| normalize_prefix(p)).collect();

    // Prefix filtering now happens inside apply_sysctl (after glob expansion),
    // but we can still skip entries whose non-glob keys clearly don't match.
    let entries_to_apply: Vec<&SysctlEntry> = settings.values().collect();

    if verbose {
        eprintln!(
            "systemd-sysctl: Applying {} sysctl setting(s)...",
            entries_to_apply.len()
        );
    }

    let mut any_failed = false;
    for entry in entries_to_apply {
        if !apply_sysctl(entry, verbose, cli.strict, &normalized_prefixes) {
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
    fn test_key_to_proc_path() {
        assert_eq!(
            key_to_proc_path("net.ipv4.ip_forward"),
            PathBuf::from("/proc/sys/net/ipv4/ip_forward")
        );
        assert_eq!(
            key_to_proc_path("kernel.shmmax"),
            PathBuf::from("/proc/sys/kernel/shmmax")
        );
        assert_eq!(
            key_to_proc_path("vm.swappiness"),
            PathBuf::from("/proc/sys/vm/swappiness")
        );
    }

    #[test]
    fn test_proc_path_to_key() {
        assert_eq!(
            proc_path_to_key(Path::new("/proc/sys/net/ipv4/ip_forward")),
            Some("net.ipv4.ip_forward".to_string())
        );
        assert_eq!(
            proc_path_to_key(Path::new("/proc/sys/kernel/shmmax")),
            Some("kernel.shmmax".to_string())
        );
        // Not under /proc/sys — should return None
        assert_eq!(proc_path_to_key(Path::new("/etc/something")), None);
    }

    #[test]
    fn test_key_path_roundtrip() {
        let keys = [
            "net.ipv4.ip_forward",
            "kernel.shmmax",
            "vm.swappiness",
            "net.core.somaxconn",
        ];
        for key in keys {
            let path = key_to_proc_path(key);
            let back = proc_path_to_key(&path).unwrap();
            assert_eq!(back, key);
        }
    }

    #[test]
    fn test_parse_config_basic() {
        let dir = std::env::temp_dir().join("systemd-sysctl-test-basic");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# This is a comment").unwrap();
        writeln!(f, "; Another comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "net.ipv4.ip_forward = 1").unwrap();
        writeln!(f, "kernel.shmmax=68719476736").unwrap();
        writeln!(f, "  vm.swappiness = 10  ").unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key, "net.ipv4.ip_forward");
        assert_eq!(entries[0].value, "1");
        assert!(!entries[0].ignore_error);
        assert_eq!(entries[1].key, "kernel.shmmax");
        assert_eq!(entries[1].value, "68719476736");
        assert_eq!(entries[2].key, "vm.swappiness");
        assert_eq!(entries[2].value, "10");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_ignore_error_prefix() {
        let dir = std::env::temp_dir().join("systemd-sysctl-test-ignore");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "-net.ipv4.ip_forward = 1").unwrap();
        writeln!(f, "- kernel.shmmax = 12345").unwrap();
        writeln!(f, "vm.swappiness = 60").unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].key, "net.ipv4.ip_forward");
        assert!(entries[0].ignore_error);
        assert_eq!(entries[1].key, "kernel.shmmax");
        assert!(entries[1].ignore_error);
        assert_eq!(entries[2].key, "vm.swappiness");
        assert!(!entries[2].ignore_error);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_empty() {
        let dir = std::env::temp_dir().join("systemd-sysctl-test-empty");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("empty.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# Only comments").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "; semicolon comment").unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert!(entries.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_missing_file() {
        let result = parse_config_file(Path::new("/nonexistent/sysctl.conf"));
        assert!(result.is_err());
    }

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("net.ipv4.ip_forward", "net.ipv4.ip_forward"));
        assert!(!glob_match("net.ipv4.ip_forward", "net.ipv4.ip_forward2"));
        assert!(!glob_match("net.ipv4.ip_forward", "net.ipv6.ip_forward"));
    }

    #[test]
    fn test_glob_match_wildcard() {
        assert!(glob_match(
            "net.ipv4.conf.*.rp_filter",
            "net.ipv4.conf.eth0.rp_filter"
        ));
        assert!(glob_match(
            "net.ipv4.conf.*.rp_filter",
            "net.ipv4.conf.all.rp_filter"
        ));
        assert!(!glob_match(
            "net.ipv4.conf.*.rp_filter",
            "net.ipv4.conf.rp_filter"
        ));
        assert!(!glob_match(
            "net.ipv4.conf.*.rp_filter",
            "net.ipv6.conf.eth0.rp_filter"
        ));
    }

    #[test]
    fn test_glob_match_question_mark() {
        assert!(glob_match("net.ipv?.ip_forward", "net.ipv4.ip_forward"));
        assert!(glob_match("net.ipv?.ip_forward", "net.ipv6.ip_forward"));
        assert!(!glob_match("net.ipv?.ip_forward", "net.ipvv4.ip_forward"));
    }

    #[test]
    fn test_is_glob_pattern() {
        assert!(is_glob_pattern("net.ipv4.conf.*.rp_filter"));
        assert!(is_glob_pattern("net.ipv?.ip_forward"));
        assert!(!is_glob_pattern("net.ipv4.ip_forward"));
    }

    #[test]
    fn test_simple_glob_match() {
        assert!(simple_glob_match("*", "anything"));
        assert!(simple_glob_match("*", ""));
        assert!(simple_glob_match("eth*", "eth0"));
        assert!(simple_glob_match("eth*", "eth123"));
        assert!(simple_glob_match("*0", "eth0"));
        assert!(!simple_glob_match("eth*", "wlan0"));
        assert!(simple_glob_match("?", "a"));
        assert!(!simple_glob_match("?", ""));
        assert!(!simple_glob_match("?", "ab"));
        assert!(simple_glob_match("e?h0", "eth0"));
    }

    #[test]
    fn test_last_writer_wins() {
        let dir = std::env::temp_dir().join("systemd-sysctl-test-override");
        let _ = fs::create_dir_all(&dir);

        let path1 = dir.join("a.conf");
        let path2 = dir.join("b.conf");

        let mut f1 = fs::File::create(&path1).unwrap();
        writeln!(f1, "net.ipv4.ip_forward = 0").unwrap();
        writeln!(f1, "vm.swappiness = 30").unwrap();
        drop(f1);

        let mut f2 = fs::File::create(&path2).unwrap();
        writeln!(f2, "net.ipv4.ip_forward = 1").unwrap();
        drop(f2);

        let e1 = parse_config_file(&path1).unwrap();
        let e2 = parse_config_file(&path2).unwrap();

        let mut settings: BTreeMap<String, SysctlEntry> = BTreeMap::new();
        for entry in e1 {
            settings.insert(entry.key.clone(), entry);
        }
        for entry in e2 {
            settings.insert(entry.key.clone(), entry);
        }

        // ip_forward should be overridden to "1"
        assert_eq!(settings["net.ipv4.ip_forward"].value, "1");
        // swappiness should still be from first file
        assert_eq!(settings["vm.swappiness"].value, "30");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_discover_config_files_no_crash() {
        // Just make sure it doesn't panic
        let _files = discover_config_files();
    }

    #[test]
    fn test_parse_config_values_with_spaces() {
        let dir = std::env::temp_dir().join("systemd-sysctl-test-spaces");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("spaces.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "kernel.domainname = example.com").unwrap();
        writeln!(f, "kernel.hostname=myhost").unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].key, "kernel.domainname");
        assert_eq!(entries[0].value, "example.com");
        assert_eq!(entries[1].key, "kernel.hostname");
        assert_eq!(entries[1].value, "myhost");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_line_numbers() {
        let dir = std::env::temp_dir().join("systemd-sysctl-test-lineno");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("lineno.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# comment on line 1").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "key1 = val1").unwrap();
        writeln!(f, "# another comment").unwrap();
        writeln!(f, "key2 = val2").unwrap();
        drop(f);

        let entries = parse_config_file(&path).unwrap();
        assert_eq!(entries.len(), 2);
        assert_eq!(entries[0].line_number, 3);
        assert_eq!(entries[1].line_number, 5);

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_glob_match_different_lengths() {
        // Different number of segments should never match
        assert!(!glob_match("a.b", "a.b.c"));
        assert!(!glob_match("a.b.c", "a.b"));
    }

    #[test]
    fn test_glob_match_all_wildcards() {
        assert!(glob_match("*.*.*", "a.b.c"));
        assert!(glob_match("*.*", "foo.bar"));
        assert!(!glob_match("*.*", "foo.bar.baz"));
    }
}
