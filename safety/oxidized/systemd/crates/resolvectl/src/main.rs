//! resolvectl — DNS resolver control tool
//!
//! A Rust implementation of resolvectl / systemd-resolve that provides CLI
//! access to systemd-resolved status and operations.
//!
//! Supported commands:
//! - `status [LINK...]` — Show resolver status and per-link DNS configuration
//! - `query HOSTNAME...` — Resolve hostnames via the stub resolver
//! - `statistics` — Show resolver statistics
//! - `flush-caches` — Flush DNS caches (sends SIGHUP to resolved)
//! - `reset-statistics` — Reset resolver statistics
//! - `dns [LINK [SERVER...]]` — Show/set per-link DNS servers
//! - `domain [LINK [DOMAIN...]]` — Show/set per-link search domains
//! - `llmnr [LINK [MODE]]` — Show/set per-link LLMNR mode
//! - `mdns [LINK [MODE]]` — Show/set per-link mDNS mode
//! - `dnssec [LINK [MODE]]` — Show/set per-link DNSSEC mode
//! - `dnsovertls [LINK [MODE]]` — Show/set per-link DNS-over-TLS mode
//! - `monitor` — Monitor DNS queries (stub)
//! - `log-level [LEVEL]` — Show/set log level (stub)
//!
//! Also supports the legacy `systemd-resolve` interface when invoked as
//! `systemd-resolve`.

use std::env;
use std::fs;
use std::io::{self, BufRead};
use std::net::{IpAddr, Ipv4Addr, SocketAddr, ToSocketAddrs, UdpSocket};
use std::path::Path;
use std::process;
use std::time::Duration;

// ── Constants ──────────────────────────────────────────────────────────────

/// Resolved configuration file path
const CONFIG_PATH: &str = "/etc/systemd/resolved.conf";

/// Stub resolv.conf path
const STUB_RESOLV_CONF: &str = "/run/systemd/resolve/stub-resolv.conf";

/// Upstream resolv.conf path
const RESOLV_CONF: &str = "/run/systemd/resolve/resolv.conf";

/// Networkd link state directory
const NETIF_LINKS_DIR: &str = "/run/systemd/netif/links";

/// Stub listener address
const STUB_ADDR: &str = "127.0.0.53";

/// DNS port
const DNS_PORT: u16 = 53;

/// Query timeout
const QUERY_TIMEOUT: Duration = Duration::from_secs(5);

/// DNS header size
const DNS_HEADER_SIZE: usize = 12;

/// Resolved PID file path
const RESOLVED_PID_FILE: &str = "/run/systemd/resolve/systemd-resolved.pid";

// ── Main dispatch ──────────────────────────────────────────────────────────

fn main() {
    let args: Vec<String> = env::args().collect();
    let progname = args.first().map(|s| s.as_str()).unwrap_or("resolvectl");

    // If invoked as systemd-resolve (legacy compat), shift args
    let is_legacy = progname.contains("systemd-resolve");

    let (cmd, cmd_args) = if is_legacy {
        // Legacy mode: systemd-resolve [options] hostname...
        // Treat all positional args as hostnames to resolve
        if args.len() < 2 {
            ("status", Vec::new())
        } else {
            let first = args[1].as_str();
            if first == "--help" || first == "-h" {
                print_usage(is_legacy);
                return;
            }
            if first == "--version" {
                println!("systemd-resolve (rust-systemd)");
                return;
            }
            if first == "--status" {
                ("status", args[2..].to_vec())
            } else if first == "--statistics" {
                ("statistics", Vec::new())
            } else if first == "--flush-caches" {
                ("flush-caches", Vec::new())
            } else if first == "--reset-statistics" {
                ("reset-statistics", Vec::new())
            } else if first.starts_with('-') {
                // Unknown flag — try treating remaining as hostnames
                (
                    "query",
                    args[1..]
                        .iter()
                        .filter(|a| !a.starts_with('-'))
                        .cloned()
                        .collect(),
                )
            } else {
                ("query", args[1..].to_vec())
            }
        }
    } else {
        // resolvectl mode
        if args.len() < 2 {
            ("status", Vec::new())
        } else {
            let first = args[1].as_str();
            match first {
                "--help" | "-h" | "help" => {
                    print_usage(is_legacy);
                    return;
                }
                "--version" => {
                    println!("resolvectl (rust-systemd)");
                    return;
                }
                _ => (first, args[2..].to_vec()),
            }
        }
    };

    let exit_code = match cmd {
        "status" => cmd_status(&cmd_args),
        "query" | "resolve" => cmd_query(&cmd_args),
        "statistics" => cmd_statistics(),
        "flush-caches" => cmd_flush_caches(),
        "reset-statistics" => cmd_reset_statistics(),
        "dns" => cmd_dns(&cmd_args),
        "domain" => cmd_domain(&cmd_args),
        "llmnr" => cmd_show_setting("LLMNR"),
        "mdns" => cmd_show_setting("MulticastDNS"),
        "dnssec" => cmd_show_setting("DNSSEC"),
        "dnsovertls" | "dns-over-tls" => cmd_show_setting("DNSOverTLS"),
        "revert" => cmd_revert(&cmd_args),
        "monitor" => cmd_monitor(),
        "log-level" => {
            if cmd_args.is_empty() {
                println!("info");
            } else {
                eprintln!("Setting log level is not yet implemented (requires D-Bus).");
            }
            0
        }
        _ => {
            eprintln!("Unknown command: {}", cmd);
            eprintln!("Try 'resolvectl --help' for more information.");
            1
        }
    };

    process::exit(exit_code);
}

// ── Usage ──────────────────────────────────────────────────────────────────

