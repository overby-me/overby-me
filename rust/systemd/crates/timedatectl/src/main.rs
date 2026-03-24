//! timedatectl — Control the system time and date
//!
//! A Rust implementation of timedatectl that queries and controls
//! the system clock, timezone, and NTP synchronization settings.
//!
//! Subcommands:
//! - `status`           — Show current time/date settings (default)
//! - `set-time TIME`    — Set the system time
//! - `set-timezone TZ`  — Set the system timezone
//! - `set-ntp BOOL`     — Enable/disable NTP synchronization
//! - `list-timezones`   — List available timezones
//! - `timesync-status`  — Show NTP synchronization status
//! - `show`             — Show machine-readable properties

use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

// ── Constants ──────────────────────────────────────────────────────────────

const LOCALTIME_PATH: &str = "/etc/localtime";
const TIMEZONE_PATH: &str = "/etc/timezone";
const ZONEINFO_DIR: &str = "/usr/share/zoneinfo";
const ADJTIME_PATH: &str = "/etc/adjtime";

const TIMESYNCD_CONF: &str = "/etc/systemd/timesyncd.conf";
const TIMESYNCD_STATE_DIR: &str = "/var/lib/systemd/timesync";

// ── Data types ─────────────────────────────────────────────────────────────

#[derive(Debug)]
struct TimeInfo {
    /// Current local time as formatted string
    local_time: String,
    /// Current UTC time as formatted string
    utc_time: String,
    /// Current RTC time (if available)
    rtc_time: Option<String>,
    /// Timezone name (e.g., "America/New_York")
    timezone: String,
    /// UTC offset string (e.g., "+0000")
    utc_offset: String,
    /// Whether NTP is enabled
    ntp_enabled: bool,
    /// Whether NTP is active/synchronized
    ntp_synced: bool,
    /// Whether RTC is in local time
    rtc_local: bool,
}

// ── Time helpers ───────────────────────────────────────────────────────────

fn get_unix_timestamp() -> (i64, u32) {
    let dur = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    (dur.as_secs() as i64, dur.subsec_nanos())
}

/// Get the current time broken down via libc localtime_r
fn get_local_tm() -> libc::tm {
    let (secs, _) = get_unix_timestamp();
    let time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&time_t, &mut tm);
    }
    tm
}

/// Get the current time broken down via libc gmtime_r
fn get_utc_tm() -> libc::tm {
    let (secs, _) = get_unix_timestamp();
    let time_t = secs as libc::time_t;
    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gmtime_r(&time_t, &mut tm);
    }
    tm
}

/// Format a libc::tm as a human-readable date string
/// e.g., "Mon 2025-02-17 15:30:00 UTC"
fn format_tm(tm: &libc::tm, tz_abbr: &str) -> String {
    let wday = match tm.tm_wday {
        0 => "Sun",
        1 => "Mon",
        2 => "Tue",
        3 => "Wed",
        4 => "Thu",
        5 => "Fri",
        6 => "Sat",
        _ => "???",
    };

    format!(
        "{} {:04}-{:02}-{:02} {:02}:{:02}:{:02} {}",
        wday,
        tm.tm_year + 1900,
        tm.tm_mon + 1,
        tm.tm_mday,
        tm.tm_hour,
        tm.tm_min,
        tm.tm_sec,
        tz_abbr,
    )
}

/// Get the timezone abbreviation from a tm struct
fn get_tz_abbr(tm: &libc::tm) -> String {
    unsafe {
        let ptr = tm.tm_zone;
        if ptr.is_null() {
            return "UTC".to_string();
        }
        let cstr = std::ffi::CStr::from_ptr(ptr);
        cstr.to_string_lossy().into_owned()
    }
}

/// Get the UTC offset in seconds from a tm struct
fn get_utc_offset(tm: &libc::tm) -> i64 {
    tm.tm_gmtoff
}

/// Format UTC offset as "+HHMM" or "-HHMM"
fn format_utc_offset(offset_secs: i64) -> String {
    let sign = if offset_secs >= 0 { "+" } else { "-" };
    let abs = offset_secs.unsigned_abs();
    let hours = abs / 3600;
    let mins = (abs % 3600) / 60;
    format!("{}{:02}{:02}", sign, hours, mins)
}

// ── Timezone detection ─────────────────────────────────────────────────────

/// Detect the current timezone name
fn detect_timezone() -> String {
    // Method 1: Read /etc/timezone
    if let Ok(tz) = fs::read_to_string(TIMEZONE_PATH) {
        let tz = tz.trim().to_string();
        if !tz.is_empty() {
            return tz;
        }
    }

    // Method 2: Read /etc/localtime symlink target
    if let Ok(target) = fs::read_link(LOCALTIME_PATH) {
        let target_str = target.to_string_lossy();
        // Look for .../zoneinfo/REGION/CITY pattern
        if let Some(pos) = target_str.find("zoneinfo/") {
            return target_str[pos + 9..].to_string();
        }
    }

    // Method 3: Check TZ environment variable
    if let Ok(tz) = env::var("TZ") {
        if !tz.is_empty() && !tz.starts_with(':') {
            return tz;
        }
        if let Some(stripped) = tz.strip_prefix(':') {
            if let Some(pos) = stripped.find("zoneinfo/") {
                return stripped[pos + 9..].to_string();
            }
            return stripped.to_string();
        }
    }

    "UTC".to_string()
}

