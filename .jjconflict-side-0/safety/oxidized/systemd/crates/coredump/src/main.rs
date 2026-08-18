//! systemd-coredump — Process and store kernel core dumps.
//!
//! A drop-in replacement for `systemd-coredump(8)`. This tool is invoked by
//! the kernel when a process crashes, via the `/proc/sys/kernel/core_pattern`
//! pipe mechanism:
//!
//!   |/path/to/systemd-coredump [--backtrace] PID UID GID SIGNAL TIMESTAMP RLIMIT HOSTNAME COMM [EXE]
//!
//! The core dump data is read from stdin.
//!
//! This tool:
//!   1. Collects metadata about the crashed process (PID, UID, GID, signal,
//!      executable, command name, hostname, timestamp, etc.)
//!   2. Enriches metadata from `/proc/PID/` (cmdline, cgroup, environ)
//!   3. Reads the core dump from stdin
//!   4. Optionally compresses it (lz4, zstd, or xz)
//!   5. Stores it in `/var/lib/systemd/coredump/` with a descriptive filename
//!   6. Writes a JSON metadata sidecar file alongside the core dump
//!   7. Logs a structured message to the systemd journal (when Storage=journal
//!      or Storage=both)
//!   8. Applies storage and size limits from `coredump.conf`
//!   9. Vacuums old core dumps when limits are exceeded
//!
//! Configuration is read from `/etc/systemd/coredump.conf` and drop-in
//! directories.
//!
//! Exit codes:
//!   0 — success (core dump stored or intentionally skipped)
//!   1 — error

use std::collections::BTreeMap;
use std::fs;
use std::io::{self, Read, Write};
#[cfg(unix)]
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process;
use std::process::Command;
use std::time::UNIX_EPOCH;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const COREDUMP_DIR: &str = "/var/lib/systemd/coredump";

const CONFIG_FILE: &str = "/etc/systemd/coredump.conf";

const CONFIG_DROPIN_DIRS: &[&str] = &[
    "/etc/systemd/coredump.conf.d",
    "/run/systemd/coredump.conf.d",
    "/usr/lib/systemd/coredump.conf.d",
];

/// Default maximum size of a single core dump to store on disk (2 GiB).
const DEFAULT_EXTERNAL_SIZE_MAX: u64 = 2 * 1024 * 1024 * 1024;

/// Default maximum size of a core dump to process at all (2 GiB).
const DEFAULT_PROCESS_SIZE_MAX: u64 = 2 * 1024 * 1024 * 1024;

/// Sentinel meaning "MaxUse= is unset": resolve it at vacuum time as a fraction
/// of the filesystem size (10%, capped at 4 GiB), mirroring upstream
/// systemd-coredump. A fixed byte default here is wrong: on a small disk (e.g.
/// a test VM) a fixed 10 GiB KeepFree makes every fresh dump get vacuumed away.
const DEFAULT_MAX_USE: u64 = u64::MAX;

/// Sentinel meaning "KeepFree= is unset": resolve it at vacuum time as 15% of
/// the filesystem size, mirroring upstream systemd-coredump.
const DEFAULT_KEEP_FREE: u64 = u64::MAX;

/// Cap for the auto-computed MaxUse (upstream systemd caps at 4 GiB).
const MAX_USE_UPPER: u64 = 4 * 1024 * 1024 * 1024;
/// Floor for the auto-computed MaxUse (upstream systemd floors at 1 MiB).
const MAX_USE_LOWER: u64 = 1024 * 1024;

// ---------------------------------------------------------------------------
// Configuration
// ---------------------------------------------------------------------------

/// The systemd journal native protocol socket path.
const JOURNAL_SOCKET: &str = "/run/systemd/journal/socket";

/// Well-known MESSAGE_ID for coredump entries (same as real systemd-coredump).
const COREDUMP_MESSAGE_ID: &str = "fc2e22bc6ee647b6b90729ab34a250b1";

#[derive(Debug, Clone, PartialEq, Eq)]
enum Storage {
    /// Do not store core dumps at all.
    None,
    /// Store core dumps as files in /var/lib/systemd/coredump/.
    External,
    /// Store core dumps in the journal.
    Journal,
    /// Store both in journal and externally.
    Both,
}

/// Compression algorithm for stored core dumps.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Compression {
    /// No compression.
    None,
    /// LZ4 frame compression (default, like real systemd).
    Lz4,
    /// Zstandard compression.
    Zstd,
    /// XZ/LZMA2 compression.
    Xz,
}

impl Compression {
    fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "lz4" => Some(Compression::Lz4),
            "zstd" | "zstandard" => Some(Compression::Zstd),
            "xz" | "lzma" => Some(Compression::Xz),
            "none" | "no" | "false" | "0" | "off" => Some(Compression::None),
            _ => None,
        }
    }

    fn extension(self) -> &'static str {
        match self {
            Compression::None => "",
            Compression::Lz4 => ".lz4",
            Compression::Zstd => ".zst",
            Compression::Xz => ".xz",
        }
    }

    fn command_name(self) -> &'static str {
        match self {
            Compression::None => "",
            Compression::Lz4 => "lz4",
            Compression::Zstd => "zstd",
            Compression::Xz => "xz",
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            Compression::None => "none",
            Compression::Lz4 => "lz4",
            Compression::Zstd => "zstd",
            Compression::Xz => "xz",
        }
    }
}

#[derive(Debug, Clone)]
struct Config {
    storage: Storage,
    compress: bool,
    compress_algorithm: Compression,
    process_size_max: u64,
    external_size_max: u64,
    max_use: u64,
    keep_free: u64,
}

impl Default for Config {
    fn default() -> Self {
        Config {
            storage: Storage::External,
            compress: true,
            compress_algorithm: Compression::Lz4,
            process_size_max: DEFAULT_PROCESS_SIZE_MAX,
            external_size_max: DEFAULT_EXTERNAL_SIZE_MAX,
            max_use: DEFAULT_MAX_USE,
            keep_free: DEFAULT_KEEP_FREE,
        }
    }
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();

    if s.eq_ignore_ascii_case("infinity") {
        return Some(u64::MAX);
    }

    // Try plain number first (bytes).
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }

    // Try with suffix.
    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('K').or(s.strip_suffix('k')) {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix('M').or(s.strip_suffix('m')) {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('G').or(s.strip_suffix('g')) {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('T').or(s.strip_suffix('t')) {
        (n, 1024 * 1024 * 1024 * 1024)
    } else {
        return None;
    };

    num_str
        .trim()
        .parse::<u64>()
        .ok()
        .map(|n| n.saturating_mul(multiplier))
}

fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "yes" | "true" | "1" | "on" => Some(true),
        "no" | "false" | "0" | "off" => Some(false),
        _ => None,
    }
}

/// Parse the Compress= config value.
///
/// Accepts: yes/no/true/false/1/0/on/off for boolean toggle,
/// or a specific algorithm name (lz4, zstd, xz) which also enables
/// compression with that algorithm.
fn parse_compress_value(s: &str, config: &mut Config) {
    // First try as a specific algorithm name.
    if let Some(algo) = Compression::from_str(s) {
        if algo == Compression::None {
            config.compress = false;
        } else {
            config.compress = true;
            config.compress_algorithm = algo;
        }
        return;
    }
    // Fall back to boolean.
    if let Some(b) = parse_bool(s) {
        config.compress = b;
    }
}

fn parse_config_file(path: &Path, config: &mut Config) {
    let contents = match fs::read_to_string(path) {
        Ok(c) => c,
        Err(_) => return,
    };

    let mut in_section = false;

    for line in contents.lines() {
        let line = line.trim();

        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            in_section = line.eq_ignore_ascii_case("[coredump]");
            continue;
        }

        if !in_section {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "Storage" => match value.to_lowercase().as_str() {
                    "none" => config.storage = Storage::None,
                    "external" => config.storage = Storage::External,
                    "journal" => config.storage = Storage::Journal,
                    "both" => config.storage = Storage::Both,
                    _ => {
                        eprintln!("Warning: unknown Storage value '{value}', using default");
                    }
                },
                "Compress" => {
                    parse_compress_value(value, config);
                }
                "ProcessSizeMax" => {
                    if let Some(n) = parse_size(value) {
                        config.process_size_max = n;
                    }
                }
                "ExternalSizeMax" => {
                    if let Some(n) = parse_size(value) {
                        config.external_size_max = n;
                    }
                }
                "MaxUse" => {
                    if let Some(n) = parse_size(value) {
                        config.max_use = n;
                    }
                }
                "KeepFree" => {
                    if let Some(n) = parse_size(value) {
                        config.keep_free = n;
                    }
                }
                _ => {}
            }
        }
    }
}

fn load_config() -> Config {
    load_config_from(Path::new(CONFIG_FILE), CONFIG_DROPIN_DIRS)
}

fn load_config_from(config_file: &Path, dropin_dirs: &[&str]) -> Config {
    let mut config = Config::default();

    parse_config_file(config_file, &mut config);

    for dir in dropin_dirs {
        let dir_path = Path::new(dir);
        if let Ok(mut entries) = fs::read_dir(dir_path) {
            let mut paths: Vec<PathBuf> = Vec::new();
            while let Some(Ok(entry)) = entries.next() {
                let p = entry.path();
                if p.extension().and_then(|e| e.to_str()) == Some("conf") {
                    paths.push(p);
                }
            }
            paths.sort();
            for p in paths {
                parse_config_file(&p, &mut config);
            }
        }
    }

    config
}

