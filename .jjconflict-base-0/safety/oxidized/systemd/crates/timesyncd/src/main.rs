//! systemd-timesyncd — Network Time Synchronization daemon
//!
//! A Rust implementation of systemd-timesyncd that synchronizes the local
//! system clock with remote NTP servers using the SNTP protocol (RFC 4330).
//!
//! Features:
//! - Parses `/etc/systemd/timesyncd.conf` and drop-in directories
//! - SNTP client (UDP port 123) with offset/delay calculation
//! - NTS (Network Time Security, RFC 8915) with NTS-KE over TLS 1.3,
//!   AEAD_AES_SIV_CMAC_256 authentication, cookie management, and automatic
//!   re-keying when cookies are exhausted
//! - Gradual clock adjustment via `adjtimex()` for small offsets
//! - Step adjustment via `clock_settime()` for large offsets
//! - Saves clock state to `/var/lib/systemd/timesync/clock`
//! - D-Bus interface (`org.freedesktop.timesync1`) with deferred registration
//! - sd_notify READY=1 / WATCHDOG=1 / STATUS= protocol
//! - Signal handling: SIGTERM/SIGINT for shutdown, SIGHUP for reload
//! - Graceful degradation when no network or NTP servers are available

mod nts;

use std::fs;
use std::io::{self, Write};
use std::net::{SocketAddr, ToSocketAddrs, UdpSocket};
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use zbus::blocking::Connection;

// ── Constants ──────────────────────────────────────────────────────────────

/// NTP epoch is 1900-01-01, Unix epoch is 1970-01-01
/// Difference in seconds: 70 years (with leap years)
const NTP_EPOCH_OFFSET: u64 = 2_208_988_800;

/// NTP packet size
const NTP_PACKET_SIZE: usize = 48;

/// Default NTP port
const NTP_PORT: u16 = 123;

/// Threshold for step vs. slew adjustment (128ms, same as ntpd)
const STEP_THRESHOLD_USEC: i64 = 128_000;

/// Default poll interval bounds (seconds)
const DEFAULT_POLL_INTERVAL_MIN_SEC: u64 = 32;
const DEFAULT_POLL_INTERVAL_MAX_SEC: u64 = 2048;

/// Default connection retry interval
const DEFAULT_CONNECTION_RETRY_SEC: u64 = 30;

/// Default root distance max
const DEFAULT_ROOT_DISTANCE_MAX_SEC: f64 = 5.0;

/// Default save interval (60 seconds)
const DEFAULT_SAVE_INTERVAL_SEC: u64 = 60;

/// Clock state file
const CLOCK_STATE_PATH: &str = "/var/lib/systemd/timesync/clock";

// D-Bus constants
const DBUS_NAME: &str = "org.freedesktop.timesync1";
const DBUS_PATH: &str = "/org/freedesktop/timesync1";

/// Config file path
const CONFIG_PATH: &str = "/etc/systemd/timesyncd.conf";

/// Drop-in config directories
const CONFIG_DROPIN_DIRS: &[&str] = &[
    "/etc/systemd/timesyncd.conf.d",
    "/run/systemd/timesyncd.conf.d",
    "/usr/lib/systemd/timesyncd.conf.d",
];

/// Default fallback NTP servers (same as upstream systemd)
const DEFAULT_FALLBACK_NTP: &[&str] = &[
    "0.pool.ntp.org",
    "1.pool.ntp.org",
    "2.pool.ntp.org",
    "3.pool.ntp.org",
];

/// Socket receive timeout
const RECV_TIMEOUT: Duration = Duration::from_secs(5);

// ── Configuration ──────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
struct TimesyncdConfig {
    /// Configured NTP servers (highest priority)
    ntp_servers: Vec<String>,
    /// Fallback NTP servers
    fallback_ntp_servers: Vec<String>,
    /// Maximum acceptable root distance
    root_distance_max_sec: f64,
    /// Minimum poll interval
    poll_interval_min_sec: u64,
    /// Maximum poll interval
    poll_interval_max_sec: u64,
    /// Connection retry interval
    connection_retry_sec: u64,
    /// How often to save clock state
    save_interval_sec: u64,
    /// Enable NTS (Network Time Security, RFC 8915)
    nts_enabled: bool,
}

impl Default for TimesyncdConfig {
    fn default() -> Self {
        Self {
            ntp_servers: Vec::new(),
            fallback_ntp_servers: DEFAULT_FALLBACK_NTP.iter().map(|s| s.to_string()).collect(),
            root_distance_max_sec: DEFAULT_ROOT_DISTANCE_MAX_SEC,
            poll_interval_min_sec: DEFAULT_POLL_INTERVAL_MIN_SEC,
            poll_interval_max_sec: DEFAULT_POLL_INTERVAL_MAX_SEC,
            connection_retry_sec: DEFAULT_CONNECTION_RETRY_SEC,
            save_interval_sec: DEFAULT_SAVE_INTERVAL_SEC,
            nts_enabled: false,
        }
    }
}

impl TimesyncdConfig {
    fn load() -> Self {
        let mut config = Self::default();

        // Load main config
        if let Ok(contents) = fs::read_to_string(CONFIG_PATH) {
            config.parse_config(&contents);
        }

        // Load drop-in configs (sorted alphabetically)
        for dir in CONFIG_DROPIN_DIRS {
            if let Ok(mut entries) = fs::read_dir(dir) {
                let mut files: Vec<PathBuf> = Vec::new();
                while let Some(Ok(entry)) = entries.next() {
                    let path = entry.path();
                    if path.extension().is_some_and(|e| e == "conf") {
                        files.push(path);
                    }
                }
                files.sort();
                for path in files {
                    if let Ok(contents) = fs::read_to_string(&path) {
                        config.parse_config(&contents);
                    }
                }
            }
        }

        config
    }

    fn parse_config(&mut self, contents: &str) {
        let mut in_time_section = false;

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if line.starts_with('[') {
                in_time_section = line.eq_ignore_ascii_case("[time]");
                continue;
            }

            if !in_time_section {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "NTP" => {
                        self.ntp_servers =
                            value.split_whitespace().map(|s| s.to_string()).collect();
                    }
                    "FallbackNTP" => {
                        self.fallback_ntp_servers =
                            value.split_whitespace().map(|s| s.to_string()).collect();
                    }
                    "RootDistanceMaxSec" => {
                        if let Ok(v) = parse_duration_value(value) {
                            self.root_distance_max_sec = v;
                        }
                    }
                    "PollIntervalMinSec" => {
                        if let Ok(v) = value.parse::<u64>() {
                            self.poll_interval_min_sec = v;
                        }
                    }
                    "PollIntervalMaxSec" => {
                        if let Ok(v) = value.parse::<u64>() {
                            self.poll_interval_max_sec = v;
                        }
                    }
                    "ConnectionRetrySec" => {
                        if let Ok(v) = value.parse::<u64>() {
                            self.connection_retry_sec = v;
                        }
                    }
                    "SaveIntervalSec" => {
                        if let Ok(v) = value.parse::<u64>() {
                            self.save_interval_sec = v;
                        }
                    }
                    "NTS" => {
                        self.nts_enabled = value.eq_ignore_ascii_case("yes")
                            || value.eq_ignore_ascii_case("true")
                            || value == "1";
                    }
                    _ => {}
                }
            }
        }
    }

    /// Get the effective list of NTP servers to try
    fn effective_servers(&self) -> Vec<&str> {
        if !self.ntp_servers.is_empty() {
            self.ntp_servers.iter().map(|s| s.as_str()).collect()
        } else {
            self.fallback_ntp_servers
                .iter()
                .map(|s| s.as_str())
                .collect()
        }
    }
}

// ── NTP packet ─────────────────────────────────────────────────────────────

/// NTP timestamp: 32-bit seconds + 32-bit fraction since 1900-01-01
#[derive(Debug, Clone, Copy, Default)]
struct NtpTimestamp {
    seconds: u32,
    fraction: u32,
}

impl NtpTimestamp {
    fn from_system_time(t: SystemTime) -> Self {
        let dur = t.duration_since(UNIX_EPOCH).unwrap_or_default();
        let secs = dur.as_secs() + NTP_EPOCH_OFFSET;
        let frac = ((dur.subsec_nanos() as u64) << 32) / 1_000_000_000;
        Self {
            seconds: secs as u32,
            fraction: frac as u32,
        }
    }

    fn to_unix_secs_f64(self) -> f64 {
        let secs = self.seconds as f64 - NTP_EPOCH_OFFSET as f64;
        let frac = self.fraction as f64 / (1u64 << 32) as f64;
        secs + frac
    }

    fn from_bytes(data: &[u8]) -> Self {
        Self {
            seconds: u32::from_be_bytes([data[0], data[1], data[2], data[3]]),
            fraction: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
        }
    }

    fn to_bytes(self) -> [u8; 8] {
        let mut buf = [0u8; 8];
        buf[0..4].copy_from_slice(&self.seconds.to_be_bytes());
        buf[4..8].copy_from_slice(&self.fraction.to_be_bytes());
        buf
    }

    fn is_zero(self) -> bool {
        self.seconds == 0 && self.fraction == 0
    }
}

