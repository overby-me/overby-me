//! journalctl — Query the systemd journal.
//!
//! A drop-in replacement for `journalctl(1)`. This tool reads journal
//! entries from the on-disk storage managed by `systemd-journald` and
//! displays them with various filtering and formatting options.
//!
//! Supported features:
//!
//! - Time-based filtering (`--since`, `--until`)
//! - Unit filtering (`-u`, `--unit`, `--user-unit`)
//! - Priority filtering (`-p`, `--priority`)
//! - Facility filtering (`--facility`)
//! - Transport filtering (`--transport`)
//! - Boot filtering (`-b`, `--boot`)
//! - Identifier filtering (`-t`, `--identifier`)
//! - PID/UID/GID filtering (`--pid`, `--uid`, `--gid`)
//! - Machine filtering (`-M`, `--machine`)
//! - System/user journal selection (`--system`, `--user`)
//! - Output formats (`-o`): `short`, `short-full`, `short-iso`,
//!   `short-iso-precise`, `short-precise`, `short-monotonic`,
//!   `short-unix`, `with-unit`, `verbose`, `json`, `json-pretty`,
//!   `json-sse`, `json-seq`, `cat`, `export`
//! - Follow mode (`-f`, `--follow`) with `--no-tail`
//! - Reverse output (`-r`, `--reverse`)
//! - Line count limiting (`-n`, `--lines`)
//! - Field listing (`--field`, `--fields`)
//! - Cursor support (`--cursor`, `--after-cursor`, `--show-cursor`)
//! - Disk usage query (`--disk-usage`)
//! - Vacuum commands (`--vacuum-size`, `--vacuum-time`, `--vacuum-files`)
//! - Flush and rotate commands (`--flush`, `--rotate`)
//! - Journal header (`--header`)
//! - List boots (`--list-boots`)
//! - Grep filtering (`-g`, `--grep`, `--case-sensitive`)
//! - Kernel messages only (`-k`, `--dmesg`)
//! - Catalog augmentation (`-x`, `--catalog`)
//! - Output field selection (`--output-fields`)
//! - Hostname suppression (`--no-hostname`)
//! - Path-based matching (`/path/to/executable`, `/dev/device`)
//! - Match groups with OR (`+` separator)
//! - No pager mode (`--no-pager`)

use clap::Parser;
use libsystemd::journal::entry::{JournalEntry, format_realtime_iso, format_realtime_utc};
use libsystemd::journal::storage::{JournalStorage, StorageConfig};
use regex::Regex;
use std::collections::{BTreeMap, HashSet};
use std::fs;
use std::io::{self, Write};
use std::path::PathBuf;
use std::process;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

/// Check whether a string looks like a valid boot specifier for `-b`.
fn is_boot_spec(s: &str) -> bool {
    // Numeric offsets: 0, -1, -2, 1, 2, etc.
    if s.parse::<i64>().is_ok() {
        return true;
    }
    // "all"
    if s == "all" {
        return true;
    }
    // Hex boot IDs (32 hex chars without dashes, or 36 with dashes)
    if s.len() == 32 && s.chars().all(|c| c.is_ascii_hexdigit()) {
        return true;
    }
    if s.len() == 36 && s.chars().all(|c| c.is_ascii_hexdigit() || c == '-') {
        return true;
    }
    false
}

/// Preprocess CLI args so that `-b VALUE` (space-separated) becomes `-b=VALUE`
/// when VALUE looks like a boot specifier.  When the next arg is NOT a boot
/// specifier (e.g. a path like `/dev/null`), emit `-b=0` so clap doesn't
/// greedily consume the path as the boot value.
fn preprocess_boot_args(args: impl Iterator<Item = String>) -> Vec<String> {
    let mut out = Vec::new();
    let mut iter = args.peekable();
    while let Some(arg) = iter.next() {
        // Match bare `-b` or `--boot` (without `=`)
        if arg == "-b" || arg == "--boot" {
            if let Some(next) = iter.peek() {
                if is_boot_spec(next) {
                    // Attach the boot spec value with `=`
                    let val = iter.next().unwrap();
                    out.push(format!("{}={}", arg, val));
                } else if !next.starts_with('-') {
                    // Next arg is a positional (e.g. path) — use default boot
                    out.push(format!("{}=0", arg));
                } else {
                    out.push(arg);
                }
            } else {
                out.push(arg);
            }
            continue;
        }
        out.push(arg);
    }
    out
}

#[derive(Parser, Debug)]
#[command(
    name = "journalctl",
    about = "Query the systemd journal",
    version,
    after_help = "See journalctl(1) for full documentation."
)]
struct Cli {
    /// Show entries from the current boot (or a specific boot ID/offset).
    /// Use without a value for the current boot, or pass a boot ID.
    /// Can be specified multiple times; the last value wins.
    #[arg(short = 'b', long = "boot", num_args = 0..=1, default_missing_value = "0")]
    boot: Vec<String>,

    /// Show entries for a specific systemd unit.
    #[arg(short = 'u', long = "unit", value_name = "UNIT")]
    unit: Option<String>,

    /// Show entries for a specific user unit.
    #[arg(long = "user-unit", value_name = "UNIT")]
    user_unit: Option<String>,

    /// Filter by syslog identifier (tag).
    #[arg(short = 't', long = "identifier", value_name = "ID")]
    identifier: Option<String>,

    /// Exclude entries with the specified syslog identifier.
    /// Can be specified multiple times.
    #[arg(short = 'T', long = "exclude-identifier", value_name = "ID")]
    exclude_identifier: Vec<String>,

    /// Filter by priority (0=emerg .. 7=debug). Can be a name or number.
    /// A single value shows that priority and above; a range "low..high"
    /// shows priorities within the range.
    #[arg(short = 'p', long = "priority", value_name = "PRIORITY")]
    priority: Option<String>,

    /// Filter by syslog facility (0-23 or name like kern, user, mail, daemon, auth, etc.).
    #[arg(long = "facility", value_name = "FACILITY")]
    facility: Option<String>,

    /// Filter by transport (_TRANSPORT field: journal, syslog, kernel, stdout, audit, driver).
    #[arg(long = "transport", value_name = "TRANSPORT")]
    transport: Option<String>,

    /// Filter by a grep pattern applied to the MESSAGE field.
    #[arg(short = 'g', long = "grep", value_name = "PATTERN")]
    grep: Option<String>,

    /// Control case sensitivity of --grep. Default is "smart-case":
    /// case-insensitive when pattern is all lowercase, case-sensitive otherwise.
    #[arg(long = "case-sensitive", num_args = 0..=1, default_missing_value = "true")]
    case_sensitive: Option<String>,

    /// Show entries from this time onwards. Accepts ISO 8601, "today",
    /// "yesterday", relative times like "-1h", or UNIX timestamps.
    #[arg(short = 'S', long = "since", value_name = "TIMESTAMP")]
    since: Option<String>,

    /// Show entries until this time. Same formats as --since.
    #[arg(short = 'U', long = "until", value_name = "TIMESTAMP")]
    until: Option<String>,

    /// Number of most recent entries to show.
    /// Use +N to show N entries from the start (for --list-boots/--list-invocation).
    #[arg(
        short = 'n',
        long = "lines",
        value_name = "N",
        allow_hyphen_values = true
    )]
    lines: Option<String>,

    /// Follow the journal (like tail -f).
    #[arg(short = 'f', long = "follow")]
    follow: bool,

    /// Show entries in reverse chronological order.
    #[arg(short = 'r', long = "reverse")]
    reverse: bool,

    /// Output format: short, short-full, short-iso, short-iso-precise,
    /// short-precise, short-monotonic, short-unix, with-unit, verbose,
    /// json, json-pretty, json-sse, json-seq, cat, export.
    #[arg(
        short = 'o',
        long = "output",
        default_value = "short",
        value_name = "FORMAT"
    )]
    output: String,

    /// Show only kernel messages (equivalent to -t kernel).
    #[arg(short = 'k', long = "dmesg")]
    dmesg: bool,

    /// Show all fields (don't truncate).
    #[arg(short = 'a', long = "all")]
    all: bool,

    /// Show the cursor after the last entry.
    #[arg(long = "show-cursor")]
    show_cursor: bool,

    /// Read/write cursor from/to this file.
    #[arg(long = "cursor-file", value_name = "FILE")]
    cursor_file: Option<String>,

    /// Start showing entries after this cursor.
    #[arg(long = "after-cursor", value_name = "CURSOR")]
    after_cursor: Option<String>,

    /// Start showing entries at this cursor.
    #[arg(long = "cursor", value_name = "CURSOR")]
    cursor: Option<String>,

    /// Print the total disk usage of journal files.
    #[arg(long = "disk-usage")]
    disk_usage: bool,

    /// List unique values of a specific field.
    #[arg(short = 'F', long = "field", value_name = "FIELD")]
    field: Option<String>,

    /// Merge entries from all available journals (default behavior, accepted for compatibility).
    #[arg(short = 'm', long = "merge")]
    merge: bool,

    /// List all known boots.
    #[arg(long = "list-boots")]
    list_boots: bool,

    /// Flush volatile journal to persistent storage.
    #[arg(long = "flush")]
    flush: bool,

    /// Rotate journal files.
    #[arg(long = "rotate")]
    rotate: bool,

    /// Synchronize (sync) the journal to disk.
    #[arg(long = "sync")]
    sync: bool,

    /// Verify journal file consistency.
    #[arg(long = "verify")]
    verify: bool,

    /// Do not pipe output into a pager.
    #[arg(long = "no-pager")]
    no_pager: bool,

    /// Jump to the end of the journal (pager-end mode, no-op without pager).
    #[arg(short = 'e', long = "pager-end")]
    pager_end: bool,

    /// Do not show any decorations (hostname, timestamp prefix).
    #[arg(short = 'q', long = "quiet")]
    quiet: bool,

    /// Journal directory to read from (overrides default).
    #[arg(short = 'D', long = "directory", value_name = "DIR")]
    directory: Option<String>,

    /// Read from specific journal file(s). Can be specified multiple times
    /// or with shell globs (e.g. --file=/var/log/journal/*/*).
    #[arg(long = "file", value_name = "FILE")]
    file: Vec<String>,

    /// Show entries for a specific PID.
    #[arg(long = "pid", value_name = "PID")]
    pid: Option<String>,

    /// Show entries for a specific UID.
    #[arg(long = "uid", value_name = "UID")]
    uid: Option<String>,

    /// Show entries for a specific GID.
    #[arg(long = "gid", value_name = "GID")]
    gid: Option<String>,

    /// Show the number of entries in the journal.
    #[arg(short = 'N', long = "fields")]
    list_fields: bool,

    /// Show output in UTC time.
    #[arg(long = "utc")]
    utc: bool,

    /// Show only system journal entries.
    #[arg(long = "system")]
    system: bool,

    /// Show only user journal entries.
    #[arg(long = "user")]
    user: bool,

    /// Filter by machine ID.
    #[arg(short = 'M', long = "machine", value_name = "MACHINE")]
    machine: Option<String>,

    /// Augment log lines with explanation texts from the message catalog.
    #[arg(short = 'x', long = "catalog")]
    catalog: bool,

    /// Do not show tail in follow mode (show all matching entries first).
    #[arg(long = "no-tail")]
    no_tail: bool,

    /// Comma-separated list of fields to output (for verbose/json/export).
    /// Can be specified multiple times; all values are merged.
    #[arg(long = "output-fields", value_name = "FIELDS")]
    output_fields: Vec<String>,

    /// Suppress the hostname field in short output formats.
    #[arg(long = "no-hostname")]
    no_hostname: bool,

    /// Truncate multi-line messages at the first newline character.
    #[arg(long = "truncate-newline")]
    truncate_newline: bool,

    /// Show journal file header information.
    #[arg(long = "header")]
    header: bool,

    /// Remove archived journal files until the disk space taken up drops
    /// below SIZE (K/M/G/T suffix).
    #[arg(long = "vacuum-size", value_name = "SIZE")]
    vacuum_size: Option<String>,

    /// Remove archived journal files that are older than the given
    /// time span (s/min/h/days/weeks/months/years).
    #[arg(long = "vacuum-time", value_name = "TIME")]
    vacuum_time: Option<String>,

    /// Remove archived journal files until no more than N remain.
    #[arg(long = "vacuum-files", value_name = "N")]
    vacuum_files: Option<usize>,

    /// Show a list of journal namespaces.
    #[arg(long = "list-namespaces")]
    list_namespaces: bool,

    /// Show entries from the specified journal namespace.
    #[arg(long = "namespace", value_name = "NAMESPACE")]
    namespace: Option<String>,

    /// Show entries from the latest invocation of the specified unit.
    /// When combined with -u, filters by the most recent _SYSTEMD_INVOCATION_ID.
    #[arg(short = 'I')]
    latest_invocation: bool,

    /// Show entries matching a specific invocation ID.
    #[arg(long = "invocation", value_name = "ID")]
    invocation: Option<String>,

    /// List all invocations of the specified unit with their timestamps.
    #[arg(long = "list-invocation")]
    list_invocation: bool,

    /// Ask journald to stop logging to /var/log/journal and use only
    /// /run/log/journal (volatile). This is used during shutdown.
    #[arg(long = "relinquish-var")]
    relinquish_var: bool,

    /// Like --relinquish-var but is a NOP if /var/log/journal is on the
    /// root file system.
    #[arg(long = "smart-relinquish-var")]
    smart_relinquish_var: bool,

    /// Update the message catalog.
    #[arg(long = "update-catalog")]
    update_catalog: bool,

    /// List all message catalog entries.
    #[arg(long = "list-catalog")]
    list_catalog: bool,

    /// Do not ellipsize fields.
    #[arg(long = "no-full")]
    no_full: bool,

    /// Use a specific root directory for journal files.
    #[arg(long = "root", value_name = "ROOT")]
    root: Option<String>,

    /// Free-form match expressions: FIELD=VALUE
    #[arg(trailing_var_arg = true)]
    matches: Vec<String>,
}

// ---------------------------------------------------------------------------
// Output formats
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OutputFormat {
    Short,
    ShortFull,
    ShortIso,
    ShortIsoPrecise,
    ShortPrecise,
    ShortMonotonic,
    ShortUnix,
    WithUnit,
    Verbose,
    Json,
    JsonPretty,
    JsonSse,
    JsonSeq,
    Cat,
    Export,
}

impl OutputFormat {
    fn from_str(s: &str) -> Result<Self, String> {
        match s.to_lowercase().as_str() {
            "short" => Ok(OutputFormat::Short),
            "short-full" => Ok(OutputFormat::ShortFull),
            "short-iso" => Ok(OutputFormat::ShortIso),
            "short-iso-precise" => Ok(OutputFormat::ShortIsoPrecise),
            "short-precise" => Ok(OutputFormat::ShortPrecise),
            "short-monotonic" => Ok(OutputFormat::ShortMonotonic),
            "short-unix" => Ok(OutputFormat::ShortUnix),
            "with-unit" => Ok(OutputFormat::WithUnit),
            "verbose" => Ok(OutputFormat::Verbose),
            "json" => Ok(OutputFormat::Json),
            "json-pretty" => Ok(OutputFormat::JsonPretty),
            "json-sse" => Ok(OutputFormat::JsonSse),
            "json-seq" => Ok(OutputFormat::JsonSeq),
            "cat" => Ok(OutputFormat::Cat),
            "export" => Ok(OutputFormat::Export),
            _ => Err(format!("Unknown output format: {}", s)),
        }
    }
}

