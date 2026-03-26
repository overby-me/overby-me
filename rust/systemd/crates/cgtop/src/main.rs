//! systemd-cgtop — Show top control groups by their resource usage.
//!
//! A drop-in replacement for `systemd-cgtop(1)`. Reads cgroup2 resource
//! usage statistics from `/sys/fs/cgroup/` and displays them sorted by
//! CPU, memory, or I/O usage. Supports both one-shot and continuous
//! (refreshing) display modes.

use clap::Parser;
use std::collections::HashMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::thread;
use std::time::{Duration, Instant};

// ── CLI ───────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "systemd-cgtop",
    about = "Show top control groups by their resource usage",
    version
)]
struct Cli {
    /// Refresh interval (e.g. "1", "500ms", "2s")
    #[arg(short = 'd', long, default_value = "1", value_parser = parse_delay)]
    delay: f64,

    /// Number of iterations (0 = infinite)
    #[arg(short = 'n', long, default_value_t = 1)]
    iterations: u32,

    /// Run a single iteration (equivalent to --iterations=1)
    #[arg(short = '1', hide = true)]
    one_shot: bool,

    /// Sort by: cpu, memory, io, tasks, path
    #[arg(short = 's', long, default_value = "cpu", value_parser = parse_order)]
    order: String,

    /// Show raw (bytes/usec) values instead of human-readable
    #[arg(long)]
    raw: bool,

    /// Count only userspace processes instead of tasks (processes + kernel threads)
    #[arg(short = 'P')]
    count_userspace_processes: bool,

    /// Include kernel threads in output
    #[arg(short = 'k')]
    kernel_threads: bool,

    /// Do not pipe output into a pager
    #[arg(long)]
    no_pager: bool,

    /// Show recursive resource usage (yes/no)
    #[arg(long, default_value = "yes", default_missing_value = "yes", num_args = 0..=1, value_parser = parse_bool_arg)]
    recursive: bool,

    /// Limit depth of cgroup hierarchy shown
    #[arg(long)]
    depth: Option<usize>,

    /// Show batch-mode output (no terminal control codes)
    #[arg(short, long)]
    batch: bool,

    /// CPU display mode: percentage or time
    #[arg(long, value_parser = parse_cpu_mode)]
    cpu: Option<String>,

    /// Order by path
    #[arg(short = 'p', hide = true)]
    order_path: bool,

    /// Order by tasks
    #[arg(short = 't', hide = true)]
    order_tasks: bool,

    /// Order by CPU
    #[arg(short = 'c', hide = true)]
    order_cpu: bool,

    /// Order by memory
    #[arg(short = 'm', hide = true)]
    order_memory: bool,

    /// Order by I/O
    #[arg(short = 'i', hide = true)]
    order_io: bool,

    /// Specific cgroup path to show (default: /)
    cgroup: Option<String>,
}

/// Parse a delay value with optional time suffix (ms, s, min, h, d, us, usec, msec).
fn parse_delay(s: &str) -> Result<f64, String> {
    // Try parsing as plain number (seconds)
    if let Ok(v) = s.parse::<f64>() {
        return Ok(v);
    }

    // Try with suffix
    let s_lower = s.to_lowercase();
    let (num_str, multiplier) = if let Some(n) = s_lower.strip_suffix("msec") {
        (n, 0.001)
    } else if let Some(n) = s_lower.strip_suffix("usec") {
        (n, 0.000_001)
    } else if let Some(n) = s_lower.strip_suffix("ms") {
        (n, 0.001)
    } else if let Some(n) = s_lower.strip_suffix("us") {
        (n, 0.000_001)
    } else if let Some(n) = s_lower.strip_suffix("min") {
        (n, 60.0)
    } else if let Some(n) = s_lower.strip_suffix('s') {
        (n, 1.0)
    } else if let Some(n) = s_lower.strip_suffix('h') {
        (n, 3600.0)
    } else if let Some(n) = s_lower.strip_suffix('d') {
        (n, 86400.0)
    } else {
        return Err(format!("Failed to parse delay value: {s}"));
    };

    let num: f64 = num_str
        .parse()
        .map_err(|_| format!("Failed to parse delay value: {s}"))?;
    Ok(num * multiplier)
}

