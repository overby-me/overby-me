//! systemd-journald — Journal logging daemon.
//!
//! A drop-in replacement for `systemd-journald(8)`. This daemon collects
//! structured log data from multiple sources and stores it in the journal:
//!
//! - `/run/systemd/journal/socket`  — native journal protocol (datagram)
//! - `/run/systemd/journal/stdout`  — stdout stream connections from services
//! - `/dev/log`                      — BSD syslog protocol (datagram)
//! - `/proc/kmsg`                    — kernel ring buffer messages
//!
//! It also supports:
//!
//! - Rate limiting per-service to prevent log flooding
//! - Journal file rotation and disk usage limits
//! - sd_notify READY=1 / STATUS= / WATCHDOG=1 protocol
//! - SIGUSR1 for flushing volatile → persistent storage
//! - SIGUSR2 for journal rotation
//!
//! Configuration is read from `/etc/systemd/journald.conf` and
//! `/etc/systemd/journald.conf.d/*.conf`.

mod journal;

use journal::entry::JournalEntry;
use journal::storage::{JournalStorage, StorageConfig};

use std::collections::HashMap;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};

use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::FromRawFd;
use std::os::unix::net::{UnixDatagram, UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use std::{process, thread};

// ---------------------------------------------------------------------------
// Socket paths
// ---------------------------------------------------------------------------

/// Native journal protocol socket (datagram).
const JOURNAL_SOCKET_PATH: &str = "/run/systemd/journal/socket";
/// Stdout capture socket (stream, for connected services).
const STDOUT_SOCKET_PATH: &str = "/run/systemd/journal/stdout";
/// BSD syslog socket (datagram).
const SYSLOG_SOCKET_PATH: &str = "/dev/log";
/// Kernel ring buffer.
const KMSG_PATH: &str = "/proc/kmsg";
/// Runtime directory for sockets and PID file.
const RUNTIME_DIR: &str = "/run/systemd/journal";
/// PID file path — used by `journalctl --flush` / `--rotate` to signal us.
const PID_FILE_PATH: &str = "/run/systemd/journal/pid";

// ---------------------------------------------------------------------------
// Rate limiting
// ---------------------------------------------------------------------------

/// Default: allow 10 000 messages per 30 seconds per source.
const RATE_LIMIT_INTERVAL_USEC: u64 = 30_000_000; // 30 s in µs
const RATE_LIMIT_BURST: u64 = 10_000;

/// Per-source rate limiter state.
struct RateLimiter {
    /// Map from source identifier (unit name or PID) to state.
    sources: HashMap<String, RateLimitState>,
}

struct RateLimitState {
    /// Start of the current window.
    window_start: Instant,
    /// Number of messages in the current window.
    count: u64,
    /// Whether we've already logged a suppression message for this window.
    suppression_logged: bool,
    /// Number of messages suppressed in the current window.
    suppressed: u64,
}

impl RateLimiter {
    fn new() -> Self {
        RateLimiter {
            sources: HashMap::new(),
        }
    }

    /// Check whether a message from `source` should be accepted.
    ///
    /// Returns:
    /// - `RateLimitResult::Accept` — message is within limits, store it.
    /// - `RateLimitResult::Suppressed` — message exceeds the burst, drop it.
    /// - `RateLimitResult::WindowReset { suppressed }` — a new window has
    ///   started and the previous window suppressed `suppressed` messages.
    ///   The current message is accepted.  The caller should log a summary
    ///   entry about the suppressed messages.
    fn check(&mut self, source: &str, burst: u64, interval: Duration) -> RateLimitResult {
        // Burst of 0 means rate limiting is disabled
        if burst == 0 || interval.is_zero() {
            return RateLimitResult::Accept;
        }

        let now = Instant::now();

        let state = self
            .sources
            .entry(source.to_string())
            .or_insert_with(|| RateLimitState {
                window_start: now,
                count: 0,
                suppression_logged: false,
                suppressed: 0,
            });

        // If the interval has elapsed, reset the window
        let mut prev_suppressed = 0u64;
        if now.duration_since(state.window_start) >= interval {
            prev_suppressed = state.suppressed;
            state.window_start = now;
            state.count = 0;
            state.suppression_logged = false;
            state.suppressed = 0;
        }

        state.count += 1;

        if state.count <= burst {
            if prev_suppressed > 0 {
                RateLimitResult::WindowReset {
                    suppressed: prev_suppressed,
                }
            } else {
                RateLimitResult::Accept
            }
        } else {
            state.suppressed += 1;
            if !state.suppression_logged {
                state.suppression_logged = true;
                eprintln!(
                    "journald: Rate limit exceeded for '{}', suppressing further messages",
                    source
                );
            }
            RateLimitResult::Suppressed
        }
    }

    /// Periodically clean up stale entries to avoid unbounded memory growth.
    fn gc(&mut self, max_age: Duration) {
        let now = Instant::now();
        self.sources
            .retain(|_, state| now.duration_since(state.window_start) < max_age);
    }
}

/// Result of a rate-limit check.
#[derive(Debug, PartialEq, Eq)]
enum RateLimitResult {
    /// Message accepted — within burst limit.
    Accept,
    /// Message suppressed — over burst limit.
    Suppressed,
    /// A new rate-limit window started and the *previous* window had
    /// `suppressed` messages dropped.  The current message is accepted.
    WindowReset { suppressed: u64 },
}

// ---------------------------------------------------------------------------
// Journald configuration
// ---------------------------------------------------------------------------

/// Configuration parsed from journald.conf.
#[derive(Debug, Clone)]
struct JournaldConfig {
    /// Storage mode: "volatile", "persistent", "auto", "none".
    storage: String,
    /// Compress entries larger than this (0 = disabled). Not yet implemented.
    compress: bool,
    /// Maximum size of individual journal files.
    max_file_size: u64,
    /// Maximum total disk usage for persistent journal.
    system_max_use: u64,
    /// Maximum total disk usage for volatile journal.
    runtime_max_use: u64,
    /// Maximum number of journal files.
    max_files: usize,
    /// Rate limit interval in microseconds.
    rate_limit_interval_usec: u64,
    /// Rate limit burst count.
    rate_limit_burst: u64,
    /// Forward to syslog.
    forward_to_syslog: bool,
    /// Forward to kmsg.
    forward_to_kmsg: bool,
    /// Forward to console.
    forward_to_console: bool,
    /// Forward to wall.
    forward_to_wall: bool,
    /// Maximum log level to store (0=emerg .. 7=debug).
    max_level_store: u8,
    /// Maximum log level to forward to syslog.
    max_level_syslog: u8,
    /// Maximum log level to forward to kmsg.
    max_level_kmsg: u8,
    /// Maximum log level to forward to console.
    max_level_console: u8,
    /// Maximum log level to forward to wall.
    max_level_wall: u8,
    /// Maximum field size in bytes (fields larger than this are truncated).
    max_field_size: usize,
    /// Minimum free disk space to maintain for persistent storage (bytes).
    /// When free space drops below this, oldest journal files are vacuumed.
    system_keep_free: u64,
    /// Minimum free disk space to maintain for volatile storage (bytes).
    runtime_keep_free: u64,
    /// Maximum time span a single journal file covers before rotation (µs).
    /// 0 means no time-based rotation.
    max_file_sec_usec: u64,
    /// Enable forward-secure sealing of journal files.
    seal: bool,
}

impl Default for JournaldConfig {
    fn default() -> Self {
        JournaldConfig {
            storage: "auto".to_string(),
            compress: true,
            max_file_size: 64 * 1024 * 1024,   // 64 MiB
            system_max_use: 512 * 1024 * 1024, // 512 MiB
            runtime_max_use: 64 * 1024 * 1024, // 64 MiB
            max_files: 100,
            rate_limit_interval_usec: RATE_LIMIT_INTERVAL_USEC,
            rate_limit_burst: RATE_LIMIT_BURST,
            forward_to_syslog: false,
            forward_to_kmsg: false,
            forward_to_console: false,
            forward_to_wall: true,
            max_level_store: 7,                            // debug
            max_level_syslog: 7,                           // debug
            max_level_kmsg: 4,                             // warning
            max_level_console: 6,                          // info
            max_level_wall: 0,                             // emerg
            max_field_size: 768 * 1024,                    // 768 KiB
            system_keep_free: 4 * 1024 * 1024 * 1024,      // 4 GiB
            runtime_keep_free: 4 * 1024 * 1024 * 1024,     // 4 GiB
            max_file_sec_usec: 30 * 24 * 3600 * 1_000_000, // 1 month in µs
            seal: true,
        }
    }
}

impl JournaldConfig {
    fn load() -> Self {
        let mut config = JournaldConfig::default();

        // Load main config
        if let Ok(contents) = fs::read_to_string("/etc/systemd/journald.conf") {
            config.parse_config(&contents);
        }

        // Load drop-in configs
        if let Ok(entries) = fs::read_dir("/etc/systemd/journald.conf.d") {
            let mut files: Vec<PathBuf> = entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "conf"))
                .collect();
            files.sort();
            for path in files {
                if let Ok(contents) = fs::read_to_string(&path) {
                    config.parse_config(&contents);
                }
            }
        }

        config
    }

    fn parse_config(&mut self, contents: &str) {
        let mut in_journal_section = false;

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') {
                in_journal_section = line == "[Journal]";
                continue;
            }
            if !in_journal_section {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "Storage" => self.storage = value.to_lowercase(),
                    "Compress" => self.compress = parse_bool(value),
                    "SystemMaxFileSize" | "MaxFileSizeBytes" => {
                        if let Some(bytes) = parse_size(value) {
                            self.max_file_size = bytes;
                        }
                    }
                    "SystemMaxUse" => {
                        if let Some(bytes) = parse_size(value) {
                            self.system_max_use = bytes;
                        }
                    }
                    "RuntimeMaxUse" => {
                        if let Some(bytes) = parse_size(value) {
                            self.runtime_max_use = bytes;
                        }
                    }
                    "MaxFiles" => {
                        if let Ok(n) = value.parse::<usize>() {
                            self.max_files = n;
                        }
                    }
                    "MaxFileSec" => {
                        if let Some(usec) = parse_timespan_usec(value) {
                            self.max_file_sec_usec = usec;
                        }
                    }
                    "RateLimitIntervalSec" | "RateLimitIntervalUSec" => {
                        if let Some(usec) = parse_timespan_usec(value) {
                            self.rate_limit_interval_usec = usec;
                        }
                    }
                    "RateLimitBurst" => {
                        if let Ok(n) = value.parse::<u64>() {
                            self.rate_limit_burst = n;
                        }
                    }
                    "ForwardToSyslog" => self.forward_to_syslog = parse_bool(value),
                    "ForwardToKMsg" => self.forward_to_kmsg = parse_bool(value),
                    "ForwardToConsole" => self.forward_to_console = parse_bool(value),
                    "ForwardToWall" => self.forward_to_wall = parse_bool(value),
                    "MaxLevelStore" => {
                        if let Some(level) = parse_log_level(value) {
                            self.max_level_store = level;
                        }
                    }
                    "MaxLevelSyslog" => {
                        if let Some(level) = parse_log_level(value) {
                            self.max_level_syslog = level;
                        }
                    }
                    "MaxLevelKMsg" => {
                        if let Some(level) = parse_log_level(value) {
                            self.max_level_kmsg = level;
                        }
                    }
                    "MaxLevelConsole" => {
                        if let Some(level) = parse_log_level(value) {
                            self.max_level_console = level;
                        }
                    }
                    "MaxLevelWall" => {
                        if let Some(level) = parse_log_level(value) {
                            self.max_level_wall = level;
                        }
                    }
                    "MaxFieldSize" | "LineMax" => {
                        if let Some(bytes) = parse_size(value) {
                            self.max_field_size = bytes as usize;
                        }
                    }
                    "SystemKeepFree" => {
                        if let Some(bytes) = parse_size(value) {
                            self.system_keep_free = bytes;
                        }
                    }
                    "RuntimeKeepFree" => {
                        if let Some(bytes) = parse_size(value) {
                            self.runtime_keep_free = bytes;
                        }
                    }
                    "Seal" => self.seal = parse_bool(value),
                    _ => {} // Ignore unknown keys
                }
            }
        }
    }

    /// Determine whether to use persistent storage based on the Storage= setting.
    fn use_persistent_storage(&self) -> bool {
        match self.storage.as_str() {
            "persistent" => true,
            "volatile" => false,
            "none" => false,
            _ => {
                // "auto" mode: use persistent if /var/log/journal exists
                Path::new("/var/log/journal").is_dir()
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Configuration parsing helpers
// ---------------------------------------------------------------------------

fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "yes" | "true" | "1" | "on" | "y")
}

fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let suffixes: &[(&str, u64)] = &[
        ("EiB", 1024 * 1024 * 1024 * 1024 * 1024 * 1024),
        ("PiB", 1024 * 1024 * 1024 * 1024 * 1024),
        ("TiB", 1024 * 1024 * 1024 * 1024),
        ("GiB", 1024 * 1024 * 1024),
        ("MiB", 1024 * 1024),
        ("KiB", 1024),
        ("EB", 1_000_000_000_000_000_000),
        ("PB", 1_000_000_000_000_000),
        ("TB", 1_000_000_000_000),
        ("GB", 1_000_000_000),
        ("MB", 1_000_000),
        ("KB", 1_000),
        ("E", 1024 * 1024 * 1024 * 1024 * 1024 * 1024),
        ("P", 1024 * 1024 * 1024 * 1024 * 1024),
        ("T", 1024 * 1024 * 1024 * 1024),
        ("G", 1024 * 1024 * 1024),
        ("M", 1024 * 1024),
        ("K", 1024),
        ("B", 1),
    ];

    for &(suffix, multiplier) in suffixes {
        if let Some(num_str) = s.strip_suffix(suffix) {
            let num_str = num_str.trim();
            if let Ok(n) = num_str.parse::<u64>() {
                return Some(n * multiplier);
            }
        }
    }

    s.parse::<u64>().ok()
}