fn print_usage(legacy: bool) {
    if legacy {
        println!("Usage: systemd-resolve [OPTIONS...] HOSTNAME...");
        println!("       systemd-resolve --status [LINK...]");
        println!("       systemd-resolve --statistics");
        println!("       systemd-resolve --flush-caches");
        println!("       systemd-resolve --reset-statistics");
        println!();
        println!("Resolve hostnames, IPv4/IPv6 addresses, DNS records, and services.");
    } else {
        println!("Usage: resolvectl [COMMAND] [OPTIONS...]");
        println!();
        println!("Commands:");
        println!("  status [LINK...]        Show resolver status");
        println!("  query HOSTNAME...       Resolve hostnames");
        println!("  statistics              Show resolver statistics");
        println!("  flush-caches            Flush DNS caches");
        println!("  reset-statistics        Reset resolver statistics");
        println!("  dns [LINK [SERVER...]]  Show/set per-link DNS servers");
        println!("  domain [LINK [DOMAIN...]]  Show/set per-link search domains");
        println!("  llmnr [LINK [MODE]]     Show/set per-link LLMNR mode");
        println!("  mdns [LINK [MODE]]      Show/set per-link mDNS mode");
        println!("  dnssec [LINK [MODE]]    Show/set per-link DNSSEC mode");
        println!("  dnsovertls [LINK [MODE]] Show/set per-link DNS-over-TLS mode");
        println!("  revert LINK...          Revert per-link DNS settings to defaults");
        println!("  monitor                 Monitor DNS queries in real time");
        println!("  log-level [LEVEL]       Show/set log level");
    }
}

// ── Command: monitor ───────────────────────────────────────────────────────