// ---------------------------------------------------------------------------
// Core dump metadata
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct CoreDumpMeta {
    /// PID of the crashed process.
    pid: u64,
    /// UID of the crashed process.
    uid: u64,
    /// GID of the crashed process.
    gid: u64,
    /// Signal that caused the crash.
    signal: u32,
    /// Timestamp (seconds since epoch).
    timestamp: u64,
    /// RLIMIT_CORE of the crashed process.
    rlimit: u64,
    /// Hostname at the time of crash.
    hostname: String,
    /// Command name (comm).
    comm: String,
    /// Full executable path (if available).
    exe: String,
    /// Whether --backtrace mode was requested.
    backtrace: bool,
    /// Size of the stored core dump (set after writing).
    core_size: u64,
    /// Filename of the stored core dump (set after writing).
    filename: String,
    /// Boot ID (from /proc/sys/kernel/random/boot_id).
    boot_id: String,
    /// Machine ID (from /etc/machine-id).
    machine_id: String,
    /// Full command line from /proc/PID/cmdline (NUL bytes replaced with spaces).
    cmdline: String,
    /// Cgroup path from /proc/PID/cgroup.
    cgroup: String,
    /// Environment variables from /proc/PID/environ (NUL-separated, newline-joined).
    environ: String,
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

    /// Serialize metadata to JSON format.
    fn to_json(&self) -> String {
        let mut json = String::from("{\n");
        json.push_str(&format!("  \"PID\": {},\n", self.pid));
        json.push_str(&format!("  \"UID\": {},\n", self.uid));
        json.push_str(&format!("  \"GID\": {},\n", self.gid));
        json.push_str(&format!("  \"SIGNAL\": {},\n", self.signal));
        json.push_str(&format!("  \"SIGNAL_NAME\": \"{}\",\n", self.signal_name()));
        json.push_str(&format!("  \"TIMESTAMP\": {},\n", self.timestamp));
        json.push_str(&format!("  \"RLIMIT\": {},\n", self.rlimit));
        json.push_str(&format!(
            "  \"HOSTNAME\": \"{}\",\n",
            json_escape(&self.hostname)
        ));
        json.push_str(&format!("  \"COMM\": \"{}\",\n", json_escape(&self.comm)));
        json.push_str(&format!("  \"EXE\": \"{}\",\n", json_escape(&self.exe)));
        json.push_str(&format!(
            "  \"BOOT_ID\": \"{}\",\n",
            json_escape(&self.boot_id)
        ));
        json.push_str(&format!(
            "  \"MACHINE_ID\": \"{}\",\n",
            json_escape(&self.machine_id)
        ));
        json.push_str(&format!("  \"CORE_SIZE\": {},\n", self.core_size));
        json.push_str(&format!(
            "  \"FILENAME\": \"{}\",\n",
            json_escape(&self.filename)
        ));
        json.push_str(&format!(
            "  \"CMDLINE\": \"{}\",\n",
            json_escape(&self.cmdline)
        ));
        json.push_str(&format!(
            "  \"CGROUP\": \"{}\",\n",
            json_escape(&self.cgroup)
        ));
        json.push_str(&format!(
            "  \"ENVIRON\": \"{}\",\n",
            json_escape(&self.environ)
        ));
        json.push_str(&format!("  \"BACKTRACE\": {}\n", self.backtrace));
        json.push('}');
        json
    }

    /// Deserialize metadata from JSON format.
    #[allow(dead_code)]
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
            cmdline: map.get("CMDLINE").cloned().unwrap_or_default(),
            cgroup: map.get("CGROUP").cloned().unwrap_or_default(),
            environ: map.get("ENVIRON").cloned().unwrap_or_default(),
        })
    }
}

/// Minimal JSON string escaping.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Minimal JSON object parser — returns key/value string pairs.
/// Only handles flat objects with string, number, and boolean values.
#[allow(dead_code)]
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

            // Strip quotes from string values.
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

#[allow(dead_code)]
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
                        && let Some(c) = char::from_u32(n)
                    {
                        out.push(c);
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
// System information helpers
// ---------------------------------------------------------------------------

fn read_boot_id() -> String {
    fs::read_to_string("/proc/sys/kernel/random/boot_id")
        .unwrap_or_default()
        .trim()
        .replace('-', "")
}

fn read_machine_id() -> String {
    fs::read_to_string("/etc/machine-id")
        .unwrap_or_default()
        .trim()
        .to_owned()
}

// ---------------------------------------------------------------------------
// /proc/PID/ metadata enrichment
// ---------------------------------------------------------------------------

/// Read the command line of a process from `/proc/PID/cmdline`.
///
/// The kernel stores command line arguments separated by NUL bytes.
/// We replace NUL bytes with spaces for human-readable display.
/// Returns an empty string if the process has already exited or is
/// otherwise unreadable.
fn read_proc_cmdline(pid: u64) -> String {
    read_proc_cmdline_from(&format!("/proc/{pid}/cmdline"))
}

fn read_proc_cmdline_from(path: &str) -> String {
    match fs::read(path) {
        Ok(data) => {
            if data.is_empty() {
                return String::new();
            }
            // Replace NUL separators with spaces, trim trailing NUL/space.
            let s: String = data
                .iter()
                .map(|&b| if b == 0 { ' ' } else { b as char })
                .collect();
            s.trim_end().to_owned()
        }
        Err(_) => String::new(),
    }
}

/// Read the cgroup membership of a process from `/proc/PID/cgroup`.
///
/// Returns the cgroup path(s). For cgroup v2, this is typically a single
/// line like `0::/system.slice/foo.service`. We return the full contents
/// trimmed, preserving all hierarchy lines.
fn read_proc_cgroup(pid: u64) -> String {
    read_proc_cgroup_from(&format!("/proc/{pid}/cgroup"))
}

fn read_proc_cgroup_from(path: &str) -> String {
    fs::read_to_string(path)
        .unwrap_or_default()
        .trim()
        .to_owned()
}

/// Read environment variables of a process from `/proc/PID/environ`.
///
/// The kernel stores environment variables separated by NUL bytes.
/// We split on NUL and join with newlines for structured storage.
/// Returns an empty string if unreadable (process exited, or
/// permission denied for non-owned processes).
fn read_proc_environ(pid: u64) -> String {
    read_proc_environ_from(&format!("/proc/{pid}/environ"))
}

fn read_proc_environ_from(path: &str) -> String {
    match fs::read(path) {
        Ok(data) => {
            if data.is_empty() {
                return String::new();
            }
            // Split on NUL, filter empty, join with newline.
            let vars: Vec<String> = data
                .split(|&b| b == 0)
                .filter(|s| !s.is_empty())
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .collect();
            vars.join("\n")
        }
        Err(_) => String::new(),
    }
}

/// Enrich a CoreDumpMeta with data read from `/proc/PID/`.
///
/// This should be called as early as possible after the kernel invokes
/// us, because the crashing process's `/proc/PID/` entry may disappear
/// once the kernel finishes cleaning up.
fn enrich_from_proc(meta: &mut CoreDumpMeta) {
    meta.cmdline = read_proc_cmdline(meta.pid);
    meta.cgroup = read_proc_cgroup(meta.pid);
    meta.environ = read_proc_environ(meta.pid);
    // comm and the executable path are not passed via the kernel core_pattern
    // argv; read them from /proc/PID/ while the crashed process is still around
    // (the kernel keeps it until this pipe handler exits). Only fill if empty so
    // an explicitly-provided value (e.g. --backtrace mode) is preserved.
    if meta.comm.is_empty()
        && let Ok(comm) = std::fs::read_to_string(format!("/proc/{}/comm", meta.pid))
    {
        meta.comm = comm.trim_end().to_string();
    }
    if meta.exe.is_empty()
        && let Ok(path) = std::fs::read_link(format!("/proc/{}/exe", meta.pid))
    {
        meta.exe = path.to_string_lossy().into_owned();
    }
}

#[cfg(test)]
fn enrich_from_proc_paths(
    meta: &mut CoreDumpMeta,
    cmdline_path: &str,
    cgroup_path: &str,
    environ_path: &str,
) {
    meta.cmdline = read_proc_cmdline_from(cmdline_path);
    meta.cgroup = read_proc_cgroup_from(cgroup_path);
    meta.environ = read_proc_environ_from(environ_path);
}

// ---------------------------------------------------------------------------
// Compression
// ---------------------------------------------------------------------------

/// Compress data using the specified algorithm.
///
/// Compression is performed by invoking the system's compression command
/// (lz4, zstd, xz) as a subprocess with data piped through stdin/stdout.
/// This avoids heavy library dependencies while supporting all three
/// algorithms that real systemd-coredump supports.
///
/// Returns the compressed data, or an error if compression fails.
/// The caller should fall back to storing uncompressed data on error.
fn compress_data(data: &[u8], algorithm: Compression) -> Result<Vec<u8>, String> {
    if algorithm == Compression::None {
        return Ok(data.to_vec());
    }

    let cmd_name = algorithm.command_name();

    // Build command with appropriate flags for stdout output.
    let mut cmd = Command::new(cmd_name);
    match algorithm {
        Compression::Lz4 => {
            // lz4 -z -c: compress to stdout
            cmd.arg("-z").arg("-c");
        }
        Compression::Zstd => {
            // zstd -c: compress to stdout, -q: quiet
            cmd.arg("-c").arg("-q");
        }
        Compression::Xz => {
            // xz -c: compress to stdout
            cmd.arg("-c");
        }
        Compression::None => unreachable!(),
    }

    cmd.stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{cmd_name} command not found or failed to start: {e}"))?;

    // Write data to stdin in a separate scope so stdin is closed.
    if let Some(mut stdin) = child.stdin.take() {
        // Best-effort write; if the child dies we'll catch it on wait().
        let _ = stdin.write_all(data);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("{cmd_name} failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{cmd_name} exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(output.stdout)
}

/// Decompress data using the specified algorithm (detected from file extension).
///
/// Used for reading back compressed core dumps.
#[allow(dead_code)]
fn decompress_data(data: &[u8], algorithm: Compression) -> Result<Vec<u8>, String> {
    if algorithm == Compression::None {
        return Ok(data.to_vec());
    }

    let cmd_name = algorithm.command_name();

    let mut cmd = Command::new(cmd_name);
    match algorithm {
        Compression::Lz4 => {
            cmd.arg("-d").arg("-c");
        }
        Compression::Zstd => {
            cmd.arg("-d").arg("-c").arg("-q");
        }
        Compression::Xz => {
            cmd.arg("-d").arg("-c");
        }
        Compression::None => unreachable!(),
    }

    cmd.stdin(process::Stdio::piped())
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::piped());

    let mut child = cmd
        .spawn()
        .map_err(|e| format!("{cmd_name} decompression failed to start: {e}"))?;

    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(data);
    }

    let output = child
        .wait_with_output()
        .map_err(|e| format!("{cmd_name} decompression failed: {e}"))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        return Err(format!(
            "{cmd_name} decompression exited with {}: {}",
            output.status,
            stderr.trim()
        ));
    }

    Ok(output.stdout)
}

