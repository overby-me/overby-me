//! systemd-timedated — time and date management daemon
//!
//! Manages system-wide time and date configuration:
//! - **Timezone**: stored as `/etc/localtime` symlink pointing into `/usr/share/zoneinfo/`
//!   and optionally `/etc/timezone` as a plain-text timezone name
//! - **RTC mode**: stored in `/etc/adjtime` — UTC (default) or LOCAL
//! - **NTP**: controls `systemd-timesyncd.service` via `systemctl`
//!
//! The daemon listens on a Unix control socket for commands from `timedatectl`.
//! It sends sd_notify READY/STATUS/STOPPING/WATCHDOG messages and handles
//! SIGTERM/SIGINT for shutdown and SIGHUP for configuration reload.

use std::env;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zbus::blocking::Connection;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const DEFAULT_LOCALTIME_PATH: &str = "/etc/localtime";
const DEFAULT_TIMEZONE_PATH: &str = "/etc/timezone";
const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";
const DEFAULT_ADJTIME_PATH: &str = "/etc/adjtime";
const CONTROL_SOCKET_PATH: &str = "/run/systemd/timedated.sock";

/// Return the effective localtime symlink path, honoring `SYSTEMD_ETC_LOCALTIME`.
fn localtime_path() -> String {
    env::var("SYSTEMD_ETC_LOCALTIME").unwrap_or_else(|_| DEFAULT_LOCALTIME_PATH.to_string())
}

/// Return the effective timezone file path, honoring `SYSTEMD_ETC_TIMEZONE`.
fn timezone_path() -> String {
    env::var("SYSTEMD_ETC_TIMEZONE").unwrap_or_else(|_| DEFAULT_TIMEZONE_PATH.to_string())
}

/// Return the effective adjtime file path, honoring `SYSTEMD_ETC_ADJTIME`.
fn adjtime_path() -> String {
    env::var("SYSTEMD_ETC_ADJTIME").unwrap_or_else(|_| DEFAULT_ADJTIME_PATH.to_string())
}

const DBUS_NAME: &str = "org.freedesktop.timedate1";
const DBUS_PATH: &str = "/org/freedesktop/timedate1";

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// All time/date state managed by the daemon.
#[derive(Debug, Clone, PartialEq)]
pub struct TimedateState {
    /// Timezone name (e.g. "America/New_York"), or "UTC" if unknown
    pub timezone: String,
    /// Whether the RTC is set to local time (false = UTC, which is the default)
    pub local_rtc: bool,
    /// Whether NTP synchronization is enabled (systemd-timesyncd.service)
    pub ntp_enabled: bool,
    /// Whether the clock is currently synchronized via NTP
    pub ntp_synced: bool,
    /// Whether NTP is available (timesyncd binary exists)
    pub can_ntp: bool,
}

impl Default for TimedateState {
    fn default() -> Self {
        Self {
            timezone: "UTC".to_string(),
            local_rtc: false,
            ntp_enabled: false,
            ntp_synced: false,
            can_ntp: false,
        }
    }
}

impl TimedateState {
    /// Load all time/date state from the filesystem.
    pub fn load() -> Self {
        Self::load_from(
            &localtime_path(),
            &timezone_path(),
            &adjtime_path(),
            ZONEINFO_DIR,
        )
    }

    /// Load state from custom paths (for testing).
    pub fn load_from(
        localtime_path: &str,
        timezone_path: &str,
        adjtime_path: &str,
        zoneinfo_dir: &str,
    ) -> Self {
        let timezone = detect_timezone_from(localtime_path, timezone_path, zoneinfo_dir);
        let local_rtc = is_rtc_local_from(adjtime_path);
        let ntp_enabled = is_ntp_enabled();
        let ntp_synced = is_ntp_synced();
        let can_ntp = can_ntp();

        Self {
            timezone,
            local_rtc,
            ntp_enabled,
            ntp_synced,
            can_ntp,
        }
    }

    /// Format a human-readable status output (like `timedatectl status`).
    pub fn format_status(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let secs = now.as_secs() as i64;

        let local_time = format_timestamp_local(secs, &self.timezone);
        let utc_time = format_timestamp_utc(secs);
        let (tz_abbr, utc_offset) = get_tz_abbr_and_offset(&self.timezone);
        let rtc_time = read_rtc_time();

        let yes_no = |b: bool| if b { "yes" } else { "no" };

        let mut out = String::new();
        out.push_str(&format!("               Local time: {}\n", local_time));
        out.push_str(&format!("           Universal time: {}\n", utc_time));
        if let Some(ref rtc) = rtc_time {
            out.push_str(&format!("                 RTC time: {}\n", rtc));
        } else {
            out.push_str("                 RTC time: n/a\n");
        }
        out.push_str(&format!(
            "                Time zone: {} ({}, {})\n",
            self.timezone, tz_abbr, utc_offset
        ));
        out.push_str(&format!(
            "System clock synchronized: {}\n",
            yes_no(self.ntp_synced)
        ));
        out.push_str(&format!(
            "              NTP service: {}\n",
            if self.ntp_enabled {
                "active"
            } else {
                "inactive"
            }
        ));
        out.push_str(&format!(
            "          RTC in local TZ: {}\n",
            yes_no(self.local_rtc)
        ));
        out
    }

    /// Format machine-readable key=value output (like `timedatectl show`).
    pub fn format_show(&self) -> String {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        let usec = now.as_secs() * 1_000_000 + now.subsec_micros() as u64;

        let yes_no = |b: bool| if b { "yes" } else { "no" };

        let mut out = String::new();
        out.push_str(&format!("Timezone={}\n", self.timezone));
        out.push_str(&format!("LocalRTC={}\n", yes_no(self.local_rtc)));
        out.push_str(&format!("CanNTP={}\n", yes_no(self.can_ntp)));
        out.push_str(&format!("NTP={}\n", yes_no(self.ntp_enabled)));
        out.push_str(&format!("NTPSynchronized={}\n", yes_no(self.ntp_synced)));
        out.push_str(&format!("TimeUSec={}\n", usec));
        out.push_str("RTCTimeUSec=n/a\n");
        out
    }
}

// ---------------------------------------------------------------------------
// Timezone detection
// ---------------------------------------------------------------------------

/// Detect timezone from custom paths (for testing).
pub fn detect_timezone_from(
    localtime_path: &str,
    timezone_path: &str,
    zoneinfo_dir: &str,
) -> String {
    // Try /etc/timezone first (plain text)
    if let Ok(content) = fs::read_to_string(timezone_path) {
        let tz = content.trim().to_string();
        if !tz.is_empty() {
            return tz;
        }
    }

    // Try resolving /etc/localtime symlink
    if let Ok(target) = fs::read_link(localtime_path) {
        let target_str = target.to_string_lossy().to_string();
        // Look for ../usr/share/zoneinfo/ or /usr/share/zoneinfo/ prefix
        for prefix in &[
            &format!("{}/", zoneinfo_dir),
            "../usr/share/zoneinfo/",
            "../../usr/share/zoneinfo/",
        ] {
            if let Some(tz) = target_str.strip_prefix(prefix)
                && !tz.is_empty()
            {
                return tz.to_string();
            }
        }
        // Also handle posix/right subdirectories
        for prefix in &[
            &format!("{}/posix/", zoneinfo_dir) as &str,
            &format!("{}/right/", zoneinfo_dir),
        ] {
            if let Some(tz) = target_str.strip_prefix(prefix)
                && !tz.is_empty()
            {
                return tz.to_string();
            }
        }
    }

    // Try TZ environment variable
    if let Ok(tz) = env::var("TZ") {
        let tz = tz.trim_start_matches(':').to_string();
        if !tz.is_empty() {
            return tz;
        }
    }

    "UTC".to_string()
}