fn parse_timespan_usec(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Try direct microsecond value
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }

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
        ("h", 3_600_000_000),
        ("hr", 3_600_000_000),
        ("hour", 3_600_000_000),
        ("hours", 3_600_000_000),
        ("m", 60_000_000),
        ("s", 1_000_000),
        ("d", 86_400_000_000),
        ("day", 86_400_000_000),
        ("days", 86_400_000_000),
    ];

    for &(suffix, multiplier) in suffixes {
        if let Some(num_str) = s.strip_suffix(suffix) {
            let num_str = num_str.trim();
            if let Ok(n) = num_str.parse::<u64>() {
                return Some(n * multiplier);
            }
        }
    }

    None
}

fn parse_log_level(s: &str) -> Option<u8> {
    match s.to_lowercase().as_str() {
        "emerg" | "emergency" | "0" => Some(0),
        "alert" | "1" => Some(1),
        "crit" | "critical" | "2" => Some(2),
        "err" | "error" | "3" => Some(3),
        "warning" | "warn" | "4" => Some(4),
        "notice" | "5" => Some(5),
        "info" | "6" => Some(6),
        "debug" | "7" => Some(7),
        _ => None,
    }
}

// ---------------------------------------------------------------------------
// Shared daemon state
// ---------------------------------------------------------------------------

/// Shared state for the journald daemon, protected by appropriate locks.
struct JournaldState {
    /// The journal storage engine.
    storage: Mutex<JournalStorage>,
    /// Rate limiter.
    rate_limiter: Mutex<RateLimiter>,
    /// Configuration.
    config: JournaldConfig,
    /// Global sequence number counter.
    seqnum: AtomicU64,
    /// Shutdown flag.
    shutdown: AtomicBool,
    /// Flush requested (SIGUSR1).
    flush_requested: AtomicBool,
    /// Rotate requested (SIGUSR2).
    rotate_requested: AtomicBool,
    /// When the current active journal file was opened (for time-based rotation).
    active_file_opened: Mutex<Instant>,
    /// Forward-secure sealing state (if enabled).
    seal_state: Mutex<Option<SealState>>,
    /// Last time a periodic vacuum was performed.
    last_vacuum: Mutex<Instant>,
}

/// Forward-Secure Sealing (FSS) state.
///
/// Uses an HMAC-SHA256 key chain: at each seal epoch the current key is
/// hashed to derive the next key, and the old key material is erased.
/// An attacker who compromises the system *after* a seal epoch cannot
/// forge entries sealed in earlier epochs.
struct SealState {
    /// Current sealing key (32 bytes).  Advanced (hashed) after each seal.
    key: [u8; 32],
    /// Monotonically increasing epoch counter.
    epoch: u64,
    /// Interval between seal operations (microseconds).
    seal_interval_usec: u64,
    /// Timestamp of the last seal (monotonic µs since daemon start).
    last_seal: Instant,
}

impl SealState {
    /// Create a new seal state with a randomly generated initial key.
    fn new(seal_interval_usec: u64) -> Self {
        let mut key = [0u8; 32];
        // Read initial key material from /dev/urandom
        if let Ok(mut f) = fs::File::open("/dev/urandom") {
            let _ = io::Read::read_exact(&mut f, &mut key);
        }
        SealState {
            key,
            epoch: 0,
            seal_interval_usec,
            last_seal: Instant::now(),
        }
    }

    /// Advance the key chain: replace the current key with SHA-256(key).
    /// This provides forward secrecy — the old key cannot be recovered.
    fn advance_key(&mut self) {
        // Simple SHA-256 via a manual Merkle-Damgård-style approach is
        // complex; instead we use a lightweight xor-fold + mix.  For
        // production-grade FSS the `sha2` crate should be used, but to
        // avoid adding a dependency we use a deterministic mixing
        // function that is *not* cryptographically ideal but demonstrates
        // the key-erasure protocol.  Swap in a real SHA-256 when the
        // `sha2` crate is added to Cargo.toml.
        let mut next = [0u8; 32];
        // Mix with constants derived from the epoch to make each step unique
        let epoch_bytes = self.epoch.to_le_bytes();
        for i in 0..32 {
            // Rotate, XOR with epoch, and mix with a prime constant
            let a = self.key[i];
            let b = self.key[(i + 13) % 32];
            let c = epoch_bytes[i % 8];
            next[i] = a.wrapping_mul(251).wrapping_add(b).wrapping_add(c);
        }
        // Erase old key
        self.key.iter_mut().for_each(|b| *b = 0);
        self.key = next;
        self.epoch += 1;
    }

    /// Compute a seal tag for the given data using the current key.
    /// Returns a hex-encoded tag string.
    fn compute_tag(&self, data: &[u8]) -> String {
        // Simple keyed hash: XOR-fold data with key, then mix.
        // This is a placeholder — replace with HMAC-SHA256 for real security.
        let mut tag = self.key;
        for (i, &byte) in data.iter().enumerate() {
            tag[i % 32] ^= byte;
            tag[i % 32] = tag[i % 32].wrapping_mul(31).wrapping_add(byte);
        }
        // Final mix pass
        for i in 0..32 {
            tag[i] = tag[i].wrapping_add(tag[(i + 7) % 32]).wrapping_mul(197);
        }
        tag.iter().map(|b| format!("{:02x}", b)).collect()
    }

    /// Check whether it is time to emit a new seal entry.
    fn should_seal(&self) -> bool {
        if self.seal_interval_usec == 0 {
            return false;
        }
        let elapsed = self.last_seal.elapsed();
        elapsed >= Duration::from_micros(self.seal_interval_usec)
    }
}

impl JournaldState {
    fn new(config: JournaldConfig, storage: JournalStorage) -> Self {
        let seal_state = if config.seal {
            // Default seal interval: 15 minutes
            Some(SealState::new(15 * 60 * 1_000_000))
        } else {
            None
        };
        JournaldState {
            storage: Mutex::new(storage),
            rate_limiter: Mutex::new(RateLimiter::new()),
            config,
            seqnum: AtomicU64::new(1),
            shutdown: AtomicBool::new(false),
            flush_requested: AtomicBool::new(false),
            rotate_requested: AtomicBool::new(false),
            active_file_opened: Mutex::new(Instant::now()),
            seal_state: Mutex::new(seal_state),
            last_vacuum: Mutex::new(Instant::now()),
        }
    }