/// Parse order value with validation.
fn parse_order(s: &str) -> Result<String, String> {
    match s.to_lowercase().as_str() {
        "cpu" | "c" | "memory" | "mem" | "m" | "io" | "i" | "input" | "output" | "tasks" | "t"
        | "task" | "path" | "p" | "name" => Ok(s.to_string()),
        _ => Err(format!("Invalid order: {s}")),
    }
}

/// Parse --cpu mode value.
fn parse_cpu_mode(s: &str) -> Result<String, String> {
    match s.to_lowercase().as_str() {
        "percentage" | "time" => Ok(s.to_string()),
        _ => Err(format!("Invalid CPU mode: {s}")),
    }
}

/// Parse a boolean argument value (yes/no/true/false/1/0).
fn parse_bool_arg(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "yes" | "true" | "1" => Ok(true),
        "no" | "false" | "0" => Ok(false),
        _ => Err(format!("Invalid boolean value: {s}")),
    }
}

// ── Data structures ───────────────────────────────────────────────────────

/// Resource usage snapshot for a single cgroup.
#[derive(Debug, Clone)]
struct CgroupUsage {
    /// Cgroup path relative to the cgroup root
    path: String,
    /// Number of tasks (processes + threads) in this cgroup
    tasks: u64,
    /// CPU usage in microseconds (from cpu.stat usage_usec)
    cpu_usec: u64,
    /// CPU percentage (computed from delta between snapshots)
    cpu_percent: f64,
    /// Memory usage in bytes (from memory.current)
    memory_bytes: u64,
    /// I/O bytes read (from io.stat)
    io_read_bytes: u64,
    /// I/O bytes written (from io.stat)
    io_write_bytes: u64,
}

/// A snapshot of all cgroup resource usage at a point in time.
#[derive(Debug, Clone)]
struct Snapshot {
    timestamp: Instant,
    cgroups: HashMap<String, CgroupUsage>,
}

// ── Sort order ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SortOrder {
    Cpu,
    Memory,
    Io,
    Tasks,
    Path,
}

impl SortOrder {
    fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "cpu" | "c" => SortOrder::Cpu,
            "memory" | "mem" | "m" => SortOrder::Memory,
            "io" | "i" | "input" | "output" => SortOrder::Io,
            "tasks" | "t" | "task" => SortOrder::Tasks,
            "path" | "p" | "name" => SortOrder::Path,
            // Validated by CLI parser, should not reach here
            other => {
                eprintln!("Invalid order: {other}");
                process::exit(1);
            }
        }
    }
}

// ── Cgroup reading ────────────────────────────────────────────────────────

const CGROUP_ROOT: &str = "/sys/fs/cgroup";

/// Read the number of tasks in a cgroup from `cgroup.procs` (or `pids.current`).
fn read_task_count(cgroup_path: &Path) -> u64 {
    // Try pids.current first (gives the count directly)
    let pids_current = cgroup_path.join("pids.current");
    if let Ok(content) = fs::read_to_string(&pids_current)
        && let Ok(count) = content.trim().parse::<u64>()
    {
        return count;
    }

    // Fall back to counting lines in cgroup.procs
    let procs_file = cgroup_path.join("cgroup.procs");
    if let Ok(content) = fs::read_to_string(&procs_file) {
        return content.lines().filter(|l| !l.trim().is_empty()).count() as u64;
    }

    0
}