/// Detect compression algorithm from a filename's extension.
#[allow(dead_code)]
fn detect_compression(filename: &str) -> Compression {
    if filename.ends_with(".lz4") {
        Compression::Lz4
    } else if filename.ends_with(".zst") {
        Compression::Zstd
    } else if filename.ends_with(".xz") {
        Compression::Xz
    } else {
        Compression::None
    }
}

// ---------------------------------------------------------------------------
// Journal integration
// ---------------------------------------------------------------------------

/// Build a native journal protocol message for a coredump event.
///
/// The native protocol sends newline-separated `KEY=VALUE` pairs to the
/// journal socket as a single datagram. For binary-safe values, the
/// format uses: `KEY\n<64-bit LE length><data>\n`.
fn build_journal_entry(meta: &CoreDumpMeta) -> Vec<u8> {
    let mut msg = Vec::with_capacity(1024);

    // Standard journal fields.
    writeln!(
        msg,
        "MESSAGE=Process {} ({}) of user {} dumped core: signal {} ({})",
        meta.pid,
        meta.comm,
        meta.uid,
        meta.signal,
        meta.signal_name()
    )
    .unwrap();
    writeln!(msg, "MESSAGE_ID={COREDUMP_MESSAGE_ID}").unwrap();
    writeln!(msg, "PRIORITY=2").unwrap(); // LOG_CRIT

    // Coredump-specific fields (matching real systemd-coredump).
    writeln!(msg, "COREDUMP_PID={}", meta.pid).unwrap();
    writeln!(msg, "COREDUMP_UID={}", meta.uid).unwrap();
    writeln!(msg, "COREDUMP_GID={}", meta.gid).unwrap();
    writeln!(msg, "COREDUMP_SIGNAL={}", meta.signal).unwrap();
    writeln!(msg, "COREDUMP_SIGNAL_NAME={}", meta.signal_name()).unwrap();
    writeln!(msg, "COREDUMP_TIMESTAMP={}", meta.timestamp).unwrap();
    writeln!(msg, "COREDUMP_RLIMIT={}", meta.rlimit).unwrap();

    if !meta.hostname.is_empty() {
        writeln!(msg, "COREDUMP_HOSTNAME={}", meta.hostname).unwrap();
    }
    if !meta.comm.is_empty() {
        writeln!(msg, "COREDUMP_COMM={}", meta.comm).unwrap();
    }
    if !meta.exe.is_empty() {
        writeln!(msg, "COREDUMP_EXE={}", meta.exe).unwrap();
    }
    if !meta.boot_id.is_empty() {
        writeln!(msg, "COREDUMP_BOOT_ID={}", meta.boot_id).unwrap();
    }
    if !meta.machine_id.is_empty() {
        writeln!(msg, "COREDUMP_MACHINE_ID={}", meta.machine_id).unwrap();
    }
    if !meta.filename.is_empty() {
        writeln!(msg, "COREDUMP_FILENAME={}", meta.filename).unwrap();
    }
    if meta.core_size > 0 {
        writeln!(msg, "COREDUMP_SIZE={}", meta.core_size).unwrap();
    }

    // /proc/PID/ enriched fields.
    if !meta.cmdline.is_empty() {
        writeln!(msg, "COREDUMP_CMDLINE={}", meta.cmdline).unwrap();
    }
    if !meta.cgroup.is_empty() {
        // Cgroup may contain newlines; use binary-safe encoding.
        write_journal_field_binary(&mut msg, "COREDUMP_CGROUP", meta.cgroup.as_bytes());
    }
    if !meta.environ.is_empty() {
        // Environ contains newlines; use binary-safe encoding.
        write_journal_field_binary(&mut msg, "COREDUMP_ENVIRON", meta.environ.as_bytes());
    }

    msg
}

/// Write a binary-safe journal field.
///
/// Format: `KEY\n<64-bit LE length><data>\n`
fn write_journal_field_binary(msg: &mut Vec<u8>, key: &str, data: &[u8]) {
    msg.extend_from_slice(key.as_bytes());
    msg.push(b'\n');
    msg.extend_from_slice(&(data.len() as u64).to_le_bytes());
    msg.extend_from_slice(data);
    msg.push(b'\n');
}

/// Send a coredump entry to the systemd journal via the native protocol.
///
/// This sends a structured log message to `/run/systemd/journal/socket`
/// using a Unix datagram socket. If the journal is unavailable, the
/// error is logged to stderr but does not cause a failure.
fn send_to_journal(meta: &CoreDumpMeta) {
    send_to_journal_at(JOURNAL_SOCKET, meta);
}

fn send_to_journal_at(socket_path: &str, meta: &CoreDumpMeta) {
    let msg = build_journal_entry(meta);

    #[cfg(unix)]
    {
        let sock = match UnixDatagram::unbound() {
            Ok(s) => s,
            Err(e) => {
                eprintln!("Warning: failed to create journal socket: {e}");
                return;
            }
        };

        if let Err(e) = sock.send_to(&msg, socket_path) {
            eprintln!("Warning: failed to send to journal socket: {e}");
        }
    }

    #[cfg(not(unix))]
    {
        let _ = (socket_path, msg);
        eprintln!("Warning: journal integration not available on this platform");
    }
}

/// Build the core dump filename following systemd conventions:
/// `core.COMM.UID.BOOT_ID.PID.TIMESTAMP[.lz4|.zst|.xz]`
fn build_filename(meta: &CoreDumpMeta, compression: Compression) -> String {
    // Sanitize comm: replace non-alphanumeric characters.
    let safe_comm: String = meta
        .comm
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect();

    format!(
        "core.{}.{}.{}.{}.{}{}",
        safe_comm,
        meta.uid,
        meta.boot_id,
        meta.pid,
        meta.timestamp,
        compression.extension(),
    )
}

// ---------------------------------------------------------------------------
// Core dump storage
// ---------------------------------------------------------------------------

/// Read the core dump from stdin, respecting the process size limit.
fn read_core_dump(max_size: u64) -> io::Result<Vec<u8>> {
    let mut buf = Vec::new();
    let mut stdin = io::stdin().lock();

    if max_size == 0 {
        return Ok(buf);
    }

    if max_size == u64::MAX {
        stdin.read_to_end(&mut buf)?;
        return Ok(buf);
    }

    // Read up to max_size + 1 to detect truncation.
    let limit = max_size.saturating_add(1) as usize;
    let mut take = stdin.take(limit as u64);
    take.read_to_end(&mut buf)?;

    if buf.len() as u64 > max_size {
        buf.truncate(max_size as usize);
    }

    Ok(buf)
}

/// Store the core dump to disk and write the metadata sidecar.
///
/// If compression is enabled in the config, the core dump is compressed
/// before writing and the filename gets the appropriate extension
/// (`.lz4`, `.zst`, or `.xz`). If the compression tool is not available,
/// falls back to storing uncompressed.
fn store_core_dump(
    coredump_dir: &Path,
    meta: &mut CoreDumpMeta,
    data: &[u8],
    config: &Config,
) -> io::Result<()> {
    fs::create_dir_all(coredump_dir)?;

    // Check external size limit against uncompressed size.
    if data.len() as u64 > config.external_size_max {
        eprintln!(
            "Core dump for PID {} ({}) is {} bytes, exceeding ExternalSizeMax={}, skipping storage",
            meta.pid,
            meta.comm,
            data.len(),
            config.external_size_max,
        );
        return Ok(());
    }

    // Compress if enabled.
    let (write_data, actual_compression) =
        if config.compress && config.compress_algorithm != Compression::None {
            match compress_data(data, config.compress_algorithm) {
                Ok(compressed) => {
                    eprintln!(
                        "Compressed core dump with {}: {} -> {} bytes ({:.1}%)",
                        config.compress_algorithm.as_str(),
                        data.len(),
                        compressed.len(),
                        if data.is_empty() {
                            0.0
                        } else {
                            (compressed.len() as f64 / data.len() as f64) * 100.0
                        },
                    );
                    (compressed, config.compress_algorithm)
                }
                Err(e) => {
                    eprintln!(
                        "Warning: {} compression failed, storing uncompressed: {e}",
                        config.compress_algorithm.as_str(),
                    );
                    (data.to_vec(), Compression::None)
                }
            }
        } else {
            (data.to_vec(), Compression::None)
        };

    let filename = build_filename(meta, actual_compression);

    // Write core dump file.
    let core_path = coredump_dir.join(&filename);
    fs::write(&core_path, &write_data)?;

    // Set permissions to 0640 (owner read-write, group read).
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&core_path, fs::Permissions::from_mode(0o640))?;
    }

    meta.core_size = data.len() as u64;
    meta.filename = filename.clone();

    // Write JSON metadata sidecar.
    let meta_path = coredump_dir.join(format!("{filename}.json"));
    fs::write(&meta_path, meta.to_json())?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&meta_path, fs::Permissions::from_mode(0o640))?;
    }

    Ok(())
}

/// List stored core dumps (core files with .json sidecars), sorted by timestamp.
fn list_stored_coredumps(coredump_dir: &Path) -> Vec<(PathBuf, PathBuf, u64)> {
    let mut entries: Vec<(PathBuf, PathBuf, u64)> = Vec::new();

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

        // Find .json metadata files.
        if !name.ends_with(".json") {
            continue;
        }
        if !name.starts_with("core.") {
            continue;
        }

        // Corresponding core dump file.
        let core_name = name.strip_suffix(".json").unwrap_or(&name);
        let core_path = coredump_dir.join(core_name);

        // Get modification time for sorting (older = smaller).
        let mtime = path
            .metadata()
            .and_then(|m| m.modified())
            .and_then(|t| t.duration_since(UNIX_EPOCH).map_err(io::Error::other))
            .map(|d| d.as_secs())
            .unwrap_or(0);

        entries.push((core_path, path, mtime));
    }

    // Sort by mtime ascending (oldest first).
    entries.sort_by_key(|e| e.2);
    entries
}