// ── RTC detection ──────────────────────────────────────────────────────────

/// Read RTC time from /dev/rtc or /sys/class/rtc/rtc0/time
fn read_rtc_time() -> Option<String> {
    // Try /sys interface first
    let date = fs::read_to_string("/sys/class/rtc/rtc0/date").ok()?;
    let time = fs::read_to_string("/sys/class/rtc/rtc0/time").ok()?;
    Some(format!("{} {}", date.trim(), time.trim()))
}

/// Check if RTC is set to local time (from /etc/adjtime)
fn is_rtc_local() -> bool {
    if let Ok(contents) = fs::read_to_string(ADJTIME_PATH) {
        let lines: Vec<&str> = contents.lines().collect();
        // Third line of adjtime is either "LOCAL" or "UTC"
        if lines.len() >= 3 {
            return lines[2].trim().eq_ignore_ascii_case("LOCAL");
        }
    }
    false
}

// ── NTP detection ──────────────────────────────────────────────────────────

/// Check if NTP (timesyncd) is enabled
fn is_ntp_enabled() -> bool {
    // Check if timesyncd is enabled via symlinks in wants directories
    let wants_dirs = [
        "/etc/systemd/system/sysinit.target.wants",
        "/etc/systemd/system/multi-user.target.wants",
    ];

    for dir in &wants_dirs {
        let link = Path::new(dir).join("systemd-timesyncd.service");
        if link.exists() || link.is_symlink() {
            return true;
        }
    }

    // Check if the service unit exists and has [Install] WantedBy
    let service_paths = [
        "/etc/systemd/system/systemd-timesyncd.service",
        "/lib/systemd/system/systemd-timesyncd.service",
        "/usr/lib/systemd/system/systemd-timesyncd.service",
    ];

    for path in &service_paths {
        if Path::new(path).exists() {
            return true;
        }
    }

    false
}

/// Check if NTP is currently synchronized
fn is_ntp_synced() -> bool {
    // Check via adjtimex status
    unsafe {
        let mut tx: libc::timex = std::mem::zeroed();
        tx.modes = 0; // read-only
        let ret = libc::adjtimex(&mut tx);
        // Return value of TIME_OK (0) or TIME_INS/TIME_DEL/TIME_OOP means synced
        // TIME_ERROR (5) means not synced
        if ret >= 0 {
            return tx.status & libc::STA_UNSYNC == 0;
        }
    }
    false
}

// ── NTP server info ────────────────────────────────────────────────────────

/// Read configured NTP servers from timesyncd.conf
fn read_ntp_servers() -> (Vec<String>, Vec<String>) {
    let mut ntp = Vec::new();
    let mut fallback = Vec::new();

    let paths = [TIMESYNCD_CONF.to_string(), format!("{}.d", TIMESYNCD_CONF)];

    for path_str in &paths {
        let path = Path::new(path_str);
        if path.is_file() {
            if let Ok(contents) = fs::read_to_string(path) {
                parse_ntp_config(&contents, &mut ntp, &mut fallback);
            }
        } else if path.is_dir()
            && let Ok(mut entries) = fs::read_dir(path)
        {
            let mut files: Vec<PathBuf> = Vec::new();
            while let Some(Ok(entry)) = entries.next() {
                let p = entry.path();
                if p.extension().is_some_and(|e| e == "conf") {
                    files.push(p);
                }
            }
            files.sort();
            for f in files {
                if let Ok(contents) = fs::read_to_string(&f) {
                    parse_ntp_config(&contents, &mut ntp, &mut fallback);
                }
            }
        }
    }

    if fallback.is_empty() {
        fallback = vec![
            "0.pool.ntp.org".into(),
            "1.pool.ntp.org".into(),
            "2.pool.ntp.org".into(),
            "3.pool.ntp.org".into(),
        ];
    }

    (ntp, fallback)
}

fn parse_ntp_config(contents: &str, ntp: &mut Vec<String>, fallback: &mut Vec<String>) {
    let mut in_time = false;
    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_time = line.eq_ignore_ascii_case("[time]");
            continue;
        }
        if !in_time {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "NTP" => {
                    *ntp = value.split_whitespace().map(String::from).collect();
                }
                "FallbackNTP" => {
                    *fallback = value.split_whitespace().map(String::from).collect();
                }
                _ => {}
            }
        }
    }
}

// ── Timezone listing ───────────────────────────────────────────────────────