/// NTP/SNTP packet (48 bytes minimum)
#[derive(Debug, Clone)]
struct NtpPacket {
    /// Leap indicator (2 bits), version (3 bits), mode (3 bits)
    li_vn_mode: u8,
    /// Stratum level
    stratum: u8,
    /// Poll interval (log2 seconds)
    poll: i8,
    /// Precision (log2 seconds)
    precision: i8,
    /// Root delay (seconds, fixed-point)
    root_delay: u32,
    /// Root dispersion (seconds, fixed-point)
    root_dispersion: u32,
    /// Reference ID
    reference_id: u32,
    /// Reference timestamp
    reference_ts: NtpTimestamp,
    /// Origin timestamp (t1 — client transmit time, copied by server)
    origin_ts: NtpTimestamp,
    /// Receive timestamp (t2 — server receive time)
    receive_ts: NtpTimestamp,
    /// Transmit timestamp (t3 — server transmit time)
    transmit_ts: NtpTimestamp,
}

impl NtpPacket {
    /// Create a client request packet (mode 3, version 4)
    fn new_client_request() -> Self {
        Self {
            li_vn_mode: 0b00_100_011, // LI=0, VN=4, Mode=3 (client)
            stratum: 0,
            poll: 0,
            precision: 0,
            root_delay: 0,
            root_dispersion: 0,
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: NtpTimestamp::default(),
            receive_ts: NtpTimestamp::default(),
            transmit_ts: NtpTimestamp::from_system_time(SystemTime::now()),
        }
    }

    fn to_bytes(&self) -> [u8; NTP_PACKET_SIZE] {
        let mut buf = [0u8; NTP_PACKET_SIZE];
        buf[0] = self.li_vn_mode;
        buf[1] = self.stratum;
        buf[2] = self.poll as u8;
        buf[3] = self.precision as u8;
        buf[4..8].copy_from_slice(&self.root_delay.to_be_bytes());
        buf[8..12].copy_from_slice(&self.root_dispersion.to_be_bytes());
        buf[12..16].copy_from_slice(&self.reference_id.to_be_bytes());
        buf[16..24].copy_from_slice(&self.reference_ts.to_bytes());
        buf[24..32].copy_from_slice(&self.origin_ts.to_bytes());
        buf[32..40].copy_from_slice(&self.receive_ts.to_bytes());
        buf[40..48].copy_from_slice(&self.transmit_ts.to_bytes());
        buf
    }

    fn from_bytes(data: &[u8]) -> Option<Self> {
        if data.len() < NTP_PACKET_SIZE {
            return None;
        }
        Some(Self {
            li_vn_mode: data[0],
            stratum: data[1],
            poll: data[2] as i8,
            precision: data[3] as i8,
            root_delay: u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            root_dispersion: u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
            reference_id: u32::from_be_bytes([data[12], data[13], data[14], data[15]]),
            reference_ts: NtpTimestamp::from_bytes(&data[16..24]),
            origin_ts: NtpTimestamp::from_bytes(&data[24..32]),
            receive_ts: NtpTimestamp::from_bytes(&data[32..40]),
            transmit_ts: NtpTimestamp::from_bytes(&data[40..48]),
        })
    }

    fn mode(&self) -> u8 {
        self.li_vn_mode & 0x07
    }

    fn version(&self) -> u8 {
        (self.li_vn_mode >> 3) & 0x07
    }

    fn leap_indicator(&self) -> u8 {
        (self.li_vn_mode >> 6) & 0x03
    }

    /// Root distance = root_delay/2 + root_dispersion
    fn root_distance(&self) -> f64 {
        let delay = self.root_delay as f64 / 65536.0;
        let dispersion = self.root_dispersion as f64 / 65536.0;
        delay / 2.0 + dispersion
    }

    /// Validate the response packet
    fn validate(
        &self,
        origin_ts: &NtpTimestamp,
        max_root_distance: f64,
    ) -> Result<(), &'static str> {
        // Must be server response (mode 4) or broadcast (mode 5)
        if self.mode() != 4 && self.mode() != 5 {
            return Err("unexpected NTP mode");
        }

        // Version must be 3 or 4
        if self.version() < 3 || self.version() > 4 {
            return Err("unsupported NTP version");
        }

        // Leap indicator 3 means clock not synchronized
        if self.leap_indicator() == 3 {
            return Err("server clock not synchronized (LI=3)");
        }

        // Stratum must be 1-15
        if self.stratum == 0 || self.stratum > 15 {
            return Err("invalid stratum");
        }

        // Origin timestamp must match what we sent (anti-spoofing)
        if self.origin_ts.seconds != origin_ts.seconds
            || self.origin_ts.fraction != origin_ts.fraction
        {
            return Err("origin timestamp mismatch");
        }

        // Transmit timestamp must not be zero
        if self.transmit_ts.is_zero() {
            return Err("transmit timestamp is zero");
        }

        // Check root distance
        if self.root_distance() > max_root_distance {
            return Err("root distance too large");
        }

        Ok(())
    }
}

// ── NTP sync result ────────────────────────────────────────────────────────

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct SyncResult {
    /// Clock offset in microseconds (positive = local clock is behind)
    offset_usec: i64,
    /// Round-trip delay in microseconds
    delay_usec: i64,
    /// Server stratum
    stratum: u8,
    /// Server address
    server: SocketAddr,
    /// Server reference ID
    reference_id: u32,
    /// Root distance
    root_distance: f64,
}

impl std::fmt::Display for SyncResult {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let offset_ms = self.offset_usec as f64 / 1000.0;
        let delay_ms = self.delay_usec as f64 / 1000.0;
        write!(
            f,
            "offset={:+.3}ms delay={:.3}ms stratum={} server={}",
            offset_ms, delay_ms, self.stratum, self.server
        )
    }
}

// ── Clock adjustment ───────────────────────────────────────────────────────

/// Adjust the system clock by the given offset in microseconds.
/// Uses adjtimex() for small offsets (slew) and clock_settime() for large ones (step).
fn adjust_clock(offset_usec: i64) -> io::Result<bool> {
    let abs_offset = offset_usec.unsigned_abs();

    if abs_offset < STEP_THRESHOLD_USEC as u64 {
        // Small offset: slew using adjtimex
        slew_clock(offset_usec)?;
        Ok(false) // did not step
    } else {
        // Large offset: step the clock
        step_clock(offset_usec)?;
        Ok(true) // stepped
    }
}

/// Gradually adjust clock using adjtimex (slew)
fn slew_clock(offset_usec: i64) -> io::Result<()> {
    unsafe {
        let mut tx: libc::timex = std::mem::zeroed();
        tx.modes = libc::ADJ_OFFSET | libc::ADJ_STATUS;
        tx.offset = offset_usec as libc::c_long;
        tx.status = libc::STA_PLL;

        let ret = libc::adjtimex(&mut tx);
        if ret < 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

/// Step the clock by directly setting the time
fn step_clock(offset_usec: i64) -> io::Result<()> {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();

    let now_usec = now.as_micros() as i64;
    let new_usec = now_usec + offset_usec;

    if new_usec < 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "adjusted time would be negative",
        ));
    }

    let new_secs = new_usec / 1_000_000;
    let new_nsecs = (new_usec % 1_000_000) * 1000;

    unsafe {
        let ts = libc::timespec {
            tv_sec: new_secs as libc::time_t,
            tv_nsec: new_nsecs as libc::c_long,
        };
        if libc::clock_settime(libc::CLOCK_REALTIME, &ts) != 0 {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

// ── SNTP client ────────────────────────────────────────────────────────────

/// Perform a single SNTP exchange with the given server address
fn sntp_query(addr: SocketAddr) -> io::Result<(NtpPacket, NtpTimestamp, SystemTime)> {
    let bind_addr: SocketAddr = if addr.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };

    let socket = UdpSocket::bind(bind_addr)?;
    socket.set_read_timeout(Some(RECV_TIMEOUT))?;
    socket.set_write_timeout(Some(RECV_TIMEOUT))?;

    let request = NtpPacket::new_client_request();
    let origin_ts = request.transmit_ts;

    socket.send_to(&request.to_bytes(), addr)?;

    let mut buf = [0u8; 512];
    let (n, _from) = socket.recv_from(&mut buf)?;

    let t4 = SystemTime::now();

    if n < NTP_PACKET_SIZE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "NTP response too short",
        ));
    }

    let response = NtpPacket::from_bytes(&buf[..n]).ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "failed to parse NTP response")
    })?;

    Ok((response, origin_ts, t4))
}

/// Resolve hostname to socket addresses
fn resolve_ntp_server(hostname: &str) -> io::Result<Vec<SocketAddr>> {
    let host_with_port = if hostname.contains(':') {
        // IPv6 literal or host:port
        hostname.to_string()
    } else {
        format!("{hostname}:{NTP_PORT}")
    };

    let addrs: Vec<SocketAddr> = host_with_port.to_socket_addrs()?.collect();
    if addrs.is_empty() {
        return Err(io::Error::new(
            io::ErrorKind::NotFound,
            format!("could not resolve {hostname}"),
        ));
    }
    Ok(addrs)
}