/// Read CPU usage in microseconds from `cpu.stat`.
fn read_cpu_usage(cgroup_path: &Path) -> u64 {
    let stat_file = cgroup_path.join("cpu.stat");
    if let Ok(content) = fs::read_to_string(&stat_file) {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("usage_usec") {
                let val = rest.trim();
                if let Ok(usec) = val.parse::<u64>() {
                    return usec;
                }
            }
        }
    }
    0
}

/// Read memory usage in bytes from `memory.current`.
fn read_memory_usage(cgroup_path: &Path) -> u64 {
    let mem_file = cgroup_path.join("memory.current");
    if let Ok(content) = fs::read_to_string(&mem_file)
        && let Ok(bytes) = content.trim().parse::<u64>()
    {
        return bytes;
    }
    0
}

/// Read I/O statistics from `io.stat`.
/// Returns (total_read_bytes, total_write_bytes) across all devices.
fn read_io_usage(cgroup_path: &Path) -> (u64, u64) {
    let io_file = cgroup_path.join("io.stat");
    let mut total_read: u64 = 0;
    let mut total_write: u64 = 0;

    if let Ok(content) = fs::read_to_string(&io_file) {
        for line in content.lines() {
            for field in line.split_whitespace() {
                if let Some(val) = field.strip_prefix("rbytes=") {
                    if let Ok(v) = val.parse::<u64>() {
                        total_read = total_read.saturating_add(v);
                    }
                } else if let Some(val) = field.strip_prefix("wbytes=")
                    && let Ok(v) = val.parse::<u64>()
                {
                    total_write = total_write.saturating_add(v);
                }
            }
        }
    }

    (total_read, total_write)
}

/// Recursively discover all cgroups under the given root.
fn discover_cgroups(
    root: &Path,
    _base: &Path,
    max_depth: Option<usize>,
    current_depth: usize,
    result: &mut Vec<PathBuf>,
) {
    result.push(root.to_path_buf());

    if max_depth.is_some_and(|d| current_depth >= d) {
        return;
    }

    if let Ok(entries) = fs::read_dir(root) {
        let mut dirs: Vec<_> = entries
            .flatten()
            .filter(|e| e.path().is_dir() && e.path().join("cgroup.procs").exists())
            .collect();

        dirs.sort_by_key(|e| e.file_name());

        for entry in dirs {
            discover_cgroups(&entry.path(), _base, max_depth, current_depth + 1, result);
        }
    }
}

/// Take a snapshot of all cgroup resource usage.
fn take_snapshot(cgroup_root: &Path, max_depth: Option<usize>) -> Snapshot {
    let mut cgroup_paths = Vec::new();
    discover_cgroups(cgroup_root, cgroup_root, max_depth, 0, &mut cgroup_paths);

    let mut cgroups = HashMap::new();

    for cg_path in &cgroup_paths {
        let rel_path = cg_path
            .strip_prefix(cgroup_root)
            .unwrap_or(cg_path)
            .to_string_lossy()
            .to_string();

        let display_path = if rel_path.is_empty() {
            "/".to_string()
        } else {
            format!("/{}", rel_path)
        };

        let tasks = read_task_count(cg_path);
        let cpu_usec = read_cpu_usage(cg_path);
        let memory_bytes = read_memory_usage(cg_path);
        let (io_read, io_write) = read_io_usage(cg_path);

        let usage = CgroupUsage {
            path: display_path.clone(),
            tasks,
            cpu_usec,
            cpu_percent: 0.0,
            memory_bytes,
            io_read_bytes: io_read,
            io_write_bytes: io_write,
        };

        cgroups.insert(display_path, usage);
    }

    Snapshot {
        timestamp: Instant::now(),
        cgroups,
    }
}