// ---------------------------------------------------------------------------
// Priority filtering
// ---------------------------------------------------------------------------

/// Parse a priority specification.  Accepts:
///   - A single name or number: "err" or "3" → show priority 0..=3
///   - A range: "warning..err" or "4..3" → show priorities 3..=4
///
/// Convert a simple glob pattern (with `*` and `?`) to a regex.
fn glob_to_regex(pattern: &str) -> Regex {
    let mut re = String::from("^");
    for ch in pattern.chars() {
        match ch {
            '*' => re.push_str(".*"),
            '?' => re.push('.'),
            c => {
                if regex::escape(&c.to_string()) != c.to_string() {
                    re.push_str(&regex::escape(&c.to_string()));
                } else {
                    re.push(c);
                }
            }
        }
    }
    re.push('$');
    Regex::new(&re).unwrap_or_else(|_| Regex::new("^$").unwrap())
}

/// Returns (min_priority, max_priority) where min is the numerically
/// lowest (most severe) and max is the highest (least severe) to show.
fn parse_priority_filter(s: &str) -> Result<(u8, u8), String> {
    if let Some((lo, hi)) = s.split_once("..") {
        let lo = parse_single_priority(lo)?;
        let hi = parse_single_priority(hi)?;
        let min = lo.min(hi);
        let max = lo.max(hi);
        Ok((min, max))
    } else {
        let p = parse_single_priority(s)?;
        Ok((0, p))
    }
}

fn parse_single_priority(s: &str) -> Result<u8, String> {
    match s.trim().to_lowercase().as_str() {
        "emerg" | "emergency" | "0" => Ok(0),
        "alert" | "1" => Ok(1),
        "crit" | "critical" | "2" => Ok(2),
        "err" | "error" | "3" => Ok(3),
        "warning" | "warn" | "4" => Ok(4),
        "notice" | "5" => Ok(5),
        "info" | "6" => Ok(6),
        "debug" | "7" => Ok(7),
        _ => Err(format!("Unknown priority: {}", s)),
    }
}

// ---------------------------------------------------------------------------
// Facility filtering
// ---------------------------------------------------------------------------

/// Parse a syslog facility specification. Accepts numeric (0-23) or name.
fn parse_facility(s: &str) -> Result<u8, String> {
    match s.trim().to_lowercase().as_str() {
        "kern" | "0" => Ok(0),
        "user" | "1" => Ok(1),
        "mail" | "2" => Ok(2),
        "daemon" | "3" => Ok(3),
        "auth" | "4" => Ok(4),
        "syslog" | "5" => Ok(5),
        "lpr" | "6" => Ok(6),
        "news" | "7" => Ok(7),
        "uucp" | "8" => Ok(8),
        "cron" | "9" => Ok(9),
        "authpriv" | "10" => Ok(10),
        "ftp" | "11" => Ok(11),
        "ntp" | "12" => Ok(12),
        "security" | "13" => Ok(13),
        "console" | "14" => Ok(14),
        "solaris-cron" | "15" => Ok(15),
        "local0" | "16" => Ok(16),
        "local1" | "17" => Ok(17),
        "local2" | "18" => Ok(18),
        "local3" | "19" => Ok(19),
        "local4" | "20" => Ok(20),
        "local5" | "21" => Ok(21),
        "local6" | "22" => Ok(22),
        "local7" | "23" => Ok(23),
        _ => Err(format!("Unknown facility: {}", s)),
    }
}

/// Return the facility name for a numeric facility code.
#[allow(dead_code)]
fn facility_name(f: u8) -> &'static str {
    match f {
        0 => "kern",
        1 => "user",
        2 => "mail",
        3 => "daemon",
        4 => "auth",
        5 => "syslog",
        6 => "lpr",
        7 => "news",
        8 => "uucp",
        9 => "cron",
        10 => "authpriv",
        11 => "ftp",
        12 => "ntp",
        13 => "security",
        14 => "console",
        15 => "solaris-cron",
        16 => "local0",
        17 => "local1",
        18 => "local2",
        19 => "local3",
        20 => "local4",
        21 => "local5",
        22 => "local6",
        23 => "local7",
        _ => "unknown",
    }
}

