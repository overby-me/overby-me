//! coredumpctl — List, inspect, and retrieve stored core dumps.
//!
//! A drop-in replacement for `coredumpctl(1)`. This tool reads core dump
//! metadata from JSON sidecar files stored by `systemd-coredump` in
//! `/var/lib/systemd/coredump/` and presents them in various formats.
//!
//! Supported subcommands:
//!   - `list [MATCH...]`  — List stored core dumps in a table
//!   - `info [MATCH...]`  — Show detailed information about core dumps
//!   - `dump [MATCH...]`  — Write the core dump binary to stdout or a file
//!   - `debug [MATCH...]` — Launch a debugger on a core dump (alias: `gdb`)
//!
//! Match patterns:
//!   - Bare number       → match PID
//!   - Name without `/`  → match COMM (command name)
//!   - Path with `/`     → match EXE (executable path)
//!   - `*.service` etc.  → match unit name (reserved for future use)
//!
//! Exit codes:
//!   0 — success
//!   1 — error or no matching core dumps found

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;

use clap::{Parser, Subcommand};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const COREDUMP_DIR: &str = "/var/lib/systemd/coredump";

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser)]
#[command(
    name = "coredumpctl",
    about = "List and retrieve core dumps from the journal or /var/lib/systemd/coredump",
    long_about = "coredumpctl is a tool to browse, inspect, and retrieve core dumps \
                  stored by systemd-coredump."
)]
struct Cli {
    /// Show core dumps from a specific directory (default: /var/lib/systemd/coredump)
    #[arg(long = "directory", short = 'D', global = true)]
    directory: Option<PathBuf>,

    /// Show at most N entries (for list)
    #[arg(long, short = 'n', global = true)]
    lines: Option<usize>,

    /// Do not use a pager
    #[arg(long, global = true)]
    no_pager: bool,

    /// Do not show legend (column headers)
    #[arg(long, global = true)]
    no_legend: bool,

    /// Show results in reverse chronological order
    #[arg(long, short = 'r', global = true)]
    reverse: bool,

    /// Only show core dumps since this timestamp (UNIX epoch seconds)
    #[arg(long, global = true)]
    since: Option<u64>,

    /// Only show core dumps until this timestamp (UNIX epoch seconds)
    #[arg(long, global = true)]
    until: Option<u64>,

    /// Output file for dump subcommand
    #[arg(long, short = 'o', global = true)]
    output: Option<PathBuf>,

    /// Be more quiet
    #[arg(long, short = 'q', global = true)]
    quiet: bool,

    /// Debugger to use (default: gdb)
    #[arg(long, global = true)]
    debugger: Option<String>,

    /// Additional arguments for the debugger
    #[arg(long = "debugger-arguments", short = 'A', global = true)]
    debugger_arguments: Option<String>,

    #[command(subcommand)]
    command: Option<Commands>,
}

#[derive(Subcommand)]
enum Commands {
    /// List stored core dumps
    List {
        /// Match patterns (PID, COMM, EXE, or unit name)
        #[arg(trailing_var_arg = true)]
        matches: Vec<String>,
    },
    /// Show detailed information about stored core dumps
    Info {
        /// Match patterns
        #[arg(trailing_var_arg = true)]
        matches: Vec<String>,
    },
    /// Retrieve the core dump binary data
    Dump {
        /// Match patterns
        #[arg(trailing_var_arg = true)]
        matches: Vec<String>,
    },
    /// Launch a debugger on a core dump
    Debug {
        /// Match patterns
        #[arg(trailing_var_arg = true)]
        matches: Vec<String>,
    },
    /// Launch gdb on a core dump (alias for debug)
    Gdb {
        /// Match patterns
        #[arg(trailing_var_arg = true)]
        matches: Vec<String>,
    },
}

// ---------------------------------------------------------------------------
// Core dump metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct CoreDumpMeta {
    pid: u64,
    uid: u64,
    gid: u64,
    signal: u32,
    timestamp: u64,
    rlimit: u64,
    hostname: String,
    comm: String,
    exe: String,
    backtrace: bool,
    core_size: u64,
    filename: String,
    boot_id: String,
    machine_id: String,
}