/// Vacuum old core dumps to stay within limits.
fn vacuum(coredump_dir: &Path, config: &Config) {
    let mut entries = list_stored_coredumps(coredump_dir);
    if entries.is_empty() {
        return;
    }

    // Resolve MaxUse=/KeepFree= (filesystem-relative when left at their unset
    // sentinel), so a fresh dump is not vacuumed away on a small filesystem.
    let (max_use, keep_free) = resolve_disk_limits(coredump_dir, config);

    // Calculate total size.
    let mut total_size: u64 = 0;
    for (core_path, meta_path, _) in &entries {
        if let Ok(m) = core_path.metadata() {
            total_size += m.len();
        }
        if let Ok(m) = meta_path.metadata() {
            total_size += m.len();
        }
    }

    // Remove oldest entries while over limits.
    while total_size > max_use && !entries.is_empty() {
        let (core_path, meta_path, _) = entries.remove(0);

        let mut freed: u64 = 0;
        if let Ok(m) = core_path.metadata() {
            freed += m.len();
        }
        if let Ok(m) = meta_path.metadata() {
            freed += m.len();
        }

        let _ = fs::remove_file(&core_path);
        let _ = fs::remove_file(&meta_path);

        total_size = total_size.saturating_sub(freed);
    }

    // Check keep-free against available disk space.
    if keep_free > 0
        && let Some(avail) = available_disk_space(coredump_dir)
    {
        while avail + freed_so_far(&entries, coredump_dir) < keep_free && !entries.is_empty() {
            let (core_path, meta_path, _) = entries.remove(0);
            let _ = fs::remove_file(&core_path);
            let _ = fs::remove_file(&meta_path);
        }
    }
}

/// Get available disk space on the filesystem containing `path`.
fn available_disk_space(path: &Path) -> Option<u64> {
    // Use statvfs via libc.
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;

        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
                Some(stat.f_bavail * stat.f_frsize)
            } else {
                None
            }
        }
    }

    #[cfg(not(unix))]
    {
        None
    }
}

/// Total size (bytes) of the filesystem containing `path`, via statvfs.
fn total_disk_space(path: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::ffi::CString;
        use std::os::unix::ffi::OsStrExt;

        let c_path = CString::new(path.as_os_str().as_bytes()).ok()?;

        unsafe {
            let mut stat: libc::statvfs = std::mem::zeroed();
            if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
                Some(stat.f_blocks * stat.f_frsize)
            } else {
                None
            }
        }
    }

    #[cfg(not(unix))]
    {
        None
    }
}

/// Resolve the effective `MaxUse=` / `KeepFree=` limits, computing filesystem
/// relative defaults (10% / 15% of the fs, MaxUse capped at 4 GiB) when the
/// configured value is the "unset" sentinel — mirroring systemd-coredump.
fn resolve_disk_limits(coredump_dir: &Path, config: &Config) -> (u64, u64) {
    let fs_size = total_disk_space(coredump_dir);
    let max_use = if config.max_use == DEFAULT_MAX_USE {
        fs_size
            .map(|s| (s / 10).clamp(MAX_USE_LOWER, MAX_USE_UPPER))
            .unwrap_or(MAX_USE_UPPER)
    } else {
        config.max_use
    };
    let keep_free = if config.keep_free == DEFAULT_KEEP_FREE {
        fs_size.map(|s| s * 15 / 100).unwrap_or(0)
    } else {
        config.keep_free
    };
    (max_use, keep_free)
}