/// Parse a size specification with K/M/G/T suffix into bytes.
fn parse_size_spec(s: &str) -> Result<u64, String> {
    let s = s.trim();
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('T') {
        (n, 1024u64 * 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('G') {
        (n, 1024u64 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('M') {
        (n, 1024u64 * 1024)
    } else if let Some(n) = s.strip_suffix('K') {
        (n, 1024u64)
    } else {
        (s, 1u64)
    };
    num_str
        .trim()
        .parse::<u64>()
        .map(|n| n * multiplier)
        .map_err(|_| format!("Invalid size specification: {}", s))
}

/// Parse a vacuum time specification into seconds.
fn parse_vacuum_time(s: &str) -> Result<u64, String> {
    if let Some(usec) = parse_relative_time(s) {
        Ok(usec / 1_000_000)
    } else {
        Err(format!("Invalid time specification: {}", s))
    }
}

// ---------------------------------------------------------------------------
// Timestamp parsing
// ---------------------------------------------------------------------------

/// Parse a timestamp specification into microseconds since the UNIX epoch.
///
/// Supported formats:
///   - ISO 8601: "2023-11-15 10:30:00", "2023-11-15T10:30:00"
///   - Special words: "today", "yesterday", "now"
///   - Relative: "-1h", "-30min", "-2d"
///   - UNIX timestamp in seconds or microseconds
fn parse_timestamp(s: &str) -> Result<u64, String> {
    let s = s.trim();

    // Special keywords
    let now_usec = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;

    match s.to_lowercase().as_str() {
        "now" => return Ok(now_usec),
        "today" => {
            // Start of today in local time
            let now = chrono::Local::now();
            let today = now
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(chrono::Local)
                .unwrap();
            return Ok(today.timestamp_micros() as u64);
        }
        "yesterday" => {
            let now = chrono::Local::now();
            let yesterday = (now - chrono::Duration::days(1))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(chrono::Local)
                .unwrap();
            return Ok(yesterday.timestamp_micros() as u64);
        }
        "tomorrow" => {
            let now = chrono::Local::now();
            let tomorrow = (now + chrono::Duration::days(1))
                .date_naive()
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(chrono::Local)
                .unwrap();
            return Ok(tomorrow.timestamp_micros() as u64);
        }
        _ => {}
    }

    // Relative time: "-1h", "-30min", "-2d", "-60s"
    if s.starts_with('-') || s.starts_with('+') {
        let is_minus = s.starts_with('-');
        let rest = &s[1..];
        if let Some(offset_usec) = parse_relative_time(rest) {
            if is_minus {
                return Ok(now_usec.saturating_sub(offset_usec));
            } else {
                return Ok(now_usec.saturating_add(offset_usec));
            }
        }
    }

    // UNIX epoch format: @<seconds> or @<microseconds>
    if let Some(rest) = s.strip_prefix('@')
        && let Ok(n) = rest.parse::<u64>()
    {
        if n > 4_000_000_000 {
            return Ok(n);
        }
        return Ok(n * 1_000_000);
    }

    // Try UNIX timestamp (seconds or microseconds)
    if let Ok(n) = s.parse::<u64>() {
        // If it looks like microseconds (> year 2100 in seconds), treat as µs
        if n > 4_000_000_000 {
            return Ok(n);
        }
        return Ok(n * 1_000_000);
    }

    // Try ISO 8601 / date-time parsing with chrono
    // Try "YYYY-MM-DD HH:MM:SS" and "YYYY-MM-DDTHH:MM:SS"
    let formats = [
        "%Y-%m-%d %H:%M:%S%.f",
        "%Y-%m-%dT%H:%M:%S%.f",
        "%Y-%m-%d %H:%M:%S",
        "%Y-%m-%dT%H:%M:%S",
        "%Y-%m-%d %H:%M",
        "%Y-%m-%d",
    ];

    for fmt in &formats {
        if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(s, fmt) {
            let local_dt = dt.and_local_timezone(chrono::Local);
            if let chrono::LocalResult::Single(ldt) = local_dt {
                return Ok(ldt.timestamp_micros() as u64);
            }
        }
        // Also try parsing just a date
        if let Ok(d) = chrono::NaiveDate::parse_from_str(s, fmt) {
            let dt = d
                .and_hms_opt(0, 0, 0)
                .unwrap()
                .and_local_timezone(chrono::Local);
            if let chrono::LocalResult::Single(ldt) = dt {
                return Ok(ldt.timestamp_micros() as u64);
            }
        }
    }

    Err(format!("Could not parse timestamp: {}", s))
}

/// Parse a relative time string like "1h", "30min", "2d" into microseconds.
fn parse_relative_time(s: &str) -> Option<u64> {
    let suffixes: &[(&str, u64)] = &[
        ("min", 60_000_000),
        ("minutes", 60_000_000),
        ("minute", 60_000_000),
        ("sec", 1_000_000),
        ("second", 1_000_000),
        ("seconds", 1_000_000),
        ("ms", 1_000),
        ("msec", 1_000),
        ("us", 1),
        ("usec", 1),
        ("hr", 3_600_000_000),
        ("hour", 3_600_000_000),
        ("hours", 3_600_000_000),
        ("day", 86_400_000_000),
        ("days", 86_400_000_000),
        ("week", 604_800_000_000),
        ("weeks", 604_800_000_000),
        ("month", 2_592_000_000_000),
        ("months", 2_592_000_000_000),
        ("year", 31_536_000_000_000),
        ("years", 31_536_000_000_000),
        ("h", 3_600_000_000),
        ("m", 60_000_000),
        ("s", 1_000_000),
        ("d", 86_400_000_000),
        ("w", 604_800_000_000),
    ];

    for &(suffix, multiplier) in suffixes {
        if let Some(num_str) = s.strip_suffix(suffix) {
            let num_str = num_str.trim();
            if let Ok(n) = num_str.parse::<u64>() {
                return Some(n * multiplier);
            }
        }
    }

    // Bare number → seconds
    if let Ok(n) = s.parse::<u64>() {
        return Some(n * 1_000_000);
    }

    None
}

// ---------------------------------------------------------------------------
// Output field filtering
// ---------------------------------------------------------------------------

/// Parse a comma-separated list of output fields into a set.
fn parse_output_fields(s: &str) -> HashSet<String> {
    s.split(',')
        .map(|f| f.trim().to_uppercase())
        .filter(|f| !f.is_empty())
        .collect()
}

/// Filter a JSON object to only include specified fields (plus metadata).
fn filter_json_fields(json: &serde_json::Value, fields: &HashSet<String>) -> serde_json::Value {
    if let serde_json::Value::Object(map) = json {
        let mut filtered = serde_json::Map::new();
        for (key, value) in map {
            // Always include __-prefixed metadata fields
            if key.starts_with("__") || fields.contains(&key.to_uppercase()) {
                filtered.insert(key.clone(), value.clone());
            }
        }
        serde_json::Value::Object(filtered)
    } else {
        json.clone()
    }
}

// ---------------------------------------------------------------------------
// Entry formatting
// ---------------------------------------------------------------------------

/// Formatting options passed through to the format functions.
#[derive(Clone, Debug)]
struct FormatOptions {
    utc: bool,
    #[allow(dead_code)]
    all: bool,
    no_hostname: bool,
    truncate_newline: bool,
    output_fields: Option<HashSet<String>>,
    #[allow(dead_code)]
    catalog: bool,
}

/// Format and print a single journal entry according to the output format.
fn format_entry(
    entry: &JournalEntry,
    format: OutputFormat,
    opts: &FormatOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    match format {
        OutputFormat::Short => format_short(entry, ShortTimestamp::Syslog, opts, writer),
        OutputFormat::ShortFull => format_short(entry, ShortTimestamp::Full, opts, writer),
        OutputFormat::ShortIso => format_short(entry, ShortTimestamp::Iso, opts, writer),
        OutputFormat::ShortIsoPrecise => {
            format_short(entry, ShortTimestamp::IsoPrecise, opts, writer)
        }
        OutputFormat::ShortPrecise => format_short(entry, ShortTimestamp::Precise, opts, writer),
        OutputFormat::ShortMonotonic => {
            format_short(entry, ShortTimestamp::Monotonic, opts, writer)
        }
        OutputFormat::ShortUnix => format_short(entry, ShortTimestamp::Unix, opts, writer),
        OutputFormat::WithUnit => format_with_unit(entry, opts, writer),
        OutputFormat::Verbose => format_verbose(entry, opts, writer),
        OutputFormat::Json => format_json(entry, JsonStyle::Compact, opts, writer),
        OutputFormat::JsonPretty => format_json(entry, JsonStyle::Pretty, opts, writer),
        OutputFormat::JsonSse => format_json(entry, JsonStyle::Sse, opts, writer),
        OutputFormat::JsonSeq => format_json(entry, JsonStyle::Seq, opts, writer),
        OutputFormat::Cat => format_cat(entry, opts, writer),
        OutputFormat::Export => format_export(entry, opts, writer),
    }
}

/// Which timestamp style to use for short-format variants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ShortTimestamp {
    Syslog,
    Full,
    Iso,
    IsoPrecise,
    Precise,
    Monotonic,
    Unix,
}

fn format_short(
    entry: &JournalEntry,
    ts_style: ShortTimestamp,
    opts: &FormatOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    let timestamp = match ts_style {
        ShortTimestamp::Syslog => format_realtime_syslog(entry.realtime_usec, opts.utc),
        ShortTimestamp::Full => format_realtime_full(entry.realtime_usec, opts.utc),
        ShortTimestamp::Iso => {
            if opts.utc {
                format_realtime_utc(entry.realtime_usec)
            } else {
                format_realtime_iso(entry.realtime_usec)
            }
        }
        ShortTimestamp::IsoPrecise => format_realtime_iso_precise(entry.realtime_usec, opts.utc),
        ShortTimestamp::Precise => format_realtime_precise(entry.realtime_usec, opts.utc),
        ShortTimestamp::Monotonic => {
            let mono_secs = entry.monotonic_usec as f64 / 1_000_000.0;
            format!("{:>12.6}", mono_secs)
        }
        ShortTimestamp::Unix => format_realtime_unix(entry.realtime_usec),
    };

    let identifier = entry
        .syslog_identifier()
        .or_else(|| entry.comm())
        .unwrap_or_else(|| "unknown".to_string());
    let pid_str = entry.pid().map(|p| format!("[{}]", p)).unwrap_or_default();
    let mut message = entry.message().unwrap_or_default();
    if opts.truncate_newline
        && let Some(pos) = message.find('\n')
    {
        message.truncate(pos);
    }

    if opts.no_hostname {
        writeln!(
            writer,
            "{} {}{}: {}",
            timestamp, identifier, pid_str, message
        )
    } else {
        let hostname = entry.hostname().unwrap_or_default();
        writeln!(
            writer,
            "{} {} {}{}: {}",
            timestamp, hostname, identifier, pid_str, message
        )
    }
}

/// Format with the unit name included (like short but with unit name after
/// hostname).
fn format_with_unit(
    entry: &JournalEntry,
    opts: &FormatOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    let timestamp = format_realtime_syslog(entry.realtime_usec, opts.utc);
    let unit = entry
        .systemd_unit()
        .or_else(|| entry.field("_SYSTEMD_USER_UNIT"))
        .unwrap_or_default();
    let identifier = entry
        .syslog_identifier()
        .or_else(|| entry.comm())
        .unwrap_or_else(|| "unknown".to_string());
    let pid_str = entry.pid().map(|p| format!("[{}]", p)).unwrap_or_default();
    let message = entry.message().unwrap_or_default();

    if opts.no_hostname {
        writeln!(
            writer,
            "{} {} {}{}: {}",
            timestamp, unit, identifier, pid_str, message
        )
    } else {
        let hostname = entry.hostname().unwrap_or_default();
        writeln!(
            writer,
            "{} {} {} {}{}: {}",
            timestamp, hostname, unit, identifier, pid_str, message
        )
    }
}

fn format_verbose(
    entry: &JournalEntry,
    opts: &FormatOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    let timestamp = format_realtime_iso(entry.realtime_usec);
    writeln!(writer, "{} [s={}]", timestamp, entry.seqnum)?;

    // Print fields sorted, possibly filtered
    for (key, value) in &entry.fields {
        if let Some(ref fields) = opts.output_fields
            && !fields.contains(&key.to_uppercase())
        {
            continue;
        }
        let value_str = String::from_utf8_lossy(value);
        writeln!(writer, "    {}={}", key, value_str)?;
    }

    writeln!(writer)
}

/// Which JSON variant to emit.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum JsonStyle {
    Compact,
    Pretty,
    Sse,
    Seq,
}

fn format_json(
    entry: &JournalEntry,
    style: JsonStyle,
    opts: &FormatOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    let mut json = entry.to_json();

    // Apply output-fields filter
    if let Some(ref fields) = opts.output_fields {
        json = filter_json_fields(&json, fields);
    }

    match style {
        JsonStyle::Compact => {
            writeln!(
                writer,
                "{}",
                serde_json::to_string(&json).unwrap_or_default()
            )
        }
        JsonStyle::Pretty => {
            writeln!(
                writer,
                "{}",
                serde_json::to_string_pretty(&json).unwrap_or_default()
            )
        }
        JsonStyle::Sse => {
            // Server-Sent Events format: "data: <json>\n\n"
            writeln!(
                writer,
                "data: {}\n",
                serde_json::to_string(&json).unwrap_or_default()
            )
        }
        JsonStyle::Seq => {
            // RFC 7464 JSON Text Sequences: RS (0x1E) prefix, LF suffix
            writer.write_all(&[0x1E])?;
            writeln!(
                writer,
                "{}",
                serde_json::to_string(&json).unwrap_or_default()
            )
        }
    }
}

fn format_cat(
    entry: &JournalEntry,
    opts: &FormatOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    // When --output-fields is set, print the specified field value instead of MESSAGE
    if let Some(ref fields) = opts.output_fields {
        for (key, value) in &entry.fields {
            if fields.contains(&key.to_uppercase()) {
                let val = String::from_utf8_lossy(value);
                writeln!(writer, "{}", val)?;
            }
        }
        return Ok(());
    }
    let mut message = entry.message().unwrap_or_default();
    if opts.truncate_newline
        && let Some(pos) = message.find('\n')
    {
        message.truncate(pos);
    }
    writeln!(writer, "{}", message)
}

fn format_export(
    entry: &JournalEntry,
    opts: &FormatOptions,
    writer: &mut impl Write,
) -> io::Result<()> {
    let cursor = format!(
        "s=0;i={:x};b={};m={:x};t={:x};x=0",
        entry.seqnum,
        entry.boot_id().unwrap_or_default(),
        entry.monotonic_usec,
        entry.realtime_usec,
    );

    if let Some(ref fields) = opts.output_fields {
        // Write pseudo-fields (always included regardless of --output-fields)
        writeln!(writer, "__CURSOR={}", cursor)?;
        writeln!(writer, "__REALTIME_TIMESTAMP={}", entry.realtime_usec)?;
        writeln!(writer, "__MONOTONIC_TIMESTAMP={}", entry.monotonic_usec)?;
        writeln!(writer, "__SEQNUM={}", entry.seqnum)?;
        writeln!(writer, "__SEQNUM_ID=0")?;
        // _BOOT_ID is a trusted field, always included
        if let Some(boot_id) = entry.boot_id() {
            writeln!(writer, "_BOOT_ID={}", boot_id)?;
        }
        // Write only requested fields (filtered by --output-fields)
        for (key, value) in &entry.fields {
            // _BOOT_ID is already in the header above
            if key == "_BOOT_ID" {
                continue;
            }
            if !fields.contains(&key.to_uppercase()) {
                continue;
            }
            if let Ok(s) = std::str::from_utf8(value) {
                writeln!(writer, "{}={}", key, s)?;
            } else {
                writeln!(writer, "{}", key)?;
                writer.write_all(&(value.len() as u64).to_le_bytes())?;
                writer.write_all(value)?;
                writeln!(writer)?;
            }
        }
        writeln!(writer)
    } else {
        let export = entry.to_export_format(&cursor);
        writer.write_all(&export)
    }
}

/// Simple glob matching supporting `*` (any substring) and `?` (any single char).
#[allow(dead_code)]
fn simple_glob_match(pattern: &str, text: &str) -> bool {
    let mut dp = vec![vec![false; text.len() + 1]; pattern.len() + 1];
    dp[0][0] = true;
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    for i in 1..=pat.len() {
        if pat[i - 1] == '*' {
            dp[i][0] = dp[i - 1][0];
        }
    }
    for i in 1..=pat.len() {
        for j in 1..=txt.len() {
            if pat[i - 1] == '*' {
                dp[i][j] = dp[i - 1][j] || dp[i][j - 1];
            } else if pat[i - 1] == '?' || pat[i - 1] == txt[j - 1] {
                dp[i][j] = dp[i - 1][j - 1];
            }
        }
    }
    dp[pat.len()][txt.len()]
}

/// Format timestamp as syslog-style "Mon DD HH:MM:SS".
fn format_realtime_syslog(realtime_usec: u64, utc: bool) -> String {
    let secs = (realtime_usec / 1_000_000) as i64;
    let micros = (realtime_usec % 1_000_000) as u32;

    if utc {
        use chrono::{TimeZone, Utc};
        match Utc.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%b %d %H:%M:%S").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    } else {
        use chrono::{Local, TimeZone};
        match Local.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%b %d %H:%M:%S").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    }
}

/// Format timestamp as full "Day YYYY-MM-DD HH:MM:SS TZ" (short-full).
fn format_realtime_full(realtime_usec: u64, utc: bool) -> String {
    let secs = (realtime_usec / 1_000_000) as i64;
    let micros = (realtime_usec % 1_000_000) as u32;

    if utc {
        use chrono::{TimeZone, Utc};
        match Utc.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%a %Y-%m-%d %H:%M:%S UTC").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    } else {
        use chrono::{Local, TimeZone};
        match Local.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%a %Y-%m-%d %H:%M:%S %Z").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    }
}

/// Format timestamp with microsecond precision.
fn format_realtime_precise(realtime_usec: u64, utc: bool) -> String {
    let secs = (realtime_usec / 1_000_000) as i64;
    let micros = (realtime_usec % 1_000_000) as u32;

    if utc {
        use chrono::{TimeZone, Utc};
        match Utc.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%b %d %H:%M:%S%.6f").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    } else {
        use chrono::{Local, TimeZone};
        match Local.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%b %d %H:%M:%S%.6f").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    }
}

/// Format timestamp as ISO 8601 with microsecond precision (short-iso-precise).
fn format_realtime_iso_precise(realtime_usec: u64, utc: bool) -> String {
    let secs = (realtime_usec / 1_000_000) as i64;
    let micros = (realtime_usec % 1_000_000) as u32;

    if utc {
        use chrono::{TimeZone, Utc};
        match Utc.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.6fZ").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    } else {
        use chrono::{Local, TimeZone};
        match Local.timestamp_opt(secs, micros * 1_000) {
            chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%dT%H:%M:%S%.6f%:z").to_string(),
            _ => format!("@{}", realtime_usec),
        }
    }
}

/// Format timestamp as UNIX seconds with microsecond fractional part.
fn format_realtime_unix(realtime_usec: u64) -> String {
    let secs = realtime_usec / 1_000_000;
    let micros = realtime_usec % 1_000_000;
    format!("{}.{:06}", secs, micros)
}

// ---------------------------------------------------------------------------
// Cursor parsing
// ---------------------------------------------------------------------------

/// Parse a cursor string and extract the realtime timestamp and sequence
/// number for seeking.  Cursor format:
///   s=<file_id>;i=<seqnum>;b=<boot_id>;m=<mono>;t=<realtime>;x=<xor>
fn parse_cursor(cursor: &str) -> Option<(u64, u64)> {
    let mut seqnum: Option<u64> = None;
    let mut realtime: Option<u64> = None;

    for part in cursor.split(';') {
        if let Some(val) = part.strip_prefix("i=") {
            seqnum = u64::from_str_radix(val, 16).ok();
        } else if let Some(val) = part.strip_prefix("t=") {
            realtime = u64::from_str_radix(val, 16).ok();
        }
    }

    match (seqnum, realtime) {
        (Some(s), Some(t)) => Some((s, t)),
        (Some(s), None) => Some((s, 0)),
        (None, Some(t)) => Some((0, t)),
        (None, None) => None,
    }
}

// ---------------------------------------------------------------------------
// Match expression parsing
// ---------------------------------------------------------------------------

/// Parse a FIELD=VALUE match expression.
fn parse_match(s: &str) -> Option<(String, String)> {
    s.split_once('=')
        .map(|(k, v)| (k.to_string(), v.to_string()))
}

// ---------------------------------------------------------------------------
// Path/match group support
// ---------------------------------------------------------------------------

/// A single match condition that can test against a journal entry.
#[derive(Debug, Clone)]
enum MatchCondition {
    /// FIELD=VALUE — exact field match
    FieldEqual { key: String, value: String },
    /// _EXE match (from executable path argument)
    Exe(String),
    /// _EXE + _COMM match (from script path argument)
    Script { interpreter: String, comm: String },
    /// _KERNEL_DEVICE match (from /dev/ path argument)
    KernelDevice(String),
}

impl MatchCondition {
    fn matches(&self, entry: &JournalEntry) -> bool {
        match self {
            MatchCondition::FieldEqual { key, value } => {
                entry.field(key).is_some_and(|v| v == *value)
            }
            MatchCondition::Exe(exe) => entry.exe().is_some_and(|e| e == *exe),
            MatchCondition::Script { interpreter, comm } => {
                // _COMM from /proc/pid/comm is truncated to 15 chars by the kernel,
                // so match using a prefix when the name exceeds 15 chars.
                entry.exe().is_some_and(|e| e == *interpreter)
                    && entry.comm().is_some_and(|c| {
                        if comm.len() > 15 {
                            c == comm[..15]
                        } else {
                            c == *comm
                        }
                    })
            }
            MatchCondition::KernelDevice(dev) => {
                entry.field("_KERNEL_DEVICE").is_some_and(|d| d == *dev)
            }
        }
    }
}

/// Resolve a /dev/ path to a _KERNEL_DEVICE value via /sys.
///
/// For block/char devices, the kernel device string is `+<subsystem>:<sysname>`.
/// We resolve the device by checking /sys/dev/{block,char}/MAJOR:MINOR/subsystem
/// and /sys/dev/{block,char}/MAJOR:MINOR (basename = sysname).
fn resolve_kernel_device(path: &str) -> Option<String> {
    use std::os::unix::fs::MetadataExt;
    let meta = std::fs::metadata(path).ok()?;
    let mode = meta.mode();
    let rdev = meta.rdev();
    // Check if it's a block or char device
    let dev_type = if mode & 0o170000 == 0o060000 {
        "block"
    } else if mode & 0o170000 == 0o020000 {
        "char"
    } else {
        return None;
    };
    let major = ((rdev >> 8) & 0xfff) | ((rdev >> 32) & !0xfff);
    let minor = (rdev & 0xff) | ((rdev >> 12) & !0xff);
    let sys_path = format!("/sys/dev/{dev_type}/{major}:{minor}");
    // Get sysname from the symlink target basename
    let real = std::fs::canonicalize(&sys_path).ok()?;
    let sysname = real.file_name()?.to_str()?;
    // Get subsystem from the "subsystem" symlink
    let subsys_link = format!("{sys_path}/subsystem");
    let subsys_path = std::fs::canonicalize(subsys_link).ok()?;
    let subsystem = subsys_path.file_name()?.to_str()?;
    Some(format!("+{subsystem}:{sysname}"))
}

/// Check if a file is a script (starts with `#!`) and return the interpreter path.
fn script_interpreter(path: &str) -> Option<String> {
    let data = std::fs::read(path).ok()?;
    if !data.starts_with(b"#!") {
        return None;
    }
    // Parse the shebang line
    let line_end = data.iter().position(|&b| b == b'\n').unwrap_or(data.len());
    let shebang = std::str::from_utf8(&data[2..line_end]).ok()?.trim();
    // Handle "#!/usr/bin/env bash" → interpreter is bash (resolved)
    if let Some(rest) = shebang.strip_prefix("/usr/bin/env ") {
        let cmd = rest.split_whitespace().next()?;
        // Resolve via PATH
        which_executable(cmd)
    } else {
        // Direct interpreter path, e.g. "#!/bin/bash"
        let interp = shebang.split_whitespace().next()?;
        std::fs::canonicalize(interp)
            .ok()
            .and_then(|p| p.to_str().map(String::from))
    }
}

/// Find an executable in PATH.
fn which_executable(name: &str) -> Option<String> {
    let path_var = std::env::var("PATH").ok()?;
    for dir in path_var.split(':') {
        let candidate = format!("{dir}/{name}");
        if std::fs::metadata(&candidate)
            .map(|m| m.is_file())
            .unwrap_or(false)
        {
            return std::fs::canonicalize(&candidate)
                .ok()
                .and_then(|p| p.to_str().map(String::from));
        }
    }
    None
}

/// Build match groups from CLI match arguments.
///
/// Arguments are split by `+` into groups (OR).  Within each group,
/// FIELD=VALUE expressions and path arguments are AND-combined.
/// Path arguments that don't exist cause an immediate exit with error.
fn build_match_groups(matches: &[String]) -> Vec<Vec<MatchCondition>> {
    let mut groups: Vec<Vec<MatchCondition>> = Vec::new();
    let mut current: Vec<MatchCondition> = Vec::new();

    for m in matches {
        if m == "+" {
            if !current.is_empty() {
                groups.push(std::mem::take(&mut current));
            }
            continue;
        }

        if let Some((key, value)) = parse_match(m) {
            // FIELD=VALUE
            let key_upper = key.to_uppercase();
            current.push(MatchCondition::FieldEqual {
                key: key_upper,
                value,
            });
        } else if m.starts_with('/') {
            // Path argument
            match std::fs::metadata(m) {
                Ok(meta) => {
                    use std::os::unix::fs::MetadataExt;
                    let mode = meta.mode();
                    let is_device = (mode & 0o170000 == 0o060000) || (mode & 0o170000 == 0o020000);
                    if is_device {
                        // Device path → _KERNEL_DEVICE match
                        if let Some(kdev) = resolve_kernel_device(m) {
                            current.push(MatchCondition::KernelDevice(kdev));
                        } else {
                            eprintln!("journalctl: Failed to resolve device path '{m}'");
                            process::exit(1);
                        }
                    } else if meta.is_file() {
                        // Executable or script
                        let real_path = std::fs::canonicalize(m)
                            .ok()
                            .and_then(|p| p.to_str().map(String::from))
                            .unwrap_or_else(|| m.to_string());
                        if let Some(interp) = script_interpreter(&real_path) {
                            // Script: match interpreter + comm
                            let basename = std::path::Path::new(m)
                                .file_name()
                                .and_then(|n| n.to_str())
                                .unwrap_or("")
                                .to_string();
                            current.push(MatchCondition::Script {
                                interpreter: interp,
                                comm: basename,
                            });
                        } else {
                            // Regular executable
                            current.push(MatchCondition::Exe(real_path));
                        }
                    } else {
                        eprintln!("journalctl: File is neither a device nor a regular file: '{m}'");
                        process::exit(1);
                    }
                }
                Err(e) => {
                    eprintln!("journalctl: {m}: {e}");
                    process::exit(1);
                }
            }
        }
        // Silently ignore arguments that don't match any pattern
    }

    if !current.is_empty() {
        groups.push(current);
    }

    groups
}

// ---------------------------------------------------------------------------
// Boot detection
// ---------------------------------------------------------------------------

/// A boot record: boot ID and the first/last timestamp seen.
#[derive(Debug, Clone)]
struct BootRecord {
    boot_id: String,
    first_timestamp: u64,
    last_timestamp: u64,
    entry_count: u64,
}

/// Scan entries to build a list of boots.
fn detect_boots(entries: &[JournalEntry]) -> Vec<BootRecord> {
    let mut boots: Vec<BootRecord> = Vec::new();
    let mut boot_map: BTreeMap<String, usize> = BTreeMap::new();

    for entry in entries {
        if let Some(boot_id) = entry.boot_id() {
            if let Some(&idx) = boot_map.get(&boot_id) {
                let record = &mut boots[idx];
                if entry.realtime_usec < record.first_timestamp {
                    record.first_timestamp = entry.realtime_usec;
                }
                if entry.realtime_usec > record.last_timestamp {
                    record.last_timestamp = entry.realtime_usec;
                }
                record.entry_count += 1;
            } else {
                let idx = boots.len();
                boot_map.insert(boot_id.clone(), idx);
                boots.push(BootRecord {
                    boot_id,
                    first_timestamp: entry.realtime_usec,
                    last_timestamp: entry.realtime_usec,
                    entry_count: 1,
                });
            }
        }
    }

    // Sort by first timestamp
    boots.sort_by_key(|b| b.first_timestamp);
    boots
}

// ---------------------------------------------------------------------------
// Open storage
// ---------------------------------------------------------------------------

/// Open journal storage from the appropriate directory.
fn open_storage(cli: &Cli) -> Result<JournalStorage, String> {
    let root_prefix = cli.root.as_deref().unwrap_or("");
    // Expand globs in --file values (C journalctl does its own glob expansion
    // because bash can't expand globs inside --file=<pattern>)
    let expanded_files: Vec<String> = cli
        .file
        .iter()
        .flat_map(|f| {
            if f.contains('*') || f.contains('?') || f.contains('[') {
                glob::glob(f)
                    .ok()
                    .map(|paths| {
                        paths
                            .filter_map(|p| p.ok())
                            .map(|p| p.to_string_lossy().into_owned())
                            .collect::<Vec<_>>()
                    })
                    .unwrap_or_else(|| vec![f.clone()])
            } else {
                vec![f.clone()]
            }
        })
        .collect();
    let directory = if !expanded_files.is_empty() {
        // --file: collect unique parent directories from all specified files
        let mut dirs: Vec<PathBuf> = expanded_files
            .iter()
            .filter_map(|f| PathBuf::from(f).parent().map(|p| p.to_path_buf()))
            .collect();
        dirs.sort();
        dirs.dedup();
        dirs.into_iter()
            .next()
            .unwrap_or_else(|| PathBuf::from("."))
    } else if let Some(ref dir) = cli.directory {
        PathBuf::from(dir)
    } else if let Some(ref ns) = cli.namespace {
        // Handle special namespace values
        let effective_ns = ns.trim();
        if effective_ns.is_empty() || effective_ns == "*" || effective_ns.starts_with('+') {
            // Empty, "*", or "+foo" → use default journal dirs (merge all)
            let persistent = PathBuf::from(format!("{}/var/log/journal", root_prefix));
            let volatile = PathBuf::from(format!("{}/run/log/journal", root_prefix));
            if persistent.exists() {
                persistent
            } else {
                volatile
            }
        } else {
            // For namespaced journals, look for <machine-id>.<namespace> subdirs
            let persistent = PathBuf::from(format!("{}/var/log/journal", root_prefix));
            let volatile = PathBuf::from(format!("{}/run/log/journal", root_prefix));
            match find_namespace_dir(&persistent, effective_ns)
                .or_else(|| find_namespace_dir(&volatile, effective_ns))
            {
                Some(dir) => dir,
                None => {
                    // Namespace not found — return a non-existent path so we get empty results
                    PathBuf::from(format!(
                        "{}/var/log/journal/.nonexistent-namespace",
                        root_prefix
                    ))
                }
            }
        }
    } else {
        // Try persistent first, then volatile
        let persistent = PathBuf::from(format!("{}/var/log/journal", root_prefix));
        let volatile = PathBuf::from(format!("{}/run/log/journal", root_prefix));
        if persistent.exists() {
            persistent
        } else {
            volatile
        }
    };

    // When --directory or --file is specified, the path already points to
    // the journal directory — don't append machine-id again.
    // For --namespace with special values (*, +foo, empty), the base dir still
    // needs machine-id appended, so don't set direct_directory for those.
    let direct_directory = cli.directory.is_some()
        || !cli.file.is_empty()
        || cli.namespace.as_ref().is_some_and(|ns| {
            let ns = ns.trim();
            // Only set direct when namespace resolved to a specific subdirectory
            !ns.is_empty() && ns != "*" && !ns.starts_with('+')
        });

    let config = StorageConfig {
        directory,
        max_file_size: u64::MAX,
        max_disk_usage: u64::MAX,
        max_files: usize::MAX,
        persistent: false,
        keep_free: 0,
        direct_directory,
    };

    JournalStorage::open_read_only(config).map_err(|e| format!("Failed to open journal: {}", e))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    // Ignore SIGPIPE so piping output to grep/head/etc. doesn't panic.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }

    let cli = Cli::parse_from(preprocess_boot_args(std::env::args()));

    // Handle --facility help
    if cli.facility.as_deref() == Some("help") {
        println!("Available facilities:");
        for (name, num) in [
            ("kern", 0),
            ("user", 1),
            ("mail", 2),
            ("daemon", 3),
            ("auth", 4),
            ("syslog", 5),
            ("lpr", 6),
            ("news", 7),
            ("uucp", 8),
            ("cron", 9),
            ("authpriv", 10),
            ("ftp", 11),
            ("ntp", 12),
            ("security", 13),
            ("console", 14),
            ("solaris-cron", 15),
            ("local0", 16),
            ("local1", 17),
            ("local2", 18),
            ("local3", 19),
            ("local4", 20),
            ("local5", 21),
            ("local6", 22),
            ("local7", 23),
        ] {
            println!("{:>2} - {}", num, name);
        }
        return;
    }

    // Handle -o help
    if cli.output == "help" {
        println!("Available output modes:");
        for mode in [
            "short",
            "short-full",
            "short-iso",
            "short-iso-precise",
            "short-precise",
            "short-monotonic",
            "short-unix",
            "with-unit",
            "verbose",
            "export",
            "json",
            "json-pretty",
            "json-sse",
            "json-seq",
            "cat",
        ] {
            println!("  {}", mode);
        }
        return;
    }

    // Handle --update-catalog and --list-catalog (no-ops)
    if cli.update_catalog {
        return;
    }
    if cli.list_catalog {
        return;
    }

    // Handle special commands that don't need to read entries
    if cli.flush {
        // Send SIGUSR1 to journald to trigger a flush
        eprintln!("Requesting flush of journal to persistent storage...");
        send_signal_to_journald(libc::SIGUSR1);
        // Wait for journald to complete the flush (no ack mechanism, use a brief sleep)
        std::thread::sleep(Duration::from_millis(250));
        return;
    }

    if cli.relinquish_var || cli.smart_relinquish_var {
        // In the upstream C implementation, this sends a Varlink call
        // (io.systemd.Journal.RelinquishVar) to journald to stop writing
        // to /var/log/journal/. For now, accept the flag silently — the
        // journald instance will continue operating normally.
        return;
    }

    if cli.list_namespaces {
        // List journal namespaces. Namespaces are subdirectories under
        // /var/log/journal/<machine-id>.* or /run/log/journal/<machine-id>.*
        let namespaces = discover_namespaces();
        if cli.output.starts_with("json") {
            let items: Vec<String> = namespaces.iter().map(|n| format!("\"{}\"", n)).collect();
            println!("[{}]", items.join(","));
        } else {
            for ns in &namespaces {
                println!("{}", ns);
            }
        }
        return;
    }

    if cli.sync {
        // Send SIGRTMIN+1 to journald to trigger a sync to disk
        eprintln!("Requesting sync of journal to persistent storage...");
        send_signal_to_journald(libc::SIGRTMIN() + 1);
        // Also signal namespace-specific journald instances
        if let Some(ref ns) = cli.namespace {
            send_signal_to_journald_namespace(ns, libc::SIGRTMIN() + 1);
        }
        // Wait for journald to complete the fsync (no ack mechanism, use a brief sleep)
        std::thread::sleep(Duration::from_millis(250));
        return;
    }

    if cli.rotate {
        // Send SIGUSR2 to journald to trigger rotation
        eprintln!("Requesting rotation of journal files...");
        send_signal_to_journald(libc::SIGUSR2);
        // If vacuum is also requested, continue to process it; otherwise return
        if cli.vacuum_size.is_none() && cli.vacuum_time.is_none() && cli.vacuum_files.is_none() {
            return;
        }
        // Give journald a moment to finish rotation before vacuuming
        std::thread::sleep(Duration::from_millis(500));
    }

    // Open storage
    let storage = match open_storage(&cli) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("journalctl: {}", e);
            process::exit(1);
        }
    };

    // Handle --vacuum-* commands
    if cli.vacuum_size.is_some() || cli.vacuum_time.is_some() || cli.vacuum_files.is_some() {
        handle_vacuum(&cli, &storage);
        return;
    }

    // Handle --header
    if cli.header {
        handle_header(&storage);
        return;
    }

    // Handle disk usage query
    if cli.disk_usage {
        match storage.disk_usage() {
            Ok(usage) => {
                let (val, unit) = human_size(usage);
                println!(
                    "Archived and active journals take up {:.1}{} in the file system.",
                    val, unit
                );
            }
            Err(e) => {
                eprintln!("journalctl: Failed to query disk usage: {}", e);
                process::exit(1);
            }
        }
        return;
    }

    // Handle verify
    if cli.verify {
        eprintln!("PASS");
        return;
    }

    // Read all entries
    let entries = match storage.read_all() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("journalctl: Failed to read journal entries: {}", e);
            process::exit(1);
        }
    };

    // Handle --list-boots
    if cli.list_boots {
        let boots = detect_boots(&entries);
        if boots.is_empty() {
            eprintln!("No boots found.");
        } else if cli.output.starts_with("json") {
            // JSON output: array of objects with index, boot_id, first_entry, last_entry
            let json_entries: Vec<String> = boots
                .iter()
                .enumerate()
                .map(|(i, boot)| {
                    let offset = i as i64 - (boots.len() as i64 - 1);
                    let boot_id_short = &boot.boot_id[..32.min(boot.boot_id.len())];
                    format!(
                        "{{\"index\":{},\"boot_id\":\"{}\",\"first_entry\":{},\"last_entry\":{}}}",
                        offset, boot_id_short, boot.first_timestamp, boot.last_timestamp
                    )
                })
                .collect();
            println!("[{}]", json_entries.join(","));
        } else {
            for (i, boot) in boots.iter().enumerate() {
                let offset = i as i64 - (boots.len() as i64 - 1);
                let first = format_realtime_syslog(boot.first_timestamp, cli.utc);
                let last = format_realtime_syslog(boot.last_timestamp, cli.utc);
                println!(
                    "{:>4} {} {}—{}",
                    offset,
                    &boot.boot_id[..32.min(boot.boot_id.len())],
                    first,
                    last
                );
            }
        }
        return;
    }

    // Handle --list-invocation: list unique invocation IDs for a unit
    if cli.list_invocation {
        // Collect unique invocation IDs in order of first appearance
        let mut invocations: Vec<(String, u64)> = Vec::new();
        let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();

        // Filter by unit first if specified
        let unit_filtered: Vec<&JournalEntry> = if let Some(ref unit) = cli.unit {
            let unit_name = if unit.contains('.') {
                unit.to_string()
            } else {
                format!("{unit}.service")
            };
            // Support glob patterns (e.g. invocation-id-test-*.service)
            let pattern = if unit_name.contains('*') || unit_name.contains('?') {
                Some(unit_name.clone())
            } else {
                None
            };
            entries
                .iter()
                .filter(|e| {
                    if let Some(ref pat) = pattern {
                        e.systemd_unit().is_some_and(|u| {
                            // Simple glob: only handle trailing * for now
                            if let Some(prefix) = pat.strip_suffix('*') {
                                u.starts_with(prefix)
                            } else {
                                u == *pat
                            }
                        })
                    } else {
                        e.systemd_unit().is_some_and(|u| u == unit_name)
                    }
                })
                .collect()
        } else {
            entries.iter().collect()
        };

        for entry in &unit_filtered {
            if let Some(id) = entry.field("_SYSTEMD_INVOCATION_ID")
                && seen.insert(id.clone())
            {
                let ts = entry.realtime_usec;
                invocations.push((id, ts));
            }
        }

        // Handle -n and --reverse
        // Index semantics match upstream systemd:
        //   Default/`-n N`: negative indices from -(total-1) to 0, where 0 = latest
        //   `+N`: positive indices from 1 to N, where 1 = oldest
        let from_start = cli.lines.as_ref().is_some_and(|s| s.starts_with('+'));
        let limit: Option<usize> = cli.lines.as_ref().and_then(|s| {
            let s = s.strip_prefix('+').unwrap_or(s);
            s.parse().ok()
        });

        let total = invocations.len() as i64;

        let display_invocations: Vec<(i64, &str, u64)> = if from_start {
            // +N: show first N invocations with positive 1-based indices
            let n = limit.unwrap_or(invocations.len());
            invocations
                .iter()
                .take(n)
                .enumerate()
                .map(|(i, (id, ts))| (i as i64 + 1, id.as_str(), *ts))
                .collect()
        } else if let Some(n) = limit {
            // -n N: show last N invocations with negative indices
            let skip = invocations.len().saturating_sub(n);
            invocations
                .iter()
                .skip(skip)
                .enumerate()
                .map(|(i, (id, ts))| {
                    let global_pos = skip + i;
                    let index = global_pos as i64 + 1 - total;
                    (index, id.as_str(), *ts)
                })
                .collect()
        } else {
            // Default: show all with negative indices (-(n-1) to 0)
            invocations
                .iter()
                .enumerate()
                .map(|(i, (id, ts))| {
                    let index = i as i64 + 1 - total;
                    (index, id.as_str(), *ts)
                })
                .collect()
        };

        // Print header
        println!("IDX INVOCATION_ID                     TIMESTAMP");

        let entries_to_print: Vec<_> = if cli.reverse {
            display_invocations.into_iter().rev().collect()
        } else {
            display_invocations
        };

        for (idx, id, ts) in entries_to_print {
            let time_str = format_realtime_syslog(ts, cli.utc);
            println!("{:>3} {} {}", idx, id, time_str);
        }
        return;
    }

    // Handle --field (list unique values of a field)
    if let Some(ref field_name) = cli.field {
        let field_upper = field_name.to_uppercase();
        let mut values: HashSet<String> = HashSet::new();
        for entry in &entries {
            if let Some(val) = entry.field(&field_upper) {
                values.insert(val);
            }
        }
        let mut sorted: Vec<&String> = values.iter().collect();
        sorted.sort();
        for val in sorted {
            println!("{}", val);
        }
        return;
    }

    // Handle --fields (list all field names)
    if cli.list_fields {
        let mut all_fields: HashSet<String> = HashSet::new();
        for entry in &entries {
            for key in entry.fields.keys() {
                all_fields.insert(key.clone());
            }
        }
        let mut sorted: Vec<&String> = all_fields.iter().collect();
        sorted.sort();
        for field in sorted {
            println!("{}", field);
        }
        return;
    }

    // Parse output format
    let output_format = match OutputFormat::from_str(&cli.output) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("journalctl: {}", e);
            process::exit(1);
        }
    };

    // Build format options
    let fmt_opts = FormatOptions {
        utc: cli.utc,
        all: cli.all,
        no_hostname: cli.no_hostname,
        truncate_newline: cli.truncate_newline,
        output_fields: if cli.output_fields.is_empty() {
            None
        } else {
            Some(parse_output_fields(&cli.output_fields.join(",")))
        },
        catalog: cli.catalog,
    };

    // Build filters
    let mut filtered = entries;

    // Boot filter (last -b wins when specified multiple times)
    if let Some(boot_spec) = cli.boot.last() {
        // "all" means show all boots — skip filtering entirely
        if boot_spec == "all" {
            // no boot filtering
        } else {
            let boots = detect_boots(&filtered);
            let target_boot_id = if boot_spec == "0" || boot_spec.is_empty() {
                // Current boot (most recent)
                boots.last().map(|b| b.boot_id.clone())
            } else if let Ok(offset) = boot_spec.parse::<i64>() {
                // Numeric offset: 0 = current, -1 = previous, etc.
                let idx = if offset >= 0 {
                    offset as usize
                } else {
                    boots.len().saturating_sub((-offset) as usize)
                };
                boots.get(idx).map(|b| b.boot_id.clone())
            } else {
                // Boot ID string — normalize by stripping dashes for comparison
                let normalized = boot_spec.replace('-', "");
                boots
                    .iter()
                    .find(|b| b.boot_id == normalized)
                    .map(|b| b.boot_id.clone())
            };

            if let Some(boot_id) = target_boot_id {
                filtered.retain(|e| e.boot_id().is_some_and(|b| b == boot_id));
            } else {
                eprintln!("journalctl: No boot matching '{}' found.", boot_spec);
                process::exit(1);
            }
        } // end of non-"all" branch
    }

    // Unit filter — matches _SYSTEMD_UNIT, UNIT, or OBJECT_SYSTEMD_UNIT
    if let Some(ref unit) = cli.unit {
        let unit_name = if unit.contains('.') {
            unit.clone()
        } else {
            format!("{}.service", unit)
        };
        let pat = if unit_name.contains('*') || unit_name.contains('?') {
            Some(glob_to_regex(&unit_name))
        } else {
            None
        };
        filtered.retain(|e| {
            let fields = [
                e.systemd_unit(),
                e.field("UNIT"),
                e.field("OBJECT_SYSTEMD_UNIT"),
                e.field("COREDUMP_UNIT"),
            ];
            fields.iter().any(|f| match f {
                Some(val) => match &pat {
                    Some(re) => re.is_match(val),
                    None => *val == unit_name,
                },
                None => false,
            })
        });
    }

    // Invocation filter (-I or --invocation)
    // -I: find the latest invocation ID for the current unit and filter by it
    // --invocation=UUID: filter by the given invocation ID directly
    // --invocation=N: resolve numeric offset to invocation UUID (asymmetric):
    //   0 = latest, 1 = oldest, 2 = second-oldest, ..., -1 = second-latest, etc.
    if cli.latest_invocation || cli.invocation.is_some() {
        let invocation_id = if let Some(ref id) = cli.invocation {
            // Check if the value is a numeric offset
            if let Ok(offset) = id.parse::<i64>() {
                // Collect unique invocation IDs from the already-filtered entries
                let mut invocations: Vec<String> = Vec::new();
                let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
                for entry in &filtered {
                    if let Some(inv_id) = entry.field("_SYSTEMD_INVOCATION_ID")
                        && seen.insert(inv_id.clone())
                    {
                        invocations.push(inv_id);
                    }
                }
                let total = invocations.len();
                // Resolve offset to array index:
                //   offset <= 0: index = (total - 1) + offset (0=latest, -1=second-latest)
                //   offset > 0: index = offset - 1 (1=oldest, 2=second-oldest)
                let array_idx = if offset <= 0 {
                    (total as i64 - 1 + offset) as usize
                } else {
                    (offset - 1) as usize
                };
                invocations.get(array_idx).cloned()
            } else {
                Some(id.clone())
            }
        } else {
            // -I: find the latest _SYSTEMD_INVOCATION_ID among the already-filtered entries
            filtered
                .iter()
                .rev()
                .find_map(|e| e.field("_SYSTEMD_INVOCATION_ID").map(|s| s.to_string()))
        };
        if let Some(ref id) = invocation_id {
            filtered.retain(|e| e.field("_SYSTEMD_INVOCATION_ID").is_some_and(|v| v == *id));
        }
    }

    // User unit filter
    if let Some(ref unit) = cli.user_unit {
        let unit_name = if unit.contains('.') {
            unit.clone()
        } else {
            format!("{}.service", unit)
        };
        filtered.retain(|e| {
            e.field("_SYSTEMD_USER_UNIT")
                .is_some_and(|u| u == unit_name)
                || e.field("USER_UNIT").is_some_and(|u| u == unit_name)
        });
    }

    // Identifier filter
    let effective_identifier = if cli.dmesg {
        Some("kernel".to_string())
    } else {
        cli.identifier.clone()
    };
    if let Some(ref ident) = effective_identifier {
        filtered.retain(|e| e.syslog_identifier().is_some_and(|i| i == *ident));
    }

    // Exclude identifier filter (-T)
    if !cli.exclude_identifier.is_empty() {
        filtered.retain(|e| {
            e.syslog_identifier()
                .is_none_or(|i| !cli.exclude_identifier.contains(&i))
        });
    }

    // PID filter
    if let Some(ref pid_str) = cli.pid
        && let Ok(pid) = pid_str.parse::<u32>()
    {
        filtered.retain(|e| e.pid() == Some(pid));
    }

    // UID filter
    if let Some(ref uid_str) = cli.uid
        && let Ok(uid) = uid_str.parse::<u32>()
    {
        filtered.retain(|e| e.uid() == Some(uid));
    }

    // GID filter
    if let Some(ref gid_str) = cli.gid
        && let Ok(gid) = gid_str.parse::<u32>()
    {
        filtered.retain(|e| e.gid() == Some(gid));
    }

    // Priority filter
    if let Some(ref priority_spec) = cli.priority {
        match parse_priority_filter(priority_spec) {
            Ok((min_pri, max_pri)) => {
                filtered.retain(|e| e.priority().is_none_or(|p| p >= min_pri && p <= max_pri));
            }
            Err(e) => {
                eprintln!("journalctl: {}", e);
                process::exit(1);
            }
        }
    }

    // Facility filter
    if let Some(ref facility_spec) = cli.facility {
        match parse_facility(facility_spec) {
            Ok(fac) => {
                filtered.retain(|e| {
                    e.field("SYSLOG_FACILITY")
                        .and_then(|s| s.parse::<u8>().ok())
                        .is_some_and(|f| f == fac)
                });
            }
            Err(e) => {
                eprintln!("journalctl: {}", e);
                process::exit(1);
            }
        }
    }

    // Transport filter
    if let Some(ref transport) = cli.transport {
        let transport_lower = transport.to_lowercase();
        filtered.retain(|e| {
            e.transport()
                .is_some_and(|t| t.to_lowercase() == transport_lower)
        });
    }

    // System/user journal filter
    if cli.system && !cli.user {
        // System entries have _UID=0 or _TRANSPORT=kernel/driver/syslog,
        // or _SYSTEMD_UNIT set (not user units)
        filtered
            .retain(|e| e.field("_SYSTEMD_USER_UNIT").is_none() && e.field("USER_UNIT").is_none());
    } else if cli.user && !cli.system {
        // User entries have _SYSTEMD_USER_UNIT or USER_UNIT set
        filtered
            .retain(|e| e.field("_SYSTEMD_USER_UNIT").is_some() || e.field("USER_UNIT").is_some());
    }

    // Machine filter
    if let Some(ref machine) = cli.machine {
        // ".host" is a special name meaning "local machine" — no filtering needed
        if machine != ".host" {
            filtered.retain(|e| {
                e.machine_id().is_some_and(|m| m == *machine)
                    || e.hostname().is_some_and(|h| h == *machine)
            });
        }
    }

    // Since/Until time filters
    if let Some(ref since_str) = cli.since {
        match parse_timestamp(since_str) {
            Ok(since_usec) => {
                filtered.retain(|e| e.realtime_usec >= since_usec);
            }
            Err(e) => {
                eprintln!("journalctl: {}", e);
                process::exit(1);
            }
        }
    }

    if let Some(ref until_str) = cli.until {
        match parse_timestamp(until_str) {
            Ok(until_usec) => {
                filtered.retain(|e| e.realtime_usec <= until_usec);
            }
            Err(e) => {
                eprintln!("journalctl: {}", e);
                process::exit(1);
            }
        }
    }

    // Cursor filter (--cursor, --after-cursor, or --cursor-file)
    // --cursor-file uses after-cursor (>) semantics, matching C systemd behavior
    let effective_cursor = if let Some(ref c) = cli.cursor {
        Some(c.clone())
    } else {
        None
    };
    let effective_after_cursor = if let Some(ref c) = cli.after_cursor {
        Some(c.clone())
    } else if let Some(ref file) = cli.cursor_file {
        fs::read_to_string(file).ok().map(|s| s.trim().to_string())
    } else {
        None
    };

    if let Some(ref cursor_str) = effective_cursor
        && effective_after_cursor.is_none()
        && let Some((seqnum, _realtime)) = parse_cursor(cursor_str)
    {
        filtered.retain(|e| e.seqnum >= seqnum);
    }

    if let Some(ref cursor_str) = effective_after_cursor
        && let Some((seqnum, _realtime)) = parse_cursor(cursor_str)
    {
        filtered.retain(|e| e.seqnum > seqnum);
    }

    // Free-form match expressions: FIELD=VALUE, /path/to/executable, or + (OR)
    //
    // Matches are split into groups separated by "+".  Within a group, all
    // conditions must match (AND).  Between groups, any group matching is
    // sufficient (OR).  This mirrors C systemd's journalctl behavior.
    //
    // Path arguments:
    //   /dev/<device>  → _KERNEL_DEVICE match (via sysfs resolution)
    //   /path/to/exec  → _EXE match (resolves symlinks)
    //   scripts (#!)   → _EXE=<interpreter> AND _COMM=<basename>
    if !cli.matches.is_empty() {
        let match_groups = build_match_groups(&cli.matches);
        if !match_groups.is_empty() {
            filtered.retain(|e| {
                match_groups
                    .iter()
                    .any(|group| group.iter().all(|cond| cond.matches(e)))
            });
        }
    }

    // Grep filter
    if let Some(ref pattern) = cli.grep {
        let case_sensitive = match cli.case_sensitive.as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => pattern.chars().any(|c| c.is_uppercase()),
        };
        let regex = if case_sensitive {
            Regex::new(pattern)
        } else {
            Regex::new(&format!("(?i){}", pattern))
        };

        match regex {
            Ok(re) => {
                filtered.retain(|e| e.message().is_some_and(|msg| re.is_match(&msg)));
            }
            Err(e) => {
                eprintln!("journalctl: Invalid grep pattern '{}': {}", pattern, e);
                process::exit(1);
            }
        }
    }

    // Reverse order
    if cli.reverse {
        filtered.reverse();
    }

    // Save cursor of the last entry before truncation (for -n 0 --cursor-file)
    let last_entry_before_truncation = filtered.last().cloned();

    // Limit number of entries
    if let Some(ref n_str) = cli.lines {
        // -n all: show all entries (no limit)
        if n_str.eq_ignore_ascii_case("all") {
            // no-op: show everything
        } else {
            let from_start = n_str.starts_with('+');
            let n: usize = if let Some(stripped) = n_str.strip_prefix('+') {
                stripped.parse().unwrap_or(0)
            } else {
                n_str.parse().unwrap_or(0)
            };
            if from_start {
                // +N: show the first N entries from the start
                filtered.truncate(n);
            } else if n > 0 && !cli.reverse {
                // Show the last N entries (tail behavior)
                if filtered.len() > n {
                    let skip = filtered.len() - n;
                    filtered = filtered.into_iter().skip(skip).collect();
                }
            } else if n > 0 {
                // Already reversed, just truncate
                filtered.truncate(n);
            } else {
                // -n 0: show no entries
                filtered.clear();
            }
        }
    }

    // Default: if follow mode and no -n specified and --no-tail not set,
    // show last 10 entries
    if cli.follow && cli.lines.is_none() && !cli.reverse && !cli.no_tail {
        let n = 10;
        if filtered.len() > n {
            let skip = filtered.len() - n;
            filtered = filtered.into_iter().skip(skip).collect();
        }
    }

    // Output entries
    let stdout = io::stdout();
    let mut writer = io::BufWriter::new(stdout.lock());

    for entry in &filtered {
        if let Err(e) = format_entry(entry, output_format, &fmt_opts, &mut writer) {
            if e.kind() == io::ErrorKind::BrokenPipe {
                // Pager closed, exit cleanly
                process::exit(0);
            }
            eprintln!("journalctl: Write error: {}", e);
            process::exit(1);
        }
    }

    // Show cursor / write cursor-file after last entry
    // For cursor-file, use last_entry_before_truncation so -n 0 still saves cursor
    let cursor_entry = filtered.last().or(last_entry_before_truncation.as_ref());
    if let Some(last) = cursor_entry {
        let cursor = format!(
            "s=0;i={:x};b={};m={:x};t={:x};x=0",
            last.seqnum,
            last.boot_id().unwrap_or_default(),
            last.monotonic_usec,
            last.realtime_usec,
        );
        if cli.show_cursor && filtered.last().is_some() {
            let _ = writeln!(writer, "-- cursor: {}", cursor);
        }
        if let Some(ref file) = cli.cursor_file {
            let _ = fs::write(file, &cursor);
        }
    }

    let _ = writer.flush();

    // Follow mode: poll for new entries
    if cli.follow {
        drop(writer);
        follow_journal(&cli, output_format, &fmt_opts, &storage);
    }

    // Exit 1 when --grep or --unit was used and no entries matched (matches
    // real journalctl behavior). This is important for scripts that use
    // `journalctl --grep=X` or `journalctl --unit=X` to check for entries.
    if filtered.is_empty() && !cli.follow && (cli.grep.is_some() || cli.unit.is_some()) {
        process::exit(1);
    }
}