impl CoreDumpMeta {
    fn signal_name(&self) -> &'static str {
        match self.signal {
            1 => "SIGHUP",
            2 => "SIGINT",
            3 => "SIGQUIT",
            4 => "SIGILL",
            5 => "SIGTRAP",
            6 => "SIGABRT",
            7 => "SIGBUS",
            8 => "SIGFPE",
            9 => "SIGKILL",
            11 => "SIGSEGV",
            13 => "SIGPIPE",
            14 => "SIGALRM",
            15 => "SIGTERM",
            _ => "unknown",
        }
    }

    fn from_json(s: &str) -> Option<Self> {
        let map = parse_json_object(s)?;

        Some(CoreDumpMeta {
            pid: map.get("PID")?.trim().parse().ok()?,
            uid: map.get("UID")?.trim().parse().ok()?,
            gid: map.get("GID")?.trim().parse().ok()?,
            signal: map.get("SIGNAL")?.trim().parse().ok()?,
            timestamp: map.get("TIMESTAMP")?.trim().parse().ok()?,
            rlimit: map
                .get("RLIMIT")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            hostname: map.get("HOSTNAME").cloned().unwrap_or_default(),
            comm: map.get("COMM").cloned().unwrap_or_default(),
            exe: map.get("EXE").cloned().unwrap_or_default(),
            backtrace: map.get("BACKTRACE").is_some_and(|v| v.trim() == "true"),
            core_size: map
                .get("CORE_SIZE")
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(0),
            filename: map.get("FILENAME").cloned().unwrap_or_default(),
            boot_id: map.get("BOOT_ID").cloned().unwrap_or_default(),
            machine_id: map.get("MACHINE_ID").cloned().unwrap_or_default(),
        })
    }
}

/// Minimal JSON object parser — returns key/value string pairs.
fn parse_json_object(s: &str) -> Option<BTreeMap<String, String>> {
    let s = s.trim();
    if !s.starts_with('{') || !s.ends_with('}') {
        return None;
    }

    let inner = &s[1..s.len() - 1];
    let mut map = BTreeMap::new();

    for line in inner.lines() {
        let line = line.trim().trim_end_matches(',');
        if line.is_empty() {
            continue;
        }

        if let Some((key_part, value_part)) = line.split_once(':') {
            let key = key_part.trim().trim_matches('"');
            let value = value_part.trim();

            let value = if value.starts_with('"') && value.ends_with('"') && value.len() >= 2 {
                json_unescape(&value[1..value.len() - 1])
            } else {
                value.to_owned()
            };

            map.insert(key.to_owned(), value);
        }
    }

    Some(map)
}

fn json_unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();

    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('\\') => out.push('\\'),
                Some('n') => out.push('\n'),
                Some('r') => out.push('\r'),
                Some('t') => out.push('\t'),
                Some('/') => out.push('/'),
                Some('u') => {
                    let hex: String = chars.by_ref().take(4).collect();
                    if let Ok(n) = u32::from_str_radix(&hex, 16)
                        && let Some(ch) = char::from_u32(n)
                    {
                        out.push(ch);
                    }
                }
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
                None => out.push('\\'),
            }
        } else {
            out.push(c);
        }
    }

    out
}

// ---------------------------------------------------------------------------
// Core dump discovery and filtering
// ---------------------------------------------------------------------------

#[allow(dead_code)]
struct CoreDumpEntry {
    meta: CoreDumpMeta,
    core_path: PathBuf,
    meta_path: PathBuf,
}

fn discover_coredumps(coredump_dir: &Path) -> Vec<CoreDumpEntry> {
    let mut entries = Vec::new();

    let dir_entries = match fs::read_dir(coredump_dir) {
        Ok(e) => e,
        Err(_) => return entries,
    };

    for entry in dir_entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(n) => n.to_owned(),
            None => continue,
        };

        if !name.ends_with(".json") || !name.starts_with("core.") {
            continue;
        }

        let json_content = match fs::read_to_string(&path) {
            Ok(c) => c,
            Err(_) => continue,
        };

        let meta = match CoreDumpMeta::from_json(&json_content) {
            Some(m) => m,
            None => continue,
        };

        let core_name = name.strip_suffix(".json").unwrap_or(&name);
        let core_path = coredump_dir.join(core_name);

        entries.push(CoreDumpEntry {
            meta,
            core_path,
            meta_path: path,
        });
    }

    // Sort by timestamp ascending (oldest first).
    entries.sort_by_key(|e| e.meta.timestamp);
    entries
}

/// Check if a core dump entry matches the given patterns.
///
/// Match semantics:
///   - Bare number: match PID
///   - Contains `/`: match executable path (prefix or exact)
///   - Otherwise: match command name (exact or prefix)
fn matches_filters(meta: &CoreDumpMeta, patterns: &[String]) -> bool {
    if patterns.is_empty() {
        return true;
    }

    for pattern in patterns {
        if let Ok(pid) = pattern.parse::<u64>()
            && meta.pid == pid
        {
            return true;
        }

        if pattern.contains('/') {
            // Match executable path.
            if meta.exe == *pattern || meta.exe.starts_with(pattern.as_str()) {
                return true;
            }
        } else {
            // Match command name.
            if meta.comm == *pattern || meta.comm.starts_with(pattern.as_str()) {
                return true;
            }
        }
    }

    false
}

