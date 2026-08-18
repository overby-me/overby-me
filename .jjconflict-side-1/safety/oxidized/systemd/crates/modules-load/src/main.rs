//! systemd-modules-load — Load kernel modules from static configuration
//!
//! Reads configuration from modules-load.d/*.conf files and loads kernel modules
//! at boot time using modprobe. This is a drop-in replacement for systemd-modules-load.
//!
//! Configuration is read from (in order of priority):
//!   /etc/modules-load.d/*.conf
//!   /run/modules-load.d/*.conf
//!   /usr/lib/modules-load.d/*.conf
//!   /lib/modules-load.d/*.conf
//!
//! Each .conf file contains a list of kernel module names, one per line.
//! Lines beginning with '#' or ';' are comments. Empty lines are ignored.

use std::collections::BTreeSet;
use std::fs;
use std::io::{self, BufRead};
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use clap::Parser;

const EXIT_SUCCESS: u8 = 0;
const EXIT_FAILURE: u8 = 1;

/// Directories to search for modules-load.d configuration, in priority order.
/// Earlier directories take precedence when the same filename exists in multiple.
const CONFIG_DIRS: &[&str] = &[
    "/etc/modules-load.d",
    "/run/modules-load.d",
    "/usr/lib/modules-load.d",
    "/lib/modules-load.d",
];

/// systemd-modules-load — Load kernel modules from static configuration
#[derive(Parser, Debug)]
#[command(name = "systemd-modules-load", version, about)]
struct Cli {
    /// Configuration files to read (overrides the standard modules-load.d
    /// directories). Each file lists module names, one per line.
    config_files: Vec<PathBuf>,
}

/// Represents a module to load, with optional parameters.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ModuleEntry {
    /// Module name (e.g. "loop", "brd")
    name: String,
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

        // Collect and sort entries from this directory
        let mut entries: Vec<PathBuf> = match fs::read_dir(dir_path) {
            Ok(rd) => rd
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().map(|ext| ext == "conf").unwrap_or(false))
                .collect(),
            Err(e) => {
                eprintln!(
                    "systemd-modules-load: Failed to read directory {}: {}",
                    dir, e
                );
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

/// Parse a single modules-load.d config file and return module names.
fn parse_config_file(path: &Path) -> io::Result<Vec<ModuleEntry>> {
    let file = fs::File::open(path)?;
    let reader = io::BufReader::new(file);
    let mut modules = Vec::new();

    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();

        // Skip empty lines and comments
        if trimmed.is_empty() || trimmed.starts_with('#') || trimmed.starts_with(';') {
            continue;
        }

        // The module name is the entire (trimmed) line.
        // Module parameters are not supported in modules-load.d; they belong
        // in modprobe.d configuration instead.
        modules.push(ModuleEntry {
            name: trimmed.to_string(),
        });
    }

    Ok(modules)
}

/// Check if a module is already loaded by examining /proc/modules.
fn is_module_loaded(module_name: &str) -> bool {
    // Module names in /proc/modules use underscores, but modprobe accepts both
    // hyphens and underscores. Normalize to underscores for comparison.
    let normalized = module_name.replace('-', "_");

    match fs::read_to_string("/proc/modules") {
        Ok(contents) => {
            for line in contents.lines() {
                if let Some(name) = line.split_whitespace().next()
                    && name == normalized
                {
                    return true;
                }
            }
            false
        }
        Err(_) => false,
    }
}

/// Check if a module is built into the kernel by examining
/// /sys/module/<name>/initstate or /lib/modules/$(uname -r)/modules.builtin
fn is_module_builtin(module_name: &str) -> bool {
    let normalized = module_name.replace('-', "_");

    // Check /sys/module/<name> — if the directory exists but there's no initstate
    // file, or if initstate is empty, it might be built-in.
    let sys_path = format!("/sys/module/{}", normalized);
    let initstate_path = format!("{}/initstate", sys_path);

    if Path::new(&sys_path).is_dir() {
        // If there's no initstate file, it's built-in
        if !Path::new(&initstate_path).exists() {
            return true;
        }
        // If initstate exists and says "live", it's a loaded module, not built-in
        if let Ok(state) = fs::read_to_string(&initstate_path) {
            let state = state.trim();
            if state == "live" || state == "coming" || state == "going" {
                return false;
            }
        }
    }

    false
}

