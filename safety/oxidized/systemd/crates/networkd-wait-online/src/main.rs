#![allow(dead_code)]

//! systemd-networkd-wait-online — wait for network to be configured.
//!
//! Monitors networkd state files to determine when the network is online.
//! Supports:
//! - Polling `/run/systemd/netif/state` for overall operational state
//! - Per-link state files in `/run/systemd/netif/links/`
//! - `--timeout` for maximum wait time
//! - `--interface` to wait for specific interfaces (with optional `:state`)
//! - `--ignore` to exclude interfaces from consideration
//! - `--any` to succeed when any single qualifying interface is online
//! - `--operational-state` to set the minimum required operational state
//!
//! Usage:
//!   systemd-networkd-wait-online                    # Wait for all managed links online
//!   systemd-networkd-wait-online --timeout=30       # Wait up to 30 seconds
//!   systemd-networkd-wait-online --interface=eth0   # Wait for eth0 specifically
//!   systemd-networkd-wait-online --any              # Succeed when any link is online
//!   systemd-networkd-wait-online --help             # Show help

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

// ── Constants ──────────────────────────────────────────────────────────────

const DEFAULT_TIMEOUT_SEC: u64 = 120;
const POLL_INTERVAL_MS: u64 = 500;
const STATE_DIR: &str = "/run/systemd/netif/links";
const OVERALL_STATE_FILE: &str = "/run/systemd/netif/state";
const SYSFS_NET_DIR: &str = "/sys/class/net";

// ── Operational state ordering ─────────────────────────────────────────────

/// Operational states ordered from worst to best.
/// A link is "online" if its operational state is >= the required minimum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum OperationalState {
    /// Link is missing or state file not readable.
    Missing,
    /// Link has no configuration.
    Off,
    /// No carrier detected.
    NoCarrier,
    /// Link is dormant.
    Dormant,
    /// Link is degraded-carrier (partial).
    DegradedCarrier,
    /// Link has carrier but not fully configured.
    Carrier,
    /// Link has degraded configuration.
    Degraded,
    /// Link is enslaved to a master device.
    Enslaved,
    /// Link is fully configured and routable.
    Routable,
}

impl OperationalState {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "off" => Self::Off,
            "no-carrier" => Self::NoCarrier,
            "dormant" => Self::Dormant,
            "degraded-carrier" => Self::DegradedCarrier,
            "carrier" => Self::Carrier,
            "degraded" => Self::Degraded,
            "enslaved" => Self::Enslaved,
            "routable" => Self::Routable,
            _ => Self::Missing,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Missing => "missing",
            Self::Off => "off",
            Self::NoCarrier => "no-carrier",
            Self::Dormant => "dormant",
            Self::DegradedCarrier => "degraded-carrier",
            Self::Carrier => "carrier",
            Self::Degraded => "degraded",
            Self::Enslaved => "enslaved",
            Self::Routable => "routable",
        }
    }

    /// Returns true if this state is at least as good as the required minimum.
    pub fn satisfies(&self, required: Self) -> bool {
        *self >= required
    }
}

/// The default minimum operational state for considering a link "online".
const DEFAULT_REQUIRED_STATE: OperationalState = OperationalState::Degraded;

// ── Link state reading ─────────────────────────────────────────────────────

/// Information about a single link read from state files.
#[derive(Debug, Clone)]
pub struct LinkState {
    pub ifindex: u32,
    pub ifname: Option<String>,
    pub admin_state: String,
    pub oper_state: OperationalState,
    pub network_file: Option<String>,
}

/// Read a state file and return key-value pairs.
pub fn parse_state_file(content: &str) -> Vec<(String, String)> {
    content
        .lines()
        .filter(|line| !line.starts_with('#') && !line.is_empty())
        .filter_map(|line| {
            let (key, value) = line.split_once('=')?;
            Some((key.to_string(), value.to_string()))
        })
        .collect()
}

/// Read a link state file from the given directory.
pub fn read_link_state(state_dir: &Path, ifindex: u32) -> Option<LinkState> {
    let path = state_dir.join(ifindex.to_string());
    let content = std::fs::read_to_string(&path).ok()?;
    parse_link_state(&content, ifindex)
}

/// Parse link state from file content.
pub fn parse_link_state(content: &str, ifindex: u32) -> Option<LinkState> {
    let pairs = parse_state_file(content);

    let admin_state = pairs
        .iter()
        .find(|(k, _)| k == "ADMIN_STATE")
        .map(|(_, v)| v.clone())
        .unwrap_or_default();

    let oper_state_str = pairs
        .iter()
        .find(|(k, _)| k == "OPER_STATE")
        .map(|(_, v)| v.as_str())
        .unwrap_or("");

    let network_file = pairs
        .iter()
        .find(|(k, _)| k == "NETWORK_FILE")
        .map(|(_, v)| v.clone());

    Some(LinkState {
        ifindex,
        ifname: None,
        admin_state,
        oper_state: OperationalState::parse(oper_state_str),
        network_file,
    })
}

/// Read the overall network state file.
pub fn read_overall_state(state_file: &Path) -> OperationalState {
    let content = match std::fs::read_to_string(state_file) {
        Ok(c) => c,
        Err(_) => return OperationalState::Missing,
    };
    let pairs = parse_state_file(&content);
    pairs
        .iter()
        .find(|(k, _)| k == "OPER_STATE")
        .map(|(_, v)| OperationalState::parse(v))
        .unwrap_or(OperationalState::Missing)
}

