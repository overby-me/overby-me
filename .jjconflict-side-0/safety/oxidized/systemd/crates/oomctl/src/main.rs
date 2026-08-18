//! oomctl — CLI for systemd-oomd
//!
//! Provides a command-line interface to inspect the state of the systemd-oomd
//! daemon, including system-wide memory pressure and per-cgroup pressure metrics.

use std::fs;
use std::path::{Path, PathBuf};

use clap::{Parser, Subcommand};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "oomctl",
    about = "Analyze OOM killer",
    long_about = "oomctl may be used to get information about the various contexts monitored by systemd-oomd."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// Show systemd-oomd state and monitored cgroups
    Dump,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

const UNIT_DIRS: &[&str] = &[
    "/etc/systemd/system",
    "/run/systemd/system",
    "/usr/lib/systemd/system",
    "/lib/systemd/system",
    "/run/systemd/generator",
    "/run/systemd/generator.early",
    "/run/systemd/generator.late",
];

// ---------------------------------------------------------------------------
// PSI parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct PsiMetrics {
    some_avg10: f64,
    some_avg60: f64,
    some_avg300: f64,
    some_total: u64,
    full_avg10: f64,
    full_avg60: f64,
    full_avg300: f64,
    full_total: u64,
}

impl PsiMetrics {
    fn read(path: &Path) -> Option<Self> {
        let contents = fs::read_to_string(path).ok()?;
        Some(Self::parse(&contents))
    }

    fn parse(contents: &str) -> Self {
        let mut metrics = Self::default();

        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("some ") {
                let (avg10, avg60, avg300, total) = parse_psi_line(line);
                metrics.some_avg10 = avg10;
                metrics.some_avg60 = avg60;
                metrics.some_avg300 = avg300;
                metrics.some_total = total;
            } else if line.starts_with("full ") {
                let (avg10, avg60, avg300, total) = parse_psi_line(line);
                metrics.full_avg10 = avg10;
                metrics.full_avg60 = avg60;
                metrics.full_avg300 = avg300;
                metrics.full_total = total;
            }
        }

        metrics
    }
}

fn parse_psi_line(line: &str) -> (f64, f64, f64, u64) {
    let mut avg10 = 0.0;
    let mut avg60 = 0.0;
    let mut avg300 = 0.0;
    let mut total = 0u64;

    for part in line.split_whitespace() {
        if let Some(val) = part.strip_prefix("avg10=") {
            avg10 = val.parse().unwrap_or(0.0);
        } else if let Some(val) = part.strip_prefix("avg60=") {
            avg60 = val.parse().unwrap_or(0.0);
        } else if let Some(val) = part.strip_prefix("avg300=") {
            avg300 = val.parse().unwrap_or(0.0);
        } else if let Some(val) = part.strip_prefix("total=") {
            total = val.parse().unwrap_or(0);
        }
    }

    (avg10, avg60, avg300, total)
}

// ---------------------------------------------------------------------------
// Swap info
// ---------------------------------------------------------------------------

fn read_swap_info() -> Option<(u64, u64)> {
    let contents = fs::read_to_string("/proc/meminfo").ok()?;
    let mut swap_total: Option<u64> = None;
    let mut swap_free: Option<u64> = None;

    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("SwapTotal:") {
            swap_total = parse_meminfo_kb(rest);
        } else if let Some(rest) = line.strip_prefix("SwapFree:") {
            swap_free = parse_meminfo_kb(rest);
        }
    }

    match (swap_total, swap_free) {
        (Some(total), Some(free)) => Some((total.saturating_sub(free), total)),
        _ => None,
    }
}

fn parse_meminfo_kb(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches("kB").trim();
    s.parse::<u64>().ok().map(|kb| kb * 1024)
}

fn format_bytes(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.1}G", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1}M", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1}K", bytes as f64 / 1024.0)
    } else {
        format!("{bytes}B")
    }
}

// ---------------------------------------------------------------------------
// Managed cgroup discovery
// ---------------------------------------------------------------------------

struct ManagedCgroup {
    path: PathBuf,
    unit_name: String,
    swap_action: String,
    memory_pressure_action: String,
    memory_pressure_limit: Option<String>,
}

fn discover_managed_cgroups() -> Vec<ManagedCgroup> {
    let mut results = Vec::new();
    let mut seen = std::collections::HashSet::new();

    for dir in UNIT_DIRS {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let entries = match fs::read_dir(dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };

            if !name.ends_with(".slice") && !name.ends_with(".service") {
                continue;
            }

            if seen.contains(&name) {
                continue;
            }
            seen.insert(name.clone());

            let contents = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(managed) = parse_managed_oom(&name, &contents) {
                results.push(managed);
            }
        }
    }

    results
}