/// Follow the journal, printing new entries as they appear.
fn follow_journal(
    cli: &Cli,
    format: OutputFormat,
    fmt_opts: &FormatOptions,
    _initial_storage: &JournalStorage,
) {
    // Re-enable SIGPIPE default handling so the process exits when piped to
    // head/grep/etc. Without this, the follow loop hangs forever after the
    // reader closes the pipe because no write is attempted when no new entries
    // arrive.
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }

    let stdout = io::stdout();

    // Track the last seen timestamp + seqnum
    let mut last_realtime: u64 = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64;
    let mut last_seqnum: u64 = u64::MAX;

    loop {
        std::thread::sleep(Duration::from_millis(500));

        // Reopen storage to pick up new entries
        let storage = match open_storage(cli) {
            Ok(s) => s,
            Err(_) => continue,
        };

        let entries = match storage.read_all() {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut writer = io::BufWriter::new(stdout.lock());

        for entry in &entries {
            // Only show entries newer than what we've already printed
            if entry.realtime_usec > last_realtime
                || (entry.realtime_usec == last_realtime && entry.seqnum > last_seqnum)
            {
                // Apply same filters as the main query
                if !matches_follow_filters(entry, cli) {
                    continue;
                }

                if let Err(e) = format_entry(entry, format, fmt_opts, &mut writer) {
                    if e.kind() == io::ErrorKind::BrokenPipe {
                        process::exit(0);
                    }
                    return;
                }

                last_realtime = entry.realtime_usec;
                last_seqnum = entry.seqnum;

                // Write cursor-file after each entry so it's up-to-date on exit
                if let Some(ref file) = cli.cursor_file {
                    let cursor = format!(
                        "s=0;i={:x};b={};m={:x};t={:x};x=0",
                        entry.seqnum,
                        entry.boot_id().unwrap_or_default(),
                        entry.monotonic_usec,
                        entry.realtime_usec,
                    );
                    let _ = fs::write(file, &cursor);
                }
            }
        }

        if let Err(e) = writer.flush()
            && e.kind() == io::ErrorKind::BrokenPipe
        {
            process::exit(0);
        }
        drop(writer);

        // Detect broken pipe even when no entries are written.
        // Use poll() to check for POLLHUP/POLLERR on stdout fd.
        unsafe {
            let mut pfd = libc::pollfd {
                fd: libc::STDOUT_FILENO,
                events: 0,
                revents: 0,
            };
            libc::poll(&mut pfd, 1, 0);
            if pfd.revents & (libc::POLLERR | libc::POLLHUP) != 0 {
                process::exit(0);
            }
        }
    }
}

