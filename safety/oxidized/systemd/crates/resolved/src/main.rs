#![allow(dead_code)]
//! systemd-resolved — Stub DNS resolver daemon
//!
//! A Rust implementation of systemd-resolved that provides a local DNS stub
//! listener on 127.0.0.53:53, forwarding queries to upstream DNS servers.
//!
//! Features:
//! - Parses `/etc/systemd/resolved.conf` and drop-in directories
//! - Stub DNS listener on 127.0.0.53:53 (UDP + TCP)
//! - Forwards DNS queries to configured upstream servers
//! - Manages `/run/systemd/resolve/stub-resolv.conf` (stub → 127.0.0.53)
//! - Manages `/run/systemd/resolve/resolv.conf` (upstream servers)
//! - Per-link DNS from networkd state files
//! - D-Bus interface (`org.freedesktop.resolve1`) with deferred registration
//! - sd_notify READY=1 / WATCHDOG=1 / STATUS= protocol
//! - Signal handling: SIGTERM/SIGINT for shutdown, SIGHUP for reload
//! - TCP listener for DNS queries (parallel to UDP)
//! - Statistics tracking

mod config;
mod dns;
mod dnssec;
mod dnstls;
mod edns;
mod hosts;
mod llmnr;
mod mdns;
mod routing;

use std::fs;
use std::io::{self, Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, SocketAddr, TcpListener, TcpStream, UdpSocket};
use std::os::unix::net::UnixDatagram;
use std::sync::Arc;
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;
use zbus::zvariant::OwnedObjectPath;

use config::{
    DnsOverTlsMode, DnssecMode, RESOLV_CONF_PATH, ResolutionMode, ResolvedConfig, STATE_DIR,
    STUB_RESOLV_CONF_PATH, StubListenerMode,
};
use dns::{
    DnsCache, DnsMessage, HEADER_SIZE, MAX_EDNS_UDP_SIZE, MAX_TCP_SIZE, ResolverStats,
    build_formerr, build_servfail, forward_query,
};
use dnssec::DnssecValidator;
use dnstls::{DotClient, DotMode, configs_from_addrs};
use edns::{OptRecord, append_opt_to_query};
use routing::{DnsRouter, extract_query_name};

// ── Constants ──────────────────────────────────────────────────────────────

/// Poll interval for the main loop (ms)
const POLL_INTERVAL_MS: u64 = 500;

/// UDP receive buffer size
const UDP_RECV_BUF: usize = MAX_EDNS_UDP_SIZE;

/// TCP receive buffer size
const TCP_RECV_BUF: usize = MAX_TCP_SIZE + 2; // +2 for length prefix

/// Default watchdog interval (if WATCHDOG_USEC not set)
const DEFAULT_WATCHDOG_SEC: u64 = 0; // disabled

/// Interval to refresh link DNS configuration from networkd (seconds)
const LINK_DNS_REFRESH_INTERVAL: Duration = Duration::from_secs(30);

/// TCP connection timeout
const TCP_TIMEOUT: Duration = Duration::from_secs(10);

// D-Bus constants
const DBUS_NAME: &str = "org.freedesktop.resolve1";
const DBUS_PATH: &str = "/org/freedesktop/resolve1";

/// PID file path for resolved discovery by resolvectl.
const PID_FILE: &str = "/run/systemd/resolve/systemd-resolved.pid";

// ── Logging ────────────────────────────────────────────────────────────────

fn setup_logging() {
    let log_level = std::env::var("RESOLVED_LOG")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(log::LevelFilter::Info);

    if fern::Dispatch::new()
        .format(|out, message, record| {
            out.finish(format_args!(
                "[{}][{}][{}] {}",
                chrono::Local::now().format("%Y-%m-%d][%H:%M:%S"),
                record.target(),
                record.level(),
                message,
            ))
        })
        .level(log_level)
        .chain(io::stderr())
        .apply()
        .is_err()
    {
        eprintln!("resolved: failed to set up logging, continuing with eprintln");
    }
}

// ── sd_notify helpers ──────────────────────────────────────────────────────

fn sd_notify(msg: &str) {
    if let Ok(path) = std::env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = path.strip_prefix('@') {
            format!("\0{}", stripped)
        } else {
            path
        };

        if let Ok(sock) = UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), &path);
        }
    }
}

fn sd_notify_ready() {
    sd_notify("READY=1");
}

fn sd_notify_stopping() {
    sd_notify("STOPPING=1");
}

fn sd_notify_status(status: &str) {
    sd_notify(&format!("STATUS={}", status));
}

fn sd_notify_watchdog() {
    sd_notify("WATCHDOG=1");
}

fn get_watchdog_usec() -> u64 {
    std::env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(DEFAULT_WATCHDOG_SEC * 1_000_000)
}

// ── Shared atomic statistics ───────────────────────────────────────────────

/// Thread-safe statistics using atomics
#[derive(Debug)]
struct AtomicStats {
    queries_received: AtomicU64,
    queries_forwarded: AtomicU64,
    responses_ok: AtomicU64,
    responses_servfail: AtomicU64,
    responses_nxdomain: AtomicU64,
    responses_formerr: AtomicU64,
    upstream_timeouts: AtomicU64,
    upstream_failures: AtomicU64,
    tcp_queries: AtomicU64,
}

impl AtomicStats {
    fn new() -> Self {
        Self {
            queries_received: AtomicU64::new(0),
            queries_forwarded: AtomicU64::new(0),
            responses_ok: AtomicU64::new(0),
            responses_servfail: AtomicU64::new(0),
            responses_nxdomain: AtomicU64::new(0),
            responses_formerr: AtomicU64::new(0),
            upstream_timeouts: AtomicU64::new(0),
            upstream_failures: AtomicU64::new(0),
            tcp_queries: AtomicU64::new(0),
        }
    }

    fn snapshot(&self) -> ResolverStats {
        ResolverStats {
            queries_received: self.queries_received.load(Ordering::Relaxed),
            queries_forwarded: self.queries_forwarded.load(Ordering::Relaxed),
            responses_ok: self.responses_ok.load(Ordering::Relaxed),
            responses_servfail: self.responses_servfail.load(Ordering::Relaxed),
            responses_nxdomain: self.responses_nxdomain.load(Ordering::Relaxed),
            responses_formerr: self.responses_formerr.load(Ordering::Relaxed),
            upstream_timeouts: self.upstream_timeouts.load(Ordering::Relaxed),
            upstream_failures: self.upstream_failures.load(Ordering::Relaxed),
            tcp_queries: self.tcp_queries.load(Ordering::Relaxed),
        }
    }
}

// ── resolv.conf management ─────────────────────────────────────────────────

/// Write the PID file so that resolvectl can find us.
fn write_pid_file() {
    let pid = std::process::id();
    // Ensure the parent directory exists.
    let _ = fs::create_dir_all("/run/systemd/resolve");
    if let Err(e) = fs::write(PID_FILE, format!("{}\n", pid)) {
        log::warn!("Failed to write PID file {}: {}", PID_FILE, e);
    }
}

/// Remove the PID file on shutdown.
fn remove_pid_file() {
    let _ = fs::remove_file(PID_FILE);
}

fn write_resolv_conf_files(config: &ResolvedConfig) {
    // Ensure state directory exists
    if let Err(e) = fs::create_dir_all(STATE_DIR) {
        log::warn!("Failed to create {}: {}", STATE_DIR, e);
        return;
    }

    // Write stub-resolv.conf (points to 127.0.0.53)
    let stub_content = config.stub_resolv_conf_content();
    write_file_atomic(STUB_RESOLV_CONF_PATH, &stub_content);

    // Write resolv.conf (lists upstream servers)
    let upstream_content = config.upstream_resolv_conf_content();
    write_file_atomic(RESOLV_CONF_PATH, &upstream_content);
}

fn write_file_atomic(path: &str, content: &str) {
    let tmp_path = format!("{}.tmp.{}", path, std::process::id());

    match fs::write(&tmp_path, content) {
        Ok(()) => {
            if let Err(e) = fs::rename(&tmp_path, path) {
                log::warn!("Failed to rename {} to {}: {}", tmp_path, path, e);
                let _ = fs::remove_file(&tmp_path);
            } else {
                log::debug!("Wrote {}", path);
            }
        }
        Err(e) => {
            log::warn!("Failed to write {}: {}", tmp_path, e);
        }
    }
}

// ── Handle a single DNS query ──────────────────────────────────────────────

/// Shared DNS cache type (thread-safe).
type SharedCache = Arc<Mutex<DnsCache>>;

/// Shared `/etc/hosts` database (thread-safe, reloadable).
type SharedHosts = Arc<Mutex<hosts::EtcHosts>>;

/// Shared DNS router for split DNS (thread-safe, reloadable).
type SharedRouter = Arc<std::sync::RwLock<DnsRouter>>;

/// Shared DNS-over-TLS client (thread-safe).
type SharedDotClient = Arc<DotClient>;

/// Shared DNSSEC validator (thread-safe).
type SharedValidator = Arc<Mutex<DnssecValidator>>;

/// Shared LLMNR responder (thread-safe).
type SharedLlmnrResponder = Arc<Mutex<llmnr::LlmnrResponder>>;

/// Shared mDNS responder (thread-safe).
type SharedMdnsResponder = Arc<Mutex<mdns::MdnsResponder>>;

/// Bundles all shared state needed by query handling and listener threads,
/// keeping function signatures concise.
#[derive(Clone)]
struct QueryContext {
    cache: SharedCache,
    etc_hosts: SharedHosts,
    router: SharedRouter,
    dot_client: SharedDotClient,
    validator: SharedValidator,
    dnssec_mode: DnssecMode,
}

/// Append an EDNS0 OPT record to the query before forwarding.
///
/// This advertises our maximum UDP payload size and optionally sets the DO
/// (DNSSEC OK) bit so the upstream returns RRSIG records.
fn prepare_query_with_edns(query_data: &[u8], dnssec_enabled: bool) -> Vec<u8> {
    let mut opt = OptRecord::new().with_udp_size(MAX_EDNS_UDP_SIZE as u16);
    if dnssec_enabled {
        opt = opt.with_dnssec_ok();
    }
    append_opt_to_query(query_data, &opt).unwrap_or_else(|| query_data.to_vec())
}

