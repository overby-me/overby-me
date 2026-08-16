//! systemd-oomd — Userspace Out-Of-Memory (OOM) Killer
//!
//! A Rust implementation of systemd-oomd that monitors memory pressure via PSI
//! (Pressure Stall Information) in cgroups v2 and kills processes in cgroups
//! that exceed configured thresholds.
//!
//! This implementation reads configuration from `/etc/systemd/oomd.conf` and
//! discovers managed cgroups by scanning slice unit files for `ManagedOOM*`
//! directives, avoiding the need for D-Bus or varlink communication with PID 1.

use std::collections::HashMap;
use std::fs;
use std::io;
use std::os::unix::net::UnixDatagram;
use std::path::{Path, PathBuf};
use std::process;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use log::{debug, info, trace, warn};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const CGROUP_ROOT: &str = "/sys/fs/cgroup";
const SYSTEM_PRESSURE_PATH: &str = "/proc/pressure/memory";

const DEFAULT_SWAP_USED_LIMIT_PERCENT: u64 = 90;
const DEFAULT_MEMORY_PRESSURE_LIMIT_PERCENT: u64 = 60;
const DEFAULT_MEMORY_PRESSURE_DURATION_SECS: u64 = 30;

/// How often the main monitoring loop runs (in seconds).
const MONITOR_INTERVAL_SECS: u64 = 1;

/// Minimum time between kills on the same cgroup to prevent kill storms.
const KILL_COOLDOWN_SECS: u64 = 5;

/// Unit file directories to scan for ManagedOOM directives.
const UNIT_DIRS: &[&str] = &[
    "/etc/systemd/system",
    "/run/systemd/system",
    "/usr/lib/systemd/system",
    "/lib/systemd/system",
    "/etc/systemd/system.control",
    "/run/systemd/system.control",
    "/run/systemd/generator",
    "/run/systemd/generator.early",
    "/run/systemd/generator.late",
];

// ---------------------------------------------------------------------------
// Configuration: /etc/systemd/oomd.conf
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct OomdConfig {
    /// Kill cgroups when total swap usage exceeds this percentage (0–100).
    swap_used_limit_percent: u64,
    /// Default duration over which memory pressure is averaged.
    default_memory_pressure_duration: Duration,
    /// Default memory pressure threshold percentage (0–100).
    default_memory_pressure_limit_percent: u64,
}

impl Default for OomdConfig {
    fn default() -> Self {
        Self {
            swap_used_limit_percent: DEFAULT_SWAP_USED_LIMIT_PERCENT,
            default_memory_pressure_duration: Duration::from_secs(
                DEFAULT_MEMORY_PRESSURE_DURATION_SECS,
            ),
            default_memory_pressure_limit_percent: DEFAULT_MEMORY_PRESSURE_LIMIT_PERCENT,
        }
    }
}

impl OomdConfig {
    fn load() -> Self {
        let mut cfg = Self::default();

        for path in &[
            "/etc/systemd/oomd.conf",
            "/etc/systemd/oomd.conf.d",
            "/run/systemd/oomd.conf.d",
            "/usr/lib/systemd/oomd.conf.d",
        ] {
            let p = Path::new(path);
            if p.is_file() {
                if let Ok(contents) = fs::read_to_string(p) {
                    cfg.parse_config(&contents);
                }
            } else if p.is_dir()
                && let Ok(entries) = fs::read_dir(p)
            {
                let mut files: Vec<PathBuf> = entries
                    .filter_map(|e| e.ok())
                    .map(|e| e.path())
                    .filter(|p| p.extension().is_some_and(|ext| ext == "conf"))
                    .collect();
                files.sort();
                for file in files {
                    if let Ok(contents) = fs::read_to_string(&file) {
                        cfg.parse_config(&contents);
                    }
                }
            }
        }

        cfg
    }