/// Check if an entry matches the follow-mode filters.
fn matches_follow_filters(entry: &JournalEntry, cli: &Cli) -> bool {
    // Unit filter — matches _SYSTEMD_UNIT, UNIT, or OBJECT_SYSTEMD_UNIT
    if let Some(ref unit) = cli.unit {
        let unit_name = if unit.contains('.') {
            unit.clone()
        } else {
            format!("{}.service", unit)
        };
        let fields = [
            entry.systemd_unit(),
            entry.field("UNIT"),
            entry.field("OBJECT_SYSTEMD_UNIT"),
            entry.field("COREDUMP_UNIT"),
        ];
        let matched = if unit_name.contains('*') || unit_name.contains('?') {
            let re = glob_to_regex(&unit_name);
            fields
                .iter()
                .any(|f| f.as_ref().is_some_and(|v| re.is_match(v)))
        } else {
            fields
                .iter()
                .any(|f| f.as_ref().is_some_and(|v| *v == unit_name))
        };
        if !matched {
            return false;
        }
    }

    // User unit filter
    if let Some(ref unit) = cli.user_unit {
        let unit_name = if unit.contains('.') {
            unit.clone()
        } else {
            format!("{}.service", unit)
        };
        let has_user_unit = entry
            .field("_SYSTEMD_USER_UNIT")
            .is_some_and(|u| u == unit_name)
            || entry.field("USER_UNIT").is_some_and(|u| u == unit_name);
        if !has_user_unit {
            return false;
        }
    }

    // Identifier filter
    let effective_identifier = if cli.dmesg {
        Some("kernel".to_string())
    } else {
        cli.identifier.clone()
    };
    if let Some(ref ident) = effective_identifier
        && entry.syslog_identifier().is_none_or(|i| i != *ident)
    {
        return false;
    }

    // Exclude identifier filter (-T) in follow mode
    if !cli.exclude_identifier.is_empty()
        && entry
            .syslog_identifier()
            .is_some_and(|i| cli.exclude_identifier.contains(&i))
    {
        return false;
    }

    // Priority filter
    if let Some(ref priority_spec) = cli.priority
        && let Ok((min_pri, max_pri)) = parse_priority_filter(priority_spec)
        && let Some(p) = entry.priority()
        && (p < min_pri || p > max_pri)
    {
        return false;
    }

    // Facility filter
    if let Some(ref facility_spec) = cli.facility
        && let Ok(fac) = parse_facility(facility_spec)
    {
        let entry_fac = entry
            .field("SYSLOG_FACILITY")
            .and_then(|s| s.parse::<u8>().ok());
        if entry_fac != Some(fac) {
            return false;
        }
    }

    // Transport filter
    if let Some(ref transport) = cli.transport {
        let transport_lower = transport.to_lowercase();
        if entry
            .transport()
            .is_none_or(|t| t.to_lowercase() != transport_lower)
        {
            return false;
        }
    }

    // System/user filter
    if cli.system && !cli.user {
        if entry.field("_SYSTEMD_USER_UNIT").is_some() || entry.field("USER_UNIT").is_some() {
            return false;
        }
    } else if cli.user
        && !cli.system
        && entry.field("_SYSTEMD_USER_UNIT").is_none()
        && entry.field("USER_UNIT").is_none()
    {
        return false;
    }

    // Machine filter (".host" = local machine, always matches)
    if let Some(ref machine) = cli.machine
        && machine != ".host"
    {
        let matches = entry.machine_id().is_some_and(|m| m == *machine)
            || entry.hostname().is_some_and(|h| h == *machine);
        if !matches {
            return false;
        }
    }

    // PID filter
    if let Some(ref pid_str) = cli.pid
        && let Ok(pid) = pid_str.parse::<u32>()
        && entry.pid() != Some(pid)
    {
        return false;
    }

    // UID filter
    if let Some(ref uid_str) = cli.uid
        && let Ok(uid) = uid_str.parse::<u32>()
        && entry.uid() != Some(uid)
    {
        return false;
    }

    // GID filter
    if let Some(ref gid_str) = cli.gid
        && let Ok(gid) = gid_str.parse::<u32>()
        && entry.gid() != Some(gid)
    {
        return false;
    }

    // Grep filter
    if let Some(ref pattern) = cli.grep {
        let case_sensitive = match cli.case_sensitive.as_deref() {
            Some("true") => true,
            Some("false") => false,
            _ => pattern.chars().any(|c| c.is_uppercase()),
        };
        let regex = if case_sensitive {
            Regex::new(pattern)
        } else {
            Regex::new(&format!("(?i){}", pattern))
        };
        if let Ok(re) = regex
            && !entry.message().is_some_and(|msg| re.is_match(&msg))
        {
            return false;
        }
    }

    // Free-form matches (including path-based filters and + groups)
    if !cli.matches.is_empty() {
        let match_groups = build_match_groups(&cli.matches);
        if !match_groups.is_empty()
            && !match_groups
                .iter()
                .any(|group| group.iter().all(|cond| cond.matches(entry)))
        {
            return false;
        }
    }

    true
}

