//! systemd-sleep — Put the system to sleep (suspend, hibernate, hybrid-sleep,
//! suspend-then-hibernate).
//!
//! A drop-in replacement for `systemd-sleep(8)`. This binary is typically
//! invoked by the service manager via `sleep.target` / `suspend.target` /
//! `hibernate.target` etc., not directly by users.
//!
//! It reads configuration from `/etc/systemd/sleep.conf` and
//! `/etc/systemd/sleep.conf.d/*.conf`, executes sleep hooks, writes to
//! `/sys/power/state` and `/sys/power/disk` as appropriate, and handles
//! the suspend-then-hibernate two-phase sleep mode.

use clap::Parser;
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{Duration, Instant};

/// Exit codes matching systemd conventions.
const EXIT_SUCCESS: i32 = 0;
const EXIT_FAILURE: i32 = 1;

/// Default paths for sleep configuration.
const SLEEP_CONF_PATH: &str = "/etc/systemd/sleep.conf";
const SLEEP_CONF_DIR: &str = "/etc/systemd/sleep.conf.d";

/// Kernel interfaces for sleep control.
const SYS_POWER_STATE: &str = "/sys/power/state";
const SYS_POWER_DISK: &str = "/sys/power/disk";
const SYS_POWER_RESUME: &str = "/sys/power/resume";
const SYS_POWER_MEM_SLEEP: &str = "/sys/power/mem_sleep";

/// The action to perform.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum SleepAction {
    Suspend,
    Hibernate,
    HybridSleep,
    SuspendThenHibernate,
}

impl SleepAction {
    fn from_verb(verb: &str) -> Option<Self> {
        match verb {
            "suspend" => Some(SleepAction::Suspend),
            "hibernate" => Some(SleepAction::Hibernate),
            "hybrid-sleep" => Some(SleepAction::HybridSleep),
            "suspend-then-hibernate" => Some(SleepAction::SuspendThenHibernate),
            _ => None,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            SleepAction::Suspend => "suspend",
            SleepAction::Hibernate => "hibernate",
            SleepAction::HybridSleep => "hybrid-sleep",
            SleepAction::SuspendThenHibernate => "suspend-then-hibernate",
        }
    }

    fn display_name(self) -> &'static str {
        match self {
            SleepAction::Suspend => "Suspend",
            SleepAction::Hibernate => "Hibernate",
            SleepAction::HybridSleep => "Hybrid-Sleep",
            SleepAction::SuspendThenHibernate => "Suspend-then-Hibernate",
        }
    }
}

/// Configuration parsed from sleep.conf.
#[derive(Debug, Clone)]
struct SleepConfig {
    /// Whether each sleep mode is allowed.
    allow_suspend: bool,
    allow_hibernate: bool,
    allow_hybrid_sleep: bool,
    allow_suspend_then_hibernate: bool,

    /// Suspend mode preference list (written to /sys/power/mem_sleep or
    /// used as the state value). Defaults to ["mem", "standby", "freeze"].
    suspend_mode: Vec<String>,
    /// Suspend state preference list (written to /sys/power/state).
    suspend_state: Vec<String>,

    /// Hibernate mode preference list (written to /sys/power/disk).
    hibernate_mode: Vec<String>,
    /// Hibernate state preference list (written to /sys/power/state).
    hibernate_state: Vec<String>,

    /// Hybrid-sleep mode preference list.
    hybrid_sleep_mode: Vec<String>,
    /// Hybrid-sleep state preference list.
    hybrid_sleep_state: Vec<String>,

    /// How long to suspend before waking for hibernate (seconds).
    hibernate_delay_sec: u64,
}