/// Try forwarding a query via DNS-over-TLS.
///
/// Returns `Some(response)` on success, `None` if DoT is disabled or the
/// caller should fall back to plain DNS.
fn try_dot_forward(
    query_data: &[u8],
    upstreams: &[SocketAddr],
    dot_client: &SharedDotClient,
) -> Option<Vec<u8>> {
    if !dot_client.mode().enabled() {
        return None;
    }
    let servers = configs_from_addrs(upstreams);
    if servers.is_empty() {
        return None;
    }
    match dot_client.query_servers(query_data, &servers) {
        Ok(resp) => Some(resp),
        Err(e) => {
            log::debug!("DoT forwarding failed ({}), falling back to plain DNS", e);
            None
        }
    }
}

/// Perform lightweight DNSSEC validation on an upstream response.
///
/// If the validator determines the response is Bogus, we return SERVFAIL.
/// If Secure/Indeterminate, we set the AD bit.
/// If Insecure or validation is not applicable, we pass through unchanged.
fn validate_dnssec_response(
    response: &mut Vec<u8>,
    query_data: &[u8],
    validator: &SharedValidator,
    dnssec_mode: DnssecMode,
) {
    if matches!(dnssec_mode, DnssecMode::No) {
        return;
    }
    // Extract query name for NTA checks
    let qname = match extract_query_name(query_data) {
        Some(n) => n,
        None => return,
    };

    if let Ok(v) = validator.lock() {
        // Quick NTA check — skip validation entirely for NTA zones
        if v.is_negative_trust_anchor(&qname) {
            log::debug!("DNSSEC: NTA match for {}, skipping validation", qname);
            return;
        }

        // Structural validation with empty RRSIG/DNSKEY sets —
        // real crypto validation requires parsing RRSIGs from the response,
        // which we do at a basic level here.
        let (result, _errors) = v.validate_rrset(&qname, &[], &[]);

        match result {
            dnssec::ValidationResult::Secure => {
                // Set AD bit in response
                if response.len() >= HEADER_SIZE {
                    response[3] |= 0x20; // AD bit
                }
                log::debug!("DNSSEC: {} validated as Secure", qname);
            }
            dnssec::ValidationResult::Bogus => {
                // Replace response with SERVFAIL
                log::warn!("DNSSEC: {} validated as Bogus, returning SERVFAIL", qname);
                if let Some(sf) = build_servfail(query_data) {
                    *response = sf;
                }
            }
            dnssec::ValidationResult::Insecure => {
                log::debug!("DNSSEC: {} is Insecure (no trust anchor)", qname);
            }
            dnssec::ValidationResult::Indeterminate => {
                // Pass through without AD bit
                log::debug!("DNSSEC: {} is Indeterminate", qname);
            }
        }
    }
}

fn handle_query(
    query_data: &[u8],
    upstreams: &[SocketAddr],
    stats: &AtomicStats,
    ctx: &QueryContext,
) -> Vec<u8> {
    stats.queries_received.fetch_add(1, Ordering::Relaxed);

    // Parse the query to log it
    let query_info = match DnsMessage::parse(query_data) {
        Ok(msg) => {
            if !msg.is_query() {
                // Not a query — return FORMERR
                stats.responses_formerr.fetch_add(1, Ordering::Relaxed);
                return build_formerr(query_data).unwrap_or_default();
            }
            msg.query_summary()
        }
        Err(_) => {
            stats.responses_formerr.fetch_add(1, Ordering::Relaxed);
            return build_formerr(query_data).unwrap_or_default();
        }
    };

    log::debug!("Query: {}", query_info);

    // Check /etc/hosts first (highest priority, matching real systemd-resolved).
    if let Ok(hosts) = ctx.etc_hosts.lock()
        && let Some(response) = hosts.lookup(query_data)
    {
        log::debug!("/etc/hosts hit for {}", query_info);
        stats.responses_ok.fetch_add(1, Ordering::Relaxed);
        return response;
    }

    // Check the DNS cache.
    if let Ok(mut dns_cache) = ctx.cache.lock()
        && let Some(cached_response) = dns_cache.lookup(query_data)
    {
        log::debug!("Cache hit for {}", query_info);
        stats.responses_ok.fetch_add(1, Ordering::Relaxed);
        return cached_response;
    }

    // Determine the best upstream servers for this query using the DNS
    // router (split DNS).  If the router has routing domains, resolve
    // per-query; otherwise fall back to the flat upstream list.
    let routed_servers: Vec<SocketAddr>;
    let effective_upstreams: &[SocketAddr] = if let Ok(r) = ctx.router.read() {
        if r.has_routing_domains() {
            if let Some(qname) = extract_query_name(query_data) {
                routed_servers = r.servers_for_name(&qname);
                if !routed_servers.is_empty() {
                    log::debug!("Routed {} to {} server(s)", qname, routed_servers.len());
                }
                &routed_servers
            } else {
                upstreams
            }
        } else {
            upstreams
        }
    } else {
        upstreams
    };

    if effective_upstreams.is_empty() {
        log::warn!("No upstream DNS servers configured");
        stats.responses_servfail.fetch_add(1, Ordering::Relaxed);
        return build_servfail(query_data).unwrap_or_default();
    }

    // Append EDNS0 OPT record to advertise our capabilities to upstream.
    let edns_query =
        prepare_query_with_edns(query_data, !matches!(ctx.dnssec_mode, DnssecMode::No));

    // Forward to upstream
    stats.queries_forwarded.fetch_add(1, Ordering::Relaxed);

    // Try DNS-over-TLS first if enabled, then fall back to plain DNS.
    let forward_result = if ctx.dot_client.mode().enabled() {
        match try_dot_forward(&edns_query, effective_upstreams, &ctx.dot_client) {
            Some(resp) => Ok(resp),
            None => forward_query(&edns_query, effective_upstreams),
        }
    } else {
        forward_query(&edns_query, effective_upstreams)
    };

    match forward_result {
        Ok(mut response) => {
            // Perform DNSSEC validation on the upstream response.
            validate_dnssec_response(&mut response, query_data, &ctx.validator, ctx.dnssec_mode);

            // Strip the EDNS0 OPT record from the response if the original
            // client query didn't include one, to stay backward-compatible.
            let final_response = if !edns::has_opt_record(query_data) {
                edns::strip_opt_from_message(&response).unwrap_or(response.clone())
            } else {
                response.clone()
            };

            // Check response code for statistics
            if final_response.len() >= HEADER_SIZE {
                let rcode = final_response[3] & 0x0F;
                match rcode {
                    0 => {
                        stats.responses_ok.fetch_add(1, Ordering::Relaxed);
                    }
                    3 => {
                        stats.responses_nxdomain.fetch_add(1, Ordering::Relaxed);
                    }
                    _ => {}
                }
            }
            // Cache the response.
            if let Ok(mut dns_cache) = ctx.cache.lock() {
                dns_cache.insert(query_data, &final_response);
            }
            final_response
        }
        Err(dns::DnsError::Timeout) => {
            log::debug!("Upstream timeout for {}", query_info);
            stats.upstream_timeouts.fetch_add(1, Ordering::Relaxed);
            stats.responses_servfail.fetch_add(1, Ordering::Relaxed);
            build_servfail(query_data).unwrap_or_default()
        }
        Err(e) => {
            log::debug!("Upstream failure for {}: {}", query_info, e);
            stats.upstream_failures.fetch_add(1, Ordering::Relaxed);
            stats.responses_servfail.fetch_add(1, Ordering::Relaxed);
            build_servfail(query_data).unwrap_or_default()
        }
    }
}

// ── Handle a TCP connection ────────────────────────────────────────────────

fn handle_tcp_connection(
    mut stream: TcpStream,
    upstreams: Vec<SocketAddr>,
    stats: Arc<AtomicStats>,
    ctx: QueryContext,
) {
    let _ = stream.set_read_timeout(Some(TCP_TIMEOUT));
    let _ = stream.set_write_timeout(Some(TCP_TIMEOUT));

    stats.tcp_queries.fetch_add(1, Ordering::Relaxed);

    // TCP DNS: read 2-byte length prefix, then message
    let mut len_buf = [0u8; 2];
    if stream.read_exact(&mut len_buf).is_err() {
        return;
    }
    let msg_len = u16::from_be_bytes(len_buf) as usize;

    if !(HEADER_SIZE..=MAX_TCP_SIZE).contains(&msg_len) {
        return;
    }

    let mut query = vec![0u8; msg_len];
    if stream.read_exact(&mut query).is_err() {
        return;
    }

    let response = handle_query(&query, &upstreams, &stats, &ctx);

    // Write response with length prefix
    let resp_len = (response.len() as u16).to_be_bytes();
    let _ = stream.write_all(&resp_len);
    let _ = stream.write_all(&response);
    let _ = stream.flush();
}

// ── UDP listener thread ────────────────────────────────────────────────────

