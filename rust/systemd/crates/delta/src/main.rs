//! systemd-delta — Show overridden, extended, masked and redirected unit files.
//!
//! A drop-in replacement for `systemd-delta(1)`. This tool scans the
//! systemd unit search directories and identifies unit files that:
//!
//! - **[OVERRIDDEN]** — A higher-priority directory provides a file with the
//!   same name, completely replacing the lower-priority version.
//! - **[EXTENDED]** — Drop-in directories (`.d/`) contain `.conf` files that
//!   extend the base unit.
//! - **[MASKED]** — A unit file is a symlink to `/dev/null`, preventing the
//!   unit from being loaded.
//! - **[REDIRECTED]** — A unit file is a symlink to another unit file.
//! - **[EQUIVALENT]** — Two files in different directories have identical
//!   content.
//!
//! Supported options:
//!
//! - `--type=TYPE`    — Filter by override type (overridden, extended, masked,
//!   redirected, equivalent). Can be specified multiple times.
//! - `--diff`         — Show a unified diff for overridden files (default: true)
//! - `--no-pager`     — Do not pipe output into a pager
//! - `PREFIX...`      — Limit output to unit directories matching these prefixes

use clap::Parser;
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::process;

#[derive(Parser, Debug)]
#[command(
    name = "systemd-delta",
    about = "Find overridden, extended, masked and redirected configuration files",
    version
)]
struct Cli {
    /// Show only a specific type of difference. Can be specified multiple
    /// times. Valid values: overridden, extended, masked, redirected,
    /// equivalent.
    #[arg(long = "type", value_name = "TYPE", num_args = 1)]
    filter_type: Vec<String>,

    /// Show a unified diff for overridden files.
    #[arg(long, default_value = "true", num_args = 0..=1, default_missing_value = "true", value_parser = clap::builder::BoolishValueParser::new())]
    diff: bool,

    /// Do not pipe output through a pager.
    #[arg(long)]
    no_pager: bool,

    /// Limit output to directories matching these prefixes (e.g. /etc /run).
    prefixes: Vec<String>,
}

/// The well-known systemd unit search directories, listed from highest
/// priority to lowest. A unit file in a higher-priority directory
/// overrides one with the same name in a lower-priority directory.
const SYSTEM_UNIT_DIRS: &[&str] = &[
    "/etc/systemd/system",
    "/run/systemd/system",
    "/usr/local/lib/systemd/system",
    "/usr/lib/systemd/system",
    "/lib/systemd/system",
];

/// Configuration directories (non-unit, but also searched by systemd-delta).
const SYSTEM_CONF_DIRS: &[&str] = &["/etc/systemd", "/run/systemd", "/usr/lib/systemd"];

/// Tmpfiles directories.
const TMPFILES_DIRS: &[&str] = &["/etc/tmpfiles.d", "/run/tmpfiles.d", "/usr/lib/tmpfiles.d"];

/// Sysusers directories.
const SYSUSERS_DIRS: &[&str] = &["/etc/sysusers.d", "/run/sysusers.d", "/usr/lib/sysusers.d"];

/// Sysctl directories.
const SYSCTL_DIRS: &[&str] = &["/etc/sysctl.d", "/run/sysctl.d", "/usr/lib/sysctl.d"];

/// Udev rules directories.
const UDEV_RULES_DIRS: &[&str] = &[
    "/etc/udev/rules.d",
    "/run/udev/rules.d",
    "/usr/lib/udev/rules.d",
];

/// Modules-load directories.
const MODULES_LOAD_DIRS: &[&str] = &[
    "/etc/modules-load.d",
    "/run/modules-load.d",
    "/usr/lib/modules-load.d",
];

