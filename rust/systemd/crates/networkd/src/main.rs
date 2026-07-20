#![allow(dead_code)]

//! systemd-networkd — network link manager daemon.
//!
//! Manages network interfaces based on `.network` configuration files.
//! Supports:
//! - Static IPv4 address and route configuration
//! - DHCPv4 client with full DORA state machine
//! - DNS resolver configuration (`/run/systemd/resolve/resolv.conf`)
//! - D-Bus interface (`org.freedesktop.network1`) with deferred registration
//! - sd_notify protocol (READY/WATCHDOG/STATUS/STOPPING)
//! - Signal handling (SIGTERM/SIGINT for shutdown, SIGHUP for reload)
//! - Runtime state files in `/run/systemd/netif/`
//!
//! Usage:
//!   systemd-networkd              # Run as daemon
//!   systemd-networkd --help       # Show help

mod config;
mod dhcp;
mod dhcpv6;
mod ipv6_ra;
mod link;
mod manager;
mod netdev;
mod netdev_create;
mod varlink_metrics;

use std::collections::HashMap;
use std::fs;
use std::io;
use std::net::Ipv4Addr;
use std::os::unix::net::UnixDatagram;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

use manager::NetworkManager;

// ── D-Bus constants ────────────────────────────────────────────────────────

const DBUS_NAME: &str = "org.freedesktop.network1";
const DBUS_PATH: &str = "/org/freedesktop/network1";

/// PID file path for networkd discovery by networkctl.
const PID_FILE: &str = "/run/systemd/netif/systemd-networkd.pid";

/// Send an sd_notify message to the service manager.
fn sd_notify(msg: &str) {
    if let Ok(path) = std::env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = path.strip_prefix('@') {
            // Abstract socket — replace leading '@' with '\0'.
            format!("\0{}", stripped)
        } else {
            path
        };
        if let Ok(sock) = UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), &path);
        }
    }
}

/// Parse WATCHDOG_USEC from the environment and return the keepalive interval
/// (half the watchdog period).
fn watchdog_interval() -> Option<Duration> {
    let usec: u64 = std::env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    if usec == 0 {
        return None;
    }
    // Kick at half the watchdog interval.
    Some(Duration::from_micros(usec / 2))
}

/// Open a raw socket bound to a specific interface for sending/receiving
/// DHCP packets. Uses AF_INET + SOCK_DGRAM + SO_BINDTODEVICE.
///
/// Returns the socket fd on success.
fn open_dhcp_socket(ifname: &str) -> io::Result<i32> {
    let fd = unsafe {
        libc::socket(
            libc::AF_INET,
            libc::SOCK_DGRAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK,
            libc::IPPROTO_UDP,
        )
    };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    // SO_BROADCAST — needed for DHCP broadcast.
    let one: libc::c_int = 1;
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BROADCAST,
            &one as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }

    // SO_REUSEADDR.
    unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &one as *const libc::c_int as *const libc::c_void,
            std::mem::size_of::<libc::c_int>() as libc::socklen_t,
        );
    }

    // SO_BINDTODEVICE — bind to the specific interface.
    let ifname_c = std::ffi::CString::new(ifname)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidInput, "invalid interface name"))?;
    let ret = unsafe {
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            ifname_c.as_ptr() as *const libc::c_void,
            ifname_c.as_bytes_with_nul().len() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    // Bind to 0.0.0.0:68 (DHCP client port).
    let mut addr: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    addr.sin_family = libc::AF_INET as libc::sa_family_t;
    addr.sin_port = 68u16.to_be();
    addr.sin_addr.s_addr = libc::INADDR_ANY;

    let ret = unsafe {
        libc::bind(
            fd,
            &addr as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if ret < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    Ok(fd)
}

/// Send a DHCP packet via broadcast on the given socket.
fn send_dhcp_broadcast(fd: i32, pkt: &dhcp::DhcpPacket) -> io::Result<()> {
    let data = pkt.serialize();

    let mut dst: libc::sockaddr_in = unsafe { std::mem::zeroed() };
    dst.sin_family = libc::AF_INET as libc::sa_family_t;
    dst.sin_port = 67u16.to_be(); // DHCP server port
    dst.sin_addr.s_addr = u32::from(Ipv4Addr::BROADCAST).to_be();

    let sent = unsafe {
        libc::sendto(
            fd,
            data.as_ptr() as *const libc::c_void,
            data.len(),
            0,
            &dst as *const libc::sockaddr_in as *const libc::sockaddr,
            std::mem::size_of::<libc::sockaddr_in>() as libc::socklen_t,
        )
    };
    if sent < 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// Receive a DHCP packet from the given socket (non-blocking).
fn recv_dhcp_packet(fd: i32) -> Option<dhcp::DhcpPacket> {
    let mut buf = [0u8; 4096];
    let n = unsafe {
        libc::recv(
            fd,
            buf.as_mut_ptr() as *mut libc::c_void,
            buf.len(),
            libc::MSG_DONTWAIT,
        )
    };
    if n <= 0 {
        return None;
    }
    let n = n as usize;
    dhcp::DhcpPacket::parse(&buf[..n]).ok()
}

fn print_help() {
    eprintln!("systemd-networkd — network link manager daemon");
    eprintln!();
    eprintln!("Usage: systemd-networkd [OPTIONS]");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --help, -h    Show this help message");
    eprintln!("  --version     Show version information");
}

fn print_version() {
    eprintln!("systemd-networkd (rust-systemd) 0.1.0");
}

fn setup_logging() {
    let level = std::env::var("SYSTEMD_LOG_LEVEL")
        .ok()
        .and_then(|l| match l.to_lowercase().as_str() {
            "debug" | "7" => Some(log::LevelFilter::Debug),
            "info" | "6" => Some(log::LevelFilter::Info),
            "notice" | "5" | "warning" | "4" => Some(log::LevelFilter::Warn),
            "err" | "3" | "crit" | "2" | "alert" | "1" | "emerg" | "0" => {
                Some(log::LevelFilter::Error)
            }
            _ => None,
        })
        .unwrap_or(log::LevelFilter::Info);

    fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}][systemd-networkd][{}] {}",
                chrono::Local::now().format("%H:%M:%S"),
                record.level(),
                message
            ))
        })
        .level(level)
        .chain(std::io::stderr())
        .apply()
        .ok();
}