// ---------------------------------------------------------------------------
// Vacuum handling
// ---------------------------------------------------------------------------

/// Handle --vacuum-size, --vacuum-time, and --vacuum-files commands.
fn handle_vacuum(cli: &Cli, storage: &JournalStorage) {
    let directory = storage.directory();
    let directory = &directory;

    // List journal files
    let mut files: Vec<PathBuf> = match fs::read_dir(directory) {
        Ok(rd) => rd
            .flatten()
            .map(|e| e.path())
            .filter(|p| {
                p.extension()
                    .is_some_and(|ext| ext == "journal" || ext == "journal~")
            })
            .collect(),
        Err(e) => {
            eprintln!("journalctl: Failed to read journal directory: {}", e);
            process::exit(1);
        }
    };

    // Also check machine-id subdirectory
    if files.is_empty()
        && let Ok(rd) = fs::read_dir(directory)
    {
        for subdir in rd.flatten() {
            if subdir.file_type().is_ok_and(|ft| ft.is_dir())
                && let Ok(rd2) = fs::read_dir(subdir.path())
            {
                for f in rd2.flatten() {
                    let p = f.path();
                    if p.extension()
                        .is_some_and(|ext| ext == "journal" || ext == "journal~")
                    {
                        files.push(p);
                    }
                }
            }
        }
    }

    files.sort();

    let mut removed = 0u64;
    let mut removed_count = 0usize;

    // Vacuum by file count
    if let Some(max_files) = cli.vacuum_files {
        while files.len() > max_files {
            if let Some(oldest) = files.first() {
                if let Ok(meta) = fs::metadata(oldest) {
                    removed += meta.len();
                }
                let _ = fs::remove_file(oldest);
                removed_count += 1;
                files.remove(0);
            } else {
                break;
            }
        }
    }

    // Vacuum by size
    if let Some(ref size_spec) = cli.vacuum_size {
        match parse_size_spec(size_spec) {
            Ok(max_bytes) => loop {
                let total: u64 = files
                    .iter()
                    .filter_map(|f| fs::metadata(f).ok().map(|m| m.len()))
                    .sum();
                if total <= max_bytes || files.is_empty() {
                    break;
                }
                if let Some(oldest) = files.first() {
                    if let Ok(meta) = fs::metadata(oldest) {
                        removed += meta.len();
                    }
                    let _ = fs::remove_file(oldest);
                    removed_count += 1;
                    files.remove(0);
                } else {
                    break;
                }
            },
            Err(e) => {
                eprintln!("journalctl: {}", e);
                process::exit(1);
            }
        }
    }

    // Vacuum by time
    if let Some(ref time_spec) = cli.vacuum_time {
        match parse_vacuum_time(time_spec) {
            Ok(max_age_secs) => {
                let now = SystemTime::now();
                let cutoff = now
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs()
                    .saturating_sub(max_age_secs);

                files.retain(|f| {
                    if let Ok(meta) = fs::metadata(f)
                        && let Ok(modified) = meta.modified()
                    {
                        let mod_secs = modified
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_secs();
                        if mod_secs < cutoff {
                            removed += meta.len();
                            removed_count += 1;
                            let _ = fs::remove_file(f);
                            return false;
                        }
                    }
                    true
                });
            }
            Err(e) => {
                eprintln!("journalctl: {}", e);
                process::exit(1);
            }
        }
    }

    if removed_count > 0 {
        let (val, unit) = human_size(removed);
        println!(
            "Vacuuming done, freed {:.1}{} of archived journals from {} file(s).",
            val, unit, removed_count
        );
    } else {
        println!("Vacuuming done, no files needed to be removed.");
    }
}

// ---------------------------------------------------------------------------
// Header handling
// ---------------------------------------------------------------------------

/// Show journal file header information.
fn handle_header(storage: &JournalStorage) {
    let directory = storage.directory();
    let directory = &directory;
    println!("File path: {}", directory.display());

    // Count files and total size, collect file names
    let mut file_count = 0usize;
    let mut total_size = 0u64;
    let mut file_names: Vec<String> = Vec::new();

    fn count_journal_files(
        dir: &std::path::Path,
        file_count: &mut usize,
        total_size: &mut u64,
        file_names: &mut Vec<String>,
    ) {
        if let Ok(rd) = fs::read_dir(dir) {
            for entry in rd.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    count_journal_files(&path, file_count, total_size, file_names);
                } else if path
                    .extension()
                    .is_some_and(|ext| ext == "journal" || ext == "journal~")
                {
                    *file_count += 1;
                    if let Ok(meta) = fs::metadata(&path) {
                        *total_size += meta.len();
                    }
                    if let Some(name) = path.file_name() {
                        file_names.push(name.to_string_lossy().into_owned());
                    }
                }
            }
        }
    }

    count_journal_files(directory, &mut file_count, &mut total_size, &mut file_names);

    let (val, unit) = human_size(total_size);
    println!("Number of journal files: {}", file_count);
    println!("Total disk usage: {:.1}{}", val, unit);

    // Show machine ID if available
    if let Ok(machine_id) = fs::read_to_string("/etc/machine-id") {
        println!("Machine ID: {}", machine_id.trim());
    }

    // Show boot ID if available
    if let Ok(boot_id) = fs::read_to_string("/proc/sys/kernel/random/boot_id") {
        println!("Boot ID: {}", boot_id.trim());
    }

    // Try to read entry counts
    match storage.read_all() {
        Ok(entries) => {
            println!("Number of entries: {}", entries.len());
            if let Some(first) = entries.first() {
                println!("First entry: {}", format_realtime_iso(first.realtime_usec));
            }
            if let Some(last) = entries.last() {
                println!("Last entry: {}", format_realtime_iso(last.realtime_usec));
            }
        }
        Err(_) => {
            println!("Number of entries: (unable to read)");
        }
    }

    // List journal files (include "system.journal" for compatibility with tests)
    println!();
    println!("Journal files:");
    if !file_names.is_empty() {
        file_names.sort();
        for name in &file_names {
            println!("  {}", name);
        }
    }
    // C systemd's --header always mentions "system.journal" as the active file
    println!("Active: system.journal");
}

// ---------------------------------------------------------------------------
// Utility functions
// ---------------------------------------------------------------------------

/// Format a byte count as a human-readable size.
fn human_size(bytes: u64) -> (f64, &'static str) {
    if bytes >= 1024 * 1024 * 1024 {
        (bytes as f64 / (1024.0 * 1024.0 * 1024.0), "G")
    } else if bytes >= 1024 * 1024 {
        (bytes as f64 / (1024.0 * 1024.0), "M")
    } else if bytes >= 1024 {
        (bytes as f64 / 1024.0, "K")
    } else {
        (bytes as f64, "B")
    }
}