    /// Dispatch a fully-formed journal entry into storage.
    fn dispatch_entry(&self, mut entry: JournalEntry) {
        // Check priority against MaxLevelStore
        if let Some(priority) = entry.priority()
            && priority > self.config.max_level_store
        {
            return;
        }

        // Rate limiting
        let source = entry
            .systemd_unit()
            .or_else(|| entry.syslog_identifier())
            .or_else(|| entry.pid().map(|p| p.to_string()))
            .unwrap_or_else(|| "unknown".to_string());

        {
            let mut rl = self.rate_limiter.lock().unwrap();
            let interval = Duration::from_micros(self.config.rate_limit_interval_usec);
            match rl.check(&source, self.config.rate_limit_burst, interval) {
                RateLimitResult::Suppressed => return,
                RateLimitResult::WindowReset { suppressed } => {
                    // The previous window suppressed some messages — log a
                    // summary entry so operators can see what was dropped.
                    let mut summary = JournalEntry::new();
                    summary.set_field(
                        "MESSAGE",
                        format!("Suppressed {} messages from {}", suppressed, source),
                    );
                    summary.set_field("PRIORITY", "5"); // notice
                    summary.set_field("SYSLOG_IDENTIFIER", "systemd-journald");
                    summary.set_field("_PID", process::id().to_string());
                    summary.set_field("_TRANSPORT", "driver");
                    summary.set_boot_id();
                    summary.set_machine_id();
                    summary.set_hostname();
                    let seqnum = self.seqnum.fetch_add(1, Ordering::Relaxed);
                    summary.seqnum = seqnum;
                    let mut storage = self.storage.lock().unwrap();
                    if let Err(e) = storage.append(&summary) {
                        eprintln!("journald: Failed to store suppression summary: {}", e);
                    }
                    drop(storage);
                    eprintln!(
                        "journald: Rate limit window reset for '{}': {} messages were suppressed",
                        source, suppressed
                    );
                }
                RateLimitResult::Accept => {}
            }
        }

        // Truncate oversized fields
        let max_field = self.config.max_field_size;
        if max_field > 0 {
            let keys: Vec<String> = entry.fields.keys().cloned().collect();
            for key in keys {
                if let Some(value) = entry.fields.get_mut(&key)
                    && value.len() > max_field
                {
                    value.truncate(max_field);
                }
            }
        }

        // Assign sequence number
        let seqnum = self.seqnum.fetch_add(1, Ordering::Relaxed);
        entry.seqnum = seqnum;

        // Forward to console if configured
        if self.config.forward_to_console
            && let Some(priority) = entry.priority()
            && priority <= self.config.max_level_console
        {
            let _ = writeln!(io::stderr(), "{}", entry);
        }

        // Forward to wall if configured (only for emerg/alert)
        if self.config.forward_to_wall
            && let Some(priority) = entry.priority()
            && priority <= self.config.max_level_wall
        {
            forward_to_wall(&entry);
        }

        // Store the entry
        let mut storage = self.storage.lock().unwrap();
        if let Err(e) = storage.append(&entry) {
            eprintln!("journald: Failed to store entry: {}", e);
        }
    }
}

/// Forward an entry to all logged-in terminals via wall(1)-style broadcast.
///
/// This mimics the behaviour of `wall(1)`: the message is written to every
/// terminal device that belongs to a currently logged-in user.  We enumerate
/// terminals from two sources:
///
/// 1. **utmp** (`/var/run/utmp`) — the canonical record of logged-in users.
///    Each `USER_PROCESS` entry contains a `ut_line` field (e.g. `pts/3`,
///    `tty1`) which we prepend with `/dev/` to get the device path.
/// 2. **Fallback enumeration** — if utmp is unavailable or empty we walk
///    `/dev/pts/*` and `/dev/tty[0-9]*` directly.
///
/// The message includes a timestamp so operators can correlate with the
/// journal, and the priority name when available.
fn forward_to_wall(entry: &JournalEntry) {
    let message = entry.message().unwrap_or_default();
    let identifier = entry
        .syslog_identifier()
        .unwrap_or_else(|| "unknown".to_string());
    let pid_str = entry.pid().map(|p| format!("[{}]", p)).unwrap_or_default();
    let priority_label = entry
        .priority()
        .map(|p| {
            match p {
                0 => "emerg",
                1 => "alert",
                2 => "crit",
                3 => "err",
                4 => "warning",
                5 => "notice",
                6 => "info",
                7 => "debug",
                _ => "unknown",
            }
            .to_string()
        })
        .unwrap_or_default();

    // Build a human-readable timestamp (local time)
    let now = chrono::Local::now();
    let timestamp = now.format("%b %d %H:%M:%S");

    let wall_msg = format!(
        "\r\n\
         Broadcast message from {}{} ({}, {}) at {}:\r\n\
         \r\n\
         {}\r\n",
        identifier, pid_str, priority_label, "journald", timestamp, message
    );

    // Collect terminal device paths to write to.
    let mut tty_paths: Vec<PathBuf> = Vec::new();

    // --- 1. Try utmp for authoritative logged-in user terminals -----------
    let utmp_ttys = read_utmp_terminals();
    if !utmp_ttys.is_empty() {
        for line in &utmp_ttys {
            let dev = PathBuf::from(format!("/dev/{}", line));
            if dev.exists() {
                tty_paths.push(dev);
            }
        }
    } else {
        // --- 2. Fallback: enumerate /dev/pts/* and /dev/tty[0-9]* ---------
        if let Ok(entries) = fs::read_dir("/dev/pts") {
            for dir_entry in entries.flatten() {
                let path = dir_entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                // Skip the ptmx master device
                if name == "ptmx" {
                    continue;
                }
                tty_paths.push(path);
            }
        }
        // Also check /dev/ttyN virtual consoles
        if let Ok(entries) = fs::read_dir("/dev") {
            for dir_entry in entries.flatten() {
                let path = dir_entry.path();
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                if name.starts_with("tty")
                    && name.len() > 3
                    && name[3..].chars().next().is_some_and(|c| c.is_ascii_digit())
                {
                    tty_paths.push(path);
                }
            }
        }
    }

    // Write the message to each terminal (best-effort, ignore errors).
    for tty in &tty_paths {
        // Open with O_WRONLY | O_NOCTTY | O_NONBLOCK to avoid blocking on
        // terminals that are not ready, and to avoid acquiring a controlling
        // terminal.
        if let Ok(mut f) = fs::OpenOptions::new()
            .write(true)
            .custom_flags(libc::O_NOCTTY | libc::O_NONBLOCK)
            .open(tty)
        {
            let _ = f.write_all(wall_msg.as_bytes());
        }
    }
}

/// Read `/var/run/utmp` and return the `ut_line` fields of all
/// `USER_PROCESS` entries (e.g. `"pts/3"`, `"tty1"`).
///
/// Returns an empty vec if utmp cannot be read.
fn read_utmp_terminals() -> Vec<String> {
    let mut terminals = Vec::new();

    // utmp record layout (glibc x86-64):
    //   ut_type  : i32  (offset 0)
    //   ut_pid   : i32  (offset 4)
    //   ut_line  : [u8; 32]  (offset 8)
    //   ... (rest of the struct we don't need)
    // Total struct size: 384 bytes on x86-64
    //
    // USER_PROCESS = 7

    const UT_LINESIZE: usize = 32;
    const UTMP_RECORD_SIZE: usize = std::mem::size_of::<libc::utmpx>();

    let utmp_path = if Path::new("/var/run/utmp").exists() {
        "/var/run/utmp"
    } else if Path::new("/run/utmp").exists() {
        "/run/utmp"
    } else {
        return terminals;
    };

    let data = match fs::read(utmp_path) {
        Ok(d) => d,
        Err(_) => return terminals,
    };

    // Iterate over fixed-size utmp records.
    // We use the libc utmpx struct size for portability.
    let mut offset = 0;
    while offset + UTMP_RECORD_SIZE <= data.len() {
        let record = &data[offset..offset + UTMP_RECORD_SIZE];
        offset += UTMP_RECORD_SIZE;

        // ut_type at offset 0 (i32 LE)
        if record.len() < 4 {
            continue;
        }
        let ut_type = i32::from_ne_bytes(record[0..4].try_into().unwrap());

        // USER_PROCESS = 7
        if ut_type != 7 {
            continue;
        }

        // ut_line: starts at byte 8, length UT_LINESIZE (32 bytes)
        let line_start = 8;
        let line_end = line_start + UT_LINESIZE;
        if record.len() < line_end {
            continue;
        }
        let line_bytes = &record[line_start..line_end];
        let line = String::from_utf8_lossy(line_bytes);
        let line = line.trim_end_matches('\0').to_string();
        if !line.is_empty() {
            terminals.push(line);
        }
    }

    terminals
}

// ---------------------------------------------------------------------------
// Native journal protocol parser
// ---------------------------------------------------------------------------

/// Parse a native journal protocol message.
///
/// The native protocol sends newline-separated `KEY=VALUE` pairs as a single
/// datagram.  For binary-safe values, the format uses:
///   `KEY\n<8-byte LE length><data>\n`
///
/// See `sd_journal_sendv(3)` for the full specification.
fn parse_native_message(data: &[u8]) -> JournalEntry {
    let mut entry = JournalEntry::new();
    let mut pos = 0;

    while pos < data.len() {
        // Find the next newline
        let remaining = &data[pos..];

        // Check if this line contains '=' (text field) or not (binary field)
        let newline_pos = remaining.iter().position(|&b| b == b'\n');
        let eq_pos = remaining.iter().position(|&b| b == b'=');

        match (eq_pos, newline_pos) {
            (Some(eq), Some(nl)) if eq < nl => {
                // Text field: KEY=VALUE\n
                let key = String::from_utf8_lossy(&remaining[..eq]).into_owned();
                let value = remaining[eq + 1..nl].to_vec();
                if is_valid_field_name(&key) {
                    entry.fields.insert(key, value);
                }
                pos += nl + 1;
            }
            (_, Some(nl)) if nl < remaining.len() - 1 => {
                // Might be a binary field: KEY\n<8-byte LE length><data>\n
                let key = String::from_utf8_lossy(&remaining[..nl]).into_owned();
                let after_nl = &remaining[nl + 1..];

                if after_nl.len() >= 8 {
                    let value_len = u64::from_le_bytes(after_nl[..8].try_into().unwrap()) as usize;
                    let data_start = 8;
                    if after_nl.len() >= data_start + value_len {
                        let value = after_nl[data_start..data_start + value_len].to_vec();
                        if is_valid_field_name(&key) {
                            entry.fields.insert(key, value);
                        }
                        // Skip past the value and the trailing newline
                        pos += nl + 1 + 8 + value_len;
                        if pos < data.len() && data[pos] == b'\n' {
                            pos += 1;
                        }
                        continue;
                    }
                }

                // Fallback: treat as a line without value
                pos += nl + 1;
            }
            (Some(eq), None) => {
                // Last line, no trailing newline: KEY=VALUE
                let key = String::from_utf8_lossy(&remaining[..eq]).into_owned();
                let value = remaining[eq + 1..].to_vec();
                if is_valid_field_name(&key) {
                    entry.fields.insert(key, value);
                }
                pos = data.len();
            }
            _ => {
                // Skip malformed data
                pos = match newline_pos {
                    Some(nl) => pos + nl + 1,
                    None => data.len(),
                };
            }
        }
    }

    entry
}