/// Apply time-based filters (--since / --until).
fn matches_time_range(meta: &CoreDumpMeta, since: Option<u64>, until: Option<u64>) -> bool {
    if let Some(s) = since
        && meta.timestamp < s
    {
        return false;
    }
    if let Some(u) = until
        && meta.timestamp > u
    {
        return false;
    }
    true
}

// ---------------------------------------------------------------------------
// Formatting helpers
// ---------------------------------------------------------------------------

fn format_timestamp(epoch_secs: u64) -> String {
    // Convert epoch seconds to a human-readable UTC string.
    // Use a simple format: YYYY-MM-DD HH:MM:SS UTC
    let secs = epoch_secs;

    let days = secs / 86400;
    let time_of_day = secs % 86400;
    let hours = time_of_day / 3600;
    let minutes = (time_of_day % 3600) / 60;
    let seconds = time_of_day % 60;

    // Compute year/month/day from days since epoch (1970-01-01).
    let (year, month, day) = days_to_ymd(days);

    format!("{year:04}-{month:02}-{day:02} {hours:02}:{minutes:02}:{seconds:02} UTC")
}

fn days_to_ymd(mut days: u64) -> (u64, u64, u64) {
    // Algorithm: compute year, then month, then day from days since 1970-01-01.
    let mut year = 1970u64;

    loop {
        let days_in_year = if is_leap_year(year) { 366 } else { 365 };
        if days < days_in_year {
            break;
        }
        days -= days_in_year;
        year += 1;
    }

    let leap = is_leap_year(year);
    let month_days: [u64; 12] = [
        31,
        if leap { 29 } else { 28 },
        31,
        30,
        31,
        30,
        31,
        31,
        30,
        31,
        30,
        31,
    ];

    let mut month = 1u64;
    for &md in &month_days {
        if days < md {
            break;
        }
        days -= md;
        month += 1;
    }

    (year, month, days + 1)
}

fn is_leap_year(y: u64) -> bool {
    (y.is_multiple_of(4) && !y.is_multiple_of(100)) || y.is_multiple_of(400)
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

/// Resolve UID to a username via /etc/passwd lookup.
fn uid_to_name(uid: u64) -> String {
    if let Ok(contents) = fs::read_to_string("/etc/passwd") {
        for line in contents.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() >= 3
                && let Ok(file_uid) = fields[2].parse::<u64>()
                && file_uid == uid
            {
                return fields[0].to_owned();
            }
        }
    }
    uid.to_string()
}

/// Resolve GID to a group name via /etc/group lookup.
fn gid_to_name(gid: u64) -> String {
    if let Ok(contents) = fs::read_to_string("/etc/group") {
        for line in contents.lines() {
            let fields: Vec<&str> = line.split(':').collect();
            if fields.len() >= 3
                && let Ok(file_gid) = fields[2].parse::<u64>()
                && file_gid == gid
            {
                return fields[0].to_owned();
            }
        }
    }
    gid.to_string()
}

// ---------------------------------------------------------------------------
// Subcommand implementations
// ---------------------------------------------------------------------------

fn cmd_list(entries: &[CoreDumpEntry], patterns: &[String], cli: &Cli) -> i32 {
    let filtered: Vec<&CoreDumpEntry> = entries
        .iter()
        .filter(|e| matches_filters(&e.meta, patterns))
        .filter(|e| matches_time_range(&e.meta, cli.since, cli.until))
        .collect();

    if filtered.is_empty() {
        if !cli.quiet {
            eprintln!("No coredumps found.");
        }
        return 1;
    }

    let mut display: Vec<&CoreDumpEntry> = if cli.reverse {
        filtered.into_iter().rev().collect()
    } else {
        filtered
    };

    if let Some(n) = cli.lines
        && display.len() > n
    {
        let start = display.len() - n;
        display = display[start..].to_vec();
    }

    if !cli.no_legend {
        println!(
            "{:<19}  {:>7}  {:>7}  {:>6}  {:>9}  {:>12}  EXE/COMM",
            "TIME", "PID", "UID", "GID", "SIG", "COREFILE"
        );
    }

    for entry in &display {
        let m = &entry.meta;
        let time_str = format_timestamp(m.timestamp);
        // Truncate time string to 19 chars for compact display.
        let time_short = if time_str.len() > 19 {
            &time_str[..19]
        } else {
            &time_str
        };

        let corefile_status = if entry.core_path.exists() {
            format_bytes(m.core_size)
        } else if m.core_size > 0 {
            "missing".to_owned()
        } else {
            "none".to_owned()
        };

        let exe_or_comm = if m.exe.is_empty() { &m.comm } else { &m.exe };

        println!(
            "{:<19}  {:>7}  {:>7}  {:>6}  {:>9}  {:>12}  {}",
            time_short,
            m.pid,
            m.uid,
            m.gid,
            m.signal_name(),
            corefile_status,
            exe_or_comm,
        );
    }

    if !cli.no_legend {
        println!("\n{} entries listed.", display.len());
    }

    0
}