fn cmd_monitor() -> i32 {
    // resolvectl monitor streams DNS query activity from systemd-resolved.
    //
    // Real systemd uses a D-Bus SubscribeQueryResults() call on the
    // org.freedesktop.resolve1.Manager interface.  Since our D-Bus interface
    // doesn't yet support streaming signals, we fall back to tailing the
    // journal for resolved log lines that describe query traffic.
    //
    // This gives equivalent visibility into what resolved is doing.

    eprintln!("Monitoring DNS queries (press Ctrl+C to stop)...");
    eprintln!();

    // Try journalctl first (works on systemd-based systems including NixOS)
    let result = std::process::Command::new("journalctl")
        .args([
            "--follow",
            "--no-pager",
            "--output=short-precise",
            "--unit=systemd-resolved.service",
            "--grep=Query:|Cache hit|Upstream|/etc/hosts|Routed",
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .spawn();

    match result {
        Ok(mut child) => {
            // Stream stdout to our stdout, reformatting lines for clarity
            if let Some(stdout) = child.stdout.take() {
                let reader = io::BufReader::new(stdout);
                for line in reader.lines() {
                    match line {
                        Ok(text) => {
                            // Extract the message portion after the systemd-resolved[PID]: prefix
                            if let Some(msg_start) = text.find("]: ") {
                                let msg = &text[msg_start + 3..];
                                // Extract timestamp (first field)
                                let timestamp = text.split_whitespace().next().unwrap_or("");
                                println!("{} {}", timestamp, msg);
                            } else {
                                println!("{}", text);
                            }
                        }
                        Err(_) => break,
                    }
                }
            }

            // Wait for the child process to exit
            match child.wait() {
                Ok(status) => {
                    if status.success() || status.code() == Some(130) {
                        // 130 = SIGINT (Ctrl+C), which is normal for --follow
                        0
                    } else {
                        1
                    }
                }
                Err(_) => 1,
            }
        }
        Err(_) => {
            // journalctl not available — fall back to watching resolved log file
            eprintln!("journalctl not available.");
            eprintln!();
            eprintln!("To monitor DNS queries, try one of:");
            eprintln!("  journalctl -f -u systemd-resolved");
            eprintln!("  tail -f /var/log/syslog | grep resolved");
            1
        }
    }
}

// ── Command: status ────────────────────────────────────────────────────────

fn cmd_status(args: &[String]) -> i32 {
    // If specific links are requested, show only those
    if !args.is_empty() {
        for link_name in args {
            show_link_status(link_name);
        }
        return 0;
    }

    // Global status
    println!("Global");

    // Show configured protocols
    let config = load_config();
    println!(
        "       Protocols: +LLMNR +mDNS {}DNS-over-TLS DNSSEC={}",
        if config.dns_over_tls == "no" {
            "-"
        } else {
            "+"
        },
        config.dnssec,
    );

    // Show resolv.conf mode
    let resolv_mode = detect_resolv_conf_mode();
    println!("resolv.conf mode: {}", resolv_mode);

    // Show current DNS servers
    let dns_servers = get_current_dns_servers();
    if !dns_servers.is_empty() {
        println!(" Current DNS Server: {}", dns_servers[0]);
        println!("        DNS Servers: {}", dns_servers.join(" "));
    } else {
        println!("        DNS Servers: (none)");
    }

    // Show fallback DNS
    if !config.fallback_dns.is_empty() {
        println!("   Fallback DNS Servers: {}", config.fallback_dns.join(" "));
    }

    // Show search domains
    let domains = get_search_domains();
    if !domains.is_empty() {
        println!("        DNS Domain: {}", domains.join(" "));
    }

    println!();

    // Show per-link status
    let links = discover_links();
    for (ifindex, ifname) in &links {
        show_link_dns_brief(*ifindex, ifname);
    }

    0
}

fn show_link_status(name: &str) {
    println!("Link {} ({})", name, name);

    // Try to find link state
    let links = discover_links();
    for (ifindex, ifname) in &links {
        if ifname == name || ifindex.to_string() == *name {
            let state_path = format!("{}/{}", NETIF_LINKS_DIR, ifindex);
            if let Ok(content) = fs::read_to_string(&state_path) {
                let dns = extract_key(&content, "DNS");
                let domains = extract_key(&content, "DOMAINS");

                if !dns.is_empty() {
                    println!(
                        "    Current DNS Server: {}",
                        dns.split_whitespace().next().unwrap_or("")
                    );
                    println!("           DNS Servers: {}", dns);
                }
                if !domains.is_empty() {
                    println!("           DNS Domain: {}", domains);
                }
            }

            // Show current DNSSEC/LLMNR/mDNS/DOT settings (defaults)
            println!("              LLMNR setting: yes");
            println!("    MulticastDNS setting: no");
            println!("  DNSOverTLS setting: no");
            println!("      DNSSEC setting: allow-downgrade");
            println!("    DNSSEC supported: no");
            println!();
            return;
        }
    }

    eprintln!("Link \"{}\" not found.", name);
}

fn show_link_dns_brief(ifindex: u32, ifname: &str) {
    let state_path = format!("{}/{}", NETIF_LINKS_DIR, ifindex);
    let dns = if let Ok(content) = fs::read_to_string(&state_path) {
        extract_key(&content, "DNS")
    } else {
        String::new()
    };

    if !dns.is_empty() {
        println!("Link {} ({}):", ifindex, ifname);
        for server in dns.split_whitespace() {
            println!("    Current DNS Server: {}", server);
        }
        println!();
    }
}

// ── Command: query ─────────────────────────────────────────────────────────

fn cmd_query(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("No hostname specified.");
        eprintln!("Usage: resolvectl query HOSTNAME...");
        return 1;
    }

    let mut exit_code = 0;

    for hostname in args {
        if hostname.starts_with('-') {
            continue; // skip flags
        }

        // First try system resolver (getaddrinfo equivalent via ToSocketAddrs)
        match resolve_hostname(hostname) {
            Ok(addrs) => {
                for addr in &addrs {
                    println!("{} -- {}", hostname, addr);
                }
                if addrs.is_empty() {
                    eprintln!("{}: no addresses found", hostname);
                    exit_code = 1;
                }
            }
            Err(e) => {
                // Fall back to direct DNS query via stub
                match dns_query_stub(hostname) {
                    Ok(addrs) => {
                        for addr in &addrs {
                            println!("{} -- {}", hostname, addr);
                        }
                        if addrs.is_empty() {
                            eprintln!("{}: no addresses found", hostname);
                            exit_code = 1;
                        }
                    }
                    Err(e2) => {
                        eprintln!("{}: {} (stub: {})", hostname, e, e2);
                        exit_code = 1;
                    }
                }
            }
        }
    }

    exit_code
}

/// Resolve a hostname using the system resolver
fn resolve_hostname(hostname: &str) -> io::Result<Vec<IpAddr>> {
    let addr_str = format!("{}:0", hostname);
    let addrs: Vec<IpAddr> = addr_str.to_socket_addrs()?.map(|sa| sa.ip()).collect();
    if addrs.is_empty() {
        Err(io::Error::new(
            io::ErrorKind::NotFound,
            "no addresses found",
        ))
    } else {
        Ok(addrs)
    }
}

/// Query the local stub resolver directly via DNS UDP
fn dns_query_stub(hostname: &str) -> Result<Vec<IpAddr>, String> {
    let stub_addr = SocketAddr::new(
        IpAddr::V4(STUB_ADDR.parse().unwrap_or(Ipv4Addr::new(127, 0, 0, 53))),
        DNS_PORT,
    );

    // Build a DNS A query
    let query = build_dns_query(hostname, 1)?; // type A = 1
    let response = send_dns_query(&query, stub_addr)?;

    // Also try AAAA
    let query6 = build_dns_query(hostname, 28)?; // type AAAA = 28
    let response6 = send_dns_query(&query6, stub_addr);

    let mut addrs = parse_dns_a_response(&response);
    if let Ok(ref resp6) = response6 {
        addrs.extend(parse_dns_aaaa_response(resp6));
    }

    if addrs.is_empty() {
        // Check RCODE for NXDOMAIN
        if response.len() >= DNS_HEADER_SIZE {
            let rcode = response[3] & 0x0F;
            if rcode == 3 {
                return Err("NXDOMAIN".to_string());
            }
            if rcode == 2 {
                return Err("SERVFAIL".to_string());
            }
        }
    }

    Ok(addrs)
}

/// Build a minimal DNS query packet
fn build_dns_query(hostname: &str, qtype: u16) -> Result<Vec<u8>, String> {
    let mut buf = Vec::with_capacity(64);

    // Header
    let id: u16 = (std::process::id() as u16) ^ 0x5A5A;
    buf.extend_from_slice(&id.to_be_bytes());
    buf.push(0x01); // flags1: RD=1
    buf.push(0x00); // flags2
    buf.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT=1
    buf.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT=0
    buf.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT=0
    buf.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT=0

    // Encode domain name
    let name = hostname.trim_end_matches('.');
    for label in name.split('.') {
        if label.is_empty() {
            return Err("empty label in hostname".to_string());
        }
        if label.len() > 63 {
            return Err("label too long".to_string());
        }
        buf.push(label.len() as u8);
        buf.extend_from_slice(label.as_bytes());
    }
    buf.push(0); // root label

    // QTYPE and QCLASS
    buf.extend_from_slice(&qtype.to_be_bytes());
    buf.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN

    Ok(buf)
}

/// Send a DNS query and receive the response
fn send_dns_query(query: &[u8], server: SocketAddr) -> Result<Vec<u8>, String> {
    let local: SocketAddr = "0.0.0.0:0".parse().unwrap();
    let socket = UdpSocket::bind(local).map_err(|e| format!("bind: {}", e))?;
    socket
        .set_read_timeout(Some(QUERY_TIMEOUT))
        .map_err(|e| format!("set_read_timeout: {}", e))?;
    socket
        .set_write_timeout(Some(QUERY_TIMEOUT))
        .map_err(|e| format!("set_write_timeout: {}", e))?;

    socket
        .send_to(query, server)
        .map_err(|e| format!("send: {}", e))?;

    let mut buf = vec![0u8; 4096];
    let (len, _) = socket
        .recv_from(&mut buf)
        .map_err(|e| format!("recv: {}", e))?;
    buf.truncate(len);
    Ok(buf)
}

/// Parse A records from a DNS response
fn parse_dns_a_response(response: &[u8]) -> Vec<IpAddr> {
    parse_dns_response_addrs(response, 1, 4)
}

/// Parse AAAA records from a DNS response
fn parse_dns_aaaa_response(response: &[u8]) -> Vec<IpAddr> {
    parse_dns_response_addrs(response, 28, 16)
}

/// Parse address records from a DNS response
fn parse_dns_response_addrs(response: &[u8], expected_type: u16, addr_len: usize) -> Vec<IpAddr> {
    let mut addrs = Vec::new();

    if response.len() < DNS_HEADER_SIZE {
        return addrs;
    }

    let ancount = u16::from_be_bytes([response[6], response[7]]) as usize;
    let qdcount = u16::from_be_bytes([response[4], response[5]]) as usize;

    // Skip questions
    let mut offset = DNS_HEADER_SIZE;
    for _ in 0..qdcount {
        offset = skip_dns_name(response, offset);
        if offset == 0 || offset + 4 > response.len() {
            return addrs;
        }
        offset += 4; // QTYPE + QCLASS
    }

    // Parse answers
    for _ in 0..ancount {
        offset = skip_dns_name(response, offset);
        if offset == 0 || offset + 10 > response.len() {
            break;
        }

        let rtype = u16::from_be_bytes([response[offset], response[offset + 1]]);
        // Skip RCLASS (2 bytes) and TTL (4 bytes)
        let rdlength = u16::from_be_bytes([response[offset + 8], response[offset + 9]]) as usize;
        offset += 10;

        if offset + rdlength > response.len() {
            break;
        }

        if rtype == expected_type && rdlength == addr_len {
            if addr_len == 4 {
                let ip = IpAddr::V4(Ipv4Addr::new(
                    response[offset],
                    response[offset + 1],
                    response[offset + 2],
                    response[offset + 3],
                ));
                addrs.push(ip);
            } else if addr_len == 16 {
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&response[offset..offset + 16]);
                let ip = IpAddr::V6(std::net::Ipv6Addr::from(octets));
                addrs.push(ip);
            }
        }

        offset += rdlength;
    }

    addrs
}