/// Binfmt directories.
const BINFMT_DIRS: &[&str] = &["/etc/binfmt.d", "/run/binfmt.d", "/usr/lib/binfmt.d"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DeltaType {
    Overridden,
    Extended,
    Masked,
    Redirected,
    Equivalent,
    Unchanged,
}

impl DeltaType {
    fn label(self) -> &'static str {
        match self {
            DeltaType::Overridden => "[OVERRIDDEN]",
            DeltaType::Extended => "[EXTENDED]",
            DeltaType::Masked => "[MASKED]",
            DeltaType::Redirected => "[REDIRECTED]",
            DeltaType::Equivalent => "[EQUIVALENT]",
            DeltaType::Unchanged => "[UNCHANGED]",
        }
    }

    fn color_code(self) -> &'static str {
        match self {
            DeltaType::Overridden => "\x1b[31m", // red
            DeltaType::Extended => "\x1b[32m",   // green
            DeltaType::Masked => "\x1b[33m",     // yellow
            DeltaType::Redirected => "\x1b[36m", // cyan
            DeltaType::Equivalent => "\x1b[90m", // gray
            DeltaType::Unchanged => "\x1b[90m",  // gray
        }
    }

    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "overridden" => Some(DeltaType::Overridden),
            "extended" => Some(DeltaType::Extended),
            "masked" => Some(DeltaType::Masked),
            "redirected" => Some(DeltaType::Redirected),
            "equivalent" => Some(DeltaType::Equivalent),
            "unchanged" => Some(DeltaType::Unchanged),
            _ => None,
        }
    }
}

struct DeltaEntry {
    delta_type: DeltaType,
    top_path: PathBuf,
    bottom_path: Option<PathBuf>,
}

/// Check if a file is masked (symlink to /dev/null).
fn is_masked(path: &Path) -> bool {
    if let Ok(target) = fs::read_link(path) {
        target == Path::new("/dev/null")
    } else {
        false
    }
}

/// Check if a file is a symlink to another real file (redirected).
fn is_redirected(path: &Path) -> bool {
    if let Ok(target) = fs::read_link(path) {
        target != Path::new("/dev/null") && target.exists()
    } else {
        false
    }
}

/// Compare two files for byte-equality.
fn files_equal(a: &Path, b: &Path) -> bool {
    match (fs::read(a), fs::read(b)) {
        (Ok(content_a), Ok(content_b)) => content_a == content_b,
        _ => false,
    }
}

/// Find drop-in directories for a unit file name.
///
/// For a unit named `foo.service`, drop-in directories are
/// `foo.service.d/` in each search directory.
fn find_dropins(name: &str, dirs: &[&str]) -> Vec<PathBuf> {
    let dropin_dir_name = format!("{name}.d");
    let mut results = Vec::new();

    for dir in dirs {
        let dropin_path = Path::new(dir).join(&dropin_dir_name);
        if dropin_path.is_dir()
            && let Ok(entries) = fs::read_dir(&dropin_path)
        {
            for entry in entries.flatten() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "conf") && p.is_file() {
                    results.push(p);
                }
            }
        }
    }

    results
}