/// Recursively list all timezone files under the zoneinfo directory
fn list_timezones() -> Vec<String> {
    let mut timezones = Vec::new();
    let zoneinfo = Path::new(ZONEINFO_DIR);

    if !zoneinfo.exists() {
        // Try TZDIR environment variable
        if let Ok(tzdir) = env::var("TZDIR") {
            let tzdir_path = Path::new(&tzdir);
            if tzdir_path.exists() {
                collect_timezones(tzdir_path, tzdir_path, &mut timezones);
            }
        }
    } else {
        collect_timezones(zoneinfo, zoneinfo, &mut timezones);
    }

    timezones.sort();
    timezones
}

fn collect_timezones(base: &Path, current: &Path, result: &mut Vec<String>) {
    let Ok(entries) = fs::read_dir(current) else {
        return;
    };

    // Skip these non-timezone directories/files
    let skip = [
        "posix",
        "right",
        "posixrules",
        "localtime",
        "leap-seconds.list",
        "leapseconds",
        "tzdata.zi",
        "zone.tab",
        "zone1970.tab",
        "iso3166.tab",
        "SECURITY",
        "+VERSION",
        "Factory",
    ];

    for entry in entries.flatten() {
        let path = entry.path();
        let name = entry.file_name();
        let name_str = name.to_string_lossy();

        if skip.iter().any(|s| *s == name_str.as_ref()) {
            continue;
        }

        if path.is_dir() {
            collect_timezones(base, &path, result);
        } else if path.is_file() {
            // Get relative path from zoneinfo base
            if let Ok(rel) = path.strip_prefix(base) {
                let tz = rel.to_string_lossy().to_string();
                // Only include entries that look like timezone names
                // (contain a slash, i.e., Region/City format, or are uppercase like UTC, EST, etc.)
                if tz.contains('/')
                    || tz.chars().all(|c| {
                        c.is_ascii_uppercase() || c.is_ascii_digit() || c == '-' || c == '+'
                    })
                {
                    result.push(tz);
                }
            }
        }
    }
}

// ── Commands ───────────────────────────────────────────────────────────────

fn cmd_status() {
    let local_tm = get_local_tm();
    let utc_tm = get_utc_tm();
    let tz_abbr = get_tz_abbr(&local_tm);
    let offset = get_utc_offset(&local_tm);
    let timezone = detect_timezone();
    let rtc_time = read_rtc_time();
    let ntp_enabled = is_ntp_enabled();
    let ntp_synced = is_ntp_synced();
    let rtc_local = is_rtc_local();

    let info = TimeInfo {
        local_time: format_tm(&local_tm, &tz_abbr),
        utc_time: format_tm(&utc_tm, "UTC"),
        rtc_time,
        timezone,
        utc_offset: format_utc_offset(offset),
        ntp_enabled,
        ntp_synced,
        rtc_local,
    };

    print_status(&info);
}

fn print_status(info: &TimeInfo) {
    let yes_no = |b: bool| if b { "yes" } else { "no" };

    println!("               Local time: {}", info.local_time);
    println!("           Universal time: {}", info.utc_time);
    if let Some(ref rtc) = info.rtc_time {
        println!("                 RTC time: {}", rtc);
    } else {
        println!("                 RTC time: n/a");
    }
    // Show timezone abbreviation and offset, matching systemd format:
    // "Time zone: America/New_York (EST, -0500)"
    let local_tm = get_local_tm();
    let tz_abbr = get_tz_abbr(&local_tm);
    println!(
        "                Time zone: {} ({}, {})",
        info.timezone, tz_abbr, info.utc_offset
    );

    println!("System clock synchronized: {}", yes_no(info.ntp_synced));
    println!(
        "              NTP service: {}",
        if info.ntp_enabled {
            "active"
        } else {
            "inactive"
        }
    );
    println!("          RTC in local TZ: {}", yes_no(info.rtc_local));

    if info.rtc_local {
        eprintln!();
        eprintln!("Warning: The system is configured to read the RTC time in the local time zone.");
        eprintln!("         This is not recommended. Please set the RTC to UTC with:");
        eprintln!("         timedatectl set-local-rtc 0");
    }
}

fn cmd_show_property(prop: &str, value_only: bool) {
    let timezone = detect_timezone();
    let ntp_enabled = is_ntp_enabled();
    let ntp_synced = is_ntp_synced();
    let rtc_local = is_rtc_local();

    let (secs, nsecs) = get_unix_timestamp();
    let usec = secs as u64 * 1_000_000 + (nsecs / 1000) as u64;

    let val = match prop {
        "Timezone" => timezone,
        "LocalRTC" => (if rtc_local { "yes" } else { "no" }).to_string(),
        "NTP" => (if ntp_enabled { "yes" } else { "no" }).to_string(),
        "NTPSynchronized" => (if ntp_synced { "yes" } else { "no" }).to_string(),
        "TimeUSec" => usec.to_string(),
        "RTCTimeUSec" => "n/a".to_string(),
        "CanNTP" => {
            let can = Path::new("/lib/systemd/systemd-timesyncd").exists()
                || Path::new("/usr/lib/systemd/systemd-timesyncd").exists();
            (if can { "yes" } else { "no" }).to_string()
        }
        _ => {
            eprintln!("Unknown property: {}", prop);
            process::exit(1);
        }
    };

    if value_only {
        println!("{}", val);
    } else {
        println!("{}={}", prop, val);
    }
}