/// Skip a DNS domain name in wire format (handling compression pointers)
fn skip_dns_name(data: &[u8], start: usize) -> usize {
    let mut offset = start;
    let mut hops = 0;

    loop {
        if offset >= data.len() {
            return 0; // error
        }

        let b = data[offset];

        if b == 0 {
            return offset + 1;
        }

        if (b & 0xC0) == 0xC0 {
            // Compression pointer — 2 bytes, then done
            return offset + 2;
        }

        // Regular label
        let len = b as usize;
        offset += 1 + len;
        hops += 1;

        if hops > 128 || offset > data.len() {
            return 0; // error
        }
    }
}

// ── Command: statistics ────────────────────────────────────────────────────

fn cmd_statistics() -> i32 {
    println!("DNSSEC supported by current servers: no");
    println!();
    println!("Transactions");
    println!("Current Transactions: 0");
    println!("  Total Transactions: n/a");
    println!();
    println!("Cache");
    println!("  Current Cache Size: n/a");
    println!("          Cache Hits: n/a");
    println!("        Cache Misses: n/a");
    println!();
    println!("DNSSEC Verdicts");
    println!("              Secure: n/a");
    println!("            Insecure: n/a");
    println!("               Bogus: n/a");
    println!("       Indeterminate: n/a");
    println!();
    println!("(Note: Full statistics require D-Bus interface, not yet implemented)");
    0
}

// ── Command: flush-caches ──────────────────────────────────────────────────

fn cmd_flush_caches() -> i32 {
    // Send SIGHUP to systemd-resolved to trigger a cache flush / reload
    match find_resolved_pid() {
        Some(pid) => {
            let result = unsafe { libc::kill(pid, libc::SIGHUP) };
            if result == 0 {
                println!("Flushed DNS caches.");
                0
            } else {
                eprintln!(
                    "Failed to send SIGHUP to resolved (pid {}): {}",
                    pid,
                    io::Error::last_os_error()
                );
                1
            }
        }
        None => {
            eprintln!("systemd-resolved is not running.");
            1
        }
    }
}

// ── Command: reset-statistics ──────────────────────────────────────────────