fn cmd_info(entries: &[CoreDumpEntry], patterns: &[String], cli: &Cli) -> i32 {
    let filtered: Vec<&CoreDumpEntry> = entries
        .iter()
        .filter(|e| matches_filters(&e.meta, patterns))
        .filter(|e| matches_time_range(&e.meta, cli.since, cli.until))
        .collect();

    if filtered.is_empty() {
        if !cli.quiet {
            eprintln!("No coredumps found.");
        }
        return 1;
    }

    // If no specific pattern given, show only the most recent.
    let to_show: Vec<&CoreDumpEntry> = if patterns.is_empty() {
        vec![filtered.last().unwrap()]
    } else {
        filtered
    };

    for (i, entry) in to_show.iter().enumerate() {
        if i > 0 {
            println!();
        }
        print_info(&entry.meta, &entry.core_path);
    }

    0
}

fn print_info(meta: &CoreDumpMeta, core_path: &Path) {
    let time_str = format_timestamp(meta.timestamp);
    let user = uid_to_name(meta.uid);
    let group = gid_to_name(meta.gid);

    println!("           PID: {} ({})", meta.pid, meta.comm);
    println!("           UID: {} ({})", meta.uid, user);
    println!("           GID: {} ({})", meta.gid, group);
    println!("        Signal: {} ({})", meta.signal, meta.signal_name());
    println!("     Timestamp: {time_str}");

    if !meta.hostname.is_empty() {
        println!("      Hostname: {}", meta.hostname);
    }

    if !meta.comm.is_empty() {
        println!("       Command: {}", meta.comm);
    }

    if !meta.exe.is_empty() {
        println!("    Executable: {}", meta.exe);
    }

    if !meta.boot_id.is_empty() {
        println!("       Boot ID: {}", meta.boot_id);
    }

    if !meta.machine_id.is_empty() {
        println!("    Machine ID: {}", meta.machine_id);
    }

    if meta.rlimit > 0 && meta.rlimit != u64::MAX {
        println!("  Core Limit: {}", format_bytes(meta.rlimit));
    } else if meta.rlimit == u64::MAX {
        println!("  Core Limit: infinity");
    }

    if core_path.exists() {
        let actual_size = core_path
            .metadata()
            .map(|m| m.len())
            .unwrap_or(meta.core_size);
        println!(
            "      Storage: {} ({})",
            core_path.display(),
            format_bytes(actual_size)
        );
    } else if meta.core_size > 0 {
        println!(
            "      Storage: missing (was {})",
            format_bytes(meta.core_size)
        );
    } else {
        println!("      Storage: none");
    }
}

fn cmd_dump(entries: &[CoreDumpEntry], patterns: &[String], cli: &Cli) -> i32 {
    let filtered: Vec<&CoreDumpEntry> = entries
        .iter()
        .filter(|e| matches_filters(&e.meta, patterns))
        .filter(|e| matches_time_range(&e.meta, cli.since, cli.until))
        .collect();

    if filtered.is_empty() {
        if !cli.quiet {
            eprintln!("No coredumps found.");
        }
        return 1;
    }

    // Use the most recent matching entry.
    let entry = filtered.last().unwrap();

    if !entry.core_path.exists() {
        eprintln!("Core dump file not found: {}", entry.core_path.display());
        return 1;
    }

    let data = match fs::read(&entry.core_path) {
        Ok(d) => d,
        Err(e) => {
            eprintln!(
                "Failed to read core dump {}: {}",
                entry.core_path.display(),
                e
            );
            return 1;
        }
    };

    if let Some(ref output_path) = cli.output {
        match fs::write(output_path, &data) {
            Ok(()) => {
                if !cli.quiet {
                    eprintln!(
                        "Written core dump to {} ({}).",
                        output_path.display(),
                        format_bytes(data.len() as u64)
                    );
                }
            }
            Err(e) => {
                eprintln!(
                    "Failed to write core dump to {}: {}",
                    output_path.display(),
                    e
                );
                return 1;
            }
        }
    } else {
        // Write to stdout.
        // Check if stdout is a TTY; refuse to write binary data to a terminal.
        if is_stdout_tty() {
            eprintln!("Refusing to dump core to terminal (use -o FILE or pipe).");
            return 1;
        }

        let stdout = io::stdout();
        let mut handle = stdout.lock();
        if let Err(e) = handle.write_all(&data) {
            eprintln!("Failed to write core dump to stdout: {e}");
            return 1;
        }
    }

    0
}