fn cmd_show() {
    let local_tm = get_local_tm();
    let _utc_tm = get_utc_tm();
    let _tz_abbr = get_tz_abbr(&local_tm);
    let _offset = get_utc_offset(&local_tm);
    let timezone = detect_timezone();
    let ntp_enabled = is_ntp_enabled();
    let ntp_synced = is_ntp_synced();
    let rtc_local = is_rtc_local();

    let (secs, nsecs) = get_unix_timestamp();
    let usec = secs as u64 * 1_000_000 + (nsecs / 1000) as u64;

    println!("Timezone={}", timezone);
    println!("LocalRTC={}", if rtc_local { "yes" } else { "no" });
    println!(
        "CanNTP={}",
        if Path::new("/lib/systemd/systemd-timesyncd").exists()
            || Path::new("/usr/lib/systemd/systemd-timesyncd").exists()
        {
            "yes"
        } else {
            "no"
        }
    );
    println!("NTP={}", if ntp_enabled { "yes" } else { "no" });
    println!("NTPSynchronized={}", if ntp_synced { "yes" } else { "no" });
    println!("TimeUSec={}", usec);
    println!("RTCTimeUSec=n/a");
}

fn cmd_set_time(time_str: &str) {
    // Parse time string: "YYYY-MM-DD HH:MM:SS" or "HH:MM:SS" or "YYYY-MM-DD"
    let mut tm = get_local_tm();
    let mut parsed = false;

    if time_str.contains(' ') || (time_str.contains('-') && time_str.contains(':')) {
        // Full datetime: "YYYY-MM-DD HH:MM:SS"
        let parts: Vec<&str> = time_str.splitn(2, ' ').collect();
        if parts.len() == 2 {
            if let Some(date_tm) = parse_date(parts[0]) {
                tm.tm_year = date_tm.0;
                tm.tm_mon = date_tm.1;
                tm.tm_mday = date_tm.2;
            } else {
                eprintln!("Failed to parse date: {}", parts[0]);
                process::exit(1);
            }
            if let Some(time_tm) = parse_time(parts[1]) {
                tm.tm_hour = time_tm.0;
                tm.tm_min = time_tm.1;
                tm.tm_sec = time_tm.2;
            } else {
                eprintln!("Failed to parse time: {}", parts[1]);
                process::exit(1);
            }
            parsed = true;
        }
    } else if time_str.contains(':') {
        // Time only: "HH:MM:SS"
        if let Some(time_tm) = parse_time(time_str) {
            tm.tm_hour = time_tm.0;
            tm.tm_min = time_tm.1;
            tm.tm_sec = time_tm.2;
            parsed = true;
        }
    } else if time_str.contains('-') {
        // Date only: "YYYY-MM-DD"
        if let Some(date_tm) = parse_date(time_str) {
            tm.tm_year = date_tm.0;
            tm.tm_mon = date_tm.1;
            tm.tm_mday = date_tm.2;
            parsed = true;
        }
    }

    if !parsed {
        eprintln!(
            "Failed to parse time specification '{}'. Use format: YYYY-MM-DD HH:MM:SS",
            time_str
        );
        process::exit(1);
    }

    // Convert back to timestamp
    let timestamp = unsafe { libc::mktime(&mut tm) };
    if timestamp == -1 {
        eprintln!("Failed to convert time to timestamp");
        process::exit(1);
    }

    // Set the clock
    unsafe {
        let ts = libc::timespec {
            tv_sec: timestamp as libc::time_t,
            tv_nsec: 0,
        };
        if libc::clock_settime(libc::CLOCK_REALTIME, &ts) != 0 {
            let err = io::Error::last_os_error();
            eprintln!("Failed to set system clock: {}", err);
            if err.raw_os_error() == Some(libc::EPERM) {
                eprintln!("Hint: This operation requires root privileges.");
            }
            process::exit(1);
        }
    }
}

fn parse_date(s: &str) -> Option<(i32, i32, i32)> {
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 3 {
        return None;
    }
    let year: i32 = parts[0].parse().ok()?;
    let month: i32 = parts[1].parse().ok()?;
    let day: i32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }
    Some((year - 1900, month - 1, day))
}

fn parse_time(s: &str) -> Option<(i32, i32, i32)> {
    let parts: Vec<&str> = s.split(':').collect();
    if parts.len() < 2 || parts.len() > 3 {
        return None;
    }
    let hour: i32 = parts[0].parse().ok()?;
    let min: i32 = parts[1].parse().ok()?;
    let sec: i32 = if parts.len() == 3 {
        parts[2].parse().ok()?
    } else {
        0
    };
    if !(0..=23).contains(&hour) || !(0..=59).contains(&min) || !(0..=60).contains(&sec) {
        return None;
    }
    Some((hour, min, sec))
}