/// Validate that a timezone name exists in the zoneinfo database.
pub fn is_valid_timezone(tz: &str) -> bool {
    is_valid_timezone_in(tz, ZONEINFO_DIR)
}

/// Validate timezone against a custom zoneinfo directory.
pub fn is_valid_timezone_in(tz: &str, zoneinfo_dir: &str) -> bool {
    if tz.is_empty() {
        return false;
    }
    // Reject path traversal
    if tz.contains("..") || tz.starts_with('/') {
        return false;
    }
    // Also check TZDIR env
    let tzdir = env::var("TZDIR").ok();

    let primary = Path::new(zoneinfo_dir).join(tz);
    let alt = tzdir.as_ref().map(|d| Path::new(d).join(tz));

    primary.exists() || alt.as_ref().is_some_and(|p| p.exists())
}

/// List available timezones from the zoneinfo database.
pub fn list_timezones() -> Vec<String> {
    list_timezones_from(ZONEINFO_DIR)
}

/// List timezones from a custom zoneinfo directory.
pub fn list_timezones_from(zoneinfo_dir: &str) -> Vec<String> {
    let mut tzs = Vec::new();
    collect_timezones(Path::new(zoneinfo_dir), Path::new(zoneinfo_dir), &mut tzs);
    tzs.sort();
    tzs
}

