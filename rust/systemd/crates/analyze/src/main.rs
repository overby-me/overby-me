//! systemd-analyze — Analyze and debug the system manager.
//!
//! A drop-in replacement for `systemd-analyze(1)`. Provides various
//! subcommands for inspecting boot performance, validating unit files,
//! parsing calendar/time expressions, and querying the service manager.

use clap::{Parser, Subcommand};
use libsystemd::calendar_spec::CalendarSpec;
use std::fmt;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ── CLI ───────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "systemd-analyze",
    about = "Analyze and debug the system manager",
    version,
    disable_help_subcommand = true
)]
struct Cli {
    /// Operate on the system instance (default)
    #[arg(long, global = true)]
    system: bool,

    /// Operate on the user instance
    #[arg(long, global = true)]
    user: bool,

    /// Do not pipe output into a pager
    #[arg(long, global = true)]
    no_pager: bool,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show time spent in the kernel, initrd, and userspace during boot
    Time,

    /// List running units ordered by time to initialize
    Blame {
        /// Do not pipe output into a pager
        #[arg(long)]
        no_pager: bool,
    },

    /// Print a tree of the time-critical chain of units
    #[command(name = "critical-chain")]
    CriticalChain {
        /// Unit(s) to show the chain for (default: default.target)
        units: Vec<String>,

        /// Fuzz timespan for chain analysis
        #[arg(long)]
        fuzz: Option<String>,

        /// Do not pipe output into a pager
        #[arg(long)]
        no_pager: bool,
    },

    /// Generate a dependency graph in dot(1) format
    Dot {
        /// Pattern(s) to filter unit names (glob)
        patterns: Vec<String>,

        /// Show 'Requires' dependencies
        #[arg(long)]
        require: bool,

        /// Show ordering dependencies (After/Before)
        #[arg(long)]
        order: bool,

        /// From pattern (can be specified multiple times)
        #[arg(long)]
        from_pattern: Vec<String>,

        /// To pattern (can be specified multiple times)
        #[arg(long)]
        to_pattern: Vec<String>,
    },

    /// Normalize a calendar time specification and calculate next elapse
    Calendar {
        /// Calendar expression(s) to parse
        expressions: Vec<String>,

        /// Number of iterations to show
        #[arg(short = 'i', long, default_value_t = 1)]
        iterations: u32,

        /// Base time for relative calculations
        #[arg(long)]
        base_time: Option<String>,
    },

    /// Normalize a time span specification
    Timespan {
        /// Time span expression(s) to parse
        expressions: Vec<String>,
    },

    /// Normalize a timestamp specification
    Timestamp {
        /// Timestamp expression(s) to parse
        expressions: Vec<String>,
    },

    /// Verify unit file(s) for correctness
    Verify {
        /// Unit file(s) to verify
        files: Vec<String>,

        /// Also check man page references
        #[arg(long, num_args = 0..=1, default_missing_value = "yes")]
        man: Option<String>,

        /// Control recursive error checking
        #[arg(long, value_name = "MODE")]
        recursive_errors: Option<String>,

        /// Also check generator output
        #[arg(long)]
        generators: Option<String>,
    },

    /// Evaluate Condition*= and Assert*= expressions
    Condition {
        /// Condition expression(s) to evaluate
        expressions: Vec<String>,

        /// Unit to evaluate conditions for
        #[arg(long)]
        unit: Option<String>,
    },

    /// List unit file search paths
    #[command(name = "unit-paths")]
    UnitPaths,

    /// Get/set the log level of the manager
    #[command(name = "log-level")]
    LogLevel {
        /// New log level to set (omit to query current level)
        level: Option<String>,
    },

    /// Get/set the log target of the manager
    #[command(name = "log-target")]
    LogTarget {
        /// New log target to set (omit to query current target)
        target: Option<String>,
    },

    /// Get/set service watchdog state
    #[command(name = "service-watchdogs")]
    ServiceWatchdogs {
        /// New state (yes/no) (omit to query current state)
        state: Option<String>,
    },

    /// Get the log level (legacy alias)
    #[command(name = "get-log-level")]
    GetLogLevel,

    /// Set the log level (legacy alias)
    #[command(name = "set-log-level")]
    SetLogLevel {
        /// Log level to set
        level: String,
    },

    /// Get the log target (legacy alias)
    #[command(name = "get-log-target")]
    GetLogTarget,

    /// Set the log target (legacy alias)
    #[command(name = "set-log-target")]
    SetLogTarget {
        /// Log target to set
        target: String,
    },

    /// Show service security assessment
    Security {
        /// Unit(s) to assess (default: all loaded services)
        units: Vec<String>,

        /// Do not pipe output into a pager
        #[arg(long)]
        no_pager: bool,
    },

    /// Generate an SVG plot of the boot sequence
    Plot,

    /// Inspect an ELF binary for embedded metadata
    #[command(name = "inspect-elf")]
    InspectElf {
        /// ELF binary path(s) to inspect
        files: Vec<String>,
    },

    /// Show file descriptor store contents of a service
    Fdstore {
        /// Service unit name
        unit: String,
    },

    /// Analyze image dissection policies
    #[command(name = "image-policy")]
    ImagePolicy {
        /// Policy string(s) to parse and normalize
        policies: Vec<String>,
    },

    /// Show full contents of configuration files
    #[command(name = "cat-config")]
    CatConfig {
        /// Configuration file name(s) (e.g. "systemd/system.conf")
        items: Vec<String>,

        /// Show only non-comment, non-empty lines
        #[arg(long)]
        tldr: bool,
    },

    /// List unit files from search paths
    #[command(name = "unit-files")]
    UnitFiles {
        /// Pattern(s) to filter unit files (glob)
        patterns: Vec<String>,
    },

    /// Show TPM2 PCR (Platform Configuration Register) values
    Pcrs,

    /// Show the TPM2 SRK (Storage Root Key)
    Srk,

    /// Map systemd-specific exit statuses to/from their names
    #[command(name = "exit-status")]
    ExitStatus {
        /// Status name(s) or number(s) to look up (omit for full table)
        statuses: Vec<String>,
    },

    /// Map Linux capability names to/from their numbers
    Capability {
        /// Capability name(s) or number(s) to look up (omit for full table)
        capabilities: Vec<String>,

        /// Look up capabilities from a hex mask
        #[arg(short = 'm', long)]
        mask: Option<String>,
    },
}

// ── Boot timing data structures ───────────────────────────────────────────

/// Timing info for a single unit, read from /run/rust-systemd/timing/
#[derive(Debug, Clone)]
struct UnitTiming {
    name: String,
    /// Microseconds from manager start to unit activation start
    activating_us: u64,
    /// Microseconds from manager start to unit becoming active
    active_us: u64,
}

impl UnitTiming {
    fn duration_us(&self) -> u64 {
        self.active_us.saturating_sub(self.activating_us)
    }
}

/// Overall boot timing phases
#[derive(Debug, Clone, Default)]
struct BootTiming {
    firmware_us: Option<u64>,
    loader_us: Option<u64>,
    kernel_us: Option<u64>,
    initrd_us: Option<u64>,
    userspace_us: Option<u64>,
    total_us: Option<u64>,
}

// ── Time span constants ───────────────────────────────────────────────────

const USEC_PER_SEC: u64 = 1_000_000;
const USEC_PER_MSEC: u64 = 1_000;
const USEC_PER_MINUTE: u64 = 60 * USEC_PER_SEC;
const USEC_PER_HOUR: u64 = 60 * USEC_PER_MINUTE;
const USEC_PER_DAY: u64 = 24 * USEC_PER_HOUR;
const USEC_PER_WEEK: u64 = 7 * USEC_PER_DAY;
const USEC_PER_MONTH: u64 = 2_629_800 * USEC_PER_SEC; // 30.44 days
const USEC_PER_YEAR: u64 = 31_557_600 * USEC_PER_SEC; // 365.25 days

// ── Time span parsing ─────────────────────────────────────────────────────

/// A parsed time span value.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct TimeSpan {
    usec: u64,
}

impl TimeSpan {
    fn parse(input: &str) -> Result<Self, String> {
        let input = input.trim();

        if input == "infinity" {
            return Ok(TimeSpan { usec: u64::MAX });
        }

        // Try parsing as a plain number (treated as microseconds)
        if let Ok(v) = input.parse::<u64>() {
            return Ok(TimeSpan { usec: v });
        }

        let mut total: u64 = 0;
        let mut remaining = input;
        let mut found_any = false;

        while !remaining.is_empty() {
            remaining = remaining.trim_start();
            if remaining.is_empty() {
                break;
            }

            // Parse the numeric part
            let num_end = remaining
                .find(|c: char| !c.is_ascii_digit() && c != '.')
                .unwrap_or(remaining.len());

            if num_end == 0 {
                return Err(format!("Failed to parse time span: {input}"));
            }

            let num_str = &remaining[..num_end];
            let value: f64 = num_str
                .parse()
                .map_err(|_| format!("Invalid number in time span: {num_str}"))?;

            remaining = remaining[num_end..].trim_start();

            // Parse the unit suffix
            let (multiplier, unit_len) = parse_time_unit(remaining)?;

            total = total.saturating_add((value * multiplier as f64) as u64);
            remaining = &remaining[unit_len..];
            found_any = true;
        }

        if !found_any {
            return Err(format!("Failed to parse time span: {input}"));
        }

        Ok(TimeSpan { usec: total })
    }
}

impl fmt::Display for TimeSpan {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.usec == u64::MAX {
            return write!(f, "infinity");
        }
        if self.usec == 0 {
            return write!(f, "0");
        }

        let mut remaining = self.usec;
        let mut parts = Vec::new();

        let units: &[(u64, &str)] = &[
            (USEC_PER_YEAR, "y"),
            (USEC_PER_MONTH, "month"),
            (USEC_PER_WEEK, "w"),
            (USEC_PER_DAY, "d"),
            (USEC_PER_HOUR, "h"),
            (USEC_PER_MINUTE, "min"),
            (USEC_PER_SEC, "s"),
            (USEC_PER_MSEC, "ms"),
            (1, "us"),
        ];

        for &(divisor, suffix) in units {
            if remaining >= divisor {
                let count = remaining / divisor;
                remaining %= divisor;
                parts.push(format!("{count}{suffix}"));
            }
        }

        write!(f, "{}", parts.join(" "))
    }
}

/// Format microseconds into a human-readable duration string.
fn format_usec(usec: u64) -> String {
    if usec == 0 {
        return "0".to_string();
    }

    if usec < USEC_PER_MSEC {
        return format!("{}us", usec);
    }
    if usec < USEC_PER_SEC {
        let ms = usec as f64 / USEC_PER_MSEC as f64;
        return format!("{ms:.0}ms");
    }
    if usec < USEC_PER_MINUTE {
        let s = usec as f64 / USEC_PER_SEC as f64;
        return format!("{s:.3}s");
    }

    let span = TimeSpan { usec };
    format!("{span}")
}

/// Parse a time unit suffix and return (multiplier_in_usec, chars_consumed).
fn parse_time_unit(s: &str) -> Result<(u64, usize), String> {
    let units: &[(&str, u64)] = &[
        ("microseconds", 1),
        ("microsecond", 1),
        ("milliseconds", USEC_PER_MSEC),
        ("millisecond", USEC_PER_MSEC),
        ("minutes", USEC_PER_MINUTE),
        ("minute", USEC_PER_MINUTE),
        ("months", USEC_PER_MONTH),
        ("month", USEC_PER_MONTH),
        ("seconds", USEC_PER_SEC),
        ("second", USEC_PER_SEC),
        ("hours", USEC_PER_HOUR),
        ("hour", USEC_PER_HOUR),
        ("weeks", USEC_PER_WEEK),
        ("week", USEC_PER_WEEK),
        ("days", USEC_PER_DAY),
        ("day", USEC_PER_DAY),
        ("years", USEC_PER_YEAR),
        ("year", USEC_PER_YEAR),
        ("msec", USEC_PER_MSEC),
        ("usec", 1),
        ("min", USEC_PER_MINUTE),
        ("ms", USEC_PER_MSEC),
        ("us", 1),
        ("sec", USEC_PER_SEC),
        ("s", USEC_PER_SEC),
        ("m", USEC_PER_MINUTE),
        ("h", USEC_PER_HOUR),
        ("d", USEC_PER_DAY),
        ("w", USEC_PER_WEEK),
        ("y", USEC_PER_YEAR),
    ];

    let lower = s.to_lowercase();
    for &(suffix, mult) in units {
        if lower.starts_with(suffix) {
            let after = &s[suffix.len()..];
            if after.is_empty()
                || after.starts_with(' ')
                || after.starts_with(|c: char| c.is_ascii_digit())
            {
                return Ok((mult, suffix.len()));
            }
        }
    }

    // If no unit, assume microseconds
    if s.is_empty() || s.starts_with(|c: char| c.is_ascii_digit()) {
        return Ok((1, 0));
    }

    Err(format!("Unknown time unit in: {s}"))
}

// CalendarSpec is provided by libsystemd::calendar_spec::CalendarSpec

// ── Timestamp parsing ─────────────────────────────────────────────────────