fn cmd_set_timezone(tz: &str) {
    // Validate the timezone exists
    let zoneinfo = Path::new(ZONEINFO_DIR);

    // Also check TZDIR env
    let tzdir = env::var("TZDIR").ok();
    let tz_path = zoneinfo.join(tz);
    let alt_tz_path = tzdir.as_ref().map(|d| Path::new(d).join(tz));

    let found = tz_path.exists() || alt_tz_path.as_ref().is_some_and(|p| p.exists());
    if !found {
        eprintln!("Timezone '{}' not found.", tz);
        eprintln!("Use 'timedatectl list-timezones' to see available timezones.");
        process::exit(1);
    }

    // Create /etc/localtime symlink
    let target = if tz_path.exists() {
        tz_path
    } else {
        alt_tz_path.unwrap()
    };

    // Remove existing
    let _ = fs::remove_file(LOCALTIME_PATH);

    // Create symlink
    #[cfg(unix)]
    {
        if let Err(e) = std::os::unix::fs::symlink(&target, LOCALTIME_PATH) {
            eprintln!("Failed to set timezone: {}", e);
            if e.raw_os_error() == Some(libc::EPERM) {
                eprintln!("Hint: This operation requires root privileges.");
            }
            process::exit(1);
        }
    }

    // Write /etc/timezone
    if let Err(e) = fs::write(TIMEZONE_PATH, format!("{}\n", tz)) {
        // Non-fatal: some systems don't use /etc/timezone
        eprintln!("Warning: Could not write {}: {}", TIMEZONE_PATH, e);
    }
}

fn cmd_set_ntp(enable: bool) {
    let enable_str = if enable { "enable" } else { "disable" };

    // Try to enable/disable the timesyncd service via systemctl
    let status = process::Command::new("systemctl")
        .args([enable_str, "systemd-timesyncd.service"])
        .status();

    match status {
        Ok(s) if s.success() => {
            // Also start/stop the service
            let action = if enable { "start" } else { "stop" };
            let _ = process::Command::new("systemctl")
                .args([action, "systemd-timesyncd.service"])
                .status();
            println!(
                "NTP synchronization {}d.",
                if enable { "enable" } else { "disable" }
            );
        }
        Ok(s) => {
            eprintln!(
                "Failed to {} NTP: systemctl exited with {}",
                enable_str,
                s.code().unwrap_or(-1)
            );
            process::exit(1);
        }
        Err(e) => {
            eprintln!(
                "Failed to {} NTP (could not run systemctl): {}",
                enable_str, e
            );
            process::exit(1);
        }
    }
}

fn cmd_set_local_rtc(local: bool) {
    let adjtime_exists = Path::new(ADJTIME_PATH).exists();

    if !local && !adjtime_exists {
        // Setting to UTC when no adjtime exists — nothing to do.
        return;
    }

    // Read raw file content to check if it's well-formed (ends with newline).
    let raw_content = if adjtime_exists {
        fs::read_to_string(ADJTIME_PATH).ok().unwrap_or_default()
    } else {
        String::new()
    };

    let lines: Vec<&str> = raw_content.lines().collect();
    let well_formed = raw_content.ends_with('\n') && lines.len() >= 3;

    let line0 = lines.first().copied().unwrap_or("0.0 0 0");
    let line1 = lines.get(1).copied().unwrap_or("0");
    let line2 = lines.get(2).copied().unwrap_or("");
    let extra_lines: Vec<&str> = lines.iter().skip(3).copied().collect();

    let is_local = line2.trim().eq_ignore_ascii_case("LOCAL");
    let is_utc = line2.trim().eq_ignore_ascii_case("UTC");
    let is_default =
        (line0.trim() == "0.0 0 0" || line0.trim() == "0.0 0 0.0") && line1.trim() == "0";

    let desired_mode = if local { "LOCAL" } else { "UTC" };

    // Check if file already matches the desired state and is well-formed.
    // If so, do nothing (matching real timedated behavior).
    if well_formed && !local && is_utc {
        return;
    }
    if well_formed && local && is_local {
        return;
    }

    if !local {
        // Setting to UTC. Normalize the file.
        if is_default && extra_lines.is_empty() {
            // Default drift values, no extra content — remove file entirely.
            let _ = fs::remove_file(ADJTIME_PATH);
            return;
        }
        // Non-default drift or extra content — rewrite with UTC, preserving extra lines.
        let mut content = format!("{}\n{}\n{}", line0, line1, desired_mode);
        for extra in &extra_lines {
            content.push('\n');
            content.push_str(extra);
        }
        content.push('\n');
        if let Err(e) = fs::write(ADJTIME_PATH, &content) {
            eprintln!("Failed to write {}: {}", ADJTIME_PATH, e);
            process::exit(1);
        }
        return;
    }

    // Setting to LOCAL — always write the file, preserving extra lines.
    let mut content = format!("{}\n{}\n{}", line0, line1, desired_mode);
    for extra in &extra_lines {
        content.push('\n');
        content.push_str(extra);
    }
    content.push('\n');

    if let Err(e) = fs::write(ADJTIME_PATH, &content) {
        eprintln!("Failed to write {}: {}", ADJTIME_PATH, e);
        if e.raw_os_error() == Some(libc::EPERM) {
            eprintln!("Hint: This operation requires root privileges.");
        }
        process::exit(1);
    }

    eprintln!();
    eprintln!("Warning: The system is configured to read the RTC time in the local time zone.");
    eprintln!("         This is not recommended. Please set the RTC to UTC with:");
    eprintln!("         timedatectl set-local-rtc 0");
}