    fn parse_config(&mut self, contents: &str) {
        let mut in_oom_section = false;

        for line in contents.lines() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            if line.starts_with('[') {
                in_oom_section = line.eq_ignore_ascii_case("[oom]");
                continue;
            }

            if !in_oom_section {
                continue;
            }

            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();

                match key {
                    "SwapUsedLimit" => {
                        if let Some(pct) = parse_percent(value) {
                            self.swap_used_limit_percent = pct;
                        }
                    }
                    "DefaultMemoryPressureDurationSec" => {
                        if let Some(secs) = parse_duration_secs(value) {
                            self.default_memory_pressure_duration = Duration::from_secs(secs);
                        }
                    }
                    "DefaultMemoryPressureLimit" => {
                        if let Some(pct) = parse_percent(value) {
                            self.default_memory_pressure_limit_percent = pct;
                        }
                    }
                    _ => {
                        trace!("oomd: ignoring unknown config key: {key}");
                    }
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// ManagedOOM policy (from unit files)
// ---------------------------------------------------------------------------

/// What action to take on OOM.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedOOMAction {
    /// No action (monitoring only).
    Auto,
    /// Kill processes in the cgroup.
    Kill,
}

/// Per-cgroup OOM preference.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ManagedOOMPreference {
    None,
    Avoid,
    Omit,
}

/// Parsed ManagedOOM settings for a single cgroup (derived from a unit file).
#[derive(Debug, Clone)]
struct ManagedOOMPolicy {
    /// The cgroup path to monitor.
    cgroup_path: PathBuf,
    /// What to do when swap usage is high.
    swap_action: ManagedOOMAction,
    /// What to do when memory pressure is high.
    memory_pressure_action: ManagedOOMAction,
    /// Override for the memory pressure limit (percentage).
    /// `None` means use the global default.
    memory_pressure_limit_percent: Option<u64>,
    /// Preference for this cgroup.
    preference: ManagedOOMPreference,
}

/// Discover managed cgroups by scanning unit files.
fn discover_managed_cgroups() -> Vec<ManagedOOMPolicy> {
    let mut policies = Vec::new();
    let mut seen_units: HashMap<String, PathBuf> = HashMap::new();

    // Scan unit dirs in priority order; first occurrence of a unit name wins.
    for dir in UNIT_DIRS {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let entries = match fs::read_dir(dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };

            // We care about .slice and .service units
            if !name.ends_with(".slice") && !name.ends_with(".service") {
                continue;
            }

            // First-seen wins (highest priority dir first in UNIT_DIRS)
            if seen_units.contains_key(&name) {
                continue;
            }
            seen_units.insert(name.clone(), path.clone());

            let contents = match fs::read_to_string(&path) {
                Ok(c) => c,
                Err(_) => continue,
            };

            if let Some(policy) = parse_managed_oom_from_unit(&name, &contents) {
                policies.push(policy);
            }
        }

        // Also scan drop-in directories (e.g., foo.slice.d/*.conf)
        let entries = match fs::read_dir(dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if !path.is_dir() {
                continue;
            }
            let dir_name = match path.file_name().and_then(|n| n.to_str()) {
                Some(n) => n.to_owned(),
                None => continue,
            };
            // Drop-in dirs end with .d and the base is a unit name
            if !dir_name.ends_with(".d") {
                continue;
            }
            let unit_name = &dir_name[..dir_name.len() - 2];
            if !unit_name.ends_with(".slice") && !unit_name.ends_with(".service") {
                continue;
            }

            // Read all .conf files in the drop-in directory
            let conf_entries = match fs::read_dir(&path) {
                Ok(e) => e,
                Err(_) => continue,
            };
            let mut conf_files: Vec<PathBuf> = conf_entries
                .filter_map(|e| e.ok())
                .map(|e| e.path())
                .filter(|p| p.extension().is_some_and(|ext| ext == "conf"))
                .collect();
            conf_files.sort();

            for conf_file in conf_files {
                let contents = match fs::read_to_string(&conf_file) {
                    Ok(c) => c,
                    Err(_) => continue,
                };
                if let Some(policy) = parse_managed_oom_from_unit(unit_name, &contents) {
                    // Drop-in overrides: replace existing policy for this cgroup
                    policies.retain(|p| p.cgroup_path != policy.cgroup_path);
                    policies.push(policy);
                }
            }
        }
    }

    policies
}

/// Parse ManagedOOM* directives from a unit file's contents.
fn parse_managed_oom_from_unit(unit_name: &str, contents: &str) -> Option<ManagedOOMPolicy> {
    let mut swap_action = ManagedOOMAction::Auto;
    let mut memory_pressure_action = ManagedOOMAction::Auto;
    let mut memory_pressure_limit: Option<u64> = None;
    let mut preference = ManagedOOMPreference::None;
    let mut found_managed = false;

    let mut in_relevant_section = false;

    for line in contents.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') {
            let section = line.to_ascii_lowercase();
            in_relevant_section = section == "[slice]" || section == "[service]";
            continue;
        }

        if !in_relevant_section {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();

            match key {
                "ManagedOOMSwap" => {
                    swap_action = parse_oom_action(value);
                    found_managed = true;
                }
                "ManagedOOMMemoryPressure" => {
                    memory_pressure_action = parse_oom_action(value);
                    found_managed = true;
                }
                "ManagedOOMMemoryPressureLimit" => {
                    memory_pressure_limit = parse_percent(value);
                    found_managed = true;
                }
                "ManagedOOMPreference" => {
                    preference = match value.to_ascii_lowercase().as_str() {
                        "avoid" => ManagedOOMPreference::Avoid,
                        "omit" => ManagedOOMPreference::Omit,
                        _ => ManagedOOMPreference::None,
                    };
                    found_managed = true;
                }
                _ => {}
            }
        }
    }

    if !found_managed {
        return None;
    }

    // If both actions are Auto and no explicit preference/limit, skip
    if swap_action == ManagedOOMAction::Auto
        && memory_pressure_action == ManagedOOMAction::Auto
        && preference == ManagedOOMPreference::None
        && memory_pressure_limit.is_none()
    {
        return None;
    }

    let cgroup_path = unit_name_to_cgroup_path(unit_name);

    Some(ManagedOOMPolicy {
        cgroup_path,
        swap_action,
        memory_pressure_action,
        memory_pressure_limit_percent: memory_pressure_limit,
        preference,
    })
}

fn parse_oom_action(value: &str) -> ManagedOOMAction {
    match value.to_ascii_lowercase().as_str() {
        "kill" => ManagedOOMAction::Kill,
        _ => ManagedOOMAction::Auto,
    }
}

/// Convert a systemd unit name to its cgroup v2 path.
///
/// Examples:
///   - `-.slice` → `/sys/fs/cgroup/`
///   - `system.slice` → `/sys/fs/cgroup/system.slice`
///   - `user.slice` → `/sys/fs/cgroup/user.slice`
///   - `foo-bar.slice` → `/sys/fs/cgroup/foo.slice/foo-bar.slice`
fn unit_name_to_cgroup_path(unit_name: &str) -> PathBuf {
    let root = PathBuf::from(CGROUP_ROOT);

    if unit_name == "-.slice" {
        return root;
    }

    root.join(unit_name)
}