/// Enumerate all link state files in the state directory.
pub fn enumerate_links(state_dir: &Path) -> Vec<LinkState> {
    let dir = match std::fs::read_dir(state_dir) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut links = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Ok(ifindex) = name_str.parse::<u32>()
            && let Some(mut state) = read_link_state(state_dir, ifindex)
        {
            // Try to resolve interface name from sysfs.
            state.ifname = resolve_ifname_sysfs(ifindex);
            links.push(state);
        }
    }

    links.sort_by_key(|l| l.ifindex);
    links
}

/// Resolve interface name from sysfs by scanning /sys/class/net/*/ifindex.
fn resolve_ifname_sysfs(ifindex: u32) -> Option<String> {
    resolve_ifname_from(Path::new(SYSFS_NET_DIR), ifindex)
}

/// Resolve interface name from a sysfs net directory.
pub fn resolve_ifname_from(sysfs_dir: &Path, ifindex: u32) -> Option<String> {
    let dir = std::fs::read_dir(sysfs_dir).ok()?;
    for entry in dir.flatten() {
        let ifindex_path = entry.path().join("ifindex");
        if let Ok(content) = std::fs::read_to_string(&ifindex_path)
            && let Ok(idx) = content.trim().parse::<u32>()
            && idx == ifindex
        {
            return Some(entry.file_name().to_string_lossy().to_string());
        }
    }
    None
}

// ── Wait logic ─────────────────────────────────────────────────────────────

/// Configuration for the wait operation.
#[derive(Debug, Clone)]
pub struct WaitConfig {
    /// Maximum time to wait.
    pub timeout: Duration,
    /// Specific interfaces to wait for (name -> optional required state).
    pub interfaces: HashMap<String, Option<OperationalState>>,
    /// Interfaces to ignore.
    pub ignore: Vec<String>,
    /// If true, succeed when any single link is online.
    pub any: bool,
    /// Minimum operational state required.
    pub required_state: OperationalState,
    /// State directory path (for testing).
    pub state_dir: PathBuf,
    /// Overall state file (for testing).
    pub overall_state_file: PathBuf,
    /// Sysfs net directory (for testing).
    pub sysfs_dir: PathBuf,
    /// Quiet mode — suppress progress messages.
    pub quiet: bool,
}

impl Default for WaitConfig {
    fn default() -> Self {
        Self {
            timeout: Duration::from_secs(DEFAULT_TIMEOUT_SEC),
            interfaces: HashMap::new(),
            ignore: Vec::new(),
            any: false,
            required_state: DEFAULT_REQUIRED_STATE,
            state_dir: PathBuf::from(STATE_DIR),
            overall_state_file: PathBuf::from(OVERALL_STATE_FILE),
            sysfs_dir: PathBuf::from(SYSFS_NET_DIR),
            quiet: false,
        }
    }
}

/// Result of a single check iteration.
#[derive(Debug, Clone, PartialEq)]
pub enum CheckResult {
    /// Network is online — we can exit successfully.
    Online,
    /// Network is not yet online — keep waiting.
    Waiting(String),
}

/// Check whether the network is online given the current state.
pub fn check_online(config: &WaitConfig) -> CheckResult {
    // If specific interfaces are requested, check those.
    if !config.interfaces.is_empty() {
        return check_specific_interfaces(config);
    }

    // Otherwise check all managed links.
    check_all_links(config)
}

/// Check specific interfaces requested via --interface.
fn check_specific_interfaces(config: &WaitConfig) -> CheckResult {
    let links = enumerate_links_with_sysfs(&config.state_dir, &config.sysfs_dir);

    for (iface, required_override) in &config.interfaces {
        let required = required_override.unwrap_or(config.required_state);

        // Find this interface in the link list.
        let found = links
            .iter()
            .find(|l| l.ifname.as_deref() == Some(iface.as_str()));

        match found {
            Some(link) => {
                if !link.oper_state.satisfies(required) {
                    return CheckResult::Waiting(format!(
                        "interface {} is {} (need {})",
                        iface,
                        link.oper_state.as_str(),
                        required.as_str()
                    ));
                }
            }
            None => {
                return CheckResult::Waiting(format!("interface {} not found", iface));
            }
        }
    }

    CheckResult::Online
}

/// Check all managed links.
fn check_all_links(config: &WaitConfig) -> CheckResult {
    let links = enumerate_links_with_sysfs(&config.state_dir, &config.sysfs_dir);

    if links.is_empty() {
        return CheckResult::Waiting("no links found".to_string());
    }

    // Filter out ignored links and unmanaged links.
    let relevant: Vec<&LinkState> = links
        .iter()
        .filter(|l| {
            // Skip ignored interfaces.
            if let Some(ref name) = l.ifname
                && config.ignore.iter().any(|i| i == name)
            {
                return false;
            }
            // Skip links without a network file (unmanaged).
            if l.network_file.is_none() {
                return false;
            }
            true
        })
        .collect();

    if relevant.is_empty() {
        // If there are no managed links at all, check overall state.
        let overall = read_overall_state(&config.overall_state_file);
        if overall.satisfies(config.required_state) {
            return CheckResult::Online;
        }
        return CheckResult::Waiting("no managed links found".to_string());
    }

    if config.any {
        // --any mode: succeed if any link meets the requirement.
        for link in &relevant {
            if link.oper_state.satisfies(config.required_state) {
                return CheckResult::Online;
            }
        }
        CheckResult::Waiting("no link is online yet".to_string())
    } else {
        // Default mode: all managed links must be online.
        for link in &relevant {
            if !link.oper_state.satisfies(config.required_state) {
                let name = link.ifname.as_deref().unwrap_or("?");
                return CheckResult::Waiting(format!(
                    "link {} (idx {}) is {} (need {})",
                    name,
                    link.ifindex,
                    link.oper_state.as_str(),
                    config.required_state.as_str()
                ));
            }
        }
        CheckResult::Online
    }
}