fn cmd_debug(entries: &[CoreDumpEntry], patterns: &[String], cli: &Cli) -> i32 {
    let filtered: Vec<&CoreDumpEntry> = entries
        .iter()
        .filter(|e| matches_filters(&e.meta, patterns))
        .filter(|e| matches_time_range(&e.meta, cli.since, cli.until))
        .collect();

    if filtered.is_empty() {
        if !cli.quiet {
            eprintln!("No coredumps found.");
        }
        return 1;
    }

    let entry = filtered.last().unwrap();

    if !entry.core_path.exists() {
        eprintln!("Core dump file not found: {}", entry.core_path.display());
        return 1;
    }

    let debugger = cli.debugger.as_deref().unwrap_or("gdb");

    if !cli.quiet {
        eprintln!(
            "PID: {} ({}), Signal: {} ({}), Exe: {}",
            entry.meta.pid,
            entry.meta.comm,
            entry.meta.signal,
            entry.meta.signal_name(),
            if entry.meta.exe.is_empty() {
                "n/a"
            } else {
                &entry.meta.exe
            }
        );
    }

    let mut cmd = process::Command::new(debugger);

    // Add user-supplied debugger arguments.
    if let Some(ref args) = cli.debugger_arguments {
        for arg in args.split_whitespace() {
            cmd.arg(arg);
        }
    }

    // If the executable exists, pass it to gdb.
    if !entry.meta.exe.is_empty() && Path::new(&entry.meta.exe).exists() {
        cmd.arg(&entry.meta.exe);
    }

    cmd.arg(entry.core_path.as_os_str());

    match cmd.status() {
        Ok(status) => {
            if status.success() {
                0
            } else {
                status.code().unwrap_or(1)
            }
        }
        Err(e) => {
            eprintln!("Failed to launch debugger '{debugger}': {e}");
            1
        }
    }
}

// ---------------------------------------------------------------------------
// TTY detection
// ---------------------------------------------------------------------------