// ── Shared D-Bus state ─────────────────────────────────────────────────────

/// Summary of a single network link for D-Bus exposure.
#[derive(Debug, Clone, Default)]
struct LinkSummary {
    ifindex: u32,
    name: String,
    link_type: String,
    oper_state: String,
    admin_state: String,
    address: String,
    gateway: String,
}

/// Shared signal flags that allow the D-Bus interface to trigger actions
/// in the main event loop (e.g. Reload, ForceRenew).
#[derive(Debug)]
struct SignalFlags {
    reload: AtomicBool,
    /// ifindex of link to force-renew; 0 means no pending renew, -1 means all.
    force_renew_ifindex: std::sync::atomic::AtomicI32,
}

impl SignalFlags {
    fn new() -> Self {
        Self {
            reload: AtomicBool::new(false),
            force_renew_ifindex: std::sync::atomic::AtomicI32::new(0),
        }
    }
}

/// Snapshot of networkd state exposed via D-Bus properties.
#[derive(Debug, Clone)]
struct NetworkdState {
    /// Overall operational state (initializing, configuring, configured, degraded, no-carrier).
    oper_state: String,
    /// Per-link summaries.
    links: Vec<LinkSummary>,
    /// Global DNS servers.
    dns_servers: Vec<String>,
    /// Global search domains.
    search_domains: Vec<String>,
}

impl Default for NetworkdState {
    fn default() -> Self {
        Self {
            oper_state: "initializing".to_string(),
            links: Vec::new(),
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
        }
    }
}

type SharedState = Arc<Mutex<NetworkdState>>;

// ── D-Bus interface: org.freedesktop.network1.Manager ──────────────────────

/// D-Bus interface struct for org.freedesktop.network1.Manager.
///
/// Properties (read-only):
///   OperationalState (s)  — overall network operational state
///   CarrierState (s)      — whether any link has carrier
///   AddressState (s)      — whether any link has an address
///   OnlineState (s)       — online/partial/offline
///   NamespaceId (t)       — network namespace ID (always 0 for host)
///
/// Methods:
///   ListLinks() → a(iso)    — array of (ifindex, name, object_path)
///   GetLinkByName(s) → (io) — ifindex + object path for a link name
///   GetLinkByIndex(i) → (so)— name + object path for an ifindex
///   Describe() → s          — JSON description of the manager state
///   Reload()                — trigger configuration reload
///   ForceRenew(i ifindex)   — force DHCP renew on a link (stub)
struct Network1Manager {
    state: SharedState,
    signals: Arc<SignalFlags>,
}

#[zbus::interface(name = "org.freedesktop.network1.Manager")]
impl Network1Manager {
    // --- Properties (read-only) ---

    #[zbus(property, name = "OperationalState")]
    fn operational_state(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.oper_state.clone()
    }

    #[zbus(property, name = "CarrierState")]
    fn carrier_state(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let has_carrier = s
            .links
            .iter()
            .any(|l| l.oper_state != "no-carrier" && l.link_type != "loopback");
        if has_carrier {
            "carrier".to_string()
        } else {
            "no-carrier".to_string()
        }
    }

    #[zbus(property, name = "AddressState")]
    fn address_state(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let has_addr = s.links.iter().any(|l| !l.address.is_empty());
        if has_addr {
            "routable".to_string()
        } else {
            "off".to_string()
        }
    }

    #[zbus(property, name = "OnlineState")]
    fn online_state(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let online = match s.oper_state.as_str() {
            "configured" => "online",
            "degraded" => "partial",
            _ => "offline",
        };
        online.to_string()
    }

    #[zbus(property, name = "NamespaceId")]
    fn namespace_id(&self) -> u64 {
        0u64
    }

    // --- Methods ---

    /// ListLinks() → a(iso) — array of (ifindex, name, object_path)
    fn list_links(&self) -> Vec<(i32, String, String)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.links
            .iter()
            .map(|l| {
                (
                    l.ifindex as i32,
                    l.name.clone(),
                    link_object_path(l.ifindex),
                )
            })
            .collect()
    }

    /// GetLinkByName(s name) → (io)
    fn get_link_by_name(&self, name: String) -> zbus::fdo::Result<(i32, String)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = s.links.iter().find(|l| l.name == name) {
            Ok((l.ifindex as i32, link_object_path(l.ifindex)))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No link '{}' known",
                name
            )))
        }
    }

    /// GetLinkByIndex(i ifindex) → (so)
    fn get_link_by_index(&self, ifindex: i32) -> zbus::fdo::Result<(String, String)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(l) = s.links.iter().find(|l| l.ifindex == ifindex as u32) {
            Ok((l.name.clone(), link_object_path(l.ifindex)))
        } else {
            Err(zbus::fdo::Error::Failed(format!(
                "No link with index {}",
                ifindex
            )))
        }
    }

    /// Describe() → s (JSON description)
    fn describe(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let mut links_json = String::from("[");
        for (i, l) in s.links.iter().enumerate() {
            if i > 0 {
                links_json.push(',');
            }
            links_json.push_str(&format!(
                concat!(
                    "{{",
                    "\"Index\":{},",
                    "\"Name\":\"{}\",",
                    "\"Type\":\"{}\",",
                    "\"OperationalState\":\"{}\",",
                    "\"AdministrativeState\":\"{}\",",
                    "\"Address\":\"{}\",",
                    "\"Gateway\":\"{}\"",
                    "}}"
                ),
                l.ifindex,
                json_escape(&l.name),
                json_escape(&l.link_type),
                json_escape(&l.oper_state),
                json_escape(&l.admin_state),
                json_escape(&l.address),
                json_escape(&l.gateway),
            ));
        }
        links_json.push(']');

        let dns_json: Vec<String> = s
            .dns_servers
            .iter()
            .map(|d| format!("\"{}\"", json_escape(d)))
            .collect();

        let domains_json: Vec<String> = s
            .search_domains
            .iter()
            .map(|d| format!("\"{}\"", json_escape(d)))
            .collect();

        format!(
            concat!(
                "{{",
                "\"OperationalState\":\"{}\",",
                "\"NLinks\":{},",
                "\"Links\":{},",
                "\"DNS\":[{}],",
                "\"SearchDomains\":[{}]",
                "}}"
            ),
            json_escape(&s.oper_state),
            s.links.len(),
            links_json,
            dns_json.join(","),
            domains_json.join(","),
        )
    }

    /// Reload() — trigger configuration reload via shared signal flag
    fn reload(&self) {
        log::info!("D-Bus Reload() called");
        self.signals.reload.store(true, Ordering::Relaxed);
    }

    /// ForceRenew(i ifindex) — trigger DHCP renewal on a link
    fn force_renew(&self, ifindex: i32) {
        log::info!("D-Bus ForceRenew() called for ifindex={}", ifindex);
        self.signals
            .force_renew_ifindex
            .store(ifindex, Ordering::Relaxed);
    }
}