fn parse_timestamp(input: &str) -> Result<SystemTime, String> {
    let input = input.trim();

    match input.to_lowercase().as_str() {
        "now" => return Ok(SystemTime::now()),
        "today" => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?;
            let secs = now.as_secs();
            let day_start = secs - (secs % 86400);
            return Ok(UNIX_EPOCH + Duration::from_secs(day_start));
        }
        "yesterday" => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?;
            let secs = now.as_secs();
            let day_start = secs - (secs % 86400) - 86400;
            return Ok(UNIX_EPOCH + Duration::from_secs(day_start));
        }
        "tomorrow" => {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_err(|e| e.to_string())?;
            let secs = now.as_secs();
            let day_start = secs - (secs % 86400) + 86400;
            return Ok(UNIX_EPOCH + Duration::from_secs(day_start));
        }
        "epoch" => return Ok(UNIX_EPOCH),
        _ => {}
    }

    // Relative: "+5min", "-2h"
    if let Some(rest) = input.strip_prefix('+') {
        let span = TimeSpan::parse(rest)?;
        return Ok(SystemTime::now() + Duration::from_micros(span.usec));
    }
    if let Some(rest) = input.strip_prefix('-') {
        let span = TimeSpan::parse(rest)?;
        return SystemTime::now()
            .checked_sub(Duration::from_micros(span.usec))
            .ok_or_else(|| "Timestamp would be before UNIX epoch".to_string());
    }

    // "N ago"
    if let Some(rest) = input.strip_suffix("ago") {
        let span = TimeSpan::parse(rest.trim())?;
        return SystemTime::now()
            .checked_sub(Duration::from_micros(span.usec))
            .ok_or_else(|| "Timestamp would be before UNIX epoch".to_string());
    }

    // "@EPOCH_SECONDS"
    if let Some(rest) = input.strip_prefix('@') {
        let secs: u64 = rest
            .trim()
            .parse()
            .map_err(|_| format!("Invalid epoch timestamp: {rest}"))?;
        return Ok(UNIX_EPOCH + Duration::from_secs(secs));
    }

    // YYYY-MM-DD HH:MM:SS
    if let Some(ts) = try_parse_datetime(input) {
        return Ok(ts);
    }

    Err(format!("Failed to parse timestamp: {input}"))
}

fn try_parse_datetime(input: &str) -> Option<SystemTime> {
    let parts: Vec<&str> = input.splitn(2, [' ', 'T']).collect();

    let date_str = parts.first()?;
    let time_str = parts.get(1).copied().unwrap_or("00:00:00");

    let date_parts: Vec<&str> = date_str.split('-').collect();
    if date_parts.len() != 3 {
        return None;
    }

    let year: i64 = date_parts[0].parse().ok()?;
    let month: u32 = date_parts[1].parse().ok()?;
    let day: u32 = date_parts[2].parse().ok()?;

    if !(1..=12).contains(&month) || !(1..=31).contains(&day) {
        return None;
    }

    let time_parts: Vec<&str> = time_str.split(':').collect();
    let hour: u32 = time_parts.first().and_then(|s| s.parse().ok()).unwrap_or(0);
    let minute: u32 = time_parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
    let second: u32 = time_parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);

    if hour > 23 || minute > 59 || second > 60 {
        return None;
    }

    let days = days_from_civil(year, month, day);
    let secs = days as u64 * 86400 + hour as u64 * 3600 + minute as u64 * 60 + second as u64;

    Some(UNIX_EPOCH + Duration::from_secs(secs))
}

/// Convert a civil date to days since UNIX epoch (Howard Hinnant algorithm).
fn days_from_civil(year: i64, month: u32, day: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let m = if month <= 2 {
        month as i64 + 9
    } else {
        month as i64 - 3
    };
    let era = if y >= 0 { y / 400 } else { (y - 399) / 400 };
    let yoe = (y - era * 400) as u64;
    let doy = (153 * m as u64 + 2) / 5 + day as u64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe as i64 - 719468
}

/// Convert days since UNIX epoch to civil date.
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719468;
    let era = if z >= 0 {
        z / 146097
    } else {
        (z - 146096) / 146097
    };
    let doe = (z - era * 146097) as u64;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    (y, m as u32, d as u32)
}

/// Day of week from days since epoch (0=Thu, 1=Fri, ..., 6=Wed).
fn weekday_from_days(days: i64) -> u32 {
    days.rem_euclid(7) as u32
}

/// Format a SystemTime as a human-readable timestamp.
fn format_timestamp(t: SystemTime) -> String {
    let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
    let total_secs = dur.as_secs();
    let days = (total_secs / 86400) as i64;
    let tod = total_secs % 86400;
    let hour = tod / 3600;
    let minute = (tod % 3600) / 60;
    let second = tod % 60;
    let (year, month, day) = civil_from_days(days);
    let wd = weekday_from_days(days);
    let dow = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let dow_name = dow[wd as usize % 7];
    format!(
        "{} {}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
        dow_name, year, month, day, hour, minute, second
    )
}

// ── Boot timing data ──────────────────────────────────────────────────────

/// Read boot timing from /run/rust-systemd/boot-timing if available.
fn read_boot_timing() -> BootTiming {
    let mut bt = BootTiming::default();
    let path = Path::new("/run/rust-systemd/boot-timing");
    if let Ok(content) = fs::read_to_string(path) {
        for line in content.lines() {
            let line = line.trim();
            if let Some((key, val)) = line.split_once('=') {
                let val: u64 = match val.trim().parse() {
                    Ok(v) => v,
                    Err(_) => continue,
                };
                match key.trim() {
                    "firmware" => bt.firmware_us = Some(val),
                    "loader" => bt.loader_us = Some(val),
                    "kernel" => bt.kernel_us = Some(val),
                    "initrd" => bt.initrd_us = Some(val),
                    "userspace" => bt.userspace_us = Some(val),
                    "total" => bt.total_us = Some(val),
                    _ => {}
                }
            }
        }
    }
    // Try /proc/stat for kernel boot time as fallback
    if bt.kernel_us.is_none()
        && let Ok(content) = fs::read_to_string("/proc/stat")
    {
        for line in content.lines() {
            if let Some(rest) = line.strip_prefix("btime ")
                && let Ok(btime) = rest.trim().parse::<u64>()
                && let Ok(now) = SystemTime::now().duration_since(UNIX_EPOCH)
            {
                let uptime_s = now.as_secs().saturating_sub(btime);
                bt.kernel_us = Some(uptime_s * USEC_PER_SEC);
            }
        }
    }
    bt
}

/// Read per-unit timing data from /run/rust-systemd/timing/.
fn read_unit_timings() -> Vec<UnitTiming> {
    let dir = Path::new("/run/rust-systemd/timing");
    let mut timings = Vec::new();
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if let Ok(content) = fs::read_to_string(entry.path()) {
                let mut activating = None;
                let mut active = None;
                for line in content.lines() {
                    if let Some((k, v)) = line.split_once('=') {
                        let v: u64 = match v.trim().parse() {
                            Ok(x) => x,
                            Err(_) => continue,
                        };
                        match k.trim() {
                            "activating" => activating = Some(v),
                            "active" => active = Some(v),
                            _ => {}
                        }
                    }
                }
                if let (Some(a), Some(b)) = (activating, active) {
                    timings.push(UnitTiming {
                        name,
                        activating_us: a,
                        active_us: b,
                    });
                }
            }
        }
    }
    timings
}

// ── Condition evaluation ──────────────────────────────────────────────────

fn evaluate_condition(expr: &str) -> (bool, &'static str) {
    let (negated, key, value) = if let Some((k, v)) = expr.split_once('=') {
        let (neg, k) = if let Some(stripped) = k.strip_suffix('!') {
            (true, stripped)
        } else {
            (false, k)
        };
        (neg, k.trim(), v.trim())
    } else {
        return (false, "Invalid condition expression (expected KEY=VALUE)");
    };

    let result = match key {
        "ConditionPathExists" | "AssertPathExists" => {
            let (negate_path, path) = if let Some(p) = value.strip_prefix('!') {
                (true, p)
            } else {
                (false, value)
            };
            let exists = Path::new(path).exists();
            if negate_path { !exists } else { exists }
        }
        "ConditionPathIsDirectory" | "AssertPathIsDirectory" => {
            let (np, path) = if let Some(p) = value.strip_prefix('!') {
                (true, p)
            } else {
                (false, value)
            };
            let is_dir = Path::new(path).is_dir();
            if np { !is_dir } else { is_dir }
        }
        "ConditionPathIsSymbolicLink" | "AssertPathIsSymbolicLink" => {
            let (np, path) = if let Some(p) = value.strip_prefix('!') {
                (true, p)
            } else {
                (false, value)
            };
            let is_link = Path::new(path).is_symlink();
            if np { !is_link } else { is_link }
        }
        "ConditionFileIsExecutable" | "AssertFileIsExecutable" => {
            use std::os::unix::fs::PermissionsExt;
            let (np, path) = if let Some(p) = value.strip_prefix('!') {
                (true, p)
            } else {
                (false, value)
            };
            let is_exec = fs::metadata(path)
                .map(|m| m.permissions().mode() & 0o111 != 0)
                .unwrap_or(false);
            if np { !is_exec } else { is_exec }
        }
        "ConditionFileNotEmpty" | "AssertFileNotEmpty" => {
            let (np, path) = if let Some(p) = value.strip_prefix('!') {
                (true, p)
            } else {
                (false, value)
            };
            let not_empty = fs::metadata(path).map(|m| m.len() > 0).unwrap_or(false);
            if np { !not_empty } else { not_empty }
        }
        "ConditionDirectoryNotEmpty" | "AssertDirectoryNotEmpty" => {
            let (np, path) = if let Some(p) = value.strip_prefix('!') {
                (true, p)
            } else {
                (false, value)
            };
            let not_empty = fs::read_dir(path)
                .map(|mut d| d.next().is_some())
                .unwrap_or(false);
            if np { !not_empty } else { not_empty }
        }
        "ConditionVirtualization" | "AssertVirtualization" => {
            let detected = detect_virtualization();
            if value.is_empty() || value == "yes" {
                detected.is_some()
            } else if value == "no" {
                detected.is_none()
            } else {
                detected.as_deref() == Some(value)
            }
        }
        "ConditionHost" | "AssertHost" => {
            let hostname = hostname();
            hostname.as_deref() == Some(value)
        }
        "ConditionKernelCommandLine" | "AssertKernelCommandLine" => {
            let cmdline = fs::read_to_string("/proc/cmdline").unwrap_or_default();
            cmdline
                .split_whitespace()
                .any(|arg| arg == value || arg.starts_with(&format!("{value}=")))
        }
        _ => {
            return (false, "Unknown condition type");
        }
    };

    let final_result = if negated { !result } else { result };
    if final_result {
        (true, "met")
    } else {
        (false, "not met")
    }
}

fn detect_virtualization() -> Option<String> {
    // Check /proc/cpuinfo for hypervisor flag
    if let Ok(cpuinfo) = fs::read_to_string("/proc/cpuinfo")
        && cpuinfo.contains("hypervisor")
    {
        // Try to identify the specific hypervisor
        if let Ok(dmi) = fs::read_to_string("/sys/class/dmi/id/product_name") {
            let name = dmi.trim().to_lowercase();
            if name.contains("kvm") || name.contains("qemu") {
                return Some("kvm".to_string());
            }
            if name.contains("virtualbox") {
                return Some("oracle".to_string());
            }
            if name.contains("vmware") {
                return Some("vmware".to_string());
            }
            if name.contains("cloud hypervisor") {
                return Some("kvm".to_string());
            }
            return Some("vm".to_string());
        }
        return Some("vm".to_string());
    }
    // Check for container
    if Path::new("/run/.containerenv").exists() {
        return Some("podman".to_string());
    }
    if Path::new("/.dockerenv").exists() {
        return Some("docker".to_string());
    }
    if let Ok(content) = fs::read_to_string("/proc/1/environ")
        && content.contains("container=")
    {
        return Some("container-other".to_string());
    }
    None
}

fn hostname() -> Option<String> {
    fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

// ── Unit paths ────────────────────────────────────────────────────────────

fn system_unit_paths() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/etc/systemd/system.control"),
        PathBuf::from("/run/systemd/system.control"),
        PathBuf::from("/run/systemd/transient"),
        PathBuf::from("/run/systemd/generator.early"),
        PathBuf::from("/etc/systemd/system"),
        PathBuf::from("/etc/systemd/system.attached"),
        PathBuf::from("/run/systemd/system"),
        PathBuf::from("/run/systemd/system.attached"),
        PathBuf::from("/run/systemd/generator"),
        PathBuf::from("/usr/local/lib/systemd/system"),
        PathBuf::from("/usr/lib/systemd/system"),
        PathBuf::from("/run/systemd/generator.late"),
    ]
}

fn user_unit_paths() -> Vec<PathBuf> {
    let home = std::env::var("HOME").unwrap_or_default();
    let xdg_config = std::env::var("XDG_CONFIG_HOME").unwrap_or_else(|_| format!("{home}/.config"));
    let uid = fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            s.lines()
                .find(|l| l.starts_with("Uid:"))
                .and_then(|l| l.split_whitespace().nth(1))
                .and_then(|v| v.parse::<u32>().ok())
        })
        .unwrap_or(0);
    let xdg_runtime =
        std::env::var("XDG_RUNTIME_DIR").unwrap_or_else(|_| format!("/run/user/{}", uid));
    let xdg_data =
        std::env::var("XDG_DATA_HOME").unwrap_or_else(|_| format!("{home}/.local/share"));

    vec![
        PathBuf::from(format!("{xdg_config}/systemd/user.control")),
        PathBuf::from(format!("{xdg_runtime}/systemd/user.control")),
        PathBuf::from(format!("{xdg_runtime}/systemd/transient")),
        PathBuf::from(format!("{xdg_runtime}/systemd/generator.early")),
        PathBuf::from(format!("{xdg_config}/systemd/user")),
        PathBuf::from(format!("{xdg_config}/systemd/user.attached")),
        PathBuf::from("/etc/systemd/user"),
        PathBuf::from(format!("{xdg_runtime}/systemd/user")),
        PathBuf::from("/run/systemd/user"),
        PathBuf::from(format!("{xdg_runtime}/systemd/generator")),
        PathBuf::from(format!("{xdg_data}/systemd/user")),
        PathBuf::from("/usr/local/lib/systemd/user"),
        PathBuf::from("/usr/lib/systemd/user"),
        PathBuf::from(format!("{xdg_runtime}/systemd/generator.late")),
    ]
}

// ── Unit file verification ────────────────────────────────────────────────