fn is_stdout_tty() -> bool {
    #[cfg(unix)]
    {
        unsafe { libc::isatty(1) != 0 }
    }
    #[cfg(not(unix))]
    {
        false
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    let coredump_dir = cli
        .directory
        .clone()
        .unwrap_or_else(|| PathBuf::from(COREDUMP_DIR));

    let entries = discover_coredumps(&coredump_dir);

    let exit_code = match &cli.command {
        Some(Commands::List { matches }) => cmd_list(&entries, matches, &cli),
        None => cmd_list(&entries, &Vec::new(), &cli),
        Some(Commands::Info { matches }) => cmd_info(&entries, matches, &cli),
        Some(Commands::Dump { matches }) => cmd_dump(&entries, matches, &cli),
        Some(Commands::Debug { matches }) | Some(Commands::Gdb { matches }) => {
            cmd_debug(&entries, matches, &cli)
        }
    };

    process::exit(exit_code);
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn make_meta(pid: u64, comm: &str, exe: &str, signal: u32, timestamp: u64) -> CoreDumpMeta {
        CoreDumpMeta {
            pid,
            uid: 1000,
            gid: 1000,
            signal,
            timestamp,
            rlimit: 0,
            hostname: "testhost".into(),
            comm: comm.into(),
            exe: exe.into(),
            backtrace: false,
            core_size: 4096,
            filename: format!("core.{comm}.1000.boot.{pid}.{timestamp}"),
            boot_id: "abcdef123456".into(),
            machine_id: "deadbeef".into(),
        }
    }

    fn meta_to_json(meta: &CoreDumpMeta) -> String {
        format!(
            r#"{{
  "PID": {},
  "UID": {},
  "GID": {},
  "SIGNAL": {},
  "SIGNAL_NAME": "{}",
  "TIMESTAMP": {},
  "RLIMIT": {},
  "HOSTNAME": "{}",
  "COMM": "{}",
  "EXE": "{}",
  "BOOT_ID": "{}",
  "MACHINE_ID": "{}",
  "CORE_SIZE": {},
  "FILENAME": "{}",
  "BACKTRACE": {}
}}"#,
            meta.pid,
            meta.uid,
            meta.gid,
            meta.signal,
            meta.signal_name(),
            meta.timestamp,
            meta.rlimit,
            meta.hostname,
            meta.comm,
            meta.exe,
            meta.boot_id,
            meta.machine_id,
            meta.core_size,
            meta.filename,
            meta.backtrace,
        )
    }

    fn create_test_coredump(dir: &Path, meta: &CoreDumpMeta) {
        let filename = &meta.filename;
        fs::write(dir.join(filename), vec![0xEF; meta.core_size as usize]).unwrap();
        fs::write(dir.join(format!("{filename}.json")), meta_to_json(meta)).unwrap();
    }

    // -- JSON parsing -------------------------------------------------------

    #[test]
    fn test_parse_json_basic() {
        let json = r#"{
  "PID": 1234,
  "UID": 1000,
  "GID": 1000,
  "SIGNAL": 11,
  "TIMESTAMP": 1700000000,
  "HOSTNAME": "myhost",
  "COMM": "myapp",
  "EXE": "/usr/bin/myapp",
  "BOOT_ID": "abc",
  "MACHINE_ID": "def",
  "CORE_SIZE": 4096,
  "FILENAME": "core.myapp.1000.abc.1234.1700000000",
  "BACKTRACE": false
}"#;

        let meta = CoreDumpMeta::from_json(json).unwrap();
        assert_eq!(meta.pid, 1234);
        assert_eq!(meta.uid, 1000);
        assert_eq!(meta.signal, 11);
        assert_eq!(meta.comm, "myapp");
        assert_eq!(meta.exe, "/usr/bin/myapp");
        assert_eq!(meta.hostname, "myhost");
        assert_eq!(meta.core_size, 4096);
        assert!(!meta.backtrace);
    }

    #[test]
    fn test_parse_json_invalid() {
        assert!(CoreDumpMeta::from_json("not json").is_none());
        assert!(CoreDumpMeta::from_json("").is_none());
        assert!(CoreDumpMeta::from_json("{}").is_none());
    }

    #[test]
    fn test_json_unescape_basic() {
        assert_eq!(json_unescape("hello"), "hello");
        assert_eq!(json_unescape(r#"a\"b"#), "a\"b");
        assert_eq!(json_unescape(r"a\\b"), "a\\b");
        assert_eq!(json_unescape(r"a\nb"), "a\nb");
        assert_eq!(json_unescape(r"a\tb"), "a\tb");
    }

    #[test]
    fn test_json_unescape_unicode() {
        assert_eq!(json_unescape(r"\u0041"), "A");
        assert_eq!(json_unescape(r"\u00e9"), "é");
    }

    // -- Signal names -------------------------------------------------------

    #[test]
    fn test_signal_names() {
        let meta = make_meta(1, "test", "", 11, 0);
        assert_eq!(meta.signal_name(), "SIGSEGV");

        let meta = make_meta(1, "test", "", 6, 0);
        assert_eq!(meta.signal_name(), "SIGABRT");

        let meta = make_meta(1, "test", "", 8, 0);
        assert_eq!(meta.signal_name(), "SIGFPE");

        let meta = make_meta(1, "test", "", 4, 0);
        assert_eq!(meta.signal_name(), "SIGILL");

        let meta = make_meta(1, "test", "", 7, 0);
        assert_eq!(meta.signal_name(), "SIGBUS");

        let meta = make_meta(1, "test", "", 5, 0);
        assert_eq!(meta.signal_name(), "SIGTRAP");

        let meta = make_meta(1, "test", "", 3, 0);
        assert_eq!(meta.signal_name(), "SIGQUIT");

        let meta = make_meta(1, "test", "", 99, 0);
        assert_eq!(meta.signal_name(), "unknown");
    }

    // -- Matching -----------------------------------------------------------

    #[test]
    fn test_matches_empty_patterns() {
        let meta = make_meta(100, "myapp", "/usr/bin/myapp", 11, 0);
        assert!(matches_filters(&meta, &[]));
    }

    #[test]
    fn test_matches_pid() {
        let meta = make_meta(100, "myapp", "/usr/bin/myapp", 11, 0);
        assert!(matches_filters(&meta, &["100".to_string()]));
        assert!(!matches_filters(&meta, &["999".to_string()]));
    }

    #[test]
    fn test_matches_comm() {
        let meta = make_meta(100, "myapp", "/usr/bin/myapp", 11, 0);
        assert!(matches_filters(&meta, &["myapp".to_string()]));
        assert!(matches_filters(&meta, &["my".to_string()]));
        assert!(!matches_filters(&meta, &["other".to_string()]));
    }

    #[test]
    fn test_matches_exe() {
        let meta = make_meta(100, "myapp", "/usr/bin/myapp", 11, 0);
        assert!(matches_filters(&meta, &["/usr/bin/myapp".to_string()]));
        assert!(matches_filters(&meta, &["/usr/bin/".to_string()]));
        assert!(!matches_filters(&meta, &["/opt/".to_string()]));
    }

    #[test]
    fn test_matches_multiple_patterns() {
        let meta = make_meta(100, "myapp", "/usr/bin/myapp", 11, 0);
        // Any match suffices (OR semantics).
        assert!(matches_filters(
            &meta,
            &["nonexist".to_string(), "myapp".to_string()]
        ));
    }

    #[test]
    fn test_matches_time_range() {
        let meta = make_meta(1, "a", "", 11, 1000);
        assert!(matches_time_range(&meta, None, None));
        assert!(matches_time_range(&meta, Some(500), None));
        assert!(matches_time_range(&meta, None, Some(2000)));
        assert!(matches_time_range(&meta, Some(500), Some(2000)));
        assert!(!matches_time_range(&meta, Some(2000), None));
        assert!(!matches_time_range(&meta, None, Some(500)));
    }

    // -- Discovery ----------------------------------------------------------

    #[test]
    fn test_discover_empty_dir() {
        let dir = TempDir::new().unwrap();
        let entries = discover_coredumps(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_discover_nonexistent_dir() {
        let entries = discover_coredumps(Path::new("/nonexistent/coredump/dir"));
        assert!(entries.is_empty());
    }

    #[test]
    fn test_discover_with_entries() {
        let dir = TempDir::new().unwrap();

        let meta1 = make_meta(100, "app1", "/usr/bin/app1", 11, 1700000000);
        let meta2 = make_meta(200, "app2", "/usr/bin/app2", 6, 1700001000);
        let meta3 = make_meta(300, "app3", "/usr/bin/app3", 8, 1700002000);

        create_test_coredump(dir.path(), &meta1);
        create_test_coredump(dir.path(), &meta2);
        create_test_coredump(dir.path(), &meta3);

        let entries = discover_coredumps(dir.path());
        assert_eq!(entries.len(), 3);

        // Should be sorted by timestamp.
        assert_eq!(entries[0].meta.pid, 100);
        assert_eq!(entries[1].meta.pid, 200);
        assert_eq!(entries[2].meta.pid, 300);
    }

    #[test]
    fn test_discover_skips_non_core_files() {
        let dir = TempDir::new().unwrap();

        // Random JSON file (not a core dump).
        fs::write(dir.path().join("random.json"), r#"{"key": "value"}"#).unwrap();

        // Core file without JSON sidecar.
        fs::write(dir.path().join("core.test.0.b.1.100"), b"data").unwrap();

        // Invalid JSON in a core-named sidecar.
        fs::write(dir.path().join("core.bad.0.b.2.200"), b"data").unwrap();
        fs::write(dir.path().join("core.bad.0.b.2.200.json"), "not valid json").unwrap();

        let entries = discover_coredumps(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_discover_handles_missing_core_file() {
        let dir = TempDir::new().unwrap();

        let meta = make_meta(100, "app", "/usr/bin/app", 11, 1700000000);
        // Only write the JSON sidecar, not the core file.
        fs::write(
            dir.path().join(format!("{}.json", meta.filename)),
            meta_to_json(&meta),
        )
        .unwrap();

        let entries = discover_coredumps(dir.path());
        assert_eq!(entries.len(), 1);
        // The core_path won't exist, but discovery should still work.
        assert!(!entries[0].core_path.exists());
    }

    // -- Formatting ---------------------------------------------------------

    #[test]
    fn test_format_timestamp_epoch() {
        let s = format_timestamp(0);
        assert_eq!(s, "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn test_format_timestamp_known_date() {
        // 2023-11-14 22:13:20 UTC = 1700000000
        let s = format_timestamp(1700000000);
        assert_eq!(s, "2023-11-14 22:13:20 UTC");
    }

    #[test]
    fn test_format_timestamp_leap_year() {
        // 2024-02-29 00:00:00 UTC = 1709164800
        let s = format_timestamp(1709164800);
        assert_eq!(s, "2024-02-29 00:00:00 UTC");
    }

    #[test]
    fn test_days_to_ymd() {
        assert_eq!(days_to_ymd(0), (1970, 1, 1));
        assert_eq!(days_to_ymd(365), (1971, 1, 1));
        // 2000-01-01 is day 10957
        assert_eq!(days_to_ymd(10957), (2000, 1, 1));
    }

    #[test]
    fn test_is_leap_year() {
        assert!(!is_leap_year(1970));
        assert!(is_leap_year(2000));
        assert!(is_leap_year(2024));
        assert!(!is_leap_year(1900));
        assert!(!is_leap_year(2023));
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

    // -- Roundtrip test -----------------------------------------------------

    #[test]
    fn test_create_and_discover_roundtrip() {
        let dir = TempDir::new().unwrap();

        let meta = make_meta(42, "crasher", "/usr/bin/crasher", 11, 1700000500);
        create_test_coredump(dir.path(), &meta);

        let entries = discover_coredumps(dir.path());
        assert_eq!(entries.len(), 1);

        let found = &entries[0].meta;
        assert_eq!(found.pid, 42);
        assert_eq!(found.comm, "crasher");
        assert_eq!(found.exe, "/usr/bin/crasher");
        assert_eq!(found.signal, 11);
        assert_eq!(found.timestamp, 1700000500);
        assert_eq!(found.core_size, 4096);

        // Core file should exist and have the right size.
        assert!(entries[0].core_path.exists());
        let core_data = fs::read(&entries[0].core_path).unwrap();
        assert_eq!(core_data.len(), 4096);
    }

    // -- Filter + discovery integration -------------------------------------

    #[test]
    fn test_filter_by_comm() {
        let dir = TempDir::new().unwrap();

        create_test_coredump(
            dir.path(),
            &make_meta(1, "firefox", "/usr/bin/firefox", 11, 1000),
        );
        create_test_coredump(
            dir.path(),
            &make_meta(2, "chrome", "/usr/bin/chrome", 6, 2000),
        );
        create_test_coredump(
            dir.path(),
            &make_meta(3, "firefox", "/usr/bin/firefox", 11, 3000),
        );

        let entries = discover_coredumps(dir.path());
        assert_eq!(entries.len(), 3);

        let firefox: Vec<_> = entries
            .iter()
            .filter(|e| matches_filters(&e.meta, &["firefox".to_string()]))
            .collect();
        assert_eq!(firefox.len(), 2);

        let chrome: Vec<_> = entries
            .iter()
            .filter(|e| matches_filters(&e.meta, &["chrome".to_string()]))
            .collect();
        assert_eq!(chrome.len(), 1);
    }

    #[test]
    fn test_filter_by_pid() {
        let dir = TempDir::new().unwrap();

        create_test_coredump(dir.path(), &make_meta(100, "app", "/bin/app", 11, 1000));
        create_test_coredump(dir.path(), &make_meta(200, "app", "/bin/app", 11, 2000));

        let entries = discover_coredumps(dir.path());

        let matched: Vec<_> = entries
            .iter()
            .filter(|e| matches_filters(&e.meta, &["200".to_string()]))
            .collect();
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].meta.pid, 200);
    }

    #[test]
    fn test_filter_by_exe_path() {
        let dir = TempDir::new().unwrap();

        create_test_coredump(dir.path(), &make_meta(1, "a", "/usr/bin/alpha", 11, 1000));
        create_test_coredump(dir.path(), &make_meta(2, "b", "/opt/bin/beta", 11, 2000));

        let entries = discover_coredumps(dir.path());

        let usr: Vec<_> = entries
            .iter()
            .filter(|e| matches_filters(&e.meta, &["/usr/bin/".to_string()]))
            .collect();
        assert_eq!(usr.len(), 1);
        assert_eq!(usr[0].meta.comm, "a");

        let opt: Vec<_> = entries
            .iter()
            .filter(|e| matches_filters(&e.meta, &["/opt/bin/beta".to_string()]))
            .collect();
        assert_eq!(opt.len(), 1);
        assert_eq!(opt[0].meta.comm, "b");
    }

    #[test]
    fn test_filter_time_range() {
        let dir = TempDir::new().unwrap();

        create_test_coredump(dir.path(), &make_meta(1, "a", "", 11, 1000));
        create_test_coredump(dir.path(), &make_meta(2, "b", "", 11, 2000));
        create_test_coredump(dir.path(), &make_meta(3, "c", "", 11, 3000));

        let entries = discover_coredumps(dir.path());

        let in_range: Vec<_> = entries
            .iter()
            .filter(|e| matches_time_range(&e.meta, Some(1500), Some(2500)))
            .collect();
        assert_eq!(in_range.len(), 1);
        assert_eq!(in_range[0].meta.pid, 2);
    }

    // -- Edge cases ---------------------------------------------------------

    #[test]
    fn test_meta_with_empty_exe() {
        let meta = make_meta(1, "app", "", 11, 0);
        assert_eq!(meta.exe, "");
        // Signal name should still work.
        assert_eq!(meta.signal_name(), "SIGSEGV");
    }

    #[test]
    fn test_meta_with_special_chars_in_comm() {
        let json = r#"{
  "PID": 1,
  "UID": 0,
  "GID": 0,
  "SIGNAL": 6,
  "TIMESTAMP": 100,
  "COMM": "my \"quoted\" app",
  "EXE": "/path/with\\backslash",
  "HOSTNAME": "host\nwith\nnewlines"
}"#;
        let meta = CoreDumpMeta::from_json(json).unwrap();
        assert_eq!(meta.comm, "my \"quoted\" app");
        assert_eq!(meta.exe, "/path/with\\backslash");
        assert_eq!(meta.hostname, "host\nwith\nnewlines");
    }

    #[test]
    fn test_parse_json_missing_optional_fields() {
        let json = r#"{
  "PID": 1,
  "UID": 0,
  "GID": 0,
  "SIGNAL": 6,
  "TIMESTAMP": 100
}"#;
        let meta = CoreDumpMeta::from_json(json).unwrap();
        assert_eq!(meta.pid, 1);
        assert_eq!(meta.hostname, "");
        assert_eq!(meta.comm, "");
        assert_eq!(meta.exe, "");
        assert_eq!(meta.core_size, 0);
        assert_eq!(meta.boot_id, "");
        assert!(!meta.backtrace);
    }
}