impl Default for SleepConfig {
    fn default() -> Self {
        SleepConfig {
            allow_suspend: true,
            allow_hibernate: true,
            allow_hybrid_sleep: true,
            allow_suspend_then_hibernate: true,
            suspend_mode: vec![],
            suspend_state: vec![
                "mem".to_string(),
                "standby".to_string(),
                "freeze".to_string(),
            ],
            hibernate_mode: vec!["platform".to_string(), "shutdown".to_string()],
            hibernate_state: vec!["disk".to_string()],
            hybrid_sleep_mode: vec![
                "suspend".to_string(),
                "platform".to_string(),
                "shutdown".to_string(),
            ],
            hybrid_sleep_state: vec!["disk".to_string()],
            hibernate_delay_sec: 7200, // 2 hours
        }
    }
}

impl SleepConfig {
    fn load() -> Self {
        let mut config = SleepConfig::default();

        // Load main config file
        if let Ok(contents) = fs::read_to_string(SLEEP_CONF_PATH) {
            config.parse_config(&contents);
        }

        // Load drop-in config files
        if let Ok(entries) = fs::read_dir(SLEEP_CONF_DIR) {
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
        let mut in_sleep_section = false;
        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }
            if line.starts_with('[') {
                in_sleep_section = line == "[Sleep]";
                continue;
            }
            if !in_sleep_section {
                continue;
            }
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                match key {
                    "AllowSuspend" => self.allow_suspend = parse_bool(value),
                    "AllowHibernation" => self.allow_hibernate = parse_bool(value),
                    "AllowHybridSleep" => self.allow_hybrid_sleep = parse_bool(value),
                    "AllowSuspendThenHibernate" => {
                        self.allow_suspend_then_hibernate = parse_bool(value);
                    }
                    "SuspendMode" => self.suspend_mode = parse_word_list(value),
                    "SuspendState" => self.suspend_state = parse_word_list(value),
                    "HibernateMode" => self.hibernate_mode = parse_word_list(value),
                    "HibernateState" => self.hibernate_state = parse_word_list(value),
                    "HybridSleepMode" => self.hybrid_sleep_mode = parse_word_list(value),
                    "HybridSleepState" => self.hybrid_sleep_state = parse_word_list(value),
                    "HibernateDelaySec" => {
                        if let Some(secs) = parse_timespan(value) {
                            self.hibernate_delay_sec = secs;
                        }
                    }
                    _ => {} // Ignore unknown keys
                }
            }
        }
    }

    fn is_action_allowed(&self, action: SleepAction) -> bool {
        match action {
            SleepAction::Suspend => self.allow_suspend,
            SleepAction::Hibernate => self.allow_hibernate,
            SleepAction::HybridSleep => self.allow_hybrid_sleep,
            SleepAction::SuspendThenHibernate => self.allow_suspend_then_hibernate,
        }
    }
}

fn parse_bool(s: &str) -> bool {
    matches!(s.to_lowercase().as_str(), "yes" | "true" | "1" | "on" | "y")
}

fn parse_word_list(s: &str) -> Vec<String> {
    s.split_whitespace().map(|w| w.to_string()).collect()
}