fn verify_unit_file(path: &str) -> Vec<String> {
    let mut issues = Vec::new();
    let content = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(e) => {
            issues.push(format!("{path}: Cannot open: {e}"));
            return issues;
        }
    };

    let mut has_unit_section = false;
    let mut has_service_section = false;
    let mut has_exec_start = false;
    let mut has_type = false;
    let mut current_section = String::new();

    for (lineno, line) in content.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_string();
            match current_section.as_str() {
                "Unit" => has_unit_section = true,
                "Service" => has_service_section = true,
                "Install" | "Socket" | "Timer" | "Mount" | "Automount" | "Swap" | "Path"
                | "Slice" | "Scope" => {}
                other => {
                    issues.push(format!(
                        "{path}:{}: Unknown section [{}]",
                        lineno + 1,
                        other
                    ));
                }
            }
            continue;
        }

        if !line.contains('=') {
            issues.push(format!(
                "{path}:{}: Line is not a key=value pair: {}",
                lineno + 1,
                line
            ));
            continue;
        }

        if let Some((key, _val)) = line.split_once('=') {
            let key = key.trim();
            if current_section.as_str() == "Service" {
                if key == "ExecStart" {
                    has_exec_start = true;
                }
                if key == "Type" {
                    has_type = true;
                }
            }
        }
    }

    if path.ends_with(".service") {
        if !has_unit_section && !has_service_section {
            issues.push(format!("{path}: Missing [Unit] and [Service] sections"));
        }
        if has_service_section && !has_exec_start && !has_type {
            issues.push(format!(
                "{path}: Service has no ExecStart= and no Type= setting"
            ));
        }
    }

    if issues.is_empty() {
        issues.push(format!("{path}: OK"));
    }

    issues
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    match cli.command {
        None | Some(Command::Time) => cmd_time(),
        Some(Command::Blame { .. }) => cmd_blame(),
        Some(Command::CriticalChain { ref units, .. }) => cmd_critical_chain(units),
        Some(Command::Dot {
            ref patterns,
            require,
            order,
            ref from_pattern,
            ref to_pattern,
        }) => cmd_dot(patterns, require, order, from_pattern, to_pattern),
        Some(Command::Calendar {
            ref expressions,
            iterations,
            ..
        }) => cmd_calendar(expressions, iterations),
        Some(Command::Timespan { ref expressions }) => cmd_timespan(expressions),
        Some(Command::Timestamp { ref expressions }) => cmd_timestamp(expressions),
        Some(Command::Verify { ref files, .. }) => cmd_verify(files),
        Some(Command::Condition {
            ref expressions,
            ref unit,
        }) => cmd_condition(expressions, unit.as_deref()),
        Some(Command::UnitPaths) => cmd_unit_paths(cli.user),
        Some(Command::LogLevel { ref level }) => cmd_log_level(level),
        Some(Command::LogTarget { ref target }) => cmd_log_target(target),
        Some(Command::ServiceWatchdogs { ref state }) => cmd_service_watchdogs(state),
        Some(Command::GetLogLevel) => cmd_log_level(&None),
        Some(Command::SetLogLevel { ref level }) => cmd_log_level(&Some(level.clone())),
        Some(Command::GetLogTarget) => cmd_log_target(&None),
        Some(Command::SetLogTarget { ref target }) => cmd_log_target(&Some(target.clone())),
        Some(Command::Security { ref units, .. }) => cmd_security(units),
        Some(Command::Plot) => cmd_plot(),
        Some(Command::InspectElf { ref files }) => cmd_inspect_elf(files),
        Some(Command::Fdstore { ref unit }) => cmd_fdstore(unit),
        Some(Command::ImagePolicy { ref policies }) => cmd_image_policy(policies),
        Some(Command::Pcrs) => cmd_pcrs(),
        Some(Command::Srk) => cmd_srk(),
        Some(Command::CatConfig { ref items, tldr }) => cmd_cat_config(items, tldr),
        Some(Command::UnitFiles { ref patterns }) => cmd_unit_files(patterns, cli.user),
        Some(Command::ExitStatus { ref statuses }) => cmd_exit_status(statuses),
        Some(Command::Capability {
            ref capabilities,
            ref mask,
        }) => cmd_capability(capabilities, mask.as_deref()),
    }
}

fn cmd_time() {
    let bt = read_boot_timing();

    println!("Startup finished in:");
    if let Some(k) = bt.kernel_us {
        println!("  kernel: {}", format_usec(k));
    }
    if let Some(i) = bt.initrd_us {
        println!("  initrd: {}", format_usec(i));
    }
    if let Some(u) = bt.userspace_us {
        println!("  userspace: {}", format_usec(u));
    }

    // If we have nothing from our timing file, try /proc/uptime
    if bt.kernel_us.is_none()
        && bt.initrd_us.is_none()
        && bt.userspace_us.is_none()
        && let Ok(content) = fs::read_to_string("/proc/uptime")
        && let Some(uptime_str) = content.split_whitespace().next()
        && let Ok(uptime_s) = uptime_str.parse::<f64>()
    {
        let uptime_us = (uptime_s * USEC_PER_SEC as f64) as u64;
        println!(
            "  (system running for {}; no detailed timing available)",
            format_usec(uptime_us)
        );
    }

    if let Some(t) = bt.total_us {
        println!("  total: {}", format_usec(t));
    }
}

fn cmd_blame() {
    let mut timings = read_unit_timings();

    if timings.is_empty() {
        // No timing data; show a message
        println!("No unit timing data available.");
        println!("(Boot timing data is written to /run/rust-systemd/timing/)");
        return;
    }

    timings.sort_by_key(|b| std::cmp::Reverse(b.duration_us()));

    for t in &timings {
        let dur = t.duration_us();
        println!("{:>10} {}", format_usec(dur), t.name);
    }
}

fn cmd_critical_chain(units: &[String]) {
    let target = if units.is_empty() {
        "default.target"
    } else {
        &units[0]
    };

    let timings = read_unit_timings();
    if timings.is_empty() {
        println!("The time when unit became active or started is not known.");
        println!("(No timing data in /run/rust-systemd/timing/)");
        return;
    }

    println!("The time when unit became active or started is printed after the \"@\" character.");
    println!("The time the unit took to start is printed after the \"+\" character.");
    println!();
    println!("{target}");

    // Find the target and its dependencies (simplified: just show sorted by activation time)
    let mut sorted = timings.clone();
    sorted.sort_by_key(|t| t.active_us);

    for t in sorted.iter().rev().take(10) {
        println!(
            "  {} @{} +{}",
            t.name,
            format_usec(t.activating_us),
            format_usec(t.duration_us())
        );
    }
}