// ---------------------------------------------------------------------------
// PSI (Pressure Stall Information) parsing
// ---------------------------------------------------------------------------

/// Parsed PSI entry.
#[derive(Debug, Clone, Default)]
struct PsiMetrics {
    /// "some" average over 10 seconds (percentage, 0.0–100.0).
    some_avg10: f64,
    /// "some" average over 60 seconds.
    some_avg60: f64,
    /// "some" average over 300 seconds.
    some_avg300: f64,
    /// "full" average over 10 seconds.
    full_avg10: f64,
    /// "full" average over 60 seconds.
    full_avg60: f64,
    /// "full" average over 300 seconds.
    full_avg300: f64,
}

impl PsiMetrics {
    /// Read PSI metrics from a pressure file (e.g., `memory.pressure` or `/proc/pressure/memory`).
    fn read(path: &Path) -> io::Result<Self> {
        let contents = fs::read_to_string(path)?;
        Ok(Self::parse(&contents))
    }

    fn parse(contents: &str) -> Self {
        let mut metrics = Self::default();

        for line in contents.lines() {
            let line = line.trim();
            if line.starts_with("some ") {
                let (avg10, avg60, avg300) = parse_psi_line(line);
                metrics.some_avg10 = avg10;
                metrics.some_avg60 = avg60;
                metrics.some_avg300 = avg300;
            } else if line.starts_with("full ") {
                let (avg10, avg60, avg300) = parse_psi_line(line);
                metrics.full_avg10 = avg10;
                metrics.full_avg60 = avg60;
                metrics.full_avg300 = avg300;
            }
        }

        metrics
    }

    /// Returns the pressure percentage to compare against thresholds.
    /// Uses `some avg10` as the primary metric for responsiveness, consistent
    /// with systemd-oomd's default behavior when duration ≤ 10s.
    /// For longer durations, uses avg60.
    fn effective_pressure(&self, duration: Duration) -> f64 {
        if duration.as_secs() <= 10 {
            self.some_avg10
        } else if duration.as_secs() <= 60 {
            self.some_avg60
        } else {
            self.some_avg300
        }
    }
}

/// Parse a PSI line like: `some avg10=0.00 avg60=0.00 avg300=0.00 total=0`
fn parse_psi_line(line: &str) -> (f64, f64, f64) {
    let mut avg10 = 0.0;
    let mut avg60 = 0.0;
    let mut avg300 = 0.0;

    for part in line.split_whitespace() {
        if let Some(val) = part.strip_prefix("avg10=") {
            avg10 = val.parse().unwrap_or(0.0);
        } else if let Some(val) = part.strip_prefix("avg60=") {
            avg60 = val.parse().unwrap_or(0.0);
        } else if let Some(val) = part.strip_prefix("avg300=") {
            avg300 = val.parse().unwrap_or(0.0);
        }
    }

    (avg10, avg60, avg300)
}

// ---------------------------------------------------------------------------
// Swap usage
// ---------------------------------------------------------------------------

/// Returns (swap_used, swap_total) in bytes, or None if swap info is unavailable.
fn read_swap_usage() -> Option<(u64, u64)> {
    let contents = fs::read_to_string("/proc/meminfo").ok()?;
    let mut swap_total: Option<u64> = None;
    let mut swap_free: Option<u64> = None;

    for line in contents.lines() {
        if let Some(rest) = line.strip_prefix("SwapTotal:") {
            swap_total = parse_meminfo_kb(rest);
        } else if let Some(rest) = line.strip_prefix("SwapFree:") {
            swap_free = parse_meminfo_kb(rest);
        }
    }

    match (swap_total, swap_free) {
        (Some(total), Some(free)) if total > 0 => Some((total.saturating_sub(free), total)),
        _ => None,
    }
}

fn parse_meminfo_kb(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches("kB").trim();
    s.parse::<u64>().ok().map(|kb| kb * 1024)
}

/// Returns swap used percentage (0–100), or 0 if no swap.
fn swap_used_percent() -> u64 {
    match read_swap_usage() {
        Some((used, total)) if total > 0 => (used * 100) / total,
        _ => 0,
    }
}

// ---------------------------------------------------------------------------
// Cgroup memory usage
// ---------------------------------------------------------------------------

/// Read current memory usage of a cgroup (in bytes).
#[allow(dead_code)]
fn read_cgroup_memory_current(cgroup_path: &Path) -> Option<u64> {
    let path = cgroup_path.join("memory.current");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
}

/// Read the `memory.pressure` PSI file for a specific cgroup.
fn read_cgroup_memory_pressure(cgroup_path: &Path) -> Option<PsiMetrics> {
    let path = cgroup_path.join("memory.pressure");
    PsiMetrics::read(&path).ok()
}

/// Get all PIDs in a cgroup (non-recursive, from `cgroup.procs`).
fn read_cgroup_procs(cgroup_path: &Path) -> Vec<i32> {
    let path = cgroup_path.join("cgroup.procs");
    let contents = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    contents
        .lines()
        .filter_map(|line| line.trim().parse::<i32>().ok())
        .collect()
}