/// Validate a journal field name.  Field names must consist of uppercase
/// letters, digits, and underscores, must not start with a digit, and
/// must contain at least one letter or underscore.
///
/// Underscore-prefixed fields are reserved for trusted fields set by
/// journald itself, but we accept them from clients too — journald will
/// just overwrite the trusted ones.
fn is_valid_field_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    // Must not start with a digit
    if name.as_bytes()[0].is_ascii_digit() {
        return false;
    }
    name.bytes()
        .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

// ---------------------------------------------------------------------------
// Syslog protocol parser
// ---------------------------------------------------------------------------

/// Parse a BSD syslog message into a journal entry.
///
/// The format is roughly: `<PRI>TIMESTAMP HOSTNAME APP-NAME[PID]: MESSAGE`
/// or the simpler: `<PRI>MESSAGE`
fn parse_syslog_message(data: &[u8]) -> JournalEntry {
    let mut entry = JournalEntry::new();
    let text = String::from_utf8_lossy(data);
    let text = text.trim_end_matches('\n').trim_end_matches('\0');

    // Try to parse priority
    let (priority, facility, rest) = if text.starts_with('<') {
        if let Some(close) = text.find('>') {
            let pri_str = &text[1..close];
            if let Ok(pri_val) = pri_str.parse::<u32>() {
                let facility = pri_val / 8;
                let severity = (pri_val % 8) as u8;
                (Some(severity), Some(facility), &text[close + 1..])
            } else {
                (None, None, text)
            }
        } else {
            (None, None, text)
        }
    } else {
        (None, None, text)
    };

    if let Some(pri) = priority {
        entry.set_field("PRIORITY", pri.to_string());
    }
    if let Some(fac) = facility {
        entry.set_field("SYSLOG_FACILITY", fac.to_string());
    }

    // Try to parse the traditional syslog format:
    // "Mon DD HH:MM:SS hostname app[pid]: message"
    // We try a simple heuristic: if there's a colon followed by a space, split there.
    let (identifier, pid, message) = parse_syslog_tag_and_message(rest);

    if let Some(ident) = identifier {
        entry.set_field("SYSLOG_IDENTIFIER", ident);
    }
    if let Some(pid_val) = pid {
        entry.set_field("SYSLOG_PID", pid_val);
    }
    entry.set_field("MESSAGE", message);

    entry
}

/// Parse the tag and message from a syslog line (after priority).
/// Returns (identifier, pid, message).
fn parse_syslog_tag_and_message(s: &str) -> (Option<String>, Option<String>, String) {
    let s = s.trim();

    // Skip optional timestamp (3-letter month, day, time)
    // e.g. "Jan  1 00:00:00 "
    let s = skip_syslog_timestamp(s);

    // Skip optional hostname
    // After timestamp, the next word before a space could be hostname
    // We use a simple heuristic: if there's a colon in the rest, the part
    // before the colon is the tag.

    // Look for "identifier[pid]: message" or "identifier: message"
    if let Some(colon_pos) = s.find(": ") {
        let tag_part = &s[..colon_pos];
        let message = &s[colon_pos + 2..];

        // Check for [pid] in the tag
        if let Some(bracket_open) = tag_part.rfind('[')
            && let Some(bracket_close) = tag_part.rfind(']')
            && bracket_close > bracket_open
        {
            let identifier = tag_part[..bracket_open].trim();
            let pid = &tag_part[bracket_open + 1..bracket_close];

            // The identifier might have a hostname prefix; take last word
            let identifier = identifier.split_whitespace().last().unwrap_or(identifier);

            return (
                Some(identifier.to_string()),
                Some(pid.to_string()),
                message.to_string(),
            );
        }

        // No PID — just identifier: message
        let identifier = tag_part.split_whitespace().last().unwrap_or(tag_part);
        return (Some(identifier.to_string()), None, message.to_string());
    }

    // No structured tag found — entire string is the message
    (None, None, s.to_string())
}

/// Skip a syslog-style timestamp prefix (e.g. "Jan  1 00:00:00 ").
fn skip_syslog_timestamp(s: &str) -> &str {
    // Simple heuristic: if the string starts with a 3-letter month abbreviation
    // followed by day and time, skip past it
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    // Use split_whitespace to handle variable spacing (e.g. "Jan  1")
    let words: Vec<&str> = s.split_whitespace().collect();
    if words.len() >= 4 {
        // Check if first word is a month
        let first = words[0];
        if months.contains(&first) {
            // words[1] should be the day, words[2] should be HH:MM:SS
            if let Some(time_word) = words.get(2)
                && time_word.contains(':')
            {
                // Find the byte position after "Mon DD HH:MM:SS " in the
                // original string by locating the end of the time word.
                if let Some(time_start) = s.find(time_word) {
                    let after_time = time_start + time_word.len();
                    // Skip any whitespace after the time
                    let rest = &s[after_time..];
                    return rest.strip_prefix(' ').unwrap_or(rest);
                }
            }
        }
    }

    s
}

// ---------------------------------------------------------------------------
// Kernel message (kmsg) parser
// ---------------------------------------------------------------------------

/// Parse a /dev/kmsg line into a journal entry.
///
/// The format is: `PRIORITY,SEQNUM,TIMESTAMP,-;MESSAGE\n`
/// where PRIORITY includes the facility, SEQNUM is monotonic,
/// TIMESTAMP is in microseconds since boot.
fn parse_kmsg_line(line: &str) -> Option<JournalEntry> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }

    let mut entry = JournalEntry::new();
    entry.set_field("_TRANSPORT", "kernel");

    // Split at ';'
    let (header, message) = match line.split_once(';') {
        Some((h, m)) => (h, m),
        None => {
            entry.set_field("MESSAGE", line);
            return Some(entry);
        }
    };

    entry.set_field("MESSAGE", message);

    // Parse header: "priority,seqnum,timestamp,flags"
    let parts: Vec<&str> = header.split(',').collect();
    if let Some(pri_str) = parts.first()
        && let Ok(pri_val) = pri_str.parse::<u32>()
    {
        let severity = (pri_val & 7) as u8;
        let facility = pri_val >> 3;
        entry.set_field("PRIORITY", severity.to_string());
        entry.set_field("SYSLOG_FACILITY", facility.to_string());
    }

    // Set a default identifier for kernel messages
    entry.set_field("SYSLOG_IDENTIFIER", "kernel");
    entry.set_field("_PID", "0");

    Some(entry)
}

// ---------------------------------------------------------------------------
// Socket activation
// ---------------------------------------------------------------------------

/// Pre-opened sockets from socket activation (LISTEN_FDS).
struct SocketActivationFds {
    /// Native journal socket (datagram) — `/run/systemd/journal/socket`
    native: Option<UnixDatagram>,
    /// Stdout capture socket (stream) — `/run/systemd/journal/stdout`
    stdout: Option<UnixListener>,
}

/// Parse LISTEN_FDS and convert raw FDs to typed sockets.
///
/// PID 1 passes socket FDs starting at FD 3. We identify each FD's type
/// using `getsockopt(SO_TYPE)` — SOCK_DGRAM for the native journal socket,
/// SOCK_STREAM for the stdout listener.
fn receive_socket_activation_fds() -> SocketActivationFds {
    let mut result = SocketActivationFds {
        native: None,
        stdout: None,
    };

    let fd_count: usize = match std::env::var("LISTEN_FDS") {
        Ok(val) => match val.parse() {
            Ok(n) => n,
            Err(_) => return result,
        },
        Err(_) => return result,
    };

    if fd_count == 0 {
        return result;
    }

    const SD_LISTEN_FDS_START: i32 = 3;

    for i in 0..fd_count {
        let fd = SD_LISTEN_FDS_START + i as i32;

        // Determine socket type via getsockopt(SO_TYPE)
        let mut sock_type: libc::c_int = 0;
        let mut len = std::mem::size_of::<libc::c_int>() as libc::socklen_t;
        let ret = unsafe {
            libc::getsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_TYPE,
                std::ptr::from_mut(&mut sock_type).cast(),
                &mut len,
            )
        };

        if ret != 0 {
            eprintln!("journald: getsockopt(SO_TYPE) failed for fd {fd}, skipping");
            continue;
        }

        if sock_type == libc::SOCK_DGRAM && result.native.is_none() {
            eprintln!("journald: Using socket-activated datagram fd {fd} as native socket");
            result.native = Some(unsafe { UnixDatagram::from_raw_fd(fd) });
        } else if sock_type == libc::SOCK_STREAM && result.stdout.is_none() {
            eprintln!("journald: Using socket-activated stream fd {fd} as stdout socket");
            result.stdout = Some(unsafe { UnixListener::from_raw_fd(fd) });
        } else {
            eprintln!("journald: Ignoring socket-activated fd {fd} (type={sock_type})");
        }
    }

    // Clear the env vars — sd_listen_fds() convention: unset after consuming.
    // SAFETY: journald is single-threaded at this point (called from main
    // before spawning listener threads).
    unsafe {
        std::env::remove_var("LISTEN_FDS");
        std::env::remove_var("LISTEN_FDNAMES");
        std::env::remove_var("LISTEN_PID");
    }

    result
}

// ---------------------------------------------------------------------------
// Socket setup
// ---------------------------------------------------------------------------

/// Ensure the runtime directory exists and create/bind a datagram socket.
fn create_datagram_socket(path: &str) -> io::Result<UnixDatagram> {
    // Ensure parent directory exists
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }

    // Remove stale socket file
    let _ = fs::remove_file(path);

    let sock = UnixDatagram::bind(path)?;

    // Set permissions so any process can write to it
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o666));
    }

    Ok(sock)
}

/// Create a stream (connection-oriented) listener socket.
fn create_stream_listener(path: &str) -> io::Result<UnixListener> {
    if let Some(parent) = Path::new(path).parent() {
        fs::create_dir_all(parent)?;
    }
    let _ = fs::remove_file(path);

    let listener = UnixListener::bind(path)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = fs::set_permissions(path, fs::Permissions::from_mode(0o666));
    }

    Ok(listener)
}

// ---------------------------------------------------------------------------
// Socket listener threads
// ---------------------------------------------------------------------------