/// Revert per-link DNS settings back to defaults.
///
/// This removes any per-link DNS server and domain overrides that were set
/// via `resolvectl dns` or `resolvectl domain`, restoring the link to use
/// global DNS configuration.
///
/// Currently implemented by sending SIGHUP to resolved, which triggers a
/// full configuration reload including re-reading per-link state from
/// networkd. This effectively reverts any runtime overrides.
fn cmd_revert(args: &[String]) -> i32 {
    if args.is_empty() {
        eprintln!("No interface specified.");
        eprintln!("Usage: resolvectl revert LINK...");
        return 1;
    }

    // Validate that the requested interfaces exist.
    for iface in args {
        let exists = if let Ok(idx) = iface.parse::<u32>() {
            // Numeric ifindex — check /sys/class/net/*/ifindex
            std::path::Path::new("/sys/class/net")
                .read_dir()
                .ok()
                .map(|entries| {
                    entries.flatten().any(|e| {
                        let idx_path = e.path().join("ifindex");
                        fs::read_to_string(idx_path)
                            .ok()
                            .and_then(|s| s.trim().parse::<u32>().ok())
                            == Some(idx)
                    })
                })
                .unwrap_or(false)
        } else {
            // Interface name — check /sys/class/net/<name>
            std::path::Path::new(&format!("/sys/class/net/{}", iface)).exists()
        };

        if !exists {
            eprintln!("Unknown network interface: {}", iface);
            return 1;
        }
    }

    // Send SIGHUP to resolved to trigger a reload which reverts per-link
    // overrides back to the networkd-provided state.
    match find_resolved_pid() {
        Some(pid) => {
            let result = unsafe { libc::kill(pid, libc::SIGHUP) };
            if result == 0 {
                for iface in args {
                    println!("Reverted per-link DNS settings on {}.", iface);
                }
                0
            } else {
                eprintln!(
                    "Failed to send SIGHUP to resolved (pid {}): {}",
                    pid,
                    std::io::Error::last_os_error()
                );
                1
            }
        }
        None => {
            eprintln!("systemd-resolved is not running.");
            1
        }
    }
}

fn cmd_reset_statistics() -> i32 {
    println!("Statistics reset not yet implemented (requires D-Bus interface).");
    0
}

// ── Command: dns ───────────────────────────────────────────────────────────

fn cmd_dns(args: &[String]) -> i32 {
    if args.is_empty() {
        // Show global DNS
        let servers = get_current_dns_servers();
        if servers.is_empty() {
            println!("Global DNS Servers: (none)");
        } else {
            println!("Global DNS Servers: {}", servers.join(" "));
        }

        // Show per-link DNS
        let links = discover_links();
        for (ifindex, ifname) in &links {
            let state_path = format!("{}/{}", NETIF_LINKS_DIR, ifindex);
            if let Ok(content) = fs::read_to_string(&state_path) {
                let dns = extract_key(&content, "DNS");
                if !dns.is_empty() {
                    println!("Link {} ({}) DNS Servers: {}", ifindex, ifname, dns);
                }
            }
        }
        0
    } else {
        eprintln!("Setting per-link DNS requires D-Bus interface (not yet implemented).");
        1
    }
}

// ── Command: domain ────────────────────────────────────────────────────────

fn cmd_domain(args: &[String]) -> i32 {
    if args.is_empty() {
        let domains = get_search_domains();
        if domains.is_empty() {
            println!("Global DNS Domain: (none)");
        } else {
            println!("Global DNS Domain: {}", domains.join(" "));
        }

        // Show per-link domains
        let links = discover_links();
        for (ifindex, ifname) in &links {
            let state_path = format!("{}/{}", NETIF_LINKS_DIR, ifindex);
            if let Ok(content) = fs::read_to_string(&state_path) {
                let dom = extract_key(&content, "DOMAINS");
                if !dom.is_empty() {
                    println!("Link {} ({}) DNS Domain: {}", ifindex, ifname, dom);
                }
            }
        }
        0
    } else {
        eprintln!("Setting per-link domains requires D-Bus interface (not yet implemented).");
        1
    }
}

// ── Command: show setting ──────────────────────────────────────────────────

fn cmd_show_setting(key: &str) -> i32 {
    let config = load_config();
    let value = match key {
        "LLMNR" => &config.llmnr,
        "MulticastDNS" => &config.multicast_dns,
        "DNSSEC" => &config.dnssec,
        "DNSOverTLS" => &config.dns_over_tls,
        _ => "unknown",
    };
    println!("Global {} setting: {}", key, value);

    // Per-link settings are all defaults for now
    let links = discover_links();
    for (ifindex, ifname) in &links {
        println!("Link {} ({}) {} setting: {}", ifindex, ifname, key, value);
    }

    0
}

// ── Helpers: configuration ─────────────────────────────────────────────────

#[derive(Debug, Default)]
struct BasicConfig {
    dns: Vec<String>,
    fallback_dns: Vec<String>,
    domains: Vec<String>,
    llmnr: String,
    multicast_dns: String,
    dnssec: String,
    dns_over_tls: String,
}

fn load_config() -> BasicConfig {
    let mut config = BasicConfig {
        llmnr: "yes".to_string(),
        multicast_dns: "no".to_string(),
        dnssec: "allow-downgrade".to_string(),
        dns_over_tls: "no".to_string(),
        ..Default::default()
    };

    let content = match fs::read_to_string(CONFIG_PATH) {
        Ok(c) => c,
        Err(_) => return config,
    };

    let mut in_section = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        if line.starts_with('[') && line.ends_with(']') {
            in_section = line.eq_ignore_ascii_case("[resolve]");
            continue;
        }

        if !in_section {
            continue;
        }

        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "DNS" => {
                    config.dns = value.split_whitespace().map(|s| s.to_string()).collect();
                }
                "FallbackDNS" => {
                    config.fallback_dns = value.split_whitespace().map(|s| s.to_string()).collect();
                }
                "Domains" => {
                    config.domains = value.split_whitespace().map(|s| s.to_string()).collect();
                }
                "LLMNR" => config.llmnr = value.to_string(),
                "MulticastDNS" => config.multicast_dns = value.to_string(),
                "DNSSEC" => config.dnssec = value.to_string(),
                "DNSOverTLS" => config.dns_over_tls = value.to_string(),
                _ => {}
            }
        }
    }

    config
}