/// Parse a simple timespan value. Supports bare seconds, and suffixes
/// like "s", "min", "h", "d".
fn parse_timespan(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Try bare number (seconds)
    if let Ok(n) = s.parse::<u64>() {
        return Some(n);
    }

    // Try with suffix
    let suffixes: &[(&str, u64)] = &[
        ("min", 60),
        ("minutes", 60),
        ("minute", 60),
        ("sec", 1),
        ("second", 1),
        ("seconds", 1),
        ("hr", 3600),
        ("hour", 3600),
        ("hours", 3600),
        ("day", 86400),
        ("days", 86400),
        ("ms", 0), // milliseconds rounded to 0 seconds
        ("us", 0), // microseconds rounded to 0 seconds
        ("h", 3600),
        ("m", 60),
        ("s", 1),
        ("d", 86400),
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

/// Read a kernel sysfs file and return its trimmed contents.
fn read_sysfs(path: &str) -> io::Result<String> {
    fs::read_to_string(path).map(|s| s.trim().to_string())
}

/// Write a string to a kernel sysfs file.
fn write_sysfs(path: &str, value: &str) -> io::Result<()> {
    let mut f = fs::OpenOptions::new().write(true).open(path)?;
    f.write_all(value.as_bytes())?;
    f.flush()
}

/// Get the set of available sleep states from /sys/power/state.
fn available_states() -> Vec<String> {
    match read_sysfs(SYS_POWER_STATE) {
        Ok(s) => s.split_whitespace().map(|w| w.to_string()).collect(),
        Err(_) => vec![],
    }
}

/// Get the set of available disk modes from /sys/power/disk.
/// The file looks like: "[platform] shutdown reboot suspend"
/// where the bracketed one is currently selected.
fn available_disk_modes() -> Vec<String> {
    match read_sysfs(SYS_POWER_DISK) {
        Ok(s) => s
            .split_whitespace()
            .map(|w| w.trim_start_matches('[').trim_end_matches(']').to_string())
            .collect(),
        Err(_) => vec![],
    }
}

/// Get available suspend modes from /sys/power/mem_sleep.
/// The file looks like: "s2idle [deep]"
fn available_mem_sleep_modes() -> Vec<String> {
    match read_sysfs(SYS_POWER_MEM_SLEEP) {
        Ok(s) => s
            .split_whitespace()
            .map(|w| w.trim_start_matches('[').trim_end_matches(']').to_string())
            .collect(),
        Err(_) => vec![],
    }
}

/// Check whether hibernation has a usable resume device configured.
fn has_resume_device() -> bool {
    // Check /sys/power/resume for a non-zero device
    if let Ok(contents) = read_sysfs(SYS_POWER_RESUME)
        && contents != "0:0"
        && !contents.is_empty()
        && contents != "0"
    {
        return true;
    }
    // Also check the kernel command line for a resume= parameter
    if let Ok(cmdline) = fs::read_to_string("/proc/cmdline")
        && cmdline.split_whitespace().any(|w| w.starts_with("resume="))
    {
        return true;
    }
    false
}

/// Check whether a particular sleep action can be performed on this system.
fn can_sleep(action: SleepAction, config: &SleepConfig) -> bool {
    if !config.is_action_allowed(action) {
        return false;
    }

    let states = available_states();

    match action {
        SleepAction::Suspend => {
            // Need at least one of the configured suspend states to be available
            config.suspend_state.iter().any(|s| states.contains(s))
        }
        SleepAction::Hibernate => {
            // Need "disk" state and a resume device
            if !states.contains(&"disk".to_string()) {
                return false;
            }
            has_resume_device()
        }
        SleepAction::HybridSleep => {
            // Need "disk" state and a resume device
            if !states.contains(&"disk".to_string()) {
                return false;
            }
            has_resume_device()
        }
        SleepAction::SuspendThenHibernate => {
            // Need both suspend and hibernate to work
            let can_suspend = config.suspend_state.iter().any(|s| states.contains(s));
            let can_hibernate = states.contains(&"disk".to_string()) && has_resume_device();
            can_suspend && can_hibernate
        }
    }
}

/// Execute pre/post sleep hooks by running scripts in the hook directories.
fn run_hooks(action: SleepAction, phase: &str) {
    let hook_dirs = [
        "/usr/lib/systemd/system-sleep".to_string(),
        "/etc/systemd/system-sleep".to_string(),
    ];

    for dir in &hook_dirs {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let mut entries: Vec<PathBuf> = match fs::read_dir(dir_path) {
            Ok(rd) => rd.filter_map(|e| e.ok()).map(|e| e.path()).collect(),
            Err(_) => continue,
        };
        entries.sort();

        for script in entries {
            if !script.is_file() {
                continue;
            }

            // Check if executable
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = match script.metadata() {
                    Ok(m) => m.permissions(),
                    Err(_) => continue,
                };
                if perms.mode() & 0o111 == 0 {
                    continue;
                }
            }

            let verb = action.as_str();
            eprintln!(
                "systemd-sleep: Executing {} hook: {}",
                phase,
                script.display()
            );
            let result = process::Command::new(&script).arg(phase).arg(verb).status();
            match result {
                Ok(status) if !status.success() => {
                    eprintln!(
                        "systemd-sleep: Hook {} exited with status {}",
                        script.display(),
                        status
                    );
                }
                Err(e) => {
                    eprintln!(
                        "systemd-sleep: Failed to execute hook {}: {}",
                        script.display(),
                        e
                    );
                }
                _ => {}
            }
        }
    }
}

/// Perform a single suspend operation.
fn do_suspend(config: &SleepConfig) -> Result<(), String> {
    let states = available_states();
    let mem_modes = available_mem_sleep_modes();

    // If there are suspend mode preferences, try to set /sys/power/mem_sleep
    for mode in &config.suspend_mode {
        if mem_modes.contains(mode) {
            eprintln!("systemd-sleep: Setting mem_sleep to {}", mode);
            if let Err(e) = write_sysfs(SYS_POWER_MEM_SLEEP, mode) {
                eprintln!("systemd-sleep: Failed to set mem_sleep to {}: {}", mode, e);
            } else {
                break;
            }
        }
    }

    // Write to /sys/power/state
    for state in &config.suspend_state {
        if states.contains(state) {
            eprintln!("systemd-sleep: Suspending system (state={})...", state);
            return write_sysfs(SYS_POWER_STATE, state)
                .map_err(|e| format!("Failed to write '{}' to {}: {}", state, SYS_POWER_STATE, e));
        }
    }

    Err("No supported suspend state available".to_string())
}

/// Perform a hibernate operation.
fn do_hibernate(config: &SleepConfig) -> Result<(), String> {
    let states = available_states();
    let disk_modes = available_disk_modes();

    // Set the hibernate mode in /sys/power/disk
    for mode in &config.hibernate_mode {
        if disk_modes.contains(mode) {
            eprintln!("systemd-sleep: Setting disk mode to {}", mode);
            if let Err(e) = write_sysfs(SYS_POWER_DISK, mode) {
                eprintln!("systemd-sleep: Failed to set disk mode to {}: {}", mode, e);
            } else {
                break;
            }
        }
    }

    // Write to /sys/power/state
    for state in &config.hibernate_state {
        if states.contains(state) {
            eprintln!("systemd-sleep: Hibernating system (state={})...", state);
            return write_sysfs(SYS_POWER_STATE, state)
                .map_err(|e| format!("Failed to write '{}' to {}: {}", state, SYS_POWER_STATE, e));
        }
    }

    Err("No supported hibernate state available".to_string())
}

/// Perform a hybrid-sleep operation (suspend + hibernate simultaneously).
fn do_hybrid_sleep(config: &SleepConfig) -> Result<(), String> {
    let states = available_states();
    let disk_modes = available_disk_modes();

    // Set the disk mode to "suspend" for hybrid-sleep
    for mode in &config.hybrid_sleep_mode {
        if disk_modes.contains(mode) {
            eprintln!(
                "systemd-sleep: Setting disk mode to {} for hybrid-sleep",
                mode
            );
            if let Err(e) = write_sysfs(SYS_POWER_DISK, mode) {
                eprintln!("systemd-sleep: Failed to set disk mode to {}: {}", mode, e);
            } else {
                break;
            }
        }
    }

    // Write to /sys/power/state
    for state in &config.hybrid_sleep_state {
        if states.contains(state) {
            eprintln!("systemd-sleep: Entering hybrid-sleep (state={})...", state);
            return write_sysfs(SYS_POWER_STATE, state)
                .map_err(|e| format!("Failed to write '{}' to {}: {}", state, SYS_POWER_STATE, e));
        }
    }

    Err("No supported hybrid-sleep state available".to_string())
}

/// Perform suspend-then-hibernate: suspend for a configured duration, then
/// wake up via RTC alarm and hibernate.
fn do_suspend_then_hibernate(config: &SleepConfig) -> Result<(), String> {
    // Set up an RTC alarm to wake from suspend after the configured delay.
    let delay = Duration::from_secs(config.hibernate_delay_sec);
    eprintln!(
        "systemd-sleep: Will suspend for {} seconds, then hibernate",
        config.hibernate_delay_sec
    );

    // Try to set RTC wakealarm
    let rtc_path = find_rtc_device();
    let alarm_set = if let Some(ref rtc) = rtc_path {
        set_rtc_wakealarm(rtc, config.hibernate_delay_sec)
    } else {
        false
    };

    if !alarm_set {
        eprintln!(
            "systemd-sleep: Warning: Could not set RTC wake alarm; \
             will suspend indefinitely (manual wakeup required for hibernate)"
        );
    }

    // Phase 1: Suspend
    let suspend_start = Instant::now();
    do_suspend(config)?;

    // We've woken up — check if we slept long enough to hibernate
    let elapsed = suspend_start.elapsed();

    // Clear the RTC alarm
    if let Some(ref rtc) = rtc_path {
        clear_rtc_wakealarm(rtc);
    }

    if elapsed >= delay.saturating_sub(Duration::from_secs(30)) {
        // We slept approximately long enough (within 30s tolerance),
        // meaning the RTC alarm woke us. Time to hibernate.
        eprintln!(
            "systemd-sleep: Woke after {:.0}s (>= {}s delay), proceeding to hibernate",
            elapsed.as_secs_f64(),
            config.hibernate_delay_sec
        );
        do_hibernate(config)?;
    } else {
        eprintln!(
            "systemd-sleep: Woke after {:.0}s (< {}s delay), assuming user wakeup",
            elapsed.as_secs_f64(),
            config.hibernate_delay_sec
        );
    }

    Ok(())
}

/// Find an RTC device to use for wake alarms.
fn find_rtc_device() -> Option<PathBuf> {
    // Try rtc0 first, then look for others
    let rtc0 = PathBuf::from("/sys/class/rtc/rtc0");
    if rtc0.exists() {
        return Some(rtc0);
    }

    if let Ok(entries) = fs::read_dir("/sys/class/rtc") {
        for entry in entries.flatten() {
            let path = entry.path();
            if path.join("wakealarm").exists() {
                return Some(path);
            }
        }
    }

    None
}

/// Set an RTC wake alarm to fire after `delay_secs` seconds from now.
fn set_rtc_wakealarm(rtc_path: &Path, delay_secs: u64) -> bool {
    let wakealarm_path = rtc_path.join("wakealarm");
    if !wakealarm_path.exists() {
        return false;
    }

    // Clear any existing alarm first
    if write_sysfs(wakealarm_path.to_str().unwrap(), "0").is_err() {
        // Some kernels want an empty write to clear
        let _ = write_sysfs(wakealarm_path.to_str().unwrap(), "");
    }

    // Set the alarm using the relative format (+N seconds)
    let value = format!("+{}", delay_secs);
    match write_sysfs(wakealarm_path.to_str().unwrap(), &value) {
        Ok(()) => {
            eprintln!(
                "systemd-sleep: Set RTC wake alarm for +{}s on {}",
                delay_secs,
                rtc_path.display()
            );
            true
        }
        Err(e) => {
            eprintln!(
                "systemd-sleep: Failed to set RTC wake alarm on {}: {}",
                rtc_path.display(),
                e
            );
            false
        }
    }
}

/// Clear the RTC wake alarm.
fn clear_rtc_wakealarm(rtc_path: &Path) {
    let wakealarm_path = rtc_path.join("wakealarm");
    let _ = write_sysfs(wakealarm_path.to_str().unwrap(), "0");
}

#[derive(Parser, Debug)]
#[command(name = "systemd-sleep", about = "Put the system to sleep", version)]
struct Cli {
    /// The sleep action to perform: suspend, hibernate, hybrid-sleep,
    /// or suspend-then-hibernate.
    action: String,
}

fn main() {
    let cli = Cli::parse();

    let action = match SleepAction::from_verb(&cli.action) {
        Some(a) => a,
        None => {
            eprintln!(
                "systemd-sleep: Unknown action '{}'. \
                 Expected: suspend, hibernate, hybrid-sleep, suspend-then-hibernate",
                cli.action
            );
            process::exit(EXIT_FAILURE);
        }
    };

    let config = SleepConfig::load();

    // Check if this action is allowed
    if !config.is_action_allowed(action) {
        eprintln!(
            "systemd-sleep: {} is disabled by configuration",
            action.display_name()
        );
        process::exit(EXIT_FAILURE);
    }

    // Check if this action can be performed on this system
    if !can_sleep(action, &config) {
        eprintln!(
            "systemd-sleep: {} is not supported on this system",
            action.display_name()
        );
        process::exit(EXIT_FAILURE);
    }

    eprintln!("systemd-sleep: {}...", action.display_name());

    // Run pre-sleep hooks
    run_hooks(action, "pre");

    // Execute the sleep action
    let result = match action {
        SleepAction::Suspend => do_suspend(&config),
        SleepAction::Hibernate => do_hibernate(&config),
        SleepAction::HybridSleep => do_hybrid_sleep(&config),
        SleepAction::SuspendThenHibernate => do_suspend_then_hibernate(&config),
    };

    // Run post-sleep hooks (even on failure, matching systemd behavior)
    run_hooks(action, "post");

    match result {
        Ok(()) => {
            eprintln!(
                "systemd-sleep: System returned from {}",
                action.display_name()
            );
            process::exit(EXIT_SUCCESS);
        }
        Err(e) => {
            eprintln!("systemd-sleep: Failed to {}: {}", action.as_str(), e);
            process::exit(EXIT_FAILURE);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sleep_action_from_verb() {
        assert_eq!(
            SleepAction::from_verb("suspend"),
            Some(SleepAction::Suspend)
        );
        assert_eq!(
            SleepAction::from_verb("hibernate"),
            Some(SleepAction::Hibernate)
        );
        assert_eq!(
            SleepAction::from_verb("hybrid-sleep"),
            Some(SleepAction::HybridSleep)
        );
        assert_eq!(
            SleepAction::from_verb("suspend-then-hibernate"),
            Some(SleepAction::SuspendThenHibernate)
        );
        assert_eq!(SleepAction::from_verb("unknown"), None);
        assert_eq!(SleepAction::from_verb(""), None);
    }

    #[test]
    fn test_sleep_action_roundtrip() {
        for action in [
            SleepAction::Suspend,
            SleepAction::Hibernate,
            SleepAction::HybridSleep,
            SleepAction::SuspendThenHibernate,
        ] {
            assert_eq!(SleepAction::from_verb(action.as_str()), Some(action));
        }
    }

    #[test]
    fn test_parse_bool() {
        assert!(parse_bool("yes"));
        assert!(parse_bool("true"));
        assert!(parse_bool("1"));
        assert!(parse_bool("on"));
        assert!(parse_bool("y"));
        assert!(parse_bool("Yes"));
        assert!(parse_bool("TRUE"));
        assert!(!parse_bool("no"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool("off"));
        assert!(!parse_bool(""));
    }

    #[test]
    fn test_parse_word_list() {
        assert_eq!(
            parse_word_list("mem standby freeze"),
            vec!["mem", "standby", "freeze"]
        );
        assert_eq!(parse_word_list("platform"), vec!["platform"]);
        assert_eq!(parse_word_list(""), Vec::<String>::new());
        assert_eq!(parse_word_list("  mem   standby  "), vec!["mem", "standby"]);
    }

    #[test]
    fn test_parse_timespan() {
        assert_eq!(parse_timespan("3600"), Some(3600));
        assert_eq!(parse_timespan("120s"), Some(120));
        assert_eq!(parse_timespan("2h"), Some(7200));
        assert_eq!(parse_timespan("30min"), Some(1800));
        assert_eq!(parse_timespan("1d"), Some(86400));
        assert_eq!(parse_timespan("5m"), Some(300));
        assert_eq!(parse_timespan(""), None);
        assert_eq!(parse_timespan("abc"), None);
    }

    #[test]
    fn test_default_config() {
        let config = SleepConfig::default();
        assert!(config.allow_suspend);
        assert!(config.allow_hibernate);
        assert!(config.allow_hybrid_sleep);
        assert!(config.allow_suspend_then_hibernate);
        assert_eq!(config.suspend_state, vec!["mem", "standby", "freeze"]);
        assert_eq!(config.hibernate_mode, vec!["platform", "shutdown"]);
        assert_eq!(config.hibernate_state, vec!["disk"]);
        assert_eq!(config.hibernate_delay_sec, 7200);
    }

    #[test]
    fn test_parse_config() {
        let mut config = SleepConfig::default();
        let contents = r#"
[Sleep]
AllowSuspend=no
AllowHibernation=yes
SuspendState=freeze
HibernateMode=shutdown
HibernateDelaySec=3600
"#;
        config.parse_config(contents);
        assert!(!config.allow_suspend);
        assert!(config.allow_hibernate);
        assert_eq!(config.suspend_state, vec!["freeze"]);
        assert_eq!(config.hibernate_mode, vec!["shutdown"]);
        assert_eq!(config.hibernate_delay_sec, 3600);
    }

    #[test]
    fn test_parse_config_ignores_other_sections() {
        let mut config = SleepConfig::default();
        let contents = r#"
[Other]
AllowSuspend=no

[Sleep]
AllowHibernation=no

[AnotherSection]
AllowHibernation=yes
"#;
        config.parse_config(contents);
        // AllowSuspend was in [Other], so should stay default (true)
        assert!(config.allow_suspend);
        // AllowHibernation was in [Sleep], so should be false
        assert!(!config.allow_hibernate);
    }

    #[test]
    fn test_is_action_allowed() {
        let mut config = SleepConfig::default();
        assert!(config.is_action_allowed(SleepAction::Suspend));
        assert!(config.is_action_allowed(SleepAction::Hibernate));

        config.allow_suspend = false;
        assert!(!config.is_action_allowed(SleepAction::Suspend));
        assert!(config.is_action_allowed(SleepAction::Hibernate));

        config.allow_hibernate = false;
        assert!(!config.is_action_allowed(SleepAction::Hibernate));
    }

    #[test]
    fn test_display_names() {
        assert_eq!(SleepAction::Suspend.display_name(), "Suspend");
        assert_eq!(SleepAction::Hibernate.display_name(), "Hibernate");
        assert_eq!(SleepAction::HybridSleep.display_name(), "Hybrid-Sleep");
        assert_eq!(
            SleepAction::SuspendThenHibernate.display_name(),
            "Suspend-then-Hibernate"
        );
    }

    #[test]
    fn test_parse_timespan_with_suffixes() {
        assert_eq!(parse_timespan("1sec"), Some(1));
        assert_eq!(parse_timespan("1second"), Some(1));
        assert_eq!(parse_timespan("2seconds"), Some(2));
        assert_eq!(parse_timespan("1minute"), Some(60));
        assert_eq!(parse_timespan("2minutes"), Some(120));
        assert_eq!(parse_timespan("1hour"), Some(3600));
        assert_eq!(parse_timespan("2hours"), Some(7200));
        assert_eq!(parse_timespan("1day"), Some(86400));
        assert_eq!(parse_timespan("2days"), Some(172800));
    }
}