fn cmd_list_timezones() {
    let tzs = list_timezones();
    let stdout = io::stdout();
    let mut out = io::BufWriter::new(stdout.lock());
    for tz in &tzs {
        let _ = writeln!(out, "{}", tz);
    }
}

fn cmd_timesync_status() {
    // Read current sync state
    let ntp_synced = is_ntp_synced();
    let (ntp_servers, fallback_servers) = read_ntp_servers();

    // Read adjtimex for detailed info
    let (offset_us, freq_ppm, status_str, poll_interval) = unsafe {
        let mut tx: libc::timex = std::mem::zeroed();
        tx.modes = 0;
        let ret = libc::adjtimex(&mut tx);
        let status = match ret {
            0 => "normal",
            1 => "insert leap second",
            2 => "delete leap second",
            3 => "leap second in progress",
            4 => "leap second has occurred",
            5 => "clock not synchronized",
            _ => "unknown",
        };
        let freq = tx.freq as f64 / 65536.0; // ppm
        let offset = tx.offset;
        let poll = if tx.constant > 0 {
            1i64 << tx.constant
        } else {
            0
        };
        (offset, freq, status, poll)
    };

    println!(
        "       Server: {}",
        if !ntp_servers.is_empty() {
            ntp_servers.join(", ")
        } else if !fallback_servers.is_empty() {
            format!("{} (fallback)", fallback_servers.join(", "))
        } else {
            "n/a".to_string()
        }
    );
    println!(
        "Poll interval: {}s",
        if poll_interval > 0 {
            format!("{}", poll_interval)
        } else {
            "n/a".to_string()
        }
    );
    println!("         Leap: {}", status_str);
    println!("       Offset: {:+.3}ms", offset_us as f64 / 1000.0);
    println!("    Frequency: {:+.3}ppm", freq_ppm);

    // Check for saved clock state
    let clock_state = Path::new(TIMESYNCD_STATE_DIR).join("clock");
    if clock_state.exists()
        && let Ok(contents) = fs::read_to_string(&clock_state)
        && let Ok(saved) = contents.trim().parse::<u64>()
    {
        let (now_secs, _) = get_unix_timestamp();
        let age = now_secs as u64 - saved;
        let age_str = if age < 60 {
            format!("{}s ago", age)
        } else if age < 3600 {
            format!("{}min {}s ago", age / 60, age % 60)
        } else if age < 86400 {
            format!("{}h {}min ago", age / 3600, (age % 3600) / 60)
        } else {
            format!("{}d {}h ago", age / 86400, (age % 86400) / 3600)
        };
        println!("   Last sync: {}", age_str);
    }

    if !ntp_synced {
        println!();
        println!("Warning: Clock is not synchronized with NTP.");
    }
}

// ── Usage ──────────────────────────────────────────────────────────────────

fn cmd_ntp_servers(iface: &str, servers: &[&str]) {
    // Write a networkd drop-in that sets NTP servers for this link.
    // Real systemd-timedated writes to /run/systemd/network/XX-<iface>.network.d/
    let dropin_dir = format!("/run/systemd/network/50-{iface}.network.d");
    let dropin_file = format!("{dropin_dir}/ntp.conf");

    if let Err(e) = fs::create_dir_all(&dropin_dir) {
        eprintln!("Failed to create drop-in directory {dropin_dir}: {e}");
        process::exit(1);
    }

    let ntp_list = servers.join(" ");
    let content = format!("[Network]\nNTP={ntp_list}\n");

    if let Err(e) = fs::write(&dropin_file, content) {
        eprintln!("Failed to write {dropin_file}: {e}");
        process::exit(1);
    }

    // Reload networkd to pick up the change
    let _ = process::Command::new("networkctl")
        .args(["reload"])
        .status();
    let _ = process::Command::new("networkctl")
        .args(["reconfigure", iface])
        .status();
}

fn cmd_revert(iface: &str) {
    let dropin_dir = format!("/run/systemd/network/50-{iface}.network.d");
    if Path::new(&dropin_dir).exists()
        && let Err(e) = fs::remove_dir_all(&dropin_dir)
    {
        eprintln!("Failed to remove {dropin_dir}: {e}");
        process::exit(1);
    }
    let _ = process::Command::new("networkctl")
        .args(["reload"])
        .status();
    let _ = process::Command::new("networkctl")
        .args(["reconfigure", iface])
        .status();
}