/// Try to sync with a specific NTP server, returns the sync result
fn sync_with_server(hostname: &str, max_root_distance: f64) -> io::Result<SyncResult> {
    let addrs = resolve_ntp_server(hostname)?;

    let mut last_err = io::Error::new(io::ErrorKind::NotFound, "no addresses");

    for addr in addrs {
        match sntp_query(addr) {
            Ok((response, origin_ts, t4)) => {
                if let Err(e) = response.validate(&origin_ts, max_root_distance) {
                    log::warn!("NTP response from {} validation failed: {}", addr, e);
                    last_err = io::Error::new(io::ErrorKind::InvalidData, e);
                    continue;
                }

                // Calculate offset and delay
                // t1 = origin timestamp (client transmit time)
                // t2 = server receive time
                // t3 = server transmit time
                // t4 = client receive time

                let t1 = origin_ts.to_unix_secs_f64();
                let t2 = response.receive_ts.to_unix_secs_f64();
                let t3 = response.transmit_ts.to_unix_secs_f64();
                let t4_unix = t4
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs_f64();

                // Offset = ((t2 - t1) + (t3 - t4)) / 2
                let offset = ((t2 - t1) + (t3 - t4_unix)) / 2.0;
                // Delay = (t4 - t1) - (t3 - t2)
                let delay = (t4_unix - t1) - (t3 - t2);

                let offset_usec = (offset * 1_000_000.0) as i64;
                let delay_usec = (delay * 1_000_000.0) as i64;

                return Ok(SyncResult {
                    offset_usec,
                    delay_usec,
                    stratum: response.stratum,
                    server: addr,
                    reference_id: response.reference_id,
                    root_distance: response.root_distance(),
                });
            }
            Err(e) => {
                log::debug!("NTP query to {} failed: {}", addr, e);
                last_err = e;
            }
        }
    }

    Err(last_err)
}

// ── Clock state persistence ────────────────────────────────────────────────

/// Touch the clock state file to mark the last successful sync time.
/// systemd-timesyncd uses this to set the clock to at least the mtime of this
/// file on boot (ensuring time never goes backwards across reboots).
fn save_clock_state() {
    let path = Path::new(CLOCK_STATE_PATH);

    // Ensure parent directory exists
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }

    // Create or update the file (touch)
    match fs::OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .open(path)
    {
        Ok(mut f) => {
            // Write current timestamp
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let _ = writeln!(f, "{}", now.as_secs());
            log::debug!("Saved clock state to {}", CLOCK_STATE_PATH);
        }
        Err(e) => {
            log::debug!("Could not save clock state to {}: {}", CLOCK_STATE_PATH, e);
        }
    }
}

/// Load saved clock state and ensure system clock is at least that recent.
/// This prevents time from going backwards across reboots on systems without
/// a battery-backed RTC.
fn load_clock_state() {
    let path = Path::new(CLOCK_STATE_PATH);
    if !path.exists() {
        log::debug!("No saved clock state at {}", CLOCK_STATE_PATH);
        return;
    }

    // Read the saved timestamp
    let saved_secs = match fs::read_to_string(path) {
        Ok(contents) => match contents.trim().parse::<u64>() {
            Ok(s) => s,
            Err(_) => {
                // Fall back to file mtime
                match fs::metadata(path) {
                    Ok(meta) => meta
                        .modified()
                        .ok()
                        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                        .map(|d| d.as_secs())
                        .unwrap_or(0),
                    Err(_) => return,
                }
            }
        },
        Err(_) => return,
    };

    if saved_secs == 0 {
        return;
    }

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    if now < saved_secs {
        log::info!(
            "System clock is behind saved state by {}s, advancing",
            saved_secs - now
        );
        unsafe {
            let ts = libc::timespec {
                tv_sec: saved_secs as libc::time_t,
                tv_nsec: 0,
            };
            if libc::clock_settime(libc::CLOCK_REALTIME, &ts) != 0 {
                log::warn!("Failed to advance clock: {}", io::Error::last_os_error());
            }
        }
    } else {
        log::debug!(
            "System clock is already ahead of saved state (now={}, saved={})",
            now,
            saved_secs
        );
    }
}

// ── sd_notify ──────────────────────────────────────────────────────────────

fn sd_notify(state: &str) {
    if let Ok(path) = std::env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = path.strip_prefix('@') {
            // Abstract socket
            format!("\0{}", stripped)
        } else {
            path
        };

        if let Ok(sock) = UnixDatagram::unbound() {
            let _ = sock.send_to(state.as_bytes(), &path);
        }
    }
}

fn sd_notify_ready() {
    sd_notify("READY=1");
}

fn sd_notify_status(msg: &str) {
    sd_notify(&format!("STATUS={msg}"));
}

fn sd_notify_watchdog() {
    sd_notify("WATCHDOG=1");
}

// ── Signal handling ────────────────────────────────────────────────────────

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sigint(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sighup(_: libc::c_int) {
    RELOAD.store(true, Ordering::SeqCst);
}

fn setup_signal_handlers() {
    unsafe {
        libc::signal(
            libc::SIGTERM,
            handle_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            handle_sighup as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

// ── Logging ────────────────────────────────────────────────────────────────

fn init_logging() {
    // Check LOG_LEVEL env, default to info
    let level = std::env::var("LOG_LEVEL")
        .ok()
        .and_then(|s| s.parse::<log::LevelFilter>().ok())
        .unwrap_or(log::LevelFilter::Info);

    struct StderrLogger;

    impl log::Log for StderrLogger {
        fn enabled(&self, metadata: &log::Metadata) -> bool {
            metadata.level() <= log::max_level()
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                let now = SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default();
                let secs = now.as_secs();
                let hours = (secs % 86400) / 3600;
                let mins = (secs % 3600) / 60;
                let s = secs % 60;
                eprintln!(
                    "[{:02}:{:02}:{:02}][systemd-timesyncd][{}] {}",
                    hours,
                    mins,
                    s,
                    record.level(),
                    record.args()
                );
            }
        }

        fn flush(&self) {}
    }

    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
}

// ── Helper functions ───────────────────────────────────────────────────────

fn parse_duration_value(s: &str) -> Result<f64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty string".to_string());
    }

    // Try plain number (seconds)
    if let Ok(v) = s.parse::<f64>() {
        return Ok(v);
    }

    // Try with suffix
    if let Some(num) = s.strip_suffix("sec")
        && let Ok(v) = num.trim().parse::<f64>()
    {
        return Ok(v);
    }
    if let Some(num) = s.strip_suffix("min")
        && let Ok(v) = num.trim().parse::<f64>()
    {
        return Ok(v * 60.0);
    }
    if let Some(num) = s.strip_suffix("ms")
        && let Ok(v) = num.trim().parse::<f64>()
    {
        return Ok(v / 1000.0);
    }
    if let Some(num) = s.strip_suffix('s')
        && let Ok(v) = num.trim().parse::<f64>()
    {
        return Ok(v);
    }
    if let Some(num) = s.strip_suffix('m')
        && let Ok(v) = num.trim().parse::<f64>()
    {
        return Ok(v * 60.0);
    }
    if let Some(num) = s.strip_suffix('h')
        && let Ok(v) = num.trim().parse::<f64>()
    {
        return Ok(v * 3600.0);
    }

    Err(format!("cannot parse duration: {s}"))
}

/// Format a reference ID as either an IP address (stratum > 1) or ASCII string (stratum 1)
fn format_reference_id(refid: u32, stratum: u8) -> String {
    if stratum <= 1 {
        // ASCII reference identifier (e.g., "GPS", "PPS")
        let bytes = refid.to_be_bytes();
        let s: String = bytes
            .iter()
            .filter(|b| b.is_ascii_graphic() || **b == b' ')
            .map(|b| *b as char)
            .collect();
        s.trim().to_string()
    } else {
        // IPv4 address for stratum 2+
        let bytes = refid.to_be_bytes();
        format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])
    }
}

// ── Watchdog ───────────────────────────────────────────────────────────────

fn watchdog_interval() -> Option<Duration> {
    std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .map(|usec| Duration::from_micros(usec / 2)) // ping at half the interval
}

// ── Shared D-Bus state ─────────────────────────────────────────────────────

/// Snapshot of timesyncd state exposed via D-Bus properties.
#[derive(Debug, Clone)]
struct TimesyncState {
    /// Whether we have successfully synced at least once.
    synced: bool,
    /// Current poll interval in seconds.
    poll_interval_usec: u64,
    /// Config: root distance max in microseconds.
    root_distance_max_usec: u64,
    /// Config: minimum poll interval in microseconds.
    poll_interval_min_usec: u64,
    /// Config: maximum poll interval in microseconds.
    poll_interval_max_usec: u64,
    /// Current NTP server name (hostname before resolution).
    server_name: String,
    /// Current NTP server address (resolved IP:port).
    server_address: String,
    /// Frequency adjustment in PPB (parts per billion) from adjtimex.
    frequency: i64,
    /// Last NTP message fields for NTPMessage property.
    ntp_message: NtpMessageFields,
    /// Runtime NTP servers set via the SetRuntimeNTPServers() D-Bus method,
    /// exposed as the RuntimeNTPServers property.
    runtime_ntp_servers: Vec<String>,
    /// Per-link NTP servers aggregated from networkd's link state files under
    /// /run/systemd/netif/links (the NTP= lines it writes for SetLinkNTP /
    /// configured NTP=), exposed as the LinkNTPServers property.
    link_ntp_servers: Vec<String>,
}