/// Get all PIDs in a cgroup and its children recursively.
fn read_cgroup_procs_recursive(cgroup_path: &Path) -> Vec<i32> {
    let mut pids = read_cgroup_procs(cgroup_path);

    if let Ok(entries) = fs::read_dir(cgroup_path) {
        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            if path.is_dir() {
                pids.extend(read_cgroup_procs_recursive(&path));
            }
        }
    }

    pids
}

// ---------------------------------------------------------------------------
// OOM killing logic
// ---------------------------------------------------------------------------

/// Information about a candidate process for OOM killing.
#[derive(Debug)]
struct OomCandidate {
    pid: i32,
    /// RSS in bytes (from /proc/PID/statm).
    rss_bytes: u64,
    /// OOM score adjustment from /proc/PID/oom_score_adj.
    oom_score_adj: i32,
    /// Comm (process name).
    comm: String,
}

/// Read the RSS (Resident Set Size) of a process in bytes.
fn read_process_rss(pid: i32) -> Option<u64> {
    let statm_path = format!("/proc/{pid}/statm");
    let contents = fs::read_to_string(&statm_path).ok()?;
    let fields: Vec<&str> = contents.split_whitespace().collect();
    // Field 1 is resident pages
    if fields.len() >= 2 {
        let pages: u64 = fields[1].parse().ok()?;
        let page_size = unsafe { libc::sysconf(libc::_SC_PAGESIZE) };
        if page_size > 0 {
            return Some(pages * page_size as u64);
        }
    }
    None
}

/// Read oom_score_adj for a process.
fn read_oom_score_adj(pid: i32) -> i32 {
    let path = format!("/proc/{pid}/oom_score_adj");
    fs::read_to_string(&path)
        .ok()
        .and_then(|s| s.trim().parse().ok())
        .unwrap_or(0)
}

/// Read comm (process name) for a process.
fn read_comm(pid: i32) -> String {
    let path = format!("/proc/{pid}/comm");
    fs::read_to_string(&path)
        .ok()
        .map(|s| s.trim().to_owned())
        .unwrap_or_else(|| "<unknown>".to_owned())
}

/// Select the best candidate for OOM killing from a list of PIDs.
///
/// Strategy: pick the process with the highest memory usage, adjusted by
/// oom_score_adj. Processes with oom_score_adj == -1000 are never killed.
fn select_oom_victim(pids: &[i32]) -> Option<OomCandidate> {
    let mut candidates: Vec<OomCandidate> = pids
        .iter()
        .filter_map(|&pid| {
            // Don't kill PID 1
            if pid <= 1 {
                return None;
            }

            let rss_bytes = read_process_rss(pid)?;
            let oom_score_adj = read_oom_score_adj(pid);

            // oom_score_adj == -1000 means OOM-immune
            if oom_score_adj <= -1000 {
                return None;
            }

            let comm = read_comm(pid);

            Some(OomCandidate {
                pid,
                rss_bytes,
                oom_score_adj,
                comm,
            })
        })
        .collect();

    if candidates.is_empty() {
        return None;
    }

    // Sort by effective score: higher oom_score_adj and higher RSS = more likely to kill.
    // We compute a simple score: rss_bytes * (1000 + oom_score_adj) / 1000
    candidates.sort_by(|a, b| {
        let score_a = (a.rss_bytes as f64) * (1000.0 + a.oom_score_adj as f64) / 1000.0;
        let score_b = (b.rss_bytes as f64) * (1000.0 + b.oom_score_adj as f64) / 1000.0;
        score_b
            .partial_cmp(&score_a)
            .unwrap_or(std::cmp::Ordering::Equal)
    });

    candidates.into_iter().next()
}

/// Kill a process with SIGKILL.
fn kill_process(pid: i32) -> bool {
    match nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGKILL,
    ) {
        Ok(()) => true,
        Err(e) => {
            warn!("oomd: Failed to kill PID {pid}: {e}");
            false
        }
    }
}

// ---------------------------------------------------------------------------
// Monitoring state
// ---------------------------------------------------------------------------

struct MonitorState {
    config: OomdConfig,
    policies: Vec<ManagedOOMPolicy>,
    /// Track last kill time per cgroup to implement cooldown.
    last_kill: HashMap<PathBuf, Instant>,
    /// Counters for logging.
    total_kills: u64,
}

impl MonitorState {
    fn new(config: OomdConfig, policies: Vec<ManagedOOMPolicy>) -> Self {
        Self {
            config,
            policies,
            last_kill: HashMap::new(),
            total_kills: 0,
        }
    }