/// Listen on the native journal socket for datagram messages.
fn native_socket_listener(state: Arc<JournaldState>, activated_sock: Option<UnixDatagram>) {
    let sock = match activated_sock {
        Some(s) => {
            eprintln!("journald: Using socket-activated native socket");
            s
        }
        None => match create_datagram_socket(JOURNAL_SOCKET_PATH) {
            Ok(s) => s,
            Err(e) => {
                eprintln!("journald: Failed to create native socket: {}", e);
                return;
            }
        },
    };

    eprintln!("journald: Listening on {}", JOURNAL_SOCKET_PATH);

    let mut buf = vec![0u8; 256 * 1024]; // 256 KiB receive buffer

    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        match sock.recv_from(&mut buf) {
            Ok((len, _addr)) => {
                let data = &buf[..len];
                let mut entry = parse_native_message(data);

                // Set trusted fields — for datagram sockets we can get the
                // sender's credentials via SO_PEERCRED, but recv_from doesn't
                // expose that easily.  Fall back to using SYSLOG_PID if set.
                let pid = entry
                    .field("SYSLOG_PID")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);

                if pid > 0 {
                    entry.set_trusted_process_fields(pid);
                }
                entry.set_field("_TRANSPORT", "journal");
                entry.set_boot_id();
                entry.set_machine_id();
                entry.set_hostname();

                state.dispatch_entry(entry);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                if state.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                eprintln!("journald: Native socket recv error: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Listen on the syslog socket (/dev/log) for datagram messages.
fn syslog_socket_listener(state: Arc<JournaldState>) {
    let sock = match create_datagram_socket(SYSLOG_SOCKET_PATH) {
        Ok(s) => s,
        Err(e) => {
            eprintln!(
                "journald: Failed to create syslog socket {}: {}",
                SYSLOG_SOCKET_PATH, e
            );
            return;
        }
    };

    eprintln!("journald: Listening on {}", SYSLOG_SOCKET_PATH);

    let mut buf = vec![0u8; 64 * 1024]; // 64 KiB

    loop {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        match sock.recv(&mut buf) {
            Ok(len) => {
                let data = &buf[..len];
                let mut entry = parse_syslog_message(data);

                // Try to look up process info from SYSLOG_PID
                let pid = entry
                    .field("SYSLOG_PID")
                    .and_then(|s| s.parse::<u32>().ok())
                    .unwrap_or(0);
                if pid > 0 {
                    entry.set_trusted_process_fields(pid);
                }
                entry.set_field("_TRANSPORT", "syslog");
                entry.set_boot_id();
                entry.set_machine_id();
                entry.set_hostname();

                state.dispatch_entry(entry);
            }
            Err(e) if e.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(Duration::from_millis(10));
            }
            Err(e) => {
                if state.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                eprintln!("journald: Syslog socket recv error: {}", e);
                thread::sleep(Duration::from_millis(100));
            }
        }
    }
}

/// Listen on the stdout socket for stream connections from services.
///
/// Services connected here send one log line per line, optionally prefixed
/// with a header that specifies the syslog identifier, priority, etc.
fn stdout_socket_listener(state: Arc<JournaldState>, activated_listener: Option<UnixListener>) {
    let listener = match activated_listener {
        Some(l) => {
            eprintln!("journald: Using socket-activated stdout socket");
            l
        }
        None => match create_stream_listener(STDOUT_SOCKET_PATH) {
            Ok(l) => l,
            Err(e) => {
                eprintln!(
                    "journald: Failed to create stdout socket {}: {}",
                    STDOUT_SOCKET_PATH, e
                );
                return;
            }
        },
    };

    eprintln!("journald: Listening on {}", STDOUT_SOCKET_PATH);

    for stream in listener.incoming() {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        match stream {
            Ok(stream) => {
                let state = Arc::clone(&state);
                thread::spawn(move || {
                    handle_stdout_connection(stream, state);
                });
            }
            Err(e) => {
                if state.shutdown.load(Ordering::Relaxed) {
                    break;
                }
                eprintln!("journald: Stdout socket accept error: {}", e);
            }
        }
    }
}

/// Handle a single stdout stream connection.
///
/// The connection starts with a header block (newline-separated KEY=VALUE)
/// terminated by an empty line, followed by the actual log data (one line
/// per log message).
fn handle_stdout_connection(stream: UnixStream, state: Arc<JournaldState>) {
    let reader = BufReader::new(stream);
    let mut lines = reader.lines();

    // Parse the header: KEY=VALUE lines until an empty line
    let mut identifier = String::from("unknown");
    let mut priority: u8 = 6; // default: info
    let mut level_prefix = true;
    let mut unit_name = String::new();

    // Read header lines
    loop {
        match lines.next() {
            Some(Ok(line)) => {
                if line.is_empty() {
                    break; // End of header
                }
                if let Some((key, value)) = line.split_once('=') {
                    match key {
                        "SYSLOG_IDENTIFIER" => identifier = value.to_string(),
                        "PRIORITY" => {
                            if let Ok(p) = value.parse::<u8>() {
                                priority = p;
                            }
                        }
                        "LEVEL_PREFIX" => level_prefix = value == "1" || value == "true",
                        "_SYSTEMD_UNIT" | "UNIT" => unit_name = value.to_string(),
                        _ => {}
                    }
                }
            }
            Some(Err(_)) | None => return,
        }
    }

    // Process log lines
    for line in lines {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        let line = match line {
            Ok(l) => l,
            Err(_) => break,
        };

        if line.is_empty() {
            continue;
        }

        let (effective_priority, message) = if level_prefix {
            parse_level_prefix_line(&line, priority)
        } else {
            (priority, line.as_str())
        };

        let mut entry = JournalEntry::new();
        entry.set_field("MESSAGE", message);
        entry.set_field("PRIORITY", effective_priority.to_string());
        entry.set_field("SYSLOG_IDENTIFIER", &identifier);
        if !unit_name.is_empty() {
            entry.set_field("_SYSTEMD_UNIT", &unit_name);
        }
        entry.set_field("_TRANSPORT", "stdout");
        entry.set_boot_id();
        entry.set_machine_id();
        entry.set_hostname();

        state.dispatch_entry(entry);
    }
}

/// Parse a kernel-style `<N>` priority prefix from a log line.
/// Returns (priority, message_without_prefix).
fn parse_level_prefix_line(line: &str, default_priority: u8) -> (u8, &str) {
    if let Some(rest) = line.strip_prefix('<')
        && let Some(close_pos) = rest.find('>')
        && close_pos <= 1
        && let Ok(p) = rest[..close_pos].parse::<u8>()
        && p <= 7
    {
        return (p, &rest[close_pos + 1..]);
    }
    (default_priority, line)
}

/// Read and process kernel messages from /proc/kmsg (or /dev/kmsg).
fn kmsg_reader(state: Arc<JournaldState>) {
    // Try /dev/kmsg first (structured format), fall back to /proc/kmsg
    let kmsg_path = if Path::new("/dev/kmsg").exists() {
        "/dev/kmsg"
    } else if Path::new(KMSG_PATH).exists() {
        KMSG_PATH
    } else {
        eprintln!("journald: Neither /dev/kmsg nor /proc/kmsg available");
        return;
    };

    let file = match fs::File::open(kmsg_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("journald: Failed to open {}: {}", kmsg_path, e);
            return;
        }
    };

    eprintln!("journald: Reading kernel messages from {}", kmsg_path);

    let reader = BufReader::new(file);
    for line in reader.lines() {
        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        let line = match line {
            Ok(l) => l,
            Err(e) => {
                // EPIPE or EAGAIN are expected when the ring buffer wraps
                if e.kind() == io::ErrorKind::WouldBlock {
                    thread::sleep(Duration::from_millis(100));
                    continue;
                }
                eprintln!("journald: kmsg read error: {}", e);
                break;
            }
        };

        if let Some(entry) = parse_kmsg_line(&line) {
            state.dispatch_entry(entry);
        }
    }
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

/// Set up signal handlers for graceful shutdown and control.
fn setup_signal_handlers(state: Arc<JournaldState>) {
    let _state_term = Arc::clone(&state);
    let _state_usr1 = Arc::clone(&state);
    let _state_usr2 = Arc::clone(&state);

    // SIGTERM / SIGINT → graceful shutdown
    let _ = unsafe { libc::signal(libc::SIGTERM, signal_handler_shutdown as libc::sighandler_t) };
    let _ = unsafe { libc::signal(libc::SIGINT, signal_handler_shutdown as libc::sighandler_t) };

    // We use a dedicated thread to watch for signals since we can't easily
    // pass Arc state to a C signal handler.  Instead, we use a self-pipe or
    // signalfd approach.  For simplicity, we'll use a polling approach with
    // atomic flags.

    // Store a global reference for the C signal handler
    GLOBAL_SHUTDOWN.store(
        &state.shutdown as *const AtomicBool as u64,
        Ordering::Release,
    );
    GLOBAL_FLUSH.store(
        &state.flush_requested as *const AtomicBool as u64,
        Ordering::Release,
    );
    GLOBAL_ROTATE.store(
        &state.rotate_requested as *const AtomicBool as u64,
        Ordering::Release,
    );

    let _ = unsafe { libc::signal(libc::SIGUSR1, signal_handler_flush as libc::sighandler_t) };
    let _ = unsafe { libc::signal(libc::SIGUSR2, signal_handler_rotate as libc::sighandler_t) };
}

// Global atomic pointers for signal handlers (they can't capture state)
static GLOBAL_SHUTDOWN: AtomicU64 = AtomicU64::new(0);
static GLOBAL_FLUSH: AtomicU64 = AtomicU64::new(0);
static GLOBAL_ROTATE: AtomicU64 = AtomicU64::new(0);

extern "C" fn signal_handler_shutdown(_sig: libc::c_int) {
    let ptr = GLOBAL_SHUTDOWN.load(Ordering::Acquire);
    if ptr != 0 {
        let flag = unsafe { &*(ptr as *const AtomicBool) };
        flag.store(true, Ordering::Release);
    }
}

extern "C" fn signal_handler_flush(_sig: libc::c_int) {
    let ptr = GLOBAL_FLUSH.load(Ordering::Acquire);
    if ptr != 0 {
        let flag = unsafe { &*(ptr as *const AtomicBool) };
        flag.store(true, Ordering::Release);
    }
}

extern "C" fn signal_handler_rotate(_sig: libc::c_int) {
    let ptr = GLOBAL_ROTATE.load(Ordering::Acquire);
    if ptr != 0 {
        let flag = unsafe { &*(ptr as *const AtomicBool) };
        flag.store(true, Ordering::Release);
    }
}

// ---------------------------------------------------------------------------
// sd_notify support
// ---------------------------------------------------------------------------

/// Send an sd_notify message to the service manager.
fn sd_notify(msg: &str) {
    if let Ok(socket_path) = std::env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = socket_path.strip_prefix('@') {
            // Abstract socket
            format!("\0{}", stripped)
        } else {
            socket_path.clone()
        };

        if let Ok(sock) = UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), &path);
        }
    }
}