/// Enumerate links and resolve names from sysfs.
fn enumerate_links_with_sysfs(state_dir: &Path, sysfs_dir: &Path) -> Vec<LinkState> {
    let dir = match std::fs::read_dir(state_dir) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };

    let mut links = Vec::new();
    for entry in dir.flatten() {
        let name = entry.file_name();
        let name_str = name.to_string_lossy();
        if let Ok(ifindex) = name_str.parse::<u32>()
            && let Some(mut state) = read_link_state(state_dir, ifindex)
        {
            state.ifname = resolve_ifname_from(sysfs_dir, ifindex);
            links.push(state);
        }
    }

    links.sort_by_key(|l| l.ifindex);
    links
}

/// Run the wait loop. Returns 0 for success, 1 for timeout/failure.
pub fn wait_online(config: &WaitConfig) -> i32 {
    let start = Instant::now();
    let poll = Duration::from_millis(POLL_INTERVAL_MS);

    loop {
        match check_online(config) {
            CheckResult::Online => {
                if !config.quiet {
                    let elapsed = start.elapsed();
                    log::info!("Network is online (took {:.1}s)", elapsed.as_secs_f64());
                }
                return 0;
            }
            CheckResult::Waiting(reason) => {
                if start.elapsed() >= config.timeout {
                    log::error!(
                        "Timeout waiting for network after {}s: {}",
                        config.timeout.as_secs(),
                        reason
                    );
                    return 1;
                }
                log::debug!("Waiting: {}", reason);
                std::thread::sleep(poll);
            }
        }
    }
}

// ── Argument parsing ───────────────────────────────────────────────────────

/// Parse command-line arguments into a WaitConfig.
pub fn parse_args(args: &[String]) -> Result<WaitConfig, String> {
    let mut config = WaitConfig::default();

    let mut i = 0;
    while i < args.len() {
        let arg = &args[i];

        if arg == "--help" || arg == "-h" {
            return Err("help".to_string());
        }

        if arg == "--version" {
            return Err("version".to_string());
        }

        if arg == "--any" {
            config.any = true;
            i += 1;
            continue;
        }

        if arg == "--quiet" || arg == "-q" {
            config.quiet = true;
            i += 1;
            continue;
        }

        // --timeout=N or --timeout N
        if let Some(val) = arg.strip_prefix("--timeout=") {
            config.timeout = Duration::from_secs(
                val.parse::<u64>()
                    .map_err(|_| format!("invalid timeout: {}", val))?,
            );
            i += 1;
            continue;
        }
        if arg == "--timeout" {
            i += 1;
            if i >= args.len() {
                return Err("--timeout requires an argument".to_string());
            }
            config.timeout = Duration::from_secs(
                args[i]
                    .parse::<u64>()
                    .map_err(|_| format!("invalid timeout: {}", args[i]))?,
            );
            i += 1;
            continue;
        }

        // --interface=IFACE[:STATE] or --interface IFACE[:STATE] (also -i)
        if let Some(val) = arg
            .strip_prefix("--interface=")
            .or_else(|| arg.strip_prefix("-i="))
        {
            parse_interface_arg(val, &mut config)?;
            i += 1;
            continue;
        }
        if arg == "--interface" || arg == "-i" {
            i += 1;
            if i >= args.len() {
                return Err("--interface requires an argument".to_string());
            }
            parse_interface_arg(&args[i], &mut config)?;
            i += 1;
            continue;
        }

        // --ignore=IFACE or --ignore IFACE
        if let Some(val) = arg.strip_prefix("--ignore=") {
            config.ignore.push(val.to_string());
            i += 1;
            continue;
        }
        if arg == "--ignore" {
            i += 1;
            if i >= args.len() {
                return Err("--ignore requires an argument".to_string());
            }
            config.ignore.push(args[i].clone());
            i += 1;
            continue;
        }

        // --operational-state=STATE or --operational-state STATE
        if let Some(val) = arg.strip_prefix("--operational-state=") {
            config.required_state = parse_required_state(val)?;
            i += 1;
            continue;
        }
        if arg == "--operational-state" {
            i += 1;
            if i >= args.len() {
                return Err("--operational-state requires an argument".to_string());
            }
            config.required_state = parse_required_state(&args[i])?;
            i += 1;
            continue;
        }

        // Ignore unknown flags (systemd passes various flags).
        i += 1;
    }

    Ok(config)
}

fn parse_interface_arg(val: &str, config: &mut WaitConfig) -> Result<(), String> {
    if let Some((name, state)) = val.split_once(':') {
        let parsed = OperationalState::parse(state);
        if parsed == OperationalState::Missing {
            return Err(format!("unknown operational state: {}", state));
        }
        config.interfaces.insert(name.to_string(), Some(parsed));
    } else {
        config.interfaces.insert(val.to_string(), None);
    }
    Ok(())
}

fn parse_required_state(val: &str) -> Result<OperationalState, String> {
    let state = OperationalState::parse(val);
    if state == OperationalState::Missing {
        Err(format!("unknown operational state: {}", val))
    } else {
        Ok(state)
    }
}

// ── Logging ────────────────────────────────────────────────────────────────