/// Scan a set of ordered directories (highest priority first) and find
/// all delta relationships between files with the same name.
fn scan_dirs(dirs: &[&str], prefix_filter: &[String]) -> Vec<DeltaEntry> {
    let mut entries = Vec::new();

    // Filter directories by prefix if specified
    let effective_dirs: Vec<&str> = if prefix_filter.is_empty() {
        dirs.to_vec()
    } else {
        dirs.iter()
            .filter(|d| prefix_filter.iter().any(|p| d.starts_with(p.as_str())))
            .copied()
            .collect()
    };

    if effective_dirs.is_empty() {
        return entries;
    }

    // Collect all filenames and which directories they appear in.
    // Map: filename -> list of (priority_index, full_path)
    let mut file_map: BTreeMap<String, Vec<(usize, PathBuf)>> = BTreeMap::new();
    let mut seen_names: BTreeSet<String> = BTreeSet::new();

    for (priority, dir) in effective_dirs.iter().enumerate() {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let read_dir = match fs::read_dir(dir_path) {
            Ok(rd) => rd,
            Err(_) => continue,
        };

        for entry in read_dir.flatten() {
            let file_name = entry.file_name().to_string_lossy().to_string();

            // Skip drop-in directories (they're handled separately)
            if file_name.ends_with(".d") && entry.path().is_dir() {
                continue;
            }

            // Only consider regular files and symlinks
            let path = entry.path();
            if !path.is_file() && !path.is_symlink() {
                continue;
            }

            file_map
                .entry(file_name.clone())
                .or_default()
                .push((priority, path));
            seen_names.insert(file_name);
        }
    }

    // Now analyze each file
    for (name, locations) in &file_map {
        if locations.len() < 2 {
            // Check for masked or redirected even with a single location
            if locations.len() == 1 {
                let path = &locations[0].1;
                if is_masked(path) {
                    entries.push(DeltaEntry {
                        delta_type: DeltaType::Masked,
                        top_path: path.clone(),
                        bottom_path: None,
                    });
                } else if is_redirected(path)
                    && let Ok(target) = fs::read_link(path)
                {
                    entries.push(DeltaEntry {
                        delta_type: DeltaType::Redirected,
                        top_path: path.clone(),
                        bottom_path: Some(target),
                    });
                }
            }
        } else {
            // Multiple locations: the highest priority (lowest index) wins
            let top = &locations[0]; // highest priority
            let bottom = &locations[locations.len() - 1]; // lowest priority

            if is_masked(&top.1) {
                entries.push(DeltaEntry {
                    delta_type: DeltaType::Masked,
                    top_path: top.1.clone(),
                    bottom_path: Some(bottom.1.clone()),
                });
            } else if is_redirected(&top.1) {
                if let Ok(target) = fs::read_link(&top.1) {
                    entries.push(DeltaEntry {
                        delta_type: DeltaType::Redirected,
                        top_path: top.1.clone(),
                        bottom_path: Some(target),
                    });
                }
            } else if files_equal(&top.1, &bottom.1) {
                entries.push(DeltaEntry {
                    delta_type: DeltaType::Equivalent,
                    top_path: top.1.clone(),
                    bottom_path: Some(bottom.1.clone()),
                });
            } else {
                entries.push(DeltaEntry {
                    delta_type: DeltaType::Overridden,
                    top_path: top.1.clone(),
                    bottom_path: Some(bottom.1.clone()),
                });
            }
        }

        // Check for drop-in extensions
        let dropins = find_dropins(name, &effective_dirs);
        if !dropins.is_empty() {
            // Find the base unit path (from any location)
            let base_path = &locations[0].1;
            for dropin in &dropins {
                entries.push(DeltaEntry {
                    delta_type: DeltaType::Extended,
                    top_path: dropin.clone(),
                    bottom_path: Some(base_path.clone()),
                });
            }
        }
    }

    // Also scan for drop-in directories where the base unit doesn't exist
    // in the current scan (orphaned drop-ins)
    for dir in &effective_dirs {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }
        if let Ok(read_dir) = fs::read_dir(dir_path) {
            for entry in read_dir.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if name.ends_with(".d") && entry.path().is_dir() {
                    let base_name = &name[..name.len() - 2];
                    if !seen_names.contains(base_name) {
                        // Drop-in dir exists but no base unit found in our scan
                        let dropins = find_dropins(base_name, &effective_dirs);
                        for dropin in &dropins {
                            entries.push(DeltaEntry {
                                delta_type: DeltaType::Extended,
                                top_path: dropin.clone(),
                                bottom_path: None,
                            });
                        }
                        seen_names.insert(base_name.to_string());
                    }
                }
            }
        }
    }

    entries
}