/// Helper — not actually used for iterative removal, just a placeholder
/// to keep the compiler happy.  The real vacuum loop removes one at a time.
fn freed_so_far(_entries: &[(PathBuf, PathBuf, u64)], _dir: &Path) -> u64 {
    0
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Args {
    meta: CoreDumpMeta,
}

/// Parse kernel-supplied arguments.
///
/// The kernel invokes us as:
///   systemd-coredump [--backtrace] PID UID GID SIGNAL TIMESTAMP RLIMIT HOSTNAME COMM [EXE]
fn parse_args(args: &[String]) -> Result<Args, String> {
    let mut iter = args.iter().skip(1); // skip argv[0]
    let mut backtrace = false;

    // Peek at first arg for --backtrace flag.
    let first = iter.next().ok_or("missing arguments")?;
    let pid_str = if first == "--backtrace" {
        backtrace = true;
        iter.next().ok_or("missing PID")?
    } else {
        first
    };

    let pid: u64 = pid_str
        .parse()
        .map_err(|_| format!("invalid PID: {pid_str}"))?;

    let uid: u64 = iter
        .next()
        .ok_or("missing UID")?
        .parse()
        .map_err(|e| format!("invalid UID: {e}"))?;

    let gid: u64 = iter
        .next()
        .ok_or("missing GID")?
        .parse()
        .map_err(|e| format!("invalid GID: {e}"))?;

    let signal: u32 = iter
        .next()
        .ok_or("missing SIGNAL")?
        .parse()
        .map_err(|e| format!("invalid SIGNAL: {e}"))?;

    let timestamp: u64 = iter
        .next()
        .ok_or("missing TIMESTAMP")?
        .parse()
        .map_err(|e| format!("invalid TIMESTAMP: {e}"))?;

    let rlimit: u64 = iter
        .next()
        .ok_or("missing RLIMIT")?
        .parse()
        .map_err(|e| format!("invalid RLIMIT: {e}"))?;

    let hostname = iter.next().ok_or("missing HOSTNAME")?.clone();
    // The remaining core_pattern specifiers are %d (dumpable) and %F (pidfd);
    // both were added to the kernel core_pattern in later versions and so are
    // optional. Crucially, comm and the executable path are NOT passed via argv
    // — upstream reads them from /proc/PID/ (see enrich_from_proc). Previously
    // this code mistook %d/%F for comm/exe, storing e.g. COREDUMP_EXE="3" (a
    // pidfd number), so `coredumpctl list <exe>` could never match the dump.
    let _dumpable = iter.next();
    let _pidfd = iter.next();
    let comm = String::new();
    let exe = String::new();

    Ok(Args {
        meta: CoreDumpMeta {
            pid,
            uid,
            gid,
            signal,
            timestamp,
            rlimit,
            hostname,
            comm,
            exe,
            backtrace,
            core_size: 0,
            filename: String::new(),
            boot_id: read_boot_id(),
            machine_id: read_machine_id(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        },
    })
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run() -> Result<(), String> {
    let args: Vec<String> = std::env::args().collect();

    let parsed = parse_args(&args)?;
    let mut meta = parsed.meta;
    let config = load_config();

    // Enrich metadata from /proc/PID/ as early as possible, before the
    // process's proc entry disappears.
    enrich_from_proc(&mut meta);

    eprintln!(
        "Process {} ({}) of user {} dumped core: signal {} ({})",
        meta.pid,
        meta.comm,
        meta.uid,
        meta.signal,
        meta.signal_name(),
    );

    if !meta.cmdline.is_empty() {
        eprintln!("  Command line: {}", meta.cmdline);
    }
    if !meta.cgroup.is_empty() {
        // Show first line of cgroup for brevity.
        let first_line = meta.cgroup.lines().next().unwrap_or("");
        eprintln!("  Cgroup: {first_line}");
    }

    if config.storage == Storage::None {
        eprintln!("Core dump storage is disabled (Storage=none), skipping.");
        return Ok(());
    }

    // Read the core dump from stdin.
    let data = read_core_dump(config.process_size_max)
        .map_err(|e| format!("failed to read core dump from stdin: {e}"))?;

    if data.is_empty() {
        eprintln!("Empty core dump, nothing to store.");
        return Ok(());
    }

    eprintln!(
        "Core dump size: {} bytes for PID {} ({})",
        data.len(),
        meta.pid,
        meta.comm,
    );

    let coredump_dir = Path::new(COREDUMP_DIR);

    // Store externally if Storage is External or Both.
    if config.storage == Storage::External || config.storage == Storage::Both {
        store_core_dump(coredump_dir, &mut meta, &data, &config)
            .map_err(|e| format!("failed to store core dump: {e}"))?;

        if !meta.filename.is_empty() {
            eprintln!("Stored core dump: {}/{}", COREDUMP_DIR, meta.filename);
        }
    }

    // For journal-only storage, record the size (no external file was written).
    if config.storage == Storage::Journal {
        meta.core_size = data.len() as u64;
        meta.filename = build_filename(&meta, Compression::None);
    }

    // Always send the coredump metadata message to the journal. coredumpctl
    // enumerates dumps by reading these journal entries (matched by
    // MESSAGE_ID), so even externally-stored dumps need one — with
    // Storage=external the entry carries COREDUMP_FILENAME pointing at the file
    // under /var/lib/systemd/coredump/. (Storage=none already returned early.)
    send_to_journal(&meta);
    eprintln!(
        "Sent coredump metadata to journal for PID {} ({})",
        meta.pid, meta.comm
    );

    // Vacuum old dumps (only relevant if we store externally).
    if config.storage == Storage::External || config.storage == Storage::Both {
        vacuum(coredump_dir, &config);
    }

    Ok(())
}

fn main() {
    if let Err(e) = run() {
        eprintln!("systemd-coredump: {e}");
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    // -- Config tests -------------------------------------------------------

    #[test]
    fn test_default_config() {
        let config = Config::default();
        assert_eq!(config.storage, Storage::External);
        assert!(config.compress);
        assert_eq!(config.compress_algorithm, Compression::Lz4);
        assert_eq!(config.process_size_max, DEFAULT_PROCESS_SIZE_MAX);
        assert_eq!(config.external_size_max, DEFAULT_EXTERNAL_SIZE_MAX);
        assert_eq!(config.max_use, DEFAULT_MAX_USE);
        assert_eq!(config.keep_free, DEFAULT_KEEP_FREE);
    }

    #[test]
    fn test_parse_config_storage_none() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nStorage=none\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
    }

    #[test]
    fn test_parse_config_storage_external() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nStorage=external\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::External);
    }

    #[test]
    fn test_parse_config_storage_journal() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nStorage=journal\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::Journal);
    }

    #[test]
    fn test_parse_config_storage_both() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nStorage=both\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::Both);
    }

    #[test]
    fn test_parse_config_compress_no() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nCompress=no\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert!(!config.compress);
    }

    #[test]
    fn test_parse_config_compress_yes() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nCompress=yes\n").unwrap();
        let mut config = Config {
            compress: false,
            ..Default::default()
        };
        parse_config_file(&conf, &mut config);
        assert!(config.compress);
    }

    #[test]
    fn test_parse_config_compress_lz4() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nCompress=lz4\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert!(config.compress);
        assert_eq!(config.compress_algorithm, Compression::Lz4);
    }

    #[test]
    fn test_parse_config_compress_zstd() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nCompress=zstd\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert!(config.compress);
        assert_eq!(config.compress_algorithm, Compression::Zstd);
    }

    #[test]
    fn test_parse_config_compress_xz() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nCompress=xz\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert!(config.compress);
        assert_eq!(config.compress_algorithm, Compression::Xz);
    }

    #[test]
    fn test_parse_config_compress_none_algorithm() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nCompress=none\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert!(!config.compress);
    }

    #[test]
    fn test_parse_config_sizes() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(
            &conf,
            "[Coredump]\nProcessSizeMax=100M\nExternalSizeMax=50M\nMaxUse=1G\nKeepFree=500M\n",
        )
        .unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.process_size_max, 100 * 1024 * 1024);
        assert_eq!(config.external_size_max, 50 * 1024 * 1024);
        assert_eq!(config.max_use, 1024 * 1024 * 1024);
        assert_eq!(config.keep_free, 500 * 1024 * 1024);
    }

    #[test]
    fn test_parse_config_infinity() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nProcessSizeMax=infinity\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.process_size_max, u64::MAX);
    }

    #[test]
    fn test_parse_config_missing_file() {
        let mut config = Config::default();
        parse_config_file(Path::new("/nonexistent/coredump.conf"), &mut config);
        assert_eq!(config.storage, Storage::External); // unchanged
    }

    #[test]
    fn test_parse_config_wrong_section_ignored() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Network]\nStorage=none\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::External); // unchanged
    }

    #[test]
    fn test_parse_config_comments_and_blanks() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(
            &conf,
            "# Comment\n\n; Another comment\n[Coredump]\n# Inline\nStorage=none\n",
        )
        .unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
    }

    #[test]
    fn test_parse_config_case_insensitive_section() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[COREDUMP]\nStorage=none\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
    }

    #[test]
    fn test_parse_config_dropin() {
        let dir = TempDir::new().unwrap();
        let main_conf = dir.path().join("coredump.conf");
        fs::write(&main_conf, "[Coredump]\nStorage=none\n").unwrap();

        let dropin_dir = dir.path().join("coredump.conf.d");
        fs::create_dir(&dropin_dir).unwrap();
        fs::write(
            dropin_dir.join("override.conf"),
            "[Coredump]\nStorage=external\n",
        )
        .unwrap();

        let dropin_str = dropin_dir.to_str().unwrap().to_owned();
        let config = load_config_from(&main_conf, &[dropin_str.as_str()]);
        assert_eq!(config.storage, Storage::External); // override wins
    }

    // -- Size parsing tests -------------------------------------------------

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("1024"), Some(1024));
    }

    #[test]
    fn test_parse_size_kilobytes() {
        assert_eq!(parse_size("100K"), Some(100 * 1024));
        assert_eq!(parse_size("100k"), Some(100 * 1024));
    }

    #[test]
    fn test_parse_size_megabytes() {
        assert_eq!(parse_size("50M"), Some(50 * 1024 * 1024));
    }

    #[test]
    fn test_parse_size_gigabytes() {
        assert_eq!(parse_size("2G"), Some(2 * 1024 * 1024 * 1024));
    }

    #[test]
    fn test_parse_size_terabytes() {
        assert_eq!(parse_size("1T"), Some(1024 * 1024 * 1024 * 1024));
    }

    #[test]
    fn test_parse_size_infinity() {
        assert_eq!(parse_size("infinity"), Some(u64::MAX));
    }

    #[test]
    fn test_parse_size_invalid() {
        assert_eq!(parse_size("abc"), None);
    }

    #[test]
    fn test_parse_size_zero() {
        assert_eq!(parse_size("0"), Some(0));
    }

    // -- Bool parsing tests -------------------------------------------------

    #[test]
    fn test_parse_bool_yes() {
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("on"), Some(true));
    }

    #[test]
    fn test_parse_bool_no() {
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("off"), Some(false));
    }

    #[test]
    fn test_parse_bool_invalid() {
        assert_eq!(parse_bool("maybe"), None);
    }

    // -- JSON tests ---------------------------------------------------------

    #[test]
    fn test_json_escape_basic() {
        assert_eq!(json_escape("hello"), "hello");
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("a\rb"), "a\\rb");
        assert_eq!(json_escape("a\tb"), "a\\tb");
    }

    #[test]
    fn test_json_escape_control_chars() {
        assert_eq!(json_escape("\x01"), "\\u0001");
    }

    #[test]
    fn test_json_roundtrip() {
        let meta = CoreDumpMeta {
            pid: 12345,
            uid: 1000,
            gid: 1000,
            signal: 11,
            timestamp: 1700000000,
            rlimit: u64::MAX,
            hostname: "test-host".into(),
            comm: "my-program".into(),
            exe: "/usr/bin/my-program".into(),
            backtrace: false,
            core_size: 4096,
            filename: "core.my-program.1000.abc.12345.1700000000".into(),
            boot_id: "abcdef0123456789".into(),
            machine_id: "deadbeef12345678".into(),
            cmdline: "/usr/bin/my-program --flag arg1".into(),
            cgroup: "0::/user.slice/user-1000.slice".into(),
            environ: "HOME=/home/user\nPATH=/usr/bin".into(),
        };

        let json = meta.to_json();
        let parsed = CoreDumpMeta::from_json(&json).unwrap();

        assert_eq!(parsed.pid, meta.pid);
        assert_eq!(parsed.uid, meta.uid);
        assert_eq!(parsed.gid, meta.gid);
        assert_eq!(parsed.signal, meta.signal);
        assert_eq!(parsed.timestamp, meta.timestamp);
        assert_eq!(parsed.hostname, meta.hostname);
        assert_eq!(parsed.comm, meta.comm);
        assert_eq!(parsed.exe, meta.exe);
        assert_eq!(parsed.backtrace, meta.backtrace);
        assert_eq!(parsed.core_size, meta.core_size);
        assert_eq!(parsed.filename, meta.filename);
        assert_eq!(parsed.boot_id, meta.boot_id);
        assert_eq!(parsed.machine_id, meta.machine_id);
        assert_eq!(parsed.cmdline, meta.cmdline);
        assert_eq!(parsed.cgroup, meta.cgroup);
        assert_eq!(parsed.environ, meta.environ);
    }

    #[test]
    fn test_json_unescape() {
        assert_eq!(json_unescape("hello"), "hello");
        assert_eq!(json_unescape("a\\\"b"), "a\"b");
        assert_eq!(json_unescape("a\\\\b"), "a\\b");
        assert_eq!(json_unescape("a\\nb"), "a\nb");
    }

    #[test]
    fn test_json_roundtrip_special_chars() {
        let meta = CoreDumpMeta {
            pid: 1,
            uid: 0,
            gid: 0,
            signal: 6,
            timestamp: 0,
            rlimit: 0,
            hostname: "host \"with\" quotes".into(),
            comm: "path/to\\prog".into(),
            exe: "/path/with\nnewline".into(),
            backtrace: true,
            core_size: 0,
            filename: String::new(),
            boot_id: String::new(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let json = meta.to_json();
        let parsed = CoreDumpMeta::from_json(&json).unwrap();
        assert_eq!(parsed.hostname, meta.hostname);
        assert_eq!(parsed.comm, meta.comm);
        assert_eq!(parsed.exe, meta.exe);
    }

    #[test]
    fn test_parse_json_invalid() {
        assert!(parse_json_object("not json").is_none());
        assert!(parse_json_object("").is_none());
    }

    // -- Metadata tests -----------------------------------------------------

    #[test]
    fn test_signal_name() {
        let mut meta = CoreDumpMeta {
            pid: 0,
            uid: 0,
            gid: 0,
            signal: 11,
            timestamp: 0,
            rlimit: 0,
            hostname: String::new(),
            comm: String::new(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: String::new(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        assert_eq!(meta.signal_name(), "SIGSEGV");
        meta.signal = 6;
        assert_eq!(meta.signal_name(), "SIGABRT");
        meta.signal = 8;
        assert_eq!(meta.signal_name(), "SIGFPE");
        meta.signal = 4;
        assert_eq!(meta.signal_name(), "SIGILL");
        meta.signal = 7;
        assert_eq!(meta.signal_name(), "SIGBUS");
        meta.signal = 5;
        assert_eq!(meta.signal_name(), "SIGTRAP");
        meta.signal = 3;
        assert_eq!(meta.signal_name(), "SIGQUIT");
        meta.signal = 99;
        assert_eq!(meta.signal_name(), "unknown");
    }

    // -- Filename tests -----------------------------------------------------

    #[test]
    fn test_build_filename_basic() {
        let meta = CoreDumpMeta {
            pid: 1234,
            uid: 1000,
            gid: 1000,
            signal: 11,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "myhost".into(),
            comm: "myapp".into(),
            exe: "/usr/bin/myapp".into(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "abc123".into(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let name = build_filename(&meta, Compression::None);
        assert_eq!(name, "core.myapp.1000.abc123.1234.1700000000");
    }

    #[test]
    fn test_build_filename_with_lz4() {
        let meta = CoreDumpMeta {
            pid: 1234,
            uid: 1000,
            gid: 1000,
            signal: 11,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "myhost".into(),
            comm: "myapp".into(),
            exe: "/usr/bin/myapp".into(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "abc123".into(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let name = build_filename(&meta, Compression::Lz4);
        assert_eq!(name, "core.myapp.1000.abc123.1234.1700000000.lz4");
    }

    #[test]
    fn test_build_filename_with_zstd() {
        let meta = CoreDumpMeta {
            pid: 1234,
            uid: 1000,
            gid: 1000,
            signal: 11,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "myhost".into(),
            comm: "myapp".into(),
            exe: "/usr/bin/myapp".into(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "abc123".into(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let name = build_filename(&meta, Compression::Zstd);
        assert_eq!(name, "core.myapp.1000.abc123.1234.1700000000.zst");
    }

    #[test]
    fn test_build_filename_with_xz() {
        let meta = CoreDumpMeta {
            pid: 1234,
            uid: 1000,
            gid: 1000,
            signal: 11,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "myhost".into(),
            comm: "myapp".into(),
            exe: "/usr/bin/myapp".into(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "abc123".into(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let name = build_filename(&meta, Compression::Xz);
        assert_eq!(name, "core.myapp.1000.abc123.1234.1700000000.xz");
    }

    #[test]
    fn test_build_filename_special_chars() {
        let meta = CoreDumpMeta {
            pid: 5,
            uid: 0,
            gid: 0,
            signal: 6,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "my app!@#".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "bid".into(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let name = build_filename(&meta, Compression::None);
        assert_eq!(name, "core.my_app___.0.bid.5.100");
    }

    // -- Argument parsing tests ---------------------------------------------

    #[test]
    fn test_parse_args_basic() {
        // Kernel core_pattern args: %P %u %g %s %t %c %h %d %F. The last two are
        // dumpable and pidfd; comm/exe are NOT in argv (they come from /proc).
        let args: Vec<String> = vec![
            "systemd-coredump",
            "1234",
            "1000",
            "1000",
            "11",
            "1700000000",
            "18446744073709551615",
            "myhost",
            "1", // %d dumpable
            "3", // %F pidfd
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.meta.pid, 1234);
        assert_eq!(parsed.meta.uid, 1000);
        assert_eq!(parsed.meta.gid, 1000);
        assert_eq!(parsed.meta.signal, 11);
        assert_eq!(parsed.meta.timestamp, 1700000000);
        assert_eq!(parsed.meta.rlimit, u64::MAX);
        assert_eq!(parsed.meta.hostname, "myhost");
        // comm/exe are populated later from /proc, not from argv.
        assert_eq!(parsed.meta.comm, "");
        assert_eq!(parsed.meta.exe, "");
        assert!(!parsed.meta.backtrace);
    }

    #[test]
    fn test_parse_args_with_backtrace() {
        let args: Vec<String> = vec![
            "systemd-coredump",
            "--backtrace",
            "42",
            "0",
            "0",
            "6",
            "100",
            "0",
            "localhost",
            "test",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let parsed = parse_args(&args).unwrap();
        assert!(parsed.meta.backtrace);
        assert_eq!(parsed.meta.pid, 42);
        assert_eq!(parsed.meta.signal, 6);
    }

    #[test]
    fn test_parse_args_no_exe() {
        let args: Vec<String> = vec![
            "systemd-coredump",
            "1",
            "0",
            "0",
            "11",
            "100",
            "0",
            "host",
            "comm",
        ]
        .into_iter()
        .map(String::from)
        .collect();

        let parsed = parse_args(&args).unwrap();
        assert_eq!(parsed.meta.exe, "");
    }

    #[test]
    fn test_parse_args_missing_args() {
        let args: Vec<String> = vec!["systemd-coredump"]
            .into_iter()
            .map(String::from)
            .collect();
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_invalid_pid() {
        let args: Vec<String> = vec![
            "systemd-coredump",
            "not-a-number",
            "0",
            "0",
            "11",
            "100",
            "0",
            "host",
            "comm",
        ]
        .into_iter()
        .map(String::from)
        .collect();
        assert!(parse_args(&args).is_err());
    }

    // -- Storage tests ------------------------------------------------------

    #[test]
    fn test_store_core_dump_basic() {
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("coredump");
        let config = Config {
            compress: false,
            ..Config::default()
        };

        let mut meta = CoreDumpMeta {
            pid: 100,
            uid: 0,
            gid: 0,
            signal: 11,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "test".into(),
            comm: "testapp".into(),
            exe: "/usr/bin/testapp".into(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "bootid".into(),
            machine_id: "machineid".into(),
            cmdline: "/usr/bin/testapp --flag".into(),
            cgroup: "0::/system.slice/test.service".into(),
            environ: "HOME=/root\nPATH=/usr/bin".into(),
        };

        let data = b"CORE DUMP DATA HERE";
        store_core_dump(&coredump_dir, &mut meta, data, &config).unwrap();

        assert!(!meta.filename.is_empty());
        assert_eq!(meta.core_size, data.len() as u64);

        // Check that the core file exists.
        let core_path = coredump_dir.join(&meta.filename);
        assert!(core_path.exists());
        assert_eq!(fs::read(&core_path).unwrap(), data);

        // Check that the metadata file exists.
        let meta_path = coredump_dir.join(format!("{}.json", meta.filename));
        assert!(meta_path.exists());

        // Parse the metadata back.
        let meta_json = fs::read_to_string(&meta_path).unwrap();
        let parsed = CoreDumpMeta::from_json(&meta_json).unwrap();
        assert_eq!(parsed.pid, 100);
        assert_eq!(parsed.comm, "testapp");
        assert_eq!(parsed.signal, 11);
    }

    #[test]
    fn test_store_core_dump_exceeds_external_max() {
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("coredump");
        let config = Config {
            external_size_max: 10, // tiny limit
            ..Config::default()
        };

        let mut meta = CoreDumpMeta {
            pid: 1,
            uid: 0,
            gid: 0,
            signal: 6,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "c".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "b".into(),
            machine_id: "m".into(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let data = b"THIS IS MORE THAN TEN BYTES OF DATA";
        store_core_dump(&coredump_dir, &mut meta, data, &config).unwrap();

        // Should NOT have stored anything (exceeds limit).
        assert!(meta.filename.is_empty());
    }

    #[test]
    fn test_store_creates_directory() {
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("deeply").join("nested").join("coredump");
        let config = Config {
            compress: false,
            ..Config::default()
        };

        let mut meta = CoreDumpMeta {
            pid: 1,
            uid: 0,
            gid: 0,
            signal: 6,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "c".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "b".into(),
            machine_id: "m".into(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        store_core_dump(&coredump_dir, &mut meta, b"data", &config).unwrap();
        assert!(coredump_dir.exists());
    }

    // -- Vacuum tests -------------------------------------------------------

    #[test]
    fn test_vacuum_removes_oldest() {
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("coredump");
        fs::create_dir_all(&coredump_dir).unwrap();

        // Create 3 core dumps totaling 300 bytes.
        for i in 0..3 {
            let name = format!("core.test.0.boot.{i}.{}", 1700000000 + i);
            fs::write(coredump_dir.join(&name), vec![b'X'; 100]).unwrap();

            let meta = CoreDumpMeta {
                pid: i as u64,
                uid: 0,
                gid: 0,
                signal: 6,
                timestamp: 1700000000 + i as u64,
                rlimit: 0,
                hostname: "h".into(),
                comm: "test".into(),
                exe: String::new(),
                backtrace: false,
                core_size: 100,
                filename: name.clone(),
                boot_id: "boot".into(),
                machine_id: "m".into(),
                cmdline: String::new(),
                cgroup: String::new(),
                environ: String::new(),
            };
            fs::write(coredump_dir.join(format!("{name}.json")), meta.to_json()).unwrap();
        }

        // Vacuum with a 150-byte limit should remove the two oldest.
        let config = Config {
            max_use: 150,
            keep_free: 0,
            ..Config::default()
        };
        vacuum(&coredump_dir, &config);

        let remaining: Vec<_> = fs::read_dir(&coredump_dir)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| {
                let name = e.file_name().to_string_lossy().to_string();
                name.starts_with("core.") && !name.ends_with(".json")
            })
            .collect();

        // Should have removed at least 1 (the oldest).
        assert!(remaining.len() < 3);
    }

    #[test]
    fn test_vacuum_empty_dir() {
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("coredump");
        fs::create_dir_all(&coredump_dir).unwrap();

        let config = Config {
            max_use: 0,
            ..Config::default()
        };

        // Should not panic.
        vacuum(&coredump_dir, &config);
    }

    #[test]
    fn test_vacuum_nonexistent_dir() {
        let config = Config::default();
        // Should not panic.
        vacuum(Path::new("/nonexistent/coredump"), &config);
    }

    // -- list_stored_coredumps tests ----------------------------------------

    #[test]
    fn test_list_stored_coredumps_empty() {
        let dir = TempDir::new().unwrap();
        let entries = list_stored_coredumps(dir.path());
        assert!(entries.is_empty());
    }

    #[test]
    fn test_list_stored_coredumps_with_entries() {
        let dir = TempDir::new().unwrap();

        for i in 0..3 {
            let name = format!("core.app.0.boot.{i}.{}", 1700000000 + i);
            fs::write(dir.path().join(&name), b"data").unwrap();
            fs::write(dir.path().join(format!("{name}.json")), b"{}").unwrap();
        }

        let entries = list_stored_coredumps(dir.path());
        assert_eq!(entries.len(), 3);
    }

    #[test]
    fn test_list_stored_coredumps_skips_non_core_files() {
        let dir = TempDir::new().unwrap();

        // A non-core file.
        fs::write(dir.path().join("random.json"), b"{}").unwrap();
        // A core file without .json shouldn't be listed either.
        fs::write(dir.path().join("core.x.0.b.1.100"), b"data").unwrap();

        let entries = list_stored_coredumps(dir.path());
        assert!(entries.is_empty());
    }

    // -- Integration test ---------------------------------------------------

    #[test]
    fn test_store_and_list_roundtrip() {
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("coredump");
        let config = Config {
            compress: false,
            ..Config::default()
        };

        for i in 0..3 {
            let mut meta = CoreDumpMeta {
                pid: 100 + i,
                uid: 1000,
                gid: 1000,
                signal: 11,
                timestamp: 1700000000 + i,
                rlimit: 0,
                hostname: "myhost".into(),
                comm: format!("app{i}"),
                exe: format!("/usr/bin/app{i}"),
                backtrace: false,
                core_size: 0,
                filename: String::new(),
                boot_id: format!("boot{i}"),
                machine_id: "machine".into(),
                cmdline: format!("/usr/bin/app{i} --run"),
                cgroup: "0::/user.slice".into(),
                environ: "LANG=C".into(),
            };

            let data = format!("core dump data {i}");
            store_core_dump(&coredump_dir, &mut meta, data.as_bytes(), &config).unwrap();
        }

        let entries = list_stored_coredumps(&coredump_dir);
        assert_eq!(entries.len(), 3);

        // Verify metadata can be read back.
        for (_core_path, meta_path, _) in &entries {
            let json = fs::read_to_string(meta_path).unwrap();
            let parsed = CoreDumpMeta::from_json(&json).unwrap();
            assert!(parsed.pid >= 100);
            assert_eq!(parsed.uid, 1000);
            assert_eq!(parsed.signal, 11);
        }
    }

    #[test]
    fn test_store_empty_data() {
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("coredump");
        let config = Config {
            compress: false,
            ..Config::default()
        };

        let mut meta = CoreDumpMeta {
            pid: 1,
            uid: 0,
            gid: 0,
            signal: 6,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "c".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "b".into(),
            machine_id: "m".into(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        // Empty data should still store (the file will be 0 bytes).
        store_core_dump(&coredump_dir, &mut meta, b"", &config).unwrap();
        assert!(!meta.filename.is_empty());
        assert_eq!(meta.core_size, 0);
    }

    #[test]
    fn test_config_storage_none_is_noop() {
        // Verify the Storage::None path (tested at the run() level in
        // integration, but we can verify the config parses correctly).
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nStorage=none\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::None);
    }

    #[test]
    fn test_parse_size_whitespace() {
        assert_eq!(parse_size("  1024  "), Some(1024));
        assert_eq!(parse_size("  100M  "), Some(100 * 1024 * 1024));
    }

    // -- Compression tests --------------------------------------------------

    #[test]
    fn test_compression_from_str() {
        assert_eq!(Compression::from_str("lz4"), Some(Compression::Lz4));
        assert_eq!(Compression::from_str("LZ4"), Some(Compression::Lz4));
        assert_eq!(Compression::from_str("zstd"), Some(Compression::Zstd));
        assert_eq!(Compression::from_str("zstandard"), Some(Compression::Zstd));
        assert_eq!(Compression::from_str("ZSTD"), Some(Compression::Zstd));
        assert_eq!(Compression::from_str("xz"), Some(Compression::Xz));
        assert_eq!(Compression::from_str("XZ"), Some(Compression::Xz));
        assert_eq!(Compression::from_str("lzma"), Some(Compression::Xz));
        assert_eq!(Compression::from_str("none"), Some(Compression::None));
        assert_eq!(Compression::from_str("no"), Some(Compression::None));
        assert_eq!(Compression::from_str("false"), Some(Compression::None));
        assert_eq!(Compression::from_str("0"), Some(Compression::None));
        assert_eq!(Compression::from_str("off"), Some(Compression::None));
        assert_eq!(Compression::from_str("unknown"), None);
    }

    #[test]
    fn test_compression_extension() {
        assert_eq!(Compression::None.extension(), "");
        assert_eq!(Compression::Lz4.extension(), ".lz4");
        assert_eq!(Compression::Zstd.extension(), ".zst");
        assert_eq!(Compression::Xz.extension(), ".xz");
    }

    #[test]
    fn test_compression_command_name() {
        assert_eq!(Compression::None.command_name(), "");
        assert_eq!(Compression::Lz4.command_name(), "lz4");
        assert_eq!(Compression::Zstd.command_name(), "zstd");
        assert_eq!(Compression::Xz.command_name(), "xz");
    }

    #[test]
    fn test_compression_as_str() {
        assert_eq!(Compression::None.as_str(), "none");
        assert_eq!(Compression::Lz4.as_str(), "lz4");
        assert_eq!(Compression::Zstd.as_str(), "zstd");
        assert_eq!(Compression::Xz.as_str(), "xz");
    }

    #[test]
    fn test_compress_data_none() {
        let data = b"hello world";
        let result = compress_data(data, Compression::None).unwrap();
        assert_eq!(result, data);
    }

    #[test]
    fn test_compress_data_lz4() {
        // This test requires lz4 to be installed.
        let data = b"hello world, this is some test data for compression";
        match compress_data(data, Compression::Lz4) {
            Ok(compressed) => {
                // Compressed data should be non-empty.
                assert!(!compressed.is_empty());
                // Verify roundtrip.
                let decompressed = decompress_data(&compressed, Compression::Lz4).unwrap();
                assert_eq!(decompressed, data);
            }
            Err(e) => {
                // lz4 not installed — skip test gracefully.
                eprintln!("Skipping lz4 test: {e}");
            }
        }
    }

    #[test]
    fn test_compress_data_zstd() {
        // This test requires zstd to be installed.
        let data = b"hello world, this is some test data for compression";
        match compress_data(data, Compression::Zstd) {
            Ok(compressed) => {
                assert!(!compressed.is_empty());
                let decompressed = decompress_data(&compressed, Compression::Zstd).unwrap();
                assert_eq!(decompressed, data);
            }
            Err(e) => {
                eprintln!("Skipping zstd test: {e}");
            }
        }
    }

    #[test]
    fn test_compress_data_xz() {
        // This test requires xz to be installed.
        let data = b"hello world, this is some test data for compression";
        match compress_data(data, Compression::Xz) {
            Ok(compressed) => {
                assert!(!compressed.is_empty());
                let decompressed = decompress_data(&compressed, Compression::Xz).unwrap();
                assert_eq!(decompressed, data);
            }
            Err(e) => {
                eprintln!("Skipping xz test: {e}");
            }
        }
    }

    #[test]
    fn test_compress_empty_data() {
        let data = b"";
        let result = compress_data(data, Compression::None).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn test_detect_compression() {
        assert_eq!(
            detect_compression("core.app.0.boot.1.123456"),
            Compression::None
        );
        assert_eq!(
            detect_compression("core.app.0.boot.1.123456.lz4"),
            Compression::Lz4
        );
        assert_eq!(
            detect_compression("core.app.0.boot.1.123456.zst"),
            Compression::Zstd
        );
        assert_eq!(
            detect_compression("core.app.0.boot.1.123456.xz"),
            Compression::Xz
        );
    }

    #[test]
    fn test_store_core_dump_with_compression() {
        // Test that compression works end-to-end in store_core_dump.
        // This test requires lz4 to be installed.
        let dir = TempDir::new().unwrap();
        let coredump_dir = dir.path().join("coredump");
        let config = Config {
            compress: true,
            compress_algorithm: Compression::Lz4,
            ..Config::default()
        };

        let mut meta = CoreDumpMeta {
            pid: 100,
            uid: 0,
            gid: 0,
            signal: 11,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "test".into(),
            comm: "testapp".into(),
            exe: "/usr/bin/testapp".into(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "bootid".into(),
            machine_id: "machineid".into(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let data = b"CORE DUMP DATA REPEATED ".repeat(100);
        store_core_dump(&coredump_dir, &mut meta, &data, &config).unwrap();

        if meta.filename.ends_with(".lz4") {
            // lz4 was available, verify compressed file exists.
            let core_path = coredump_dir.join(&meta.filename);
            assert!(core_path.exists());
            let stored = fs::read(&core_path).unwrap();
            // Compressed should be smaller than original for repetitive data.
            assert!(stored.len() < data.len());
        } else {
            // lz4 not available, stored uncompressed — still valid.
            assert!(!meta.filename.is_empty());
        }
    }

    // -- /proc/PID/ metadata enrichment tests --------------------------------

    #[test]
    fn test_read_proc_cmdline_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cmdline");
        // Kernel stores cmdline with NUL separators.
        fs::write(&path, b"/usr/bin/myapp\0--flag\0arg1\0").unwrap();

        let result = read_proc_cmdline_from(path.to_str().unwrap());
        assert_eq!(result, "/usr/bin/myapp --flag arg1");
    }

    #[test]
    fn test_read_proc_cmdline_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cmdline");
        fs::write(&path, b"").unwrap();

        let result = read_proc_cmdline_from(path.to_str().unwrap());
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_proc_cmdline_nonexistent() {
        let result = read_proc_cmdline_from("/nonexistent/cmdline");
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_proc_cmdline_no_trailing_nul() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cmdline");
        fs::write(&path, b"/usr/bin/app\0arg1").unwrap();

        let result = read_proc_cmdline_from(path.to_str().unwrap());
        assert_eq!(result, "/usr/bin/app arg1");
    }

    #[test]
    fn test_read_proc_cgroup_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cgroup");
        fs::write(&path, "0::/system.slice/sshd.service\n").unwrap();

        let result = read_proc_cgroup_from(path.to_str().unwrap());
        assert_eq!(result, "0::/system.slice/sshd.service");
    }

    #[test]
    fn test_read_proc_cgroup_v1_multiple_hierarchies() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cgroup");
        fs::write(
            &path,
            "12:cpuset:/\n11:memory:/user.slice\n0::/user.slice/user-1000.slice\n",
        )
        .unwrap();

        let result = read_proc_cgroup_from(path.to_str().unwrap());
        assert!(result.contains("cpuset:/"));
        assert!(result.contains("memory:/user.slice"));
        assert!(result.contains("0::/user.slice"));
    }

    #[test]
    fn test_read_proc_cgroup_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("cgroup");
        fs::write(&path, "").unwrap();

        let result = read_proc_cgroup_from(path.to_str().unwrap());
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_proc_cgroup_nonexistent() {
        let result = read_proc_cgroup_from("/nonexistent/cgroup");
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_proc_environ_basic() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("environ");
        // Kernel stores environ with NUL separators.
        fs::write(&path, b"HOME=/home/user\0PATH=/usr/bin\0LANG=C\0").unwrap();

        let result = read_proc_environ_from(path.to_str().unwrap());
        assert_eq!(result, "HOME=/home/user\nPATH=/usr/bin\nLANG=C");
    }

    #[test]
    fn test_read_proc_environ_empty() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("environ");
        fs::write(&path, b"").unwrap();

        let result = read_proc_environ_from(path.to_str().unwrap());
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_proc_environ_nonexistent() {
        let result = read_proc_environ_from("/nonexistent/environ");
        assert_eq!(result, "");
    }

    #[test]
    fn test_read_proc_environ_single_var() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("environ");
        fs::write(&path, b"HOME=/root\0").unwrap();

        let result = read_proc_environ_from(path.to_str().unwrap());
        assert_eq!(result, "HOME=/root");
    }

    #[test]
    fn test_enrich_from_proc_paths() {
        let dir = TempDir::new().unwrap();

        let cmdline_path = dir.path().join("cmdline");
        fs::write(&cmdline_path, b"/usr/bin/crash\0--boom\0").unwrap();

        let cgroup_path = dir.path().join("cgroup");
        fs::write(&cgroup_path, "0::/system.slice/crash.service\n").unwrap();

        let environ_path = dir.path().join("environ");
        fs::write(&environ_path, b"HOME=/root\0TERM=xterm\0").unwrap();

        let mut meta = CoreDumpMeta {
            pid: 42,
            uid: 0,
            gid: 0,
            signal: 11,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "crash".into(),
            exe: "/usr/bin/crash".into(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "b".into(),
            machine_id: "m".into(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        enrich_from_proc_paths(
            &mut meta,
            cmdline_path.to_str().unwrap(),
            cgroup_path.to_str().unwrap(),
            environ_path.to_str().unwrap(),
        );

        assert_eq!(meta.cmdline, "/usr/bin/crash --boom");
        assert_eq!(meta.cgroup, "0::/system.slice/crash.service");
        assert_eq!(meta.environ, "HOME=/root\nTERM=xterm");
    }

    #[test]
    fn test_enrich_from_proc_paths_missing_files() {
        let mut meta = CoreDumpMeta {
            pid: 42,
            uid: 0,
            gid: 0,
            signal: 11,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "crash".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: "b".into(),
            machine_id: "m".into(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        enrich_from_proc_paths(
            &mut meta,
            "/nonexistent/cmdline",
            "/nonexistent/cgroup",
            "/nonexistent/environ",
        );

        assert_eq!(meta.cmdline, "");
        assert_eq!(meta.cgroup, "");
        assert_eq!(meta.environ, "");
    }

    #[test]
    fn test_json_roundtrip_with_proc_metadata() {
        let meta = CoreDumpMeta {
            pid: 555,
            uid: 1000,
            gid: 1000,
            signal: 6,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "host".into(),
            comm: "app".into(),
            exe: "/usr/bin/app".into(),
            backtrace: false,
            core_size: 4096,
            filename: "core.app.1000.boot.555.1700000000".into(),
            boot_id: "boot".into(),
            machine_id: "machine".into(),
            cmdline: "/usr/bin/app --verbose --config=/etc/app.conf".into(),
            cgroup: "0::/user.slice/user-1000.slice/session-1.scope".into(),
            environ: "HOME=/home/user\nPATH=/usr/bin:/bin\nSHELL=/bin/bash".into(),
        };

        let json = meta.to_json();
        let parsed = CoreDumpMeta::from_json(&json).unwrap();

        assert_eq!(parsed.cmdline, meta.cmdline);
        assert_eq!(parsed.cgroup, meta.cgroup);
        assert_eq!(parsed.environ, meta.environ);
    }

    // -- Journal integration tests ------------------------------------------

    #[test]
    fn test_build_journal_entry_basic() {
        let meta = CoreDumpMeta {
            pid: 1234,
            uid: 1000,
            gid: 1000,
            signal: 11,
            timestamp: 1700000000,
            rlimit: 0,
            hostname: "testhost".into(),
            comm: "testapp".into(),
            exe: "/usr/bin/testapp".into(),
            backtrace: false,
            core_size: 4096,
            filename: "core.testapp.1000.boot.1234.1700000000".into(),
            boot_id: "abc123".into(),
            machine_id: "def456".into(),
            cmdline: "/usr/bin/testapp --run".into(),
            cgroup: "0::/system.slice/test.service".into(),
            environ: "HOME=/home/user".into(),
        };

        let msg = build_journal_entry(&meta);
        let msg_str = String::from_utf8_lossy(&msg);

        // Verify key fields are present.
        assert!(msg_str.contains("MESSAGE=Process 1234 (testapp)"));
        assert!(msg_str.contains(&format!("MESSAGE_ID={COREDUMP_MESSAGE_ID}")));
        assert!(msg_str.contains("PRIORITY=2"));
        assert!(msg_str.contains("COREDUMP_PID=1234"));
        assert!(msg_str.contains("COREDUMP_UID=1000"));
        assert!(msg_str.contains("COREDUMP_GID=1000"));
        assert!(msg_str.contains("COREDUMP_SIGNAL=11"));
        assert!(msg_str.contains("COREDUMP_SIGNAL_NAME=SIGSEGV"));
        assert!(msg_str.contains("COREDUMP_HOSTNAME=testhost"));
        assert!(msg_str.contains("COREDUMP_COMM=testapp"));
        assert!(msg_str.contains("COREDUMP_EXE=/usr/bin/testapp"));
        assert!(msg_str.contains("COREDUMP_BOOT_ID=abc123"));
        assert!(msg_str.contains("COREDUMP_MACHINE_ID=def456"));
        assert!(msg_str.contains("COREDUMP_SIZE=4096"));
        assert!(msg_str.contains("COREDUMP_CMDLINE=/usr/bin/testapp --run"));
    }

    #[test]
    fn test_build_journal_entry_empty_optional_fields() {
        let meta = CoreDumpMeta {
            pid: 1,
            uid: 0,
            gid: 0,
            signal: 6,
            timestamp: 100,
            rlimit: 0,
            hostname: String::new(),
            comm: String::new(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: String::new(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        let msg = build_journal_entry(&meta);
        let msg_str = String::from_utf8_lossy(&msg);

        // Required fields should be present.
        assert!(msg_str.contains("COREDUMP_PID=1"));
        assert!(msg_str.contains("COREDUMP_SIGNAL=6"));

        // Optional empty fields should NOT be present.
        assert!(!msg_str.contains("COREDUMP_HOSTNAME="));
        assert!(!msg_str.contains("COREDUMP_COMM="));
        assert!(!msg_str.contains("COREDUMP_EXE="));
        assert!(!msg_str.contains("COREDUMP_CMDLINE="));
        assert!(!msg_str.contains("COREDUMP_SIZE="));
    }

    #[test]
    fn test_build_journal_entry_binary_safe_cgroup() {
        let meta = CoreDumpMeta {
            pid: 42,
            uid: 0,
            gid: 0,
            signal: 11,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "c".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: String::new(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: "12:cpuset:/\n0::/user.slice".into(),
            environ: String::new(),
        };

        let msg = build_journal_entry(&meta);

        // Cgroup with newlines should use binary-safe encoding.
        // Binary format: KEY\n<8-byte LE length><data>\n
        // Check that COREDUMP_CGROUP appears in the message.
        let msg_bytes = &msg;
        let key = b"COREDUMP_CGROUP";
        let pos = msg_bytes
            .windows(key.len())
            .position(|w| w == key)
            .expect("COREDUMP_CGROUP not found");

        // After the key, the next byte should be \n (binary format).
        assert_eq!(msg_bytes[pos + key.len()], b'\n');

        // Then 8 bytes of LE length.
        let len_bytes = &msg_bytes[pos + key.len() + 1..pos + key.len() + 9];
        let data_len = u64::from_le_bytes(len_bytes.try_into().unwrap());
        assert_eq!(data_len as usize, meta.cgroup.len());
    }

    #[test]
    fn test_build_journal_entry_binary_safe_environ() {
        let meta = CoreDumpMeta {
            pid: 42,
            uid: 0,
            gid: 0,
            signal: 11,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "c".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: String::new(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: "HOME=/root\nPATH=/usr/bin".into(),
        };

        let msg = build_journal_entry(&meta);
        let msg_bytes = &msg;

        // Environ contains newlines, should use binary encoding.
        let key = b"COREDUMP_ENVIRON";
        let pos = msg_bytes
            .windows(key.len())
            .position(|w| w == key)
            .expect("COREDUMP_ENVIRON not found");

        assert_eq!(msg_bytes[pos + key.len()], b'\n');

        let len_bytes = &msg_bytes[pos + key.len() + 1..pos + key.len() + 9];
        let data_len = u64::from_le_bytes(len_bytes.try_into().unwrap());
        assert_eq!(data_len as usize, meta.environ.len());
    }

    #[test]
    fn test_write_journal_field_binary() {
        let mut msg = Vec::new();
        write_journal_field_binary(&mut msg, "TEST_FIELD", b"hello\nworld");

        // Should be: TEST_FIELD\n<8 bytes LE len>hello\nworld\n
        assert!(msg.starts_with(b"TEST_FIELD\n"));
        let len_start = "TEST_FIELD\n".len();
        let len_bytes = &msg[len_start..len_start + 8];
        let data_len = u64::from_le_bytes(len_bytes.try_into().unwrap());
        assert_eq!(data_len, 11); // "hello\nworld".len()

        let data = &msg[len_start + 8..len_start + 8 + 11];
        assert_eq!(data, b"hello\nworld");
        assert_eq!(msg[len_start + 8 + 11], b'\n');
    }

    #[test]
    fn test_send_to_journal_at_nonexistent_socket() {
        // Should not panic, just print a warning to stderr.
        let meta = CoreDumpMeta {
            pid: 1,
            uid: 0,
            gid: 0,
            signal: 6,
            timestamp: 100,
            rlimit: 0,
            hostname: "h".into(),
            comm: "c".into(),
            exe: String::new(),
            backtrace: false,
            core_size: 0,
            filename: String::new(),
            boot_id: String::new(),
            machine_id: String::new(),
            cmdline: String::new(),
            cgroup: String::new(),
            environ: String::new(),
        };

        send_to_journal_at("/nonexistent/journal/socket", &meta);
        // No panic = success.
    }

    #[test]
    fn test_coredump_message_id_format() {
        // MESSAGE_ID should be a 32-char hex string (128-bit UUID without dashes).
        assert_eq!(COREDUMP_MESSAGE_ID.len(), 32);
        assert!(COREDUMP_MESSAGE_ID.chars().all(|c| c.is_ascii_hexdigit()));
    }

    // -- Config storage mode tests ------------------------------------------

    #[test]
    fn test_config_storage_journal() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nStorage=journal\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::Journal);
    }

    #[test]
    fn test_config_storage_both() {
        let dir = TempDir::new().unwrap();
        let conf = dir.path().join("coredump.conf");
        fs::write(&conf, "[Coredump]\nStorage=both\n").unwrap();
        let mut config = Config::default();
        parse_config_file(&conf, &mut config);
        assert_eq!(config.storage, Storage::Both);
    }
}