    /// Run one monitoring cycle.
    fn monitor_cycle(&mut self) {
        let swap_pct = swap_used_percent();
        let swap_high = swap_pct >= self.config.swap_used_limit_percent;

        if swap_high {
            debug!(
                "oomd: Swap usage {swap_pct}% >= threshold {}%",
                self.config.swap_used_limit_percent
            );
        }

        for policy in &self.policies.clone() {
            if policy.preference == ManagedOOMPreference::Omit {
                continue;
            }

            if !policy.cgroup_path.exists() {
                trace!(
                    "oomd: Cgroup {} does not exist, skipping",
                    policy.cgroup_path.display()
                );
                continue;
            }

            // Check cooldown
            if let Some(last) = self.last_kill.get(&policy.cgroup_path)
                && last.elapsed() < Duration::from_secs(KILL_COOLDOWN_SECS)
            {
                continue;
            }

            let mut should_kill = false;
            let mut reason = String::new();

            // Check swap-based killing
            if swap_high && policy.swap_action == ManagedOOMAction::Kill {
                should_kill = true;
                reason = format!(
                    "swap usage {swap_pct}% >= {}%",
                    self.config.swap_used_limit_percent
                );
            }

            // Check memory pressure-based killing
            if policy.memory_pressure_action == ManagedOOMAction::Kill
                && let Some(psi) = read_cgroup_memory_pressure(&policy.cgroup_path)
            {
                let limit = policy
                    .memory_pressure_limit_percent
                    .unwrap_or(self.config.default_memory_pressure_limit_percent);
                let pressure = psi.effective_pressure(self.config.default_memory_pressure_duration);

                if pressure >= limit as f64 {
                    should_kill = true;
                    reason = format!(
                        "memory pressure {pressure:.1}% >= {limit}% in {}",
                        policy.cgroup_path.display()
                    );
                }
            }

            if !should_kill {
                continue;
            }

            if policy.preference == ManagedOOMPreference::Avoid {
                debug!(
                    "oomd: Cgroup {} has preference=avoid, deferring kill",
                    policy.cgroup_path.display()
                );
                continue;
            }

            // Collect candidate PIDs
            let pids = read_cgroup_procs_recursive(&policy.cgroup_path);
            if pids.is_empty() {
                trace!("oomd: No processes in {}", policy.cgroup_path.display());
                continue;
            }

            // Select and kill victim
            if let Some(victim) = select_oom_victim(&pids) {
                info!(
                    "oomd: Killing PID {} ({}) with {} bytes RSS — {reason}",
                    victim.pid, victim.comm, victim.rss_bytes
                );

                if kill_process(victim.pid) {
                    self.total_kills += 1;
                    self.last_kill
                        .insert(policy.cgroup_path.clone(), Instant::now());
                    info!(
                        "oomd: Killed PID {} ({}) successfully (total kills: {})",
                        victim.pid, victim.comm, self.total_kills
                    );
                }
            }
        }
    }

    /// Reload policies from unit files.
    fn reload_policies(&mut self) {
        let new_policies = discover_managed_cgroups();
        info!(
            "oomd: Reloaded configuration, monitoring {} cgroups",
            new_policies.len()
        );
        self.policies = new_policies;
    }
}

// ---------------------------------------------------------------------------
// sd_notify helper
// ---------------------------------------------------------------------------