fn cmd_dot(
    _patterns: &[String],
    _require: bool,
    _order: bool,
    _from_pattern: &[String],
    _to_pattern: &[String],
) {
    // Read unit files from standard paths and generate dot output
    println!("digraph systemd {{");
    println!("  rankdir=LR;");
    println!("  node [shape=box];");

    let search_paths = system_unit_paths();
    for dir in &search_paths {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if !path.is_file() {
                    continue;
                }
                let unit_name = entry.file_name().to_string_lossy().to_string();
                if let Ok(content) = fs::read_to_string(&path) {
                    for line in content.lines() {
                        let line = line.trim();
                        for prefix in &[
                            "Requires=",
                            "Wants=",
                            "After=",
                            "Before=",
                            "BindsTo=",
                            "PartOf=",
                            "Conflicts=",
                        ] {
                            if let Some(deps) = line.strip_prefix(prefix) {
                                let style = match *prefix {
                                    "Requires=" | "BindsTo=" => "bold",
                                    "Wants=" => "dashed",
                                    "After=" | "Before=" => "dotted",
                                    "Conflicts=" => "bold,color=red",
                                    "PartOf=" => "bold,color=blue",
                                    _ => "",
                                };
                                for dep in deps.split_whitespace() {
                                    println!(
                                        "  \"{}\" -> \"{}\" [style={}];",
                                        unit_name, dep, style
                                    );
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    println!("}}");
}

fn cmd_calendar(expressions: &[String], iterations: u32) {
    if expressions.is_empty() {
        eprintln!("No calendar expression specified.");
        process::exit(1);
    }

    for expr in expressions {
        match CalendarSpec::parse(expr) {
            Ok(spec) => {
                println!("  Original form: {}", spec.original);
                println!("Normalized form: {}", spec.normalized());

                if iterations > 0 {
                    let now = SystemTime::now();
                    let mut ref_dt = CalendarSpec::system_time_to_datetime(now);
                    // Start from one second after now for "next" semantics
                    ref_dt = ref_dt.add_second();

                    for i in 0..iterations {
                        if let Some(next) = spec.next_elapse(ref_dt) {
                            let next_unix = CalendarSpec::datetime_to_unix(&next);
                            let label = if i == 0 {
                                "    Next elapse".to_string()
                            } else {
                                format!("          Iter. #{}", i + 1)
                            };

                            // Format as a human-readable UTC timestamp
                            let weekday_name =
                                match libsystemd::calendar_spec::weekday_from_datetime(&next) {
                                    0 => "Mon",
                                    1 => "Tue",
                                    2 => "Wed",
                                    3 => "Thu",
                                    4 => "Fri",
                                    5 => "Sat",
                                    6 => "Sun",
                                    _ => "???",
                                };
                            println!(
                                "{}: {} {:04}-{:02}-{:02} {:02}:{:02}:{:02} UTC",
                                label,
                                weekday_name,
                                next.year,
                                next.month,
                                next.day,
                                next.hour,
                                next.minute,
                                next.second,
                            );

                            // Compute relative time from now
                            let now_unix = CalendarSpec::datetime_to_unix(
                                &CalendarSpec::system_time_to_datetime(now),
                            );
                            let diff = next_unix - now_unix;
                            if diff > 0 && i == 0 {
                                let d = Duration::from_secs(diff as u64);
                                println!("       From now: {}", format_relative_duration(d));
                            }

                            // Next iteration starts one second after this elapse
                            ref_dt = next.add_second();
                        } else {
                            if i == 0 {
                                println!("    Next elapse: never");
                            }
                            break;
                        }
                    }
                }
                println!();
            }
            Err(e) => {
                eprintln!("Failed to parse calendar expression '{}': {}", expr, e);
                process::exit(1);
            }
        }
    }
}

fn format_relative_duration(d: Duration) -> String {
    let total_secs = d.as_secs();
    if total_secs == 0 {
        return "now".to_string();
    }
    let days = total_secs / 86400;
    let hours = (total_secs % 86400) / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    let mut parts = Vec::new();
    if days > 0 {
        parts.push(format!("{}day", days));
    }
    if hours > 0 {
        parts.push(format!("{}h", hours));
    }
    if minutes > 0 {
        parts.push(format!("{}min", minutes));
    }
    if seconds > 0 || parts.is_empty() {
        parts.push(format!("{}s", seconds));
    }
    parts.join(" ") + " left"
}

fn cmd_timespan(expressions: &[String]) {
    if expressions.is_empty() {
        eprintln!("No time span expression specified.");
        process::exit(1);
    }

    for expr in expressions {
        match TimeSpan::parse(expr) {
            Ok(span) => {
                println!("Original: {expr}");
                println!("      {}:", format_usec(span.usec));
                println!("   {} us", span.usec);
                println!("   {span}");
                println!();
            }
            Err(e) => {
                eprintln!("Failed to parse time span '{}': {}", expr, e);
                process::exit(1);
            }
        }
    }
}

fn cmd_timestamp(expressions: &[String]) {
    if expressions.is_empty() {
        eprintln!("No timestamp expression specified.");
        process::exit(1);
    }

    for expr in expressions {
        match parse_timestamp(expr) {
            Ok(ts) => {
                let dur = ts.duration_since(UNIX_EPOCH).unwrap_or_default();
                println!("  Original form: {expr}");
                println!("Normalized form: {}", format_timestamp(ts));
                println!("       (in UTC): {}", format_timestamp(ts));
                println!("   UNIX seconds: @{}", dur.as_secs());
                let from_now = if let Ok(d) = ts.duration_since(SystemTime::now()) {
                    format!("in {}", format_usec(d.as_micros() as u64))
                } else if let Ok(d) = SystemTime::now().duration_since(ts) {
                    format!("{} ago", format_usec(d.as_micros() as u64))
                } else {
                    "now".to_string()
                };
                println!("      From now: {from_now}");
                println!();
            }
            Err(e) => {
                eprintln!("Failed to parse timestamp '{}': {}", expr, e);
                process::exit(1);
            }
        }
    }
}

fn cmd_verify(files: &[String]) {
    if files.is_empty() {
        eprintln!("No unit files specified.");
        process::exit(1);
    }

    let mut any_error = false;
    for file in files {
        let issues = verify_unit_file(file);
        for issue in &issues {
            if !issue.ends_with(": OK") {
                any_error = true;
            }
            println!("{issue}");
        }
    }

    if any_error {
        process::exit(1);
    }
}

fn cmd_condition(expressions: &[String], unit: Option<&str>) {
    let mut all_exprs: Vec<String> = Vec::new();

    if let Some(unit_name) = unit {
        // Read conditions and asserts from the unit file
        let unit_exprs = read_unit_conditions(unit_name);
        if unit_exprs.is_empty() {
            eprintln!("No conditions or asserts found in unit {unit_name}");
            process::exit(1);
        }
        all_exprs.extend(unit_exprs);
    }
    all_exprs.extend(expressions.iter().cloned());

    if all_exprs.is_empty() {
        eprintln!("No condition expressions specified.");
        process::exit(1);
    }

    let mut all_met = true;
    for expr in &all_exprs {
        let (met, reason) = evaluate_condition(expr);
        let status = if met { "met" } else { "not met" };
        println!("{expr}: {status} ({reason})");
        if !met {
            all_met = false;
        }
    }

    if !all_met {
        process::exit(1);
    }
}

/// Read Condition*= and Assert*= lines from a unit file, searching unit paths.
fn read_unit_conditions(unit_name: &str) -> Vec<String> {
    let paths = system_unit_paths();
    let mut conditions = Vec::new();

    let is_condition_line = |line: &str| {
        (line.starts_with("Condition") || line.starts_with("Assert")) && line.contains('=')
    };

    // Find the unit file
    for dir in &paths {
        let unit_path = dir.join(unit_name);
        if unit_path.is_file() {
            if let Ok(content) = fs::read_to_string(&unit_path) {
                for line in content.lines() {
                    let line = line.trim();
                    if is_condition_line(line) {
                        conditions.push(line.to_string());
                    }
                }
            }
            break;
        }
    }

    // Also check drop-in directories
    let dropin_dir_name = format!("{unit_name}.d");
    for dir in &paths {
        let dropin_dir = dir.join(&dropin_dir_name);
        if dropin_dir.is_dir()
            && let Ok(entries) = fs::read_dir(&dropin_dir)
        {
            let mut conf_files: Vec<_> = entries
                .filter_map(|e| e.ok())
                .filter(|e| e.path().extension().is_some_and(|ext| ext == "conf"))
                .collect();
            conf_files.sort_by_key(|e| e.file_name());
            for entry in conf_files {
                if let Ok(content) = fs::read_to_string(entry.path()) {
                    for line in content.lines() {
                        let line = line.trim();
                        if is_condition_line(line) {
                            conditions.push(line.to_string());
                        }
                    }
                }
            }
        }
    }

    conditions
}

fn cmd_unit_paths(user_mode: bool) {
    let paths = if user_mode {
        user_unit_paths()
    } else {
        system_unit_paths()
    };

    for p in &paths {
        println!("{}", p.display());
    }
}

fn cmd_cat_config(items: &[String], tldr: bool) {
    // Config search paths (highest to lowest priority)
    let search_dirs = ["/etc", "/run", "/usr/local/lib", "/usr/lib"];

    for item in items {
        // If it's an absolute path, just cat it
        let paths_to_try: Vec<PathBuf> = if item.starts_with('/') {
            vec![PathBuf::from(item)]
        } else {
            search_dirs
                .iter()
                .map(|dir| PathBuf::from(dir).join(item))
                .collect()
        };

        let mut found = false;
        for path in &paths_to_try {
            if path.is_file() {
                found = true;
                // Resolve symlinks for the comment header
                let resolved = fs::canonicalize(path).unwrap_or_else(|_| path.clone());
                if resolved != *path {
                    println!("# {} -> {}", path.display(), resolved.display());
                } else {
                    println!("# {}", path.display());
                }
                if let Ok(content) = fs::read_to_string(path) {
                    if tldr {
                        for line in content.lines() {
                            let trimmed = line.trim();
                            if !trimmed.is_empty()
                                && !trimmed.starts_with('#')
                                && !trimmed.starts_with(';')
                            {
                                println!("{line}");
                            }
                        }
                    } else {
                        print!("{content}");
                        if !content.ends_with('\n') {
                            println!();
                        }
                    }
                }
            }

            // Also check for .d/ drop-in directory
            let dropin_dir = PathBuf::from(format!("{}.d", path.display()));
            if dropin_dir.is_dir()
                && let Ok(entries) = fs::read_dir(&dropin_dir)
            {
                let mut conf_files: Vec<_> = entries
                    .filter_map(|e| e.ok())
                    .filter(|e| e.path().extension().is_some_and(|ext| ext == "conf"))
                    .collect();
                conf_files.sort_by_key(|e| e.file_name());
                for entry in conf_files {
                    found = true;
                    let p = entry.path();
                    let resolved = fs::canonicalize(&p).unwrap_or_else(|_| p.clone());
                    if resolved != p {
                        println!("\n# {} -> {}", p.display(), resolved.display());
                    } else {
                        println!("\n# {}", p.display());
                    }
                    if let Ok(content) = fs::read_to_string(&p) {
                        if tldr {
                            for line in content.lines() {
                                let trimmed = line.trim();
                                if !trimmed.is_empty()
                                    && !trimmed.starts_with('#')
                                    && !trimmed.starts_with(';')
                                {
                                    println!("{line}");
                                }
                            }
                        } else {
                            print!("{content}");
                            if !content.ends_with('\n') {
                                println!();
                            }
                        }
                    }
                }
            }
        }

        if !found {
            // Not an error — C systemd also silently skips missing configs
        }
    }
}

fn cmd_unit_files(patterns: &[String], user_mode: bool) {
    let paths = if user_mode {
        user_unit_paths()
    } else {
        system_unit_paths()
    };

    let unit_suffixes = [
        ".service",
        ".socket",
        ".target",
        ".mount",
        ".automount",
        ".swap",
        ".timer",
        ".path",
        ".slice",
        ".scope",
        ".device",
    ];

    let matches_pattern = |name: &str, patterns: &[String]| -> bool {
        if patterns.is_empty() {
            return true;
        }
        for pat in patterns {
            if pat == "*" || simple_glob_match(pat, name) {
                return true;
            }
        }
        false
    };

    for dir in &paths {
        if let Ok(entries) = fs::read_dir(dir) {
            for entry in entries.filter_map(|e| e.ok()) {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();
                if !unit_suffixes.iter().any(|s| name_str.ends_with(s)) {
                    continue;
                }
                if matches_pattern(&name_str, patterns) {
                    println!("ids: {} \u{2192} {}", name_str, entry.path().display());
                }
            }
        }
    }
}

/// Simple glob matching supporting * and ? wildcards.
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_inner(&pat, &txt)
}

fn glob_match_inner(pat: &[char], txt: &[char]) -> bool {
    if pat.is_empty() {
        return txt.is_empty();
    }
    match pat[0] {
        '*' => {
            // Try matching * with 0..n characters
            for i in 0..=txt.len() {
                if glob_match_inner(&pat[1..], &txt[i..]) {
                    return true;
                }
            }
            false
        }
        '?' => !txt.is_empty() && glob_match_inner(&pat[1..], &txt[1..]),
        c => !txt.is_empty() && txt[0] == c && glob_match_inner(&pat[1..], &txt[1..]),
    }
}

fn cmd_log_level(level: &Option<String>) {
    // Try to communicate with PID 1 via control socket
    let socket_path = "/run/systemd/rust-systemd-notify/control.socket";

    match level {
        Some(l) => {
            // Strip surrounding quotes (test scripts may pass e.g. '"debug"')
            let l = l.trim_matches('"').trim_matches('\'');
            // Validate the level
            match l.to_lowercase().as_str() {
                "emerg" | "alert" | "crit" | "err" | "warning" | "notice" | "info" | "debug" => {}
                _ => {
                    eprintln!("Invalid log level: {l}");
                    process::exit(1);
                }
            }

            // Send log-level command to PID 1
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
                use std::io::Write;
                let request = format!(
                    r#"{{"jsonrpc":"2.0","method":"log-level","params":"{}"}}"#,
                    l
                );
                let _ = stream.write_all(request.as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                // Read response (we don't need to check it for set operations)
                let _resp: Result<serde_json::Value, _> = serde_json::from_reader(&mut stream);
            } else {
                // Fallback: write directly (works if we have permissions)
                let _ = fs::create_dir_all("/run/rust-systemd");
                let _ = fs::write("/run/rust-systemd/log-level", l);
            }
        }
        None => {
            // Query current level — try control socket first, then file
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
                use std::io::Write;
                let request = r#"{"jsonrpc":"2.0","method":"log-level","id":1}"#;
                let _ = stream.write_all(request.as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                if let Ok(resp) = serde_json::from_reader::<_, serde_json::Value>(&mut stream)
                    && let Some(result) = resp.get("result").and_then(|v| v.as_str())
                {
                    println!("{result}");
                    return;
                }
            }
            // Fallback
            if let Ok(level) = fs::read_to_string("/run/rust-systemd/log-level") {
                println!("{}", level.trim());
            } else {
                println!("info");
            }
        }
    }
}

fn cmd_log_target(target: &Option<String>) {
    let socket_path = "/run/systemd/rust-systemd-notify/control.socket";
    match target {
        Some(t) => {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
                use std::io::Write;
                let request = format!(
                    r#"{{"jsonrpc":"2.0","method":"log-target","params":"{}"}}"#,
                    t
                );
                let _ = stream.write_all(request.as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let _resp: Result<serde_json::Value, _> = serde_json::from_reader(&mut stream);
            } else {
                let _ = fs::create_dir_all("/run/rust-systemd");
                let _ = fs::write("/run/rust-systemd/log-target", t);
            }
        }
        None => {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
                use std::io::Write;
                let request = r#"{"jsonrpc":"2.0","method":"log-target","id":1}"#;
                let _ = stream.write_all(request.as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                if let Ok(resp) = serde_json::from_reader::<_, serde_json::Value>(&mut stream)
                    && let Some(result) = resp.get("result").and_then(|v| v.as_str())
                {
                    println!("{result}");
                    return;
                }
            }
            if let Ok(target) = fs::read_to_string("/run/rust-systemd/log-target") {
                println!("{}", target.trim());
            } else {
                println!("journal-or-kmsg");
            }
        }
    }
}

fn cmd_service_watchdogs(state: &Option<String>) {
    let socket_path = "/run/systemd/rust-systemd-notify/control.socket";
    match state {
        Some(s) => {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
                use std::io::Write;
                let request = format!(
                    r#"{{"jsonrpc":"2.0","method":"service-watchdogs","params":"{}"}}"#,
                    s
                );
                let _ = stream.write_all(request.as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                let _resp: Result<serde_json::Value, _> = serde_json::from_reader(&mut stream);
            } else {
                let _ = fs::create_dir_all("/run/rust-systemd");
                let _ = fs::write("/run/rust-systemd/service-watchdogs", s);
            }
        }
        None => {
            if let Ok(mut stream) = std::os::unix::net::UnixStream::connect(socket_path) {
                use std::io::Write;
                let request = r#"{"jsonrpc":"2.0","method":"service-watchdogs","id":1}"#;
                let _ = stream.write_all(request.as_bytes());
                let _ = stream.shutdown(std::net::Shutdown::Write);
                if let Ok(resp) = serde_json::from_reader::<_, serde_json::Value>(&mut stream)
                    && let Some(result) = resp.get("result").and_then(|v| v.as_str())
                {
                    println!("{result}");
                    return;
                }
            }
            if let Ok(val) = fs::read_to_string("/run/rust-systemd/service-watchdogs") {
                println!("{}", val.trim());
            } else {
                println!("yes");
            }
        }
    }
}

fn cmd_security(units: &[String]) {
    if units.is_empty() {
        println!("UNIT                                 EXPOSURE PREDICATE HAPPY");
        println!("No units specified and service enumeration not yet implemented.");
        return;
    }

    for unit in units {
        // Basic security assessment — check for obvious concerns
        let path = find_unit_file(unit);
        match path {
            Some(p) => {
                let content = fs::read_to_string(&p).unwrap_or_default();
                let mut score: f64 = 10.0; // Start at max exposure

                // Check for security hardening directives
                let hardening: &[(&str, f64)] = &[
                    ("ProtectSystem=", -1.0),
                    ("ProtectHome=", -1.0),
                    ("PrivateTmp=", -0.5),
                    ("NoNewPrivileges=", -0.5),
                    ("PrivateDevices=", -0.5),
                    ("ProtectKernelTunables=", -0.5),
                    ("ProtectKernelModules=", -0.5),
                    ("ProtectControlGroups=", -0.5),
                    ("RestrictSUIDSGID=", -0.5),
                    ("MemoryDenyWriteExecute=", -0.5),
                    ("RestrictNamespaces=", -0.5),
                    ("RestrictRealtime=", -0.5),
                    ("LockPersonality=", -0.5),
                    ("DynamicUser=", -1.0),
                    ("CapabilityBoundingSet=", -0.5),
                    ("SystemCallFilter=", -1.0),
                ];

                for (directive, adjustment) in hardening {
                    if content.contains(directive) {
                        score += adjustment;
                    }
                }

                // Penalize for running as root
                if content.contains("User=root")
                    || (!content.contains("User=") && !content.contains("DynamicUser="))
                {
                    score += 0.5;
                }

                score = score.clamp(0.0, 10.0);

                let predicate = if score <= 2.0 {
                    "OK"
                } else if score <= 5.0 {
                    "MEDIUM"
                } else if score <= 7.5 {
                    "EXPOSED"
                } else {
                    "UNSAFE"
                };

                let happy = if score <= 2.0 {
                    "😀"
                } else if score <= 5.0 {
                    "😐"
                } else if score <= 7.5 {
                    "🙁"
                } else {
                    "😨"
                };

                println!("{:<40} {:>4.1} {:<10} {}", unit, score, predicate, happy);
            }
            None => {
                println!("{unit}: unit file not found");
            }
        }
    }
}

fn find_unit_file(name: &str) -> Option<PathBuf> {
    for dir in system_unit_paths() {
        let path = dir.join(name);
        if path.exists() {
            return Some(path);
        }
    }
    None
}

// ── Plot (SVG) ────────────────────────────────────────────────────────────

/// Color for a unit bar based on the unit type suffix.
fn unit_color(name: &str) -> &'static str {
    if name.ends_with(".target") {
        "#4e9a06" // green
    } else if name.ends_with(".service") {
        "#729fcf" // blue
    } else if name.ends_with(".socket") {
        "#ef2929" // red
    } else if name.ends_with(".mount") || name.ends_with(".automount") {
        "#ad7fa8" // purple
    } else if name.ends_with(".timer") {
        "#fcaf3e" // orange
    } else if name.ends_with(".device") {
        "#e9b96e" // tan
    } else if name.ends_with(".path") {
        "#fce94f" // yellow
    } else if name.ends_with(".slice") || name.ends_with(".scope") {
        "#8ae234" // light green
    } else {
        "#888a85" // grey
    }
}

fn cmd_plot() {
    let bt = read_boot_timing();
    let mut timings = read_unit_timings();

    if timings.is_empty() {
        eprintln!("No unit timing data available in /run/rust-systemd/timing/.");
        eprintln!("Boot the system with rust-systemd to generate timing data.");
        process::exit(1);
    }

    // Sort by activation start time
    timings.sort_by_key(|t| t.activating_us);

    let total_us = bt
        .total_us
        .or_else(|| timings.iter().map(|t| t.active_us).max())
        .unwrap_or(1);

    // SVG dimensions
    let left_margin = 300.0_f64;
    let right_margin = 50.0;
    let top_margin = 120.0;
    let row_height = 18.0;
    let bar_height = 14.0;
    let chart_width = 800.0_f64;
    let total_width = left_margin + chart_width + right_margin;
    let total_height = top_margin + (timings.len() as f64 * row_height) + 60.0;

    let scale = chart_width / total_us as f64;

    println!(r#"<?xml version="1.0" encoding="UTF-8" standalone="no"?>"#);
    println!(
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="{total_width}" height="{total_height}" version="1.1">"#
    );
    println!(r#"<defs><style type="text/css">"#);
    println!(
        r#"  text {{ font-family: "Liberation Sans", sans-serif; font-size: 10px; fill: #000; }}"#
    );
    println!(r#"  text.left {{ text-anchor: end; }}"#);
    println!(r#"  text.header {{ font-size: 14px; font-weight: bold; }}"#);
    println!(r#"  text.phase {{ font-size: 11px; }}"#);
    println!(r#"  rect.bg {{ fill: #fff; }}"#);
    println!(r#"  line.grid {{ stroke: #ddd; stroke-width: 0.5; }}"#);
    println!(r#"</style></defs>"#);

    // Background
    println!(r#"<rect class="bg" x="0" y="0" width="{total_width}" height="{total_height}"/>"#);

    // Title
    let host = hostname().unwrap_or_else(|| "unknown".to_string());
    println!(
        r#"<text class="header" x="{}" y="20">Boot chart for {host}</text>"#,
        total_width / 2.0
    );

    // Phase summary bar
    let phase_y = 40.0;
    let mut phase_x = left_margin;

    if let Some(k) = bt.kernel_us {
        let w = k as f64 * scale;
        let color = "#fee391";
        println!(r#"<rect x="{phase_x}" y="{phase_y}" width="{w}" height="20" fill="{color}"/>"#);
        println!(
            r#"<text class="phase" x="{}" y="{}">kernel ({})</text>"#,
            phase_x + w / 2.0,
            phase_y + 14.0,
            format_usec(k)
        );
        phase_x += w;
    }
    if let Some(i) = bt.initrd_us {
        let w = i as f64 * scale;
        let color = "#fdd0a2";
        println!(r#"<rect x="{phase_x}" y="{phase_y}" width="{w}" height="20" fill="{color}"/>"#);
        println!(
            r#"<text class="phase" x="{}" y="{}">initrd ({})</text>"#,
            phase_x + w / 2.0,
            phase_y + 14.0,
            format_usec(i)
        );
        phase_x += w;
    }
    if let Some(u) = bt.userspace_us {
        let w = u as f64 * scale;
        let color = "#c6dbef";
        println!(r#"<rect x="{phase_x}" y="{phase_y}" width="{w}" height="20" fill="{color}"/>"#);
        println!(
            r#"<text class="phase" x="{}" y="{}">userspace ({})</text>"#,
            phase_x + w / 2.0,
            phase_y + 14.0,
            format_usec(u)
        );
    }

    // Time axis
    let axis_y = top_margin - 15.0;
    let tick_count = 10usize;
    for i in 0..=tick_count {
        let frac = i as f64 / tick_count as f64;
        let x = left_margin + frac * chart_width;
        let t = (frac * total_us as f64) as u64;
        println!(r#"<line class="grid" x1="{x}" y1="{axis_y}" x2="{x}" y2="{total_height}"/>"#);
        println!(
            r#"<text x="{x}" y="{}" style="text-anchor:middle;font-size:9px">{}</text>"#,
            axis_y - 2.0,
            format_usec(t)
        );
    }

    // Unit bars
    for (i, t) in timings.iter().enumerate() {
        let y = top_margin + i as f64 * row_height;
        let x_start = left_margin + t.activating_us as f64 * scale;
        let activating_w = (t.active_us.saturating_sub(t.activating_us)) as f64 * scale;
        let bar_w = activating_w.max(1.0);
        let color = unit_color(&t.name);

        // Activating phase (lighter)
        println!(
            r#"<rect x="{x_start}" y="{}" width="{bar_w}" height="{bar_height}" fill="{color}" opacity="0.5"/>"#,
            y + (row_height - bar_height) / 2.0
        );
        // Active marker (full color, right edge)
        let active_x = left_margin + t.active_us as f64 * scale;
        println!(
            r#"<rect x="{}" y="{}" width="2" height="{bar_height}" fill="{color}"/>"#,
            active_x - 1.0,
            y + (row_height - bar_height) / 2.0
        );

        // Unit name label
        let label_text = if t.name.len() > 38 {
            format!("{}...", &t.name[..35])
        } else {
            t.name.clone()
        };
        println!(
            r#"<text class="left" x="{}" y="{}">{label_text} ({})</text>"#,
            left_margin - 5.0,
            y + row_height / 2.0 + 3.0,
            format_usec(t.duration_us())
        );
    }

    // Legend
    let legend_y = total_height - 40.0;
    let legend_items: &[(&str, &str)] = &[
        ("Service", "#729fcf"),
        ("Target", "#4e9a06"),
        ("Socket", "#ef2929"),
        ("Mount", "#ad7fa8"),
        ("Timer", "#fcaf3e"),
        ("Device", "#e9b96e"),
        ("Other", "#888a85"),
    ];
    let mut lx = left_margin;
    for &(label, color) in legend_items {
        println!(r#"<rect x="{lx}" y="{legend_y}" width="12" height="12" fill="{color}"/>"#);
        println!(
            r#"<text x="{}" y="{}">{label}</text>"#,
            lx + 16.0,
            legend_y + 10.0
        );
        lx += 80.0;
    }

    println!("</svg>");
}

// ── ELF inspection ────────────────────────────────────────────────────────

/// Read a u16 from a byte slice at the given offset with the given endianness.
fn elf_read_u16(data: &[u8], offset: usize, little_endian: bool) -> u16 {
    if little_endian {
        u16::from_le_bytes([data[offset], data[offset + 1]])
    } else {
        u16::from_be_bytes([data[offset], data[offset + 1]])
    }
}

/// Read a u32 from a byte slice at the given offset with the given endianness.
fn elf_read_u32(data: &[u8], offset: usize, little_endian: bool) -> u32 {
    let b = &data[offset..offset + 4];
    if little_endian {
        u32::from_le_bytes([b[0], b[1], b[2], b[3]])
    } else {
        u32::from_be_bytes([b[0], b[1], b[2], b[3]])
    }
}

/// Read a u64 from a byte slice at the given offset with the given endianness.
fn elf_read_u64(data: &[u8], offset: usize, little_endian: bool) -> u64 {
    let b = &data[offset..offset + 8];
    if little_endian {
        u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    } else {
        u64::from_be_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
    }
}

/// ELF machine type name.
fn elf_machine_name(em: u16) -> &'static str {
    match em {
        0x03 => "x86",
        0x08 => "MIPS",
        0x14 => "PowerPC",
        0x15 => "PowerPC64",
        0x28 => "ARM",
        0x3E => "x86-64",
        0xB7 => "AArch64",
        0xF3 => "RISC-V",
        0xF7 => "BPF",
        _ => "unknown",
    }
}

/// ELF OS/ABI name.
fn elf_osabi_name(osabi: u8) -> &'static str {
    match osabi {
        0 => "UNIX System V",
        1 => "HP-UX",
        2 => "NetBSD",
        3 => "GNU/Linux",
        6 => "Solaris",
        9 => "FreeBSD",
        12 => "OpenBSD",
        _ => "unknown",
    }
}

/// ELF type name.
fn elf_type_name(et: u16) -> &'static str {
    match et {
        0 => "NONE",
        1 => "REL (Relocatable)",
        2 => "EXEC (Executable)",
        3 => "DYN (Shared object)",
        4 => "CORE (Core dump)",
        _ => "unknown",
    }
}

/// Inspect a single ELF file and print metadata.
fn inspect_elf_file(path: &str) -> Result<(), String> {
    let mut file = fs::File::open(path).map_err(|e| format!("Cannot open {path}: {e}"))?;
    let mut data = Vec::new();
    file.read_to_end(&mut data)
        .map_err(|e| format!("Cannot read {path}: {e}"))?;

    // Check ELF magic
    if data.len() < 64 || &data[0..4] != b"\x7fELF" {
        return Err(format!("{path}: Not an ELF file"));
    }

    let class = data[4]; // 1 = 32-bit, 2 = 64-bit
    let is_64 = class == 2;
    let little_endian = data[5] == 1;
    let osabi = data[7];
    let elf_type = elf_read_u16(&data, 16, little_endian);
    let machine = elf_read_u16(&data, 18, little_endian);

    println!("           {path}:");
    println!("      Type: {}", elf_type_name(elf_type));
    println!("     Class: {}", if is_64 { "ELF64" } else { "ELF32" });
    println!(
        "      Data: {} endian",
        if little_endian { "little" } else { "big" }
    );
    println!("    OS/ABI: {}", elf_osabi_name(osabi));
    println!("   Machine: {}", elf_machine_name(machine));

    // Parse section headers to find interesting sections
    let (sh_offset, sh_entsize, sh_num, sh_strndx) = if is_64 {
        let sh_off = elf_read_u64(&data, 40, little_endian) as usize;
        let sh_ent = elf_read_u16(&data, 58, little_endian) as usize;
        let sh_n = elf_read_u16(&data, 60, little_endian) as usize;
        let sh_str = elf_read_u16(&data, 62, little_endian) as usize;
        (sh_off, sh_ent, sh_n, sh_str)
    } else {
        let sh_off = elf_read_u32(&data, 32, little_endian) as usize;
        let sh_ent = elf_read_u16(&data, 46, little_endian) as usize;
        let sh_n = elf_read_u16(&data, 48, little_endian) as usize;
        let sh_str = elf_read_u16(&data, 50, little_endian) as usize;
        (sh_off, sh_ent, sh_n, sh_str)
    };

    if sh_offset == 0 || sh_num == 0 || sh_entsize == 0 {
        println!("  (no section headers)");
        return Ok(());
    }

    // Read section name string table
    let strtab_offset = if sh_strndx < sh_num {
        let entry = sh_offset + sh_strndx * sh_entsize;
        if is_64 {
            elf_read_u64(&data, entry + 24, little_endian) as usize
        } else {
            elf_read_u32(&data, entry + 16, little_endian) as usize
        }
    } else {
        0
    };

    let get_section_name = |name_offset: usize| -> String {
        if strtab_offset == 0 || strtab_offset + name_offset >= data.len() {
            return String::new();
        }
        let start = strtab_offset + name_offset;
        let end = data[start..]
            .iter()
            .position(|&b| b == 0)
            .map(|p| start + p)
            .unwrap_or(data.len());
        String::from_utf8_lossy(&data[start..end]).to_string()
    };

    // Build-ID from .note.gnu.build-id
    let mut build_id: Option<String> = None;
    let mut package_note: Option<String> = None;

    for i in 0..sh_num {
        let entry = sh_offset + i * sh_entsize;
        if entry + sh_entsize > data.len() {
            break;
        }

        let name_idx = elf_read_u32(&data, entry, little_endian) as usize;
        let sh_type = elf_read_u32(&data, entry + 4, little_endian);
        let (sec_offset, sec_size) = if is_64 {
            (
                elf_read_u64(&data, entry + 24, little_endian) as usize,
                elf_read_u64(&data, entry + 32, little_endian) as usize,
            )
        } else {
            (
                elf_read_u32(&data, entry + 16, little_endian) as usize,
                elf_read_u32(&data, entry + 20, little_endian) as usize,
            )
        };

        let sec_name = get_section_name(name_idx);

        // SHT_NOTE = 7
        if sh_type == 7 && sec_offset + sec_size <= data.len() {
            let note_data = &data[sec_offset..sec_offset + sec_size];
            let mut off = 0;
            while off + 12 <= note_data.len() {
                let namesz = elf_read_u32(note_data, off, little_endian) as usize;
                let descsz = elf_read_u32(note_data, off + 4, little_endian) as usize;
                let note_type = elf_read_u32(note_data, off + 8, little_endian);
                let name_start = off + 12;
                let name_end = name_start + namesz;
                let align4 = |x: usize| (x + 3) & !3;
                let desc_start = align4(name_end);
                let desc_end = desc_start + descsz;

                if desc_end > note_data.len() {
                    break;
                }

                let note_name = if namesz > 0 && name_end <= note_data.len() {
                    let end = if note_data[name_end - 1] == 0 {
                        name_end - 1
                    } else {
                        name_end
                    };
                    String::from_utf8_lossy(&note_data[name_start..end]).to_string()
                } else {
                    String::new()
                };

                // NT_GNU_BUILD_ID = 3, owner "GNU"
                if note_name == "GNU" && note_type == 3 && build_id.is_none() {
                    let id_bytes = &note_data[desc_start..desc_end];
                    build_id = Some(
                        id_bytes
                            .iter()
                            .map(|b| format!("{b:02x}"))
                            .collect::<String>(),
                    );
                }

                // .note.package (FDO package metadata), owner "FDO"
                if (note_name == "FDO" || sec_name == ".note.package") && note_type == 0xcafe1a7e {
                    let json_bytes = &note_data[desc_start..desc_end];
                    // Trim trailing NULs
                    let json_str = String::from_utf8_lossy(json_bytes);
                    let trimmed = json_str.trim_end_matches('\0').trim();
                    if !trimmed.is_empty() {
                        package_note = Some(trimmed.to_string());
                    }
                }

                off = align4(desc_end);
            }
        }
    }

    if let Some(ref id) = build_id {
        println!("  Build ID: {id}");
    }
    if let Some(ref pkg) = package_note {
        println!("   Package: {pkg}");
    }

    // Show interpreter (.interp section)
    for i in 0..sh_num {
        let entry = sh_offset + i * sh_entsize;
        if entry + sh_entsize > data.len() {
            break;
        }
        let name_idx = elf_read_u32(&data, entry, little_endian) as usize;
        let sec_name = get_section_name(name_idx);
        if sec_name == ".interp" {
            let (sec_offset, sec_size) = if is_64 {
                (
                    elf_read_u64(&data, entry + 24, little_endian) as usize,
                    elf_read_u64(&data, entry + 32, little_endian) as usize,
                )
            } else {
                (
                    elf_read_u32(&data, entry + 16, little_endian) as usize,
                    elf_read_u32(&data, entry + 20, little_endian) as usize,
                )
            };
            if sec_offset + sec_size <= data.len() {
                let interp = String::from_utf8_lossy(&data[sec_offset..sec_offset + sec_size]);
                let trimmed = interp.trim_end_matches('\0');
                println!("   Interp.: {trimmed}");
            }
            break;
        }
    }

    if build_id.is_none() && package_note.is_none() {
        println!("  (no embedded package metadata found)");
    }

    Ok(())
}

fn cmd_inspect_elf(files: &[String]) {
    if files.is_empty() {
        eprintln!("No ELF binary specified.");
        process::exit(1);
    }

    let mut any_error = false;
    for file in files {
        if let Err(e) = inspect_elf_file(file) {
            eprintln!("{e}");
            any_error = true;
        }
        if files.len() > 1 {
            println!();
        }
    }

    if any_error {
        process::exit(1);
    }
}

// ── FD store ──────────────────────────────────────────────────────────────

fn cmd_fdstore(unit: &str) {
    // Attempt to query fd store via the runtime state directory
    let fdstore_dir = format!("/run/rust-systemd/fdstore/{unit}");
    let path = Path::new(&fdstore_dir);

    println!("         Unit: {unit}");

    if !path.exists() {
        // Try the standard systemd path as well
        let systemd_path = format!("/run/systemd/units/fdstore/{unit}");
        if Path::new(&systemd_path).exists()
            && let Ok(entries) = fs::read_dir(&systemd_path)
        {
            let fds: Vec<_> = entries.flatten().collect();
            println!("    FD Store: {} entries", fds.len());
            for entry in &fds {
                let name = entry.file_name().to_string_lossy().to_string();
                let metadata = entry.metadata().ok();
                let size = metadata.as_ref().map(|m| m.len()).unwrap_or(0);
                println!("      {name}: {size} bytes");
            }
            return;
        }

        println!("    FD Store: (no entries)");
        println!();
        println!("No file descriptor store data found for {unit}.");
        println!("The service must be running with FileDescriptorStoreMax= set to a value > 0.");
        return;
    }

    match fs::read_dir(path) {
        Ok(entries) => {
            let fds: Vec<_> = entries.flatten().collect();
            if fds.is_empty() {
                println!("    FD Store: (empty)");
            } else {
                println!("    FD Store: {} entries", fds.len());
                println!();
                println!("  {:>4}  {:<20}  INFO", "IDX", "NAME");
                for (idx, entry) in fds.iter().enumerate() {
                    let name = entry.file_name().to_string_lossy().to_string();
                    // Try to read metadata about the fd
                    let info = fs::read_to_string(entry.path())
                        .unwrap_or_default()
                        .trim()
                        .to_string();
                    let info_display = if info.is_empty() {
                        "(no info)".to_string()
                    } else {
                        info
                    };
                    println!("  {:>4}  {:<20}  {}", idx, name, info_display);
                }
            }
        }
        Err(e) => {
            println!("    FD Store: error reading: {e}");
        }
    }
}

// ── Image policy ──────────────────────────────────────────────────────────

/// Known partition designations for image policies.
const IMAGE_POLICY_PARTITIONS: &[&str] = &[
    "root",
    "usr",
    "home",
    "srv",
    "esp",
    "xbootldr",
    "swap",
    "root-verity",
    "root-verity-sig",
    "usr-verity",
    "usr-verity-sig",
    "tmp",
    "var",
];

/// Known use policies for partitions.
const IMAGE_POLICY_USES: &[&str] = &[
    "verity",
    "signed",
    "encrypted",
    "unprotected",
    "unused",
    "absent",
    "ignore",
];

/// Parse a single partition policy element like "root=verity+signed+encrypted".
fn parse_partition_policy(element: &str) -> Result<(String, Vec<String>), String> {
    let (partition, uses_str) = element
        .split_once('=')
        .ok_or_else(|| format!("Invalid policy element (missing '='): {element}"))?;

    let partition = partition.trim().to_lowercase();
    if partition != "*" && !IMAGE_POLICY_PARTITIONS.contains(&partition.as_str()) {
        return Err(format!("Unknown partition designation: {partition}"));
    }

    let uses: Vec<String> = uses_str
        .split('+')
        .map(|s| s.trim().to_lowercase())
        .collect();

    for u in &uses {
        if !IMAGE_POLICY_USES.contains(&u.as_str()) {
            return Err(format!(
                "Unknown use policy '{u}' for partition '{partition}'"
            ));
        }
    }

    Ok((partition, uses))
}

/// Normalize an image policy string.
fn normalize_image_policy(policy: &str) -> Result<String, String> {
    let policy = policy.trim();
    if policy.is_empty() {
        return Err("Empty image policy".to_string());
    }

    // Special well-known policies
    match policy {
        "allow" | "*" => return Ok("*=verity+signed+encrypted+unprotected+unused".to_string()),
        "deny" => return Ok("*=absent".to_string()),
        "ignore" => return Ok("*=ignore".to_string()),
        _ => {}
    }

    let elements: Vec<&str> = policy.split(':').collect();
    let mut parts = Vec::new();

    for element in &elements {
        let (partition, uses) = parse_partition_policy(element)?;
        parts.push(format!("{}={}", partition, uses.join("+")));
    }

    Ok(parts.join(":"))
}

fn cmd_image_policy(policies: &[String]) {
    if policies.is_empty() {
        eprintln!("No image policy specified.");
        eprintln!();
        eprintln!("Usage: systemd-analyze image-policy POLICY...");
        eprintln!();
        eprintln!(
            "Partition designations: {}",
            IMAGE_POLICY_PARTITIONS.join(", ")
        );
        eprintln!("Use policies: {}", IMAGE_POLICY_USES.join(", "));
        eprintln!();
        eprintln!("Special policies: allow, deny, ignore");
        eprintln!(
            "Example: root=verity+signed+encrypted+unprotected:swap=absent:home=encrypted+unprotected"
        );
        process::exit(1);
    }

    let mut any_error = false;
    for policy in policies {
        match normalize_image_policy(policy) {
            Ok(normalized) => {
                println!("  Original: {policy}");
                println!("Normalized: {normalized}");

                // Decompose and display
                for element in normalized.split(':') {
                    if let Some((part, uses)) = element.split_once('=') {
                        let uses_list: Vec<&str> = uses.split('+').collect();
                        println!("  {part:>20} -> {}", uses_list.join(", "));
                    }
                }
                println!();
            }
            Err(e) => {
                eprintln!("Invalid policy '{policy}': {e}");
                any_error = true;
            }
        }
    }

    if any_error {
        process::exit(1);
    }
}

// ── TPM2 PCRs ─────────────────────────────────────────────────────────────

/// PCR register descriptions.
fn pcr_description(index: u32) -> &'static str {
    match index {
        0 => "SRTM Contents (platform firmware)",
        1 => "Platform Configuration",
        2 => "Option ROM Code",
        3 => "Option ROM Configuration and Data",
        4 => "IPL Code (boot loader code)",
        5 => "IPL Configuration and Data (boot loader config)",
        6 => "State Transition and Wake Events",
        7 => "Secure Boot Policy",
        8 => "Kernel command line (grub, sd-stub)",
        9 => "Kernel image/initrd (grub, sd-stub)",
        10 => "Reserved for IMA",
        11 => "Unified Kernel Image components (sd-stub)",
        12 => "Kernel command line overrides (sd-stub)",
        13 => "System Extensions (sd-stub)",
        14 => "MOK certificates and hashes (shim)",
        15 => "TSS/User defined",
        16..=23 => "Dynamic/User defined",
        _ => "Unknown",
    }
}

fn cmd_pcrs() {
    // Try multiple TPM sysfs paths
    let hash_algs = ["sha256", "sha1", "sha384", "sha512"];
    let mut found = false;

    for alg in &hash_algs {
        let pcr_dir = format!("/sys/class/tpm/tpm0/pcr-{alg}");
        let path = Path::new(&pcr_dir);

        if !path.is_dir() {
            continue;
        }

        if !found {
            println!("TPM2 PCR Values:");
            println!();
        }
        found = true;
        println!("  Algorithm: {alg}");
        println!("  {:>3}  {:<48}  DESCRIPTION", "NR", "VALUE");

        // Read PCR 0-23
        for i in 0..24u32 {
            let pcr_file = format!("{pcr_dir}/{i}");
            let value =
                fs::read_to_string(&pcr_file).unwrap_or_else(|_| "(unreadable)".to_string());
            let value = value.trim();
            let desc = pcr_description(i);
            println!("  {:>3}  {:<48}  {desc}", i, value);
        }
        println!();
    }

    if !found {
        // No sysfs PCR data; try /dev/tpm0 existence
        if Path::new("/dev/tpm0").exists() || Path::new("/dev/tpmrm0").exists() {
            println!("TPM2 device found but PCR sysfs interface not available.");
            println!("PCR values could not be read from /sys/class/tpm/tpm0/pcr-*/");
        } else {
            println!("No TPM2 device found.");
            println!("Looked for /sys/class/tpm/tpm0/pcr-sha256/ and /dev/tpm0.");
        }
        process::exit(1);
    }
}

// ── TPM2 SRK ──────────────────────────────────────────────────────────────

fn cmd_srk() {
    // The SRK (Storage Root Key) is a primary key in the TPM2 storage hierarchy.
    // We check for its existence and display available information.

    println!("TPM2 SRK (Storage Root Key):");
    println!();

    // Check TPM device availability
    let has_tpm = Path::new("/dev/tpmrm0").exists() || Path::new("/dev/tpm0").exists();

    if !has_tpm {
        println!("No TPM2 device found.");
        println!("Looked for /dev/tpmrm0 and /dev/tpm0.");
        process::exit(1);
    }

    // Show TPM device info
    if Path::new("/dev/tpmrm0").exists() {
        println!("  Device: /dev/tpmrm0 (resource manager)");
    } else {
        println!("  Device: /dev/tpm0 (direct access)");
    }

    // Read TPM version info
    if let Ok(caps) = fs::read_to_string("/sys/class/tpm/tpm0/caps") {
        for line in caps.lines() {
            let line = line.trim();
            if !line.is_empty() {
                println!("  {line}");
            }
        }
    }

    // Read TPM device description
    if let Ok(desc) = fs::read_to_string("/sys/class/tpm/tpm0/description") {
        let desc = desc.trim();
        if !desc.is_empty() {
            println!("  Description: {desc}");
        }
    }

    // Read TPM PCR banks to show which algorithms are supported
    let hash_algs = ["sha256", "sha384", "sha512", "sha1"];
    let mut supported_algs = Vec::new();
    for alg in &hash_algs {
        let pcr_dir = format!("/sys/class/tpm/tpm0/pcr-{alg}");
        if Path::new(&pcr_dir).is_dir() {
            supported_algs.push(*alg);
        }
    }

    if !supported_algs.is_empty() {
        println!("  Supported hash algorithms: {}", supported_algs.join(", "));
    }

    // Check tpm2-tools availability
    if let Ok(output) = std::process::Command::new("tpm2_readpublic")
        .args(["-c", "0x81000001"])
        .output()
        && output.status.success()
    {
        println!();
        println!("  SRK public key (handle 0x81000001):");
        let stdout = String::from_utf8_lossy(&output.stdout);
        for line in stdout.lines() {
            println!("    {line}");
        }
        return;
    }

    // Also try the well-known SRK at the owner hierarchy primary
    if let Ok(output) = std::process::Command::new("tpm2_createprimary")
        .args([
            "-C",
            "o",
            "-G",
            "rsa2048",
            "-c",
            "/dev/null",
            "--format=pem",
            "-Q",
        ])
        .stderr(std::process::Stdio::null())
        .output()
        && output.status.success()
    {
        println!();
        println!("  SRK can be created in owner hierarchy (RSA-2048).");
    }

    println!();
    println!("  Note: Full SRK display requires tpm2-tools (tpm2_readpublic, tpm2_createprimary).");
    println!("  The SRK is a primary key in the TPM2 owner hierarchy used for sealing");
    println!("  secrets and key wrapping. Its handle is typically 0x81000001.");
}

// ── Exit status table ─────────────────────────────────────────────────────

/// All known exit statuses, matching C systemd's table exactly.
const EXIT_STATUS_TABLE: &[(&str, u8, &str)] = &[
    ("SUCCESS", 0, "libc"),
    ("FAILURE", 1, "libc"),
    ("INVALIDARGUMENT", 2, "LSB"),
    ("NOTIMPLEMENTED", 3, "LSB"),
    ("NOPERMISSION", 4, "LSB"),
    ("NOTINSTALLED", 5, "LSB"),
    ("NOTCONFIGURED", 6, "LSB"),
    ("NOTRUNNING", 7, "LSB"),
    ("USAGE", 64, "BSD"),
    ("DATAERR", 65, "BSD"),
    ("NOINPUT", 66, "BSD"),
    ("NOUSER", 67, "BSD"),
    ("NOHOST", 68, "BSD"),
    ("UNAVAILABLE", 69, "BSD"),
    ("SOFTWARE", 70, "BSD"),
    ("OSERR", 71, "BSD"),
    ("OSFILE", 72, "BSD"),
    ("CANTCREAT", 73, "BSD"),
    ("IOERR", 74, "BSD"),
    ("TEMPFAIL", 75, "BSD"),
    ("PROTOCOL", 76, "BSD"),
    ("NOPERM", 77, "BSD"),
    ("CONFIG", 78, "BSD"),
    ("CHDIR", 200, "systemd"),
    ("NICE", 201, "systemd"),
    ("FDS", 202, "systemd"),
    ("EXEC", 203, "systemd"),
    ("MEMORY", 204, "systemd"),
    ("LIMITS", 205, "systemd"),
    ("OOM_ADJUST", 206, "systemd"),
    ("SIGNAL_MASK", 207, "systemd"),
    ("STDIN", 208, "systemd"),
    ("STDOUT", 209, "systemd"),
    ("CHROOT", 210, "systemd"),
    ("IOPRIO", 211, "systemd"),
    ("TIMERSLACK", 212, "systemd"),
    ("SECUREBITS", 213, "systemd"),
    ("SETSCHEDULER", 214, "systemd"),
    ("CPUAFFINITY", 215, "systemd"),
    ("GROUP", 216, "systemd"),
    ("USER", 217, "systemd"),
    ("CAPABILITIES", 218, "systemd"),
    ("CGROUP", 219, "systemd"),
    ("SETSID", 220, "systemd"),
    ("CONFIRM", 221, "systemd"),
    ("STDERR", 222, "systemd"),
    ("PAM", 224, "systemd"),
    ("NETWORK", 225, "systemd"),
    ("NAMESPACE", 226, "systemd"),
    ("NO_NEW_PRIVILEGES", 227, "systemd"),
    ("SECCOMP", 228, "systemd"),
    ("SELINUX_CONTEXT", 229, "systemd"),
    ("PERSONALITY", 230, "systemd"),
    ("APPARMOR", 231, "systemd"),
    ("ADDRESS_FAMILIES", 232, "systemd"),
    ("RUNTIME_DIRECTORY", 233, "systemd"),
    ("CHOWN", 235, "systemd"),
    ("SMACK_PROCESS_LABEL", 236, "systemd"),
    ("KEYRING", 237, "systemd"),
    ("STATE_DIRECTORY", 238, "systemd"),
    ("CACHE_DIRECTORY", 239, "systemd"),
    ("LOGS_DIRECTORY", 240, "systemd"),
    ("CONFIGURATION_DIRECTORY", 241, "systemd"),
    ("NUMA_POLICY", 242, "systemd"),
    ("CREDENTIALS", 243, "systemd"),
    ("BPF", 244, "systemd"),
    ("KSM", 245, "systemd"),
    ("EXCEPTION", 255, "systemd"),
];

fn cmd_exit_status(statuses: &[String]) {
    println!("{:<24} {:>6} CLASS", "NAME", "STATUS");

    if statuses.is_empty() {
        for &(name, code, class) in EXIT_STATUS_TABLE {
            println!("{:<24} {:>6} {}", name, code, class);
        }
        return;
    }

    let mut had_error = false;
    for s in statuses {
        if let Ok(num) = s.parse::<u8>() {
            if let Some(&(name, code, class)) =
                EXIT_STATUS_TABLE.iter().find(|&&(_, c, _)| c == num)
            {
                println!("{:<24} {:>6} {}", name, code, class);
            } else {
                println!("{:<24} {:>6} -", "-", num);
            }
        } else {
            let upper = s.to_uppercase();
            if let Some(&(name, code, class)) =
                EXIT_STATUS_TABLE.iter().find(|&&(n, _, _)| n == upper)
            {
                println!("{:<24} {:>6} {}", name, code, class);
            } else {
                eprintln!("Unknown exit status: {s}");
                had_error = true;
            }
        }
    }
    if had_error {
        std::process::exit(1);
    }
}

// ── Capability table ──────────────────────────────────────────────────────

/// Linux capabilities, ordered by number (matches /usr/include/linux/capability.h).
const CAPABILITY_TABLE: &[(&str, u32)] = &[
    ("cap_chown", 0),
    ("cap_dac_override", 1),
    ("cap_dac_read_search", 2),
    ("cap_fowner", 3),
    ("cap_fsetid", 4),
    ("cap_kill", 5),
    ("cap_setgid", 6),
    ("cap_setuid", 7),
    ("cap_setpcap", 8),
    ("cap_linux_immutable", 9),
    ("cap_net_bind_service", 10),
    ("cap_net_broadcast", 11),
    ("cap_net_admin", 12),
    ("cap_net_raw", 13),
    ("cap_ipc_lock", 14),
    ("cap_ipc_owner", 15),
    ("cap_sys_module", 16),
    ("cap_sys_rawio", 17),
    ("cap_sys_chroot", 18),
    ("cap_sys_ptrace", 19),
    ("cap_sys_pacct", 20),
    ("cap_sys_admin", 21),
    ("cap_sys_boot", 22),
    ("cap_sys_nice", 23),
    ("cap_sys_resource", 24),
    ("cap_sys_time", 25),
    ("cap_sys_tty_config", 26),
    ("cap_mknod", 27),
    ("cap_lease", 28),
    ("cap_audit_write", 29),
    ("cap_audit_control", 30),
    ("cap_setfcap", 31),
    ("cap_mac_override", 32),
    ("cap_mac_admin", 33),
    ("cap_syslog", 34),
    ("cap_wake_alarm", 35),
    ("cap_block_suspend", 36),
    ("cap_audit_read", 37),
    ("cap_perfmon", 38),
    ("cap_bpf", 39),
    ("cap_checkpoint_restore", 40),
];

fn cmd_capability(capabilities: &[String], mask: Option<&str>) {
    if let Some(hex) = mask {
        let hex = hex.strip_prefix("0x").unwrap_or(hex);
        let mask_val = match u64::from_str_radix(hex, 16) {
            Ok(v) => v,
            Err(_) => {
                eprintln!("Invalid capability mask: {hex}");
                std::process::exit(1);
            }
        };
        // Print capabilities matching the mask
        let mut found = false;
        for &(name, num) in CAPABILITY_TABLE {
            if num < 64 && (mask_val & (1u64 << num)) != 0 {
                if found {
                    print!(" ");
                }
                print!("{name}");
                found = true;
            }
        }
        if found {
            println!();
        }
        return;
    }

    println!("{:<28} {:>6}", "NAME", "NUMBER");

    if capabilities.is_empty() {
        for &(name, num) in CAPABILITY_TABLE {
            println!("{:<28} {:>6}", name, num);
        }
        return;
    }

    let mut had_error = false;
    for s in capabilities {
        if let Ok(num) = s.parse::<u32>() {
            if let Some(&(name, n)) = CAPABILITY_TABLE.iter().find(|&&(_, n)| n == num) {
                println!("{:<28} {:>6}", name, n);
            } else {
                eprintln!("Unknown capability: {s}");
                had_error = true;
            }
        } else {
            let lower = s.to_lowercase();
            let search = if lower.starts_with("cap_") {
                lower.clone()
            } else {
                format!("cap_{lower}")
            };
            if let Some(&(name, num)) = CAPABILITY_TABLE.iter().find(|&&(n, _)| n == search) {
                println!("{:<28} {:>6}", name, num);
            } else {
                eprintln!("Unknown capability: {s}");
                had_error = true;
            }
        }
    }
    if had_error {
        std::process::exit(1);
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Plot tests ────────────────────────────────────────────────────────

    #[test]
    fn test_unit_color_service() {
        assert_eq!(unit_color("foo.service"), "#729fcf");
    }

    #[test]
    fn test_unit_color_target() {
        assert_eq!(unit_color("default.target"), "#4e9a06");
    }

    #[test]
    fn test_unit_color_socket() {
        assert_eq!(unit_color("dbus.socket"), "#ef2929");
    }

    #[test]
    fn test_unit_color_mount() {
        assert_eq!(unit_color("tmp.mount"), "#ad7fa8");
    }

    #[test]
    fn test_unit_color_timer() {
        assert_eq!(unit_color("foo.timer"), "#fcaf3e");
    }

    #[test]
    fn test_unit_color_unknown() {
        assert_eq!(unit_color("something"), "#888a85");
    }

    // ── ELF inspection tests ──────────────────────────────────────────────

    #[test]
    fn test_elf_read_u16_le() {
        let data = [0x01, 0x02];
        assert_eq!(elf_read_u16(&data, 0, true), 0x0201);
    }

    #[test]
    fn test_elf_read_u16_be() {
        let data = [0x01, 0x02];
        assert_eq!(elf_read_u16(&data, 0, false), 0x0102);
    }

    #[test]
    fn test_elf_read_u32_le() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(elf_read_u32(&data, 0, true), 0x04030201);
    }

    #[test]
    fn test_elf_read_u32_be() {
        let data = [0x01, 0x02, 0x03, 0x04];
        assert_eq!(elf_read_u32(&data, 0, false), 0x01020304);
    }

    #[test]
    fn test_elf_read_u64_le() {
        let data = [0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08];
        assert_eq!(elf_read_u64(&data, 0, true), 0x0807060504030201);
    }

    #[test]
    fn test_elf_machine_name_x86_64() {
        assert_eq!(elf_machine_name(0x3E), "x86-64");
    }

    #[test]
    fn test_elf_machine_name_aarch64() {
        assert_eq!(elf_machine_name(0xB7), "AArch64");
    }

    #[test]
    fn test_elf_machine_name_unknown() {
        assert_eq!(elf_machine_name(0xFF), "unknown");
    }

    #[test]
    fn test_elf_osabi_name_linux() {
        assert_eq!(elf_osabi_name(3), "GNU/Linux");
    }

    #[test]
    fn test_elf_osabi_name_sysv() {
        assert_eq!(elf_osabi_name(0), "UNIX System V");
    }

    #[test]
    fn test_elf_type_name_exec() {
        assert_eq!(elf_type_name(2), "EXEC (Executable)");
    }

    #[test]
    fn test_elf_type_name_dyn() {
        assert_eq!(elf_type_name(3), "DYN (Shared object)");
    }

    #[test]
    fn test_inspect_elf_not_elf() {
        // Create a temp file that's not an ELF
        let tmp = "/tmp/systemd-analyze-test-not-elf";
        fs::write(tmp, b"this is not an ELF file").unwrap();
        let result = inspect_elf_file(tmp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Not an ELF"));
        let _ = fs::remove_file(tmp);
    }

    #[test]
    fn test_inspect_elf_nonexistent() {
        let result = inspect_elf_file("/nonexistent/path/to/elf");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot open"));
    }

    #[test]
    fn test_inspect_elf_self() {
        // Inspect our own binary (should be a valid ELF)
        let exe = std::env::current_exe().unwrap();
        let result = inspect_elf_file(exe.to_str().unwrap());
        assert!(result.is_ok());
    }

    // ── Image policy tests ────────────────────────────────────────────────

    #[test]
    fn test_normalize_image_policy_allow() {
        let result = normalize_image_policy("allow").unwrap();
        assert!(result.contains("verity"));
        assert!(result.contains("unprotected"));
    }

    #[test]
    fn test_normalize_image_policy_deny() {
        let result = normalize_image_policy("deny").unwrap();
        assert_eq!(result, "*=absent");
    }

    #[test]
    fn test_normalize_image_policy_ignore() {
        let result = normalize_image_policy("ignore").unwrap();
        assert_eq!(result, "*=ignore");
    }

    #[test]
    fn test_normalize_image_policy_custom() {
        let result = normalize_image_policy("root=verity+signed:swap=absent").unwrap();
        assert_eq!(result, "root=verity+signed:swap=absent");
    }

    #[test]
    fn test_normalize_image_policy_wildcard() {
        let result = normalize_image_policy("*=unprotected").unwrap();
        assert_eq!(result, "*=unprotected");
    }

    #[test]
    fn test_normalize_image_policy_empty() {
        assert!(normalize_image_policy("").is_err());
    }

    #[test]
    fn test_normalize_image_policy_unknown_partition() {
        assert!(normalize_image_policy("foobar=verity").is_err());
    }

    #[test]
    fn test_normalize_image_policy_unknown_use() {
        assert!(normalize_image_policy("root=foobar").is_err());
    }

    #[test]
    fn test_parse_partition_policy_valid() {
        let (part, uses) = parse_partition_policy("root=verity+signed").unwrap();
        assert_eq!(part, "root");
        assert_eq!(uses, vec!["verity", "signed"]);
    }

    #[test]
    fn test_parse_partition_policy_missing_eq() {
        assert!(parse_partition_policy("rootverity").is_err());
    }

    // ── PCR tests ─────────────────────────────────────────────────────────

    #[test]
    fn test_pcr_description_known() {
        assert_eq!(pcr_description(0), "SRTM Contents (platform firmware)");
        assert_eq!(pcr_description(7), "Secure Boot Policy");
        assert_eq!(
            pcr_description(11),
            "Unified Kernel Image components (sd-stub)"
        );
    }

    #[test]
    fn test_pcr_description_dynamic() {
        assert_eq!(pcr_description(16), "Dynamic/User defined");
        assert_eq!(pcr_description(23), "Dynamic/User defined");
    }

    #[test]
    fn test_pcr_description_unknown() {
        assert_eq!(pcr_description(24), "Unknown");
    }

    // TimeSpan parsing tests

    #[test]
    fn test_timespan_parse_seconds() {
        let ts = TimeSpan::parse("5s").unwrap();
        assert_eq!(ts.usec, 5 * USEC_PER_SEC);
    }

    #[test]
    fn test_timespan_parse_minutes() {
        let ts = TimeSpan::parse("3min").unwrap();
        assert_eq!(ts.usec, 3 * USEC_PER_MINUTE);
    }

    #[test]
    fn test_timespan_parse_hours() {
        let ts = TimeSpan::parse("2h").unwrap();
        assert_eq!(ts.usec, 2 * USEC_PER_HOUR);
    }

    #[test]
    fn test_timespan_parse_days() {
        let ts = TimeSpan::parse("1d").unwrap();
        assert_eq!(ts.usec, USEC_PER_DAY);
    }

    #[test]
    fn test_timespan_parse_weeks() {
        let ts = TimeSpan::parse("2w").unwrap();
        assert_eq!(ts.usec, 2 * USEC_PER_WEEK);
    }

    #[test]
    fn test_timespan_parse_milliseconds() {
        let ts = TimeSpan::parse("500ms").unwrap();
        assert_eq!(ts.usec, 500 * USEC_PER_MSEC);
    }

    #[test]
    fn test_timespan_parse_microseconds() {
        let ts = TimeSpan::parse("100us").unwrap();
        assert_eq!(ts.usec, 100);
    }

    #[test]
    fn test_timespan_parse_compound() {
        let ts = TimeSpan::parse("1h 30min").unwrap();
        assert_eq!(ts.usec, USEC_PER_HOUR + 30 * USEC_PER_MINUTE);
    }

    #[test]
    fn test_timespan_parse_compound_no_space() {
        let ts = TimeSpan::parse("2h30min").unwrap();
        assert_eq!(ts.usec, 2 * USEC_PER_HOUR + 30 * USEC_PER_MINUTE);
    }

    #[test]
    fn test_timespan_parse_infinity() {
        let ts = TimeSpan::parse("infinity").unwrap();
        assert_eq!(ts.usec, u64::MAX);
    }

    #[test]
    fn test_timespan_parse_bare_number() {
        let ts = TimeSpan::parse("5000000").unwrap();
        assert_eq!(ts.usec, 5_000_000);
    }

    #[test]
    fn test_timespan_parse_full_word() {
        let ts = TimeSpan::parse("3 seconds").unwrap();
        assert_eq!(ts.usec, 3 * USEC_PER_SEC);
    }

    #[test]
    fn test_timespan_parse_error() {
        assert!(TimeSpan::parse("foobar").is_err());
    }

    #[test]
    fn test_timespan_display_zero() {
        let ts = TimeSpan { usec: 0 };
        assert_eq!(ts.to_string(), "0");
    }

    #[test]
    fn test_timespan_display_seconds() {
        let ts = TimeSpan {
            usec: 5 * USEC_PER_SEC,
        };
        assert_eq!(ts.to_string(), "5s");
    }

    #[test]
    fn test_timespan_display_compound() {
        let ts = TimeSpan {
            usec: USEC_PER_HOUR + 30 * USEC_PER_MINUTE + 15 * USEC_PER_SEC,
        };
        assert_eq!(ts.to_string(), "1h 30min 15s");
    }

    #[test]
    fn test_timespan_display_infinity() {
        let ts = TimeSpan { usec: u64::MAX };
        assert_eq!(ts.to_string(), "infinity");
    }

    // Calendar parsing tests

    #[test]
    fn test_calendar_parse_daily() {
        let spec = CalendarSpec::parse("daily").unwrap();
        assert_eq!(spec.normalized(), "*-*-* 00:00:00");
    }

    #[test]
    fn test_calendar_parse_weekly() {
        let spec = CalendarSpec::parse("weekly").unwrap();
        assert_eq!(spec.normalized(), "Mon *-*-* 00:00:00");
    }

    #[test]
    fn test_calendar_parse_monthly() {
        let spec = CalendarSpec::parse("monthly").unwrap();
        assert_eq!(spec.normalized(), "*-*-01 00:00:00");
    }

    #[test]
    fn test_calendar_parse_yearly() {
        let spec = CalendarSpec::parse("yearly").unwrap();
        assert_eq!(spec.normalized(), "*-01-01 00:00:00");
    }

    #[test]
    fn test_calendar_parse_hourly() {
        let spec = CalendarSpec::parse("hourly").unwrap();
        assert_eq!(spec.normalized(), "*-*-* *:00:00");
    }

    #[test]
    fn test_calendar_parse_minutely() {
        let spec = CalendarSpec::parse("minutely").unwrap();
        assert_eq!(spec.normalized(), "*-*-* *:*:00");
    }

    #[test]
    fn test_calendar_parse_quarterly() {
        let spec = CalendarSpec::parse("quarterly").unwrap();
        assert_eq!(spec.normalized(), "*-01,04,07,10-01 00:00:00");
    }

    #[test]
    fn test_calendar_parse_custom() {
        let spec = CalendarSpec::parse("*-*-* 06:00:00").unwrap();
        assert_eq!(spec.normalized(), "*-*-* 06:00:00");
    }

    #[test]
    fn test_calendar_parse_empty_error() {
        assert!(CalendarSpec::parse("").is_err());
    }

    #[test]
    fn test_calendar_next_elapse_daily() {
        use libsystemd::calendar_spec::DateTime;
        let spec = CalendarSpec::parse("daily").unwrap();
        let after = DateTime {
            year: 2025,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
            second: 1,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.day, 2);
        assert_eq!(next.hour, 0);
    }

    #[test]
    fn test_calendar_next_elapse_complex() {
        use libsystemd::calendar_spec::DateTime;
        let spec = CalendarSpec::parse("Mon..Fri *-*-* 09:00:00").unwrap();
        // 2025-06-14 is Saturday
        let after = DateTime {
            year: 2025,
            month: 6,
            day: 14,
            hour: 10,
            minute: 0,
            second: 0,
        };
        let next = spec.next_elapse(after).unwrap();
        assert_eq!(next.day, 16); // Monday
        assert_eq!(next.hour, 9);
    }

    // Timestamp parsing tests

    #[test]
    fn test_timestamp_now() {
        let ts = parse_timestamp("now").unwrap();
        let dur = ts.duration_since(UNIX_EPOCH).unwrap();
        assert!(dur.as_secs() > 0);
    }

    #[test]
    fn test_timestamp_epoch() {
        let ts = parse_timestamp("epoch").unwrap();
        assert_eq!(ts, UNIX_EPOCH);
    }

    #[test]
    fn test_timestamp_at_epoch() {
        let ts = parse_timestamp("@0").unwrap();
        assert_eq!(ts, UNIX_EPOCH);
    }

    #[test]
    fn test_timestamp_at_value() {
        let ts = parse_timestamp("@1700000000").unwrap();
        let dur = ts.duration_since(UNIX_EPOCH).unwrap();
        assert_eq!(dur.as_secs(), 1_700_000_000);
    }

    #[test]
    fn test_timestamp_datetime() {
        let ts = parse_timestamp("2023-11-14 12:00:00").unwrap();
        let dur = ts.duration_since(UNIX_EPOCH).unwrap();
        // 2023-11-14 12:00:00 UTC = 1699963200
        assert!(dur.as_secs() > 1_699_900_000);
        assert!(dur.as_secs() < 1_700_100_000);
    }

    #[test]
    fn test_timestamp_relative_plus() {
        let before = SystemTime::now();
        let ts = parse_timestamp("+5s").unwrap();
        let after = SystemTime::now();
        assert!(ts > before);
        let dur = ts.duration_since(after).unwrap_or_default();
        // Should be roughly 5 seconds from now (within 1 second tolerance)
        assert!(dur.as_secs() <= 6);
    }

    #[test]
    fn test_timestamp_invalid() {
        assert!(parse_timestamp("not-a-timestamp").is_err());
    }

    // Date conversion tests

    #[test]
    fn test_days_from_civil_epoch() {
        assert_eq!(days_from_civil(1970, 1, 1), 0);
    }

    #[test]
    fn test_days_from_civil_day2() {
        assert_eq!(days_from_civil(1970, 1, 2), 1);
    }

    #[test]
    fn test_days_from_civil_2000() {
        // 2000-01-01 is day 10957
        assert_eq!(days_from_civil(2000, 1, 1), 10957);
    }

    #[test]
    fn test_civil_from_days_epoch() {
        assert_eq!(civil_from_days(0), (1970, 1, 1));
    }

    #[test]
    fn test_civil_from_days_2000() {
        assert_eq!(civil_from_days(10957), (2000, 1, 1));
    }

    #[test]
    fn test_civil_roundtrip() {
        for &(y, m, d) in &[(1970, 1, 1), (2000, 2, 29), (2023, 12, 31), (1999, 6, 15)] {
            let days = days_from_civil(y, m, d);
            assert_eq!(civil_from_days(days), (y, m, d));
        }
    }

    #[test]
    fn test_weekday_epoch() {
        // 1970-01-01 was a Thursday (index 0 in our system)
        assert_eq!(weekday_from_days(0), 0); // Thursday
    }

    // Format tests

    #[test]
    fn test_format_usec_zero() {
        assert_eq!(format_usec(0), "0");
    }

    #[test]
    fn test_format_usec_microseconds() {
        assert_eq!(format_usec(500), "500us");
    }

    #[test]
    fn test_format_usec_milliseconds() {
        assert_eq!(format_usec(5000), "5ms");
    }

    #[test]
    fn test_format_usec_seconds() {
        assert_eq!(format_usec(1_500_000), "1.500s");
    }

    #[test]
    fn test_format_usec_minutes() {
        let ts = TimeSpan {
            usec: 2 * USEC_PER_MINUTE + 30 * USEC_PER_SEC,
        };
        assert_eq!(ts.to_string(), "2min 30s");
    }

    // Weekday parsing tests (via CalendarSpec)

    #[test]
    fn test_weekday_parsing() {
        assert!(CalendarSpec::parse("Mon *-*-* 00:00:00").is_ok());
        assert!(CalendarSpec::parse("Fri *-*-* 00:00:00").is_ok());
        assert!(CalendarSpec::parse("Sun *-*-* 00:00:00").is_ok());
        // "Foo" is not a weekday, so it should be treated as a date/time part
        // and fail to parse as a valid date
        assert!(CalendarSpec::parse("Foo *-*-* 00:00:00").is_err());
    }

    // Unit paths tests

    #[test]
    fn test_system_unit_paths_not_empty() {
        let paths = system_unit_paths();
        assert!(!paths.is_empty());
        assert!(
            paths
                .iter()
                .any(|p| p == &PathBuf::from("/etc/systemd/system"))
        );
    }

    #[test]
    fn test_user_unit_paths_not_empty() {
        let paths = user_unit_paths();
        assert!(!paths.is_empty());
    }

    // Verify tests

    #[test]
    fn test_verify_nonexistent_file() {
        let issues = verify_unit_file("/nonexistent/test.service");
        assert!(!issues.is_empty());
        assert!(issues[0].contains("Cannot open"));
    }

    // Condition tests

    #[test]
    fn test_condition_path_exists_root() {
        let (met, _) = evaluate_condition("ConditionPathExists=/");
        assert!(met);
    }

    #[test]
    fn test_condition_path_exists_missing() {
        let (met, _) = evaluate_condition("ConditionPathExists=/nonexistent/path/foobar");
        assert!(!met);
    }

    #[test]
    fn test_condition_path_is_directory() {
        let (met, _) = evaluate_condition("ConditionPathIsDirectory=/tmp");
        assert!(met);
    }

    #[test]
    fn test_condition_path_is_directory_file() {
        let (met, _) = evaluate_condition("ConditionPathIsDirectory=/etc/hostname");
        // /etc/hostname might not exist in all test environments
        // but if it does, it's not a directory
        if Path::new("/etc/hostname").exists() {
            assert!(!met);
        }
    }

    #[test]
    fn test_condition_invalid_format() {
        let (met, reason) = evaluate_condition("garbage");
        assert!(!met);
        assert!(reason.contains("Invalid"));
    }

    #[test]
    fn test_condition_unknown_type() {
        let (met, reason) = evaluate_condition("ConditionFoo=bar");
        assert!(!met);
        assert!(reason.contains("Unknown"));
    }

    // Boot timing tests

    #[test]
    fn test_read_boot_timing_no_panic() {
        let _bt = read_boot_timing();
    }

    #[test]
    fn test_read_unit_timings_no_panic() {
        let _timings = read_unit_timings();
    }

    // Detect virtualization test

    #[test]
    fn test_detect_virtualization_no_panic() {
        let _v = detect_virtualization();
    }

    // Format timestamp test

    #[test]
    fn test_format_timestamp_epoch() {
        let s = format_timestamp(UNIX_EPOCH);
        assert!(s.contains("1970"));
        assert!(s.contains("Thu"));
    }

    #[test]
    fn test_format_timestamp_2000() {
        let t = UNIX_EPOCH + Duration::from_secs(946684800); // 2000-01-01 00:00:00 UTC
        let s = format_timestamp(t);
        assert!(s.contains("2000"));
    }

    // Hostname test

    #[test]
    fn test_hostname_no_panic() {
        let _h = hostname();
    }

    // Unit timing tests

    #[test]
    fn test_unit_timing_duration() {
        let t = UnitTiming {
            name: "test.service".to_string(),
            activating_us: 1000,
            active_us: 5000,
        };
        assert_eq!(t.duration_us(), 4000);
    }

    #[test]
    fn test_unit_timing_duration_zero() {
        let t = UnitTiming {
            name: "test.service".to_string(),
            activating_us: 5000,
            active_us: 5000,
        };
        assert_eq!(t.duration_us(), 0);
    }

    #[test]
    fn test_unit_timing_duration_saturating() {
        let t = UnitTiming {
            name: "test.service".to_string(),
            activating_us: 10000,
            active_us: 5000,
        };
        assert_eq!(t.duration_us(), 0);
    }

    // Find unit file test

    #[test]
    fn test_find_unit_file_nonexistent() {
        assert!(find_unit_file("nonexistent-unit-name-12345.service").is_none());
    }

    // Parse time unit edge cases

    #[test]
    fn test_parse_time_unit_empty() {
        let (mult, len) = parse_time_unit("").unwrap();
        assert_eq!(mult, 1);
        assert_eq!(len, 0);
    }

    #[test]
    fn test_parse_time_unit_unknown() {
        assert!(parse_time_unit("foobar").is_err());
    }

    // Normalize calendar expression (via CalendarSpec::normalized)

    #[test]
    fn test_normalize_calendar_date_only() {
        let spec = CalendarSpec::parse("2023-12-25").unwrap();
        let norm = spec.normalized();
        assert!(norm.contains("2023-12-25"));
        assert!(norm.contains("00:00:00"));
    }

    #[test]
    fn test_normalize_calendar_time_only() {
        let spec = CalendarSpec::parse("12:30").unwrap();
        let norm = spec.normalized();
        assert!(norm.contains("12:30:00"));
    }

    #[test]
    fn test_normalize_calendar_with_dow() {
        let spec = CalendarSpec::parse("Mon *-*-* 09:00").unwrap();
        let norm = spec.normalized();
        assert!(norm.starts_with("Mon"));
        assert!(norm.contains("09:00:00"));
    }
}