fn setup_logging() {
    // Use simple stderr logging.
    let level = match std::env::var("SYSTEMD_LOG_LEVEL") {
        Ok(val) => match val.as_str() {
            "debug" | "7" => log::LevelFilter::Debug,
            "info" | "6" => log::LevelFilter::Info,
            "notice" | "5" => log::LevelFilter::Info,
            "warning" | "4" => log::LevelFilter::Warn,
            "err" | "3" => log::LevelFilter::Error,
            _ => log::LevelFilter::Info,
        },
        Err(_) => log::LevelFilter::Info,
    };

    // Minimal logger implementation.
    struct SimpleLogger;

    impl log::Log for SimpleLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!(
                    "systemd-networkd-wait-online[{}]: {}: {}",
                    std::process::id(),
                    record.level(),
                    record.args()
                );
            }
        }

        fn flush(&self) {}
    }

    static LOGGER: SimpleLogger = SimpleLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(level);
}

// ── Help / Version ─────────────────────────────────────────────────────────

fn print_help() {
    println!(
        "\
systemd-networkd-wait-online — Wait for network to be configured

Usage: systemd-networkd-wait-online [OPTIONS]

Options:
  --timeout=SECS         Maximum time to wait (default: {})
  --interface=IFACE[:STATE]  Wait for specific interface (can be repeated)
  --ignore=IFACE         Ignore this interface (can be repeated)
  --any                  Succeed when any single link is online
  --operational-state=STATE  Required operational state (default: degraded)
  --quiet, -q            Suppress output
  --help, -h             Show this help
  --version              Show version

Operational states (ascending order):
  off, no-carrier, dormant, degraded-carrier, carrier, degraded, enslaved, routable

Exit status:
  0  Network is online
  1  Timeout or failure",
        DEFAULT_TIMEOUT_SEC
    );
}

fn print_version() {
    println!("systemd-networkd-wait-online (rust-systemd)");
}

// ── sd_notify ──────────────────────────────────────────────────────────────