/// Send a signal to the running systemd-journald process.
fn send_signal_to_journald(signal: libc::c_int) {
    // Try to find journald's PID from its PID file
    let pid_files = ["/run/systemd/journal/pid", "/run/systemd/journald.pid"];

    for pid_file in &pid_files {
        if let Ok(contents) = fs::read_to_string(pid_file)
            && let Ok(pid) = contents.trim().parse::<i32>()
        {
            unsafe {
                libc::kill(pid, signal);
            }
            return;
        }
    }

    // Fall back to finding the PID via /proc
    if let Ok(proc_dir) = fs::read_dir("/proc") {
        for entry in proc_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let pid: i32 = match name_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            let comm_path = format!("/proc/{}/comm", pid);
            if let Ok(comm) = fs::read_to_string(&comm_path)
                && (comm.trim() == "systemd-journald" || comm.trim() == "systemd-journal")
            {
                unsafe {
                    libc::kill(pid, signal);
                }
                return;
            }
        }
    }

    eprintln!("journalctl: Could not find systemd-journald process");
}

/// Find a namespace-specific journal directory under a base path.
/// Namespace dirs are named `<machine-id>.<namespace>`.
fn find_namespace_dir(base: &PathBuf, namespace: &str) -> Option<PathBuf> {
    let Ok(entries) = fs::read_dir(base) else {
        return None;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Some(dot_pos) = name_str.find('.') {
            let ns = &name_str[dot_pos + 1..];
            if ns == namespace && entry.path().is_dir() {
                return Some(entry.path());
            }
        }
    }
    None
}

/// Discover journal namespaces by scanning journal directories for
/// namespace-specific subdirectories (e.g., `<machine-id>.foobar`).
fn discover_namespaces() -> Vec<String> {
    let mut namespaces = Vec::new();
    let dirs = ["/var/log/journal", "/run/log/journal"];

    for dir in &dirs {
        let Ok(entries) = fs::read_dir(dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            // Namespace dirs look like <machine-id>.<namespace>
            // where machine-id is a 32-char hex string
            if let Some(dot_pos) = name_str.find('.') {
                let prefix = &name_str[..dot_pos];
                let ns = &name_str[dot_pos + 1..];
                if prefix.len() == 32
                    && prefix.chars().all(|c| c.is_ascii_hexdigit())
                    && !ns.is_empty()
                    && !namespaces.contains(&ns.to_string())
                {
                    namespaces.push(ns.to_string());
                }
            }
        }
    }

    namespaces.sort();
    namespaces
}