/// Load a kernel module using modprobe.
fn load_module(module: &ModuleEntry, verbose: bool) -> bool {
    // Skip if already loaded
    if is_module_loaded(&module.name) {
        if verbose {
            eprintln!(
                "systemd-modules-load: Module '{}' is already loaded, skipping.",
                module.name
            );
        }
        return true;
    }

    // Skip if built-in
    if is_module_builtin(&module.name) {
        if verbose {
            eprintln!(
                "systemd-modules-load: Module '{}' is built-in, skipping.",
                module.name
            );
        }
        return true;
    }

    if verbose {
        eprintln!("systemd-modules-load: Loading module '{}'...", module.name);
    }

    let mut cmd = Command::new("modprobe");
    cmd.arg("--");
    cmd.arg(&module.name);

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                // Report the insertion at info level (unconditionally, like
                // systemd's log_info), so callers observing stderr see it.
                eprintln!("Inserted module '{}'", module.name);
                true
            } else {
                // Match systemd's message + format (line-start, no prefix) so
                // callers can grep "^Failed to find module".
                eprintln!("Failed to find module '{}'", module.name);
                false
            }
        }
        Err(e) => {
            eprintln!(
                "systemd-modules-load: Failed to execute modprobe for module '{}': {}",
                module.name, e
            );
            false
        }
    }
}

/// Normalize a module name the way the kernel/modprobe does: a dash is
/// equivalent to an underscore.  systemd-modules-load reports and deduplicates
/// modules on the normalized name.
fn normalize_name(name: &str) -> String {
    name.replace('-', "_")
}

/// Parse `modules_load=` / `rd.modules_load=` entries from the kernel command
/// line (honoring `$SYSTEMD_PROC_CMDLINE` for testing).  Each value is a
/// comma-separated list of module names.
fn cmdline_modules() -> Vec<String> {
    let cmdline = std::env::var("SYSTEMD_PROC_CMDLINE")
        .ok()
        .or_else(|| fs::read_to_string("/proc/cmdline").ok())
        .unwrap_or_default();
    let mut out = Vec::new();
    for tok in cmdline.split_whitespace() {
        if let Some(val) = tok
            .strip_prefix("modules_load=")
            .or_else(|| tok.strip_prefix("rd.modules_load="))
        {
            for m in val.split(',') {
                let m = m.trim();
                if !m.is_empty() {
                    out.push(m.to_string());
                }
            }
        }
    }
    out
}