fn collect_timezones(base: &Path, dir: &Path, out: &mut Vec<String>) {
    let entries = match fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();

        // Skip special directories and files
        if name.starts_with('.')
            || name == "posix"
            || name == "right"
            || name == "posixrules"
            || name == "leap-seconds.list"
            || name == "leapseconds"
            || name == "+VERSION"
            || name == "tzdata.zi"
            || name == "zone.tab"
            || name == "zone1970.tab"
            || name == "iso3166.tab"
            || name == "factory"
            || name == "localtime"
        {
            continue;
        }

        if path.is_dir() {
            collect_timezones(base, &path, out);
        } else if path.is_file()
            && let Ok(rel) = path.strip_prefix(base)
        {
            let tz = rel.to_string_lossy().to_string();
            // Filter out uppercase-only names (like EST, HST) since
            // those are POSIX compat, not IANA region/city names.
            // Keep names that contain a slash (region/city).
            if tz.contains('/') {
                out.push(tz);
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Timezone management
// ---------------------------------------------------------------------------

/// Set the system timezone by updating /etc/localtime and /etc/timezone.
pub fn set_timezone(tz: &str) -> Result<(), String> {
    set_timezone_at(tz, &localtime_path(), &timezone_path(), ZONEINFO_DIR)
}

/// Set the timezone using custom paths (for testing).
pub fn set_timezone_at(
    tz: &str,
    localtime_path: &str,
    timezone_path: &str,
    zoneinfo_dir: &str,
) -> Result<(), String> {
    if tz.is_empty() {
        return Err("Timezone cannot be empty".to_string());
    }
    if tz.contains("..") || tz.starts_with('/') {
        return Err(format!("Invalid timezone name '{}'", tz));
    }

    let zoneinfo = Path::new(zoneinfo_dir).join(tz);
    if !zoneinfo.exists() {
        // Also check TZDIR env
        let alt = env::var("TZDIR")
            .ok()
            .map(|d| Path::new(&d).join(tz).to_path_buf());
        if !alt.as_ref().is_some_and(|p| p.exists()) {
            return Err(format!("Timezone '{}' not found", tz));
        }
    }

    // Remove existing /etc/localtime
    let _ = fs::remove_file(localtime_path);

    // Create symlink to zoneinfo file
    #[cfg(unix)]
    {
        let target = Path::new(zoneinfo_dir).join(tz);
        std::os::unix::fs::symlink(&target, localtime_path)
            .map_err(|e| format!("Failed to create localtime symlink: {}", e))?;
    }

    // Write /etc/timezone
    if let Err(e) = fs::write(timezone_path, format!("{}\n", tz)) {
        log::warn!("Could not write {}: {} (non-fatal)", timezone_path, e);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// RTC local/UTC management
// ---------------------------------------------------------------------------

/// Check whether the RTC is configured for local time.
pub fn is_rtc_local() -> bool {
    is_rtc_local_from(&adjtime_path())
}

/// Check RTC mode from a custom adjtime path (for testing).
pub fn is_rtc_local_from(adjtime_path: &str) -> bool {
    if let Ok(content) = fs::read_to_string(adjtime_path) {
        // The third line of /etc/adjtime is either "UTC" or "LOCAL"
        if let Some(third_line) = content.lines().nth(2) {
            return third_line.trim().eq_ignore_ascii_case("LOCAL");
        }
    }
    false // Default is UTC
}

/// Set the RTC to local or UTC mode by writing /etc/adjtime.
pub fn set_local_rtc(local: bool) -> Result<(), String> {
    set_local_rtc_at(local, &adjtime_path())
}

/// Set RTC mode using a custom adjtime path (for testing).
pub fn set_local_rtc_at(local: bool, adjtime_path: &str) -> Result<(), String> {
    const NULL_ADJTIME_UTC: &str = "0.0 0 0\n0\nUTC\n";

    let content = match fs::read_to_string(adjtime_path) {
        Ok(s) => s,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            if !local {
                return Ok(()); // No file + UTC is the default — nothing to do
            }
            return fs::write(adjtime_path, "0.0 0 0\n0\nLOCAL\n")
                .map_err(|e| format!("Failed to write {}: {}", adjtime_path, e));
        }
        Err(e) => return Err(format!("Failed to read {}: {}", adjtime_path, e)),
    };

    let mode = if local { "LOCAL" } else { "UTC" };
    let bytes = content.as_bytes();

    // Build new content following the upstream C logic:
    // Find the boundary between the prefix (lines 1-2) and the mode line (line 3),
    // then reconstruct with the new mode and any suffix (lines 4+).
    let new_content = match bytes.iter().position(|&b| b == b'\n') {
        None => {
            // "line0" — no newlines at all
            format!("{}\n0\n{}\n", content, mode)
        }
        Some(pos1) if pos1 + 1 >= bytes.len() => {
            // "line0\n" — only one line with trailing newline
            format!("{}0\n{}\n", content, mode)
        }
        Some(pos1) => {
            match bytes[pos1 + 1..].iter().position(|&b| b == b'\n') {
                None => {
                    // "line0\nline1" — two lines, no trailing newline
                    format!("{}\n{}\n", content, mode)
                }
                Some(rel_pos2) => {
                    let pos2 = pos1 + 1 + rel_pos2;
                    let prefix = &content[..pos2 + 1]; // through second \n
                    // Find end of line 3 (third \n)
                    let suffix = match bytes[pos2 + 1..].iter().position(|&b| b == b'\n') {
                        Some(rel_pos3) => &content[pos2 + 1 + rel_pos3..],
                        None => "\n",
                    };
                    format!("{}{}{}", prefix, mode, suffix)
                }
            }
        }
    };

    // If the rebuilt content is identical to the original, leave the file as-is.
    // This mirrors the upstream daemon's early-return when the mode already matches.
    if new_content == content {
        return Ok(());
    }

    if new_content == NULL_ADJTIME_UTC {
        match fs::remove_file(adjtime_path) {
            Ok(()) => return Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
            Err(e) => return Err(format!("Failed to remove {}: {}", adjtime_path, e)),
        }
    }

    fs::write(adjtime_path, &new_content)
        .map_err(|e| format!("Failed to write {}: {}", adjtime_path, e))
}

// ---------------------------------------------------------------------------
// NTP management
// ---------------------------------------------------------------------------

/// Check if NTP synchronization is enabled (systemd-timesyncd.service is enabled).
pub fn is_ntp_enabled() -> bool {
    // Check if the service is active or enabled
    let status = process::Command::new("systemctl")
        .args(["is-active", "systemd-timesyncd.service"])
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::null())
        .status();
    if let Ok(s) = status
        && s.success()
    {
        return true;
    }

    // Fall back to checking if the service is enabled
    let status = process::Command::new("systemctl")
        .args(["is-enabled", "systemd-timesyncd.service"])
        .stdout(process::Stdio::piped())
        .stderr(process::Stdio::null())
        .status();
    matches!(status, Ok(s) if s.success())
}

/// Check if the clock is synchronized via NTP.
pub fn is_ntp_synced() -> bool {
    // Check adjtimex STA_UNSYNC flag
    unsafe {
        let mut tx: libc::timex = std::mem::zeroed();
        tx.modes = 0;
        let rc = libc::adjtimex(&mut tx);
        if rc >= 0 {
            // STA_UNSYNC = 0x0040 — if this flag is NOT set, clock is synced
            return (tx.status & 0x0040) == 0;
        }
    }
    false
}

/// Check if NTP is available (timesyncd binary exists).
pub fn can_ntp() -> bool {
    // Check well-known paths for systemd-timesyncd
    for path in &[
        "/lib/systemd/systemd-timesyncd",
        "/usr/lib/systemd/systemd-timesyncd",
    ] {
        if Path::new(path).exists() {
            return true;
        }
    }

    // Also check relative to our own executable (for NixOS)
    if let Ok(exe) = env::current_exe()
        && let Some(parent) = exe.parent()
    {
        if parent.join("systemd-timesyncd").exists() {
            return true;
        }
        // Check ../lib/systemd/systemd-timesyncd
        if let Some(pp) = parent.parent()
            && pp.join("lib/systemd/systemd-timesyncd").exists()
        {
            return true;
        }
    }

    false
}

/// Enable or disable NTP synchronization by controlling systemd-timesyncd.service.
pub fn set_ntp(enable: bool) -> Result<(), String> {
    let enable_str = if enable { "enable" } else { "disable" };
    let action = if enable { "start" } else { "stop" };

    // Enable/disable the service
    let status = process::Command::new("systemctl")
        .args([enable_str, "systemd-timesyncd.service"])
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::piped())
        .status();

    match status {
        Ok(s) if s.success() => {}
        Ok(s) => {
            return Err(format!(
                "systemctl {} failed with exit code {}",
                enable_str,
                s.code().unwrap_or(-1)
            ));
        }
        Err(e) => {
            return Err(format!("Failed to run systemctl {}: {}", enable_str, e));
        }
    }

    // Start/stop the service
    let status = process::Command::new("systemctl")
        .args([action, "systemd-timesyncd.service"])
        .stdout(process::Stdio::null())
        .stderr(process::Stdio::piped())
        .status();

    match status {
        Ok(s) if s.success() => Ok(()),
        Ok(s) => {
            // Non-fatal: the enable/disable already succeeded
            log::warn!(
                "systemctl {} exited with code {}",
                action,
                s.code().unwrap_or(-1)
            );
            Ok(())
        }
        Err(e) => {
            log::warn!("Failed to run systemctl {}: {}", action, e);
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Time formatting helpers
// ---------------------------------------------------------------------------

/// Read the RTC time if available.
fn read_rtc_time() -> Option<String> {
    fs::read_to_string("/sys/class/rtc/rtc0/time")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// Sync the process's TZ environment with the timezone we manage.
///
/// This ensures that subsequent `localtime_r` calls (used by `format_status`)
/// reflect the timezone configured via `SYSTEMD_ETC_LOCALTIME`, not necessarily
/// `/etc/localtime`.
/// Sync the process TZ with the given timezone name or localtime file path.
///
/// Sets `TZ=:<localtime-path>` so glibc resolves the timezone directly from
/// the localtime symlink, then calls `tzset()` to reload.
fn sync_process_tz(_tz: &str) {
    unsafe extern "C" {
        fn tzset();
    }

    let lt_path = localtime_path();
    if !Path::new(&lt_path).exists() {
        return;
    }

    unsafe {
        env::set_var("TZ", format!(":{}", lt_path));
        tzset();
    }
}

/// Get the UTC offset and abbreviation for a given timezone name.
///
/// Uses `date` subprocess with `TZ=<name>` to resolve the timezone, avoiding
/// glibc TZ caching issues within the long-running daemon process.
fn get_tz_abbr_and_offset(tz_name: &str) -> (String, String) {
    if let Ok(output) = process::Command::new("date")
        .env("TZ", tz_name)
        .arg("+%Z %z")
        .output()
        && output.status.success()
    {
        let s = String::from_utf8_lossy(&output.stdout);
        let parts: Vec<&str> = s.trim().splitn(2, ' ').collect();
        if parts.len() == 2 {
            return (parts[0].to_string(), parts[1].to_string());
        }
    }
    ("UTC".to_string(), "+0000".to_string())
}

/// Format a Unix timestamp as a local time string.
fn format_timestamp_local(secs: i64, tz_name: &str) -> String {
    if let Ok(output) = process::Command::new("date")
        .env("TZ", tz_name)
        .arg("-d")
        .arg(format!("@{}", secs))
        .arg("+%a %Y-%b-%d %H:%M:%S")
        .output()
        && output.status.success()
    {
        return String::from_utf8_lossy(&output.stdout).trim().to_string();
    }
    // Fallback
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::localtime_r(&secs, &mut tm);
        format_tm(&tm)
    }
}

/// Format a Unix timestamp as a UTC time string.
fn format_timestamp_utc(secs: i64) -> String {
    unsafe {
        let mut tm: libc::tm = std::mem::zeroed();
        libc::gmtime_r(&secs, &mut tm);
        format_tm(&tm)
    }
}

fn format_tm(tm: &libc::tm) -> String {
    let weekdays = ["Sun", "Mon", "Tue", "Wed", "Thu", "Fri", "Sat"];
    let months = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];

    let wday = tm.tm_wday as usize % 7;
    let mon = tm.tm_mon as usize % 12;

    format!(
        "{} {}-{:02}-{:02} {:02}:{:02}:{:02}",
        weekdays[wday],
        tm.tm_year + 1900,
        months[mon],
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec
    )
}

// ---------------------------------------------------------------------------
// Control socket command handler
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// Shared state for D-Bus
// ---------------------------------------------------------------------------

type SharedState = Arc<Mutex<TimedateState>>;

// ---------------------------------------------------------------------------
// D-Bus interface: org.freedesktop.timedate1
// ---------------------------------------------------------------------------

/// D-Bus interface struct for org.freedesktop.timedate1.
///
/// Properties (read-only):
///   Timezone, LocalRTC, CanNTP, NTP, NTPSynchronized, TimeUSec, RTCTimeUSec
///
/// Methods:
///   SetTime(x usec_utc, b relative, b interactive)
///   SetTimezone(s timezone, b interactive)
///   SetLocalRTC(b local_rtc, b fix_system, b interactive)
///   SetNTP(b use_ntp, b interactive)
///   ListTimezones() → as
struct Timedate1Manager {
    state: SharedState,
}

#[zbus::interface(name = "org.freedesktop.timedate1")]
impl Timedate1Manager {
    // --- Properties (read-only) ---

    #[zbus(property, name = "Timezone")]
    fn timezone(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.timezone.clone()
    }

    #[zbus(property, name = "LocalRTC")]
    fn local_rtc(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.local_rtc
    }

    #[zbus(property, name = "CanNTP")]
    fn can_ntp(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.can_ntp
    }

    #[zbus(property, name = "NTP")]
    fn ntp(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.ntp_enabled
    }

    #[zbus(property, name = "NTPSynchronized")]
    fn ntp_synchronized(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.ntp_synced
    }

    #[zbus(property, name = "TimeUSec")]
    fn time_usec(&self) -> u64 {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default();
        now.as_micros() as u64
    }

    #[zbus(property, name = "RTCTimeUSec")]
    fn rtc_time_usec(&self) -> u64 {
        read_rtc_time()
            .and_then(|s| {
                let parts: Vec<&str> = s.split(':').collect();
                if parts.len() == 3 {
                    let h: u64 = parts[0].parse().ok()?;
                    let m: u64 = parts[1].parse().ok()?;
                    let s: u64 = parts[2].parse().ok()?;
                    Some((h * 3600 + m * 60 + s) * 1_000_000)
                } else {
                    None
                }
            })
            .unwrap_or(0u64)
    }

    // --- Methods ---

    fn set_time(&self, usec_utc: i64, relative: bool, _interactive: bool) -> zbus::fdo::Result<()> {
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if s.ntp_enabled {
                return Err(zbus::fdo::Error::Failed(
                    "Automatic time synchronization is enabled".into(),
                ));
            }
        }

        let new_time = if relative {
            let now = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default();
            let now_usec = now.as_micros() as i64;
            now_usec.saturating_add(usec_utc)
        } else {
            usec_utc
        };

        if new_time < 0 {
            return Err(zbus::fdo::Error::Failed("Time value out of range".into()));
        }

        let secs = new_time / 1_000_000;
        let nsecs = (new_time % 1_000_000) * 1000;
        let ts = libc::timespec {
            tv_sec: secs,
            tv_nsec: nsecs,
        };
        let ret = unsafe { libc::clock_settime(libc::CLOCK_REALTIME, &ts) };
        if ret != 0 {
            return Err(zbus::fdo::Error::Failed(
                "Failed to set system clock".into(),
            ));
        }

        Ok(())
    }

    fn set_timezone(&self, timezone: String, _interactive: bool) -> zbus::fdo::Result<()> {
        if timezone.is_empty() {
            return Err(zbus::fdo::Error::Failed(
                "Timezone must not be empty".into(),
            ));
        }
        if !is_valid_timezone(&timezone) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Invalid timezone '{}'",
                timezone
            )));
        }
        if let Err(e) = set_timezone(&timezone) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set timezone: {}",
                e
            )));
        }
        sync_process_tz(&timezone);
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.timezone = timezone;
        Ok(())
    }

    #[zbus(name = "SetLocalRTC")]
    fn set_local_rtc(
        &self,
        local_rtc: bool,
        fix_system: bool,
        _interactive: bool,
    ) -> zbus::fdo::Result<()> {
        {
            let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if local_rtc == s.local_rtc && !fix_system {
                return Ok(());
            }
        }
        if let Err(e) = set_local_rtc(local_rtc) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set local RTC: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.local_rtc = local_rtc;
        Ok(())
    }

    #[zbus(name = "SetNTP")]
    fn set_ntp(&self, use_ntp: bool, _interactive: bool) -> zbus::fdo::Result<()> {
        if let Err(e) = set_ntp(use_ntp) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set NTP: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.ntp_enabled = use_ntp;
        Ok(())
    }

    fn list_timezones(&self) -> Vec<String> {
        list_timezones()
    }
}