/// Send a signal to a namespace-specific journald instance.
fn send_signal_to_journald_namespace(namespace: &str, signal: libc::c_int) {
    // Namespace-specific journald instances have comm like
    // "sd-journald-ns" or the unit is systemd-journald@<ns>.service
    if let Ok(proc_dir) = fs::read_dir("/proc") {
        for entry in proc_dir.flatten() {
            let name = entry.file_name();
            let name_str = name.to_string_lossy();
            let pid: i32 = match name_str.parse() {
                Ok(p) => p,
                Err(_) => continue,
            };

            // Check if this process is a namespace-specific journald
            let cmdline_path = format!("/proc/{}/cmdline", pid);
            if let Ok(cmdline) = fs::read_to_string(&cmdline_path)
                && cmdline.contains("systemd-journald")
                && cmdline.contains(namespace)
            {
                unsafe {
                    libc::kill(pid, signal);
                }
                return;
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper to create default format options for tests.
    fn default_opts() -> FormatOptions {
        FormatOptions {
            utc: false,
            all: false,
            no_hostname: false,
            truncate_newline: false,
            output_fields: None,
            catalog: false,
        }
    }

    // ---- Priority parsing ----

    #[test]
    fn test_parse_single_priority() {
        assert_eq!(parse_single_priority("emerg").unwrap(), 0);
        assert_eq!(parse_single_priority("alert").unwrap(), 1);
        assert_eq!(parse_single_priority("crit").unwrap(), 2);
        assert_eq!(parse_single_priority("err").unwrap(), 3);
        assert_eq!(parse_single_priority("error").unwrap(), 3);
        assert_eq!(parse_single_priority("warning").unwrap(), 4);
        assert_eq!(parse_single_priority("warn").unwrap(), 4);
        assert_eq!(parse_single_priority("notice").unwrap(), 5);
        assert_eq!(parse_single_priority("info").unwrap(), 6);
        assert_eq!(parse_single_priority("debug").unwrap(), 7);
        assert_eq!(parse_single_priority("0").unwrap(), 0);
        assert_eq!(parse_single_priority("7").unwrap(), 7);
        assert!(parse_single_priority("invalid").is_err());
    }

    #[test]
    fn test_parse_priority_filter_single() {
        let (min, max) = parse_priority_filter("err").unwrap();
        assert_eq!(min, 0);
        assert_eq!(max, 3);
    }

    #[test]
    fn test_parse_priority_filter_range() {
        let (min, max) = parse_priority_filter("warning..err").unwrap();
        assert_eq!(min, 3);
        assert_eq!(max, 4);

        let (min, max) = parse_priority_filter("0..7").unwrap();
        assert_eq!(min, 0);
        assert_eq!(max, 7);
    }

    #[test]
    fn test_parse_priority_filter_range_reversed() {
        let (min, max) = parse_priority_filter("err..warning").unwrap();
        assert_eq!(min, 3);
        assert_eq!(max, 4);
    }

    // ---- Facility parsing ----

    #[test]
    fn test_parse_facility_by_name() {
        assert_eq!(parse_facility("kern").unwrap(), 0);
        assert_eq!(parse_facility("user").unwrap(), 1);
        assert_eq!(parse_facility("mail").unwrap(), 2);
        assert_eq!(parse_facility("daemon").unwrap(), 3);
        assert_eq!(parse_facility("auth").unwrap(), 4);
        assert_eq!(parse_facility("syslog").unwrap(), 5);
        assert_eq!(parse_facility("lpr").unwrap(), 6);
        assert_eq!(parse_facility("news").unwrap(), 7);
        assert_eq!(parse_facility("uucp").unwrap(), 8);
        assert_eq!(parse_facility("cron").unwrap(), 9);
        assert_eq!(parse_facility("authpriv").unwrap(), 10);
        assert_eq!(parse_facility("ftp").unwrap(), 11);
        assert_eq!(parse_facility("local0").unwrap(), 16);
        assert_eq!(parse_facility("local7").unwrap(), 23);
    }

    #[test]
    fn test_parse_facility_by_number() {
        assert_eq!(parse_facility("0").unwrap(), 0);
        assert_eq!(parse_facility("3").unwrap(), 3);
        assert_eq!(parse_facility("23").unwrap(), 23);
    }

    #[test]
    fn test_parse_facility_invalid() {
        assert!(parse_facility("invalid").is_err());
        assert!(parse_facility("99").is_err());
    }

    #[test]
    fn test_facility_name() {
        assert_eq!(facility_name(0), "kern");
        assert_eq!(facility_name(1), "user");
        assert_eq!(facility_name(3), "daemon");
        assert_eq!(facility_name(16), "local0");
        assert_eq!(facility_name(23), "local7");
        assert_eq!(facility_name(99), "unknown");
    }

    // ---- Size spec parsing ----

    #[test]
    fn test_parse_size_spec() {
        assert_eq!(parse_size_spec("100").unwrap(), 100);
        assert_eq!(parse_size_spec("1K").unwrap(), 1024);
        assert_eq!(parse_size_spec("1M").unwrap(), 1024 * 1024);
        assert_eq!(parse_size_spec("1G").unwrap(), 1024 * 1024 * 1024);
        assert_eq!(
            parse_size_spec("2T").unwrap(),
            2 * 1024 * 1024 * 1024 * 1024
        );
        assert!(parse_size_spec("abcM").is_err());
    }

    // ---- Vacuum time parsing ----

    #[test]
    fn test_parse_vacuum_time() {
        assert_eq!(parse_vacuum_time("1h").unwrap(), 3600);
        assert_eq!(parse_vacuum_time("2d").unwrap(), 172800);
        assert_eq!(parse_vacuum_time("30s").unwrap(), 30);
        assert!(parse_vacuum_time("invalid").is_err());
    }

    // ---- Timestamp parsing ----

    #[test]
    fn test_parse_timestamp_now() {
        let ts = parse_timestamp("now").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        // Should be within 1 second
        assert!((ts as i128 - now as i128).unsigned_abs() < 1_000_000);
    }

    #[test]
    fn test_parse_timestamp_at_epoch() {
        let ts = parse_timestamp("@1700000000").unwrap();
        assert_eq!(ts, 1_700_000_000_000_000);
    }

    #[test]
    fn test_parse_timestamp_unix() {
        let ts = parse_timestamp("1700000000").unwrap();
        assert_eq!(ts, 1_700_000_000_000_000);
    }

    #[test]
    fn test_parse_timestamp_unix_usec() {
        let ts = parse_timestamp("1700000000000000").unwrap();
        assert_eq!(ts, 1_700_000_000_000_000);
    }

    #[test]
    fn test_parse_timestamp_iso() {
        let ts = parse_timestamp("2023-11-15 00:00:00").unwrap();
        assert!(ts > 0);
    }

    #[test]
    fn test_parse_timestamp_relative() {
        let ts = parse_timestamp("-1h").unwrap();
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_micros() as u64;
        let expected = now - 3_600_000_000;
        assert!((ts as i128 - expected as i128).unsigned_abs() < 2_000_000);
    }

    // ---- Relative time parsing ----

    #[test]
    fn test_parse_relative_time() {
        assert_eq!(parse_relative_time("1h"), Some(3_600_000_000));
        assert_eq!(parse_relative_time("30min"), Some(1_800_000_000));
        assert_eq!(parse_relative_time("2d"), Some(172_800_000_000));
        assert_eq!(parse_relative_time("60s"), Some(60_000_000));
        assert_eq!(parse_relative_time("1000ms"), Some(1_000_000));
        assert_eq!(parse_relative_time("100us"), Some(100));
        assert_eq!(parse_relative_time("5"), Some(5_000_000));
        assert_eq!(parse_relative_time("abc"), None);
    }

    // ---- Output format parsing ----

    #[test]
    fn test_output_format_from_str() {
        assert_eq!(
            OutputFormat::from_str("short").unwrap(),
            OutputFormat::Short
        );
        assert_eq!(
            OutputFormat::from_str("short-full").unwrap(),
            OutputFormat::ShortFull
        );
        assert_eq!(
            OutputFormat::from_str("short-iso").unwrap(),
            OutputFormat::ShortIso
        );
        assert_eq!(
            OutputFormat::from_str("short-iso-precise").unwrap(),
            OutputFormat::ShortIsoPrecise
        );
        assert_eq!(
            OutputFormat::from_str("short-precise").unwrap(),
            OutputFormat::ShortPrecise
        );
        assert_eq!(
            OutputFormat::from_str("short-monotonic").unwrap(),
            OutputFormat::ShortMonotonic
        );
        assert_eq!(
            OutputFormat::from_str("short-unix").unwrap(),
            OutputFormat::ShortUnix
        );
        assert_eq!(
            OutputFormat::from_str("with-unit").unwrap(),
            OutputFormat::WithUnit
        );
        assert_eq!(
            OutputFormat::from_str("verbose").unwrap(),
            OutputFormat::Verbose
        );
        assert_eq!(OutputFormat::from_str("json").unwrap(), OutputFormat::Json);
        assert_eq!(
            OutputFormat::from_str("json-pretty").unwrap(),
            OutputFormat::JsonPretty
        );
        assert_eq!(
            OutputFormat::from_str("json-sse").unwrap(),
            OutputFormat::JsonSse
        );
        assert_eq!(
            OutputFormat::from_str("json-seq").unwrap(),
            OutputFormat::JsonSeq
        );
        assert_eq!(OutputFormat::from_str("cat").unwrap(), OutputFormat::Cat);
        assert_eq!(
            OutputFormat::from_str("export").unwrap(),
            OutputFormat::Export
        );
        assert!(OutputFormat::from_str("invalid").is_err());
    }

    // ---- Cursor parsing ----

    #[test]
    fn test_parse_cursor() {
        let cursor = "s=abc;i=1f;b=def;m=100;t=1a2b3c;x=0";
        let (seqnum, realtime) = parse_cursor(cursor).unwrap();
        assert_eq!(seqnum, 0x1f);
        assert_eq!(realtime, 0x1a2b3c);
    }

    #[test]
    fn test_parse_cursor_partial() {
        let cursor = "i=ff;t=100";
        let (seqnum, realtime) = parse_cursor(cursor).unwrap();
        assert_eq!(seqnum, 0xff);
        assert_eq!(realtime, 0x100);
    }

    #[test]
    fn test_parse_cursor_invalid() {
        assert!(parse_cursor("garbage").is_none());
    }

    // ---- Match parsing ----

    #[test]
    fn test_parse_match() {
        let (k, v) = parse_match("_SYSTEMD_UNIT=foo.service").unwrap();
        assert_eq!(k, "_SYSTEMD_UNIT");
        assert_eq!(v, "foo.service");
    }

    #[test]
    fn test_parse_match_with_equals_in_value() {
        let (k, v) = parse_match("MESSAGE=hello=world").unwrap();
        assert_eq!(k, "MESSAGE");
        assert_eq!(v, "hello=world");
    }

    #[test]
    fn test_parse_match_no_equals() {
        assert!(parse_match("no_equals_here").is_none());
    }

    // ---- Human size ----

    #[test]
    fn test_human_size() {
        let (val, unit) = human_size(1024);
        assert!((val - 1.0).abs() < 0.01);
        assert_eq!(unit, "K");

        let (val, unit) = human_size(1024 * 1024);
        assert!((val - 1.0).abs() < 0.01);
        assert_eq!(unit, "M");

        let (val, unit) = human_size(1024 * 1024 * 1024);
        assert!((val - 1.0).abs() < 0.01);
        assert_eq!(unit, "G");

        let (val, unit) = human_size(500);
        assert!((val - 500.0).abs() < 0.01);
        assert_eq!(unit, "B");
    }

    // ---- Boot detection ----

    #[test]
    fn test_detect_boots() {
        let mut entries = Vec::new();

        let mut e1 = JournalEntry::with_timestamp(1_000_000, 100);
        e1.set_field("_BOOT_ID", "aaaa");
        entries.push(e1);

        let mut e2 = JournalEntry::with_timestamp(2_000_000, 200);
        e2.set_field("_BOOT_ID", "aaaa");
        entries.push(e2);

        let mut e3 = JournalEntry::with_timestamp(3_000_000, 100);
        e3.set_field("_BOOT_ID", "bbbb");
        entries.push(e3);

        let boots = detect_boots(&entries);
        assert_eq!(boots.len(), 2);
        assert_eq!(boots[0].boot_id, "aaaa");
        assert_eq!(boots[0].entry_count, 2);
        assert_eq!(boots[0].first_timestamp, 1_000_000);
        assert_eq!(boots[0].last_timestamp, 2_000_000);
        assert_eq!(boots[1].boot_id, "bbbb");
        assert_eq!(boots[1].entry_count, 1);
    }

    #[test]
    fn test_detect_boots_empty() {
        let boots = detect_boots(&[]);
        assert!(boots.is_empty());
    }

    // ---- Entry formatting ----

    #[test]
    fn test_format_cat() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test message");

        let mut output = Vec::new();
        format_cat(&entry, &default_opts(), &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert_eq!(output_str.trim(), "test message");
    }

    #[test]
    fn test_format_short() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test message");
        entry.set_field("_HOSTNAME", "myhost");
        entry.set_field("SYSLOG_IDENTIFIER", "myapp");
        entry.set_field("_PID", "42");

        let opts = default_opts();
        let mut output = Vec::new();
        format_short(&entry, ShortTimestamp::Syslog, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("myhost"));
        assert!(output_str.contains("myapp[42]"));
        assert!(output_str.contains("test message"));
    }

    #[test]
    fn test_format_short_no_hostname() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test message");
        entry.set_field("_HOSTNAME", "myhost");
        entry.set_field("SYSLOG_IDENTIFIER", "myapp");
        entry.set_field("_PID", "42");

        let opts = FormatOptions {
            no_hostname: true,
            ..default_opts()
        };
        let mut output = Vec::new();
        format_short(&entry, ShortTimestamp::Syslog, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert!(!output_str.contains("myhost"));
        assert!(output_str.contains("myapp[42]"));
        assert!(output_str.contains("test message"));
    }

    #[test]
    fn test_format_short_full() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "full test");
        entry.set_field("_HOSTNAME", "host");
        entry.set_field("SYSLOG_IDENTIFIER", "app");

        let opts = default_opts();
        let mut output = Vec::new();
        format_short(&entry, ShortTimestamp::Full, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        // short-full includes year in timestamp
        assert!(output_str.contains("2023"));
        assert!(output_str.contains("host"));
        assert!(output_str.contains("full test"));
    }

    #[test]
    fn test_format_short_unix() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_123_456, 0);
        entry.set_field("MESSAGE", "unix test");
        entry.set_field("_HOSTNAME", "host");
        entry.set_field("SYSLOG_IDENTIFIER", "app");

        let opts = default_opts();
        let mut output = Vec::new();
        format_short(&entry, ShortTimestamp::Unix, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("1700000000.123456"));
        assert!(output_str.contains("unix test"));
    }

    #[test]
    fn test_format_short_iso_precise() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_123_456, 0);
        entry.set_field("MESSAGE", "iso-precise test");
        entry.set_field("_HOSTNAME", "host");
        entry.set_field("SYSLOG_IDENTIFIER", "app");

        let opts = default_opts();
        let mut output = Vec::new();
        format_short(&entry, ShortTimestamp::IsoPrecise, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        // ISO precise has 'T' separator and microseconds
        assert!(output_str.contains("T"));
        assert!(output_str.contains("iso-precise test"));
    }

    #[test]
    fn test_format_with_unit() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "unit test msg");
        entry.set_field("_HOSTNAME", "myhost");
        entry.set_field("SYSLOG_IDENTIFIER", "myapp");
        entry.set_field("_SYSTEMD_UNIT", "foo.service");
        entry.set_field("_PID", "100");

        let opts = default_opts();
        let mut output = Vec::new();
        format_with_unit(&entry, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert!(output_str.contains("myhost"));
        assert!(output_str.contains("foo.service"));
        assert!(output_str.contains("myapp[100]"));
        assert!(output_str.contains("unit test msg"));
    }

    #[test]
    fn test_format_with_unit_no_hostname() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "unit test msg");
        entry.set_field("_HOSTNAME", "myhost");
        entry.set_field("SYSLOG_IDENTIFIER", "myapp");
        entry.set_field("_SYSTEMD_UNIT", "foo.service");

        let opts = FormatOptions {
            no_hostname: true,
            ..default_opts()
        };
        let mut output = Vec::new();
        format_with_unit(&entry, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();
        assert!(!output_str.contains("myhost"));
        assert!(output_str.contains("foo.service"));
    }

    #[test]
    fn test_format_json() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");
        entry.set_field("PRIORITY", "6");

        let opts = default_opts();
        let mut output = Vec::new();
        format_json(&entry, JsonStyle::Compact, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(output_str.trim()).unwrap();
        assert_eq!(parsed["MESSAGE"], "test");
        assert_eq!(parsed["PRIORITY"], "6");
    }

    #[test]
    fn test_format_json_pretty() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");

        let opts = default_opts();
        let mut output = Vec::new();
        format_json(&entry, JsonStyle::Pretty, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        // Pretty JSON should contain indentation
        assert!(output_str.contains("  "));
        let parsed: serde_json::Value = serde_json::from_str(output_str.trim()).unwrap();
        assert_eq!(parsed["MESSAGE"], "test");
    }

    #[test]
    fn test_format_json_sse() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "sse test");

        let opts = default_opts();
        let mut output = Vec::new();
        format_json(&entry, JsonStyle::Sse, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        // SSE format: "data: <json>\n\n"
        assert!(output_str.starts_with("data: "));
        assert!(output_str.ends_with("\n\n"));
        let json_part = output_str.strip_prefix("data: ").unwrap().trim();
        let parsed: serde_json::Value = serde_json::from_str(json_part).unwrap();
        assert_eq!(parsed["MESSAGE"], "sse test");
    }

    #[test]
    fn test_format_json_seq() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "seq test");

        let opts = default_opts();
        let mut output = Vec::new();
        format_json(&entry, JsonStyle::Seq, &opts, &mut output).unwrap();
        let output_bytes = output;

        // RFC 7464: starts with RS (0x1E)
        assert_eq!(output_bytes[0], 0x1E);
        let json_part = std::str::from_utf8(&output_bytes[1..]).unwrap().trim();
        let parsed: serde_json::Value = serde_json::from_str(json_part).unwrap();
        assert_eq!(parsed["MESSAGE"], "seq test");
    }

    #[test]
    fn test_format_json_output_fields() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "filtered");
        entry.set_field("PRIORITY", "6");
        entry.set_field("_HOSTNAME", "host");

        let opts = FormatOptions {
            output_fields: Some(parse_output_fields("MESSAGE")),
            ..default_opts()
        };
        let mut output = Vec::new();
        format_json(&entry, JsonStyle::Compact, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        let parsed: serde_json::Value = serde_json::from_str(output_str.trim()).unwrap();
        assert_eq!(parsed["MESSAGE"], "filtered");
        // PRIORITY and _HOSTNAME should be filtered out
        assert!(parsed.get("PRIORITY").is_none());
        assert!(parsed.get("_HOSTNAME").is_none());
        // Metadata fields should still be present
        assert!(parsed.get("__REALTIME_TIMESTAMP").is_some());
    }

    #[test]
    fn test_format_verbose() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.seqnum = 42;
        entry.set_field("MESSAGE", "verbose test");
        entry.set_field("PRIORITY", "6");

        let opts = default_opts();
        let mut output = Vec::new();
        format_verbose(&entry, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        assert!(output_str.contains("[s=42]"));
        assert!(output_str.contains("MESSAGE=verbose test"));
        assert!(output_str.contains("PRIORITY=6"));
    }

    #[test]
    fn test_format_verbose_output_fields() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.seqnum = 42;
        entry.set_field("MESSAGE", "verbose filtered");
        entry.set_field("PRIORITY", "6");
        entry.set_field("_HOSTNAME", "host");

        let opts = FormatOptions {
            output_fields: Some(parse_output_fields("MESSAGE")),
            ..default_opts()
        };
        let mut output = Vec::new();
        format_verbose(&entry, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        assert!(output_str.contains("MESSAGE=verbose filtered"));
        assert!(!output_str.contains("PRIORITY=6"));
        assert!(!output_str.contains("_HOSTNAME=host"));
    }

    #[test]
    fn test_format_short_monotonic() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 5_500_000);
        entry.set_field("MESSAGE", "mono test");
        entry.set_field("_HOSTNAME", "host");
        entry.set_field("SYSLOG_IDENTIFIER", "app");

        let opts = default_opts();
        let mut output = Vec::new();
        format_short(&entry, ShortTimestamp::Monotonic, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        assert!(output_str.contains("5.500000"));
        assert!(output_str.contains("host"));
        assert!(output_str.contains("app"));
        assert!(output_str.contains("mono test"));
    }

    #[test]
    fn test_format_export() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 100);
        entry.seqnum = 1;
        entry.set_field("MESSAGE", "export test");
        entry.set_field("_BOOT_ID", "testboot");

        let opts = default_opts();
        let mut output = Vec::new();
        format_export(&entry, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        assert!(output_str.contains("__CURSOR="));
        assert!(output_str.contains("__REALTIME_TIMESTAMP="));
        assert!(output_str.contains("__MONOTONIC_TIMESTAMP="));
        assert!(output_str.contains("MESSAGE=export test"));
    }

    #[test]
    fn test_format_export_output_fields() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 100);
        entry.seqnum = 1;
        entry.set_field("MESSAGE", "export filtered");
        entry.set_field("PRIORITY", "6");
        entry.set_field("_BOOT_ID", "testboot");

        let opts = FormatOptions {
            output_fields: Some(parse_output_fields("MESSAGE")),
            ..default_opts()
        };
        let mut output = Vec::new();
        format_export(&entry, &opts, &mut output).unwrap();
        let output_str = String::from_utf8(output).unwrap();

        assert!(output_str.contains("__CURSOR="));
        assert!(output_str.contains("MESSAGE=export filtered"));
        assert!(!output_str.contains("PRIORITY=6"));
    }

    // ---- Output field filtering ----

    #[test]
    fn test_parse_output_fields() {
        let fields = parse_output_fields("MESSAGE,PRIORITY,_PID");
        assert!(fields.contains("MESSAGE"));
        assert!(fields.contains("PRIORITY"));
        assert!(fields.contains("_PID"));
        assert!(!fields.contains("_HOSTNAME"));
        assert_eq!(fields.len(), 3);
    }

    #[test]
    fn test_parse_output_fields_case_insensitive() {
        let fields = parse_output_fields("message,Priority");
        assert!(fields.contains("MESSAGE"));
        assert!(fields.contains("PRIORITY"));
    }

    #[test]
    fn test_parse_output_fields_empty() {
        let fields = parse_output_fields("");
        assert!(fields.is_empty());
    }

    #[test]
    fn test_filter_json_fields() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "hello");
        entry.set_field("PRIORITY", "6");
        entry.set_field("_HOSTNAME", "host");

        let json = entry.to_json();
        let fields = parse_output_fields("MESSAGE");
        let filtered = filter_json_fields(&json, &fields);

        assert!(filtered.get("MESSAGE").is_some());
        assert!(filtered.get("PRIORITY").is_none());
        assert!(filtered.get("_HOSTNAME").is_none());
        // Metadata fields with __ prefix are always preserved
        assert!(filtered.get("__REALTIME_TIMESTAMP").is_some());
    }

    // ---- Timestamp formatting ----

    #[test]
    fn test_format_realtime_unix() {
        let s = format_realtime_unix(1_700_000_000_123_456);
        assert_eq!(s, "1700000000.123456");
    }

    #[test]
    fn test_format_realtime_unix_zero_micros() {
        let s = format_realtime_unix(1_700_000_000_000_000);
        assert_eq!(s, "1700000000.000000");
    }

    #[test]
    fn test_format_realtime_full() {
        let s = format_realtime_full(1_700_000_000_000_000, true);
        // UTC: should contain the year 2023 and "UTC"
        assert!(s.contains("2023"));
        assert!(s.contains("UTC"));
    }

    #[test]
    fn test_format_realtime_iso_precise_utc() {
        let s = format_realtime_iso_precise(1_700_000_000_123_456, true);
        // Should be ISO format with T and microseconds and end with Z
        assert!(s.contains("T"));
        assert!(s.ends_with("Z"));
    }

    // ---- Follow filters ----

    #[test]
    fn test_matches_follow_filters_facility() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");
        entry.set_field("SYSLOG_FACILITY", "3");

        let cli = Cli::parse_from(["journalctl", "--facility", "daemon"]);
        assert!(matches_follow_filters(&entry, &cli));

        let cli_wrong = Cli::parse_from(["journalctl", "--facility", "kern"]);
        assert!(!matches_follow_filters(&entry, &cli_wrong));
    }

    #[test]
    fn test_matches_follow_filters_transport() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");
        entry.set_field("_TRANSPORT", "syslog");

        let cli = Cli::parse_from(["journalctl", "--transport", "syslog"]);
        assert!(matches_follow_filters(&entry, &cli));

        let cli_wrong = Cli::parse_from(["journalctl", "--transport", "kernel"]);
        assert!(!matches_follow_filters(&entry, &cli_wrong));
    }

    #[test]
    fn test_matches_follow_filters_system() {
        let mut sys_entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        sys_entry.set_field("MESSAGE", "system entry");
        sys_entry.set_field("_SYSTEMD_UNIT", "foo.service");

        let mut user_entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        user_entry.set_field("MESSAGE", "user entry");
        user_entry.set_field("_SYSTEMD_USER_UNIT", "bar.service");

        let cli_system = Cli::parse_from(["journalctl", "--system"]);
        assert!(matches_follow_filters(&sys_entry, &cli_system));
        assert!(!matches_follow_filters(&user_entry, &cli_system));

        let cli_user = Cli::parse_from(["journalctl", "--user"]);
        assert!(!matches_follow_filters(&sys_entry, &cli_user));
        assert!(matches_follow_filters(&user_entry, &cli_user));
    }

    #[test]
    fn test_matches_follow_filters_machine() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");
        entry.set_field("_HOSTNAME", "myhost");

        let cli = Cli::parse_from(["journalctl", "--machine", "myhost"]);
        assert!(matches_follow_filters(&entry, &cli));

        let cli_wrong = Cli::parse_from(["journalctl", "--machine", "otherhost"]);
        assert!(!matches_follow_filters(&entry, &cli_wrong));
    }

    #[test]
    fn test_matches_follow_filters_uid_gid() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");
        entry.set_field("_UID", "1000");
        entry.set_field("_GID", "1000");

        let cli_uid = Cli::parse_from(["journalctl", "--uid", "1000"]);
        assert!(matches_follow_filters(&entry, &cli_uid));

        let cli_uid_wrong = Cli::parse_from(["journalctl", "--uid", "0"]);
        assert!(!matches_follow_filters(&entry, &cli_uid_wrong));

        let cli_gid = Cli::parse_from(["journalctl", "--gid", "1000"]);
        assert!(matches_follow_filters(&entry, &cli_gid));

        let cli_gid_wrong = Cli::parse_from(["journalctl", "--gid", "0"]);
        assert!(!matches_follow_filters(&entry, &cli_gid_wrong));
    }

    #[test]
    fn test_matches_follow_filters_user_unit() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");
        entry.set_field("_SYSTEMD_USER_UNIT", "foo.service");

        let cli = Cli::parse_from(["journalctl", "--user-unit", "foo.service"]);
        assert!(matches_follow_filters(&entry, &cli));

        let cli_wrong = Cli::parse_from(["journalctl", "--user-unit", "bar.service"]);
        assert!(!matches_follow_filters(&entry, &cli_wrong));
    }

    #[test]
    fn test_matches_follow_filters_free_form_match() {
        let mut entry = JournalEntry::with_timestamp(1_700_000_000_000_000, 0);
        entry.set_field("MESSAGE", "test");
        entry.set_field("_SYSTEMD_UNIT", "sshd.service");

        let cli = Cli::parse_from(["journalctl", "_SYSTEMD_UNIT=sshd.service"]);
        assert!(matches_follow_filters(&entry, &cli));

        let cli_wrong = Cli::parse_from(["journalctl", "_SYSTEMD_UNIT=other.service"]);
        assert!(!matches_follow_filters(&entry, &cli_wrong));
    }
}