/// Produce a simple unified diff between two files.
fn simple_diff(old_path: &Path, new_path: &Path) -> String {
    let old_content = fs::read_to_string(old_path).unwrap_or_default();
    let new_content = fs::read_to_string(new_path).unwrap_or_default();

    let old_lines: Vec<&str> = old_content.lines().collect();
    let new_lines: Vec<&str> = new_content.lines().collect();

    let mut output = String::new();
    output.push_str(&format!(
        "--- {}\n+++ {}\n",
        old_path.display(),
        new_path.display()
    ));

    // Simple line-by-line comparison (not a real unified diff algorithm,
    // but sufficient for showing what changed)
    let _max_len = old_lines.len().max(new_lines.len());
    let mut in_hunk = false;
    let mut hunk = String::new();
    let mut hunk_start_old = 0;
    let mut hunk_start_new = 0;
    let mut hunk_count_old = 0;
    let mut hunk_count_new = 0;

    let flush_hunk = |output: &mut String,
                      hunk: &mut String,
                      start_old: usize,
                      count_old: usize,
                      start_new: usize,
                      count_new: usize| {
        if !hunk.is_empty() {
            output.push_str(&format!(
                "@@ -{},{} +{},{} @@\n",
                start_old + 1,
                count_old,
                start_new + 1,
                count_new
            ));
            output.push_str(hunk);
            hunk.clear();
        }
    };

    let context = 3;
    let mut i = 0;
    let mut j = 0;

    // Very simple: just show removed and added lines
    while i < old_lines.len() || j < new_lines.len() {
        if i < old_lines.len() && j < new_lines.len() && old_lines[i] == new_lines[j] {
            if in_hunk {
                hunk.push_str(&format!(" {}\n", old_lines[i]));
                hunk_count_old += 1;
                hunk_count_new += 1;
            }
            i += 1;
            j += 1;
        } else {
            if !in_hunk {
                in_hunk = true;
                hunk_start_old = i.saturating_sub(context);
                hunk_start_new = j.saturating_sub(context);
                hunk_count_old = 0;
                hunk_count_new = 0;
                // Add context before
                let ctx_start = i.saturating_sub(context);
                for k in ctx_start..i {
                    if k < old_lines.len() {
                        hunk.push_str(&format!(" {}\n", old_lines[k]));
                        hunk_count_old += 1;
                        hunk_count_new += 1;
                    }
                }
            }

            // Consume differing lines
            if i < old_lines.len()
                && (j >= new_lines.len() || old_lines[i] != *new_lines.get(j).unwrap_or(&""))
            {
                hunk.push_str(&format!("-{}\n", old_lines[i]));
                hunk_count_old += 1;
                i += 1;
            } else if j < new_lines.len() {
                hunk.push_str(&format!("+{}\n", new_lines[j]));
                hunk_count_new += 1;
                j += 1;
            }
        }
    }

    flush_hunk(
        &mut output,
        &mut hunk,
        hunk_start_old,
        hunk_count_old,
        hunk_start_new,
        hunk_count_new,
    );

    output
}

/// Check if the terminal supports colors.
fn use_colors() -> bool {
    std::env::var("NO_COLOR").is_err() && std::env::var("TERM").map(|t| t != "dumb").unwrap_or(true)
}