fn sd_notify(msg: &str) {
    if let Ok(path) = std::env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = path.strip_prefix('@') {
            format!("\0{}", stripped)
        } else {
            path
        };
        if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), &path);
        }
    }
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();

    match parse_args(&args) {
        Ok(config) => {
            setup_logging();
            log::info!(
                "systemd-networkd-wait-online starting (timeout={}s, any={}, required={})",
                config.timeout.as_secs(),
                config.any,
                config.required_state.as_str()
            );

            // Notify systemd we're starting.
            sd_notify("STATUS=Waiting for network...");

            let exit_code = wait_online(&config);

            if exit_code == 0 {
                sd_notify("READY=1\nSTATUS=Network is online");
            } else {
                sd_notify("STATUS=Timed out waiting for network");
            }

            std::process::exit(exit_code);
        }
        Err(ref e) if e == "help" => {
            print_help();
        }
        Err(ref e) if e == "version" => {
            print_version();
        }
        Err(e) => {
            eprintln!("Error: {}", e);
            std::process::exit(2);
        }
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    // ── OperationalState tests ─────────────────────────────────────────────

    #[test]
    fn test_operational_state_parse() {
        assert_eq!(
            OperationalState::parse("routable"),
            OperationalState::Routable
        );
        assert_eq!(
            OperationalState::parse("degraded"),
            OperationalState::Degraded
        );
        assert_eq!(
            OperationalState::parse("carrier"),
            OperationalState::Carrier
        );
        assert_eq!(
            OperationalState::parse("no-carrier"),
            OperationalState::NoCarrier
        );
        assert_eq!(OperationalState::parse("off"), OperationalState::Off);
        assert_eq!(
            OperationalState::parse("enslaved"),
            OperationalState::Enslaved
        );
        assert_eq!(
            OperationalState::parse("dormant"),
            OperationalState::Dormant
        );
        assert_eq!(
            OperationalState::parse("degraded-carrier"),
            OperationalState::DegradedCarrier
        );
        assert_eq!(
            OperationalState::parse("unknown-state"),
            OperationalState::Missing
        );
        assert_eq!(OperationalState::parse(""), OperationalState::Missing);
    }

    #[test]
    fn test_operational_state_parse_case_insensitive() {
        assert_eq!(
            OperationalState::parse("ROUTABLE"),
            OperationalState::Routable
        );
        assert_eq!(
            OperationalState::parse("Degraded"),
            OperationalState::Degraded
        );
        assert_eq!(
            OperationalState::parse("NO-CARRIER"),
            OperationalState::NoCarrier
        );
    }

    #[test]
    fn test_operational_state_as_str() {
        assert_eq!(OperationalState::Routable.as_str(), "routable");
        assert_eq!(OperationalState::Missing.as_str(), "missing");
        assert_eq!(OperationalState::NoCarrier.as_str(), "no-carrier");
        assert_eq!(
            OperationalState::DegradedCarrier.as_str(),
            "degraded-carrier"
        );
    }

    #[test]
    fn test_operational_state_ordering() {
        assert!(OperationalState::Routable > OperationalState::Degraded);
        assert!(OperationalState::Degraded > OperationalState::Carrier);
        assert!(OperationalState::Carrier > OperationalState::NoCarrier);
        assert!(OperationalState::NoCarrier > OperationalState::Off);
        assert!(OperationalState::Off > OperationalState::Missing);
        assert!(OperationalState::Enslaved > OperationalState::Degraded);
    }

    #[test]
    fn test_operational_state_satisfies() {
        assert!(OperationalState::Routable.satisfies(OperationalState::Degraded));
        assert!(OperationalState::Degraded.satisfies(OperationalState::Degraded));
        assert!(!OperationalState::Carrier.satisfies(OperationalState::Degraded));
        assert!(!OperationalState::NoCarrier.satisfies(OperationalState::Degraded));
        assert!(OperationalState::Routable.satisfies(OperationalState::Routable));
    }

    // ── State file parsing tests ───────────────────────────────────────────

    #[test]
    fn test_parse_state_file_basic() {
        let content = "# comment\nADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/foo\n";
        let pairs = parse_state_file(content);
        assert_eq!(pairs.len(), 3);
        assert_eq!(pairs[0], ("ADMIN_STATE".to_string(), "Up".to_string()));
        assert_eq!(pairs[1], ("OPER_STATE".to_string(), "routable".to_string()));
        assert_eq!(
            pairs[2],
            ("NETWORK_FILE".to_string(), "/etc/foo".to_string())
        );
    }

    #[test]
    fn test_parse_state_file_empty() {
        let pairs = parse_state_file("");
        assert!(pairs.is_empty());
    }

    #[test]
    fn test_parse_state_file_comments_only() {
        let content = "# comment 1\n# comment 2\n";
        let pairs = parse_state_file(content);
        assert!(pairs.is_empty());
    }

    #[test]
    fn test_parse_state_file_no_equals() {
        let content = "NOEQUALS\nKEY=VALUE\n";
        let pairs = parse_state_file(content);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0], ("KEY".to_string(), "VALUE".to_string()));
    }

    #[test]
    fn test_parse_state_file_empty_value() {
        let content = "KEY=\n";
        let pairs = parse_state_file(content);
        assert_eq!(pairs.len(), 1);
        assert_eq!(pairs[0], ("KEY".to_string(), "".to_string()));
    }

    #[test]
    fn test_parse_state_file_value_with_equals() {
        let content = "DNS=1.1.1.1\nPATH=/a=b\n";
        let pairs = parse_state_file(content);
        assert_eq!(pairs.len(), 2);
        assert_eq!(pairs[1], ("PATH".to_string(), "/a=b".to_string()));
    }

    // ── Link state parsing tests ───────────────────────────────────────────

    #[test]
    fn test_parse_link_state_routable() {
        let content = "# systemd-networkd state file\nADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/systemd/network/10-ethernet.network\n";
        let state = parse_link_state(content, 2).unwrap();
        assert_eq!(state.ifindex, 2);
        assert_eq!(state.admin_state, "Up");
        assert_eq!(state.oper_state, OperationalState::Routable);
        assert_eq!(
            state.network_file.as_deref(),
            Some("/etc/systemd/network/10-ethernet.network")
        );
    }

    #[test]
    fn test_parse_link_state_no_carrier() {
        let content = "ADMIN_STATE=Up\nOPER_STATE=no-carrier\n";
        let state = parse_link_state(content, 5).unwrap();
        assert_eq!(state.oper_state, OperationalState::NoCarrier);
        assert!(state.network_file.is_none());
    }

    #[test]
    fn test_parse_link_state_minimal() {
        let content = "OPER_STATE=degraded\n";
        let state = parse_link_state(content, 1).unwrap();
        assert_eq!(state.oper_state, OperationalState::Degraded);
        assert_eq!(state.admin_state, "");
    }

    // ── Overall state reading tests ────────────────────────────────────────

    #[test]
    fn test_read_overall_state_routable() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state");
        fs::write(&state_file, "# overall\nOPER_STATE=routable\nDNS=1.1.1.1\n").unwrap();
        assert_eq!(read_overall_state(&state_file), OperationalState::Routable);
    }

    #[test]
    fn test_read_overall_state_degraded() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state");
        fs::write(&state_file, "OPER_STATE=degraded\n").unwrap();
        assert_eq!(read_overall_state(&state_file), OperationalState::Degraded);
    }

    #[test]
    fn test_read_overall_state_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("nonexistent");
        assert_eq!(read_overall_state(&state_file), OperationalState::Missing);
    }

    #[test]
    fn test_read_overall_state_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let state_file = dir.path().join("state");
        fs::write(&state_file, "").unwrap();
        assert_eq!(read_overall_state(&state_file), OperationalState::Missing);
    }

    // ── Enumerate links tests ──────────────────────────────────────────────

    #[test]
    fn test_enumerate_links_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        let links = enumerate_links(dir.path());
        assert!(links.is_empty());
    }

    #[test]
    fn test_enumerate_links_with_files() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("1"),
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/test.network\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("2"),
            "ADMIN_STATE=Up\nOPER_STATE=degraded\n",
        )
        .unwrap();
        let links = enumerate_links(dir.path());
        assert_eq!(links.len(), 2);
        assert_eq!(links[0].ifindex, 1);
        assert_eq!(links[0].oper_state, OperationalState::Routable);
        assert_eq!(links[1].ifindex, 2);
        assert_eq!(links[1].oper_state, OperationalState::Degraded);
    }

    #[test]
    fn test_enumerate_links_skips_non_numeric() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("1"),
            "ADMIN_STATE=Up\nOPER_STATE=routable\n",
        )
        .unwrap();
        fs::write(dir.path().join("state"), "OPER_STATE=routable\n").unwrap();
        fs::write(dir.path().join("not-a-number"), "something\n").unwrap();
        let links = enumerate_links(dir.path());
        assert_eq!(links.len(), 1);
    }

    #[test]
    fn test_enumerate_links_nonexistent_dir() {
        let links = enumerate_links(Path::new("/nonexistent/path/for/test"));
        assert!(links.is_empty());
    }

    // ── resolve_ifname_from tests ──────────────────────────────────────────

    #[test]
    fn test_resolve_ifname_from_found() {
        let dir = tempfile::tempdir().unwrap();
        let eth0_dir = dir.path().join("eth0");
        fs::create_dir(&eth0_dir).unwrap();
        fs::write(eth0_dir.join("ifindex"), "2\n").unwrap();
        assert_eq!(resolve_ifname_from(dir.path(), 2), Some("eth0".to_string()));
    }

    #[test]
    fn test_resolve_ifname_from_not_found() {
        let dir = tempfile::tempdir().unwrap();
        let eth0_dir = dir.path().join("eth0");
        fs::create_dir(&eth0_dir).unwrap();
        fs::write(eth0_dir.join("ifindex"), "2\n").unwrap();
        assert_eq!(resolve_ifname_from(dir.path(), 99), None);
    }

    #[test]
    fn test_resolve_ifname_from_empty_dir() {
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(resolve_ifname_from(dir.path(), 1), None);
    }

    #[test]
    fn test_resolve_ifname_from_nonexistent_dir() {
        assert_eq!(
            resolve_ifname_from(Path::new("/nonexistent/sysfs"), 1),
            None
        );
    }

    // ── check_online tests ─────────────────────────────────────────────────

    fn make_test_config(state_dir: &Path, overall_file: &Path, sysfs_dir: &Path) -> WaitConfig {
        WaitConfig {
            state_dir: state_dir.to_path_buf(),
            overall_state_file: overall_file.to_path_buf(),
            sysfs_dir: sysfs_dir.to_path_buf(),
            ..WaitConfig::default()
        }
    }

    fn write_link_state(state_dir: &Path, ifindex: u32, content: &str) {
        fs::write(state_dir.join(ifindex.to_string()), content).unwrap();
    }

    fn write_sysfs_entry(sysfs_dir: &Path, name: &str, ifindex: u32) {
        let iface_dir = sysfs_dir.join(name);
        fs::create_dir_all(&iface_dir).unwrap();
        fs::write(iface_dir.join("ifindex"), format!("{}\n", ifindex)).unwrap();
    }

    #[test]
    fn test_check_online_no_links() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        match check_online(&config) {
            CheckResult::Waiting(msg) => assert!(msg.contains("no"), "msg: {}", msg),
            CheckResult::Online => panic!("expected Waiting"),
        }
    }

    #[test]
    fn test_check_online_all_routable() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/test.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_check_online_degraded_satisfies_default() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=degraded\nNETWORK_FILE=/etc/test.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_check_online_carrier_does_not_satisfy_default() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=carrier\nNETWORK_FILE=/etc/test.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        match check_online(&config) {
            CheckResult::Waiting(msg) => assert!(msg.contains("carrier"), "msg: {}", msg),
            CheckResult::Online => panic!("expected Waiting"),
        }
    }

    #[test]
    fn test_check_online_any_mode_one_of_two() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/a.network\n",
        );
        write_link_state(
            state_dir.path(),
            3,
            "ADMIN_STATE=Up\nOPER_STATE=no-carrier\nNETWORK_FILE=/etc/b.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);
        write_sysfs_entry(sysfs.path(), "eth1", 3);

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        config.any = true;
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_check_online_all_mode_one_offline() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/a.network\n",
        );
        write_link_state(
            state_dir.path(),
            3,
            "ADMIN_STATE=Up\nOPER_STATE=no-carrier\nNETWORK_FILE=/etc/b.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);
        write_sysfs_entry(sysfs.path(), "eth1", 3);

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        match check_online(&config) {
            CheckResult::Waiting(msg) => {
                assert!(
                    msg.contains("no-carrier") || msg.contains("eth1"),
                    "msg: {}",
                    msg
                )
            }
            CheckResult::Online => panic!("expected Waiting"),
        }
    }

    #[test]
    fn test_check_online_ignore_offline_link() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/a.network\n",
        );
        write_link_state(
            state_dir.path(),
            3,
            "ADMIN_STATE=Up\nOPER_STATE=no-carrier\nNETWORK_FILE=/etc/b.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);
        write_sysfs_entry(sysfs.path(), "eth1", 3);

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        config.ignore.push("eth1".to_string());
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_check_online_unmanaged_links_skipped() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");
        fs::write(&overall_file, "OPER_STATE=routable\n").unwrap();

        // Link without NETWORK_FILE is unmanaged.
        write_link_state(
            state_dir.path(),
            1,
            "ADMIN_STATE=Unmanaged\nOPER_STATE=off\n",
        );
        write_sysfs_entry(sysfs.path(), "lo", 1);

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        // Should fall through to overall state check since no managed links.
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_check_online_custom_required_state() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=degraded\nNETWORK_FILE=/etc/a.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        config.required_state = OperationalState::Routable;
        match check_online(&config) {
            CheckResult::Waiting(msg) => assert!(msg.contains("degraded"), "msg: {}", msg),
            CheckResult::Online => panic!("expected Waiting"),
        }
    }

    // ── Specific interface tests ───────────────────────────────────────────

    #[test]
    fn test_check_online_specific_interface_found_and_online() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/a.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        config.interfaces.insert("eth0".to_string(), None);
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_check_online_specific_interface_not_found() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        config.interfaces.insert("eth99".to_string(), None);
        match check_online(&config) {
            CheckResult::Waiting(msg) => assert!(msg.contains("not found"), "msg: {}", msg),
            CheckResult::Online => panic!("expected Waiting"),
        }
    }

    #[test]
    fn test_check_online_specific_interface_with_state_override() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=degraded\nNETWORK_FILE=/etc/a.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        // Require routable for eth0 specifically.
        config
            .interfaces
            .insert("eth0".to_string(), Some(OperationalState::Routable));
        match check_online(&config) {
            CheckResult::Waiting(msg) => assert!(msg.contains("degraded"), "msg: {}", msg),
            CheckResult::Online => panic!("expected Waiting"),
        }
    }

    #[test]
    fn test_check_online_specific_interface_carrier_ok_when_required() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=carrier\nNETWORK_FILE=/etc/a.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        config
            .interfaces
            .insert("eth0".to_string(), Some(OperationalState::Carrier));
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    // ── Argument parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_args_empty() {
        let config = parse_args(&[]).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(DEFAULT_TIMEOUT_SEC));
        assert!(!config.any);
        assert!(config.interfaces.is_empty());
        assert!(config.ignore.is_empty());
        assert_eq!(config.required_state, OperationalState::Degraded);
    }

    #[test]
    fn test_parse_args_timeout_equals() {
        let args: Vec<String> = vec!["--timeout=30".into()];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(30));
    }

    #[test]
    fn test_parse_args_timeout_space() {
        let args: Vec<String> = vec!["--timeout".into(), "60".into()];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(60));
    }

    #[test]
    fn test_parse_args_timeout_invalid() {
        let args: Vec<String> = vec!["--timeout=abc".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_timeout_missing_value() {
        let args: Vec<String> = vec!["--timeout".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_any() {
        let args: Vec<String> = vec!["--any".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.any);
    }

    #[test]
    fn test_parse_args_quiet() {
        let args: Vec<String> = vec!["--quiet".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.quiet);
    }

    #[test]
    fn test_parse_args_quiet_short() {
        let args: Vec<String> = vec!["-q".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.quiet);
    }

    #[test]
    fn test_parse_args_interface_plain() {
        let args: Vec<String> = vec!["--interface=eth0".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.interfaces.contains_key("eth0"));
        assert_eq!(config.interfaces["eth0"], None);
    }

    #[test]
    fn test_parse_args_interface_with_state() {
        let args: Vec<String> = vec!["--interface=eth0:routable".into()];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.interfaces["eth0"], Some(OperationalState::Routable));
    }

    #[test]
    fn test_parse_args_interface_space() {
        let args: Vec<String> = vec!["--interface".into(), "wlan0".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.interfaces.contains_key("wlan0"));
    }

    #[test]
    fn test_parse_args_interface_short() {
        let args: Vec<String> = vec!["-i=eth0".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.interfaces.contains_key("eth0"));
    }

    #[test]
    fn test_parse_args_interface_short_space() {
        let args: Vec<String> = vec!["-i".into(), "eth0".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.interfaces.contains_key("eth0"));
    }

    #[test]
    fn test_parse_args_interface_invalid_state() {
        let args: Vec<String> = vec!["--interface=eth0:badstate".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_interface_missing_value() {
        let args: Vec<String> = vec!["--interface".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_ignore_equals() {
        let args: Vec<String> = vec!["--ignore=lo".into()];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.ignore, vec!["lo"]);
    }

    #[test]
    fn test_parse_args_ignore_space() {
        let args: Vec<String> = vec!["--ignore".into(), "veth123".into()];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.ignore, vec!["veth123"]);
    }

    #[test]
    fn test_parse_args_ignore_missing_value() {
        let args: Vec<String> = vec!["--ignore".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_operational_state_equals() {
        let args: Vec<String> = vec!["--operational-state=routable".into()];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.required_state, OperationalState::Routable);
    }

    #[test]
    fn test_parse_args_operational_state_space() {
        let args: Vec<String> = vec!["--operational-state".into(), "carrier".into()];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.required_state, OperationalState::Carrier);
    }

    #[test]
    fn test_parse_args_operational_state_invalid() {
        let args: Vec<String> = vec!["--operational-state=bogus".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_operational_state_missing_value() {
        let args: Vec<String> = vec!["--operational-state".into()];
        assert!(parse_args(&args).is_err());
    }

    #[test]
    fn test_parse_args_help() {
        let args: Vec<String> = vec!["--help".into()];
        let err = parse_args(&args).unwrap_err();
        assert_eq!(err, "help");
    }

    #[test]
    fn test_parse_args_version() {
        let args: Vec<String> = vec!["--version".into()];
        let err = parse_args(&args).unwrap_err();
        assert_eq!(err, "version");
    }

    #[test]
    fn test_parse_args_unknown_flag_ignored() {
        let args: Vec<String> = vec!["--unknown-flag".into(), "--any".into()];
        let config = parse_args(&args).unwrap();
        assert!(config.any);
    }

    #[test]
    fn test_parse_args_combined() {
        let args: Vec<String> = vec![
            "--timeout=10".into(),
            "--any".into(),
            "--ignore=lo".into(),
            "--ignore=docker0".into(),
            "--interface=eth0".into(),
            "--operational-state=carrier".into(),
            "-q".into(),
        ];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.timeout, Duration::from_secs(10));
        assert!(config.any);
        assert_eq!(config.ignore, vec!["lo", "docker0"]);
        assert!(config.interfaces.contains_key("eth0"));
        assert_eq!(config.required_state, OperationalState::Carrier);
        assert!(config.quiet);
    }

    #[test]
    fn test_parse_args_multiple_interfaces() {
        let args: Vec<String> = vec![
            "--interface=eth0:routable".into(),
            "--interface=wlan0:degraded".into(),
        ];
        let config = parse_args(&args).unwrap();
        assert_eq!(config.interfaces.len(), 2);
        assert_eq!(config.interfaces["eth0"], Some(OperationalState::Routable));
        assert_eq!(config.interfaces["wlan0"], Some(OperationalState::Degraded));
    }

    // ── wait_online quick tests ────────────────────────────────────────────

    #[test]
    fn test_wait_online_immediately_online() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/test.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let config = WaitConfig {
            timeout: Duration::from_secs(1),
            state_dir: state_dir.path().to_path_buf(),
            overall_state_file: overall_file,
            sysfs_dir: sysfs.path().to_path_buf(),
            quiet: true,
            ..WaitConfig::default()
        };

        assert_eq!(wait_online(&config), 0);
    }

    #[test]
    fn test_wait_online_timeout() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        // No links at all — will time out.
        let config = WaitConfig {
            timeout: Duration::from_millis(100),
            state_dir: state_dir.path().to_path_buf(),
            overall_state_file: overall_file,
            sysfs_dir: sysfs.path().to_path_buf(),
            quiet: true,
            ..WaitConfig::default()
        };

        assert_eq!(wait_online(&config), 1);
    }

    // ── Default config tests ───────────────────────────────────────────────

    #[test]
    fn test_default_wait_config() {
        let config = WaitConfig::default();
        assert_eq!(config.timeout, Duration::from_secs(DEFAULT_TIMEOUT_SEC));
        assert!(!config.any);
        assert!(config.interfaces.is_empty());
        assert!(config.ignore.is_empty());
        assert_eq!(config.required_state, OperationalState::Degraded);
        assert!(!config.quiet);
    }

    // ── LinkState tests ────────────────────────────────────────────────────

    #[test]
    fn test_link_state_fields() {
        let state = LinkState {
            ifindex: 5,
            ifname: Some("wlan0".to_string()),
            admin_state: "Up".to_string(),
            oper_state: OperationalState::Routable,
            network_file: Some("/etc/systemd/network/10-wireless.network".to_string()),
        };
        assert_eq!(state.ifindex, 5);
        assert_eq!(state.ifname.as_deref(), Some("wlan0"));
        assert_eq!(state.admin_state, "Up");
        assert_eq!(state.oper_state, OperationalState::Routable);
    }

    // ── Edge cases ─────────────────────────────────────────────────────────

    #[test]
    fn test_check_online_any_mode_none_online() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=no-carrier\nNETWORK_FILE=/etc/a.network\n",
        );
        write_link_state(
            state_dir.path(),
            3,
            "ADMIN_STATE=Up\nOPER_STATE=carrier\nNETWORK_FILE=/etc/b.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);
        write_sysfs_entry(sysfs.path(), "eth1", 3);

        let mut config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        config.any = true;
        match check_online(&config) {
            CheckResult::Waiting(msg) => assert!(msg.contains("no link"), "msg: {}", msg),
            CheckResult::Online => panic!("expected Waiting"),
        }
    }

    #[test]
    fn test_check_online_mixed_managed_unmanaged() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        // Unmanaged link (no NETWORK_FILE).
        write_link_state(
            state_dir.path(),
            1,
            "ADMIN_STATE=Unmanaged\nOPER_STATE=off\n",
        );
        // Managed link online.
        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=routable\nNETWORK_FILE=/etc/a.network\n",
        );
        write_sysfs_entry(sysfs.path(), "lo", 1);
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_check_online_enslaved_satisfies_degraded() {
        let state_dir = tempfile::tempdir().unwrap();
        let overall = tempfile::tempdir().unwrap();
        let sysfs = tempfile::tempdir().unwrap();
        let overall_file = overall.path().join("state");

        write_link_state(
            state_dir.path(),
            2,
            "ADMIN_STATE=Up\nOPER_STATE=enslaved\nNETWORK_FILE=/etc/a.network\n",
        );
        write_sysfs_entry(sysfs.path(), "eth0", 2);

        let config = make_test_config(state_dir.path(), &overall_file, sysfs.path());
        assert_eq!(check_online(&config), CheckResult::Online);
    }

    #[test]
    fn test_operational_state_dormant() {
        assert_eq!(
            OperationalState::parse("dormant"),
            OperationalState::Dormant
        );
        assert!(!OperationalState::Dormant.satisfies(OperationalState::Degraded));
        assert!(OperationalState::Dormant.satisfies(OperationalState::Dormant));
    }

    #[test]
    fn test_parse_state_file_dns_entries() {
        let content = "OPER_STATE=routable\nDNS=1.1.1.1\nDNS=8.8.8.8\nDOMAINS=example.com\n";
        let pairs = parse_state_file(content);
        assert_eq!(pairs.len(), 4);
        let dns_entries: Vec<_> = pairs
            .iter()
            .filter(|(k, _)| k == "DNS")
            .map(|(_, v)| v.as_str())
            .collect();
        assert_eq!(dns_entries, vec!["1.1.1.1", "8.8.8.8"]);
    }

    #[test]
    fn test_parse_interface_arg_plain() {
        let mut config = WaitConfig::default();
        parse_interface_arg("eth0", &mut config).unwrap();
        assert!(config.interfaces.contains_key("eth0"));
        assert_eq!(config.interfaces["eth0"], None);
    }

    #[test]
    fn test_parse_interface_arg_with_state() {
        let mut config = WaitConfig::default();
        parse_interface_arg("eth0:routable", &mut config).unwrap();
        assert_eq!(config.interfaces["eth0"], Some(OperationalState::Routable));
    }

    #[test]
    fn test_parse_interface_arg_invalid_state() {
        let mut config = WaitConfig::default();
        let result = parse_interface_arg("eth0:bogus", &mut config);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_required_state_valid() {
        assert_eq!(
            parse_required_state("routable").unwrap(),
            OperationalState::Routable
        );
        assert_eq!(
            parse_required_state("degraded").unwrap(),
            OperationalState::Degraded
        );
    }

    #[test]
    fn test_parse_required_state_invalid() {
        assert!(parse_required_state("invalid").is_err());
    }
}