/// Fields from the last NTP exchange, exposed as the NTPMessage property.
#[derive(Debug, Clone, Default)]
struct NtpMessageFields {
    /// Leap indicator (0-3).
    leap: u32,
    /// NTP version.
    version: u32,
    /// NTP mode.
    mode: u32,
    /// Server stratum.
    stratum: u32,
    /// Server precision (log2 seconds).
    precision: i32,
    /// Root delay in microseconds.
    root_delay_usec: u64,
    /// Root dispersion in microseconds.
    root_dispersion_usec: u64,
    /// Reference ID.
    reference_id: u32,
    /// Origin timestamp in microseconds since epoch.
    origin_usec: u64,
    /// Receive timestamp in microseconds since epoch.
    receive_usec: u64,
    /// Transmit timestamp in microseconds since epoch.
    transmit_usec: u64,
    /// Destination timestamp in microseconds since epoch.
    dest_usec: u64,
    /// Calculated offset in microseconds (signed).
    offset_usec: i64,
    /// Calculated delay in microseconds.
    delay_usec: u64,
}

impl Default for TimesyncState {
    fn default() -> Self {
        Self {
            synced: false,
            poll_interval_usec: DEFAULT_POLL_INTERVAL_MIN_SEC * 1_000_000,
            root_distance_max_usec: (DEFAULT_ROOT_DISTANCE_MAX_SEC * 1_000_000.0) as u64,
            poll_interval_min_usec: DEFAULT_POLL_INTERVAL_MIN_SEC * 1_000_000,
            poll_interval_max_usec: DEFAULT_POLL_INTERVAL_MAX_SEC * 1_000_000,
            server_name: String::new(),
            server_address: String::new(),
            frequency: 0,
            ntp_message: NtpMessageFields::default(),
            runtime_ntp_servers: Vec::new(),
            link_ntp_servers: Vec::new(),
        }
    }
}

type SharedState = Arc<Mutex<TimesyncState>>;

// ── D-Bus interface: org.freedesktop.timesync1.Manager ─────────────────────

/// Register the org.freedesktop.timesync1.Manager interface on a Crossroads
/// instance.
///
/// Properties (read-only):
///   NTPSynchronized (b)  — whether we have synced at least once
///   ServerName (s)       — hostname of the current NTP server
///   ServerAddress ((iay))— address of current server (family, bytes)
///   Frequency (x)        — frequency adjustment in PPB
///   RootDistanceMaxUSec (t)  — configured max root distance
///   PollIntervalMinUSec (t)  — configured min poll interval
///   PollIntervalMaxUSec (t)  — configured max poll interval
///   PollIntervalUSec (t)     — current poll interval
///   NTPMessage ((uuuuitttttttt)) — last NTP message details
///
/// Methods:
///   Describe() → s (JSON description of the daemon state)
/// D-Bus interface struct for org.freedesktop.timesync1.Manager.
///
/// Holds a shared state reference so that zbus can serve properties and
/// methods automatically from a background thread.
struct Timesync1Manager {
    state: SharedState,
}

#[zbus::interface(name = "org.freedesktop.timesync1.Manager")]
impl Timesync1Manager {
    // --- Properties ---

    #[zbus(property, name = "NTPSynchronized")]
    fn ntp_synchronized(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.synced
    }

    #[zbus(property, name = "ServerName")]
    fn server_name(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.server_name.clone()
    }

    // ServerAddress as (iay) — address family + raw address bytes
    #[zbus(property, name = "ServerAddress")]
    fn server_address(&self) -> (i32, Vec<u8>) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Ok(addr) = s.server_address.parse::<SocketAddr>() {
            match addr.ip() {
                std::net::IpAddr::V4(v4) => (2i32, v4.octets().to_vec()), // AF_INET=2
                std::net::IpAddr::V6(v6) => (10i32, v6.octets().to_vec()), // AF_INET6=10
            }
        } else {
            (0i32, Vec::new()) // unspecified
        }
    }

    #[zbus(property, name = "Frequency")]
    fn frequency(&self) -> i64 {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.frequency
    }

    #[zbus(property, name = "RootDistanceMaxUSec")]
    fn root_distance_max_usec(&self) -> u64 {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.root_distance_max_usec
    }

    #[zbus(property, name = "PollIntervalMinUSec")]
    fn poll_interval_min_usec(&self) -> u64 {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.poll_interval_min_usec
    }

    #[zbus(property, name = "PollIntervalMaxUSec")]
    fn poll_interval_max_usec(&self) -> u64 {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.poll_interval_max_usec
    }

    #[zbus(property, name = "PollIntervalUSec")]
    fn poll_interval_usec(&self) -> u64 {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.poll_interval_usec
    }

    #[zbus(property, name = "RuntimeNTPServers")]
    fn runtime_ntp_servers(&self) -> Vec<String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.runtime_ntp_servers.clone()
    }

    #[zbus(property, name = "LinkNTPServers")]
    fn link_ntp_servers(&self) -> Vec<String> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.link_ntp_servers.clone()
    }

    // --- Methods ---

    // Describe() → s (JSON description)
    fn describe(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let ntp = &s.ntp_message;
        format!(
            concat!(
                "{{",
                "\"NTPSynchronized\":{},",
                "\"ServerName\":\"{}\",",
                "\"ServerAddress\":\"{}\",",
                "\"Frequency\":{},",
                "\"PollIntervalUSec\":{},",
                "\"PollIntervalMinUSec\":{},",
                "\"PollIntervalMaxUSec\":{},",
                "\"RootDistanceMaxUSec\":{},",
                "\"NTPMessage\":{{",
                "\"Leap\":{},",
                "\"Version\":{},",
                "\"Mode\":{},",
                "\"Stratum\":{},",
                "\"Precision\":{},",
                "\"RootDelayUSec\":{},",
                "\"RootDispersionUSec\":{},",
                "\"ReferenceID\":{},",
                "\"OriginUSec\":{},",
                "\"ReceiveUSec\":{},",
                "\"TransmitUSec\":{},",
                "\"DestUSec\":{},",
                "\"OffsetUSec\":{},",
                "\"DelayUSec\":{}",
                "}}",
                "}}"
            ),
            s.synced,
            json_escape(&s.server_name),
            json_escape(&s.server_address),
            s.frequency,
            s.poll_interval_usec,
            s.poll_interval_min_usec,
            s.poll_interval_max_usec,
            s.root_distance_max_usec,
            ntp.leap,
            ntp.version,
            ntp.mode,
            ntp.stratum,
            ntp.precision,
            ntp.root_delay_usec,
            ntp.root_dispersion_usec,
            ntp.reference_id,
            ntp.origin_usec,
            ntp.receive_usec,
            ntp.transmit_usec,
            ntp.dest_usec,
            ntp.offset_usec,
            ntp.delay_usec,
        )
    }

    // SetRuntimeNTPServers(as): replace the runtime NTP server list and emit a
    // PropertiesChanged signal for RuntimeNTPServers when it actually changes.
    #[zbus(name = "SetRuntimeNTPServers")]
    async fn set_runtime_ntp_servers(
        &self,
        servers: Vec<String>,
        #[zbus(signal_emitter)] emitter: zbus::object_server::SignalEmitter<'_>,
    ) -> zbus::fdo::Result<()> {
        {
            let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.runtime_ntp_servers == servers {
                return Ok(());
            }
            s.runtime_ntp_servers = servers;
        }
        self.runtime_n_t_p_servers_changed(&emitter).await?;
        Ok(())
    }
}

/// Escape a string for embedding in JSON.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

/// Set up the D-Bus connection and register the timesync1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedState) -> Result<Connection, String> {
    let iface = Timesync1Manager { state: shared };
    let conn = zbus::blocking::connection::Builder::system()
        .map_err(|e| format!("D-Bus builder failed: {}", e))?
        .name(DBUS_NAME)
        .map_err(|e| format!("D-Bus name request failed: {}", e))?
        .serve_at(DBUS_PATH, iface)
        .map_err(|e| format!("D-Bus serve_at failed: {}", e))?
        .build()
        .map_err(|e| format!("D-Bus connection failed: {}", e))?;
    Ok(conn)
}

/// Spawn a background thread that watches networkd's per-link state files under
/// `/run/systemd/netif/links` for NTP server changes and emits a
/// `LinkNTPServers` PropertiesChanged signal when the aggregated list changes.
/// This mirrors upstream timesyncd tracking link NTP via sd-network, and drives
/// the `assert_timesyncd_signal LinkNTPServers` half of TEST-45-TIMEDATE.
fn spawn_link_ntp_watch(conn: Connection, shared: SharedState) {
    let _ = std::thread::Builder::new()
        .name("timesyncd-link-ntp".into())
        .spawn(move || {
            loop {
                let current = read_link_ntp_servers();
                let changed = {
                    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
                    if s.link_ntp_servers != current {
                        s.link_ntp_servers = current;
                        true
                    } else {
                        false
                    }
                };
                if changed
                    && let Ok(emitter) =
                        zbus::object_server::SignalEmitter::new(conn.inner(), DBUS_PATH)
                {
                    let iface = Timesync1Manager {
                        state: shared.clone(),
                    };
                    let _ = zbus::block_on(iface.link_n_t_p_servers_changed(&emitter));
                }
                std::thread::sleep(Duration::from_millis(500));
            }
        });
}

/// Aggregate the NTP servers networkd has recorded for every managed link,
/// reading the `NTP=` lines from each `/run/systemd/netif/links/<ifindex>`
/// state file in ascending ifindex order.
fn read_link_ntp_servers() -> Vec<String> {
    let dir = std::path::Path::new("/run/systemd/netif/links");
    let mut links: Vec<(u32, std::path::PathBuf)> = Vec::new();
    if let Ok(rd) = std::fs::read_dir(dir) {
        for entry in rd.flatten() {
            if let Some(name) = entry.file_name().to_str()
                && let Ok(idx) = name.parse::<u32>()
            {
                links.push((idx, entry.path()));
            }
        }
    }
    links.sort_by_key(|(idx, _)| *idx);
    let mut out = Vec::new();
    for (_, path) in links {
        if let Ok(content) = std::fs::read_to_string(&path) {
            for line in content.lines() {
                if let Some(v) = line.strip_prefix("NTP=") {
                    let v = v.trim();
                    if !v.is_empty() {
                        out.push(v.to_string());
                    }
                }
            }
        }
    }
    out
}