/// Set up the D-Bus connection and register the timedate1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedState) -> Result<Connection, String> {
    let iface = Timedate1Manager { state: shared };
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

// ---------------------------------------------------------------------------
// Control socket command handler
// ---------------------------------------------------------------------------

/// Handle a single control command and return the response string.
///
/// When `state` is provided, stateful early-return optimisations are used
/// (e.g. SET-LOCAL-RTC skips the write when the mode already matches).
pub fn handle_control_command(line: &str, state: Option<&SharedState>) -> String {
    let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
    let cmd = parts.first().copied().unwrap_or("");

    match cmd.to_ascii_uppercase().as_str() {
        "STATUS" => {
            let s = state
                .map(|st| st.lock().unwrap_or_else(|e| e.into_inner()).clone())
                .unwrap_or_else(TimedateState::load);
            s.format_status()
        }
        "SHOW" => {
            let s = state
                .map(|st| st.lock().unwrap_or_else(|e| e.into_inner()).clone())
                .unwrap_or_else(TimedateState::load);
            s.format_show()
        }
        "SET-TIMEZONE" => {
            let tz = parts.get(1).unwrap_or(&"");
            if tz.is_empty() {
                return "ERROR: Timezone argument required\n".to_string();
            }
            match set_timezone(tz) {
                Ok(()) => {
                    sync_process_tz(tz);
                    if let Some(st) = state {
                        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
                        s.timezone = tz.to_string();
                    }
                    log::info!("Timezone set to '{}'", tz);
                    "OK\n".to_string()
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-LOCAL-RTC" => {
            let val = parts.get(1).unwrap_or(&"");
            let local = match val.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" | "on" => true,
                "false" | "no" | "0" | "off" => false,
                "" => return "ERROR: Boolean argument required\n".to_string(),
                _ => return format!("ERROR: Invalid boolean value '{}'\n", val),
            };
            // Early return if the mode already matches (mirrors upstream behaviour)
            if let Some(st) = state {
                let s = st.lock().unwrap_or_else(|e| e.into_inner());
                if local == s.local_rtc {
                    return "OK\n".to_string();
                }
            }
            match set_local_rtc(local) {
                Ok(()) => {
                    if let Some(st) = state {
                        let mut s = st.lock().unwrap_or_else(|e| e.into_inner());
                        s.local_rtc = local;
                    }
                    log::info!("RTC in local TZ: {}", local);
                    "OK\n".to_string()
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-NTP" => {
            let val = parts.get(1).unwrap_or(&"");
            let enable = match val.to_ascii_lowercase().as_str() {
                "true" | "yes" | "1" | "on" => true,
                "false" | "no" | "0" | "off" => false,
                "" => return "ERROR: Boolean argument required\n".to_string(),
                _ => return format!("ERROR: Invalid boolean value '{}'\n", val),
            };
            match set_ntp(enable) {
                Ok(()) => {
                    log::info!("NTP {}", if enable { "enabled" } else { "disabled" });
                    "OK\n".to_string()
                }
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "LIST-TIMEZONES" => {
            let tzs = list_timezones();
            let mut out = String::new();
            for tz in &tzs {
                out.push_str(tz);
                out.push('\n');
            }
            out
        }
        "PING" => "PONG\n".to_string(),
        _ => format!("ERROR: Unknown command '{}'\n", cmd),
    }
}

/// Handle a client connection on the control socket.
fn handle_client(stream: &mut UnixStream, state: Option<&SharedState>) {
    let reader = BufReader::new(match stream.try_clone() {
        Ok(s) => s,
        Err(_) => return,
    });

    for line in reader.lines() {
        match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => {
                let response = handle_control_command(&l, state);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// sd_notify protocol
// ---------------------------------------------------------------------------

fn sd_notify(msg: &str) {
    if let Ok(path) = env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = path.strip_prefix('@') {
            // Abstract socket — replace leading @ with null byte
            format!("\0{}", stripped)
        } else {
            path
        };
        if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), &path);
        }
    }
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

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
        libc::signal(libc::SIGTERM, handle_sigterm as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_sigint as libc::sighandler_t);
        libc::signal(libc::SIGHUP, handle_sighup as libc::sighandler_t);
        // Ignore SIGPIPE (writes to closed sockets)
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

fn init_logging() {
    struct StderrLogger;

    impl log::Log for StderrLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                let ts = chrono_lite_timestamp();
                eprintln!(
                    "[{}] {} {}: {}",
                    ts,
                    record.level(),
                    record.target(),
                    record.args()
                );
            }
        }

        fn flush(&self) {}
    }

    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

fn chrono_lite_timestamp() -> String {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    let secs = dur.as_secs();
    let hours = (secs % 86400) / 3600;
    let minutes = (secs % 3600) / 60;
    let seconds = secs % 60;
    let millis = dur.subsec_millis();
    format!("{:02}:{:02}:{:02}.{:03}", hours, minutes, seconds, millis)
}

// ---------------------------------------------------------------------------
// Watchdog
// ---------------------------------------------------------------------------

/// Parse a WATCHDOG_USEC string into a keepalive interval (half the watchdog period).
fn parse_watchdog_usec(usec_str: &str) -> Option<Duration> {
    let usec: u64 = usec_str.parse().ok()?;
    Some(Duration::from_micros(usec / 2))
}

fn watchdog_interval() -> Option<Duration> {
    env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|s| parse_watchdog_usec(&s))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    init_logging();
    // Signal handlers are set up after D-Bus initialization to prevent
    // zbus from potentially overriding them.

    log::info!("systemd-timedated starting");

    // Load initial state into shared state for D-Bus and control socket
    let initial_state = TimedateState::load();
    sync_process_tz(&initial_state.timezone);
    log::info!(
        "Timezone: {}, NTP: {}",
        initial_state.timezone,
        initial_state.ntp_enabled
    );
    let shared_state: SharedState = Arc::new(Mutex::new(initial_state.clone()));

    // Watchdog support
    let wd_interval = watchdog_interval();
    if let Some(ref iv) = wd_interval {
        log::info!("Watchdog enabled, interval {:?}", iv);
    }
    let mut last_watchdog = Instant::now();

    // Ensure /run/systemd exists
    let _ = fs::create_dir_all(Path::new(CONTROL_SOCKET_PATH).parent().unwrap());

    // Remove stale socket
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);

    // Bind control socket
    let listener = match UnixListener::bind(CONTROL_SOCKET_PATH) {
        Ok(l) => {
            log::info!("Listening on {}", CONTROL_SOCKET_PATH);
            Some(l)
        }
        Err(e) => {
            log::error!(
                "Failed to bind control socket {}: {}",
                CONTROL_SOCKET_PATH,
                e
            );
            None
        }
    };

    // Non-blocking so we can check SHUTDOWN flag periodically
    if let Some(ref l) = listener {
        l.set_nonblocking(true).expect("Failed to set non-blocking");
    }

    // Setup D-Bus before sending READY=1.  The service unit has
    // BusName=org.freedesktop.timedate1, so systemd waits for both
    // READY=1 and bus-name acquisition before considering the service
    // started.  Retry a few times to handle the case where the old
    // daemon's bus name hasn't been released yet after a restart.
    let _dbus_conn: Option<Connection> = {
        let mut conn = None;
        for attempt in 0..5 {
            match setup_dbus(shared_state.clone()) {
                Ok(c) => {
                    log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                    conn = Some(c);
                    break;
                }
                Err(e) => {
                    log::warn!("D-Bus attempt {} failed: {}", attempt, e);
                    if attempt < 4 {
                        thread::sleep(Duration::from_millis(200));
                    }
                }
            }
        }
        if conn.is_none() {
            log::warn!("Failed to register D-Bus interface after retries; control socket only");
        }
        conn
    };

    // Install signal handlers AFTER D-Bus setup so zbus can't override them
    setup_signal_handlers();

    sd_notify(&format!(
        "READY=1\nSTATUS=TZ: {}{}",
        initial_state.timezone,
        if _dbus_conn.is_some() {
            " (D-Bus active)"
        } else {
            ""
        }
    ));
    log::info!("systemd-timedated ready");

    // Main loop
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            log::info!("Received shutdown signal");
            break;
        }

        if RELOAD.load(Ordering::SeqCst) {
            RELOAD.store(false, Ordering::SeqCst);
            let new_state = TimedateState::load();
            log::info!(
                "Reloaded configuration, timezone: {}, NTP: {}",
                new_state.timezone,
                new_state.ntp_enabled
            );
            {
                let mut s = shared_state.lock().unwrap_or_else(|e| e.into_inner());
                *s = new_state.clone();
            }
            sd_notify(&format!("STATUS=TZ: {}", new_state.timezone));
        }

        // Send watchdog keepalive
        if let Some(ref iv) = wd_interval
            && last_watchdog.elapsed() >= *iv
        {
            sd_notify("WATCHDOG=1");
            last_watchdog = Instant::now();
        }

        // Accept control socket connections
        if let Some(ref listener) = listener {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    handle_client(&mut stream, Some(&shared_state));
                    let _ = stream.shutdown(Shutdown::Both);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No connection waiting
                }
                Err(e) => {
                    log::warn!("Accept error: {}", e);
                }
            }
        }

        // Brief sleep to avoid busy-looping
        thread::sleep(Duration::from_millis(50));
    }

    // Cleanup
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);
    sd_notify("STOPPING=1");
    log::info!("systemd-timedated stopped");
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(&path, content).unwrap();
        path
    }

    // ── Timezone detection tests ──────────────────────────────────────────

    #[test]
    fn test_detect_timezone_from_timezone_file() {
        let dir = TempDir::new().unwrap();
        let tz_path = write_file(&dir, "timezone", "America/New_York\n");
        let lt_path = dir.path().join("localtime");

        let tz = detect_timezone_from(
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            "/nonexistent/zoneinfo",
        );
        assert_eq!(tz, "America/New_York");
    }

    #[test]
    fn test_detect_timezone_from_timezone_file_trimmed() {
        let dir = TempDir::new().unwrap();
        let tz_path = write_file(&dir, "timezone", "  Europe/Berlin  \n");
        let lt_path = dir.path().join("localtime");

        let tz = detect_timezone_from(
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            "/nonexistent/zoneinfo",
        );
        assert_eq!(tz, "Europe/Berlin");
    }

    #[test]
    fn test_detect_timezone_empty_files_returns_utc() {
        let dir = TempDir::new().unwrap();
        let tz_path = write_file(&dir, "timezone", "\n");
        let lt_path = dir.path().join("localtime");

        let tz = detect_timezone_from(
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            "/nonexistent/zoneinfo",
        );
        // Falls back to UTC when all sources are empty
        assert_eq!(tz, "UTC");
    }

    #[test]
    fn test_detect_timezone_missing_files_returns_utc() {
        let tz = detect_timezone_from(
            "/nonexistent/localtime",
            "/nonexistent/timezone",
            "/nonexistent/zoneinfo",
        );
        assert_eq!(tz, "UTC");
    }

    #[test]
    fn test_detect_timezone_from_localtime_symlink() {
        let dir = TempDir::new().unwrap();
        // Create a fake zoneinfo structure
        let zi_dir = dir.path().join("zoneinfo");
        fs::create_dir_all(zi_dir.join("Asia")).unwrap();
        fs::write(zi_dir.join("Asia/Tokyo"), "fake-tz-data").unwrap();

        // Create /etc/localtime as a symlink
        let lt_path = dir.path().join("localtime");
        std::os::unix::fs::symlink(zi_dir.join("Asia/Tokyo"), &lt_path).unwrap();

        let tz_path = dir.path().join("timezone_nonexistent");

        let tz = detect_timezone_from(
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            zi_dir.to_str().unwrap(),
        );
        assert_eq!(tz, "Asia/Tokyo");
    }

    // ── RTC local/UTC tests ──────────────────────────────────────────────

    #[test]
    fn test_is_rtc_local_default() {
        // Non-existent file → UTC (false)
        assert!(!is_rtc_local_from("/nonexistent/adjtime"));
    }

    #[test]
    fn test_is_rtc_local_utc() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "adjtime", "0.0 0 0.0\n0\nUTC\n");
        assert!(!is_rtc_local_from(path.to_str().unwrap()));
    }

    #[test]
    fn test_is_rtc_local_local() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "adjtime", "0.0 0 0.0\n0\nLOCAL\n");
        assert!(is_rtc_local_from(path.to_str().unwrap()));
    }

    #[test]
    fn test_is_rtc_local_case_insensitive() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "adjtime", "0.0 0 0.0\n0\nlocal\n");
        assert!(is_rtc_local_from(path.to_str().unwrap()));
    }

    #[test]
    fn test_is_rtc_local_short_file() {
        let dir = TempDir::new().unwrap();
        // Only 1 line → no third line → default UTC
        let path = write_file(&dir, "adjtime", "0.0 0 0.0\n");
        assert!(!is_rtc_local_from(path.to_str().unwrap()));
    }

    #[test]
    fn test_is_rtc_local_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "adjtime", "");
        assert!(!is_rtc_local_from(path.to_str().unwrap()));
    }

    #[test]
    fn test_set_local_rtc_to_local() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "adjtime", "0.0 0 0.0\n0\nUTC\n");
        set_local_rtc_at(true, path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("LOCAL"));
        assert!(!content.contains("UTC"));
    }

    #[test]
    fn test_set_local_rtc_to_utc() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "adjtime", "0.0 0 0.0\n0\nLOCAL\n");
        set_local_rtc_at(false, path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("UTC"));
    }

    #[test]
    fn test_set_local_rtc_creates_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("adjtime");
        set_local_rtc_at(true, path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("LOCAL"));
    }

    #[test]
    fn test_set_local_rtc_preserves_lines() {
        let dir = TempDir::new().unwrap();
        let path = write_file(
            &dir,
            "adjtime",
            "-0.123456 1234567890 0.0\n1234567890\nUTC\n",
        );
        set_local_rtc_at(true, path.to_str().unwrap()).unwrap();

        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "-0.123456 1234567890 0.0");
        assert_eq!(lines[1], "1234567890");
        assert_eq!(lines[2], "LOCAL");
    }

    // ── Timezone setting tests ───────────────────────────────────────────

    #[test]
    fn test_set_timezone_creates_symlink() {
        let dir = TempDir::new().unwrap();

        // Create a fake zoneinfo
        let zi_dir = dir.path().join("zoneinfo");
        fs::create_dir_all(zi_dir.join("US")).unwrap();
        fs::write(zi_dir.join("US/Eastern"), "fake-tz").unwrap();

        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        set_timezone_at(
            "US/Eastern",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            zi_dir.to_str().unwrap(),
        )
        .unwrap();

        // Verify symlink was created
        assert!(lt_path.is_symlink());
        let target = fs::read_link(&lt_path).unwrap();
        assert!(target.to_string_lossy().contains("US/Eastern"));

        // Verify timezone file was written
        let content = fs::read_to_string(&tz_path).unwrap();
        assert_eq!(content, "US/Eastern\n");
    }

    #[test]
    fn test_set_timezone_empty_error() {
        let dir = TempDir::new().unwrap();
        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        let result = set_timezone_at(
            "",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            "/nonexistent",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_set_timezone_path_traversal_error() {
        let dir = TempDir::new().unwrap();
        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        let result = set_timezone_at(
            "../etc/shadow",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            "/usr/share/zoneinfo",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid"));
    }

    #[test]
    fn test_set_timezone_absolute_path_error() {
        let dir = TempDir::new().unwrap();
        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        let result = set_timezone_at(
            "/etc/shadow",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            "/usr/share/zoneinfo",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Invalid"));
    }

    #[test]
    fn test_set_timezone_not_found_error() {
        let dir = TempDir::new().unwrap();
        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        let result = set_timezone_at(
            "Fake/Timezone",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            "/nonexistent/zoneinfo",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_set_timezone_overwrites_existing() {
        let dir = TempDir::new().unwrap();

        // Create two fake zones
        let zi_dir = dir.path().join("zoneinfo");
        fs::create_dir_all(zi_dir.join("America")).unwrap();
        fs::write(zi_dir.join("America/Chicago"), "fake-tz1").unwrap();
        fs::write(zi_dir.join("America/Denver"), "fake-tz2").unwrap();

        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        // Set first timezone
        set_timezone_at(
            "America/Chicago",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            zi_dir.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(&tz_path).unwrap().trim(),
            "America/Chicago"
        );

        // Overwrite with second timezone
        set_timezone_at(
            "America/Denver",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            zi_dir.to_str().unwrap(),
        )
        .unwrap();
        assert_eq!(
            fs::read_to_string(&tz_path).unwrap().trim(),
            "America/Denver"
        );
    }

    // ── Timezone validation tests ────────────────────────────────────────

    #[test]
    fn test_is_valid_timezone_rejects_empty() {
        assert!(!is_valid_timezone_in("", "/nonexistent"));
    }

    #[test]
    fn test_is_valid_timezone_rejects_path_traversal() {
        assert!(!is_valid_timezone_in(
            "../etc/shadow",
            "/usr/share/zoneinfo"
        ));
    }

    #[test]
    fn test_is_valid_timezone_rejects_absolute() {
        assert!(!is_valid_timezone_in("/etc/shadow", "/usr/share/zoneinfo"));
    }

    #[test]
    fn test_is_valid_timezone_nonexistent() {
        assert!(!is_valid_timezone_in("Fake/Zone", "/nonexistent"));
    }

    #[test]
    fn test_is_valid_timezone_real() {
        // This may only pass on systems with zoneinfo installed
        let zi = Path::new(ZONEINFO_DIR);
        if zi.join("UTC").exists() {
            assert!(is_valid_timezone("UTC"));
        }
    }

    // ── Timezone listing tests ───────────────────────────────────────────

    #[test]
    fn test_list_timezones_from_fake_dir() {
        let dir = TempDir::new().unwrap();
        let zi = dir.path().join("zoneinfo");
        fs::create_dir_all(zi.join("America")).unwrap();
        fs::create_dir_all(zi.join("Europe")).unwrap();
        fs::write(zi.join("America/New_York"), "tz").unwrap();
        fs::write(zi.join("America/Chicago"), "tz").unwrap();
        fs::write(zi.join("Europe/Berlin"), "tz").unwrap();
        // This one should be skipped (no slash → POSIX compat name)
        fs::write(zi.join("UTC"), "tz").unwrap();

        let tzs = list_timezones_from(zi.to_str().unwrap());
        assert_eq!(
            tzs,
            vec!["America/Chicago", "America/New_York", "Europe/Berlin",]
        );
    }

    #[test]
    fn test_list_timezones_empty_dir() {
        let dir = TempDir::new().unwrap();
        let tzs = list_timezones_from(dir.path().to_str().unwrap());
        assert!(tzs.is_empty());
    }

    #[test]
    fn test_list_timezones_nonexistent_dir() {
        let tzs = list_timezones_from("/nonexistent/zoneinfo");
        assert!(tzs.is_empty());
    }

    #[test]
    fn test_list_timezones_skips_special_files() {
        let dir = TempDir::new().unwrap();
        let zi = dir.path().join("zoneinfo");
        fs::create_dir_all(zi.join("US")).unwrap();
        fs::write(zi.join("US/Eastern"), "tz").unwrap();
        // These should be skipped
        fs::create_dir_all(zi.join("posix/US")).unwrap();
        fs::write(zi.join("posix/US/Eastern"), "tz").unwrap();
        fs::create_dir_all(zi.join("right/US")).unwrap();
        fs::write(zi.join("right/US/Eastern"), "tz").unwrap();
        fs::write(zi.join("zone.tab"), "tz").unwrap();
        fs::write(zi.join("zone1970.tab"), "tz").unwrap();
        fs::write(zi.join("iso3166.tab"), "tz").unwrap();
        fs::write(zi.join("leap-seconds.list"), "tz").unwrap();
        fs::write(zi.join("+VERSION"), "tz").unwrap();

        let tzs = list_timezones_from(zi.to_str().unwrap());
        assert_eq!(tzs, vec!["US/Eastern"]);
    }

    // ── TimedateState tests ──────────────────────────────────────────────

    #[test]
    fn test_timedate_state_default() {
        let state = TimedateState::default();
        assert_eq!(state.timezone, "UTC");
        assert!(!state.local_rtc);
        assert!(!state.ntp_enabled);
        assert!(!state.ntp_synced);
        assert!(!state.can_ntp);
    }

    #[test]
    fn test_timedate_state_load_from_files() {
        let dir = TempDir::new().unwrap();
        let tz_path = write_file(&dir, "timezone", "Asia/Tokyo\n");
        let lt_path = dir.path().join("localtime");
        let adj_path = write_file(&dir, "adjtime", "0.0 0 0.0\n0\nLOCAL\n");

        let state = TimedateState::load_from(
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            adj_path.to_str().unwrap(),
            "/nonexistent/zoneinfo",
        );
        assert_eq!(state.timezone, "Asia/Tokyo");
        assert!(state.local_rtc);
    }

    #[test]
    fn test_timedate_state_load_missing_files() {
        let state = TimedateState::load_from(
            "/nonexistent/localtime",
            "/nonexistent/timezone",
            "/nonexistent/adjtime",
            "/nonexistent/zoneinfo",
        );
        assert_eq!(state.timezone, "UTC");
        assert!(!state.local_rtc);
    }

    #[test]
    fn test_format_status_contains_fields() {
        let state = TimedateState {
            timezone: "Europe/Berlin".to_string(),
            local_rtc: false,
            ntp_enabled: true,
            ntp_synced: true,
            can_ntp: true,
        };
        let out = state.format_status();
        assert!(out.contains("Local time:"));
        assert!(out.contains("Universal time:"));
        assert!(out.contains("RTC time:"));
        assert!(out.contains("Time zone: Europe/Berlin"));
        assert!(out.contains("NTP service: active"));
        assert!(out.contains("System clock synchronized: yes"));
        assert!(out.contains("RTC in local TZ: no"));
    }

    #[test]
    fn test_format_status_ntp_inactive() {
        let state = TimedateState {
            timezone: "UTC".to_string(),
            local_rtc: true,
            ntp_enabled: false,
            ntp_synced: false,
            can_ntp: false,
        };
        let out = state.format_status();
        assert!(out.contains("NTP service: inactive"));
        assert!(out.contains("System clock synchronized: no"));
        assert!(out.contains("RTC in local TZ: yes"));
    }

    #[test]
    fn test_format_show_contains_fields() {
        let state = TimedateState {
            timezone: "US/Pacific".to_string(),
            local_rtc: false,
            ntp_enabled: true,
            ntp_synced: true,
            can_ntp: true,
        };
        let out = state.format_show();
        assert!(out.contains("Timezone=US/Pacific"));
        assert!(out.contains("LocalRTC=no"));
        assert!(out.contains("CanNTP=yes"));
        assert!(out.contains("NTP=yes"));
        assert!(out.contains("NTPSynchronized=yes"));
        assert!(out.contains("TimeUSec="));
        assert!(out.contains("RTCTimeUSec=n/a"));
    }

    #[test]
    fn test_format_show_disabled_ntp() {
        let state = TimedateState {
            timezone: "UTC".to_string(),
            local_rtc: true,
            ntp_enabled: false,
            ntp_synced: false,
            can_ntp: false,
        };
        let out = state.format_show();
        assert!(out.contains("LocalRTC=yes"));
        assert!(out.contains("CanNTP=no"));
        assert!(out.contains("NTP=no"));
        assert!(out.contains("NTPSynchronized=no"));
    }

    // ── Control command tests ────────────────────────────────────────────

    #[test]
    fn test_handle_control_ping() {
        assert_eq!(handle_control_command("PING", None), "PONG\n");
        assert_eq!(handle_control_command("ping", None), "PONG\n");
        assert_eq!(handle_control_command("Ping", None), "PONG\n");
    }

    #[test]
    fn test_handle_control_status() {
        let response = handle_control_command("STATUS", None);
        assert!(response.contains("Local time:"));
        assert!(response.contains("Universal time:"));
    }

    #[test]
    fn test_handle_control_show() {
        let response = handle_control_command("SHOW", None);
        assert!(response.contains("Timezone="));
        assert!(response.contains("LocalRTC="));
        assert!(response.contains("NTP="));
    }

    #[test]
    fn test_handle_control_set_timezone_empty() {
        let response = handle_control_command("SET-TIMEZONE", None);
        assert!(response.starts_with("ERROR:"));
        assert!(response.contains("required"));
    }

    #[test]
    fn test_handle_control_set_timezone_invalid() {
        let response = handle_control_command("SET-TIMEZONE ../etc/shadow", None);
        assert!(response.starts_with("ERROR:"));
    }

    #[test]
    fn test_handle_control_set_local_rtc_empty() {
        let response = handle_control_command("SET-LOCAL-RTC", None);
        assert!(response.starts_with("ERROR:"));
        assert!(response.contains("required"));
    }

    #[test]
    fn test_handle_control_set_local_rtc_invalid() {
        let response = handle_control_command("SET-LOCAL-RTC maybe", None);
        assert!(response.starts_with("ERROR:"));
        assert!(response.contains("Invalid"));
    }

    // --- D-Bus interface tests ---

    #[test]
    fn test_dbus_timedate1_manager_struct() {
        // Verify the interface struct can be created without panic
        let shared: SharedState = Arc::new(Mutex::new(TimedateState::default()));
        let _mgr = Timedate1Manager { state: shared };
    }

    #[test]
    fn test_shared_state_reload() {
        let state = TimedateState {
            timezone: "UTC".to_string(),
            ..Default::default()
        };
        let shared: SharedState = Arc::new(Mutex::new(state));

        // Simulate a reload
        {
            let mut s = shared.lock().unwrap();
            s.timezone = "Europe/Berlin".to_string();
            s.ntp_enabled = true;
        }

        let s = shared.lock().unwrap();
        assert_eq!(s.timezone, "Europe/Berlin");
        assert!(s.ntp_enabled);
    }

    #[test]
    fn test_shared_state_ntp_toggle() {
        let state = TimedateState {
            ntp_enabled: false,
            can_ntp: true,
            ..Default::default()
        };
        let shared: SharedState = Arc::new(Mutex::new(state));

        {
            let mut s = shared.lock().unwrap();
            s.ntp_enabled = true;
        }

        let s = shared.lock().unwrap();
        assert!(s.ntp_enabled);
    }

    #[test]
    fn test_handle_control_set_ntp_empty() {
        let response = handle_control_command("SET-NTP", None);
        assert!(response.starts_with("ERROR:"));
        assert!(response.contains("required"));
    }

    #[test]
    fn test_handle_control_set_ntp_invalid() {
        let response = handle_control_command("SET-NTP sometimes", None);
        assert!(response.starts_with("ERROR:"));
        assert!(response.contains("Invalid"));
    }

    #[test]
    fn test_handle_control_unknown() {
        let response = handle_control_command("FROBNICATE", None);
        assert!(response.starts_with("ERROR:"));
        assert!(response.contains("Unknown"));
    }

    #[test]
    fn test_handle_control_empty() {
        let response = handle_control_command("", None);
        assert!(response.starts_with("ERROR:"));
    }

    #[test]
    fn test_handle_control_case_insensitive() {
        let r1 = handle_control_command("status", None);
        let r2 = handle_control_command("STATUS", None);
        let r3 = handle_control_command("Status", None);
        // All should return valid status output
        assert!(r1.contains("Local time:"));
        assert!(r2.contains("Local time:"));
        assert!(r3.contains("Local time:"));
    }

    #[test]
    fn test_handle_control_list_timezones() {
        // On systems without zoneinfo, this may return empty, but should not error
        let response = handle_control_command("LIST-TIMEZONES", None);
        assert!(!response.starts_with("ERROR:"));
    }

    // ── NTP helpers tests ────────────────────────────────────────────────

    #[test]
    fn test_is_ntp_synced_runs() {
        // Just verify it doesn't panic
        let _ = is_ntp_synced();
    }

    #[test]
    fn test_is_ntp_enabled_runs() {
        // Just verify it doesn't panic
        let _ = is_ntp_enabled();
    }

    #[test]
    fn test_can_ntp_runs() {
        // Just verify it doesn't panic
        let _ = can_ntp();
    }

    // ── Time formatting tests ────────────────────────────────────────────

    #[test]
    fn test_format_timestamp_utc_epoch() {
        let formatted = format_timestamp_utc(0);
        assert!(formatted.contains("1970"));
        assert!(formatted.contains("00:00:00"));
    }

    #[test]
    fn test_format_timestamp_utc_known_date() {
        // 2024-01-01 00:00:00 UTC = 1704067200
        let formatted = format_timestamp_utc(1704067200);
        assert!(formatted.contains("2024"));
        assert!(formatted.contains("00:00:00"));
    }

    #[test]
    fn test_format_timestamp_local_runs() {
        // Just verify it doesn't panic
        let _ = format_timestamp_local(0, "UTC");
    }

    #[test]
    fn test_get_tz_abbr_and_offset_format() {
        let (abbr, offset) = get_tz_abbr_and_offset("UTC");
        assert!(!abbr.is_empty());
        // Should be in the format +HHMM or -HHMM
        assert!(offset.len() == 5);
        assert!(offset.starts_with('+') || offset.starts_with('-'));
        // The digits should be numeric
        assert!(offset[1..].chars().all(|c| c.is_ascii_digit()));
    }

    // ── Watchdog tests (pure function, no env var races) ─────────────────

    #[test]
    fn test_parse_watchdog_usec_valid() {
        let iv = parse_watchdog_usec("2000000");
        assert!(iv.is_some());
        // Half of 2000000 usec = 1 second
        assert_eq!(iv.unwrap(), Duration::from_secs(1));
    }

    #[test]
    fn test_parse_watchdog_usec_zero() {
        let iv = parse_watchdog_usec("0");
        assert_eq!(iv, Some(Duration::from_micros(0)));
    }

    #[test]
    fn test_parse_watchdog_usec_invalid() {
        assert!(parse_watchdog_usec("not-a-number").is_none());
    }

    #[test]
    fn test_parse_watchdog_usec_empty() {
        assert!(parse_watchdog_usec("").is_none());
    }

    #[test]
    fn test_parse_watchdog_usec_large() {
        let iv = parse_watchdog_usec("360000000"); // 6 minutes
        assert_eq!(iv, Some(Duration::from_secs(180))); // half = 3 min
    }

    // ── Chrono lite timestamp test ───────────────────────────────────────

    #[test]
    fn test_chrono_lite_timestamp_format() {
        let ts = chrono_lite_timestamp();
        // Format: HH:MM:SS.mmm
        assert_eq!(ts.len(), 12);
        assert_eq!(&ts[2..3], ":");
        assert_eq!(&ts[5..6], ":");
        assert_eq!(&ts[8..9], ".");
    }

    // ── Read RTC time test ───────────────────────────────────────────────

    #[test]
    fn test_read_rtc_time_runs() {
        // May return None on systems without /sys/class/rtc/rtc0
        let _ = read_rtc_time();
    }

    // ── Integration: roundtrip set/detect timezone ───────────────────────

    #[test]
    fn test_set_then_detect_timezone() {
        let dir = TempDir::new().unwrap();

        let zi_dir = dir.path().join("zoneinfo");
        fs::create_dir_all(zi_dir.join("Pacific")).unwrap();
        fs::write(zi_dir.join("Pacific/Auckland"), "fake-tz").unwrap();

        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        set_timezone_at(
            "Pacific/Auckland",
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            zi_dir.to_str().unwrap(),
        )
        .unwrap();

        let detected = detect_timezone_from(
            lt_path.to_str().unwrap(),
            tz_path.to_str().unwrap(),
            zi_dir.to_str().unwrap(),
        );
        assert_eq!(detected, "Pacific/Auckland");
    }

    // ── Integration: roundtrip set/check local RTC ───────────────────────

    #[test]
    fn test_set_then_check_local_rtc() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("adjtime");

        // Set to LOCAL
        set_local_rtc_at(true, path.to_str().unwrap()).unwrap();
        assert!(is_rtc_local_from(path.to_str().unwrap()));

        // Set back to UTC
        set_local_rtc_at(false, path.to_str().unwrap()).unwrap();
        assert!(!is_rtc_local_from(path.to_str().unwrap()));
    }

    // ── Multiple timezone changes ────────────────────────────────────────

    #[test]
    fn test_multiple_timezone_changes() {
        let dir = TempDir::new().unwrap();

        let zi_dir = dir.path().join("zoneinfo");
        fs::create_dir_all(zi_dir.join("America")).unwrap();
        fs::create_dir_all(zi_dir.join("Europe")).unwrap();
        fs::create_dir_all(zi_dir.join("Asia")).unwrap();
        fs::write(zi_dir.join("America/New_York"), "tz").unwrap();
        fs::write(zi_dir.join("Europe/London"), "tz").unwrap();
        fs::write(zi_dir.join("Asia/Seoul"), "tz").unwrap();

        let lt_path = dir.path().join("localtime");
        let tz_path = dir.path().join("timezone");

        for tz in &["America/New_York", "Europe/London", "Asia/Seoul"] {
            set_timezone_at(
                tz,
                lt_path.to_str().unwrap(),
                tz_path.to_str().unwrap(),
                zi_dir.to_str().unwrap(),
            )
            .unwrap();

            let detected = detect_timezone_from(
                lt_path.to_str().unwrap(),
                tz_path.to_str().unwrap(),
                zi_dir.to_str().unwrap(),
            );
            assert_eq!(&detected, tz);
        }
    }

    // ── Collect timezones helper ─────────────────────────────────────────

    #[test]
    fn test_collect_timezones_nested() {
        let dir = TempDir::new().unwrap();
        let zi = dir.path().join("zi");
        fs::create_dir_all(zi.join("A/B")).unwrap();
        fs::write(zi.join("A/B/C"), "tz").unwrap();

        let tzs = list_timezones_from(zi.to_str().unwrap());
        assert_eq!(tzs, vec!["A/B/C"]);
    }

    #[test]
    fn test_collect_timezones_sorted() {
        let dir = TempDir::new().unwrap();
        let zi = dir.path().join("zi");
        fs::create_dir_all(zi.join("Z")).unwrap();
        fs::create_dir_all(zi.join("A")).unwrap();
        fs::create_dir_all(zi.join("M")).unwrap();
        fs::write(zi.join("Z/zone"), "tz").unwrap();
        fs::write(zi.join("A/zone"), "tz").unwrap();
        fs::write(zi.join("M/zone"), "tz").unwrap();

        let tzs = list_timezones_from(zi.to_str().unwrap());
        assert_eq!(tzs, vec!["A/zone", "M/zone", "Z/zone"]);
    }
}