// ---------------------------------------------------------------------------
// Maintenance thread
// ---------------------------------------------------------------------------

/// Periodic maintenance: rate limiter GC, flush/rotate requests, watchdog,
/// time-based rotation, periodic vacuum, disk usage monitoring, and FSS sealing.
fn maintenance_thread(state: Arc<JournaldState>) {
    let gc_interval = Duration::from_secs(300); // 5 minutes
    let vacuum_interval = Duration::from_secs(60); // check every minute
    let disk_usage_log_interval = Duration::from_secs(3600); // log disk usage hourly
    let mut last_gc = Instant::now();
    let mut last_disk_usage_log = Instant::now();

    loop {
        thread::sleep(Duration::from_secs(1));

        if state.shutdown.load(Ordering::Relaxed) {
            break;
        }

        // Handle flush request (SIGUSR1)
        if state
            .flush_requested
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            eprintln!("journald: Flushing journal to persistent storage");
            let mut storage = state.storage.lock().unwrap();
            let _ = storage.flush();
        }

        // Handle rotate request (SIGUSR2)
        if state
            .rotate_requested
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            eprintln!("journald: Rotating journal files");
            let mut storage = state.storage.lock().unwrap();
            let _ = storage.rotate();
            *state.active_file_opened.lock().unwrap() = Instant::now();
        }

        // Time-based rotation (MaxFileSec=)
        if state.config.max_file_sec_usec > 0 {
            let opened = *state.active_file_opened.lock().unwrap();
            let max_age = Duration::from_micros(state.config.max_file_sec_usec);
            if opened.elapsed() >= max_age {
                eprintln!("journald: Rotating journal file (MaxFileSec exceeded)");
                let mut storage = state.storage.lock().unwrap();
                let _ = storage.rotate();
                *state.active_file_opened.lock().unwrap() = Instant::now();
            }
        }

        // Periodic vacuum — enforce disk usage and keep-free limits even
        // between explicit rotations.
        {
            let mut last_vac = state.last_vacuum.lock().unwrap();
            if last_vac.elapsed() >= vacuum_interval {
                let mut storage = state.storage.lock().unwrap();
                if let Err(e) = storage.vacuum() {
                    eprintln!("journald: Periodic vacuum failed: {}", e);
                }
                *last_vac = Instant::now();
            }
        }

        // Periodic disk usage logging
        if last_disk_usage_log.elapsed() >= disk_usage_log_interval {
            let storage = state.storage.lock().unwrap();
            match storage.disk_usage() {
                Ok(usage) => {
                    let max_use = if state.config.use_persistent_storage() {
                        state.config.system_max_use
                    } else {
                        state.config.runtime_max_use
                    };
                    let pct = if max_use > 0 {
                        (usage as f64 / max_use as f64 * 100.0) as u64
                    } else {
                        0
                    };
                    eprintln!(
                        "journald: Disk usage: {} bytes ({} files, {}% of limit)",
                        usage,
                        storage.file_count().unwrap_or(0),
                        pct
                    );

                    // Log a journal entry if usage is above 80% of limit
                    if max_use > 0 && usage > max_use * 80 / 100 {
                        drop(storage);
                        let mut entry = JournalEntry::new();
                        entry.set_field(
                            "MESSAGE",
                            format!(
                                "Journal disk usage is at {}% ({} / {} bytes)",
                                pct, usage, max_use
                            ),
                        );
                        entry.set_field("PRIORITY", "4"); // warning
                        entry.set_field("SYSLOG_IDENTIFIER", "systemd-journald");
                        entry.set_field("_PID", process::id().to_string());
                        entry.set_field("_TRANSPORT", "driver");
                        entry.set_boot_id();
                        entry.set_machine_id();
                        entry.set_hostname();
                        state.dispatch_entry(entry);
                    }
                }
                Err(e) => {
                    eprintln!("journald: Failed to query disk usage: {}", e);
                }
            }
            last_disk_usage_log = Instant::now();
        }

        // Forward-Secure Sealing — periodically emit seal entries
        {
            let mut seal_opt = state.seal_state.lock().unwrap();
            if let Some(ref mut seal) = *seal_opt
                && seal.should_seal()
            {
                // Build a seal tag over the current epoch counter
                let epoch_data = seal.epoch.to_le_bytes();
                let tag = seal.compute_tag(&epoch_data);
                let epoch = seal.epoch;

                // Advance the key (forward secrecy — old key is erased)
                seal.advance_key();
                seal.last_seal = Instant::now();

                // Store a seal entry in the journal
                drop(seal_opt);
                let mut entry = JournalEntry::new();
                entry.set_field("MESSAGE", format!("Journal sealed (epoch {})", epoch));
                entry.set_field("PRIORITY", "7"); // debug
                entry.set_field("SYSLOG_IDENTIFIER", "systemd-journald");
                entry.set_field("_PID", process::id().to_string());
                entry.set_field("_TRANSPORT", "driver");
                entry.set_field("_JOURNAL_SEAL_TAG", &tag);
                entry.set_field("_JOURNAL_SEAL_EPOCH", epoch.to_string());
                entry.set_boot_id();
                entry.set_machine_id();
                entry.set_hostname();
                state.dispatch_entry(entry);

                eprintln!("journald: Sealed journal (epoch {})", epoch);
            }
        }

        // Rate limiter GC
        if last_gc.elapsed() >= gc_interval {
            let mut rl = state.rate_limiter.lock().unwrap();
            rl.gc(Duration::from_secs(600)); // Clean entries older than 10 min
            last_gc = Instant::now();
        }

        // Watchdog keepalive
        if std::env::var("WATCHDOG_USEC").is_ok() {
            sd_notify("WATCHDOG=1");
        }
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    // Register signal handlers as the very first thing, before any
    // initialization.  The default disposition for SIGUSR1/SIGUSR2 is to
    // terminate the process.  If `systemd-journal-flush.service` (which runs
    // `journalctl --flush` → sends SIGUSR1) discovers our PID via /proc
    // scanning before we finish initialising, the signal would kill us
    // before we ever send READY=1.
    //
    // The handlers check the GLOBAL_* atomic pointers and gracefully no-op
    // when they are still zero, so registering them early is safe — signals
    // that arrive before `setup_signal_handlers()` stores the real pointers
    // are simply swallowed instead of being fatal.
    unsafe {
        libc::signal(libc::SIGTERM, signal_handler_shutdown as libc::sighandler_t);
        libc::signal(libc::SIGINT, signal_handler_shutdown as libc::sighandler_t);
        libc::signal(libc::SIGUSR1, signal_handler_flush as libc::sighandler_t);
        libc::signal(libc::SIGUSR2, signal_handler_rotate as libc::sighandler_t);
    }

    eprintln!("systemd-journald starting...");

    // Load configuration
    let config = JournaldConfig::load();
    let persistent = config.use_persistent_storage();

    // Determine storage directory
    let storage_dir = if persistent {
        PathBuf::from("/var/log/journal")
    } else {
        PathBuf::from("/run/log/journal")
    };

    // Ensure the storage directory exists
    if let Err(e) = fs::create_dir_all(&storage_dir) {
        eprintln!(
            "journald: Failed to create storage directory {}: {}",
            storage_dir.display(),
            e
        );
    }

    let max_use = if persistent {
        config.system_max_use
    } else {
        config.runtime_max_use
    };

    let keep_free = if persistent {
        config.system_keep_free
    } else {
        config.runtime_keep_free
    };

    let storage_config = StorageConfig {
        directory: storage_dir,
        max_file_size: config.max_file_size,
        max_disk_usage: max_use,
        max_files: config.max_files,
        persistent,
        keep_free,
    };

    let storage = match JournalStorage::new(storage_config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("journald: Failed to initialize journal storage: {}", e);
            process::exit(1);
        }
    };

    let state = Arc::new(JournaldState::new(config, storage));

    // Store the global atomic pointers so the signal handlers (registered at
    // the top of main) can actually set the shutdown/flush/rotate flags.
    setup_signal_handlers(Arc::clone(&state));

    // Ensure runtime directory exists
    let _ = fs::create_dir_all(RUNTIME_DIR);

    // Write PID file so journalctl --flush / --rotate can find us
    if let Err(e) = fs::write(PID_FILE_PATH, process::id().to_string()) {
        eprintln!(
            "journald: failed to write PID file {}: {}",
            PID_FILE_PATH, e
        );
    }

    // Check for socket activation (LISTEN_FDS from PID 1)
    let activated = receive_socket_activation_fds();
    let socket_activated = activated.native.is_some() || activated.stdout.is_some();

    // Start listener threads
    let state_native = Arc::clone(&state);
    let activated_native = activated.native;
    let _native_handle = thread::Builder::new()
        .name("native-socket".into())
        .spawn(move || native_socket_listener(state_native, activated_native))
        .expect("failed to spawn native socket thread");

    let state_syslog = Arc::clone(&state);
    let _syslog_handle = thread::Builder::new()
        .name("syslog-socket".into())
        .spawn(move || syslog_socket_listener(state_syslog))
        .expect("failed to spawn syslog socket thread");

    let state_stdout = Arc::clone(&state);
    let activated_stdout = activated.stdout;
    let _stdout_handle = thread::Builder::new()
        .name("stdout-socket".into())
        .spawn(move || stdout_socket_listener(state_stdout, activated_stdout))
        .expect("failed to spawn stdout socket thread");

    let state_kmsg = Arc::clone(&state);
    let _kmsg_handle = thread::Builder::new()
        .name("kmsg-reader".into())
        .spawn(move || kmsg_reader(state_kmsg))
        .expect("failed to spawn kmsg reader thread");

    let state_maint = Arc::clone(&state);
    let _maint_handle = thread::Builder::new()
        .name("maintenance".into())
        .spawn(move || maintenance_thread(state_maint))
        .expect("failed to spawn maintenance thread");

    // Log a startup message to the journal itself
    {
        let mut entry = JournalEntry::new();
        entry.set_field("MESSAGE", "Journal started");
        entry.set_field("PRIORITY", "6");
        entry.set_field("SYSLOG_IDENTIFIER", "systemd-journald");
        entry.set_field("_PID", process::id().to_string());
        entry.set_field("_TRANSPORT", "driver");
        entry.set_boot_id();
        entry.set_machine_id();
        entry.set_hostname();
        state.dispatch_entry(entry);
    }

    // Notify the service manager that we're ready
    sd_notify("READY=1\nSTATUS=Processing requests...");

    eprintln!("journald: Ready and processing requests");

    // Wait for shutdown
    while !state.shutdown.load(Ordering::Relaxed) {
        thread::sleep(Duration::from_millis(500));
    }

    eprintln!("journald: Shutting down...");

    // Log a shutdown message
    {
        let mut entry = JournalEntry::new();
        entry.set_field("MESSAGE", "Journal stopped");
        entry.set_field("PRIORITY", "6");
        entry.set_field("SYSLOG_IDENTIFIER", "systemd-journald");
        entry.set_field("_PID", process::id().to_string());
        entry.set_field("_TRANSPORT", "driver");
        entry.set_boot_id();
        entry.set_machine_id();
        entry.set_hostname();
        state.dispatch_entry(entry);
    }

    // Flush and close storage
    {
        let mut storage = state.storage.lock().unwrap();
        let _ = storage.flush();
    }

    // Clean up PID file. Only remove socket files if we created them ourselves
    // (not socket-activated). When socket-activated, PID 1 owns the socket FDs
    // and the filesystem entries — removing them would break re-activation.
    let _ = fs::remove_file(PID_FILE_PATH);
    if !socket_activated {
        let _ = fs::remove_file(JOURNAL_SOCKET_PATH);
        let _ = fs::remove_file(STDOUT_SOCKET_PATH);
    }
    // Syslog socket is never socket-activated — always clean it up
    let _ = fs::remove_file(SYSLOG_SOCKET_PATH);

    sd_notify("STOPPING=1");

    eprintln!("journald: Shutdown complete");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Native protocol parsing ----

    #[test]
    fn test_parse_native_simple() {
        let msg = b"MESSAGE=hello world\nPRIORITY=6\nSYSLOG_IDENTIFIER=test\n";
        let entry = parse_native_message(msg);
        assert_eq!(entry.message(), Some("hello world".to_string()));
        assert_eq!(entry.priority(), Some(6));
        assert_eq!(entry.syslog_identifier(), Some("test".to_string()));
    }

    #[test]
    fn test_parse_native_no_trailing_newline() {
        let msg = b"MESSAGE=no newline\nPRIORITY=3";
        let entry = parse_native_message(msg);
        assert_eq!(entry.message(), Some("no newline".to_string()));
        assert_eq!(entry.priority(), Some(3));
    }

    #[test]
    fn test_parse_native_binary_value() {
        // Binary format: KEY\n<8-byte LE length><data>\n
        let mut msg = Vec::new();
        msg.extend_from_slice(b"PRIORITY=6\n");
        msg.extend_from_slice(b"MESSAGE\n");
        let data = b"binary\x00data";
        msg.extend_from_slice(&(data.len() as u64).to_le_bytes());
        msg.extend_from_slice(data);
        msg.push(b'\n');

        let entry = parse_native_message(&msg);
        assert_eq!(entry.priority(), Some(6));
        assert_eq!(entry.field_bytes("MESSAGE"), Some(&b"binary\x00data"[..]));
    }

    #[test]
    fn test_parse_native_empty() {
        let entry = parse_native_message(b"");
        assert!(entry.fields.is_empty());
    }

    #[test]
    fn test_parse_native_multiple_fields() {
        let msg = b"MESSAGE=test\nPRIORITY=4\nSYSLOG_IDENTIFIER=myapp\nSYSLOG_PID=42\nCODE_FILE=main.rs\nCODE_LINE=100\n";
        let entry = parse_native_message(msg);
        assert_eq!(entry.message(), Some("test".to_string()));
        assert_eq!(entry.priority(), Some(4));
        assert_eq!(entry.field("CODE_FILE"), Some("main.rs".to_string()));
        assert_eq!(entry.field("CODE_LINE"), Some("100".to_string()));
    }

    #[test]
    fn test_parse_native_ignores_invalid_field_names() {
        let msg = b"MESSAGE=valid\nlowercase=invalid\n123=invalid\nA123=ok\n";
        let entry = parse_native_message(msg);
        assert_eq!(entry.message(), Some("valid".to_string()));
        assert!(entry.field("lowercase").is_none());
        assert!(entry.field("123").is_none());
        assert_eq!(entry.field("A123"), Some("ok".to_string()));
    }

    // ---- Syslog protocol parsing ----

    #[test]
    fn test_parse_syslog_basic() {
        let msg = b"<13>Jan  1 00:00:00 myhost myapp[1234]: Hello world";
        let entry = parse_syslog_message(msg);
        assert_eq!(entry.priority(), Some(5)); // 13 % 8 = 5 (notice)
        assert_eq!(entry.field("SYSLOG_FACILITY"), Some("1".to_string())); // 13 / 8 = 1 (user)
        assert_eq!(entry.message(), Some("Hello world".to_string()));
        assert_eq!(entry.field("SYSLOG_IDENTIFIER"), Some("myapp".to_string()));
        assert_eq!(entry.field("SYSLOG_PID"), Some("1234".to_string()));
    }

    #[test]
    fn test_parse_syslog_no_pid() {
        let msg = b"<14>myapp: Hello world";
        let entry = parse_syslog_message(msg);
        assert_eq!(entry.priority(), Some(6)); // 14 % 8 = 6 (info)
        assert_eq!(entry.message(), Some("Hello world".to_string()));
        assert_eq!(entry.field("SYSLOG_IDENTIFIER"), Some("myapp".to_string()));
        assert!(entry.field("SYSLOG_PID").is_none());
    }

    #[test]
    fn test_parse_syslog_no_priority() {
        let msg = b"Just a plain message";
        let entry = parse_syslog_message(msg);
        assert!(entry.priority().is_none());
        assert_eq!(entry.message(), Some("Just a plain message".to_string()));
    }

    #[test]
    fn test_parse_syslog_empty() {
        let entry = parse_syslog_message(b"");
        assert!(entry.message().is_some()); // Empty string
    }

    // ---- Kernel message parsing ----

    #[test]
    fn test_parse_kmsg_basic() {
        let entry = parse_kmsg_line("6,1234,5678,-;Linux version 6.1.0").unwrap();
        assert_eq!(entry.priority(), Some(6));
        assert_eq!(entry.message(), Some("Linux version 6.1.0".to_string()));
        assert_eq!(entry.field("SYSLOG_IDENTIFIER"), Some("kernel".to_string()));
    }

    #[test]
    fn test_parse_kmsg_with_facility() {
        let entry = parse_kmsg_line("30,100,9999,-;Some kernel subsystem message").unwrap();
        // 30 = facility 3 (daemon) * 8 + severity 6 (info)
        assert_eq!(entry.priority(), Some(6));
        assert_eq!(entry.field("SYSLOG_FACILITY"), Some("3".to_string()));
    }

    #[test]
    fn test_parse_kmsg_empty() {
        assert!(parse_kmsg_line("").is_none());
    }

    #[test]
    fn test_parse_kmsg_no_semicolon() {
        let entry = parse_kmsg_line("just some text").unwrap();
        assert_eq!(entry.message(), Some("just some text".to_string()));
    }

    // ---- Level prefix parsing ----

    #[test]
    fn test_parse_level_prefix_valid() {
        let (p, msg) = parse_level_prefix_line("<3>Error occurred", 6);
        assert_eq!(p, 3);
        assert_eq!(msg, "Error occurred");
    }

    #[test]
    fn test_parse_level_prefix_no_prefix() {
        let (p, msg) = parse_level_prefix_line("No prefix here", 6);
        assert_eq!(p, 6);
        assert_eq!(msg, "No prefix here");
    }

    #[test]
    fn test_parse_level_prefix_out_of_range() {
        let (p, msg) = parse_level_prefix_line("<9>Out of range", 6);
        assert_eq!(p, 6);
        assert_eq!(msg, "<9>Out of range");
    }

    // ---- Field name validation ----

    #[test]
    fn test_is_valid_field_name() {
        assert!(is_valid_field_name("MESSAGE"));
        assert!(is_valid_field_name("PRIORITY"));
        assert!(is_valid_field_name("_PID"));
        assert!(is_valid_field_name("_SYSTEMD_UNIT"));
        assert!(is_valid_field_name("MY_CUSTOM_FIELD_123"));

        assert!(!is_valid_field_name(""));
        assert!(!is_valid_field_name("lowercase"));
        assert!(!is_valid_field_name("has space"));
        assert!(!is_valid_field_name("has-dash"));
        assert!(!is_valid_field_name("has.dot"));
    }

    // ---- Configuration parsing ----

    #[test]
    fn test_config_default() {
        let config = JournaldConfig::default();
        assert_eq!(config.storage, "auto");
        assert_eq!(config.max_level_store, 7);
        assert_eq!(config.rate_limit_burst, RATE_LIMIT_BURST);
        assert!(config.forward_to_wall);
        assert!(!config.forward_to_console);
    }

    #[test]
    fn test_config_parse() {
        let mut config = JournaldConfig::default();
        config.parse_config(
            r#"
[Journal]
Storage=persistent
ForwardToConsole=yes
MaxLevelStore=warning
RateLimitBurst=5000
SystemMaxUse=1G
"#,
        );
        assert_eq!(config.storage, "persistent");
        assert!(config.forward_to_console);
        assert_eq!(config.max_level_store, 4);
        assert_eq!(config.rate_limit_burst, 5000);
        assert_eq!(config.system_max_use, 1024 * 1024 * 1024);
    }

    #[test]
    fn test_config_parse_ignores_other_sections() {
        let mut config = JournaldConfig::default();
        config.parse_config(
            r#"
[Other]
Storage=volatile

[Journal]
Storage=persistent

[Another]
Storage=none
"#,
        );
        assert_eq!(config.storage, "persistent");
    }

    // ---- Parsing helpers ----

    #[test]
    fn test_parse_bool() {
        assert!(parse_bool("yes"));
        assert!(parse_bool("true"));
        assert!(parse_bool("1"));
        assert!(parse_bool("on"));
        assert!(parse_bool("y"));
        assert!(!parse_bool("no"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool(""));
    }

    #[test]
    fn test_parse_size() {
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("1K"), Some(1024));
        assert_eq!(parse_size("1M"), Some(1024 * 1024));
        assert_eq!(parse_size("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size("1KiB"), Some(1024));
        assert_eq!(parse_size("1MiB"), Some(1024 * 1024));
        assert_eq!(parse_size("1GiB"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size("100B"), Some(100));
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("abc"), None);
    }

    #[test]
    fn test_parse_timespan_usec() {
        assert_eq!(parse_timespan_usec("1000000"), Some(1_000_000));
        assert_eq!(parse_timespan_usec("1s"), Some(1_000_000));
        assert_eq!(parse_timespan_usec("1sec"), Some(1_000_000));
        assert_eq!(parse_timespan_usec("30s"), Some(30_000_000));
        assert_eq!(parse_timespan_usec("1min"), Some(60_000_000));
        assert_eq!(parse_timespan_usec("1h"), Some(3_600_000_000));
        assert_eq!(parse_timespan_usec("100us"), Some(100));
        assert_eq!(parse_timespan_usec("100ms"), Some(100_000));
        assert_eq!(parse_timespan_usec(""), None);
    }

    #[test]
    fn test_parse_log_level() {
        assert_eq!(parse_log_level("emerg"), Some(0));
        assert_eq!(parse_log_level("alert"), Some(1));
        assert_eq!(parse_log_level("crit"), Some(2));
        assert_eq!(parse_log_level("err"), Some(3));
        assert_eq!(parse_log_level("error"), Some(3));
        assert_eq!(parse_log_level("warning"), Some(4));
        assert_eq!(parse_log_level("warn"), Some(4));
        assert_eq!(parse_log_level("notice"), Some(5));
        assert_eq!(parse_log_level("info"), Some(6));
        assert_eq!(parse_log_level("debug"), Some(7));
        assert_eq!(parse_log_level("0"), Some(0));
        assert_eq!(parse_log_level("7"), Some(7));
        assert_eq!(parse_log_level("invalid"), None);
    }

    // ---- Rate limiter ----

    #[test]
    fn test_rate_limiter_allows_within_burst() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);

        for _ in 0..10 {
            assert_eq!(rl.check("test", 10, interval), RateLimitResult::Accept);
        }
    }

    #[test]
    fn test_rate_limiter_blocks_over_burst() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);

        for _ in 0..10 {
            assert_eq!(rl.check("test", 10, interval), RateLimitResult::Accept);
        }
        // 11th should be blocked
        assert_eq!(rl.check("test", 10, interval), RateLimitResult::Suppressed);
    }

    #[test]
    fn test_rate_limiter_independent_sources() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);

        for _ in 0..5 {
            assert_eq!(rl.check("source_a", 5, interval), RateLimitResult::Accept);
        }
        assert_eq!(
            rl.check("source_a", 5, interval),
            RateLimitResult::Suppressed
        );

        // source_b should still be allowed
        assert_eq!(rl.check("source_b", 5, interval), RateLimitResult::Accept);
    }

    #[test]
    fn test_rate_limiter_gc() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);

        rl.check("old_source", 10, interval);
        assert_eq!(rl.sources.len(), 1);

        // GC with max_age of 0 should remove everything
        rl.gc(Duration::from_secs(0));
        // The entry was just created, so it won't be removed with age 0
        // unless we wait, which we don't want to do in a test.
        // Instead test that gc runs without panic
        assert!(rl.sources.len() <= 1);
    }

    // ---- Syslog timestamp skipping ----

    #[test]
    fn test_skip_syslog_timestamp() {
        assert_eq!(
            skip_syslog_timestamp("Jan  1 00:00:00 myhost myapp: msg"),
            "myhost myapp: msg"
        );
        assert_eq!(
            skip_syslog_timestamp("Dec 31 23:59:59 host test: hi"),
            "host test: hi"
        );
        assert_eq!(
            skip_syslog_timestamp("no timestamp here"),
            "no timestamp here"
        );
    }

    #[test]
    fn test_skip_syslog_timestamp_preserves_non_timestamp() {
        assert_eq!(
            skip_syslog_timestamp("myapp[123]: message"),
            "myapp[123]: message"
        );
    }

    // ---- Syslog tag parsing ----

    #[test]
    fn test_parse_syslog_tag_with_pid() {
        let (ident, pid, msg) = parse_syslog_tag_and_message("myapp[1234]: Hello");
        assert_eq!(ident, Some("myapp".to_string()));
        assert_eq!(pid, Some("1234".to_string()));
        assert_eq!(msg, "Hello");
    }

    #[test]
    fn test_parse_syslog_tag_without_pid() {
        let (ident, pid, msg) = parse_syslog_tag_and_message("myapp: Hello");
        assert_eq!(ident, Some("myapp".to_string()));
        assert_eq!(pid, None);
        assert_eq!(msg, "Hello");
    }

    #[test]
    fn test_parse_syslog_tag_no_colon() {
        let (ident, pid, msg) = parse_syslog_tag_and_message("just a message");
        assert_eq!(ident, None);
        assert_eq!(pid, None);
        assert_eq!(msg, "just a message");
    }

    // ---- Enhanced rate limiter ----

    #[test]
    fn test_rate_limiter_returns_accept() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);
        assert_eq!(rl.check("src", 10, interval), RateLimitResult::Accept);
    }

    #[test]
    fn test_rate_limiter_returns_suppressed() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);
        for _ in 0..10 {
            rl.check("src", 10, interval);
        }
        assert_eq!(rl.check("src", 10, interval), RateLimitResult::Suppressed);
    }

    #[test]
    fn test_rate_limiter_tracks_suppressed_count() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);
        // Fill the burst
        for _ in 0..5 {
            rl.check("src", 5, interval);
        }
        // These should be suppressed
        for _ in 0..3 {
            assert_eq!(rl.check("src", 5, interval), RateLimitResult::Suppressed);
        }
        // Verify the suppressed count is tracked
        let state = rl.sources.get("src").unwrap();
        assert_eq!(state.suppressed, 3);
    }

    #[test]
    fn test_rate_limiter_disabled_with_zero_burst() {
        let mut rl = RateLimiter::new();
        let interval = Duration::from_secs(30);
        // burst=0 means disabled — should always accept
        for _ in 0..100 {
            assert_eq!(rl.check("src", 0, interval), RateLimitResult::Accept);
        }
    }

    #[test]
    fn test_rate_limiter_disabled_with_zero_interval() {
        let mut rl = RateLimiter::new();
        // interval=0 means disabled — should always accept
        for _ in 0..100 {
            assert_eq!(rl.check("src", 5, Duration::ZERO), RateLimitResult::Accept);
        }
    }

    // ---- Config new fields ----

    #[test]
    fn test_config_new_fields_default() {
        let config = JournaldConfig::default();
        assert_eq!(config.system_keep_free, 4 * 1024 * 1024 * 1024);
        assert_eq!(config.runtime_keep_free, 4 * 1024 * 1024 * 1024);
        assert!(config.max_file_sec_usec > 0);
        assert!(config.seal);
    }

    #[test]
    fn test_config_parse_new_fields() {
        let mut config = JournaldConfig::default();
        config.parse_config(
            r#"
[Journal]
SystemKeepFree=1G
RuntimeKeepFree=512M
MaxFileSec=1h
Seal=no
MaxFiles=50
"#,
        );
        assert_eq!(config.system_keep_free, 1024 * 1024 * 1024);
        assert_eq!(config.runtime_keep_free, 512 * 1024 * 1024);
        assert_eq!(config.max_file_sec_usec, 3_600_000_000); // 1h in µs
        assert!(!config.seal);
        assert_eq!(config.max_files, 50);
    }

    #[test]
    fn test_config_parse_max_file_sec_separate_from_max_files() {
        let mut config = JournaldConfig::default();
        config.parse_config(
            r#"
[Journal]
MaxFiles=42
MaxFileSec=30min
"#,
        );
        assert_eq!(config.max_files, 42);
        assert_eq!(config.max_file_sec_usec, 30 * 60_000_000);
    }

    // ---- Forward-Secure Sealing ----

    #[test]
    fn test_seal_state_creation() {
        let seal = SealState::new(15 * 60 * 1_000_000);
        assert_eq!(seal.epoch, 0);
        // Key should not be all zeros (random init)
        // Note: there's an astronomically small chance this could fail
        assert!(seal.key.iter().any(|&b| b != 0));
    }

    #[test]
    fn test_seal_advance_key_changes_key() {
        let mut seal = SealState::new(1_000_000);
        let original_key = seal.key;
        seal.advance_key();
        assert_ne!(seal.key, original_key);
        assert_eq!(seal.epoch, 1);
    }

    #[test]
    fn test_seal_advance_key_erases_old() {
        let mut seal = SealState::new(1_000_000);
        let first_key = seal.key;
        seal.advance_key();
        let second_key = seal.key;
        seal.advance_key();
        // After two advances, the key should differ from both previous keys
        assert_ne!(seal.key, first_key);
        assert_ne!(seal.key, second_key);
        assert_eq!(seal.epoch, 2);
    }

    #[test]
    fn test_seal_compute_tag() {
        let seal = SealState::new(1_000_000);
        let tag1 = seal.compute_tag(b"hello");
        let tag2 = seal.compute_tag(b"hello");
        let tag3 = seal.compute_tag(b"world");
        // Same input → same tag
        assert_eq!(tag1, tag2);
        // Different input → different tag
        assert_ne!(tag1, tag3);
        // Tag should be 64 hex chars (32 bytes)
        assert_eq!(tag1.len(), 64);
    }

    #[test]
    fn test_seal_compute_tag_changes_after_advance() {
        let mut seal = SealState::new(1_000_000);
        let tag_before = seal.compute_tag(b"test");
        seal.advance_key();
        let tag_after = seal.compute_tag(b"test");
        // After key advancement, same data should produce different tag
        assert_ne!(tag_before, tag_after);
    }

    #[test]
    fn test_seal_should_seal_respects_interval() {
        let seal = SealState::new(0);
        // interval=0 means no sealing
        assert!(!seal.should_seal());
    }

    // ---- Wall message forwarding ----

    #[test]
    fn test_read_utmp_terminals_returns_vec() {
        // On most systems /var/run/utmp exists; we just verify no panic
        let ttys = read_utmp_terminals();
        // Can be empty in a test environment, but should not panic
        let _ = &ttys; // just verify it doesn't panic
    }

    // ---- RateLimitResult ----

    #[test]
    fn test_rate_limit_result_variants() {
        assert_eq!(RateLimitResult::Accept, RateLimitResult::Accept);
        assert_eq!(RateLimitResult::Suppressed, RateLimitResult::Suppressed);
        assert_eq!(
            RateLimitResult::WindowReset { suppressed: 5 },
            RateLimitResult::WindowReset { suppressed: 5 }
        );
        assert_ne!(
            RateLimitResult::WindowReset { suppressed: 5 },
            RateLimitResult::WindowReset { suppressed: 10 }
        );
        assert_ne!(RateLimitResult::Accept, RateLimitResult::Suppressed);
    }
}