/// Convert a link ifindex to a D-Bus object path.
fn link_object_path(ifindex: u32) -> String {
    format!("/org/freedesktop/network1/link/_{}", ifindex)
}

/// Set up the D-Bus connection and register the network1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedState, signals: Arc<SignalFlags>) -> Result<Connection, String> {
    let iface = Network1Manager {
        state: shared,
        signals,
    };
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

/// Update the shared D-Bus state from the current NetworkManager state.
fn update_shared_state(shared: &SharedState, mgr: &NetworkManager) {
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());
    s.oper_state = mgr.overall_state().to_string();
    s.dns_servers = mgr.dns_servers.iter().map(|d| d.to_string()).collect();
    s.search_domains = mgr.search_domains.clone();

    let mut links = Vec::new();
    let summary = mgr.status_summary();
    for ls in &summary {
        links.push(LinkSummary {
            ifindex: ls.index,
            name: ls.name.clone(),
            link_type: ls.link_type.clone(),
            oper_state: format!("{}", ls.oper_state),
            admin_state: format!("{}", ls.admin_state),
            address: ls.address.clone().unwrap_or_default(),
            gateway: ls.gateway.clone().unwrap_or_default(),
        });
    }
    s.links = links;
}

/// Write the PID file so that networkctl can find us.
fn write_pid_file() {
    let pid = std::process::id();
    // Ensure the parent directory exists.
    let _ = fs::create_dir_all("/run/systemd/netif");
    if let Err(e) = fs::write(PID_FILE, format!("{}\n", pid)) {
        log::warn!("Failed to write PID file {}: {}", PID_FILE, e);
    }
}

/// Remove the PID file on shutdown.
fn remove_pid_file() {
    let _ = fs::remove_file(PID_FILE);
}