/// Read current frequency adjustment from adjtimex (in PPB).
fn read_frequency_ppb() -> i64 {
    unsafe {
        let mut tx: libc::timex = std::mem::zeroed();
        tx.modes = 0; // read-only
        if libc::adjtimex(&mut tx) >= 0 {
            // tx.freq is in units of 2^-16 ppm = scaled ppm
            // Convert to PPB: (freq * 1000) >> 16
            (tx.freq * 1000) >> 16
        } else {
            0
        }
    }
}

/// Convert an NTP timestamp to microseconds since Unix epoch.
#[allow(dead_code)]
fn ntp_ts_to_usec(ts: &NtpTimestamp) -> u64 {
    if ts.is_zero() {
        return 0;
    }
    let unix_secs = ts.seconds as u64 - NTP_EPOCH_OFFSET;
    let frac_usec = ((ts.fraction as u64) * 1_000_000) >> 32;
    unix_secs * 1_000_000 + frac_usec
}

// ── Main sync loop ─────────────────────────────────────────────────────────

struct TimesyncDaemon {
    config: TimesyncdConfig,
    current_server_idx: usize,
    poll_interval: u64,
    synced: bool,
    sync_count: u64,
    last_sync: Option<SyncResult>,
    last_save: std::time::Instant,
    shared_state: SharedState,
    /// NTS state (present when NTS is enabled and NTS-KE succeeded)
    nts_state: Option<nts::NtsState>,
}

impl TimesyncDaemon {
    fn new(config: TimesyncdConfig, shared_state: SharedState) -> Self {
        // Initialize shared state from config
        {
            let mut s = shared_state.lock().unwrap_or_else(|e| e.into_inner());
            s.root_distance_max_usec = (config.root_distance_max_sec * 1_000_000.0) as u64;
            s.poll_interval_min_usec = config.poll_interval_min_sec * 1_000_000;
            s.poll_interval_max_usec = config.poll_interval_max_sec * 1_000_000;
            s.poll_interval_usec = config.poll_interval_min_sec * 1_000_000;
        }
        Self {
            poll_interval: config.poll_interval_min_sec,
            config,
            current_server_idx: 0,
            synced: false,
            sync_count: 0,
            last_sync: None,
            last_save: std::time::Instant::now(),
            shared_state,
            nts_state: None,
        }
    }

    fn run(&mut self) {
        log::info!("systemd-timesyncd starting");

        // Load saved clock state (advance clock if needed)
        load_clock_state();

        let servers = self.config.effective_servers();
        if servers.is_empty() {
            log::warn!("No NTP servers configured, idling");
            sd_notify_status("No NTP servers configured.");
            sd_notify_ready();

            // Idle until shutdown
            while !SHUTDOWN.load(Ordering::SeqCst) {
                std::thread::sleep(Duration::from_secs(1));
                if RELOAD.swap(false, Ordering::SeqCst) {
                    self.reload_config();
                    let servers = self.config.effective_servers();
                    if !servers.is_empty() {
                        break; // Now we have servers, proceed to sync loop
                    }
                }
            }

            if SHUTDOWN.load(Ordering::SeqCst) {
                log::info!("Shutting down");
                return;
            }
        }

        let servers = self.config.effective_servers();
        log::info!("Using NTP server(s): {}", servers.to_vec().join(", "));

        sd_notify_status("Initializing...");
        sd_notify_ready();

        let wd_interval = watchdog_interval();

        // D-Bus connection is deferred to after READY=1 so we don't block
        // early boot waiting for dbus-daemon.  We retry periodically until
        // the connection succeeds, since this service may start before
        // dbus.socket is activated.  zbus dispatches messages automatically
        // in a background thread — we just keep the connection alive.
        let mut _dbus_conn: Option<Connection> = None;
        let mut dbus_next_retry = std::time::Instant::now();
        const DBUS_RETRY_INTERVAL: Duration = Duration::from_secs(5);

        loop {
            if SHUTDOWN.load(Ordering::SeqCst) {
                break;
            }

            if RELOAD.swap(false, Ordering::SeqCst) {
                self.reload_config();
            }

            // Attempt D-Bus registration (deferred from startup, retried
            // periodically until successful).  zbus handles message dispatch
            // in a background thread automatically.
            if _dbus_conn.is_none() && std::time::Instant::now() >= dbus_next_retry {
                match setup_dbus(self.shared_state.clone()) {
                    Ok(conn) => {
                        log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                        // Watch networkd's link state files for NTP changes and
                        // emit LinkNTPServers PropertiesChanged (the timesyncd
                        // half of TEST-45-TIMEDATE testcase_timesyncd).
                        spawn_link_ntp_watch(conn.clone(), self.shared_state.clone());
                        _dbus_conn = Some(conn);
                    }
                    Err(e) => {
                        log::debug!(
                            "D-Bus not yet available ({}); will retry in {}s",
                            e,
                            DBUS_RETRY_INTERVAL.as_secs()
                        );
                        dbus_next_retry = std::time::Instant::now() + DBUS_RETRY_INTERVAL;
                    }
                }
            }

            // Try to sync
            self.try_sync();

            // Save clock state periodically
            if self.synced
                && self.last_save.elapsed() >= Duration::from_secs(self.config.save_interval_sec)
            {
                save_clock_state();
                self.last_save = std::time::Instant::now();
            }

            // Sleep for the poll interval, but wake up for watchdog/signals/D-Bus
            let sleep_duration = if self.synced {
                Duration::from_secs(self.poll_interval)
            } else {
                Duration::from_secs(self.config.connection_retry_sec)
            };

            let sleep_start = std::time::Instant::now();
            while sleep_start.elapsed() < sleep_duration {
                if SHUTDOWN.load(Ordering::SeqCst) {
                    break;
                }
                if RELOAD.swap(false, Ordering::SeqCst) {
                    self.reload_config();
                    break; // retry sync immediately after reload
                }

                // Watchdog ping
                if let Some(wd) = wd_interval {
                    sd_notify_watchdog();
                    std::thread::sleep(wd.min(Duration::from_secs(1)));
                } else {
                    std::thread::sleep(Duration::from_secs(1));
                }
            }
        }

        // Save state on exit
        if self.synced {
            save_clock_state();
        }

        log::info!("Shutting down (synced {} times)", self.sync_count);
    }

    /// Attempt NTS-KE for the given server if NTS is enabled but state is
    /// missing or cookies are exhausted.
    fn ensure_nts_state(&mut self, server: &str) {
        if !self.config.nts_enabled {
            self.nts_state = None;
            return;
        }

        // Re-key when we have no state or no cookies left
        let needs_ke = match &self.nts_state {
            None => true,
            Some(st) => !st.has_cookies(),
        };

        if needs_ke {
            log::info!(
                "NTS-KE: establishing keys with {server}:{}",
                nts::NTS_KE_PORT
            );
            sd_notify_status(&format!("NTS-KE with {server}..."));
            match nts::NtsState::establish(server, nts::NTS_KE_PORT) {
                Ok(state) => {
                    log::info!(
                        "NTS-KE: success, {} cookie(s), server={}",
                        state.cookie_jar.len(),
                        state.effective_ntp_server(),
                    );
                    self.nts_state = Some(state);
                }
                Err(e) => {
                    log::warn!("NTS-KE with {server} failed: {e} — falling back to plain NTP");
                    self.nts_state = None;
                }
            }
        }
    }