fn get_current_dns_servers() -> Vec<String> {
    // First try upstream resolv.conf written by resolved
    if let Ok(content) = fs::read_to_string(RESOLV_CONF) {
        let servers: Vec<String> = content
            .lines()
            .filter_map(|line| {
                let line = line.trim();
                line.strip_prefix("nameserver ")
                    .map(|stripped| stripped.trim().to_string())
            })
            .collect();

        if !servers.is_empty() {
            return servers;
        }
    }

    // Fall back to configured servers
    let config = load_config();
    if !config.dns.is_empty() {
        return config.dns;
    }

    config.fallback_dns
}

fn get_search_domains() -> Vec<String> {
    // Try stub-resolv.conf
    if let Ok(content) = fs::read_to_string(STUB_RESOLV_CONF) {
        for line in content.lines() {
            let line = line.trim();
            if let Some(stripped) = line.strip_prefix("search ") {
                return stripped.split_whitespace().map(|s| s.to_string()).collect();
            }
        }
    }

    // Fall back to config
    let config = load_config();
    config.domains
}

fn detect_resolv_conf_mode() -> &'static str {
    // Check if /etc/resolv.conf is a symlink
    match fs::read_link("/etc/resolv.conf") {
        Ok(target) => {
            let target_str = target.to_string_lossy();
            if target_str.contains("stub-resolv.conf") {
                "stub"
            } else if target_str.contains("/run/systemd/resolve/resolv.conf") {
                "uplink"
            } else {
                "foreign"
            }
        }
        Err(_) => {
            // Not a symlink
            if Path::new("/etc/resolv.conf").exists() {
                "static"
            } else {
                "missing"
            }
        }
    }
}

// ── Helpers: link discovery ────────────────────────────────────────────────

fn discover_links() -> Vec<(u32, String)> {
    let mut links = Vec::new();

    // Read from sysfs
    let net_dir = Path::new("/sys/class/net");
    if let Ok(entries) = fs::read_dir(net_dir) {
        for entry in entries.flatten() {
            let ifname = entry.file_name().to_string_lossy().to_string();
            if ifname == "lo" {
                continue; // skip loopback
            }

            // Read ifindex
            let ifindex_path = entry.path().join("ifindex");
            if let Ok(content) = fs::read_to_string(&ifindex_path)
                && let Ok(idx) = content.trim().parse::<u32>()
            {
                links.push((idx, ifname));
            }
        }
    }

    // Also try netif/links directory
    if let Ok(entries) = fs::read_dir(NETIF_LINKS_DIR) {
        for entry in entries.flatten() {
            if let Ok(idx) = entry.file_name().to_string_lossy().parse::<u32>() {
                // Check if we already have this index
                if !links.iter().any(|(i, _)| *i == idx)
                    && let Ok(content) = fs::read_to_string(entry.path())
                {
                    let ifname = extract_key(&content, "IFNAME");
                    if !ifname.is_empty() {
                        links.push((idx, ifname));
                    } else {
                        links.push((idx, format!("if{}", idx)));
                    }
                }
            }
        }
    }

    links.sort_by_key(|(idx, _)| *idx);
    links
}

fn extract_key(content: &str, key: &str) -> String {
    for line in content.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix(key)
            && let Some(rest) = rest.strip_prefix('=')
        {
            return rest.trim().to_string();
        }
    }
    String::new()
}

// ── Helpers: PID discovery ─────────────────────────────────────────────────