fn parse_managed_oom(unit_name: &str, contents: &str) -> Option<ManagedCgroup> {
    let mut swap_action = String::new();
    let mut pressure_action = String::new();
    let mut pressure_limit: Option<String> = None;
    let mut found = false;
    let mut in_section = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') {
            let section = line.to_ascii_lowercase();
            in_section = section == "[slice]" || section == "[service]";
            continue;
        }

        if !in_section {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "ManagedOOMSwap" => {
                    swap_action = value.to_owned();
                    found = true;
                }
                "ManagedOOMMemoryPressure" => {
                    pressure_action = value.to_owned();
                    found = true;
                }
                "ManagedOOMMemoryPressureLimit" => {
                    pressure_limit = Some(value.to_owned());
                    found = true;
                }
                _ => {}
            }
        }
    }

    if !found {
        return None;
    }

    let cgroup_path = if unit_name == "-.slice" {
        PathBuf::from(CGROUP_ROOT)
    } else {
        PathBuf::from(CGROUP_ROOT).join(unit_name)
    };

    Some(ManagedCgroup {
        path: cgroup_path,
        unit_name: unit_name.to_owned(),
        swap_action,
        memory_pressure_action: pressure_action,
        memory_pressure_limit: pressure_limit,
    })
}

// ---------------------------------------------------------------------------
// Dump command
// ---------------------------------------------------------------------------

fn cmd_dump() {
    println!("System Context:");
    println!("    Swap: {}", format_swap_status());
    println!();

    // System-wide memory pressure
    println!("System Memory Pressure:");
    if let Some(psi) = PsiMetrics::read(Path::new("/proc/pressure/memory")) {
        print_psi(&psi, "    ");
    } else {
        println!("    (unavailable)");
    }
    println!();

    // Managed cgroups
    let managed = discover_managed_cgroups();
    if managed.is_empty() {
        println!("No monitored cgroups.");
        println!();
        println!(
            "Hint: Configure ManagedOOMSwap= or ManagedOOMMemoryPressure= in slice/service units,"
        );
        println!(
            "      or enable systemd.oomd.enableRootSlice / enableSystemSlice / enableUserSlices."
        );
    } else {
        println!("Monitored Cgroups:");
        for cg in &managed {
            println!();
            println!("  {}:", cg.unit_name);
            println!("    Path: {}", cg.path.display());

            if !cg.swap_action.is_empty() {
                println!("    ManagedOOMSwap: {}", cg.swap_action);
            }
            if !cg.memory_pressure_action.is_empty() {
                println!(
                    "    ManagedOOMMemoryPressure: {}",
                    cg.memory_pressure_action
                );
            }
            if let Some(ref limit) = cg.memory_pressure_limit {
                println!("    ManagedOOMMemoryPressureLimit: {limit}");
            }

            // Show current memory usage
            let mem_current_path = cg.path.join("memory.current");
            if let Ok(val) = fs::read_to_string(&mem_current_path)
                && let Ok(bytes) = val.trim().parse::<u64>()
            {
                println!("    Current Memory: {}", format_bytes(bytes));
            }

            // Show memory min/low
            let mem_min_path = cg.path.join("memory.min");
            if let Ok(val) = fs::read_to_string(&mem_min_path)
                && let Ok(bytes) = val.trim().parse::<u64>()
                && bytes > 0
            {
                println!("    Memory Min: {}", format_bytes(bytes));
            }
            let mem_low_path = cg.path.join("memory.low");
            if let Ok(val) = fs::read_to_string(&mem_low_path)
                && let Ok(bytes) = val.trim().parse::<u64>()
                && bytes > 0
            {
                println!("    Memory Low: {}", format_bytes(bytes));
            }

            // Show pressure
            if let Some(psi) = PsiMetrics::read(&cg.path.join("memory.pressure")) {
                println!("    Memory Pressure:");
                print_psi(&psi, "      ");
            }
        }
    }
}

fn format_swap_status() -> String {
    match read_swap_info() {
        Some((used, total)) if total > 0 => {
            let pct = (used as f64 / total as f64) * 100.0;
            format!(
                "{} / {} ({:.1}% used)",
                format_bytes(used),
                format_bytes(total),
                pct
            )
        }
        Some((_, _)) => "no swap configured".to_owned(),
        None => "unavailable".to_owned(),
    }
}