fn sd_notify(msg: &str) {
    if let Ok(socket_path) = std::env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = socket_path.strip_prefix('@') {
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
// Signal handling
// ---------------------------------------------------------------------------

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sigint(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sighup(_sig: libc::c_int) {
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
    }
}

// ---------------------------------------------------------------------------
// Logging setup
// ---------------------------------------------------------------------------

fn init_logging() {
    // We log to stderr like other systemd daemons
    let level = std::env::var("SYSTEMD_LOG_LEVEL")
        .ok()
        .and_then(|s| match s.to_lowercase().as_str() {
            "emerg" | "0" => Some(log::LevelFilter::Error),
            "alert" | "1" => Some(log::LevelFilter::Error),
            "crit" | "2" => Some(log::LevelFilter::Error),
            "err" | "error" | "3" => Some(log::LevelFilter::Error),
            "warning" | "warn" | "4" => Some(log::LevelFilter::Warn),
            "notice" | "5" => Some(log::LevelFilter::Info),
            "info" | "6" => Some(log::LevelFilter::Info),
            "debug" | "7" => Some(log::LevelFilter::Debug),
            _ => None,
        })
        .unwrap_or(log::LevelFilter::Info);

    // Simple stderr logger
    struct StderrLogger;

    impl log::Log for StderrLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!(
                    "systemd-oomd[{}]: <{}> {}",
                    process::id(),
                    match record.level() {
                        log::Level::Error => 3,
                        log::Level::Warn => 4,
                        log::Level::Info => 6,
                        log::Level::Debug => 7,
                        log::Level::Trace => 7,
                    },
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

// ---------------------------------------------------------------------------
// Utility parsers
// ---------------------------------------------------------------------------

/// Parse a percentage string like "60%", "60", etc. into 0–100.
fn parse_percent(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches('%').trim();
    let val: u64 = s.parse().ok()?;
    if val > 100 { None } else { Some(val) }
}

/// Parse a duration string. Supports plain seconds, "30s", "5m", "1min", etc.
fn parse_duration_secs(s: &str) -> Option<u64> {
    let s = s.trim();

    // Try "Xmin" (must come before "Xm" to avoid false match)
    if let Some(rest) = s.strip_suffix("min") {
        return rest.trim().parse::<u64>().ok().map(|m| m * 60);
    }
    // Try "Xm"
    if let Some(rest) = s.strip_suffix('m') {
        return rest.trim().parse::<u64>().ok().map(|m| m * 60);
    }
    // Try "Xusec" and "Xus" (must come before "Xsec"/"Xs" to avoid false match)
    if let Some(rest) = s.strip_suffix("usec") {
        return rest.trim().parse::<u64>().ok().map(|us| us / 1_000_000);
    }
    if let Some(rest) = s.strip_suffix("us") {
        return rest.trim().parse::<u64>().ok().map(|us| us / 1_000_000);
    }
    // Try "Xseconds", "Xsec", "Xs"
    if let Some(rest) = s.strip_suffix("seconds") {
        return rest.trim().parse::<u64>().ok();
    }
    if let Some(rest) = s.strip_suffix("sec") {
        return rest.trim().parse::<u64>().ok();
    }
    if let Some(rest) = s.strip_suffix('s') {
        return rest.trim().parse::<u64>().ok();
    }
    // Plain number = seconds
    s.parse::<u64>().ok()
}

// ---------------------------------------------------------------------------
// System checks
// ---------------------------------------------------------------------------

/// Check that cgroupv2 is mounted and has the memory controller.
fn check_cgroup_v2_memory() -> bool {
    let cgroup_root = Path::new(CGROUP_ROOT);

    // Check it's a cgroup2 filesystem
    if !cgroup_root.join("cgroup.controllers").exists() {
        warn!("oomd: cgroup v2 not available at {CGROUP_ROOT}");
        return false;
    }

    // Check memory controller is available
    if let Ok(controllers) = fs::read_to_string(cgroup_root.join("cgroup.controllers"))
        && !controllers.split_whitespace().any(|c| c == "memory")
    {
        warn!("oomd: memory controller not available in cgroup v2");
        return false;
    }

    true
}

/// Check that PSI is available system-wide.
fn check_psi_available() -> bool {
    Path::new(SYSTEM_PRESSURE_PATH).exists()
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    init_logging();

    info!("systemd-oomd starting...");

    // Pre-flight checks (non-fatal — the unit file's ConditionPathExists and
    // ConditionControlGroupController already gate whether we are started at all,
    // so if we get here the conditions were met; log warnings and continue
    // gracefully even if something looks odd from our perspective).
    if !check_psi_available() {
        warn!(
            "oomd: PSI (Pressure Stall Information) not available — /proc/pressure/memory missing"
        );
        warn!("oomd: Memory pressure monitoring will be degraded");
    }

    if !check_cgroup_v2_memory() {
        warn!("oomd: cgroup v2 with memory controller not detected at {CGROUP_ROOT}");
        warn!("oomd: Cgroup-level pressure monitoring may be unavailable");
    }

    // Load configuration
    let config = OomdConfig::load();
    info!(
        "oomd: Config: swap_limit={}%, pressure_limit={}%, pressure_duration={}s",
        config.swap_used_limit_percent,
        config.default_memory_pressure_limit_percent,
        config.default_memory_pressure_duration.as_secs()
    );

    // Discover managed cgroups
    let policies = discover_managed_cgroups();
    info!("oomd: Discovered {} managed cgroups", policies.len());
    for policy in &policies {
        info!(
            "oomd:   {} (swap={:?}, pressure={:?}, limit={:?})",
            policy.cgroup_path.display(),
            policy.swap_action,
            policy.memory_pressure_action,
            policy.memory_pressure_limit_percent,
        );
    }

    // Set up signal handlers
    setup_signal_handlers();

    // Create monitoring state
    let mut state = MonitorState::new(config, policies);

    // Notify systemd we're ready
    sd_notify("READY=1\nSTATUS=Monitoring memory pressure...");
    info!("oomd: Ready and monitoring");

    // Watchdog interval (if configured)
    let watchdog_usec: Option<u64> = std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|s| s.parse().ok());
    let watchdog_interval = watchdog_usec.map(|us| Duration::from_micros(us / 2));
    let mut last_watchdog = Instant::now();

    // Main monitoring loop
    while !SHUTDOWN.load(Ordering::Relaxed) {
        // Check for reload request (SIGHUP)
        if RELOAD
            .compare_exchange(true, false, Ordering::AcqRel, Ordering::Relaxed)
            .is_ok()
        {
            info!("oomd: Reloading configuration...");
            state.config = OomdConfig::load();
            state.reload_policies();
            sd_notify(&format!(
                "READY=1\nRELOADING=1\nSTATUS=Monitoring {} cgroups",
                state.policies.len()
            ));
            // After reload, send ready again
            sd_notify(&format!(
                "READY=1\nSTATUS=Monitoring {} cgroups",
                state.policies.len()
            ));
        }

        // Run one monitoring cycle
        state.monitor_cycle();

        // Watchdog keepalive
        if let Some(interval) = watchdog_interval
            && last_watchdog.elapsed() >= interval
        {
            sd_notify("WATCHDOG=1");
            last_watchdog = Instant::now();
        }

        // Sleep until next cycle
        thread::sleep(Duration::from_secs(MONITOR_INTERVAL_SECS));
    }

    info!("oomd: Shutting down (total kills: {})", state.total_kills);
    sd_notify("STOPPING=1");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_percent() {
        assert_eq!(parse_percent("60%"), Some(60));
        assert_eq!(parse_percent("60"), Some(60));
        assert_eq!(parse_percent("100%"), Some(100));
        assert_eq!(parse_percent("0%"), Some(0));
        assert_eq!(parse_percent("  80%  "), Some(80));
        assert_eq!(parse_percent("101%"), None);
        assert_eq!(parse_percent("abc"), None);
    }

    #[test]
    fn test_parse_duration_secs() {
        assert_eq!(parse_duration_secs("30"), Some(30));
        assert_eq!(parse_duration_secs("30s"), Some(30));
        assert_eq!(parse_duration_secs("30sec"), Some(30));
        assert_eq!(parse_duration_secs("30seconds"), Some(30));
        assert_eq!(parse_duration_secs("5m"), Some(300));
        assert_eq!(parse_duration_secs("5min"), Some(300));
        assert_eq!(parse_duration_secs("1000000us"), Some(1));
        assert_eq!(parse_duration_secs("1000000usec"), Some(1));
        assert_eq!(parse_duration_secs("abc"), None);
    }

    #[test]
    fn test_config_default() {
        let cfg = OomdConfig::default();
        assert_eq!(cfg.swap_used_limit_percent, 90);
        assert_eq!(cfg.default_memory_pressure_limit_percent, 60);
        assert_eq!(
            cfg.default_memory_pressure_duration,
            Duration::from_secs(30)
        );
    }

    #[test]
    fn test_config_parse() {
        let mut cfg = OomdConfig::default();
        cfg.parse_config(
            r#"
[OOM]
SwapUsedLimit=80%
DefaultMemoryPressureLimit=50%
DefaultMemoryPressureDurationSec=10s
"#,
        );
        assert_eq!(cfg.swap_used_limit_percent, 80);
        assert_eq!(cfg.default_memory_pressure_limit_percent, 50);
        assert_eq!(
            cfg.default_memory_pressure_duration,
            Duration::from_secs(10)
        );
    }

    #[test]
    fn test_config_parse_ignores_other_sections() {
        let mut cfg = OomdConfig::default();
        cfg.parse_config(
            r#"
[Other]
SwapUsedLimit=10%

[OOM]
SwapUsedLimit=75%

[Another]
DefaultMemoryPressureLimit=5%
"#,
        );
        assert_eq!(cfg.swap_used_limit_percent, 75);
        // Should remain default since it was in [Another], not [OOM]
        assert_eq!(cfg.default_memory_pressure_limit_percent, 60);
    }

    #[test]
    fn test_psi_parse() {
        let metrics = PsiMetrics::parse(
            "some avg10=1.50 avg60=2.30 avg300=3.10 total=12345\n\
             full avg10=0.50 avg60=0.80 avg300=1.20 total=6789\n",
        );
        assert!((metrics.some_avg10 - 1.50).abs() < 0.01);
        assert!((metrics.some_avg60 - 2.30).abs() < 0.01);
        assert!((metrics.some_avg300 - 3.10).abs() < 0.01);
        assert!((metrics.full_avg10 - 0.50).abs() < 0.01);
        assert!((metrics.full_avg60 - 0.80).abs() < 0.01);
        assert!((metrics.full_avg300 - 1.20).abs() < 0.01);
    }

    #[test]
    fn test_psi_parse_empty() {
        let metrics = PsiMetrics::parse("");
        assert!((metrics.some_avg10 - 0.0).abs() < 0.01);
        assert!((metrics.full_avg10 - 0.0).abs() < 0.01);
    }

    #[test]
    fn test_effective_pressure_short_duration() {
        let metrics = PsiMetrics {
            some_avg10: 10.0,
            some_avg60: 5.0,
            some_avg300: 2.0,
            ..Default::default()
        };
        assert!((metrics.effective_pressure(Duration::from_secs(5)) - 10.0).abs() < 0.01);
        assert!((metrics.effective_pressure(Duration::from_secs(10)) - 10.0).abs() < 0.01);
        assert!((metrics.effective_pressure(Duration::from_secs(30)) - 5.0).abs() < 0.01);
        assert!((metrics.effective_pressure(Duration::from_secs(120)) - 2.0).abs() < 0.01);
    }

    #[test]
    fn test_parse_managed_oom_from_unit_slice() {
        let contents = r#"
[Slice]
ManagedOOMMemoryPressure=kill
ManagedOOMMemoryPressureLimit=80%
"#;
        let policy = parse_managed_oom_from_unit("system.slice", contents).unwrap();
        assert_eq!(
            policy.cgroup_path,
            PathBuf::from("/sys/fs/cgroup/system.slice")
        );
        assert_eq!(policy.memory_pressure_action, ManagedOOMAction::Kill);
        assert_eq!(policy.memory_pressure_limit_percent, Some(80));
        assert_eq!(policy.swap_action, ManagedOOMAction::Auto);
    }

    #[test]
    fn test_parse_managed_oom_from_unit_both_actions() {
        let contents = r#"
[Slice]
ManagedOOMSwap=kill
ManagedOOMMemoryPressure=kill
ManagedOOMPreference=avoid
"#;
        let policy = parse_managed_oom_from_unit("user.slice", contents).unwrap();
        assert_eq!(policy.swap_action, ManagedOOMAction::Kill);
        assert_eq!(policy.memory_pressure_action, ManagedOOMAction::Kill);
        assert_eq!(policy.preference, ManagedOOMPreference::Avoid);
    }

    #[test]
    fn test_parse_managed_oom_from_unit_no_directives() {
        let contents = r#"
[Slice]
Description=Some slice
"#;
        assert!(parse_managed_oom_from_unit("foo.slice", contents).is_none());
    }

    #[test]
    fn test_parse_managed_oom_from_unit_auto_only() {
        let contents = r#"
[Slice]
ManagedOOMSwap=auto
ManagedOOMMemoryPressure=auto
"#;
        // Both actions are Auto and no other overrides → returns None
        assert!(parse_managed_oom_from_unit("foo.slice", contents).is_none());
    }

    #[test]
    fn test_unit_name_to_cgroup_path() {
        assert_eq!(
            unit_name_to_cgroup_path("-.slice"),
            PathBuf::from("/sys/fs/cgroup/")
        );
        assert_eq!(
            unit_name_to_cgroup_path("system.slice"),
            PathBuf::from("/sys/fs/cgroup/system.slice")
        );
        assert_eq!(
            unit_name_to_cgroup_path("user.slice"),
            PathBuf::from("/sys/fs/cgroup/user.slice")
        );
    }

    #[test]
    fn test_parse_oom_action() {
        assert_eq!(parse_oom_action("kill"), ManagedOOMAction::Kill);
        assert_eq!(parse_oom_action("Kill"), ManagedOOMAction::Kill);
        assert_eq!(parse_oom_action("auto"), ManagedOOMAction::Auto);
        assert_eq!(parse_oom_action("something"), ManagedOOMAction::Auto);
    }

    #[test]
    fn test_parse_psi_line() {
        let (a10, a60, a300) = parse_psi_line("some avg10=1.23 avg60=4.56 avg300=7.89 total=100");
        assert!((a10 - 1.23).abs() < 0.001);
        assert!((a60 - 4.56).abs() < 0.001);
        assert!((a300 - 7.89).abs() < 0.001);
    }

    #[test]
    fn test_parse_psi_line_empty() {
        let (a10, a60, a300) = parse_psi_line("");
        assert!((a10 - 0.0).abs() < 0.001);
        assert!((a60 - 0.0).abs() < 0.001);
        assert!((a300 - 0.0).abs() < 0.001);
    }

    #[test]
    fn test_parse_meminfo_kb() {
        assert_eq!(parse_meminfo_kb("  1024 kB"), Some(1024 * 1024));
        assert_eq!(parse_meminfo_kb("0 kB"), Some(0));
        assert_eq!(parse_meminfo_kb("abc kB"), None);
    }

    #[test]
    fn test_monitor_state_new() {
        let config = OomdConfig::default();
        let state = MonitorState::new(config.clone(), vec![]);
        assert_eq!(state.total_kills, 0);
        assert!(state.policies.is_empty());
        assert!(state.last_kill.is_empty());
    }

    #[test]
    fn test_monitor_cycle_empty_policies() {
        let config = OomdConfig::default();
        let mut state = MonitorState::new(config, vec![]);
        // Should not panic with empty policies
        state.monitor_cycle();
        assert_eq!(state.total_kills, 0);
    }

    #[test]
    fn test_monitor_cycle_nonexistent_cgroup() {
        let config = OomdConfig::default();
        let policy = ManagedOOMPolicy {
            cgroup_path: PathBuf::from("/sys/fs/cgroup/nonexistent_test_slice_12345"),
            swap_action: ManagedOOMAction::Kill,
            memory_pressure_action: ManagedOOMAction::Kill,
            memory_pressure_limit_percent: Some(10),
            preference: ManagedOOMPreference::None,
        };
        let mut state = MonitorState::new(config, vec![policy]);
        // Should not panic — cgroup doesn't exist so it's skipped
        state.monitor_cycle();
        assert_eq!(state.total_kills, 0);
    }

    #[test]
    fn test_omit_preference_skips_cgroup() {
        let config = OomdConfig::default();
        let policy = ManagedOOMPolicy {
            cgroup_path: PathBuf::from("/sys/fs/cgroup"),
            swap_action: ManagedOOMAction::Kill,
            memory_pressure_action: ManagedOOMAction::Kill,
            memory_pressure_limit_percent: Some(0),
            preference: ManagedOOMPreference::Omit,
        };
        let mut state = MonitorState::new(config, vec![policy]);
        state.monitor_cycle();
        // Omit preference means no kills even with extreme thresholds
        assert_eq!(state.total_kills, 0);
    }

    #[test]
    fn test_parse_managed_oom_service_section() {
        let contents = r#"
[Unit]
Description=Test service

[Service]
ExecStart=/bin/true
ManagedOOMMemoryPressure=kill
ManagedOOMMemoryPressureLimit=70%
"#;
        let policy = parse_managed_oom_from_unit("test.service", contents).unwrap();
        assert_eq!(policy.memory_pressure_action, ManagedOOMAction::Kill);
        assert_eq!(policy.memory_pressure_limit_percent, Some(70));
    }

    #[test]
    fn test_swap_used_percent_runs() {
        // Just ensure it doesn't panic
        let _ = swap_used_percent();
    }

    #[test]
    fn test_select_oom_victim_empty() {
        assert!(select_oom_victim(&[]).is_none());
    }

    #[test]
    fn test_select_oom_victim_skips_pid1() {
        assert!(select_oom_victim(&[1]).is_none());
    }

    #[test]
    fn test_check_psi_available_runs() {
        // Just ensure it doesn't panic; result depends on kernel config
        let _ = check_psi_available();
    }

    #[test]
    fn test_check_cgroup_v2_memory_runs() {
        // Just ensure it doesn't panic
        let _ = check_cgroup_v2_memory();
    }

    #[test]
    fn test_config_parse_empty_file() {
        let mut cfg = OomdConfig::default();
        cfg.parse_config("");
        assert_eq!(cfg.swap_used_limit_percent, DEFAULT_SWAP_USED_LIMIT_PERCENT);
    }

    #[test]
    fn test_config_parse_comments_only() {
        let mut cfg = OomdConfig::default();
        cfg.parse_config(
            r#"
# This is a comment
; This is also a comment
# [OOM]
# SwapUsedLimit=10%
"#,
        );
        assert_eq!(cfg.swap_used_limit_percent, DEFAULT_SWAP_USED_LIMIT_PERCENT);
    }
}