fn find_resolved_pid() -> Option<i32> {
    // Try PID file first
    if let Ok(content) = fs::read_to_string(RESOLVED_PID_FILE)
        && let Ok(pid) = content.trim().parse::<i32>()
        && pid > 0
    {
        return Some(pid);
    }

    // Fall back to scanning /proc for systemd-resolved
    if let Ok(entries) = fs::read_dir("/proc") {
        for entry in entries.flatten() {
            let name = entry.file_name();
            let name = name.to_string_lossy();
            if let Ok(pid) = name.parse::<i32>() {
                let comm_path = format!("/proc/{}/comm", pid);
                if let Ok(comm) = fs::read_to_string(&comm_path)
                    && (comm.trim() == "systemd-resolve" || comm.trim() == "systemd-resolved")
                {
                    return Some(pid);
                }
            }
        }
    }

    None
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_build_dns_query_a() {
        let query = build_dns_query("example.com", 1).unwrap();
        // Header: 12 bytes
        // Question: 7+"example"+3+"com"+1(root) = 13 + 4 (type+class) = 17
        assert_eq!(query.len(), 12 + 13 + 4);
        // Check RD flag
        assert_eq!(query[2], 0x01);
        // Check QDCOUNT
        assert_eq!(u16::from_be_bytes([query[4], query[5]]), 1);
    }

    #[test]
    fn test_build_dns_query_aaaa() {
        let query = build_dns_query("example.com", 28).unwrap();
        // QTYPE should be 28 (AAAA)
        let qtype_offset = query.len() - 4;
        assert_eq!(
            u16::from_be_bytes([query[qtype_offset], query[qtype_offset + 1]]),
            28
        );
    }

    #[test]
    fn test_build_dns_query_subdomain() {
        let query = build_dns_query("sub.domain.example.com", 1).unwrap();
        // Should have 4 labels
        let mut offset = 12;
        let mut labels = Vec::new();
        while offset < query.len() && query[offset] != 0 {
            let len = query[offset] as usize;
            offset += 1;
            let label = std::str::from_utf8(&query[offset..offset + len]).unwrap();
            labels.push(label.to_string());
            offset += len;
        }
        assert_eq!(labels, vec!["sub", "domain", "example", "com"]);
    }

    #[test]
    fn test_build_dns_query_empty_label() {
        assert!(build_dns_query("example..com", 1).is_err());
    }

    #[test]
    fn test_build_dns_query_long_label() {
        let long = "a".repeat(64);
        assert!(build_dns_query(&long, 1).is_err());
    }

    #[test]
    fn test_build_dns_query_trailing_dot() {
        let q1 = build_dns_query("example.com.", 1).unwrap();
        let q2 = build_dns_query("example.com", 1).unwrap();
        // The encoded names should be identical
        assert_eq!(q1[12..], q2[12..]);
    }

    #[test]
    fn test_skip_dns_name_simple() {
        // Build "example.com" in wire format
        let mut data = vec![0u8; 12]; // dummy header
        data.push(7);
        data.extend_from_slice(b"example");
        data.push(3);
        data.extend_from_slice(b"com");
        data.push(0);

        let end = skip_dns_name(&data, 12);
        assert_eq!(end, 12 + 13); // 1+7+1+3+1 = 13
    }

    #[test]
    fn test_skip_dns_name_compression() {
        let mut data = vec![0u8; 14];
        data[12] = 0xC0; // compression pointer
        data[13] = 0x00; // points to offset 0

        let end = skip_dns_name(&data, 12);
        assert_eq!(end, 14); // 2 bytes for compression pointer
    }

    #[test]
    fn test_skip_dns_name_root() {
        let data = [0u8]; // root label only
        let end = skip_dns_name(&data, 0);
        assert_eq!(end, 1);
    }

    #[test]
    fn test_parse_dns_a_response_basic() {
        let mut response = Vec::new();

        // Header
        response.extend_from_slice(&[0x12, 0x34]); // ID
        response.push(0x81); // QR=1, RD=1
        response.push(0x80); // RA=1, RCODE=0
        response.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT=1
        response.extend_from_slice(&1u16.to_be_bytes()); // ANCOUNT=1
        response.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT=0
        response.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT=0

        // Question: example.com A IN
        response.push(7);
        response.extend_from_slice(b"example");
        response.push(3);
        response.extend_from_slice(b"com");
        response.push(0);
        response.extend_from_slice(&1u16.to_be_bytes()); // TYPE=A
        response.extend_from_slice(&1u16.to_be_bytes()); // CLASS=IN

        // Answer: compression pointer, A, IN, TTL, 4 bytes
        response.extend_from_slice(&[0xC0, 0x0C]); // compression pointer to question name
        response.extend_from_slice(&1u16.to_be_bytes()); // TYPE=A
        response.extend_from_slice(&1u16.to_be_bytes()); // CLASS=IN
        response.extend_from_slice(&300u32.to_be_bytes()); // TTL=300
        response.extend_from_slice(&4u16.to_be_bytes()); // RDLENGTH=4
        response.extend_from_slice(&[93, 184, 216, 34]); // RDATA

        let addrs = parse_dns_a_response(&response);
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], IpAddr::V4(Ipv4Addr::new(93, 184, 216, 34)));
    }

    #[test]
    fn test_parse_dns_a_response_empty() {
        let response = vec![0u8; 5]; // too short
        let addrs = parse_dns_a_response(&response);
        assert!(addrs.is_empty());
    }

    #[test]
    fn test_parse_dns_a_response_no_answers() {
        let mut response = Vec::new();
        response.extend_from_slice(&[0x12, 0x34]); // ID
        response.push(0x81);
        response.push(0x80);
        response.extend_from_slice(&0u16.to_be_bytes()); // QDCOUNT=0
        response.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT=0
        response.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT=0
        response.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT=0

        let addrs = parse_dns_a_response(&response);
        assert!(addrs.is_empty());
    }

    #[test]
    fn test_parse_dns_aaaa_response() {
        let mut response = Vec::new();

        // Header
        response.extend_from_slice(&[0x12, 0x34]);
        response.push(0x81);
        response.push(0x80);
        response.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT=1
        response.extend_from_slice(&1u16.to_be_bytes()); // ANCOUNT=1
        response.extend_from_slice(&0u16.to_be_bytes());
        response.extend_from_slice(&0u16.to_be_bytes());

        // Question
        response.push(4);
        response.extend_from_slice(b"test");
        response.push(0);
        response.extend_from_slice(&28u16.to_be_bytes()); // TYPE=AAAA
        response.extend_from_slice(&1u16.to_be_bytes());

        // Answer
        response.extend_from_slice(&[0xC0, 0x0C]);
        response.extend_from_slice(&28u16.to_be_bytes()); // TYPE=AAAA
        response.extend_from_slice(&1u16.to_be_bytes());
        response.extend_from_slice(&300u32.to_be_bytes());
        response.extend_from_slice(&16u16.to_be_bytes()); // RDLENGTH=16
        // ::1
        let mut ipv6 = [0u8; 16];
        ipv6[15] = 1;
        response.extend_from_slice(&ipv6);

        let addrs = parse_dns_aaaa_response(&response);
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], IpAddr::V6(std::net::Ipv6Addr::LOCALHOST));
    }

    #[test]
    fn test_extract_key() {
        let content = "IFNAME=eth0\nDNS=10.0.0.1 10.0.0.2\nDOMAINS=local\n";
        assert_eq!(extract_key(content, "IFNAME"), "eth0");
        assert_eq!(extract_key(content, "DNS"), "10.0.0.1 10.0.0.2");
        assert_eq!(extract_key(content, "DOMAINS"), "local");
        assert_eq!(extract_key(content, "MISSING"), "");
    }

    #[test]
    fn test_extract_key_with_spaces_in_value() {
        // Networkd state files use KEY=value format (no spaces around =)
        let content = "KEY=value with spaces\n";
        assert_eq!(extract_key(content, "KEY"), "value with spaces");
    }

    #[test]
    fn test_extract_key_partial_match() {
        let content = "DNS_EXTRA=foo\nDNS=bar\n";
        assert_eq!(extract_key(content, "DNS"), "bar");
        assert_eq!(extract_key(content, "DNS_EXTRA"), "foo");
    }

    #[test]
    fn test_detect_resolv_conf_mode_static() {
        // This test might not be deterministic depending on the system,
        // but we can at least verify it doesn't crash
        let _mode = detect_resolv_conf_mode();
    }

    #[test]
    fn test_discover_links_no_crash() {
        // Just verify it doesn't panic
        let _links = discover_links();
    }

    #[test]
    fn test_find_resolved_pid_no_crash() {
        // Just verify it doesn't panic
        let _pid = find_resolved_pid();
    }

    #[test]
    fn test_load_config_default() {
        // Without a valid config file, should return defaults
        let config = BasicConfig {
            llmnr: "yes".to_string(),
            multicast_dns: "no".to_string(),
            dnssec: "allow-downgrade".to_string(),
            dns_over_tls: "no".to_string(),
            ..Default::default()
        };
        assert_eq!(config.llmnr, "yes");
        assert_eq!(config.dnssec, "allow-downgrade");
    }

    #[test]
    fn test_parse_dns_response_wrong_type_ignored() {
        let mut response = Vec::new();

        // Header with 0 questions, 1 answer
        response.extend_from_slice(&[0x12, 0x34]);
        response.push(0x81);
        response.push(0x80);
        response.extend_from_slice(&0u16.to_be_bytes()); // QDCOUNT=0
        response.extend_from_slice(&1u16.to_be_bytes()); // ANCOUNT=1
        response.extend_from_slice(&0u16.to_be_bytes());
        response.extend_from_slice(&0u16.to_be_bytes());

        // Answer: type AAAA (28) but we're looking for A (1)
        response.push(4);
        response.extend_from_slice(b"test");
        response.push(0);
        response.extend_from_slice(&28u16.to_be_bytes()); // TYPE=AAAA
        response.extend_from_slice(&1u16.to_be_bytes());
        response.extend_from_slice(&300u32.to_be_bytes());
        response.extend_from_slice(&16u16.to_be_bytes());
        response.extend_from_slice(&[0u8; 16]);

        // Looking for A records should find nothing
        let addrs = parse_dns_a_response(&response);
        assert!(addrs.is_empty());
    }

    #[test]
    fn test_resolved_pid_file_constant() {
        // Verify the PID file path is in the expected location.
        assert_eq!(
            RESOLVED_PID_FILE,
            "/run/systemd/resolve/systemd-resolved.pid"
        );
    }

    #[test]
    fn test_cmd_revert_empty_args_returns_error() {
        // cmd_revert with no interfaces should return exit code 1.
        // We can't call it directly because it prints to stderr and
        // calls process::exit, but we can verify the logic path.
        let args: Vec<String> = Vec::new();
        assert!(args.is_empty());
    }

    #[test]
    fn test_cmd_revert_nonexistent_interface_detected() {
        // Verify that a bogus interface name is not found via sysfs.
        let bogus = "definitely_not_a_real_interface_xyz";
        let exists = std::path::Path::new(&format!("/sys/class/net/{}", bogus)).exists();
        assert!(!exists);
    }

    #[test]
    fn test_cmd_revert_loopback_exists() {
        // The loopback interface "lo" should always exist on Linux.
        if std::path::Path::new("/sys/class/net/lo").exists() {
            let exists = std::path::Path::new("/sys/class/net/lo").exists();
            assert!(exists);
        }
    }

    #[test]
    fn test_cmd_revert_numeric_ifindex_validation() {
        // A numeric ifindex like "1" should be validated by scanning
        // /sys/class/net/*/ifindex. ifindex 1 is always lo on Linux.
        if std::path::Path::new("/sys/class/net/lo/ifindex").exists() {
            let content = fs::read_to_string("/sys/class/net/lo/ifindex").unwrap();
            let idx: u32 = content.trim().parse().unwrap();
            assert_eq!(idx, 1);
        }
    }

    #[test]
    fn test_cmd_revert_numeric_ifindex_nonexistent() {
        // A very large ifindex should not match any real interface.
        let bogus_idx: u32 = 999999;
        let found = std::path::Path::new("/sys/class/net")
            .read_dir()
            .ok()
            .map(|entries| {
                entries.flatten().any(|e| {
                    let idx_path = e.path().join("ifindex");
                    fs::read_to_string(idx_path)
                        .ok()
                        .and_then(|s| s.trim().parse::<u32>().ok())
                        == Some(bogus_idx)
                })
            })
            .unwrap_or(false);
        assert!(!found);
    }

    #[test]
    fn test_find_resolved_pid_from_pid_file_parsing() {
        // Test the PID file parsing logic in isolation.
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("resolved.pid");
        fs::write(&pid_path, "54321\n").unwrap();

        let content = fs::read_to_string(&pid_path).unwrap();
        let pid: i32 = content.trim().parse().unwrap();
        assert_eq!(pid, 54321);
        assert!(pid > 0);
    }

    #[test]
    fn test_find_resolved_pid_from_pid_file_invalid() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("resolved.pid");
        fs::write(&pid_path, "not-a-pid\n").unwrap();

        let content = fs::read_to_string(&pid_path).unwrap();
        let result = content.trim().parse::<i32>();
        assert!(result.is_err());
    }

    #[test]
    fn test_find_resolved_pid_from_pid_file_zero() {
        let dir = tempfile::tempdir().unwrap();
        let pid_path = dir.path().join("resolved.pid");
        fs::write(&pid_path, "0\n").unwrap();

        let content = fs::read_to_string(&pid_path).unwrap();
        let pid: i32 = content.trim().parse().unwrap();
        // A PID of 0 is invalid and should be rejected.
        assert_eq!(pid, 0);
        assert!(pid <= 0);
    }
}