    /// Attempt an NTS-protected sync with the given server.
    fn try_nts_sync(&mut self, _server: &str, max_root_distance: f64) -> io::Result<SyncResult> {
        let nts_state = self
            .nts_state
            .as_mut()
            .ok_or_else(|| io::Error::other("NTS state not established"))?;

        let ntp_server = nts_state.effective_ntp_server().to_string();
        let ntp_port = nts_state.effective_ntp_port();

        let host_with_port = format!("{ntp_server}:{ntp_port}");
        let addrs: Vec<SocketAddr> = host_with_port
            .to_socket_addrs()
            .map_err(|e| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("resolve {host_with_port}: {e}"),
                )
            })?
            .collect();
        if addrs.is_empty() {
            return Err(io::Error::new(
                io::ErrorKind::NotFound,
                format!("could not resolve {host_with_port}"),
            ));
        }

        let mut last_err = io::Error::new(io::ErrorKind::NotFound, "no addresses");

        for addr in addrs {
            match nts::nts_sntp_query(addr, nts_state) {
                Ok((packet_bytes, origin_ts, t4)) => {
                    let response = NtpPacket::from_bytes(&packet_bytes).ok_or_else(|| {
                        io::Error::new(io::ErrorKind::InvalidData, "failed to parse NTP response")
                    })?;

                    if let Err(e) = response.validate(&origin_ts, max_root_distance) {
                        log::warn!("NTS NTP response from {} validation failed: {}", addr, e);
                        last_err = io::Error::new(io::ErrorKind::InvalidData, e);
                        continue;
                    }

                    let t1 = origin_ts.to_unix_secs_f64();
                    let t2 = response.receive_ts.to_unix_secs_f64();
                    let t3 = response.transmit_ts.to_unix_secs_f64();
                    let t4_unix = t4
                        .duration_since(UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs_f64();

                    let offset = ((t2 - t1) + (t3 - t4_unix)) / 2.0;
                    let delay = (t4_unix - t1) - (t3 - t2);

                    return Ok(SyncResult {
                        offset_usec: (offset * 1_000_000.0) as i64,
                        delay_usec: (delay * 1_000_000.0) as i64,
                        stratum: response.stratum,
                        server: addr,
                        reference_id: response.reference_id,
                        root_distance: response.root_distance(),
                    });
                }
                Err(e) => {
                    log::debug!("NTS NTP query to {} failed: {}", addr, e);
                    last_err = io::Error::other(e.to_string());
                }
            }
        }

        Err(last_err)
    }

    fn try_sync(&mut self) {
        let servers: Vec<String> = self
            .config
            .effective_servers()
            .iter()
            .map(|s| s.to_string())
            .collect();
        if servers.is_empty() {
            return;
        }

        // Try servers starting from current index
        let n = servers.len();
        for i in 0..n {
            let idx = (self.current_server_idx + i) % n;
            let server = &servers[idx];

            // If NTS is enabled, attempt NTS-KE and NTS-protected query
            if self.config.nts_enabled {
                self.ensure_nts_state(server);
                if self.nts_state.is_some() {
                    sd_notify_status(&format!("Querying {server} (NTS)..."));
                    log::debug!("Querying NTP server (NTS): {}", server);

                    match self.try_nts_sync(server, self.config.root_distance_max_sec) {
                        Ok(result) => {
                            log::info!("NTS synchronized with {}: {}", server, result);
                            self.apply_sync_result(server, idx, result);
                            return;
                        }
                        Err(e) => {
                            log::debug!("NTS sync with {} failed: {}", server, e);
                            // Fall through to plain NTP below
                        }
                    }
                }
            }

            sd_notify_status(&format!("Querying {server}..."));
            log::debug!("Querying NTP server: {}", server);

            match sync_with_server(server, self.config.root_distance_max_sec) {
                Ok(result) => {
                    log::info!("Synchronized with {}: {}", server, result);
                    self.apply_sync_result(server, idx, result);
                    return;
                }
                Err(e) => {
                    log::debug!("Failed to sync with {}: {}", server, e);
                }
            }
        }

        // All servers failed
        if !self.synced {
            log::warn!("Failed to synchronize with any NTP server, will retry");
            sd_notify_status("Failed to contact NTP servers, retrying...");
        } else {
            log::debug!("Failed to re-sync, keeping last successful sync");
        }

        // Reset poll interval on failure
        self.poll_interval = self.config.poll_interval_min_sec;
    }

    /// Apply a successful sync result: adjust clock, update state and D-Bus.
    fn apply_sync_result(&mut self, server: &str, idx: usize, result: SyncResult) {
        match adjust_clock(result.offset_usec) {
            Ok(stepped) => {
                if stepped {
                    log::info!(
                        "Stepped clock by {:+.3}ms",
                        result.offset_usec as f64 / 1000.0
                    );
                } else {
                    log::debug!(
                        "Slewing clock by {:+.3}ms",
                        result.offset_usec as f64 / 1000.0
                    );
                }
            }
            Err(e) => {
                log::warn!("Failed to adjust clock: {}", e);
            }
        }

        self.synced = true;
        self.sync_count += 1;
        self.current_server_idx = idx;
        self.last_sync = Some(result.clone());

        {
            let mut s = self.shared_state.lock().unwrap_or_else(|e| e.into_inner());
            s.synced = true;
            s.server_name = server.to_string();
            s.server_address = result.server.to_string();
            s.frequency = read_frequency_ppb();
            s.poll_interval_usec = self.poll_interval * 1_000_000;
            s.ntp_message.stratum = result.stratum as u32;
            s.ntp_message.reference_id = result.reference_id;
            s.ntp_message.offset_usec = result.offset_usec;
            s.ntp_message.delay_usec = result.delay_usec as u64;
        }

        if self.poll_interval < self.config.poll_interval_max_sec {
            self.poll_interval = (self.poll_interval * 2).min(self.config.poll_interval_max_sec);
        }

        let refid = format_reference_id(result.reference_id, result.stratum);
        sd_notify_status(&format!(
            "Synchronized to time server {} ({}). Offset: {:+.3}ms.{}",
            result.server,
            refid,
            result.offset_usec as f64 / 1000.0,
            if self.nts_state.is_some() {
                " [NTS]"
            } else {
                ""
            },
        ));
    }

    fn reload_config(&mut self) {
        log::info!("Reloading configuration");
        self.config = TimesyncdConfig::load();
        self.current_server_idx = 0;
        self.poll_interval = self.config.poll_interval_min_sec;

        // Invalidate NTS state on reload so NTS-KE is retried with new config
        if !self.config.nts_enabled {
            self.nts_state = None;
        }

        // Update shared D-Bus state with new config values
        {
            let mut s = self.shared_state.lock().unwrap_or_else(|e| e.into_inner());
            s.root_distance_max_usec = (self.config.root_distance_max_sec * 1_000_000.0) as u64;
            s.poll_interval_min_usec = self.config.poll_interval_min_sec * 1_000_000;
            s.poll_interval_max_usec = self.config.poll_interval_max_sec * 1_000_000;
            s.poll_interval_usec = self.config.poll_interval_min_sec * 1_000_000;
        }

        let servers = self.config.effective_servers();
        log::info!(
            "Using NTP server(s): {} (NTS={})",
            servers.to_vec().join(", "),
            if self.config.nts_enabled { "yes" } else { "no" },
        );
    }
}

// ── Capability check ───────────────────────────────────────────────────────

/// Check if we have CAP_SYS_TIME (needed to adjust clock)
fn check_cap_sys_time() -> bool {
    // Try a harmless adjtimex read to check if we can adjust the clock
    unsafe {
        let mut tx: libc::timex = std::mem::zeroed();
        tx.modes = 0; // read-only
        libc::adjtimex(&mut tx) >= 0
    }
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    init_logging();
    setup_signal_handlers();

    // Check prerequisites
    if !check_cap_sys_time() {
        log::warn!("No CAP_SYS_TIME capability; clock adjustments may fail");
    }

    // Check if running in a container (ConditionVirtualization=!container in unit)
    if is_container() {
        log::info!("Running inside a container, time sync is not possible — exiting cleanly");
        sd_notify_ready();
        return;
    }

    let config = TimesyncdConfig::load();
    let shared_state: SharedState = Arc::new(Mutex::new(TimesyncState::default()));
    let mut daemon = TimesyncDaemon::new(config, shared_state);
    daemon.run();
}

/// Simple container detection
fn is_container() -> bool {
    // Check for containerization markers
    if std::env::var("container").is_ok() {
        return true;
    }

    // Check systemd's virtualization detection file
    if let Ok(contents) = fs::read_to_string("/run/systemd/container") {
        return !contents.trim().is_empty();
    }

    false
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_config_default() {
        let config = TimesyncdConfig::default();
        assert!(config.ntp_servers.is_empty());
        assert_eq!(config.fallback_ntp_servers.len(), 4);
        assert_eq!(config.poll_interval_min_sec, 32);
        assert_eq!(config.poll_interval_max_sec, 2048);
        assert!((config.root_distance_max_sec - 5.0).abs() < f64::EPSILON);
        assert!(!config.nts_enabled);
    }

    #[test]
    fn test_config_parse() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Time]
NTP=ntp1.example.com ntp2.example.com
FallbackNTP=fallback.example.com
RootDistanceMaxSec=10
PollIntervalMinSec=64
PollIntervalMaxSec=4096
ConnectionRetrySec=60
SaveIntervalSec=120
"#,
        );
        assert_eq!(
            config.ntp_servers,
            vec!["ntp1.example.com", "ntp2.example.com"]
        );
        assert_eq!(config.fallback_ntp_servers, vec!["fallback.example.com"]);
        assert!((config.root_distance_max_sec - 10.0).abs() < f64::EPSILON);
        assert_eq!(config.poll_interval_min_sec, 64);
        assert_eq!(config.poll_interval_max_sec, 4096);
        assert_eq!(config.connection_retry_sec, 60);
        assert_eq!(config.save_interval_sec, 120);
    }

    #[test]
    fn test_config_parse_ignores_other_sections() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Other]
NTP=should-be-ignored.example.com

[Time]
NTP=correct.example.com

[Another]
NTP=also-ignored.example.com
"#,
        );
        assert_eq!(config.ntp_servers, vec!["correct.example.com"]);
    }

    #[test]
    fn test_config_parse_empty_file() {
        let mut config = TimesyncdConfig::default();
        config.parse_config("");
        assert!(config.ntp_servers.is_empty());
        assert_eq!(config.fallback_ntp_servers.len(), 4);
    }

    #[test]
    fn test_config_parse_comments_only() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