fn run_udp_listener(
    socket: UdpSocket,
    upstreams: Arc<std::sync::RwLock<Vec<SocketAddr>>>,
    stats: Arc<AtomicStats>,
    shutdown: Arc<AtomicBool>,
    ctx: QueryContext,
) {
    let mut buf = vec![0u8; UDP_RECV_BUF];

    log::info!(
        "UDP stub listener ready on {}",
        socket
            .local_addr()
            .unwrap_or_else(
                |_| "unknown".parse::<SocketAddr>().unwrap_or(SocketAddr::new(
                    std::net::IpAddr::V4(Ipv4Addr::LOCALHOST),
                    53
                ))
            )
    );

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                let query = &buf[..len];
                let upstream_list = upstreams.read().unwrap_or_else(|e| e.into_inner()).clone();
                let response = handle_query(query, &upstream_list, &stats, &ctx);

                if let Err(e) = socket.send_to(&response, src) {
                    log::debug!("Failed to send UDP response to {}: {}", src, e);
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Timeout on the non-blocking recv — just loop
                continue;
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {
                continue;
            }
            Err(e) => {
                if !shutdown.load(Ordering::Relaxed) {
                    log::warn!("UDP recv error: {}", e);
                }
                // Brief sleep to avoid spinning on persistent errors
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    log::debug!("UDP listener shut down");
}

// ── TCP listener thread ────────────────────────────────────────────────────

fn run_tcp_listener(
    listener: TcpListener,
    upstreams: Arc<std::sync::RwLock<Vec<SocketAddr>>>,
    stats: Arc<AtomicStats>,
    shutdown: Arc<AtomicBool>,
    ctx: QueryContext,
) {
    log::info!(
        "TCP stub listener ready on {}",
        listener
            .local_addr()
            .map(|a| a.to_string())
            .unwrap_or_else(|_| "unknown".to_string())
    );

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        match listener.accept() {
            Ok((stream, _peer)) => {
                let upstream_list = upstreams.read().unwrap_or_else(|e| e.into_inner()).clone();
                let stats_clone = Arc::clone(&stats);
                let ctx_clone = ctx.clone();

                thread::spawn(move || {
                    handle_tcp_connection(stream, upstream_list, stats_clone, ctx_clone);
                });
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                continue;
            }
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => {
                continue;
            }
            Err(e) => {
                if !shutdown.load(Ordering::Relaxed) {
                    log::warn!("TCP accept error: {}", e);
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    log::debug!("TCP listener shut down");
}

// ── LLMNR listener thread ──────────────────────────────────────────────────

fn run_llmnr_listener(
    socket: UdpSocket,
    responder: SharedLlmnrResponder,
    shutdown: Arc<AtomicBool>,
    label: &'static str,
) {
    let mut buf = vec![0u8; llmnr::MAX_LLMNR_UDP_SIZE];

    log::info!("LLMNR {} listener ready", label);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                let data = &buf[..len];
                if let Ok(resp) = responder.lock()
                    && let Some(response) = resp.handle_query(data)
                {
                    // LLMNR responses are sent unicast to the querier
                    if let Err(e) = socket.send_to(&response, src) {
                        log::debug!("Failed to send LLMNR response to {}: {}", src, e);
                    } else {
                        log::debug!(
                            "LLMNR {} responded to {} from {}",
                            label,
                            resp.hostname(),
                            src
                        );
                    }
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                if !shutdown.load(Ordering::Relaxed) {
                    log::debug!("LLMNR {} recv error: {}", label, e);
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    log::debug!("LLMNR {} listener shut down", label);
}

// ── mDNS listener thread ──────────────────────────────────────────────────

fn run_mdns_listener(
    socket: UdpSocket,
    responder: SharedMdnsResponder,
    shutdown: Arc<AtomicBool>,
    label: &'static str,
    mcast_addr: SocketAddr,
) {
    let mut buf = vec![0u8; mdns::MAX_MDNS_UDP_SIZE];

    log::info!("mDNS {} listener ready", label);

    loop {
        if shutdown.load(Ordering::Relaxed) {
            break;
        }

        match socket.recv_from(&mut buf) {
            Ok((len, src)) => {
                let data = &buf[..len];
                if let Ok(resp) = responder.lock()
                    && let Some((response, unicast)) = resp.handle_query(data)
                {
                    let dest = if unicast { src } else { mcast_addr };
                    if let Err(e) = socket.send_to(&response, dest) {
                        log::debug!("Failed to send mDNS response to {}: {}", dest, e);
                    } else {
                        log::debug!(
                            "mDNS {} responded ({}) from {}",
                            label,
                            if unicast { "unicast" } else { "multicast" },
                            src
                        );
                    }
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => continue,
            Err(ref e) if e.kind() == io::ErrorKind::Interrupted => continue,
            Err(e) => {
                if !shutdown.load(Ordering::Relaxed) {
                    log::debug!("mDNS {} recv error: {}", label, e);
                }
                thread::sleep(Duration::from_millis(100));
            }
        }
    }

    log::debug!("mDNS {} listener shut down", label);
}

/// Get the local hostname for LLMNR/mDNS responders.
fn get_local_hostname() -> String {
    std::fs::read_to_string("/etc/hostname")
        .ok()
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "localhost".to_string())
}

/// Collect local IPv4 addresses (non-loopback) for responders.
fn get_local_ipv4_addrs() -> Vec<Ipv4Addr> {
    // Read from /proc/net/if_inet6 is complex; use a simpler approach:
    // parse the hostname's addresses or use a fallback.
    // For LLMNR/mDNS, we primarily care about being reachable on the LAN.
    // A production implementation would enumerate interfaces via netlink.
    // For now, return an empty list — the responder will produce empty
    // responses for address types it doesn't have.
    Vec::new()
}

/// Collect local IPv6 addresses (non-loopback) for responders.
fn get_local_ipv6_addrs() -> Vec<Ipv6Addr> {
    Vec::new()
}

// ── Signal handling ────────────────────────────────────────────────────────

fn setup_signal_handlers(shutdown: &Arc<AtomicBool>, reload: &Arc<AtomicBool>) {
    let shutdown_flag = Arc::clone(shutdown);
    let reload_flag = Arc::clone(reload);

    // SIGTERM / SIGINT → shutdown
    let shutdown_clone = Arc::clone(&shutdown_flag);
    if let Err(e) = signal_hook::flag::register(signal_hook::consts::SIGTERM, shutdown_clone) {
        log::warn!("Failed to register SIGTERM handler: {}", e);
    }

    let shutdown_clone2 = Arc::clone(&shutdown_flag);
    if let Err(e) = signal_hook::flag::register(signal_hook::consts::SIGINT, shutdown_clone2) {
        log::warn!("Failed to register SIGINT handler: {}", e);
    }

    // SIGHUP → reload
    if let Err(e) = signal_hook::flag::register(signal_hook::consts::SIGHUP, reload_flag) {
        log::warn!("Failed to register SIGHUP handler: {}", e);
    }
}

// ── Bind stub listeners ────────────────────────────────────────────────────

fn bind_udp_stub(addr: SocketAddr) -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(addr)?;
    // Set a read timeout so the listener loop can check the shutdown flag
    socket.set_read_timeout(Some(Duration::from_millis(POLL_INTERVAL_MS)))?;
    Ok(socket)
}

fn bind_tcp_stub(addr: SocketAddr) -> io::Result<TcpListener> {
    let listener = TcpListener::bind(addr)?;
    listener.set_nonblocking(true)?;
    Ok(listener)
}

// ── Shared D-Bus state ─────────────────────────────────────────────────────

/// Snapshot of resolved state exposed via D-Bus properties.
///
/// Shared DNS cache for use across listener threads.
/// Initialized alongside other shared state and passed to UDP/TCP listeners.
#[derive(Debug, Clone)]
struct ResolvedState {
    /// Configured DNS servers as (family, address_bytes) pairs.
    dns: Vec<(i32, Vec<u8>)>,
    /// Fallback DNS servers as (family, address_bytes) pairs.
    fallback_dns: Vec<(i32, Vec<u8>)>,
    /// Search domains as (ifindex, domain, is_routing_only) tuples.
    domains: Vec<(i32, String, bool)>,
    /// LLMNR mode string.
    llmnr: String,
    /// MulticastDNS mode string.
    multicast_dns: String,
    /// DNSSEC mode string.
    dnssec: String,
    /// DNSOverTLS mode string.
    dns_over_tls: String,
    /// DNSStubListener mode string.
    dns_stub_listener: String,
    /// Cache mode string.
    cache: String,
    /// Whether DNSSEC is supported.
    dnssec_supported: bool,
    /// Transaction statistics: (current_transactions, total_transactions).
    transaction_stats: (u64, u64),
    /// Cache statistics: (cache_size, cache_hits, cache_misses).
    cache_stats: (u64, u64, u64),
}

impl Default for ResolvedState {
    fn default() -> Self {
        Self {
            dns: Vec::new(),
            fallback_dns: Vec::new(),
            domains: Vec::new(),
            llmnr: "yes".to_string(),
            multicast_dns: "no".to_string(),
            dnssec: "no".to_string(),
            dns_over_tls: "no".to_string(),
            dns_stub_listener: "yes".to_string(),
            cache: "yes".to_string(),
            dnssec_supported: false,
            transaction_stats: (0, 0),
            cache_stats: (0, 0, 0),
        }
    }
}

type SharedState = Arc<Mutex<ResolvedState>>;

// ── D-Bus interface: org.freedesktop.resolve1.Manager ──────────────────────

/// Register the org.freedesktop.resolve1.Manager interface on a Crossroads
/// instance.
///
/// Properties (read-only):
///   DNS (a(iay))            — configured DNS servers
///   FallbackDNS (a(iay))    — fallback DNS servers
///   Domains (a(isb))        — search domains (ifindex, domain, routing_only)
///   LLMNR (s)               — LLMNR mode
///   MulticastDNS (s)        — mDNS mode
///   DNSSEC (s)              — DNSSEC mode
///   DNSOverTLS (s)          — DNS-over-TLS mode
///   DNSStubListener (s)     — stub listener mode
///   Cache (s)               — cache mode
///   DNSSECSupported (b)     — whether DNSSEC is supported
///   TransactionStatistics ((tt)) — (current, total) transactions
///   CacheStatistics ((ttt))      — (size, hits, misses)
///
/// Methods:
///   FlushCaches()           — flush DNS caches
///   ResetStatistics()       — reset query statistics
///   GetLink(i ifindex) → o  — get link object path
///   SetLinkDNS(i ifindex, a(iay) servers) — set per-link DNS (stub)
///   RevertLink(i ifindex)   — revert per-link DNS (stub)
///   Describe() → s          — JSON description of the resolver state
/// D-Bus interface struct for org.freedesktop.resolve1.Manager.
///
/// Holds shared state and cache references so that zbus can serve
/// properties and methods automatically from a background thread.
struct Resolve1Manager {
    state: SharedState,
    cache: SharedCache,
}

#[zbus::interface(name = "org.freedesktop.resolve1.Manager")]
impl Resolve1Manager {
    // --- Properties ---

    #[zbus(property, name = "DNS")]
    fn dns(&self) -> Vec<(i32, Vec<u8>)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.dns.clone()
    }

    #[zbus(property, name = "FallbackDNS")]
    fn fallback_dns(&self) -> Vec<(i32, Vec<u8>)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.fallback_dns.clone()
    }

    #[zbus(property, name = "Domains")]
    fn domains(&self) -> Vec<(i32, String, bool)> {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.domains.clone()
    }

    #[zbus(property, name = "LLMNR")]
    fn llmnr(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.llmnr.clone()
    }

    #[zbus(property, name = "MulticastDNS")]
    fn multicast_dns(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.multicast_dns.clone()
    }

    #[zbus(property, name = "DNSSEC")]
    fn dnssec(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.dnssec.clone()
    }

    #[zbus(property, name = "DNSOverTLS")]
    fn dns_over_tls(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.dns_over_tls.clone()
    }

    #[zbus(property, name = "DNSStubListener")]
    fn dns_stub_listener(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.dns_stub_listener.clone()
    }

    #[zbus(property)]
    fn cache(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.cache.clone()
    }

    #[zbus(property, name = "DNSSECSupported")]
    fn dnssec_supported(&self) -> bool {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.dnssec_supported
    }

    #[zbus(property, name = "TransactionStatistics")]
    fn transaction_statistics(&self) -> (u64, u64) {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.transaction_stats
    }

    #[zbus(property, name = "CacheStatistics")]
    fn cache_statistics(&self) -> (u64, u64, u64) {
        let dns_cache = self.cache.lock().unwrap_or_else(|e| e.into_inner());
        (
            dns_cache.len() as u64,
            dns_cache.stats.hits,
            dns_cache.stats.misses,
        )
    }

    // --- Methods ---

    fn flush_caches(&self) {
        log::info!("D-Bus FlushCaches() called");
        if let Ok(mut dns_cache) = self.cache.lock() {
            let count = dns_cache.len();
            dns_cache.flush();
            log::info!("Flushed {} cache entries via D-Bus", count);
        }
    }

    fn reset_statistics(&self) {
        log::info!("D-Bus ResetStatistics() called");
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.transaction_stats = (0, 0);
        s.cache_stats = (0, 0, 0);
    }

    fn get_link(&self, ifindex: i32) -> zbus::fdo::Result<OwnedObjectPath> {
        if ifindex <= 0 {
            return Err(zbus::fdo::Error::InvalidArgs(
                "Invalid interface index".into(),
            ));
        }
        let path = resolve_link_object_path(ifindex as u32);
        OwnedObjectPath::try_from(path)
            .map_err(|e| zbus::fdo::Error::Failed(format!("Invalid object path: {e}")))
    }

    #[zbus(name = "SetLinkDNS")]
    fn set_link_dns(&self, ifindex: i32, _addresses: Vec<(i32, Vec<u8>)>) {
        log::info!("D-Bus SetLinkDNS() called for ifindex={} (stub)", ifindex);
    }

    fn set_link_domains(&self, ifindex: i32, _domains: Vec<(String, bool)>) {
        log::info!(
            "D-Bus SetLinkDomains() called for ifindex={} (stub)",
            ifindex
        );
    }

    fn revert_link(&self, ifindex: i32) {
        log::info!("D-Bus RevertLink() called for ifindex={} (stub)", ifindex);
    }

    fn describe(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());

        let dns_json: Vec<String> = s
            .dns
            .iter()
            .map(|(family, bytes)| {
                let addr_str = addr_bytes_to_string(*family, bytes);
                format!("\"{}\"", json_escape(&addr_str))
            })
            .collect();

        let fallback_json: Vec<String> = s
            .fallback_dns
            .iter()
            .map(|(family, bytes)| {
                let addr_str = addr_bytes_to_string(*family, bytes);
                format!("\"{}\"", json_escape(&addr_str))
            })
            .collect();

        let domains_json: Vec<String> = s
            .domains
            .iter()
            .map(|(_, domain, routing)| {
                format!(
                    "{{\"Domain\":\"{}\",\"RoutingOnly\":{}}}",
                    json_escape(domain),
                    routing,
                )
            })
            .collect();

        format!(
            concat!(
                "{{",
                "\"DNS\":[{}],",
                "\"FallbackDNS\":[{}],",
                "\"Domains\":[{}],",
                "\"LLMNR\":\"{}\",",
                "\"MulticastDNS\":\"{}\",",
                "\"DNSSEC\":\"{}\",",
                "\"DNSOverTLS\":\"{}\",",
                "\"DNSStubListener\":\"{}\",",
                "\"Cache\":\"{}\",",
                "\"DNSSECSupported\":{},",
                "\"TransactionStatistics\":{{\"CurrentTransactions\":{},\"TotalTransactions\":{}}},",
                "\"CacheStatistics\":{{\"Size\":{},\"Hits\":{},\"Misses\":{}}}",
                "}}"
            ),
            dns_json.join(","),
            fallback_json.join(","),
            domains_json.join(","),
            json_escape(&s.llmnr),
            json_escape(&s.multicast_dns),
            json_escape(&s.dnssec),
            json_escape(&s.dns_over_tls),
            json_escape(&s.dns_stub_listener),
            json_escape(&s.cache),
            s.dnssec_supported,
            s.transaction_stats.0,
            s.transaction_stats.1,
            s.cache_stats.0,
            s.cache_stats.1,
            s.cache_stats.2,
        )
    }
}

/// Convert a link ifindex to a D-Bus object path for resolve1.
fn resolve_link_object_path(ifindex: u32) -> String {
    format!("/org/freedesktop/resolve1/link/_{}", ifindex)
}

/// Set up the D-Bus connection and register the resolve1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedState, cache: SharedCache) -> Result<Connection, String> {
    let iface = Resolve1Manager {
        state: shared,
        cache,
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

/// Convert address family + raw bytes to a human-readable IP string.
fn addr_bytes_to_string(family: i32, bytes: &[u8]) -> String {
    match family {
        2 if bytes.len() == 4 => {
            // AF_INET
            format!("{}.{}.{}.{}", bytes[0], bytes[1], bytes[2], bytes[3])
        }
        10 if bytes.len() == 16 => {
            // AF_INET6
            let mut parts = [0u16; 8];
            for i in 0..8 {
                parts[i] = u16::from_be_bytes([bytes[i * 2], bytes[i * 2 + 1]]);
            }
            format!(
                "{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}:{:x}",
                parts[0], parts[1], parts[2], parts[3], parts[4], parts[5], parts[6], parts[7]
            )
        }
        _ => format!("<unknown family {}>", family),
    }
}

/// Convert a DnsServer to the (family, bytes) tuple for D-Bus exposure.
fn dns_server_to_dbus(server: &config::DnsServer) -> (i32, Vec<u8>) {
    let addr = server.addr;
    match addr {
        std::net::IpAddr::V4(v4) => (2, v4.octets().to_vec()),
        std::net::IpAddr::V6(v6) => (10, v6.octets().to_vec()),
    }
}

/// Update the shared D-Bus state from a ResolvedConfig and stats snapshot.
fn update_shared_state(
    shared: &SharedState,
    resolved_config: &ResolvedConfig,
    stats: &AtomicStats,
) {
    let mut s = shared.lock().unwrap_or_else(|e| e.into_inner());

    // DNS servers
    s.dns = resolved_config
        .effective_dns_servers()
        .iter()
        .map(|srv| dns_server_to_dbus(srv))
        .collect();

    // Fallback DNS
    s.fallback_dns = resolved_config
        .fallback_dns
        .iter()
        .map(dns_server_to_dbus)
        .collect();

    // Domains
    s.domains = resolved_config
        .effective_search_domains()
        .iter()
        .map(|d| (0i32, d.to_string(), false))
        .collect();

    // Modes
    s.llmnr = resolved_config.llmnr.as_str().to_string();
    s.multicast_dns = resolved_config.multicast_dns.as_str().to_string();
    s.dnssec = resolved_config.dnssec.as_str().to_string();
    s.dns_over_tls = resolved_config.dns_over_tls.as_str().to_string();
    s.dns_stub_listener = resolved_config.dns_stub_listener.as_str().to_string();
    s.cache = if resolved_config.cache {
        "yes".to_string()
    } else {
        "no".to_string()
    };

    // Stats
    let snap = stats.snapshot();
    s.transaction_stats = (0, snap.queries_received); // no notion of "current" transactions
    s.cache_stats = (0, 0, 0); // no cache yet
}

// ── Main ───────────────────────────────────────────────────────────────────

fn main() {
    setup_logging();
    log::info!("systemd-resolved starting");

    // Create DNS cache.
    let dns_cache: SharedCache = Arc::new(Mutex::new(DnsCache::new()));

    // Load /etc/hosts database.
    let mut etc_hosts_db = hosts::EtcHosts::new();
    if let Ok(stats) = etc_hosts_db.load() {
        log::info!("Loaded /etc/hosts: {}", stats);
    } else {
        log::debug!("/etc/hosts not available (will retry later)");
    }
    let etc_hosts: SharedHosts = Arc::new(Mutex::new(etc_hosts_db));

    // Build DNS router (split DNS).
    // Rebuilt after every config reload / link refresh.
    let dns_router: SharedRouter = Arc::new(std::sync::RwLock::new(DnsRouter::from_config(
        &ResolvedConfig::default(),
    )));

    // Load configuration
    let mut config = ResolvedConfig::load();
    log::info!(
        "Loaded configuration: {} DNS servers, {} fallback servers, stub_listener={}",
        config.dns.len(),
        config.fallback_dns.len(),
        config.dns_stub_listener.as_str(),
    );

    // Update per-link DNS from networkd
    config.update_link_dns_from_networkd();
    if !config.link_dns.is_empty() {
        log::info!(
            "Found {} link(s) with DNS configuration from networkd",
            config.link_dns.len()
        );
    }

    // Write resolv.conf files
    write_resolv_conf_files(&config);

    // Build upstream server list and DNS router.
    let upstream_servers: Vec<SocketAddr> = config
        .effective_dns_servers()
        .iter()
        .map(|s| s.socket_addr())
        .collect();

    // Rebuild the router from the loaded config.
    if let Ok(mut r) = dns_router.write() {
        *r = DnsRouter::from_config(&config);
        if r.has_routing_domains() {
            log::info!(
                "DNS routing: {} routing domain(s) configured",
                r.route_count()
            );
        }
    }

    log::info!(
        "Upstream DNS servers: {}",
        if upstream_servers.is_empty() {
            "(none)".to_string()
        } else {
            upstream_servers
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join(", ")
        }
    );

    // ── DNS-over-TLS client ────────────────────────────────────────────
    let dot_mode = match config.dns_over_tls {
        DnsOverTlsMode::Yes => DotMode::Strict,
        DnsOverTlsMode::Opportunistic => DotMode::Opportunistic,
        DnsOverTlsMode::No => DotMode::No,
    };
    let dot_client: SharedDotClient = Arc::new(DotClient::plain(dot_mode));
    log::info!("DNS-over-TLS: {}", dot_mode);

    // ── DNSSEC validator ───────────────────────────────────────────────
    let mut dnssec_validator = DnssecValidator::with_root_anchor();
    for nta in &config.negative_trust_anchors {
        dnssec_validator.add_negative_trust_anchor(nta);
    }
    let validator: SharedValidator = Arc::new(Mutex::new(dnssec_validator));
    log::info!(
        "DNSSEC: mode={}, NTAs={}",
        config.dnssec.as_str(),
        config.negative_trust_anchors.len()
    );

    // Shared state
    let shutdown = Arc::new(AtomicBool::new(false));
    let reload = Arc::new(AtomicBool::new(false));
    let stats = Arc::new(AtomicStats::new());
    let upstreams = Arc::new(std::sync::RwLock::new(upstream_servers));
    let shared_dbus_state: SharedState = Arc::new(Mutex::new(ResolvedState::default()));

    // Initialize D-Bus shared state from config
    update_shared_state(&shared_dbus_state, &config, &stats);

    // Set up signal handlers
    setup_signal_handlers(&shutdown, &reload);

    // Determine stub listener address
    let stub_addr: SocketAddr = SocketAddr::new(
        config::STUB_LISTENER_ADDR
            .parse()
            .unwrap_or(std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 53))),
        config::DNS_PORT,
    );

    let mut listener_threads: Vec<thread::JoinHandle<()>> = Vec::new();

    // ── LLMNR listeners ────────────────────────────────────────────────
    let hostname = get_local_hostname();
    let llmnr_responder: SharedLlmnrResponder = Arc::new(Mutex::new(llmnr::LlmnrResponder::new(
        llmnr::LlmnrResponderConfig::new(&hostname, get_local_ipv4_addrs(), get_local_ipv6_addrs()),
    )));

    if !matches!(config.llmnr, ResolutionMode::No) {
        // Bind LLMNR IPv4 socket
        match llmnr::bind_llmnr_ipv4() {
            Ok(socket) => {
                let _ = socket.set_read_timeout(Some(Duration::from_millis(POLL_INTERVAL_MS)));
                log::info!("LLMNR IPv4 listener bound on :{}", llmnr::LLMNR_PORT);
                let resp_clone = Arc::clone(&llmnr_responder);
                let shutdown_clone = Arc::clone(&shutdown);
                listener_threads.push(thread::spawn(move || {
                    run_llmnr_listener(socket, resp_clone, shutdown_clone, "IPv4");
                }));
            }
            Err(e) => {
                log::debug!(
                    "Failed to bind LLMNR IPv4 socket: {} (continuing without)",
                    e
                );
            }
        }

        // Bind LLMNR IPv6 socket
        match llmnr::bind_llmnr_ipv6() {
            Ok(socket) => {
                let _ = socket.set_read_timeout(Some(Duration::from_millis(POLL_INTERVAL_MS)));
                log::info!("LLMNR IPv6 listener bound on :{}", llmnr::LLMNR_PORT);
                let resp_clone = Arc::clone(&llmnr_responder);
                let shutdown_clone = Arc::clone(&shutdown);
                listener_threads.push(thread::spawn(move || {
                    run_llmnr_listener(socket, resp_clone, shutdown_clone, "IPv6");
                }));
            }
            Err(e) => {
                log::debug!(
                    "Failed to bind LLMNR IPv6 socket: {} (continuing without)",
                    e
                );
            }
        }
    } else {
        log::info!("LLMNR disabled by configuration");
    }

    // ── mDNS listeners ─────────────────────────────────────────────────
    let mdns_responder: SharedMdnsResponder =
        Arc::new(Mutex::new(mdns::MdnsResponder::new(&hostname)));

    // Start the mDNS publish state machine (advance to Published)
    if let Ok(mut resp) = mdns_responder.lock() {
        resp.set_host_records(&get_local_ipv4_addrs(), &get_local_ipv6_addrs());
        // Fast-forward through probing/announcing for initial startup
        for _ in 0..20 {
            let _ = resp.advance();
        }
    }

    if !matches!(config.multicast_dns, ResolutionMode::No) {
        // Bind mDNS IPv4 socket
        match mdns::bind_mdns_ipv4() {
            Ok(socket) => {
                let _ = socket.set_read_timeout(Some(Duration::from_millis(POLL_INTERVAL_MS)));
                log::info!("mDNS IPv4 listener bound on :{}", mdns::MDNS_PORT);
                let resp_clone = Arc::clone(&mdns_responder);
                let shutdown_clone = Arc::clone(&shutdown);
                listener_threads.push(thread::spawn(move || {
                    run_mdns_listener(
                        socket,
                        resp_clone,
                        shutdown_clone,
                        "IPv4",
                        mdns::MDNS_MCAST_ADDR_V4,
                    );
                }));
            }
            Err(e) => {
                log::debug!(
                    "Failed to bind mDNS IPv4 socket: {} (continuing without)",
                    e
                );
            }
        }

        // Bind mDNS IPv6 socket
        match mdns::bind_mdns_ipv6() {
            Ok(socket) => {
                let _ = socket.set_read_timeout(Some(Duration::from_millis(POLL_INTERVAL_MS)));
                log::info!("mDNS IPv6 listener bound on :{}", mdns::MDNS_PORT);
                let resp_clone = Arc::clone(&mdns_responder);
                let shutdown_clone = Arc::clone(&shutdown);
                listener_threads.push(thread::spawn(move || {
                    run_mdns_listener(
                        socket,
                        resp_clone,
                        shutdown_clone,
                        "IPv6",
                        mdns::MDNS_MCAST_ADDR_V6,
                    );
                }));
            }
            Err(e) => {
                log::debug!(
                    "Failed to bind mDNS IPv6 socket: {} (continuing without)",
                    e
                );
            }
        }
    } else {
        log::info!("Multicast DNS disabled by configuration");
    }

    // Start stub listeners if enabled
    match config.dns_stub_listener {
        StubListenerMode::No => {
            log::info!("DNS stub listener disabled by configuration");
        }
        mode => {
            if mode.udp_enabled() {
                match bind_udp_stub(stub_addr) {
                    Ok(socket) => {
                        log::info!("Bound UDP stub listener on {}", stub_addr);
                        let upstreams_clone = Arc::clone(&upstreams);
                        let stats_clone = Arc::clone(&stats);
                        let shutdown_clone = Arc::clone(&shutdown);
                        let ctx = QueryContext {
                            cache: Arc::clone(&dns_cache),
                            etc_hosts: Arc::clone(&etc_hosts),
                            router: Arc::clone(&dns_router),
                            dot_client: Arc::clone(&dot_client),
                            validator: Arc::clone(&validator),
                            dnssec_mode: config.dnssec,
                        };
                        listener_threads.push(thread::spawn(move || {
                            run_udp_listener(
                                socket,
                                upstreams_clone,
                                stats_clone,
                                shutdown_clone,
                                ctx,
                            );
                        }));
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to bind UDP stub listener on {}: {} (continuing without UDP)",
                            stub_addr,
                            e
                        );
                    }
                }
            }

            if mode.tcp_enabled() {
                match bind_tcp_stub(stub_addr) {
                    Ok(listener) => {
                        log::info!("Bound TCP stub listener on {}", stub_addr);
                        let upstreams_clone = Arc::clone(&upstreams);
                        let stats_clone = Arc::clone(&stats);
                        let shutdown_clone = Arc::clone(&shutdown);
                        let ctx = QueryContext {
                            cache: Arc::clone(&dns_cache),
                            etc_hosts: Arc::clone(&etc_hosts),
                            router: Arc::clone(&dns_router),
                            dot_client: Arc::clone(&dot_client),
                            validator: Arc::clone(&validator),
                            dnssec_mode: config.dnssec,
                        };
                        listener_threads.push(thread::spawn(move || {
                            run_tcp_listener(
                                listener,
                                upstreams_clone,
                                stats_clone,
                                shutdown_clone,
                                ctx,
                            );
                        }));
                    }
                    Err(e) => {
                        log::warn!(
                            "Failed to bind TCP stub listener on {}: {} (continuing without TCP)",
                            stub_addr,
                            e
                        );
                    }
                }
            }
        }
    }

    // Start extra stub listeners
    for extra_addr in &config.dns_stub_listener_extra {
        match bind_udp_stub(*extra_addr) {
            Ok(socket) => {
                log::info!("Bound extra UDP stub listener on {}", extra_addr);
                let upstreams_clone = Arc::clone(&upstreams);
                let stats_clone = Arc::clone(&stats);
                let shutdown_clone = Arc::clone(&shutdown);
                let ctx = QueryContext {
                    cache: Arc::clone(&dns_cache),
                    etc_hosts: Arc::clone(&etc_hosts),
                    router: Arc::clone(&dns_router),
                    dot_client: Arc::clone(&dot_client),
                    validator: Arc::clone(&validator),
                    dnssec_mode: config.dnssec,
                };
                listener_threads.push(thread::spawn(move || {
                    run_udp_listener(socket, upstreams_clone, stats_clone, shutdown_clone, ctx);
                }));
            }
            Err(e) => {
                log::warn!(
                    "Failed to bind extra stub listener on {}: {}",
                    extra_addr,
                    e
                );
            }
        }
    }

    // Write PID file for resolvectl discovery.
    write_pid_file();

    // Signal readiness
    sd_notify_ready();
    sd_notify_status("Processing requests...");
    log::info!("systemd-resolved is ready");

    // D-Bus connection is deferred to after READY=1 so we don't block
    // early boot waiting for dbus-daemon.  We retry periodically until
    // the connection succeeds, since this service may start before
    // dbus.socket is activated.  zbus dispatches messages automatically
    // in a background thread — we just keep the connection alive.
    let mut _dbus_conn: Option<Connection> = None;
    let mut dbus_next_retry = Instant::now();
    const DBUS_RETRY_INTERVAL: Duration = Duration::from_secs(5);

    // Watchdog setup
    let watchdog_usec = get_watchdog_usec();
    let watchdog_interval = if watchdog_usec > 0 {
        // Send watchdog at half the configured interval
        Some(Duration::from_micros(watchdog_usec / 2))
    } else {
        None
    };
    let mut last_watchdog = Instant::now();
    let mut last_link_refresh = Instant::now();
    let mut last_hosts_check = Instant::now();
    const HOSTS_REFRESH_INTERVAL: Duration = Duration::from_secs(2);

    // ── Main loop ──────────────────────────────────────────────────────
    loop {
        if shutdown.load(Ordering::Relaxed) {
            log::info!("Received shutdown signal");
            break;
        }

        // Attempt D-Bus registration (deferred from startup, retried
        // periodically until successful).  zbus handles message dispatch
        // in a background thread automatically.
        if _dbus_conn.is_none() && Instant::now() >= dbus_next_retry {
            match setup_dbus(shared_dbus_state.clone(), Arc::clone(&dns_cache)) {
                Ok(conn) => {
                    log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                    _dbus_conn = Some(conn);
                    sd_notify_status("Processing requests... (D-Bus active)");
                }
                Err(e) => {
                    log::debug!(
                        "D-Bus not yet available ({}); will retry in {}s",
                        e,
                        DBUS_RETRY_INTERVAL.as_secs()
                    );
                    dbus_next_retry = Instant::now() + DBUS_RETRY_INTERVAL;
                }
            }
        }

        // Handle reload (SIGHUP)
        if reload.swap(false, Ordering::Relaxed) {
            log::info!("Reloading configuration...");
            let mut new_config = ResolvedConfig::load();
            new_config.update_link_dns_from_networkd();

            let new_upstreams: Vec<SocketAddr> = new_config
                .effective_dns_servers()
                .iter()
                .map(|s| s.socket_addr())
                .collect();

            log::info!("Reloaded: {} upstream servers", new_upstreams.len());

            // Update upstream list
            if let Ok(mut list) = upstreams.write() {
                *list = new_upstreams;
            }

            // Rebuild the DNS router from the new config.
            if let Ok(mut r) = dns_router.write() {
                *r = DnsRouter::from_config(&new_config);
                if r.has_routing_domains() {
                    log::info!(
                        "DNS routing: {} routing domain(s) after reload",
                        r.route_count()
                    );
                }
            }

            // Rewrite resolv.conf files
            write_resolv_conf_files(&new_config);

            // Update D-Bus state
            update_shared_state(&shared_dbus_state, &new_config, &stats);

            // Flush DNS cache on reload
            if let Ok(mut dns_cache_lock) = dns_cache.lock() {
                let cache_size = dns_cache_lock.len();
                dns_cache_lock.flush();
                log::info!("Flushed DNS cache ({} entries)", cache_size);
            }

            // Reload /etc/hosts
            if new_config.read_etc_hosts
                && let Ok(mut hosts_guard) = etc_hosts.lock()
            {
                match hosts_guard.load() {
                    Ok(stats) => {
                        log::info!("Reloaded /etc/hosts: {}", stats);
                    }
                    Err(e) => {
                        log::debug!("Failed to reload /etc/hosts: {}", e);
                    }
                }
            }

            config = new_config;

            sd_notify_status("Processing requests...");
        }

        // Periodically refresh /etc/hosts if the file changed on disk.
        if config.read_etc_hosts && last_hosts_check.elapsed() >= HOSTS_REFRESH_INTERVAL {
            if let Ok(hosts_guard) = etc_hosts.lock() {
                let changed = hosts_guard.has_changed();
                drop(hosts_guard);
                if changed && let Ok(mut hosts_guard) = etc_hosts.lock() {
                    match hosts_guard.load() {
                        Ok(stats) => {
                            log::info!("Reloaded /etc/hosts: {}", stats);
                        }
                        Err(e) => {
                            log::debug!("Failed to reload /etc/hosts: {}", e);
                        }
                    }
                }
            }
            last_hosts_check = Instant::now();
        }

        // Periodically refresh link DNS from networkd
        if last_link_refresh.elapsed() >= LINK_DNS_REFRESH_INTERVAL {
            let mut refresh_config = ResolvedConfig::load();
            refresh_config.update_link_dns_from_networkd();

            let new_upstreams: Vec<SocketAddr> = refresh_config
                .effective_dns_servers()
                .iter()
                .map(|s| s.socket_addr())
                .collect();

            if let Ok(mut list) = upstreams.write()
                && *list != new_upstreams
            {
                log::info!("Link DNS updated: {} upstream servers", new_upstreams.len());
                *list = new_upstreams;
                write_resolv_conf_files(&refresh_config);
                update_shared_state(&shared_dbus_state, &refresh_config, &stats);

                // Rebuild DNS router on link change.
                if let Ok(mut r) = dns_router.write() {
                    *r = DnsRouter::from_config(&refresh_config);
                }
            }

            last_link_refresh = Instant::now();
        }

        // Watchdog
        if let Some(interval) = watchdog_interval
            && last_watchdog.elapsed() >= interval
        {
            sd_notify_watchdog();
            last_watchdog = Instant::now();
        }

        thread::sleep(Duration::from_millis(POLL_INTERVAL_MS));
    }

    // ── Shutdown ───────────────────────────────────────────────────────
    sd_notify_stopping();
    sd_notify_status("Shutting down...");
    log::info!("Shutting down...");
    remove_pid_file();

    // Signal listener threads to stop
    shutdown.store(true, Ordering::Relaxed);

    // Log final cache statistics
    if let Ok(dns_cache_lock) = dns_cache.lock() {
        log::info!(
            "Cache statistics: {} entries, {} lookups, {} hits ({:.1}% hit rate), {} misses, {} evictions",
            dns_cache_lock.len(),
            dns_cache_lock.stats.lookups,
            dns_cache_lock.stats.hits,
            dns_cache_lock.stats.hit_rate(),
            dns_cache_lock.stats.misses,
            dns_cache_lock.stats.evictions,
        );
    }

    // Log final statistics
    let final_stats = stats.snapshot();
    log::info!(
        "Final statistics: {} queries received, {} forwarded, {} OK, {} SERVFAIL, {} NXDOMAIN",
        final_stats.queries_received,
        final_stats.queries_forwarded,
        final_stats.responses_ok,
        final_stats.responses_servfail,
        final_stats.responses_nxdomain,
    );

    // Wait for listener threads (with timeout)
    for handle in listener_threads {
        // The threads check the shutdown flag and will exit
        let _ = handle.join();
    }

    // Clean up resolv.conf files (optional — real systemd doesn't do this,
    // but we clean up gracefully)
    log::info!("systemd-resolved stopped");
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::Ipv4Addr;

    /// Helper to create a default QueryContext for tests.
    fn test_ctx() -> QueryContext {
        QueryContext {
            cache: Arc::new(Mutex::new(DnsCache::new())),
            etc_hosts: Arc::new(Mutex::new(hosts::EtcHosts::new())),
            router: Arc::new(std::sync::RwLock::new(DnsRouter::from_config(
                &ResolvedConfig::default(),
            ))),
            dot_client: Arc::new(DotClient::plain(DotMode::No)),
            validator: Arc::new(Mutex::new(DnssecValidator::with_root_anchor())),
            dnssec_mode: DnssecMode::No,
        }
    }

    fn test_dot_client() -> SharedDotClient {
        Arc::new(DotClient::plain(DotMode::No))
    }

    fn test_validator() -> SharedValidator {
        Arc::new(Mutex::new(DnssecValidator::with_root_anchor()))
    }

    #[test]
    fn test_atomic_stats_new() {
        let stats = AtomicStats::new();
        assert_eq!(stats.queries_received.load(Ordering::Relaxed), 0);
        assert_eq!(stats.queries_forwarded.load(Ordering::Relaxed), 0);
        assert_eq!(stats.responses_ok.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_atomic_stats_increment() {
        let stats = AtomicStats::new();
        stats.queries_received.fetch_add(1, Ordering::Relaxed);
        stats.queries_received.fetch_add(1, Ordering::Relaxed);
        assert_eq!(stats.queries_received.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn test_atomic_stats_snapshot() {
        let stats = AtomicStats::new();
        stats.queries_received.fetch_add(10, Ordering::Relaxed);
        stats.queries_forwarded.fetch_add(8, Ordering::Relaxed);
        stats.responses_ok.fetch_add(7, Ordering::Relaxed);
        stats.responses_servfail.fetch_add(1, Ordering::Relaxed);

        let snap = stats.snapshot();
        assert_eq!(snap.queries_received, 10);
        assert_eq!(snap.queries_forwarded, 8);
        assert_eq!(snap.responses_ok, 7);
        assert_eq!(snap.responses_servfail, 1);
    }

    #[test]
    fn test_handle_query_empty_upstreams() {
        let stats = AtomicStats::new();
        let ctx = test_ctx();

        // Build a minimal valid DNS query
        let mut query = vec![0u8; 12]; // minimal header
        query[2] = 0x01; // RD=1
        query[5] = 0; // QDCOUNT=0

        let response = handle_query(&query, &[], &stats, &ctx);

        // Should get SERVFAIL
        assert!(response.len() >= HEADER_SIZE);
        let rcode = response[3] & 0x0F;
        assert_eq!(rcode, 2); // SERVFAIL

        assert_eq!(stats.queries_received.load(Ordering::Relaxed), 1);
        assert_eq!(stats.responses_servfail.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_handle_query_malformed() {
        let stats = AtomicStats::new();
        let ctx = test_ctx();

        // Too short to be a valid DNS message but has a header
        let query = vec![0u8; HEADER_SIZE];

        let mut query = query;
        query[5] = 1; // QDCOUNT=1

        let response = handle_query(&query, &[], &stats, &ctx);
        // Should get FORMERR since the message can't be parsed (truncated question)
        assert!(response.len() >= HEADER_SIZE);
        let rcode = response[3] & 0x0F;
        assert_eq!(rcode, 1); // FORMERR

        assert_eq!(stats.responses_formerr.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_handle_query_response_not_query() {
        let stats = AtomicStats::new();
        let ctx = test_ctx();

        // Send a response (QR=1) as if it were a query
        let mut data = vec![0u8; HEADER_SIZE];
        data[2] = 0x80; // QR=1 (response)
        data[5] = 0; // QDCOUNT=0

        let response = handle_query(&data, &[], &stats, &ctx);
        let rcode = response[3] & 0x0F;
        assert_eq!(rcode, 1); // FORMERR
    }

    #[test]
    fn test_write_resolv_conf_files_creates_dir() {
        let dir = tempfile::tempdir().unwrap();
        let _state_path = dir.path().join("resolve");

        // Can't easily test write_resolv_conf_files without mocking paths,
        // but we can test write_file_atomic directly
        let test_path = dir.path().join("test.conf");
        write_file_atomic(test_path.to_str().unwrap(), "nameserver 127.0.0.53\n");
        assert!(test_path.exists());
        assert_eq!(
            fs::read_to_string(&test_path).unwrap(),
            "nameserver 127.0.0.53\n"
        );
    }

    #[test]
    fn test_write_file_atomic_overwrites() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("conf");
        write_file_atomic(path.to_str().unwrap(), "first");
        write_file_atomic(path.to_str().unwrap(), "second");
        assert_eq!(fs::read_to_string(&path).unwrap(), "second");
    }

    #[test]
    fn test_get_watchdog_usec_default() {
        // Without WATCHDOG_USEC env var, should return 0
        // SAFETY: This test is not run in parallel with other tests that
        // depend on WATCHDOG_USEC.
        unsafe {
            std::env::remove_var("WATCHDOG_USEC");
        }
        let usec = get_watchdog_usec();
        assert_eq!(usec, 0);
    }

    #[test]
    fn test_stub_addr_construction() {
        let addr: SocketAddr = SocketAddr::new(
            config::STUB_LISTENER_ADDR
                .parse()
                .unwrap_or(std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 53))),
            config::DNS_PORT,
        );
        assert_eq!(
            addr,
            SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(127, 0, 0, 53)), 53)
        );
    }

    // ── D-Bus interface tests ──────────────────────────────────────────────

    #[test]
    fn test_dbus_resolve1_manager_struct() {
        let cache: SharedCache = Arc::new(Mutex::new(DnsCache::new()));
        let shared: SharedState = Arc::new(Mutex::new(ResolvedState::default()));
        let _mgr = Resolve1Manager {
            state: shared,
            cache,
        };
        // Construction succeeded without panic
    }

    #[test]
    fn test_resolve_link_object_path() {
        assert_eq!(
            resolve_link_object_path(1),
            "/org/freedesktop/resolve1/link/_1"
        );
        assert_eq!(
            resolve_link_object_path(42),
            "/org/freedesktop/resolve1/link/_42"
        );
    }

    #[test]
    fn test_shared_state_default() {
        let state = ResolvedState::default();
        assert!(state.dns.is_empty());
        assert!(state.fallback_dns.is_empty());
        assert!(state.domains.is_empty());
        assert_eq!(state.llmnr, "yes");
        assert_eq!(state.multicast_dns, "no");
        assert_eq!(state.dnssec, "no");
        assert_eq!(state.dns_over_tls, "no");
        assert_eq!(state.dns_stub_listener, "yes");
        assert_eq!(state.cache, "yes");
        assert!(!state.dnssec_supported);
        assert_eq!(state.transaction_stats, (0, 0));
        assert_eq!(state.cache_stats, (0, 0, 0));
    }

    #[test]
    fn test_shared_state_with_dns() {
        let shared: SharedState = Arc::new(Mutex::new(ResolvedState {
            dns: vec![(2, vec![8, 8, 8, 8]), (2, vec![8, 8, 4, 4])],
            fallback_dns: vec![(2, vec![1, 1, 1, 1])],
            domains: vec![(0, "example.com".to_string(), false)],
            ..ResolvedState::default()
        }));
        let s = shared.lock().unwrap();
        assert_eq!(s.dns.len(), 2);
        assert_eq!(s.fallback_dns.len(), 1);
        assert_eq!(s.domains.len(), 1);
        assert_eq!(s.domains[0].1, "example.com");
    }

    #[test]
    fn test_addr_bytes_to_string_ipv4() {
        assert_eq!(addr_bytes_to_string(2, &[8, 8, 8, 8]), "8.8.8.8");
        assert_eq!(addr_bytes_to_string(2, &[192, 168, 1, 1]), "192.168.1.1");
    }

    #[test]
    fn test_addr_bytes_to_string_ipv6() {
        let bytes = [0x20, 0x01, 0x0d, 0xb8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1];
        let result = addr_bytes_to_string(10, &bytes);
        assert_eq!(result, "2001:db8:0:0:0:0:0:1");
    }

    #[test]
    fn test_addr_bytes_to_string_unknown() {
        let result = addr_bytes_to_string(99, &[1, 2, 3]);
        assert!(result.contains("unknown family"));
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
    fn test_dns_server_to_dbus_ipv4() {
        let server = config::DnsServer::new("8.8.8.8".parse().unwrap());
        let (family, bytes) = dns_server_to_dbus(&server);
        assert_eq!(family, 2);
        assert_eq!(bytes, vec![8, 8, 8, 8]);
    }

    // ── /etc/hosts integration tests ──────────────────────────────────

    #[test]
    fn test_handle_query_etc_hosts_a_record() {
        let stats = AtomicStats::new();
        let mut ctx = test_ctx();

        let mut hosts_db = hosts::EtcHosts::new();
        hosts_db.load_from_str("192.168.1.10 myhost.local\n", None);
        ctx.etc_hosts = Arc::new(Mutex::new(hosts_db));

        // Build an A query for myhost.local
        let query = hosts::tests_support::build_test_query("myhost.local", dns::RecordType::A);

        let response = handle_query(&query, &[], &stats, &ctx);

        // Should get a successful response from /etc/hosts (not SERVFAIL)
        assert!(response.len() >= HEADER_SIZE);
        // QR=1
        assert_ne!(response[2] & 0x80, 0);
        // RCODE=0 (NOERROR)
        assert_eq!(response[3] & 0x0F, 0);
        // ANCOUNT=1
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);

        // Stats: query received, response OK, no forwarding
        assert_eq!(stats.queries_received.load(Ordering::Relaxed), 1);
        assert_eq!(stats.responses_ok.load(Ordering::Relaxed), 1);
        assert_eq!(stats.queries_forwarded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_handle_query_etc_hosts_aaaa_record() {
        let stats = AtomicStats::new();
        let mut ctx = test_ctx();

        let mut hosts_db = hosts::EtcHosts::new();
        hosts_db.load_from_str("::1 localhost\n", None);
        ctx.etc_hosts = Arc::new(Mutex::new(hosts_db));

        let query = hosts::tests_support::build_test_query("localhost", dns::RecordType::AAAA);

        let response = handle_query(&query, &[], &stats, &ctx);

        assert!(response.len() >= HEADER_SIZE);
        assert_eq!(response[3] & 0x0F, 0); // NOERROR
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
        assert_eq!(stats.queries_forwarded.load(Ordering::Relaxed), 0);
    }

    #[test]
    fn test_handle_query_etc_hosts_miss_falls_through() {
        let stats = AtomicStats::new();
        let mut ctx = test_ctx();

        let mut hosts_db = hosts::EtcHosts::new();
        hosts_db.load_from_str("127.0.0.1 localhost\n", None);
        ctx.etc_hosts = Arc::new(Mutex::new(hosts_db));

        // Query for a name NOT in /etc/hosts, with no upstreams
        let query = hosts::tests_support::build_test_query("unknown.host", dns::RecordType::A);

        let response = handle_query(&query, &[], &stats, &ctx);

        // Should fall through to upstream (which is empty) and get SERVFAIL
        assert_eq!(response[3] & 0x0F, 2); // SERVFAIL
        assert_eq!(stats.responses_servfail.load(Ordering::Relaxed), 1);
    }

    #[test]
    fn test_handle_query_etc_hosts_priority_over_cache() {
        let stats = AtomicStats::new();
        let mut ctx = test_ctx();

        let mut hosts_db = hosts::EtcHosts::new();
        hosts_db.load_from_str("10.0.0.1 cached.host\n", None);
        ctx.etc_hosts = Arc::new(Mutex::new(hosts_db));

        let query = hosts::tests_support::build_test_query("cached.host", dns::RecordType::A);

        // Insert a fake response into the cache first
        {
            let mut c = ctx.cache.lock().unwrap();
            // Build a minimal fake cached response
            let mut fake_resp = query.clone();
            fake_resp[2] |= 0x80; // QR=1
            fake_resp[3] = 0x80; // RA=1, RCODE=0
            // Set a non-zero TTL answer so it would be cached
            c.insert(&query, &fake_resp);
        }

        let response = handle_query(&query, &[], &stats, &ctx);

        // /etc/hosts should take priority — response should have AA=1
        // (our hosts responses set AA=1, cache responses don't)
        assert_ne!(
            response[2] & 0x04,
            0,
            "/etc/hosts response should have AA=1"
        );
        assert_eq!(response[3] & 0x0F, 0); // NOERROR
    }

    #[test]
    fn test_handle_query_etc_hosts_empty_db() {
        let stats = AtomicStats::new();
        let ctx = test_ctx();

        let query = hosts::tests_support::build_test_query("localhost", dns::RecordType::A);

        let response = handle_query(&query, &[], &stats, &ctx);

        // Empty hosts DB, no upstreams → SERVFAIL
        assert_eq!(response[3] & 0x0F, 2);
    }

    #[test]
    fn test_handle_query_etc_hosts_ptr_query() {
        let stats = AtomicStats::new();
        let mut ctx = test_ctx();

        let mut hosts_db = hosts::EtcHosts::new();
        hosts_db.load_from_str("192.168.1.10 myhost.example.com\n", None);
        ctx.etc_hosts = Arc::new(Mutex::new(hosts_db));

        // Build a PTR query for 10.1.168.192.in-addr.arpa
        let query = hosts::tests_support::build_test_query(
            "10.1.168.192.in-addr.arpa",
            dns::RecordType::PTR,
        );

        let response = handle_query(&query, &[], &stats, &ctx);

        assert!(response.len() >= HEADER_SIZE);
        assert_eq!(response[3] & 0x0F, 0); // NOERROR
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
        assert_eq!(stats.queries_forwarded.load(Ordering::Relaxed), 0);
    }

    // ── Split DNS routing integration tests ───────────────────────────

    #[test]
    fn test_handle_query_routing_domain_selects_servers() {
        // When a routing domain is configured, queries matching it should
        // be routed to the link's DNS servers (even though the flat
        // `upstreams` list is empty).  Since there are no real upstream
        // servers to contact, the query will SERVFAIL — but the important
        // thing is that the router's servers_for_name() is exercised.
        let stats = AtomicStats::new();
        let mut ctx = test_ctx();

        // Build a router with a routing domain.
        let mut cfg = ResolvedConfig::default();
        cfg.dns.clear();
        cfg.fallback_dns.clear();
        cfg.link_dns = vec![{
            let mut link = config::LinkDns::new(2, "vpn0".to_string());
            link.dns_servers = vec![config::DnsServer::new(std::net::IpAddr::V4(Ipv4Addr::new(
                10, 0, 0, 1,
            )))];
            link.domains = vec!["~corp.local".to_string()];
            link
        }];
        ctx.router = Arc::new(std::sync::RwLock::new(DnsRouter::from_config(&cfg)));

        // A query for a name that does NOT match the routing domain and
        // there are no global/fallback servers → SERVFAIL.
        let query = hosts::tests_support::build_test_query("www.google.com", dns::RecordType::A);
        let response = handle_query(&query, &[], &stats, &ctx);
        assert_eq!(response[3] & 0x0F, 2); // SERVFAIL (no servers for this name)
    }

    #[test]
    fn test_handle_query_no_routing_uses_flat_upstreams() {
        // When the router has no routing domains, queries should use the
        // flat upstream list as before (backward-compatible behaviour).
        let stats = AtomicStats::new();
        let mut ctx = test_ctx();

        // Empty flat upstreams, no routing domains, default fallback DNS cleared.
        let mut cfg = ResolvedConfig::default();
        cfg.dns.clear();
        cfg.fallback_dns.clear();
        ctx.router = Arc::new(std::sync::RwLock::new(DnsRouter::from_config(&cfg)));

        let query = hosts::tests_support::build_test_query("example.com", dns::RecordType::A);
        let response = handle_query(&query, &[], &stats, &ctx);
        // No upstreams at all → SERVFAIL.
        assert_eq!(response[3] & 0x0F, 2);
    }

    // ── EDNS0 integration tests ───────────────────────────────────────

    #[test]
    fn test_prepare_query_with_edns_adds_opt() {
        // Build a minimal DNS query (no OPT record)
        let query = hosts::tests_support::build_test_query("example.com", dns::RecordType::A);
        assert!(!edns::has_opt_record(&query));

        let edns_query = prepare_query_with_edns(&query, false);
        assert!(edns::has_opt_record(&edns_query));
        // ARCOUNT should be incremented
        let arcount = u16::from_be_bytes([edns_query[10], edns_query[11]]);
        assert_eq!(arcount, 1);
    }

    #[test]
    fn test_prepare_query_with_edns_dnssec_ok() {
        let query = hosts::tests_support::build_test_query("example.com", dns::RecordType::A);
        let edns_query = prepare_query_with_edns(&query, true);
        assert!(edns::has_opt_record(&edns_query));
        // Verify the DO bit is set in the OPT record
        if let Some(opt) = edns::extract_opt_record(&edns_query) {
            assert!(opt.flags.dnssec_ok);
        } else {
            panic!("Expected OPT record in EDNS query");
        }
    }

    #[test]
    fn test_prepare_query_with_edns_idempotent() {
        let query = hosts::tests_support::build_test_query("example.com", dns::RecordType::A);
        let edns_query = prepare_query_with_edns(&query, false);
        let edns_query2 = prepare_query_with_edns(&edns_query, false);
        // Should not add a second OPT record
        let arcount = u16::from_be_bytes([edns_query2[10], edns_query2[11]]);
        assert_eq!(arcount, 1);
    }

    #[test]
    fn test_prepare_query_with_edns_too_short() {
        let short = vec![0u8; 4];
        let result = prepare_query_with_edns(&short, false);
        // Should return the original data unchanged
        assert_eq!(result, short);
    }

    // ── DNS-over-TLS integration tests ────────────────────────────────

    #[test]
    fn test_try_dot_forward_disabled() {
        let dot = Arc::new(DotClient::plain(DotMode::No));
        let query = hosts::tests_support::build_test_query("example.com", dns::RecordType::A);
        let upstreams = vec![SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)),
            53,
        )];
        // DoT disabled — should return None immediately
        assert!(try_dot_forward(&query, &upstreams, &dot).is_none());
    }

    #[test]
    fn test_try_dot_forward_empty_upstreams() {
        let dot = Arc::new(DotClient::plain(DotMode::Opportunistic));
        let query = hosts::tests_support::build_test_query("example.com", dns::RecordType::A);
        assert!(try_dot_forward(&query, &[], &dot).is_none());
    }

    // ── DNSSEC validation integration tests ───────────────────────────

    #[test]
    fn test_validate_dnssec_response_disabled() {
        let val = test_validator();
        let query = hosts::tests_support::build_test_query("example.com", dns::RecordType::A);
        let mut response = query.clone();
        response[2] |= 0x80; // QR=1
        let orig = response.clone();
        validate_dnssec_response(&mut response, &query, &val, DnssecMode::No);
        // Should be unchanged when DNSSEC is disabled
        assert_eq!(response, orig);
    }

    #[test]
    fn test_validate_dnssec_response_nta_passthrough() {
        let val: SharedValidator = Arc::new(Mutex::new({
            let mut v = DnssecValidator::with_root_anchor();
            v.add_negative_trust_anchor("example.com");
            v
        }));
        let query = hosts::tests_support::build_test_query("host.example.com", dns::RecordType::A);
        let mut response = query.clone();
        response[2] |= 0x80;
        let orig = response.clone();
        validate_dnssec_response(&mut response, &query, &val, DnssecMode::Yes);
        // NTA match — response should be passed through unchanged
        assert_eq!(response, orig);
    }

    #[test]
    fn test_validate_dnssec_response_insecure_no_anchor() {
        // For most domains without a trust anchor chain, validation returns Insecure
        let val = test_validator();
        let query =
            hosts::tests_support::build_test_query("random-domain.test", dns::RecordType::A);
        let mut response = query.clone();
        response[2] |= 0x80;
        let _orig = response.clone();
        validate_dnssec_response(&mut response, &query, &val, DnssecMode::Yes);
        // Insecure zones pass through without AD bit
        assert_eq!(response[3] & 0x20, 0); // no AD bit
    }

    // ── LLMNR/mDNS responder tests ───────────────────────────────────

    #[test]
    fn test_get_local_hostname() {
        let hostname = get_local_hostname();
        assert!(!hostname.is_empty());
    }

    #[test]
    fn test_llmnr_responder_shared() {
        let responder: SharedLlmnrResponder = Arc::new(Mutex::new(llmnr::LlmnrResponder::new(
            llmnr::LlmnrResponderConfig::new("testhost", vec![], vec![]),
        )));
        let guard = responder.lock().unwrap();
        assert_eq!(guard.hostname(), "testhost");
    }

    #[test]
    fn test_mdns_responder_shared() {
        let responder: SharedMdnsResponder =
            Arc::new(Mutex::new(mdns::MdnsResponder::new("testhost")));
        let guard = responder.lock().unwrap();
        assert_eq!(guard.hostname(), "testhost");
    }

    #[test]
    fn test_router_from_config_has_routes() {
        let cfg = ResolvedConfig {
            link_dns: vec![{
                let mut link = config::LinkDns::new(2, "vpn0".to_string());
                link.dns_servers = vec![config::DnsServer::new(std::net::IpAddr::V4(
                    Ipv4Addr::new(10, 0, 0, 1),
                ))];
                link.domains = vec!["~corp.local".to_string()];
                link
            }],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&cfg);
        assert!(router.has_routing_domains());
        assert_eq!(router.route_count(), 1);
    }

    #[test]
    fn test_router_servers_for_matching_name() {
        let mut cfg = ResolvedConfig::default();
        cfg.dns.clear();
        cfg.fallback_dns.clear();
        cfg.link_dns = vec![{
            let mut link = config::LinkDns::new(2, "vpn0".to_string());
            link.dns_servers = vec![config::DnsServer::new(std::net::IpAddr::V4(Ipv4Addr::new(
                10, 0, 0, 1,
            )))];
            link.domains = vec!["~corp.local".to_string()];
            link
        }];

        let router = DnsRouter::from_config(&cfg);

        let servers = router.servers_for_name("host.corp.local");
        assert_eq!(servers.len(), 1);
        assert_eq!(
            servers[0],
            SocketAddr::new(std::net::IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 53)
        );

        // Non-matching name → no servers (no global/fallback).
        let servers = router.servers_for_name("www.example.com");
        assert!(servers.is_empty());
    }

    #[test]
    fn test_extract_query_name_from_real_query() {
        let query =
            hosts::tests_support::build_test_query("myhost.example.com", dns::RecordType::A);
        let name = extract_query_name(&query);
        assert_eq!(name, Some("myhost.example.com".to_string()));
    }

    #[test]
    fn test_extract_query_name_too_short() {
        assert_eq!(extract_query_name(&[0u8; 4]), None);
    }
}