fn print_psi(psi: &PsiMetrics, indent: &str) {
    println!(
        "{indent}some avg10={:.2} avg60={:.2} avg300={:.2} total={}",
        psi.some_avg10, psi.some_avg60, psi.some_avg300, psi.some_total
    );
    println!(
        "{indent}full avg10={:.2} avg60={:.2} avg300={:.2} total={}",
        psi.full_avg10, psi.full_avg60, psi.full_avg300, psi.full_total
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    match cli.command {
        Some(Commands::Dump) | None => {
            cmd_dump();
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_psi_parse() {
        let metrics = PsiMetrics::parse(
            "some avg10=1.50 avg60=2.30 avg300=3.10 total=12345\n\
             full avg10=0.50 avg60=0.80 avg300=1.20 total=6789\n",
        );
        assert!((metrics.some_avg10 - 1.50).abs() < 0.01);
        assert!((metrics.some_avg60 - 2.30).abs() < 0.01);
        assert!((metrics.some_avg300 - 3.10).abs() < 0.01);
        assert_eq!(metrics.some_total, 12345);
        assert!((metrics.full_avg10 - 0.50).abs() < 0.01);
        assert!((metrics.full_avg60 - 0.80).abs() < 0.01);
        assert!((metrics.full_avg300 - 1.20).abs() < 0.01);
        assert_eq!(metrics.full_total, 6789);
    }

    #[test]
    fn test_psi_parse_empty() {
        let metrics = PsiMetrics::parse("");
        assert!((metrics.some_avg10).abs() < 0.001);
        assert_eq!(metrics.some_total, 0);
    }

    #[test]
    fn test_format_bytes() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(1024), "1.0K");
        assert_eq!(format_bytes(1536), "1.5K");
        assert_eq!(format_bytes(1048576), "1.0M");
        assert_eq!(format_bytes(1073741824), "1.0G");
    }

    #[test]
    fn test_parse_meminfo_kb() {
        assert_eq!(parse_meminfo_kb("  1024 kB"), Some(1024 * 1024));
        assert_eq!(parse_meminfo_kb("0 kB"), Some(0));
        assert_eq!(parse_meminfo_kb("abc"), None);
    }

    #[test]
    fn test_parse_managed_oom_slice() {
        let contents = r#"
[Slice]
ManagedOOMMemoryPressure=kill
ManagedOOMMemoryPressureLimit=80%
"#;
        let managed = parse_managed_oom("system.slice", contents).unwrap();
        assert_eq!(managed.unit_name, "system.slice");
        assert_eq!(managed.memory_pressure_action, "kill");
        assert_eq!(managed.memory_pressure_limit, Some("80%".to_owned()));
        assert!(managed.swap_action.is_empty());
    }

    #[test]
    fn test_parse_managed_oom_no_directives() {
        let contents = r#"
[Slice]
Description=Test
"#;
        assert!(parse_managed_oom("test.slice", contents).is_none());
    }

    #[test]
    fn test_parse_managed_oom_root_slice() {
        let contents = r#"
[Slice]
ManagedOOMMemoryPressure=kill
"#;
        let managed = parse_managed_oom("-.slice", contents).unwrap();
        assert_eq!(managed.path, PathBuf::from("/sys/fs/cgroup/"));
    }

    #[test]
    fn test_parse_managed_oom_ignores_wrong_section() {
        let contents = r#"
[Unit]
ManagedOOMMemoryPressure=kill

[Slice]
Description=Test
"#;
        assert!(parse_managed_oom("test.slice", contents).is_none());
    }

    #[test]
    fn test_format_swap_no_swap() {
        // Just make sure it doesn't panic
        let _ = format_swap_status();
    }

    #[test]
    fn test_parse_psi_line() {
        let (a10, a60, a300, total) =
            parse_psi_line("some avg10=1.23 avg60=4.56 avg300=7.89 total=100");
        assert!((a10 - 1.23).abs() < 0.001);
        assert!((a60 - 4.56).abs() < 0.001);
        assert!((a300 - 7.89).abs() < 0.001);
        assert_eq!(total, 100);
    }

    #[test]
    fn test_parse_psi_line_empty() {
        let (a10, a60, a300, total) = parse_psi_line("");
        assert!((a10).abs() < 0.001);
        assert!((a60).abs() < 0.001);
        assert!((a300).abs() < 0.001);
        assert_eq!(total, 0);
    }
}