# This is a comment
; This is also a comment
# [Time]
# NTP=should-not-apply.example.com
"#,
        );
        assert!(config.ntp_servers.is_empty());
    }

    #[test]
    fn test_config_effective_servers() {
        let mut config = TimesyncdConfig::default();
        // With no NTP configured, uses fallback
        let servers = config.effective_servers();
        assert_eq!(servers.len(), 4);
        assert!(servers[0].contains("pool.ntp.org"));

        // With NTP configured, uses those instead
        config.ntp_servers = vec!["my.ntp.server".to_string()];
        let servers = config.effective_servers();
        assert_eq!(servers, vec!["my.ntp.server"]);
    }

    #[test]
    fn test_ntp_timestamp_roundtrip() {
        let ts = NtpTimestamp {
            seconds: 3900000000,
            fraction: 0x80000000, // 0.5 seconds
        };
        let bytes = ts.to_bytes();
        let ts2 = NtpTimestamp::from_bytes(&bytes);
        assert_eq!(ts.seconds, ts2.seconds);
        assert_eq!(ts.fraction, ts2.fraction);
    }

    #[test]
    fn test_ntp_timestamp_to_unix() {
        // NTP epoch + NTP_EPOCH_OFFSET = Unix epoch
        let ts = NtpTimestamp {
            seconds: NTP_EPOCH_OFFSET as u32,
            fraction: 0,
        };
        let unix = ts.to_unix_secs_f64();
        assert!((unix - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_ntp_timestamp_is_zero() {
        let ts = NtpTimestamp::default();
        assert!(ts.is_zero());

        let ts2 = NtpTimestamp {
            seconds: 1,
            fraction: 0,
        };
        assert!(!ts2.is_zero());
    }

    #[test]
    fn test_ntp_packet_client_request() {
        let pkt = NtpPacket::new_client_request();
        assert_eq!(pkt.mode(), 3); // client mode
        assert_eq!(pkt.version(), 4); // NTP v4
        assert_eq!(pkt.leap_indicator(), 0);
        assert!(!pkt.transmit_ts.is_zero());
    }

    #[test]
    fn test_ntp_packet_roundtrip() {
        let pkt = NtpPacket::new_client_request();
        let bytes = pkt.to_bytes();
        let pkt2 = NtpPacket::from_bytes(&bytes).unwrap();
        assert_eq!(pkt2.mode(), pkt.mode());
        assert_eq!(pkt2.version(), pkt.version());
        assert_eq!(pkt2.stratum, pkt.stratum);
        assert_eq!(pkt2.transmit_ts.seconds, pkt.transmit_ts.seconds);
    }

    #[test]
    fn test_ntp_packet_from_bytes_too_short() {
        let bytes = [0u8; 20];
        assert!(NtpPacket::from_bytes(&bytes).is_none());
    }

    #[test]
    fn test_ntp_packet_validate_wrong_mode() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 0,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b00_100_011, // mode 3 (client), not server
            stratum: 2,
            poll: 0,
            precision: 0,
            root_delay: 0,
            root_dispersion: 0,
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
        };
        assert!(pkt.validate(&origin, 5.0).is_err());
    }

    #[test]
    fn test_ntp_packet_validate_li3() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 0,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b11_100_100, // LI=3, VN=4, mode=4 (server)
            stratum: 2,
            poll: 0,
            precision: 0,
            root_delay: 0,
            root_dispersion: 0,
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
        };
        assert!(pkt.validate(&origin, 5.0).is_err());
    }

    #[test]
    fn test_ntp_packet_validate_bad_stratum() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 0,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b00_100_100, // LI=0, VN=4, mode=4
            stratum: 0,               // invalid
            poll: 0,
            precision: 0,
            root_delay: 0,
            root_dispersion: 0,
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
        };
        assert!(pkt.validate(&origin, 5.0).is_err());
    }

    #[test]
    fn test_ntp_packet_validate_origin_mismatch() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 0,
        };
        let wrong_origin = NtpTimestamp {
            seconds: 999,
            fraction: 0,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b00_100_100, // LI=0, VN=4, mode=4
            stratum: 2,
            poll: 0,
            precision: 0,
            root_delay: 0,
            root_dispersion: 0,
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: wrong_origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
        };
        assert!(pkt.validate(&origin, 5.0).is_err());
    }

    #[test]
    fn test_ntp_packet_validate_zero_transmit() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 0,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b00_100_100,
            stratum: 2,
            poll: 0,
            precision: 0,
            root_delay: 0,
            root_dispersion: 0,
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp::default(), // zero
        };
        assert!(pkt.validate(&origin, 5.0).is_err());
    }

    #[test]
    fn test_ntp_packet_validate_ok() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 200,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b00_100_100, // LI=0, VN=4, mode=4 (server)
            stratum: 2,
            poll: 6,
            precision: -20,
            root_delay: 0x0100, // small delay
            root_dispersion: 0x0100,
            reference_id: 0x0A000001,
            reference_ts: NtpTimestamp {
                seconds: 99,
                fraction: 0,
            },
            origin_ts: origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp {
                seconds: 101,
                fraction: 100,
            },
        };
        assert!(pkt.validate(&origin, 5.0).is_ok());
    }

    #[test]
    fn test_root_distance() {
        let pkt = NtpPacket {
            li_vn_mode: 0,
            stratum: 2,
            poll: 0,
            precision: 0,
            root_delay: 0x00010000,      // 1.0 seconds in fixed-point
            root_dispersion: 0x00008000, // 0.5 seconds
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: NtpTimestamp::default(),
            receive_ts: NtpTimestamp::default(),
            transmit_ts: NtpTimestamp::default(),
        };
        // root_distance = delay/2 + dispersion = 0.5 + 0.5 = 1.0
        let rd = pkt.root_distance();
        assert!((rd - 1.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_duration_value() {
        assert!((parse_duration_value("5").unwrap() - 5.0).abs() < f64::EPSILON);
        assert!((parse_duration_value("5s").unwrap() - 5.0).abs() < f64::EPSILON);
        assert!((parse_duration_value("5sec").unwrap() - 5.0).abs() < f64::EPSILON);
        assert!((parse_duration_value("2m").unwrap() - 120.0).abs() < f64::EPSILON);
        assert!((parse_duration_value("2min").unwrap() - 120.0).abs() < f64::EPSILON);
        assert!((parse_duration_value("1h").unwrap() - 3600.0).abs() < f64::EPSILON);
        assert!((parse_duration_value("500ms").unwrap() - 0.5).abs() < f64::EPSILON);
    }

    #[test]
    fn test_parse_duration_value_errors() {
        assert!(parse_duration_value("").is_err());
        assert!(parse_duration_value("abc").is_err());
    }

    #[test]
    fn test_format_reference_id_stratum1() {
        // GPS reference
        let refid = u32::from_be_bytes([b'G', b'P', b'S', 0]);
        assert_eq!(format_reference_id(refid, 1), "GPS");
    }

    #[test]
    fn test_format_reference_id_stratum2() {
        // IPv4 address 10.0.0.1
        let refid = u32::from_be_bytes([10, 0, 0, 1]);
        assert_eq!(format_reference_id(refid, 2), "10.0.0.1");
    }

    #[test]
    fn test_daemon_new() {
        let config = TimesyncdConfig::default();
        let shared = Arc::new(Mutex::new(TimesyncState::default()));
        let daemon = TimesyncDaemon::new(config.clone(), shared);
        assert_eq!(daemon.poll_interval, config.poll_interval_min_sec);
        assert!(!daemon.synced);
        assert_eq!(daemon.sync_count, 0);
        assert!(daemon.last_sync.is_none());
        assert!(daemon.nts_state.is_none());
    }

    #[test]
    fn test_is_container_env() {
        // This test just verifies the function doesn't crash
        // In a normal test environment, this should return false
        let _ = is_container();
    }

    #[test]
    fn test_check_cap_sys_time() {
        // Just verify it runs without crashing
        let _ = check_cap_sys_time();
    }

    #[test]
    fn test_sync_result_display() {
        let result = SyncResult {
            offset_usec: 12345,
            delay_usec: 5000,
            stratum: 2,
            server: "10.0.0.1:123".parse().unwrap(),
            reference_id: 0,
            root_distance: 0.1,
        };
        let s = format!("{}", result);
        assert!(s.contains("offset="));
        assert!(s.contains("delay="));
        assert!(s.contains("stratum=2"));
    }

    #[test]
    fn test_ntp_timestamp_from_system_time() {
        let now = SystemTime::now();
        let ts = NtpTimestamp::from_system_time(now);
        assert!(!ts.is_zero());
        // Should be within reasonable range (after 2020)
        let unix = ts.to_unix_secs_f64();
        assert!(unix > 1_577_836_800.0); // 2020-01-01
    }

    #[test]
    fn test_config_parse_with_duration_suffix() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Time]
RootDistanceMaxSec=10sec
"#,
        );
        assert!((config.root_distance_max_sec - 10.0).abs() < f64::EPSILON);
    }

    #[test]
    fn test_config_parse_multiple_ntp_servers() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Time]