/// Compute CPU percentages by comparing two snapshots.
fn compute_cpu_percentages(prev: &Snapshot, curr: &mut Snapshot) {
    let elapsed = curr.timestamp.duration_since(prev.timestamp);
    let elapsed_usec = elapsed.as_micros() as f64;

    if elapsed_usec <= 0.0 {
        return;
    }

    // Get number of CPUs for proper percentage calculation
    let num_cpus = num_cpus();

    for (path, usage) in curr.cgroups.iter_mut() {
        if let Some(prev_usage) = prev.cgroups.get(path) {
            let delta_usec = usage.cpu_usec.saturating_sub(prev_usage.cpu_usec) as f64;
            // CPU percentage: (delta_cpu_time / (elapsed_wall_time * num_cpus)) * 100
            usage.cpu_percent = (delta_usec / (elapsed_usec * num_cpus as f64)) * 100.0;
            // Clamp to reasonable range
            if usage.cpu_percent > 100.0 * num_cpus as f64 {
                usage.cpu_percent = 100.0 * num_cpus as f64;
            }
            if usage.cpu_percent < 0.0 {
                usage.cpu_percent = 0.0;
            }
        }
    }
}

/// Get the number of online CPUs.
fn num_cpus() -> usize {
    // Try /proc/stat — count lines starting with "cpu" (excluding "cpu " itself)
    if let Ok(content) = fs::read_to_string("/proc/stat") {
        let count = content
            .lines()
            .filter(|l| l.starts_with("cpu") && !l.starts_with("cpu "))
            .count();
        if count > 0 {
            return count;
        }
    }

    // Try /sys/devices/system/cpu/online
    if let Ok(content) = fs::read_to_string("/sys/devices/system/cpu/online")
        && let Some(count) = parse_cpu_range(&content)
    {
        return count;
    }

    // Fallback
    1
}

/// Parse CPU range string like "0-7" or "0-3,5-7" and return the count.
fn parse_cpu_range(s: &str) -> Option<usize> {
    let mut count = 0usize;
    for range in s.trim().split(',') {
        let parts: Vec<&str> = range.split('-').collect();
        match parts.len() {
            1 => {
                parts[0].trim().parse::<usize>().ok()?;
                count += 1;
            }
            2 => {
                let start: usize = parts[0].trim().parse().ok()?;
                let end: usize = parts[1].trim().parse().ok()?;
                count += end - start + 1;
            }
            _ => return None,
        }
    }
    Some(count)
}

// ── Formatting ────────────────────────────────────────────────────────────