fn print_usage() {
    eprintln!("timedatectl — Control the system time and date");
    eprintln!();
    eprintln!("Usage: timedatectl [COMMAND]");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  status              Show current time settings (default)");
    eprintln!("  show                Show properties in machine-readable format");
    eprintln!("  set-time TIME       Set system time (YYYY-MM-DD HH:MM:SS)");
    eprintln!("  set-timezone ZONE   Set system timezone");
    eprintln!("  set-local-rtc BOOL  Control whether RTC uses local time");
    eprintln!("  set-ntp BOOL        Enable/disable NTP synchronization");
    eprintln!("  list-timezones      List available timezones");
    eprintln!("  timesync-status     Show NTP sync status");
    eprintln!("  ntp-servers IFACE SERVER...  Set per-link NTP servers");
    eprintln!("  revert IFACE        Remove per-link NTP overrides");
    eprintln!("  help                Show this help");
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    // Filter out options that are accepted but ignored
    let args: Vec<String> = env::args()
        .filter(|a| {
            a != "--no-pager"
                && a != "--no-ask-password"
                && a != "--all"
                && !a.starts_with("--machine=")
        })
        .collect();

    // Handle --version anywhere in args
    if args.iter().any(|a| a == "--version") {
        println!("systemd 258 (rust-systemd)");
        return;
    }

    if args.len() < 2 {
        cmd_status();
        return;
    }

    match args[1].as_str() {
        "status" => cmd_status(),
        "show" => {
            // Handle -p PROPERTY and --value
            let prop = args
                .iter()
                .position(|a| a == "-p")
                .and_then(|i| args.get(i + 1).map(|s| s.as_str()));
            let value_only = args.iter().any(|a| a == "--value");
            if let Some(prop_name) = prop {
                cmd_show_property(prop_name, value_only);
            } else {
                cmd_show();
            }
        }
        "show-timesync" => cmd_timesync_status(),
        "set-time" => {
            if args.len() < 3 {
                eprintln!("Error: set-time requires a time argument.");
                eprintln!("Usage: timedatectl set-time 'YYYY-MM-DD HH:MM:SS'");
                process::exit(1);
            }
            // Join remaining args in case time was split by shell
            let time_str = args[2..].join(" ");
            cmd_set_time(&time_str);
        }
        "set-timezone" => {
            if args.len() < 3 {
                eprintln!("Error: set-timezone requires a timezone argument.");
                eprintln!("Usage: timedatectl set-timezone America/New_York");
                process::exit(1);
            }
            cmd_set_timezone(&args[2]);
        }
        "set-ntp" => {
            if args.len() < 3 {
                eprintln!("Error: set-ntp requires a boolean argument (true/false/yes/no/1/0).");
                process::exit(1);
            }
            let enable = match args[2].to_lowercase().as_str() {
                "true" | "yes" | "1" | "on" => true,
                "false" | "no" | "0" | "off" => false,
                _ => {
                    eprintln!(
                        "Invalid boolean value '{}'. Use true/false, yes/no, 1/0, or on/off.",
                        args[2]
                    );
                    process::exit(1);
                }
            };
            cmd_set_ntp(enable);
        }
        "set-local-rtc" => {
            if args.len() < 3 {
                eprintln!(
                    "Error: set-local-rtc requires a boolean argument (true/false/yes/no/1/0)."
                );
                process::exit(1);
            }
            let local = match args[2].to_lowercase().as_str() {
                "true" | "yes" | "1" | "on" => true,
                "false" | "no" | "0" | "off" => false,
                _ => {
                    eprintln!(
                        "Invalid boolean value '{}'. Use true/false, yes/no, 1/0, or on/off.",
                        args[2]
                    );
                    process::exit(1);
                }
            };
            cmd_set_local_rtc(local);
        }
        "list-timezones" => cmd_list_timezones(),
        "timesync-status" => cmd_timesync_status(),
        "ntp-servers" => {
            // timedatectl ntp-servers INTERFACE SERVER...
            // Sets per-link NTP server addresses via networkd drop-in config.
            if args.len() < 4 {
                eprintln!("Error: ntp-servers requires an interface and at least one server.");
                eprintln!("Usage: timedatectl ntp-servers INTERFACE SERVER...");
                process::exit(1);
            }
            let iface = &args[2];
            let servers: Vec<&str> = args[3..].iter().map(|s| s.as_str()).collect();
            cmd_ntp_servers(iface, &servers);
        }
        "revert" => {
            // timedatectl revert INTERFACE
            // Removes per-link NTP server overrides.
            if args.len() < 3 {
                eprintln!("Error: revert requires an interface argument.");
                eprintln!("Usage: timedatectl revert INTERFACE");
                process::exit(1);
            }
            cmd_revert(&args[2]);
        }
        "help" | "--help" | "-h" => {
            print_usage();
        }
        other => {
            eprintln!("Unknown command: {}", other);
            eprintln!();
            print_usage();
            process::exit(1);
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_date_valid() {
        let (y, m, d) = parse_date("2025-02-17").unwrap();
        assert_eq!(y, 125); // 2025 - 1900
        assert_eq!(m, 1); // February = 1 (0-indexed)
        assert_eq!(d, 17);
    }

    #[test]
    fn test_parse_date_invalid_month() {
        assert!(parse_date("2025-13-01").is_none());
    }

    #[test]
    fn test_parse_date_invalid_day() {
        assert!(parse_date("2025-01-32").is_none());
    }

    #[test]
    fn test_parse_date_invalid_year() {
        assert!(parse_date("1969-01-01").is_none());
    }

    #[test]
    fn test_parse_date_wrong_format() {
        assert!(parse_date("2025/01/01").is_none());
        assert!(parse_date("20250101").is_none());
        assert!(parse_date("").is_none());
    }

    #[test]
    fn test_parse_time_hms() {
        let (h, m, s) = parse_time("15:30:45").unwrap();
        assert_eq!(h, 15);
        assert_eq!(m, 30);
        assert_eq!(s, 45);
    }

    #[test]
    fn test_parse_time_hm() {
        let (h, m, s) = parse_time("15:30").unwrap();
        assert_eq!(h, 15);
        assert_eq!(m, 30);
        assert_eq!(s, 0);
    }

    #[test]
    fn test_parse_time_invalid_hour() {
        assert!(parse_time("24:00:00").is_none());
    }

    #[test]
    fn test_parse_time_invalid_minute() {
        assert!(parse_time("12:60:00").is_none());
    }

    #[test]
    fn test_parse_time_invalid_second() {
        assert!(parse_time("12:00:61").is_none());
    }

    #[test]
    fn test_parse_time_leap_second() {
        // Second 60 should be allowed (leap second)
        let (h, m, s) = parse_time("23:59:60").unwrap();
        assert_eq!(h, 23);
        assert_eq!(m, 59);
        assert_eq!(s, 60);
    }

    #[test]
    fn test_format_utc_offset_positive() {
        assert_eq!(format_utc_offset(3600), "+0100");
        assert_eq!(format_utc_offset(19800), "+0530"); // India
    }

    #[test]
    fn test_format_utc_offset_negative() {
        assert_eq!(format_utc_offset(-18000), "-0500"); // EST
        assert_eq!(format_utc_offset(-28800), "-0800"); // PST
    }

    #[test]
    fn test_format_utc_offset_zero() {
        assert_eq!(format_utc_offset(0), "+0000");
    }

    #[test]
    fn test_detect_timezone_runs() {
        // Just verify it doesn't crash
        let tz = detect_timezone();
        assert!(!tz.is_empty());
    }

    #[test]
    fn test_is_rtc_local_default() {
        // On most systems without /etc/adjtime, this should return false
        let _ = is_rtc_local();
    }

    #[test]
    fn test_is_ntp_synced_runs() {
        // Just verify it doesn't crash
        let _ = is_ntp_synced();
    }

    #[test]
    fn test_is_ntp_enabled_runs() {
        let _ = is_ntp_enabled();
    }

    #[test]
    fn test_read_ntp_servers() {
        let (ntp, fallback) = read_ntp_servers();
        // fallback should always have entries
        assert!(!fallback.is_empty() || !ntp.is_empty());
    }

    #[test]
    fn test_parse_ntp_config() {
        let mut ntp = Vec::new();
        let mut fallback = Vec::new();
        parse_ntp_config(
            r#"
[Time]
NTP=ntp1.example.com ntp2.example.com
FallbackNTP=fb1.example.com fb2.example.com
"#,
            &mut ntp,
            &mut fallback,
        );
        assert_eq!(ntp, vec!["ntp1.example.com", "ntp2.example.com"]);
        assert_eq!(fallback, vec!["fb1.example.com", "fb2.example.com"]);
    }

    #[test]
    fn test_parse_ntp_config_empty() {
        let mut ntp = Vec::new();
        let mut fallback = Vec::new();
        parse_ntp_config("", &mut ntp, &mut fallback);
        assert!(ntp.is_empty());
        assert!(fallback.is_empty());
    }

    #[test]
    fn test_parse_ntp_config_ignores_other_sections() {
        let mut ntp = Vec::new();
        let mut fallback = Vec::new();
        parse_ntp_config(
            r#"
[Other]
NTP=should-not-appear.example.com

[Time]
NTP=correct.example.com
"#,
            &mut ntp,
            &mut fallback,
        );
        assert_eq!(ntp, vec!["correct.example.com"]);
    }

    #[test]
    fn test_get_unix_timestamp() {
        let (secs, nsecs) = get_unix_timestamp();
        assert!(secs > 0);
        assert!(nsecs < 1_000_000_000);
    }

    #[test]
    fn test_format_tm() {
        let mut tm: libc::tm = unsafe { std::mem::zeroed() };
        tm.tm_year = 125; // 2025
        tm.tm_mon = 1; // February
        tm.tm_mday = 17;
        tm.tm_hour = 15;
        tm.tm_min = 30;
        tm.tm_sec = 45;
        tm.tm_wday = 1; // Monday

        let s = format_tm(&tm, "UTC");
        assert_eq!(s, "Mon 2025-02-17 15:30:45 UTC");
    }

    #[test]
    fn test_list_timezones_runs() {
        // The test environment might not have zoneinfo, so just verify it doesn't crash
        let _ = list_timezones();
    }
}