fn main() {
    // Parse arguments.
    let args: Vec<String> = std::env::args().collect();
    for arg in &args[1..] {
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                return;
            }
            "--version" => {
                print_version();
                return;
            }
            _ => {
                // Ignore unknown flags (systemd passes various flags).
            }
        }
    }

    setup_logging();
    log::info!("systemd-networkd starting");

    // Serve io.systemd.Network metrics at /run/systemd/report/io.systemd.Network
    // so systemd-report can enumerate per-interface network metrics.
    varlink_metrics::spawn_metrics_server();

    // Set up signal handling.
    let shutdown = Arc::new(AtomicBool::new(false));
    let reload = Arc::new(AtomicBool::new(false));

    // Register signal handlers.
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&shutdown)).ok();
    signal_hook::flag::register(signal_hook::consts::SIGINT, Arc::clone(&shutdown)).ok();
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&reload)).ok();

    // Shared D-Bus state and signal flags.
    let shared_state: SharedState = Arc::new(Mutex::new(NetworkdState::default()));
    let signal_flags = Arc::new(SignalFlags::new());

    // Create the network manager.
    let mut mgr = NetworkManager::new();

    // Load .network and .netdev configuration files.
    mgr.load_configs();

    if mgr.configs.is_empty() {
        log::info!("No .network configuration files found");
    }

    // Create virtual network devices from .netdev configs.
    // This must happen before discover_links() so the newly created
    // interfaces are visible during enumeration.
    mgr.create_netdevs();

    // Discover and match network interfaces.
    if let Err(e) = mgr.discover_links() {
        log::error!("Failed to discover links: {}", e);
        sd_notify("STATUS=Failed to discover links");
        std::process::exit(1);
    }

    // Configure links (bring up, apply static config, start DHCP).
    if let Err(e) = mgr.configure_links() {
        log::error!("Failed to configure links: {}", e);
    }

    // Open DHCP sockets for links that need DHCP.
    let mut dhcp_sockets: HashMap<u32, i32> = HashMap::new();
    for ifindex in mgr.dhcp_active_links() {
        if let Some(managed) = mgr.links.get(&ifindex) {
            match open_dhcp_socket(&managed.link.name) {
                Ok(fd) => {
                    log::debug!(
                        "Opened DHCP socket fd={} for {} (idx={})",
                        fd,
                        managed.link.name,
                        ifindex
                    );
                    dhcp_sockets.insert(ifindex, fd);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to open DHCP socket for {}: {}",
                        managed.link.name,
                        e
                    );
                }
            }
        }
    }

    // Send initial DHCP discovers.
    for (&ifindex, &fd) in &dhcp_sockets {
        if let Some(managed) = mgr.links.get_mut(&ifindex)
            && let Some(ref mut client) = managed.dhcp_client
            && let Some(pkt) = client.next_packet()
        {
            log::info!(
                "{}: sending DHCPDISCOVER (xid={:#010x})",
                client.config.ifname,
                client.xid
            );
            if let Err(e) = send_dhcp_broadcast(fd, &pkt) {
                log::warn!("{}: failed to send DISCOVER: {}", client.config.ifname, e);
            }
        }
    }

    // Open ICMPv6 sockets for links that need IPv6 RA.
    let mut ra_sockets: HashMap<u32, i32> = HashMap::new();
    for ifindex in mgr.ra_active_links() {
        if let Some(managed) = mgr.links.get(&ifindex) {
            match ipv6_ra::open_icmpv6_socket(&managed.link.name) {
                Ok(fd) => {
                    log::info!(
                        "Opened ICMPv6 RA socket fd={} for {} (idx={})",
                        fd,
                        managed.link.name,
                        ifindex
                    );
                    ra_sockets.insert(ifindex, fd);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to open ICMPv6 socket for {}: {}",
                        managed.link.name,
                        e
                    );
                }
            }
        }
    }

    // Send initial Router Solicitations on RA-enabled links.
    for (&ifindex, &fd) in &ra_sockets {
        if let Some(managed) = mgr.links.get_mut(&ifindex)
            && let Some(ref mut ra_state) = managed.ra_state
            && ra_state.should_send_rs()
        {
            log::info!("{}: sending Router Solicitation", ra_state.ifname);
            if let Err(e) = ipv6_ra::send_rs(fd, &ra_state.mac) {
                log::debug!("{}: failed to send RS (will retry): {}", ra_state.ifname, e);
            }
            ra_state.mark_rs_sent();
        }
    }

    // Open DHCPv6 sockets for links that need DHCPv6.
    let mut dhcpv6_sockets: HashMap<u32, i32> = HashMap::new();
    for ifindex in mgr.dhcpv6_active_links() {
        if let Some(managed) = mgr.links.get(&ifindex) {
            match dhcpv6::open_dhcpv6_socket(&managed.link.name) {
                Ok(fd) => {
                    log::info!(
                        "Opened DHCPv6 socket fd={} for {} (idx={})",
                        fd,
                        managed.link.name,
                        ifindex
                    );
                    dhcpv6_sockets.insert(ifindex, fd);
                }
                Err(e) => {
                    log::warn!(
                        "Failed to open DHCPv6 socket for {}: {}",
                        managed.link.name,
                        e
                    );
                }
            }
        }
    }

    // Send initial DHCPv6 Solicits / Information-Requests.
    for (&ifindex, &fd) in &dhcpv6_sockets {
        if let Some(managed) = mgr.links.get_mut(&ifindex)
            && let Some(ref mut client) = managed.dhcpv6_client
            && let Some(pkt) = client.next_packet()
        {
            log::info!(
                "{}: sending DHCPv6 {} (tid={:02x}{:02x}{:02x})",
                client.config.ifname,
                if client.config.request_addresses {
                    "SOLICIT"
                } else {
                    "INFORMATION-REQUEST"
                },
                client.transaction_id[0],
                client.transaction_id[1],
                client.transaction_id[2]
            );
            if let Err(e) = dhcpv6::send_dhcpv6(fd, ifindex, &pkt) {
                log::warn!("{}: failed to send DHCPv6: {}", client.config.ifname, e);
            }
        }
    }

    // Write initial state files.
    mgr.write_state_files();

    // Update shared D-Bus state with initial link info.
    update_shared_state(&shared_state, &mgr);

    // Write PID file for networkctl discovery.
    write_pid_file();

    // Notify systemd we're ready.
    sd_notify("READY=1\nSTATUS=Configured network interfaces");
    log::info!(
        "systemd-networkd ready (managing {} link(s))",
        mgr.links.len()
    );

    // Get watchdog interval.
    let watchdog = watchdog_interval();
    let mut last_watchdog = Instant::now();

    // D-Bus connection is deferred to after READY=1 so we don't block
    // early boot waiting for dbus-daemon.  zbus dispatches messages
    // automatically in a background thread — we just keep the connection alive.
    let mut _dbus_conn: Option<Connection> = None;
    let mut dbus_attempted = false;

    // Main event loop.
    let poll_interval = Duration::from_millis(500);

    while !shutdown.load(Ordering::Relaxed) {
        // Attempt D-Bus registration once (deferred from startup).
        if !dbus_attempted {
            dbus_attempted = true;
            match setup_dbus(shared_state.clone(), Arc::clone(&signal_flags)) {
                Ok(conn) => {
                    log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                    _dbus_conn = Some(conn);
                    sd_notify(&format!("STATUS={} (D-Bus active)", mgr.overall_state()));
                }
                Err(e) => {
                    log::info!(
                        "Failed to register D-Bus interface ({}); continuing without D-Bus",
                        e
                    );
                }
            }
        }

        // Handle reload signal (from SIGHUP or D-Bus Reload()).
        if reload.swap(false, Ordering::Relaxed)
            || signal_flags.reload.swap(false, Ordering::Relaxed)
        {
            log::info!("Reloading configuration (SIGHUP)");
            mgr.load_configs();
            mgr.create_netdevs();
            if let Err(e) = mgr.discover_links() {
                log::warn!("Failed to rediscover links on reload: {}", e);
            }
            if let Err(e) = mgr.configure_links() {
                log::warn!("Failed to reconfigure links on reload: {}", e);
            }
            // Persist per-link state files for any links added or changed by the
            // reload (e.g. a runtime .netdev added via `networkctl edit`), so
            // systemd-networkd-wait-online can see the new interface instead of
            // reporting it as missing until the next state-file write.
            mgr.write_state_files();
            update_shared_state(&shared_state, &mgr);
            sd_notify(&format!("STATUS={}", mgr.overall_state()));
        }

        // Process DHCP: receive replies, handle timeouts, send retransmits.
        for (&ifindex, &fd) in &dhcp_sockets {
            // Try to receive a DHCP reply.
            while let Some(reply) = recv_dhcp_packet(fd) {
                if let Some(managed) = mgr.links.get_mut(&ifindex)
                    && let Some(ref mut client) = managed.dhcp_client
                {
                    if let Some(lease) = client.process_reply(&reply) {
                        // Lease obtained or renewed — apply it.
                        let _ = client; // Release borrow.
                        if let Err(e) = mgr.apply_lease(ifindex, &lease) {
                            log::warn!("Failed to apply DHCP lease on idx={}: {}", ifindex, e);
                        }
                        mgr.write_state_files();
                        update_shared_state(&shared_state, &mgr);
                        sd_notify(&format!("STATUS={}", mgr.overall_state()));
                        break;
                    }

                    // If the client moved to REQUESTING after an OFFER,
                    // immediately send the REQUEST.
                    if client.state == dhcp::DhcpState::Requesting
                        && let Some(request_pkt) = client.next_packet()
                    {
                        log::info!(
                            "{}: sending DHCPREQUEST for {}",
                            client.config.ifname,
                            client
                                .last_offer
                                .as_ref()
                                .map(|o| o.yiaddr.to_string())
                                .unwrap_or_default()
                        );
                        if let Err(e) = send_dhcp_broadcast(fd, &request_pkt) {
                            log::warn!("{}: failed to send REQUEST: {}", client.config.ifname, e);
                        }
                    }
                }
            }

            // Check for retransmission timeouts.
            if let Some(managed) = mgr.links.get_mut(&ifindex)
                && let Some(ref mut client) = managed.dhcp_client
            {
                let should_retransmit = match client.last_send {
                    Some(last) => last.elapsed() >= client.retransmit_timeout(),
                    None => true,
                };

                if should_retransmit
                    && !client.max_attempts_reached()
                    && client.state != dhcp::DhcpState::Bound
                    && let Some(pkt) = client.next_packet()
                {
                    let msg_type = pkt
                        .message_type()
                        .map(dhcp::dhcp_message_type_name)
                        .unwrap_or("UNKNOWN");
                    log::debug!(
                        "{}: retransmitting DHCP{} (attempt {})",
                        client.config.ifname,
                        msg_type,
                        client.attempts
                    );
                    if let Err(e) = send_dhcp_broadcast(fd, &pkt) {
                        log::warn!("{}: failed to retransmit: {}", client.config.ifname, e);
                    }
                }

                // Check for lease renewal / rebinding.
                if client.state == dhcp::DhcpState::Bound
                    && let Some(ref lease) = client.lease
                {
                    if lease.is_expired() {
                        log::warn!("{}: DHCP lease expired", client.config.ifname);
                        let ifname = client.config.ifname.clone();
                        client.state = dhcp::DhcpState::Init;
                        client.lease = None;
                        let _ = client;
                        if let Err(e) = mgr.remove_lease(ifindex) {
                            log::warn!("{}: failed to remove expired lease: {}", ifname, e);
                        }
                        mgr.write_state_files();
                        update_shared_state(&shared_state, &mgr);
                    } else if lease.needs_renewal() {
                        // Transition to renewing and send a request.
                        if let Some(pkt) = client.next_packet() {
                            log::info!("{}: DHCP lease renewal (T1 reached)", client.config.ifname);
                            let _ = send_dhcp_broadcast(fd, &pkt);
                        }
                    }
                }
            }
        }

        // Process IPv6 RA: receive Router Advertisements, send RS retransmissions.
        for (&ifindex, &fd) in &ra_sockets {
            // Try to receive Router Advertisements.
            while let Some((data, source)) = ipv6_ra::recv_ra(fd) {
                if let Some(ra) = ipv6_ra::parse_ra(&data, source) {
                    log::info!("idx={}: received {}", ifindex, ra);

                    // Check M/O flags to trigger DHCPv6.
                    let managed_flag = ra.managed;
                    let other_flag = ra.other;

                    // Process the RA through the link's RaState.
                    let actions = if let Some(managed) = mgr.links.get_mut(&ifindex) {
                        if let Some(ref mut ra_state) = managed.ra_state {
                            ra_state.process_ra(ra)
                        } else {
                            Vec::new()
                        }
                    } else {
                        Vec::new()
                    };

                    // Apply the RA actions (add addresses, routes, DNS, etc.).
                    if !actions.is_empty() && mgr.apply_ra_actions(ifindex, actions) {
                        mgr.write_state_files();
                        update_shared_state(&shared_state, &mgr);
                        sd_notify(&format!("STATUS={}", mgr.overall_state()));
                    }

                    // Start DHCPv6 client if RA M/O flags indicate it.
                    if (managed_flag || other_flag)
                        && mgr.start_dhcpv6_from_ra(ifindex, managed_flag, other_flag)
                    {
                        // Open a DHCPv6 socket for the newly started client.
                        if let std::collections::hash_map::Entry::Vacant(entry) =
                            dhcpv6_sockets.entry(ifindex)
                            && let Some(managed) = mgr.links.get(&ifindex)
                        {
                            match dhcpv6::open_dhcpv6_socket(&managed.link.name) {
                                Ok(new_fd) => {
                                    log::info!(
                                        "Opened DHCPv6 socket fd={} for {} (RA-triggered)",
                                        new_fd,
                                        managed.link.name
                                    );
                                    entry.insert(new_fd);
                                }
                                Err(e) => {
                                    log::warn!(
                                        "Failed to open DHCPv6 socket for {}: {}",
                                        managed.link.name,
                                        e
                                    );
                                }
                            }
                        }
                        // Send the initial DHCPv6 packet.
                        if let Some(&new_fd) = dhcpv6_sockets.get(&ifindex)
                            && let Some(managed) = mgr.links.get_mut(&ifindex)
                            && let Some(ref mut client) = managed.dhcpv6_client
                            && let Some(pkt) = client.next_packet()
                        {
                            log::info!(
                                "{}: sending DHCPv6 {} (RA-triggered)",
                                client.config.ifname,
                                if client.config.request_addresses {
                                    "SOLICIT"
                                } else {
                                    "INFORMATION-REQUEST"
                                }
                            );
                            if let Err(e) = dhcpv6::send_dhcpv6(new_fd, ifindex, &pkt) {
                                log::warn!(
                                    "{}: failed to send DHCPv6: {}",
                                    client.config.ifname,
                                    e
                                );
                            }
                        }
                    }
                }
            }

            // Check if we need to retransmit RS.
            if let Some(managed) = mgr.links.get_mut(&ifindex)
                && let Some(ref mut ra_state) = managed.ra_state
                && ra_state.should_send_rs()
            {
                log::debug!(
                    "{}: retransmitting RS (attempt {})",
                    ra_state.ifname,
                    ra_state.rs_count + 1
                );
                if let Err(e) = ipv6_ra::send_rs(fd, &ra_state.mac) {
                    log::debug!("{}: failed to retransmit RS: {}", ra_state.ifname, e);
                }
                ra_state.mark_rs_sent();
            }
        }

        // Process DHCPv6: receive replies, handle timeouts, send retransmits.
        for (&ifindex, &fd) in &dhcpv6_sockets {
            // Try to receive a DHCPv6 reply.
            while let Some(reply_data) = dhcpv6::recv_dhcpv6(fd) {
                if let Some(managed) = mgr.links.get_mut(&ifindex)
                    && let Some(ref mut client) = managed.dhcpv6_client
                {
                    if let Some(lease) = client.process_reply(&reply_data) {
                        // Lease obtained or renewed — apply it.
                        let _ = client; // Release borrow.
                        if let Err(e) = mgr.apply_dhcpv6_lease(ifindex, &lease) {
                            log::warn!("Failed to apply DHCPv6 lease on idx={}: {}", ifindex, e);
                        }
                        mgr.write_state_files();
                        update_shared_state(&shared_state, &mgr);
                        sd_notify(&format!("STATUS={}", mgr.overall_state()));
                        break;
                    }

                    // If the client moved to Requesting after an Advertise,
                    // immediately send the Request.
                    if client.state == dhcpv6::Dhcpv6State::Requesting
                        && let Some(request_pkt) = client.next_packet()
                    {
                        log::info!("{}: sending DHCPv6 REQUEST", client.config.ifname);
                        if let Err(e) = dhcpv6::send_dhcpv6(fd, ifindex, &request_pkt) {
                            log::warn!(
                                "{}: failed to send DHCPv6 REQUEST: {}",
                                client.config.ifname,
                                e
                            );
                        }
                    }
                }
            }

            // Check for retransmission timeouts.
            if let Some(managed) = mgr.links.get_mut(&ifindex)
                && let Some(ref mut client) = managed.dhcpv6_client
            {
                let should_retransmit = match client.last_send {
                    Some(last) => last.elapsed() >= client.retransmit_timeout(),
                    None => true,
                };

                if should_retransmit
                    && !client.max_attempts_reached()
                    && !matches!(
                        client.state,
                        dhcpv6::Dhcpv6State::Bound | dhcpv6::Dhcpv6State::InformationReceived
                    )
                    && let Some(pkt) = client.next_packet()
                {
                    log::debug!(
                        "{}: retransmitting DHCPv6 (attempt {}, state={})",
                        client.config.ifname,
                        client.attempts,
                        client.state
                    );
                    if let Err(e) = dhcpv6::send_dhcpv6(fd, ifindex, &pkt) {
                        log::warn!(
                            "{}: failed to retransmit DHCPv6: {}",
                            client.config.ifname,
                            e
                        );
                    }
                }

                // Check for lease renewal / rebinding.
                if client.state == dhcpv6::Dhcpv6State::Bound
                    && let Some(ref lease) = client.lease
                {
                    if lease.is_expired() {
                        log::warn!("{}: DHCPv6 lease expired", client.config.ifname);
                        let ifname = client.config.ifname.clone();
                        client.state = dhcpv6::Dhcpv6State::Init;
                        client.lease = None;
                        let _ = client;
                        if let Err(e) = mgr.remove_dhcpv6_lease(ifindex) {
                            log::warn!("{}: failed to remove expired DHCPv6 lease: {}", ifname, e);
                        }
                        mgr.write_state_files();
                        update_shared_state(&shared_state, &mgr);
                    } else if lease.needs_renewal() {
                        // Transition to renewing and send a Renew.
                        if let Some(pkt) = client.next_packet() {
                            log::info!(
                                "{}: DHCPv6 lease renewal (T1 reached)",
                                client.config.ifname
                            );
                            let _ = dhcpv6::send_dhcpv6(fd, ifindex, &pkt);
                        }
                    }
                }
            }
        }

        // zbus dispatches D-Bus messages automatically in a background thread.

        // Watchdog keepalive.
        if let Some(interval) = watchdog
            && last_watchdog.elapsed() >= interval
        {
            sd_notify("WATCHDOG=1");
            last_watchdog = Instant::now();
        }

        // Handle D-Bus ForceRenew requests.
        let renew_ifindex = signal_flags.force_renew_ifindex.swap(0, Ordering::Relaxed);
        if renew_ifindex != 0 {
            let targets: Vec<u32> = if renew_ifindex < 0 {
                // Renew all DHCP links.
                dhcp_sockets.keys().copied().collect()
            } else {
                vec![renew_ifindex as u32]
            };
            for ifidx in targets {
                if let Some(&fd) = dhcp_sockets.get(&ifidx)
                    && let Some(managed) = mgr.links.get_mut(&ifidx)
                    && let Some(ref mut client) = managed.dhcp_client
                {
                    log::info!(
                        "{}: forcing DHCP renew (D-Bus ForceRenew)",
                        client.config.ifname
                    );
                    // Reset to Init so the client will re-discover.
                    client.state = dhcp::DhcpState::Init;
                    client.attempts = 0;
                    client.last_send = None;
                    if let Some(pkt) = client.next_packet()
                        && let Err(e) = send_dhcp_broadcast(fd, &pkt)
                    {
                        log::warn!(
                            "{}: failed to send DISCOVER for force-renew: {}",
                            client.config.ifname,
                            e
                        );
                    }
                }
            }
        }

        // Check SLAAC address lifetimes (deprecation and expiration).
        if mgr.check_ra_address_lifetimes() {
            mgr.write_state_files();
            update_shared_state(&shared_state, &mgr);
        }

        // Refresh link carrier state from the kernel. A link brought up on a
        // recent reload gains IFF_RUNNING (carrier) a moment later via the
        // kernel linkwatch queue, so re-read here and rewrite the state files
        // when carrier changes. systemd-networkd-wait-online reads those files.
        if mgr.refresh_carrier_states() {
            mgr.write_state_files();
            update_shared_state(&shared_state, &mgr);
        }

        // Sleep until next poll.
        std::thread::sleep(poll_interval);
    }

    // Shutdown.
    log::info!("systemd-networkd shutting down");
    sd_notify("STOPPING=1\nSTATUS=Shutting down");
    remove_pid_file();

    // Release all DHCP leases.
    for (&ifindex, &fd) in &dhcp_sockets {
        if let Some(managed) = mgr.links.get(&ifindex)
            && let Some(ref client) = managed.dhcp_client
            && let Some(release_pkt) = client.build_release()
        {
            log::info!("{}: sending DHCPRELEASE", managed.link.name);
            let _ = send_dhcp_broadcast(fd, &release_pkt);
        }
    }

    // Release all DHCPv6 leases.
    for (&ifindex, &fd) in &dhcpv6_sockets {
        if let Some(managed) = mgr.links.get(&ifindex)
            && let Some(ref client) = managed.dhcpv6_client
            && let Some(release_pkt) = client.build_release()
        {
            log::info!("{}: sending DHCPv6 RELEASE", managed.link.name);
            let _ = dhcpv6::send_dhcpv6(fd, ifindex, &release_pkt);
        }
    }

    // Close DHCP sockets.
    for (_, fd) in dhcp_sockets {
        unsafe { libc::close(fd) };
    }

    // Close DHCPv6 sockets.
    for (_, fd) in dhcpv6_sockets {
        unsafe { libc::close(fd) };
    }

    // Close RA sockets.
    for (_, fd) in ra_sockets {
        unsafe { libc::close(fd) };
    }

    // Write final state.
    mgr.write_state_files();

    log::info!("systemd-networkd stopped");
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sd_notify_no_socket() {
        // Should not panic when NOTIFY_SOCKET is not set.
        unsafe { std::env::remove_var("NOTIFY_SOCKET") };
        sd_notify("READY=1");
    }

    #[test]
    fn test_watchdog_interval_not_set() {
        unsafe { std::env::remove_var("WATCHDOG_USEC") };
        assert!(watchdog_interval().is_none());
    }

    #[test]
    fn test_watchdog_interval_zero() {
        unsafe { std::env::set_var("WATCHDOG_USEC", "0") };
        assert!(watchdog_interval().is_none());
        unsafe { std::env::remove_var("WATCHDOG_USEC") };
    }

    #[test]
    fn test_watchdog_interval_valid() {
        unsafe { std::env::set_var("WATCHDOG_USEC", "10000000") }; // 10s
        let interval = watchdog_interval();
        assert!(interval.is_some());
        // Half of 10s = 5s.
        assert_eq!(interval.unwrap(), Duration::from_secs(5));
        unsafe { std::env::remove_var("WATCHDOG_USEC") };
    }

    #[test]
    fn test_watchdog_interval_invalid() {
        unsafe { std::env::set_var("WATCHDOG_USEC", "not_a_number") };
        assert!(watchdog_interval().is_none());
        unsafe { std::env::remove_var("WATCHDOG_USEC") };
    }

    // ── D-Bus interface tests ──────────────────────────────────────────

    #[test]
    fn test_dbus_network1_manager_struct() {
        let shared: SharedState = Arc::new(Mutex::new(NetworkdState::default()));
        let signals = Arc::new(SignalFlags::new());
        let _mgr = Network1Manager {
            state: shared,
            signals,
        };
        // Struct creation succeeded without panic
    }

    #[test]
    fn test_link_object_path() {
        assert_eq!(link_object_path(1), "/org/freedesktop/network1/link/_1");
        assert_eq!(link_object_path(42), "/org/freedesktop/network1/link/_42");
        assert_eq!(link_object_path(0), "/org/freedesktop/network1/link/_0");
    }

    #[test]
    fn test_shared_state_default() {
        let state = NetworkdState::default();
        assert_eq!(state.oper_state, "initializing");
        assert!(state.links.is_empty());
        assert!(state.dns_servers.is_empty());
        assert!(state.search_domains.is_empty());
    }

    #[test]
    fn test_shared_state_with_links() {
        let shared: SharedState = Arc::new(Mutex::new(NetworkdState {
            oper_state: "configured".to_string(),
            links: vec![
                LinkSummary {
                    ifindex: 1,
                    name: "lo".to_string(),
                    link_type: "loopback".to_string(),
                    oper_state: "carrier".to_string(),
                    admin_state: "Up".to_string(),
                    address: "127.0.0.1/8".to_string(),
                    gateway: String::new(),
                },
                LinkSummary {
                    ifindex: 2,
                    name: "eth0".to_string(),
                    link_type: "ether".to_string(),
                    oper_state: "configured".to_string(),
                    admin_state: "Up".to_string(),
                    address: "192.168.1.100/24".to_string(),
                    gateway: "192.168.1.1".to_string(),
                },
            ],
            dns_servers: vec!["8.8.8.8".to_string()],
            search_domains: vec!["example.com".to_string()],
        }));
        let s = shared.lock().unwrap();
        assert_eq!(s.oper_state, "configured");
        assert_eq!(s.links.len(), 2);
        assert_eq!(s.links[0].name, "lo");
        assert_eq!(s.links[1].name, "eth0");
        assert_eq!(s.dns_servers, vec!["8.8.8.8"]);
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
    fn test_carrier_state_derived() {
        // With an ether link that has carrier
        let state = NetworkdState {
            oper_state: "configured".to_string(),
            links: vec![LinkSummary {
                ifindex: 2,
                name: "eth0".to_string(),
                link_type: "ether".to_string(),
                oper_state: "configured".to_string(),
                admin_state: "Up".to_string(),
                address: "10.0.0.1/24".to_string(),
                gateway: "10.0.0.1".to_string(),
            }],
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
        };
        let has_carrier = state
            .links
            .iter()
            .any(|l| l.oper_state != "no-carrier" && l.link_type != "loopback");
        assert!(has_carrier);

        // With only loopback
        let state2 = NetworkdState {
            oper_state: "no-carrier".to_string(),
            links: vec![LinkSummary {
                ifindex: 1,
                name: "lo".to_string(),
                link_type: "loopback".to_string(),
                oper_state: "carrier".to_string(),
                admin_state: "Up".to_string(),
                address: "127.0.0.1/8".to_string(),
                gateway: String::new(),
            }],
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
        };
        let has_carrier2 = state2
            .links
            .iter()
            .any(|l| l.oper_state != "no-carrier" && l.link_type != "loopback");
        assert!(!has_carrier2);
    }

    #[test]
    fn test_online_state_derived() {
        assert_eq!(
            match "configured" {
                "configured" => "online",
                "degraded" => "partial",
                _ => "offline",
            },
            "online"
        );
        assert_eq!(
            match "degraded" {
                "configured" => "online",
                "degraded" => "partial",
                _ => "offline",
            },
            "partial"
        );
        assert_eq!(
            match "no-carrier" {
                "configured" => "online",
                "degraded" => "partial",
                _ => "offline",
            },
            "offline"
        );
    }

    #[test]
    fn test_link_summary_fields() {
        let link = LinkSummary {
            ifindex: 3,
            name: "wlan0".to_string(),
            link_type: "wlan".to_string(),
            oper_state: "degraded".to_string(),
            admin_state: "Up".to_string(),
            address: "192.168.0.50/24".to_string(),
            gateway: "192.168.0.1".to_string(),
        };
        assert_eq!(link.ifindex, 3);
        assert_eq!(link.name, "wlan0");
        assert_eq!(link.link_type, "wlan");
        assert_eq!(link.oper_state, "degraded");
        assert_eq!(link.address, "192.168.0.50/24");
        assert_eq!(link.gateway, "192.168.0.1");
    }

    // ── SignalFlags tests ──────────────────────────────────────────────

    #[test]
    fn test_signal_flags_new() {
        let flags = SignalFlags::new();
        assert!(!flags.reload.load(Ordering::Relaxed));
        assert_eq!(flags.force_renew_ifindex.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_signal_flags_reload_roundtrip() {
        let flags = SignalFlags::new();
        assert!(!flags.reload.load(Ordering::Relaxed));

        flags.reload.store(true, Ordering::Relaxed);
        assert!(flags.reload.load(Ordering::Relaxed));

        // swap should return old value and set new
        let was = flags.reload.swap(false, Ordering::Relaxed);
        assert!(was);
        assert!(!flags.reload.load(Ordering::Relaxed));
    }

    #[test]
    fn test_signal_flags_force_renew_roundtrip() {
        let flags = SignalFlags::new();
        assert_eq!(flags.force_renew_ifindex.load(Ordering::Relaxed), 0);

        flags.force_renew_ifindex.store(42, Ordering::Relaxed);
        assert_eq!(flags.force_renew_ifindex.load(Ordering::Relaxed), 42);

        // swap should return old value and set 0
        let was = flags.force_renew_ifindex.swap(0, Ordering::Relaxed);
        assert_eq!(was, 42);
        assert_eq!(flags.force_renew_ifindex.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_signal_flags_force_renew_negative_means_all() {
        let flags = SignalFlags::new();
        flags.force_renew_ifindex.store(-1, Ordering::Relaxed);
        let val = flags.force_renew_ifindex.swap(0, Ordering::Relaxed);
        assert!(val < 0, "negative ifindex means renew all links");
    }

    #[test]
    fn test_signal_flags_shared_across_threads() {
        let flags = Arc::new(SignalFlags::new());
        let flags2 = Arc::clone(&flags);

        let handle = std::thread::spawn(move || {
            flags2.reload.store(true, Ordering::Relaxed);
            flags2.force_renew_ifindex.store(7, Ordering::Relaxed);
        });
        handle.join().unwrap();

        assert!(flags.reload.load(Ordering::Relaxed));
        assert_eq!(flags.force_renew_ifindex.load(Ordering::Relaxed), 7);
    }

    // ── PID file tests ─────────────────────────────────────────────────

    #[test]
    fn test_pid_file_constant() {
        assert_eq!(PID_FILE, "/run/systemd/netif/systemd-networkd.pid");
    }

    // ── D-Bus reload / force_renew signal integration ──────────────────

    #[test]
    fn test_dbus_reload_sets_signal_flag() {
        let shared: SharedState = Arc::new(Mutex::new(NetworkdState::default()));
        let signals = Arc::new(SignalFlags::new());
        let mgr = Network1Manager {
            state: shared,
            signals: Arc::clone(&signals),
        };

        assert!(!signals.reload.load(Ordering::Relaxed));
        // Simulate what the D-Bus Reload() method does:
        signals.reload.store(true, Ordering::Relaxed);
        assert!(signals.reload.load(Ordering::Relaxed));

        // Main loop would swap it back to false:
        let was = signals.reload.swap(false, Ordering::Relaxed);
        assert!(was);
        assert!(!signals.reload.load(Ordering::Relaxed));
        let _ = mgr; // keep alive
    }

    #[test]
    fn test_dbus_force_renew_sets_ifindex() {
        let shared: SharedState = Arc::new(Mutex::new(NetworkdState::default()));
        let signals = Arc::new(SignalFlags::new());
        let mgr = Network1Manager {
            state: shared,
            signals: Arc::clone(&signals),
        };

        assert_eq!(signals.force_renew_ifindex.load(Ordering::Relaxed), 0);
        // Simulate what ForceRenew(ifindex=5) does:
        signals.force_renew_ifindex.store(5, Ordering::Relaxed);
        assert_eq!(signals.force_renew_ifindex.load(Ordering::Relaxed), 5);

        // Main loop would swap it back to 0:
        let ifidx = signals.force_renew_ifindex.swap(0, Ordering::Relaxed);
        assert_eq!(ifidx, 5);
        assert_eq!(signals.force_renew_ifindex.load(Ordering::Relaxed), 0);
        let _ = mgr; // keep alive
    }

    #[test]
    fn test_dbus_network1_manager_with_signals() {
        let shared: SharedState = Arc::new(Mutex::new(NetworkdState::default()));
        let signals = Arc::new(SignalFlags::new());
        let _mgr = Network1Manager {
            state: shared,
            signals,
        };
        // Struct creation succeeded without panic
    }
}