/// Format bytes into a human-readable string.
fn format_bytes(bytes: u64) -> String {
    if bytes == 0 {
        return "-".to_string();
    }

    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if bytes >= TIB {
        format!("{:.1}T", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.1}G", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1}M", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1}K", bytes as f64 / KIB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Format CPU percentage.
fn format_cpu(percent: f64) -> String {
    if percent < 0.05 {
        "-".to_string()
    } else if percent >= 100.0 {
        format!("{:.0}%", percent)
    } else {
        format!("{:.1}%", percent)
    }
}

/// Format task count.
fn format_tasks(tasks: u64) -> String {
    if tasks == 0 {
        "-".to_string()
    } else {
        format!("{}", tasks)
    }
}

// ── Display ───────────────────────────────────────────────────────────────

/// Display a snapshot of cgroup resource usage.
fn display_snapshot(
    snapshot: &Snapshot,
    order: SortOrder,
    raw: bool,
    only_with_tasks: bool,
    batch: bool,
) {
    let stdout = io::stdout();
    let mut out = stdout.lock();

    // Collect and sort entries
    let mut entries: Vec<&CgroupUsage> = snapshot.cgroups.values().collect();

    // Filter if requested
    if only_with_tasks {
        entries.retain(|e| e.tasks > 0);
    }

    // Sort
    match order {
        SortOrder::Cpu => entries.sort_by(|a, b| {
            b.cpu_percent
                .partial_cmp(&a.cpu_percent)
                .unwrap_or(std::cmp::Ordering::Equal)
        }),
        SortOrder::Memory => entries.sort_by(|a, b| b.memory_bytes.cmp(&a.memory_bytes)),
        SortOrder::Io => entries.sort_by(|a, b| {
            let a_total = a.io_read_bytes.saturating_add(a.io_write_bytes);
            let b_total = b.io_read_bytes.saturating_add(b.io_write_bytes);
            b_total.cmp(&a_total)
        }),
        SortOrder::Tasks => entries.sort_by(|a, b| b.tasks.cmp(&a.tasks)),
        SortOrder::Path => entries.sort_by(|a, b| a.path.cmp(&b.path)),
    }

    // Clear screen for interactive mode
    if !batch {
        let _ = write!(out, "\x1b[2J\x1b[H");
    }

    // Header
    let _ = writeln!(
        out,
        "{:<50} {:>7} {:>8} {:>10} {:>10}",
        "Control Group", "Tasks", "%CPU", "Memory", "Input/s"
    );

    // Separator in batch mode
    if batch {
        let _ = writeln!(out, "{}", "-".repeat(89));
    }

    // Entries
    for entry in &entries {
        let path_display = if entry.path.len() > 50 {
            let start = entry.path.len() - 47;
            format!("...{}", &entry.path[start..])
        } else {
            entry.path.clone()
        };

        if raw {
            let _ = writeln!(
                out,
                "{:<50} {:>7} {:>8} {:>10} {:>10}",
                path_display,
                entry.tasks,
                format!("{:.1}%", entry.cpu_percent),
                entry.memory_bytes,
                entry.io_read_bytes.saturating_add(entry.io_write_bytes),
            );
        } else {
            let _ = writeln!(
                out,
                "{:<50} {:>7} {:>8} {:>10} {:>10}",
                path_display,
                format_tasks(entry.tasks),
                format_cpu(entry.cpu_percent),
                format_bytes(entry.memory_bytes),
                format_bytes(entry.io_read_bytes.saturating_add(entry.io_write_bytes)),
            );
        }
    }

    let _ = out.flush();
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // Determine sort order: short flags take precedence over --order
    let order = if cli.order_path {
        SortOrder::Path
    } else if cli.order_tasks {
        SortOrder::Tasks
    } else if cli.order_cpu {
        SortOrder::Cpu
    } else if cli.order_memory {
        SortOrder::Memory
    } else if cli.order_io {
        SortOrder::Io
    } else {
        SortOrder::from_str(&cli.order)
    };

    // -1 flag forces one iteration
    let iterations = if cli.one_shot { 1 } else { cli.iterations };

    let cgroup_root = if let Some(ref cg) = cli.cgroup {
        let cg = cg.trim_start_matches('/');
        if cg.is_empty() {
            PathBuf::from(CGROUP_ROOT)
        } else {
            PathBuf::from(CGROUP_ROOT).join(cg)
        }
    } else {
        PathBuf::from(CGROUP_ROOT)
    };

    if !cgroup_root.exists() {
        eprintln!(
            "Failed to access cgroup hierarchy at {}: No such file or directory",
            cgroup_root.display()
        );
        process::exit(1);
    }

    let is_one_shot = iterations == 1 || cli.delay == 0.0;
    let batch_mode = cli.batch || is_one_shot;

    // Take initial snapshot
    let mut prev_snapshot = take_snapshot(&cgroup_root, cli.depth);

    if is_one_shot {
        display_snapshot(
            &prev_snapshot,
            order,
            cli.raw,
            cli.count_userspace_processes,
            batch_mode,
        );
        return;
    }

    let delay = Duration::from_secs_f64(cli.delay);
    let mut iteration = 0u32;

    loop {
        thread::sleep(delay);

        let mut curr_snapshot = take_snapshot(&cgroup_root, cli.depth);
        compute_cpu_percentages(&prev_snapshot, &mut curr_snapshot);

        display_snapshot(
            &curr_snapshot,
            order,
            cli.raw,
            cli.count_userspace_processes,
            batch_mode,
        );

        prev_snapshot = curr_snapshot;
        iteration += 1;

        if iterations > 0 && iteration >= iterations {
            break;
        }
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // SortOrder tests

    #[test]
    fn test_sort_order_from_str_cpu() {
        assert_eq!(SortOrder::from_str("cpu"), SortOrder::Cpu);
        assert_eq!(SortOrder::from_str("c"), SortOrder::Cpu);
        assert_eq!(SortOrder::from_str("CPU"), SortOrder::Cpu);
    }

    #[test]
    fn test_sort_order_from_str_memory() {
        assert_eq!(SortOrder::from_str("memory"), SortOrder::Memory);
        assert_eq!(SortOrder::from_str("mem"), SortOrder::Memory);
        assert_eq!(SortOrder::from_str("m"), SortOrder::Memory);
    }

    #[test]
    fn test_sort_order_from_str_io() {
        assert_eq!(SortOrder::from_str("io"), SortOrder::Io);
        assert_eq!(SortOrder::from_str("i"), SortOrder::Io);
    }

    #[test]
    fn test_sort_order_from_str_tasks() {
        assert_eq!(SortOrder::from_str("tasks"), SortOrder::Tasks);
        assert_eq!(SortOrder::from_str("t"), SortOrder::Tasks);
    }

    #[test]
    fn test_sort_order_from_str_path() {
        assert_eq!(SortOrder::from_str("path"), SortOrder::Path);
        assert_eq!(SortOrder::from_str("p"), SortOrder::Path);
        assert_eq!(SortOrder::from_str("name"), SortOrder::Path);
    }

    // Unknown order now exits with error (validated by CLI parser),
    // so no unit test for unknown order fallback.

    // Formatting tests

    #[test]
    fn test_format_bytes_zero() {
        assert_eq!(format_bytes(0), "-");
    }

    #[test]
    fn test_format_bytes_small() {
        assert_eq!(format_bytes(512), "512B");
    }

    #[test]
    fn test_format_bytes_kib() {
        assert_eq!(format_bytes(1024), "1.0K");
        assert_eq!(format_bytes(1536), "1.5K");
    }

    #[test]
    fn test_format_bytes_mib() {
        assert_eq!(format_bytes(1024 * 1024), "1.0M");
        assert_eq!(format_bytes(10 * 1024 * 1024), "10.0M");
    }

    #[test]
    fn test_format_bytes_gib() {
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0G");
    }

    #[test]
    fn test_format_bytes_tib() {
        assert_eq!(format_bytes(1024u64 * 1024 * 1024 * 1024), "1.0T");
    }

    #[test]
    fn test_format_cpu_zero() {
        assert_eq!(format_cpu(0.0), "-");
    }

    #[test]
    fn test_format_cpu_small() {
        assert_eq!(format_cpu(0.01), "-");
    }

    #[test]
    fn test_format_cpu_normal() {
        assert_eq!(format_cpu(5.5), "5.5%");
    }

    #[test]
    fn test_format_cpu_high() {
        assert_eq!(format_cpu(100.0), "100%");
    }

    #[test]
    fn test_format_tasks_zero() {
        assert_eq!(format_tasks(0), "-");
    }

    #[test]
    fn test_format_tasks_nonzero() {
        assert_eq!(format_tasks(42), "42");
    }

    // CPU range parsing tests

    #[test]
    fn test_parse_cpu_range_single() {
        assert_eq!(parse_cpu_range("0"), Some(1));
    }

    #[test]
    fn test_parse_cpu_range_range() {
        assert_eq!(parse_cpu_range("0-3"), Some(4));
    }

    #[test]
    fn test_parse_cpu_range_multiple() {
        assert_eq!(parse_cpu_range("0-3,5-7"), Some(7));
    }

    #[test]
    fn test_parse_cpu_range_mixed() {
        assert_eq!(parse_cpu_range("0-1,3,5-7"), Some(6));
    }

    #[test]
    fn test_parse_cpu_range_with_whitespace() {
        assert_eq!(parse_cpu_range("0-7\n"), Some(8));
    }

    #[test]
    fn test_parse_cpu_range_invalid() {
        assert_eq!(parse_cpu_range("abc"), None);
    }

    // num_cpus test

    #[test]
    fn test_num_cpus_positive() {
        let n = num_cpus();
        assert!(n >= 1);
    }

    // Cgroup reading tests

    #[test]
    fn test_read_task_count_nonexistent() {
        assert_eq!(read_task_count(Path::new("/nonexistent")), 0);
    }

    #[test]
    fn test_read_cpu_usage_nonexistent() {
        assert_eq!(read_cpu_usage(Path::new("/nonexistent")), 0);
    }

    #[test]
    fn test_read_memory_usage_nonexistent() {
        assert_eq!(read_memory_usage(Path::new("/nonexistent")), 0);
    }

    #[test]
    fn test_read_io_usage_nonexistent() {
        assert_eq!(read_io_usage(Path::new("/nonexistent")), (0, 0));
    }

    #[test]
    fn test_read_task_count_root_cgroup() {
        if Path::new(CGROUP_ROOT).exists() {
            // Root cgroup should report some tasks on a running system
            let count = read_task_count(Path::new(CGROUP_ROOT));
            // Don't assert > 0 because root cgroup may have all tasks in children
            let _ = count;
        }
    }

    // Snapshot tests

    #[test]
    fn test_take_snapshot_no_panic() {
        if Path::new(CGROUP_ROOT).exists() {
            let snapshot = take_snapshot(Path::new(CGROUP_ROOT), Some(1));
            assert!(!snapshot.cgroups.is_empty());
        }
    }

    #[test]
    fn test_take_snapshot_nonexistent() {
        let snapshot = take_snapshot(Path::new("/nonexistent"), None);
        // Should still have at least the root entry
        assert!(snapshot.cgroups.len() <= 1);
    }

    #[test]
    fn test_take_snapshot_depth_zero() {
        if Path::new(CGROUP_ROOT).exists() {
            let snapshot = take_snapshot(Path::new(CGROUP_ROOT), Some(0));
            // With depth 0, should only have the root cgroup
            assert_eq!(snapshot.cgroups.len(), 1);
            assert!(snapshot.cgroups.contains_key("/"));
        }
    }

    // CPU percentage computation tests

    #[test]
    fn test_compute_cpu_percentages_no_prev_entry() {
        let prev = Snapshot {
            timestamp: Instant::now(),
            cgroups: HashMap::new(),
        };

        let mut curr = Snapshot {
            timestamp: Instant::now(),
            cgroups: HashMap::new(),
        };
        curr.cgroups.insert(
            "/test".to_string(),
            CgroupUsage {
                path: "/test".to_string(),
                tasks: 1,
                cpu_usec: 1000,
                cpu_percent: 0.0,
                memory_bytes: 0,
                io_read_bytes: 0,
                io_write_bytes: 0,
            },
        );

        compute_cpu_percentages(&prev, &mut curr);
        // No previous entry means cpu_percent stays at 0.0
        assert_eq!(curr.cgroups["/test"].cpu_percent, 0.0);
    }

    #[test]
    fn test_compute_cpu_percentages_same_timestamp() {
        let now = Instant::now();

        let mut prev_cgroups = HashMap::new();
        prev_cgroups.insert(
            "/test".to_string(),
            CgroupUsage {
                path: "/test".to_string(),
                tasks: 1,
                cpu_usec: 1000,
                cpu_percent: 0.0,
                memory_bytes: 0,
                io_read_bytes: 0,
                io_write_bytes: 0,
            },
        );

        let prev = Snapshot {
            timestamp: now,
            cgroups: prev_cgroups,
        };

        let mut curr_cgroups = HashMap::new();
        curr_cgroups.insert(
            "/test".to_string(),
            CgroupUsage {
                path: "/test".to_string(),
                tasks: 1,
                cpu_usec: 2000,
                cpu_percent: 0.0,
                memory_bytes: 0,
                io_read_bytes: 0,
                io_write_bytes: 0,
            },
        );

        let mut curr = Snapshot {
            timestamp: now,
            cgroups: curr_cgroups,
        };

        compute_cpu_percentages(&prev, &mut curr);
        // Same timestamp means division by zero prevention — stays at 0
        assert_eq!(curr.cgroups["/test"].cpu_percent, 0.0);
    }

    // Discover cgroups test

    #[test]
    fn test_discover_cgroups_nonexistent() {
        let mut result = Vec::new();
        discover_cgroups(
            Path::new("/nonexistent"),
            Path::new("/nonexistent"),
            None,
            0,
            &mut result,
        );
        assert_eq!(result.len(), 1); // Still includes the root itself
    }

    #[test]
    fn test_discover_cgroups_depth_zero() {
        let mut result = Vec::new();
        discover_cgroups(
            Path::new(CGROUP_ROOT),
            Path::new(CGROUP_ROOT),
            Some(0),
            0,
            &mut result,
        );
        assert_eq!(result.len(), 1); // Only the root
    }

    #[test]
    fn test_discover_cgroups_root() {
        if Path::new(CGROUP_ROOT).exists() {
            let mut result = Vec::new();
            discover_cgroups(
                Path::new(CGROUP_ROOT),
                Path::new(CGROUP_ROOT),
                Some(1),
                0,
                &mut result,
            );
            // Should have at least the root
            assert!(!result.is_empty());
        }
    }

    // Display test (just checking it doesn't panic)

    #[test]
    fn test_display_snapshot_no_panic() {
        let mut cgroups = HashMap::new();
        cgroups.insert(
            "/".to_string(),
            CgroupUsage {
                path: "/".to_string(),
                tasks: 100,
                cpu_usec: 50000000,
                cpu_percent: 15.3,
                memory_bytes: 512 * 1024 * 1024,
                io_read_bytes: 1024 * 1024,
                io_write_bytes: 2048 * 1024,
            },
        );
        cgroups.insert(
            "/system.slice".to_string(),
            CgroupUsage {
                path: "/system.slice".to_string(),
                tasks: 50,
                cpu_usec: 30000000,
                cpu_percent: 8.2,
                memory_bytes: 256 * 1024 * 1024,
                io_read_bytes: 512 * 1024,
                io_write_bytes: 1024 * 1024,
            },
        );

        let snapshot = Snapshot {
            timestamp: Instant::now(),
            cgroups,
        };

        display_snapshot(&snapshot, SortOrder::Cpu, false, false, true);
        display_snapshot(&snapshot, SortOrder::Memory, true, false, true);
        display_snapshot(&snapshot, SortOrder::Tasks, false, true, true);
        display_snapshot(&snapshot, SortOrder::Path, false, false, true);
        display_snapshot(&snapshot, SortOrder::Io, false, false, true);
    }

    // CgroupUsage construction test

    #[test]
    fn test_cgroup_usage_fields() {
        let usage = CgroupUsage {
            path: "/system.slice/sshd.service".to_string(),
            tasks: 3,
            cpu_usec: 1_000_000,
            cpu_percent: 2.5,
            memory_bytes: 10 * 1024 * 1024,
            io_read_bytes: 4096,
            io_write_bytes: 8192,
        };

        assert_eq!(usage.path, "/system.slice/sshd.service");
        assert_eq!(usage.tasks, 3);
        assert_eq!(usage.cpu_usec, 1_000_000);
        assert!((usage.cpu_percent - 2.5).abs() < f64::EPSILON);
        assert_eq!(usage.memory_bytes, 10 * 1024 * 1024);
        assert_eq!(usage.io_read_bytes, 4096);
        assert_eq!(usage.io_write_bytes, 8192);
    }
}