fn main() {
    let cli = Cli::parse();

    // Parse type filters
    let type_filters: Vec<DeltaType> = if cli.filter_type.is_empty() {
        vec![] // empty = show all
    } else {
        let mut filters = Vec::new();
        for t in &cli.filter_type {
            // Support comma-separated values too
            for part in t.split(',') {
                match DeltaType::from_str(part.trim()) {
                    Some(dt) => filters.push(dt),
                    None => {
                        eprintln!(
                            "Unknown type: {part}. Valid types: overridden, extended, masked, redirected, equivalent, unchanged"
                        );
                        process::exit(1);
                    }
                }
            }
        }
        filters
    };

    let should_show =
        |dt: DeltaType| -> bool { type_filters.is_empty() || type_filters.contains(&dt) };

    let colors = use_colors();
    let reset = if colors { "\x1b[0m" } else { "" };

    // Scan all known directory sets
    let dir_sets: Vec<&[&str]> = vec![
        SYSTEM_UNIT_DIRS,
        SYSTEM_CONF_DIRS,
        TMPFILES_DIRS,
        SYSUSERS_DIRS,
        SYSCTL_DIRS,
        UDEV_RULES_DIRS,
        MODULES_LOAD_DIRS,
        BINFMT_DIRS,
    ];

    let mut total_found = 0;
    let mut counts: BTreeMap<&str, usize> = BTreeMap::new();

    for dirs in &dir_sets {
        let deltas = scan_dirs(dirs, &cli.prefixes);

        for entry in &deltas {
            if !should_show(entry.delta_type) {
                continue;
            }

            total_found += 1;
            *counts.entry(entry.delta_type.label()).or_insert(0) += 1;

            let color = if colors {
                entry.delta_type.color_code()
            } else {
                ""
            };

            match (&entry.delta_type, &entry.bottom_path) {
                (_, Some(bottom)) => {
                    println!(
                        "{color}{}{reset} {} -> {}",
                        entry.delta_type.label(),
                        entry.top_path.display(),
                        bottom.display()
                    );
                }
                (_, None) => {
                    println!(
                        "{color}{}{reset} {}",
                        entry.delta_type.label(),
                        entry.top_path.display()
                    );
                }
            }

            // Show diff for overridden files
            if cli.diff
                && entry.delta_type == DeltaType::Overridden
                && let Some(bottom) = &entry.bottom_path
            {
                let diff = simple_diff(bottom, &entry.top_path);
                if !diff.is_empty() {
                    println!();
                    println!("{diff}");
                }
            }
        }
    }

    // Print summary
    if total_found > 0 {
        println!();
        let parts: Vec<String> = counts
            .iter()
            .map(|(label, count)| format!("{count} {label}"))
            .collect();
        println!("{} {}", total_found, parts.join(", "));
    } else if cli.prefixes.is_empty() && type_filters.is_empty() {
        // Nothing found at all — inform the user
        println!("No overridden or extended unit files found.");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn test_delta_type_from_str() {
        assert_eq!(
            DeltaType::from_str("overridden"),
            Some(DeltaType::Overridden)
        );
        assert_eq!(DeltaType::from_str("extended"), Some(DeltaType::Extended));
        assert_eq!(DeltaType::from_str("masked"), Some(DeltaType::Masked));
        assert_eq!(
            DeltaType::from_str("redirected"),
            Some(DeltaType::Redirected)
        );
        assert_eq!(
            DeltaType::from_str("equivalent"),
            Some(DeltaType::Equivalent)
        );
        assert_eq!(
            DeltaType::from_str("OVERRIDDEN"),
            Some(DeltaType::Overridden)
        );
        assert_eq!(DeltaType::from_str("unknown"), None);
    }

    #[test]
    fn test_delta_type_label() {
        assert_eq!(DeltaType::Overridden.label(), "[OVERRIDDEN]");
        assert_eq!(DeltaType::Extended.label(), "[EXTENDED]");
        assert_eq!(DeltaType::Masked.label(), "[MASKED]");
        assert_eq!(DeltaType::Redirected.label(), "[REDIRECTED]");
        assert_eq!(DeltaType::Equivalent.label(), "[EQUIVALENT]");
    }

    #[test]
    fn test_is_masked_not_symlink() {
        // A regular file is not masked
        let dir = std::env::temp_dir().join("systemd-delta-test-masked");
        let _ = fs::create_dir_all(&dir);
        let file = dir.join("test.service");
        fs::write(&file, "content").unwrap();
        assert!(!is_masked(&file));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_is_masked_symlink_to_dev_null() {
        let dir = std::env::temp_dir().join("systemd-delta-test-masked2");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        let link = dir.join("test.service");
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink("/dev/null", &link).unwrap();
            assert!(is_masked(&link));
        }
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_files_equal_same_content() {
        let dir = std::env::temp_dir().join("systemd-delta-test-equal");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        let a = dir.join("a.conf");
        let b = dir.join("b.conf");
        fs::write(&a, "same content").unwrap();
        fs::write(&b, "same content").unwrap();
        assert!(files_equal(&a, &b));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_files_equal_different_content() {
        let dir = std::env::temp_dir().join("systemd-delta-test-diff");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);
        let a = dir.join("a.conf");
        let b = dir.join("b.conf");
        fs::write(&a, "content A").unwrap();
        fs::write(&b, "content B").unwrap();
        assert!(!files_equal(&a, &b));
        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_scan_empty_dirs() {
        let entries = scan_dirs(&["/nonexistent/path/that/should/not/exist"], &[]);
        assert!(entries.is_empty());
    }

    #[test]
    fn test_scan_with_overrides() {
        let base = std::env::temp_dir().join("systemd-delta-test-scan");
        let _ = fs::remove_dir_all(&base);

        let high = base.join("high");
        let low = base.join("low");
        let _ = fs::create_dir_all(&high);
        let _ = fs::create_dir_all(&low);

        // Same name, different content -> OVERRIDDEN
        fs::write(high.join("test.conf"), "high priority").unwrap();
        fs::write(low.join("test.conf"), "low priority").unwrap();

        // Same name, same content -> EQUIVALENT
        fs::write(high.join("same.conf"), "identical").unwrap();
        fs::write(low.join("same.conf"), "identical").unwrap();

        let high_str = high.to_string_lossy().to_string();
        let low_str = low.to_string_lossy().to_string();
        let dirs: Vec<&str> = vec![high_str.as_str(), low_str.as_str()];

        let entries = scan_dirs(&dirs, &[]);

        let has_overridden = entries
            .iter()
            .any(|e| e.delta_type == DeltaType::Overridden);
        let has_equivalent = entries
            .iter()
            .any(|e| e.delta_type == DeltaType::Equivalent);

        assert!(has_overridden, "Should detect overridden file");
        assert!(has_equivalent, "Should detect equivalent file");

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_find_dropins_none() {
        let dropins = find_dropins(
            "nonexistent.service",
            &["/nonexistent/path/that/should/not/exist"],
        );
        assert!(dropins.is_empty());
    }

    #[test]
    fn test_find_dropins_found() {
        let base = std::env::temp_dir().join("systemd-delta-test-dropins");
        let _ = fs::remove_dir_all(&base);

        let dir = base.join("system");
        let dropin_dir = dir.join("test.service.d");
        let _ = fs::create_dir_all(&dropin_dir);
        fs::write(
            dropin_dir.join("override.conf"),
            "[Service]\nRestart=always\n",
        )
        .unwrap();
        fs::write(dropin_dir.join("not-a-conf.txt"), "ignored").unwrap();

        let dir_str = dir.to_string_lossy().to_string();
        let dropins = find_dropins("test.service", &[dir_str.as_str()]);

        assert_eq!(dropins.len(), 1);
        assert!(
            dropins[0]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with(".conf")
        );

        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn test_simple_diff_identical() {
        let dir = std::env::temp_dir().join("systemd-delta-test-diff-id");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);

        let a = dir.join("a.conf");
        let b = dir.join("b.conf");
        fs::write(&a, "same\n").unwrap();
        fs::write(&b, "same\n").unwrap();

        let diff = simple_diff(&a, &b);
        // Should contain header but no +/- lines
        assert!(diff.contains("---"));
        assert!(diff.contains("+++"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_simple_diff_different() {
        let dir = std::env::temp_dir().join("systemd-delta-test-diff-diff");
        let _ = fs::remove_dir_all(&dir);
        let _ = fs::create_dir_all(&dir);

        let a = dir.join("old.conf");
        let b = dir.join("new.conf");
        fs::write(&a, "old line\n").unwrap();
        fs::write(&b, "new line\n").unwrap();

        let diff = simple_diff(&a, &b);
        assert!(diff.contains("-old line"));
        assert!(diff.contains("+new line"));

        let _ = fs::remove_dir_all(&dir);
    }

    #[test]
    fn test_use_colors_no_panic() {
        // Just ensure it doesn't panic
        let _ = use_colors();
    }
}