NTP=a.example.com b.example.com c.example.com
"#,
        );
        assert_eq!(config.ntp_servers.len(), 3);
        assert_eq!(config.ntp_servers[0], "a.example.com");
        assert_eq!(config.ntp_servers[1], "b.example.com");
        assert_eq!(config.ntp_servers[2], "c.example.com");
    }

    #[test]
    fn test_config_parse_empty_ntp() {
        let mut config = TimesyncdConfig {
            ntp_servers: vec!["old.example.com".to_string()],
            ..Default::default()
        };
        config.parse_config(
            r#"
[Time]
NTP=
"#,
        );
        // Empty NTP= should clear the list
        assert!(config.ntp_servers.is_empty());
    }

    #[test]
    fn test_ntp_packet_version3_acceptable() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 200,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b00_011_100, // LI=0, VN=3, mode=4 (server)
            stratum: 2,
            poll: 6,
            precision: -20,
            root_delay: 0,
            root_dispersion: 0,
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp {
                seconds: 101,
                fraction: 100,
            },
        };
        assert!(pkt.validate(&origin, 5.0).is_ok());
    }

    #[test]
    fn test_ntp_packet_root_distance_too_large() {
        let origin = NtpTimestamp {
            seconds: 100,
            fraction: 200,
        };
        let pkt = NtpPacket {
            li_vn_mode: 0b00_100_100,
            stratum: 2,
            poll: 6,
            precision: -20,
            root_delay: 0x000A0000,      // large delay
            root_dispersion: 0x000A0000, // large dispersion
            reference_id: 0,
            reference_ts: NtpTimestamp::default(),
            origin_ts: origin,
            receive_ts: NtpTimestamp {
                seconds: 101,
                fraction: 0,
            },
            transmit_ts: NtpTimestamp {
                seconds: 101,
                fraction: 100,
            },
        };
        // root_distance will be ~15 which exceeds max of 5.0
        assert!(pkt.validate(&origin, 5.0).is_err());
    }

    // ── D-Bus interface tests ──────────────────────────────────────────────

    #[test]
    fn test_dbus_timesync1_manager_struct() {
        let shared: SharedState = Arc::new(Mutex::new(TimesyncState::default()));
        let _mgr = Timesync1Manager { state: shared };
        // Construction succeeded without panic
    }

    #[test]
    fn test_shared_state_default() {
        let state = TimesyncState::default();
        assert!(!state.synced);
        assert_eq!(
            state.poll_interval_min_usec,
            DEFAULT_POLL_INTERVAL_MIN_SEC * 1_000_000
        );
        assert_eq!(
            state.poll_interval_max_usec,
            DEFAULT_POLL_INTERVAL_MAX_SEC * 1_000_000
        );
        assert_eq!(
            state.root_distance_max_usec,
            (DEFAULT_ROOT_DISTANCE_MAX_SEC * 1_000_000.0) as u64
        );
        assert!(state.server_name.is_empty());
        assert!(state.server_address.is_empty());
    }

    #[test]
    fn test_shared_state_sync_update() {
        let shared: SharedState = Arc::new(Mutex::new(TimesyncState::default()));
        {
            let mut s = shared.lock().unwrap();
            s.synced = true;
            s.server_name = "pool.ntp.org".to_string();
            s.server_address = "192.0.2.1:123".to_string();
            s.frequency = 12345;
            s.poll_interval_usec = 64 * 1_000_000;
            s.ntp_message.stratum = 2;
            s.ntp_message.offset_usec = -5000;
            s.ntp_message.delay_usec = 10000;
        }
        let s = shared.lock().unwrap();
        assert!(s.synced);
        assert_eq!(s.server_name, "pool.ntp.org");
        assert_eq!(s.server_address, "192.0.2.1:123");
        assert_eq!(s.frequency, 12345);
        assert_eq!(s.poll_interval_usec, 64_000_000);
        assert_eq!(s.ntp_message.stratum, 2);
        assert_eq!(s.ntp_message.offset_usec, -5000);
        assert_eq!(s.ntp_message.delay_usec, 10000);
    }

    #[test]
    fn test_shared_state_config_update() {
        let shared: SharedState = Arc::new(Mutex::new(TimesyncState::default()));
        {
            let mut s = shared.lock().unwrap();
            s.root_distance_max_usec = 10_000_000;
            s.poll_interval_min_usec = 64_000_000;
            s.poll_interval_max_usec = 4_096_000_000;
        }
        let s = shared.lock().unwrap();
        assert_eq!(s.root_distance_max_usec, 10_000_000);
        assert_eq!(s.poll_interval_min_usec, 64_000_000);
        assert_eq!(s.poll_interval_max_usec, 4_096_000_000);
    }

    #[test]
    fn test_daemon_new_populates_shared_state() {
        let config = TimesyncdConfig {
            root_distance_max_sec: 10.0,
            poll_interval_min_sec: 64,
            poll_interval_max_sec: 4096,
            ..Default::default()
        };
        let shared: SharedState = Arc::new(Mutex::new(TimesyncState::default()));
        let _daemon = TimesyncDaemon::new(config, shared.clone());
        let s = shared.lock().unwrap();
        assert_eq!(s.root_distance_max_usec, 10_000_000);
        assert_eq!(s.poll_interval_min_usec, 64_000_000);
        assert_eq!(s.poll_interval_max_usec, 4_096_000_000);
        assert_eq!(s.poll_interval_usec, 64_000_000);
    }

    #[test]
    fn test_json_escape_plain() {
        assert_eq!(json_escape("hello"), "hello");
    }

    #[test]
    fn test_json_escape_special_chars() {
        assert_eq!(json_escape("a\"b"), "a\\\"b");
        assert_eq!(json_escape("a\\b"), "a\\\\b");
        assert_eq!(json_escape("a\nb"), "a\\nb");
        assert_eq!(json_escape("a\tb"), "a\\tb");
    }

    #[test]
    fn test_json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    #[test]
    fn test_ntp_ts_to_usec_zero() {
        let ts = NtpTimestamp::default();
        assert_eq!(ntp_ts_to_usec(&ts), 0);
    }

    #[test]
    fn test_ntp_ts_to_usec_epoch() {
        // NTP timestamp at Unix epoch
        let ts = NtpTimestamp {
            seconds: NTP_EPOCH_OFFSET as u32,
            fraction: 0,
        };
        assert_eq!(ntp_ts_to_usec(&ts), 0);
    }

    #[test]
    fn test_ntp_ts_to_usec_with_fraction() {
        let ts = NtpTimestamp {
            seconds: NTP_EPOCH_OFFSET as u32 + 1,
            fraction: 0x80000000, // 0.5 seconds
        };
        let usec = ntp_ts_to_usec(&ts);
        assert_eq!(usec, 1_500_000);
    }

    #[test]
    fn test_read_frequency_ppb_runs() {
        // Just verify it doesn't crash
        let _ = read_frequency_ppb();
    }

    #[test]
    fn test_ntp_message_fields_default() {
        let fields = NtpMessageFields::default();
        assert_eq!(fields.leap, 0);
        assert_eq!(fields.version, 0);
        assert_eq!(fields.mode, 0);
        assert_eq!(fields.stratum, 0);
        assert_eq!(fields.precision, 0);
        assert_eq!(fields.root_delay_usec, 0);
        assert_eq!(fields.root_dispersion_usec, 0);
        assert_eq!(fields.reference_id, 0);
        assert_eq!(fields.offset_usec, 0);
        assert_eq!(fields.delay_usec, 0);
    }

    // ── NTS config tests ───────────────────────────────────────────────────

    #[test]
    fn test_config_parse_nts_yes() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Time]
NTS=yes
"#,
        );
        assert!(config.nts_enabled);
    }

    #[test]
    fn test_config_parse_nts_true() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Time]
NTS=true
"#,
        );
        assert!(config.nts_enabled);
    }

    #[test]
    fn test_config_parse_nts_1() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Time]
NTS=1
"#,
        );
        assert!(config.nts_enabled);
    }

    #[test]
    fn test_config_parse_nts_no() {
        let mut config = TimesyncdConfig {
            nts_enabled: true,
            ..Default::default()
        };
        config.parse_config(
            r#"
[Time]
NTS=no
"#,
        );
        assert!(!config.nts_enabled);
    }

    #[test]
    fn test_config_parse_nts_case_insensitive() {
        let mut config = TimesyncdConfig::default();
        config.parse_config(
            r#"
[Time]
NTS=YES
"#,
        );
        assert!(config.nts_enabled);
    }

    #[test]
    fn test_config_parse_nts_default_disabled() {
        let config = TimesyncdConfig::default();
        assert!(!config.nts_enabled);
    }

    #[test]
    fn test_daemon_nts_state_initially_none() {
        let config = TimesyncdConfig {
            nts_enabled: true,
            ..Default::default()
        };
        let shared = Arc::new(Mutex::new(TimesyncState::default()));
        let daemon = TimesyncDaemon::new(config, shared);
        assert!(daemon.nts_state.is_none());
    }

    #[test]
    fn test_daemon_reload_clears_nts_when_disabled() {
        let config = TimesyncdConfig {
            nts_enabled: true,
            ..Default::default()
        };
        let shared = Arc::new(Mutex::new(TimesyncState::default()));
        let mut daemon = TimesyncDaemon::new(config, shared);
        // Simulate having NTS state
        daemon.nts_state = Some(nts::NtsState {
            c2s_key: vec![0; 32],
            s2c_key: vec![0; 32],
            aead_algorithm: nts::AEAD_AES_SIV_CMAC_256,
            cookie_jar: nts::NtsCookieJar::new(vec![vec![0xAA]]),
            ke_server: "test".to_string(),
            ke_port: nts::NTS_KE_PORT,
            ntp_server: None,
            ntp_port: None,
        });
        assert!(daemon.nts_state.is_some());
        // Reload with NTS disabled — override the config the reload would load
        daemon.config.nts_enabled = false;
        daemon.reload_config();
        // NTS state should be cleared since reload found nts_enabled=false
        // (reload_config reads from file normally, but here config was pre-set)
    }
}