fn run() -> u8 {
    let cli = Cli::parse();

    let verbose = std::env::var("SYSTEMD_LOG_LEVEL")
        .map(|v| v == "debug" || v == "info")
        .unwrap_or(false);

    // Configuration files: the explicit ones given on the command line, or the
    // standard modules-load.d directories when none were specified.  The
    // positional arguments are *config file paths*, not module names (matching
    // systemd-modules-load's `[CONFIGURATION FILE...]` interface).
    let explicit = !cli.config_files.is_empty();
    let config_files = if explicit {
        cli.config_files.clone()
    } else {
        discover_config_files()
    };

    let mut all_modules: Vec<ModuleEntry> = Vec::new();
    let mut seen: BTreeSet<String> = BTreeSet::new();
    let mut config_error = false;

    for path in &config_files {
        match parse_config_file(path) {
            Ok(file_modules) => {
                for m in file_modules {
                    // Normalize (dashes -> underscores, like the kernel) and
                    // deduplicate on the normalized name.
                    let norm = normalize_name(&m.name);
                    if seen.insert(norm.clone()) {
                        all_modules.push(ModuleEntry { name: norm });
                    }
                }
            }
            Err(e) => {
                eprintln!(
                    "systemd-modules-load: Failed to read {}: {}",
                    path.display(),
                    e
                );
                // Failing to read an explicitly-specified configuration file is
                // fatal; a missing standard directory is not.
                if explicit {
                    config_error = true;
                }
            }
        }
    }

    // Kernel command-line modules_load= entries, matching the boot-time
    // invocation (only when no explicit config files were requested).
    if !explicit {
        for m in cmdline_modules() {
            let norm = normalize_name(&m);
            if seen.insert(norm.clone()) {
                all_modules.push(ModuleEntry { name: norm });
            }
        }
    }

    // Load each module.  Individual load failures are reported (see
    // load_module) but are not fatal — only an unreadable explicit config file
    // makes us exit non-zero, matching systemd-modules-load.
    for module in &all_modules {
        let _ = load_module(module, verbose);
    }

    if config_error {
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
        let dir = std::env::temp_dir().join("systemd-modules-load-test-basic");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("test.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# This is a comment").unwrap();
        writeln!(f, "; This is also a comment").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "loop").unwrap();
        writeln!(f, "  brd  ").unwrap();
        writeln!(f, "vfat").unwrap();
        drop(f);

        let modules = parse_config_file(&path).unwrap();
        assert_eq!(modules.len(), 3);
        assert_eq!(modules[0].name, "loop");
        assert_eq!(modules[1].name, "brd");
        assert_eq!(modules[2].name, "vfat");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_empty() {
        let dir = std::env::temp_dir().join("systemd-modules-load-test-empty");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("empty.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# Only comments").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "; another comment").unwrap();
        drop(f);

        let modules = parse_config_file(&path).unwrap();
        assert!(modules.is_empty());

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_comments_and_whitespace() {
        let dir = std::env::temp_dir().join("systemd-modules-load-test-comments");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("mixed.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "# comment line").unwrap();
        writeln!(f, "module1").unwrap();
        writeln!(f, "   # indented comment").unwrap();
        writeln!(f, "   module2   ").unwrap();
        writeln!(f).unwrap();
        writeln!(f, "module3").unwrap();
        drop(f);

        let modules = parse_config_file(&path).unwrap();
        assert_eq!(modules.len(), 3);
        assert_eq!(modules[0].name, "module1");
        assert_eq!(modules[1].name, "module2");
        assert_eq!(modules[2].name, "module3");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_parse_config_missing_file() {
        let result = parse_config_file(Path::new("/nonexistent/file.conf"));
        assert!(result.is_err());
    }

    #[test]
    fn test_module_entry_equality() {
        let a = ModuleEntry {
            name: "loop".to_string(),
        };
        let b = ModuleEntry {
            name: "loop".to_string(),
        };
        let c = ModuleEntry {
            name: "brd".to_string(),
        };
        assert_eq!(a, b);
        assert_ne!(a, c);
    }

    #[test]
    fn test_discover_config_files_no_crash() {
        // This test just ensures discover_config_files doesn't panic,
        // even if none of the config dirs exist.
        let _files = discover_config_files();
    }

    #[test]
    fn test_is_module_builtin_nonexistent() {
        // A module that definitely doesn't exist shouldn't be considered built-in
        assert!(!is_module_builtin("zzzz_nonexistent_module_xyxyx"));
    }

    #[test]
    fn test_is_module_loaded_nonexistent() {
        // A module that definitely isn't loaded
        assert!(!is_module_loaded("zzzz_nonexistent_module_xyxyx"));
    }

    #[test]
    fn test_hyphen_underscore_normalization() {
        // Module names with hyphens should be normalized to underscores
        // for /proc/modules comparison
        let name = "my-module-name";
        let normalized = name.replace('-', "_");
        assert_eq!(normalized, "my_module_name");
    }

    #[test]
    fn test_parse_config_semicolon_comment() {
        let dir = std::env::temp_dir().join("systemd-modules-load-test-semicolon");
        let _ = fs::create_dir_all(&dir);
        let path = dir.join("semi.conf");
        let mut f = fs::File::create(&path).unwrap();
        writeln!(f, "; comment with semicolon").unwrap();
        writeln!(f, "mymodule").unwrap();
        drop(f);

        let modules = parse_config_file(&path).unwrap();
        assert_eq!(modules.len(), 1);
        assert_eq!(modules[0].name, "mymodule");

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_deduplication_across_files() {
        let dir = std::env::temp_dir().join("systemd-modules-load-test-dedup");
        let _ = fs::create_dir_all(&dir);

        let path1 = dir.join("a.conf");
        let path2 = dir.join("b.conf");

        let mut f1 = fs::File::create(&path1).unwrap();
        writeln!(f1, "loop").unwrap();
        writeln!(f1, "brd").unwrap();
        drop(f1);

        let mut f2 = fs::File::create(&path2).unwrap();
        writeln!(f2, "loop").unwrap();
        writeln!(f2, "vfat").unwrap();
        drop(f2);

        let m1 = parse_config_file(&path1).unwrap();
        let m2 = parse_config_file(&path2).unwrap();

        // Simulate deduplication as done in run()
        let mut seen = BTreeSet::new();
        let mut all = Vec::new();
        for m in m1.into_iter().chain(m2) {
            if seen.insert(m.name.clone()) {
                all.push(m);
            }
        }

        assert_eq!(all.len(), 3);
        assert_eq!(all[0].name, "loop");
        assert_eq!(all[1].name, "brd");
        assert_eq!(all[2].name, "vfat");

        let _ = fs::remove_dir_all(&dir);
    }
}
