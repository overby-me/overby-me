#![allow(dead_code)]
//! Configuration parsing for systemd-resolved
//!
//! Parses `/etc/systemd/resolved.conf` and drop-in directories following the
//! standard systemd configuration file format with `[Resolve]` section.

use std::fs;
use std::net::{IpAddr, SocketAddr};
use std::path::{Path, PathBuf};

// ── Constants ──────────────────────────────────────────────────────────────

/// Main configuration file path
pub const CONFIG_PATH: &str = "/etc/systemd/resolved.conf";

/// Drop-in configuration directories (searched in order, later values override)
pub const CONFIG_DROPIN_DIRS: &[&str] = &[
    "/etc/systemd/resolved.conf.d",
    "/run/systemd/resolved.conf.d",
    "/usr/lib/systemd/resolved.conf.d",
];

/// Default DNS servers (Cloudflare + Google)
const DEFAULT_DNS: &[&str] = &["1.1.1.1", "8.8.8.8", "1.0.0.1", "8.8.4.4"];

/// Default fallback DNS servers
const DEFAULT_FALLBACK_DNS: &[&str] = &["1.1.1.1", "8.8.8.8", "1.0.0.1", "8.8.4.4"];

/// Default stub listener address
pub const STUB_LISTENER_ADDR: &str = "127.0.0.53";

/// Alternative stub listener address (for direct forwarding)
pub const STUB_LISTENER_ADDR_EXTRA: &str = "127.0.0.54";

/// Default DNS port
pub const DNS_PORT: u16 = 53;

/// Stub resolv.conf path (points to 127.0.0.53)
pub const STUB_RESOLV_CONF_PATH: &str = "/run/systemd/resolve/stub-resolv.conf";

/// Upstream resolv.conf path (points to actual upstream servers)
pub const RESOLV_CONF_PATH: &str = "/run/systemd/resolve/resolv.conf";

/// Runtime state directory
pub const STATE_DIR: &str = "/run/systemd/resolve";

// ── DNSSEC mode ────────────────────────────────────────────────────────────

/// DNSSEC validation mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DnssecMode {
    /// Full DNSSEC validation
    Yes,
    /// DNSSEC validation disabled
    No,
    /// Allow downgrade if server doesn't support DNSSEC
    #[default]
    AllowDowngrade,
}

impl DnssecMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "yes" | "true" | "1" => Self::Yes,
            "no" | "false" | "0" => Self::No,
            "allow-downgrade" => Self::AllowDowngrade,
            _ => Self::default(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::No => "no",
            Self::AllowDowngrade => "allow-downgrade",
        }
    }
}

// ── DNS-over-TLS mode ──────────────────────────────────────────────────────

/// DNS-over-TLS mode
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum DnsOverTlsMode {
    /// DNS-over-TLS disabled
    #[default]
    No,
    /// Opportunistic DNS-over-TLS (try TLS, fall back to plain)
    Opportunistic,
    /// Strict DNS-over-TLS (require TLS)
    Yes,
}

impl DnsOverTlsMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "yes" | "true" | "1" => Self::Yes,
            "opportunistic" => Self::Opportunistic,
            "no" | "false" | "0" => Self::No,
            _ => Self::default(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Opportunistic => "opportunistic",
            Self::No => "no",
        }
    }
}

// ── LLMNR / mDNS mode ─────────────────────────────────────────────────────

/// LLMNR / mDNS support mode
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ResolutionMode {
    /// Full support (resolve + respond)
    Yes,
    /// Resolve only (don't respond)
    Resolve,
    /// Disabled
    No,
}

impl ResolutionMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "yes" | "true" | "1" => Self::Yes,
            "resolve" => Self::Resolve,
            "no" | "false" | "0" => Self::No,
            _ => Self::Yes,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Resolve => "resolve",
            Self::No => "no",
        }
    }
}

// ── Stub listener mode ─────────────────────────────────────────────────────

/// Whether the stub listener is enabled
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StubListenerMode {
    /// Listen on 127.0.0.53:53 (UDP + TCP)
    #[default]
    Yes,
    /// Listen only on UDP
    Udp,
    /// Listen only on TCP
    Tcp,
    /// Disabled
    No,
}

impl StubListenerMode {
    pub fn parse(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "yes" | "true" | "1" => Self::Yes,
            "udp" => Self::Udp,
            "tcp" => Self::Tcp,
            "no" | "false" | "0" => Self::No,
            _ => Self::default(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Yes => "yes",
            Self::Udp => "udp",
            Self::Tcp => "tcp",
            Self::No => "no",
        }
    }

    pub fn udp_enabled(&self) -> bool {
        matches!(self, Self::Yes | Self::Udp)
    }

    pub fn tcp_enabled(&self) -> bool {
        matches!(self, Self::Yes | Self::Tcp)
    }
}

// ── DNS server entry ───────────────────────────────────────────────────────

/// A configured DNS server
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnsServer {
    /// IP address of the server
    pub addr: IpAddr,
    /// Port (defaults to 53)
    pub port: u16,
    /// Optional server name (for DNS-over-TLS SNI)
    pub server_name: Option<String>,
}

impl DnsServer {
    pub fn new(addr: IpAddr) -> Self {
        Self {
            addr,
            port: DNS_PORT,
            server_name: None,
        }
    }

    pub fn with_port(mut self, port: u16) -> Self {
        self.port = port;
        self
    }

    pub fn socket_addr(&self) -> SocketAddr {
        SocketAddr::new(self.addr, self.port)
    }
}

/// Parse a DNS server specification. Supports:
/// - Plain IP: `1.1.1.1`, `2001:4860:4860::8844`
/// - IP with port: `1.1.1.1:5353`, `[2001:4860:4860::8844]:5353`
/// - IP with server name: `1.1.1.1#cloudflare-dns.com`
fn parse_dns_server(s: &str) -> Option<DnsServer> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Check for #server-name suffix
    let (addr_part, server_name) = if let Some(idx) = s.find('#') {
        let name = s[idx + 1..].to_string();
        let name = if name.is_empty() { None } else { Some(name) };
        (&s[..idx], name)
    } else {
        (s, None)
    };

    // Try [IPv6]:port format
    if addr_part.starts_with('[') {
        if let Some(bracket_end) = addr_part.find(']') {
            let ip_str = &addr_part[1..bracket_end];
            let ip: IpAddr = ip_str.parse().ok()?;
            let port = if addr_part.len() > bracket_end + 2
                && addr_part.as_bytes()[bracket_end + 1] == b':'
            {
                addr_part[bracket_end + 2..].parse().ok()?
            } else {
                DNS_PORT
            };
            return Some(DnsServer {
                addr: ip,
                port,
                server_name,
            });
        }
        return None;
    }

    // Try IPv4:port format (only if there's exactly one colon, otherwise it's IPv6)
    let colon_count = addr_part.chars().filter(|c| *c == ':').count();
    if colon_count == 1 {
        // IPv4 with port
        let (ip_str, port_str) = addr_part.rsplit_once(':')?;

        if let (Ok(ip), Ok(port)) = (ip_str.parse::<IpAddr>(), port_str.parse::<u16>()) {
            return Some(DnsServer {
                addr: ip,
                port,
                server_name,
            });
        }
    }

    // Plain IP address
    if let Ok(ip) = addr_part.parse::<IpAddr>() {
        return Some(DnsServer {
            addr: ip,
            port: DNS_PORT,
            server_name,
        });
    }

    None
}

/// Parse a space-separated list of DNS server specifications
fn parse_dns_server_list(s: &str) -> Vec<DnsServer> {
    s.split_whitespace().filter_map(parse_dns_server).collect()
}

// ── Per-link DNS configuration ─────────────────────────────────────────────

/// DNS configuration for a specific network link
#[derive(Debug, Clone)]
pub struct LinkDns {
    /// Interface index
    pub ifindex: u32,
    /// Interface name
    pub ifname: String,
    /// DNS servers configured for this link
    pub dns_servers: Vec<DnsServer>,
    /// Search domains for this link
    pub domains: Vec<String>,
    /// Whether this link's DNS config is the default route
    pub default_route: bool,
    /// DNSSEC mode for this link
    pub dnssec: Option<DnssecMode>,
    /// DNS-over-TLS mode for this link
    pub dns_over_tls: Option<DnsOverTlsMode>,
}

impl LinkDns {
    pub fn new(ifindex: u32, ifname: String) -> Self {
        Self {
            ifindex,
            ifname,
            dns_servers: Vec::new(),
            domains: Vec::new(),
            default_route: false,
            dnssec: None,
            dns_over_tls: None,
        }
    }
}

// ── Main configuration ─────────────────────────────────────────────────────

/// Resolved configuration from `resolved.conf`
#[derive(Debug, Clone)]
pub struct ResolvedConfig {
    /// Configured DNS servers
    pub dns: Vec<DnsServer>,
    /// Fallback DNS servers (used when no other servers are configured)
    pub fallback_dns: Vec<DnsServer>,
    /// Search domains
    pub domains: Vec<String>,
    /// LLMNR mode
    pub llmnr: ResolutionMode,
    /// mDNS mode
    pub multicast_dns: ResolutionMode,
    /// DNSSEC mode
    pub dnssec: DnssecMode,
    /// DNS-over-TLS mode
    pub dns_over_tls: DnsOverTlsMode,
    /// Cache mode (true = enabled)
    pub cache: bool,
    /// Cache size (0 = use compiled-in default)
    pub cache_from_localhost: bool,
    /// Stub listener mode
    pub dns_stub_listener: StubListenerMode,
    /// Extra stub listener addresses
    pub dns_stub_listener_extra: Vec<SocketAddr>,
    /// Read /etc/hosts
    pub read_etc_hosts: bool,
    /// Resolve unicast DNS (if false, only mDNS/LLMNR)
    pub resolve_unicast_single_label: bool,
    /// Per-link DNS configuration
    pub link_dns: Vec<LinkDns>,
    /// Negative trust anchors — domain names for which DNSSEC validation is
    /// skipped.  Configured via `NegativeTrustAnchors=` in `resolved.conf`
    /// (space-separated list of domains).  When DNSSEC is enabled, queries
    /// whose name falls under one of these domains will not have their
    /// signatures validated, allowing broken-DNSSEC domains to resolve.
    pub negative_trust_anchors: Vec<String>,
}

impl Default for ResolvedConfig {
    fn default() -> Self {
        Self {
            dns: Vec::new(),
            fallback_dns: DEFAULT_FALLBACK_DNS
                .iter()
                .filter_map(|s| parse_dns_server(s))
                .collect(),
            domains: Vec::new(),
            llmnr: ResolutionMode::Yes,
            multicast_dns: ResolutionMode::No,
            dnssec: DnssecMode::default(),
            dns_over_tls: DnsOverTlsMode::default(),
            cache: true,
            cache_from_localhost: false,
            dns_stub_listener: StubListenerMode::default(),
            dns_stub_listener_extra: Vec::new(),
            read_etc_hosts: true,
            resolve_unicast_single_label: false,
            link_dns: Vec::new(),
            negative_trust_anchors: Vec::new(),
        }
    }
}

impl ResolvedConfig {
    /// Parse the resolved configuration from the standard paths
    pub fn load() -> Self {
        let mut config = Self::default();

        // Parse main config file
        if let Ok(content) = fs::read_to_string(CONFIG_PATH) {
            config.parse_config(&content);
        }

        // Parse drop-in directories
        for dir in CONFIG_DROPIN_DIRS {
            config.parse_dropin_dir(Path::new(dir));
        }

        // If no DNS servers configured explicitly, use defaults
        if config.dns.is_empty() {
            config.dns = DEFAULT_DNS
                .iter()
                .filter_map(|s| parse_dns_server(s))
                .collect();
        }

        config
    }

    /// Parse configuration from a file path
    pub fn load_from(path: &Path) -> Self {
        let mut config = Self::default();
        if let Ok(content) = fs::read_to_string(path) {
            config.parse_config(&content);
        }
        if config.dns.is_empty() {
            config.dns = DEFAULT_DNS
                .iter()
                .filter_map(|s| parse_dns_server(s))
                .collect();
        }
        config
    }

    /// Parse drop-in directory for additional configuration
    fn parse_dropin_dir(&mut self, dir: &Path) {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        let mut files: Vec<PathBuf> = entries
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e == "conf"))
            .collect();

        files.sort();

        for file in files {
            if let Ok(content) = fs::read_to_string(&file) {
                self.parse_config(&content);
            }
        }
    }

    /// Parse configuration content (INI-style with [Resolve] section)
    pub fn parse_config(&mut self, content: &str) {
        let mut in_resolve_section = false;

        for line in content.lines() {
            let line = line.trim();

            // Skip empty lines and comments
            if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
                continue;
            }

            // Section headers
            if line.starts_with('[') && line.ends_with(']') {
                in_resolve_section = line.eq_ignore_ascii_case("[resolve]");
                continue;
            }

            if !in_resolve_section {
                continue;
            }

            // Key=Value pairs
            if let Some((key, value)) = line.split_once('=') {
                let key = key.trim();
                let value = value.trim();
                self.set_value(key, value);
            }
        }
    }

    fn set_value(&mut self, key: &str, value: &str) {
        match key {
            "DNS" => {
                if value.is_empty() {
                    self.dns.clear();
                } else {
                    self.dns = parse_dns_server_list(value);
                }
            }
            "FallbackDNS" => {
                if value.is_empty() {
                    self.fallback_dns.clear();
                } else {
                    self.fallback_dns = parse_dns_server_list(value);
                }
            }
            "Domains" => {
                if value.is_empty() {
                    self.domains.clear();
                } else {
                    self.domains = value.split_whitespace().map(|s| s.to_string()).collect();
                }
            }
            "LLMNR" => {
                self.llmnr = ResolutionMode::parse(value);
            }
            "MulticastDNS" => {
                self.multicast_dns = ResolutionMode::parse(value);
            }
            "DNSSEC" => {
                self.dnssec = DnssecMode::parse(value);
            }
            "DNSOverTLS" => {
                self.dns_over_tls = DnsOverTlsMode::parse(value);
            }
            "Cache" => match value.to_lowercase().as_str() {
                "no-negative" => {
                    self.cache = true;
                }
                _ => {
                    self.cache = parse_bool(value);
                }
            },
            "CacheFromLocalhost" => {
                self.cache_from_localhost = parse_bool(value);
            }
            "DNSStubListener" => {
                self.dns_stub_listener = StubListenerMode::parse(value);
            }
            "DNSStubListenerExtra" => {
                // Can be specified multiple times; each call adds to the list.
                // An empty value resets.
                if value.is_empty() {
                    self.dns_stub_listener_extra.clear();
                } else {
                    for entry in value.split_whitespace() {
                        if let Some(server) = parse_dns_server(entry) {
                            self.dns_stub_listener_extra.push(server.socket_addr());
                        }
                    }
                }
            }
            "ReadEtcHosts" => {
                self.read_etc_hosts = parse_bool(value);
            }
            "ResolveUnicastSingleLabel" => {
                self.resolve_unicast_single_label = parse_bool(value);
            }
            "NegativeTrustAnchors" => {
                if value.is_empty() {
                    self.negative_trust_anchors.clear();
                } else {
                    self.negative_trust_anchors = value
                        .split_whitespace()
                        .map(|s| s.trim_end_matches('.').to_lowercase())
                        .filter(|s| !s.is_empty())
                        .collect();
                }
            }
            _ => {
                // Unknown key, ignore
            }
        }
    }

    /// Get the effective DNS servers to use for resolution.
    ///
    /// Returns per-link DNS servers if any are configured, otherwise falls
    /// back to global DNS servers, and finally to fallback DNS servers.
    pub fn effective_dns_servers(&self) -> Vec<&DnsServer> {
        // First try per-link DNS servers
        let link_servers: Vec<&DnsServer> = self
            .link_dns
            .iter()
            .flat_map(|link| link.dns_servers.iter())
            .collect();

        if !link_servers.is_empty() {
            return link_servers;
        }

        // Then global DNS
        if !self.dns.is_empty() {
            return self.dns.iter().collect();
        }

        // Finally fallback DNS
        self.fallback_dns.iter().collect()
    }

    /// Get the effective search domains
    pub fn effective_search_domains(&self) -> Vec<&str> {
        let mut domains: Vec<&str> = Vec::new();

        // Per-link domains first
        for link in &self.link_dns {
            for d in &link.domains {
                let d_str = d.as_str();
                if !domains.contains(&d_str) {
                    domains.push(d_str);
                }
            }
        }

        // Then global domains
        for d in &self.domains {
            let d_str = d.as_str();
            if !domains.contains(&d_str) {
                domains.push(d_str);
            }
        }

        domains
    }

    /// Check whether a domain name falls under a configured negative trust
    /// anchor.  Returns `true` when DNSSEC validation should be skipped for
    /// this name — i.e. the name itself *is* one of the NTA domains or is a
    /// subdomain of one.
    pub fn is_negative_trust_anchor(&self, name: &str) -> bool {
        if self.negative_trust_anchors.is_empty() {
            return false;
        }
        let name = name.trim_end_matches('.').to_lowercase();
        for nta in &self.negative_trust_anchors {
            if name == *nta {
                return true;
            }
            if name.ends_with(&format!(".{}", nta)) {
                return true;
            }
        }
        false
    }

    /// Update per-link DNS configuration from networkd state files
    pub fn update_link_dns_from_networkd(&mut self) {
        self.link_dns.clear();

        let netif_dir = Path::new("/run/systemd/netif/links");
        let entries = match fs::read_dir(netif_dir) {
            Ok(e) => e,
            Err(_) => return,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            let ifindex: u32 = match path
                .file_name()
                .and_then(|n| n.to_str())
                .and_then(|n| n.parse().ok())
            {
                Some(idx) => idx,
                None => continue,
            };

            if let Ok(content) = fs::read_to_string(&path)
                && let Some(link_dns) = parse_link_state(ifindex, &content)
                && !link_dns.dns_servers.is_empty()
            {
                self.link_dns.push(link_dns);
            }
        }
    }

    /// Generate the content for the stub resolv.conf file
    /// This points to 127.0.0.53 so local programs use the stub resolver
    pub fn stub_resolv_conf_content(&self) -> String {
        let mut content = String::new();
        content.push_str(
            "# This is /run/systemd/resolve/stub-resolv.conf managed by systemd-resolved.\n",
        );
        content.push_str("# Do not edit.\n");
        content.push_str("#\n");
        content.push_str("# This file might be symlinked as /etc/resolv.conf. If you're looking\n");
        content.push_str(
            "# at /etc/resolv.conf and seeing this text, you have followed the symlink.\n",
        );
        content.push_str("#\n");
        content
            .push_str("# This is a dynamic resolv.conf file for connecting local clients to the\n");
        content.push_str("# internal DNS stub resolver of systemd-resolved. This file lists all\n");
        content.push_str("# configured search domains.\n");
        content.push_str("#\n");
        content
            .push_str("# Run \"resolvectl status\" to see details about the uplink DNS servers\n");
        content.push_str("# currently in use.\n");
        content.push_str("#\n");
        content.push_str(
            "# Third party programs should typically not access this file directly, but\n",
        );
        content.push_str("# only through the symlink at /etc/resolv.conf. To manage\n");
        content.push_str(
            "# man:systemd-resolved.service(8) in a different way, replace this symlink\n",
        );
        content.push_str("# by a static file or a different symlink.\n");
        content.push_str("#\n");
        content.push_str("# See man:systemd-resolved.service(8) for details.\n\n");
        content.push_str(&format!("nameserver {}\n", STUB_LISTENER_ADDR));
        content.push_str("options edns0 trust-ad\n");

        let search_domains = self.effective_search_domains();
        if !search_domains.is_empty() {
            content.push_str(&format!("search {}\n", search_domains.join(" ")));
        }

        content
    }

    /// Generate the content for the upstream resolv.conf file
    /// This lists the actual upstream DNS servers (for programs that bypass the stub)
    pub fn upstream_resolv_conf_content(&self) -> String {
        let mut content = String::new();
        content
            .push_str("# This is /run/systemd/resolve/resolv.conf managed by systemd-resolved.\n");
        content.push_str("# Do not edit.\n");
        content.push_str("#\n");
        content.push_str(
            "# This file contains the actual upstream DNS servers systemd-resolved uses.\n",
        );
        content
            .push_str("# Programs that bypass the local stub resolver (127.0.0.53) can use this\n");
        content.push_str("# file instead.\n");
        content.push_str("#\n");
        content.push_str("# See man:systemd-resolved.service(8) for details.\n\n");

        let servers = self.effective_dns_servers();
        for server in &servers {
            content.push_str(&format!("nameserver {}\n", server.addr));
        }

        let search_domains = self.effective_search_domains();
        if !search_domains.is_empty() {
            content.push_str(&format!("search {}\n", search_domains.join(" ")));
        }

        content
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

fn parse_bool(s: &str) -> bool {
    matches!(
        s.trim().to_lowercase().as_str(),
        "yes" | "true" | "1" | "on"
    )
}

/// Parse a networkd link state file to extract DNS configuration
fn parse_link_state(ifindex: u32, content: &str) -> Option<LinkDns> {
    let mut ifname = String::new();
    let mut dns_servers = Vec::new();
    let mut domains = Vec::new();

    for line in content.lines() {
        let line = line.trim();
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            match key {
                "IFNAME" | "NETWORK_FILE_IFNAME" if ifname.is_empty() => {
                    ifname = value.to_string();
                }
                "DNS" => {
                    for server_str in value.split_whitespace() {
                        if let Some(server) = parse_dns_server(server_str) {
                            dns_servers.push(server);
                        }
                    }
                }
                "DOMAINS" => {
                    for domain in value.split_whitespace() {
                        domains.push(domain.to_string());
                    }
                }
                _ => {}
            }
        }
    }

    if ifname.is_empty() {
        ifname = format!("if{}", ifindex);
    }

    let mut link = LinkDns::new(ifindex, ifname);
    link.dns_servers = dns_servers;
    link.domains = domains;
    Some(link)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    #[test]
    fn test_default_config() {
        let config = ResolvedConfig::default();
        assert!(config.dns.is_empty());
        assert!(!config.fallback_dns.is_empty());
        assert!(config.cache);
        assert_eq!(config.dnssec, DnssecMode::AllowDowngrade);
        assert_eq!(config.dns_over_tls, DnsOverTlsMode::No);
        assert_eq!(config.dns_stub_listener, StubListenerMode::Yes);
        assert!(config.read_etc_hosts);
    }

    #[test]
    fn test_parse_dns_server_ipv4() {
        let server = parse_dns_server("1.1.1.1").unwrap();
        assert_eq!(server.addr, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(server.port, 53);
        assert!(server.server_name.is_none());
    }

    #[test]
    fn test_parse_dns_server_ipv4_port() {
        let server = parse_dns_server("1.1.1.1:5353").unwrap();
        assert_eq!(server.addr, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(server.port, 5353);
    }

    #[test]
    fn test_parse_dns_server_ipv6() {
        let server = parse_dns_server("2001:4860:4860::8844").unwrap();
        assert_eq!(
            server.addr,
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8844))
        );
        assert_eq!(server.port, 53);
    }

    #[test]
    fn test_parse_dns_server_ipv6_bracket_port() {
        let server = parse_dns_server("[2001:4860:4860::8844]:5353").unwrap();
        assert_eq!(
            server.addr,
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8844))
        );
        assert_eq!(server.port, 5353);
    }

    #[test]
    fn test_parse_dns_server_with_name() {
        let server = parse_dns_server("1.1.1.1#cloudflare-dns.com").unwrap();
        assert_eq!(server.addr, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(server.port, 53);
        assert_eq!(server.server_name.as_deref(), Some("cloudflare-dns.com"));
    }

    #[test]
    fn test_parse_dns_server_empty() {
        assert!(parse_dns_server("").is_none());
        assert!(parse_dns_server("   ").is_none());
    }

    #[test]
    fn test_parse_dns_server_list() {
        let servers = parse_dns_server_list("1.1.1.1 8.8.8.8 2001:4860:4860::8844");
        assert_eq!(servers.len(), 3);
        assert_eq!(servers[0].addr, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(servers[1].addr, IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8)));
        assert_eq!(
            servers[2].addr,
            IpAddr::V6(Ipv6Addr::new(0x2001, 0x4860, 0x4860, 0, 0, 0, 0, 0x8844))
        );
    }

    #[test]
    fn test_parse_config_basic() {
        let mut config = ResolvedConfig::default();
        config.parse_config(
            "[Resolve]\n\
             DNS=9.9.9.9 149.112.112.112\n\
             FallbackDNS=1.1.1.1\n\
             Domains=example.com local\n\
             DNSSEC=yes\n\
             DNSOverTLS=opportunistic\n\
             LLMNR=no\n\
             MulticastDNS=resolve\n\
             Cache=yes\n\
             DNSStubListener=yes\n",
        );

        assert_eq!(config.dns.len(), 2);
        assert_eq!(config.dns[0].addr, IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)));
        assert_eq!(
            config.dns[1].addr,
            IpAddr::V4(Ipv4Addr::new(149, 112, 112, 112))
        );
        assert_eq!(config.fallback_dns.len(), 1);
        assert_eq!(config.domains, vec!["example.com", "local"]);
        assert_eq!(config.dnssec, DnssecMode::Yes);
        assert_eq!(config.dns_over_tls, DnsOverTlsMode::Opportunistic);
        assert_eq!(config.llmnr, ResolutionMode::No);
        assert_eq!(config.multicast_dns, ResolutionMode::Resolve);
        assert!(config.cache);
        assert_eq!(config.dns_stub_listener, StubListenerMode::Yes);
    }

    #[test]
    fn test_parse_config_comments_and_empty_lines() {
        let mut config = ResolvedConfig::default();
        config.parse_config(
            "# This is a comment\n\
             ; This is also a comment\n\
             \n\
             [Resolve]\n\
             # Comment in section\n\
             DNS=1.1.1.1\n\
             \n\
             DNSSEC=no\n",
        );

        assert_eq!(config.dns.len(), 1);
        assert_eq!(config.dnssec, DnssecMode::No);
    }

    #[test]
    fn test_parse_config_empty_values_clear() {
        let mut config = ResolvedConfig::default();
        // First set some values
        config.parse_config("[Resolve]\nDNS=1.1.1.1\nDomains=example.com\n");
        assert_eq!(config.dns.len(), 1);
        assert_eq!(config.domains.len(), 1);

        // Now clear them with empty values
        config.parse_config("[Resolve]\nDNS=\nDomains=\n");
        assert!(config.dns.is_empty());
        assert!(config.domains.is_empty());
    }

    #[test]
    fn test_parse_config_wrong_section_ignored() {
        let mut config = ResolvedConfig::default();
        config.parse_config(
            "[SomeOtherSection]\n\
             DNS=9.9.9.9\n\
             [Resolve]\n\
             DNS=1.1.1.1\n",
        );

        assert_eq!(config.dns.len(), 1);
        assert_eq!(config.dns[0].addr, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
    }

    #[test]
    fn test_parse_config_stub_listener_modes() {
        let mut config = ResolvedConfig::default();

        config.parse_config("[Resolve]\nDNSStubListener=no\n");
        assert_eq!(config.dns_stub_listener, StubListenerMode::No);
        assert!(!config.dns_stub_listener.udp_enabled());
        assert!(!config.dns_stub_listener.tcp_enabled());

        config.parse_config("[Resolve]\nDNSStubListener=udp\n");
        assert_eq!(config.dns_stub_listener, StubListenerMode::Udp);
        assert!(config.dns_stub_listener.udp_enabled());
        assert!(!config.dns_stub_listener.tcp_enabled());

        config.parse_config("[Resolve]\nDNSStubListener=tcp\n");
        assert_eq!(config.dns_stub_listener, StubListenerMode::Tcp);
        assert!(!config.dns_stub_listener.udp_enabled());
        assert!(config.dns_stub_listener.tcp_enabled());
    }

    #[test]
    fn test_parse_config_stub_listener_extra() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Resolve]\nDNSStubListenerExtra=127.0.0.1:5353\n");
        assert_eq!(config.dns_stub_listener_extra.len(), 1);
        assert_eq!(
            config.dns_stub_listener_extra[0],
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 5353)
        );
    }

    #[test]
    fn test_dnssec_mode_parse() {
        assert_eq!(DnssecMode::parse("yes"), DnssecMode::Yes);
        assert_eq!(DnssecMode::parse("no"), DnssecMode::No);
        assert_eq!(
            DnssecMode::parse("allow-downgrade"),
            DnssecMode::AllowDowngrade
        );
        assert_eq!(DnssecMode::parse("true"), DnssecMode::Yes);
        assert_eq!(DnssecMode::parse("false"), DnssecMode::No);
        assert_eq!(DnssecMode::parse("garbage"), DnssecMode::AllowDowngrade);
    }

    #[test]
    fn test_dns_over_tls_mode_parse() {
        assert_eq!(DnsOverTlsMode::parse("yes"), DnsOverTlsMode::Yes);
        assert_eq!(DnsOverTlsMode::parse("no"), DnsOverTlsMode::No);
        assert_eq!(
            DnsOverTlsMode::parse("opportunistic"),
            DnsOverTlsMode::Opportunistic
        );
        assert_eq!(DnsOverTlsMode::parse("garbage"), DnsOverTlsMode::No);
    }

    #[test]
    fn test_resolution_mode_parse() {
        assert_eq!(ResolutionMode::parse("yes"), ResolutionMode::Yes);
        assert_eq!(ResolutionMode::parse("no"), ResolutionMode::No);
        assert_eq!(ResolutionMode::parse("resolve"), ResolutionMode::Resolve);
    }

    #[test]
    fn test_effective_dns_servers_fallback_chain() {
        // No DNS and no links → fallback
        let config = ResolvedConfig::default();
        let servers = config.effective_dns_servers();
        assert!(!servers.is_empty());
        // These should be the fallback servers
        assert_eq!(servers[0].addr, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));

        // With explicit DNS → use those
        let config = ResolvedConfig {
            dns: vec![DnsServer::new(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)))],
            ..Default::default()
        };
        let servers = config.effective_dns_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].addr, IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)));

        // With per-link DNS → use those
        let mut config = ResolvedConfig {
            dns: vec![DnsServer::new(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)))],
            ..Default::default()
        };
        let mut link = LinkDns::new(2, "eth0".to_string());
        link.dns_servers = vec![DnsServer::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)))];
        config.link_dns.push(link);
        let servers = config.effective_dns_servers();
        assert_eq!(servers.len(), 1);
        assert_eq!(servers[0].addr, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn test_effective_search_domains() {
        let mut config = ResolvedConfig {
            domains: vec!["global.example.com".to_string()],
            ..Default::default()
        };

        let mut link = LinkDns::new(2, "eth0".to_string());
        link.domains = vec!["link.example.com".to_string()];
        config.link_dns.push(link);

        let domains = config.effective_search_domains();
        assert_eq!(domains, vec!["link.example.com", "global.example.com"]);
    }

    #[test]
    fn test_effective_search_domains_no_duplicates() {
        let mut config = ResolvedConfig {
            domains: vec!["example.com".to_string()],
            ..Default::default()
        };

        let mut link = LinkDns::new(2, "eth0".to_string());
        link.domains = vec!["example.com".to_string()];
        config.link_dns.push(link);

        let domains = config.effective_search_domains();
        assert_eq!(domains, vec!["example.com"]);
    }

    #[test]
    fn test_stub_resolv_conf_content() {
        let config = ResolvedConfig::default();
        let content = config.stub_resolv_conf_content();
        assert!(content.contains("nameserver 127.0.0.53"));
        assert!(content.contains("options edns0 trust-ad"));
    }

    #[test]
    fn test_stub_resolv_conf_with_domains() {
        let config = ResolvedConfig {
            domains: vec!["example.com".to_string(), "local".to_string()],
            ..Default::default()
        };
        let content = config.stub_resolv_conf_content();
        assert!(content.contains("search example.com local"));
    }

    #[test]
    fn test_upstream_resolv_conf_content() {
        let config = ResolvedConfig {
            dns: vec![
                DnsServer::new(IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9))),
                DnsServer::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))),
            ],
            ..Default::default()
        };
        let content = config.upstream_resolv_conf_content();
        assert!(content.contains("nameserver 9.9.9.9"));
        assert!(content.contains("nameserver 1.1.1.1"));
    }

    #[test]
    fn test_parse_link_state() {
        let content = "IFNAME=eth0\nDNS=10.0.0.1 10.0.0.2\nDOMAINS=local corp.example.com\n";
        let link = parse_link_state(2, content).unwrap();
        assert_eq!(link.ifindex, 2);
        assert_eq!(link.ifname, "eth0");
        assert_eq!(link.dns_servers.len(), 2);
        assert_eq!(
            link.dns_servers[0].addr,
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1))
        );
        assert_eq!(
            link.dns_servers[1].addr,
            IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2))
        );
        assert_eq!(link.domains, vec!["local", "corp.example.com"]);
    }

    #[test]
    fn test_parse_link_state_missing_ifname() {
        let content = "DNS=10.0.0.1\n";
        let link = parse_link_state(5, content).unwrap();
        assert_eq!(link.ifname, "if5");
    }

    #[test]
    fn test_dns_server_socket_addr() {
        let server = DnsServer::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(
            server.socket_addr(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 53)
        );
    }

    #[test]
    fn test_dns_server_with_port() {
        let server = DnsServer::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1))).with_port(5353);
        assert_eq!(server.port, 5353);
        assert_eq!(
            server.socket_addr(),
            SocketAddr::new(IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)), 5353)
        );
    }

    #[test]
    fn test_parse_config_last_value_wins() {
        let mut config = ResolvedConfig::default();
        config.parse_config(
            "[Resolve]\n\
             DNS=1.1.1.1\n\
             DNS=9.9.9.9\n",
        );
        // Last DNS= line replaces the previous one
        assert_eq!(config.dns.len(), 1);
        assert_eq!(config.dns[0].addr, IpAddr::V4(Ipv4Addr::new(9, 9, 9, 9)));
    }

    #[test]
    fn test_parse_config_cache_no_negative() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Resolve]\nCache=no-negative\n");
        assert!(config.cache);
    }

    #[test]
    fn test_parse_config_cache_disabled() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Resolve]\nCache=no\n");
        assert!(!config.cache);
    }

    #[test]
    fn test_parse_config_all_bool_fields() {
        let mut config = ResolvedConfig::default();
        config.parse_config(
            "[Resolve]\n\
             ReadEtcHosts=no\n\
             CacheFromLocalhost=yes\n\
             ResolveUnicastSingleLabel=yes\n",
        );
        assert!(!config.read_etc_hosts);
        assert!(config.cache_from_localhost);
        assert!(config.resolve_unicast_single_label);
    }

    #[test]
    fn test_parse_bool_values() {
        assert!(parse_bool("yes"));
        assert!(parse_bool("true"));
        assert!(parse_bool("1"));
        assert!(parse_bool("on"));
        assert!(!parse_bool("no"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool("off"));
        assert!(!parse_bool("garbage"));
    }

    #[test]
    fn test_dnssec_mode_as_str() {
        assert_eq!(DnssecMode::Yes.as_str(), "yes");
        assert_eq!(DnssecMode::No.as_str(), "no");
        assert_eq!(DnssecMode::AllowDowngrade.as_str(), "allow-downgrade");
    }

    #[test]
    fn test_dns_over_tls_mode_as_str() {
        assert_eq!(DnsOverTlsMode::Yes.as_str(), "yes");
        assert_eq!(DnsOverTlsMode::No.as_str(), "no");
        assert_eq!(DnsOverTlsMode::Opportunistic.as_str(), "opportunistic");
    }

    #[test]
    fn test_resolution_mode_as_str() {
        assert_eq!(ResolutionMode::Yes.as_str(), "yes");
        assert_eq!(ResolutionMode::No.as_str(), "no");
        assert_eq!(ResolutionMode::Resolve.as_str(), "resolve");
    }

    #[test]
    fn test_stub_listener_mode_as_str() {
        assert_eq!(StubListenerMode::Yes.as_str(), "yes");
        assert_eq!(StubListenerMode::No.as_str(), "no");
        assert_eq!(StubListenerMode::Udp.as_str(), "udp");
        assert_eq!(StubListenerMode::Tcp.as_str(), "tcp");
    }

    #[test]
    fn test_parse_dns_server_ipv6_no_brackets() {
        // Plain IPv6 without brackets or port should work
        let server = parse_dns_server("::1").unwrap();
        assert_eq!(server.addr, IpAddr::V6(Ipv6Addr::LOCALHOST));
        assert_eq!(server.port, 53);
    }

    #[test]
    fn test_parse_dns_server_ipv4_with_name_and_port() {
        let server = parse_dns_server("1.1.1.1:853#cloudflare-dns.com").unwrap();
        assert_eq!(server.addr, IpAddr::V4(Ipv4Addr::new(1, 1, 1, 1)));
        assert_eq!(server.port, 853);
        assert_eq!(server.server_name.as_deref(), Some("cloudflare-dns.com"));
    }

    #[test]
    fn test_link_dns_default_route() {
        let link = LinkDns::new(1, "lo".to_string());
        assert!(!link.default_route);
        assert!(link.dns_servers.is_empty());
        assert!(link.domains.is_empty());
        assert!(link.dnssec.is_none());
        assert!(link.dns_over_tls.is_none());
    }

    #[test]
    fn test_parse_dropin_dir_nonexistent() {
        let mut config = ResolvedConfig::default();
        config.parse_dropin_dir(Path::new("/nonexistent/path"));
        // Should not panic, just return
    }

    #[test]
    fn test_parse_dropin_dir_with_files() {
        let dir = tempfile::tempdir().unwrap();

        // Create a .conf file
        let conf_path = dir.path().join("10-custom.conf");
        fs::write(
            &conf_path,
            "[Resolve]\nDNS=10.0.0.1\nDomains=custom.example.com\n",
        )
        .unwrap();

        // Create a non-.conf file (should be ignored)
        let txt_path = dir.path().join("readme.txt");
        fs::write(&txt_path, "[Resolve]\nDNS=10.0.0.2\n").unwrap();

        let mut config = ResolvedConfig::default();
        config.parse_dropin_dir(dir.path());

        assert_eq!(config.dns.len(), 1);
        assert_eq!(config.dns[0].addr, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(config.domains, vec!["custom.example.com"]);
    }

    #[test]
    fn test_parse_dropin_dir_ordering() {
        let dir = tempfile::tempdir().unwrap();

        // Files should be processed in sorted order
        fs::write(
            dir.path().join("20-second.conf"),
            "[Resolve]\nDNS=10.0.0.2\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("10-first.conf"),
            "[Resolve]\nDNS=10.0.0.1\n",
        )
        .unwrap();

        let mut config = ResolvedConfig::default();
        config.parse_dropin_dir(dir.path());

        // 10-first.conf sets DNS=10.0.0.1, then 20-second.conf overrides with DNS=10.0.0.2
        assert_eq!(config.dns.len(), 1);
        assert_eq!(config.dns[0].addr, IpAddr::V4(Ipv4Addr::new(10, 0, 0, 2)));
    }

    // ── Negative trust anchor tests ────────────────────────────────────

    #[test]
    fn test_parse_negative_trust_anchors_basic() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Resolve]\nNegativeTrustAnchors=example.com corp.local\n");
        assert_eq!(
            config.negative_trust_anchors,
            vec!["example.com", "corp.local"]
        );
    }

    #[test]
    fn test_parse_negative_trust_anchors_empty_clears() {
        let mut config = ResolvedConfig {
            negative_trust_anchors: vec!["old.example".to_string()],
            ..Default::default()
        };
        config.parse_config("[Resolve]\nNegativeTrustAnchors=\n");
        assert!(config.negative_trust_anchors.is_empty());
    }

    #[test]
    fn test_parse_negative_trust_anchors_trailing_dot_stripped() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Resolve]\nNegativeTrustAnchors=example.com.\n");
        assert_eq!(config.negative_trust_anchors, vec!["example.com"]);
    }

    #[test]
    fn test_parse_negative_trust_anchors_lowercased() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Resolve]\nNegativeTrustAnchors=Example.COM\n");
        assert_eq!(config.negative_trust_anchors, vec!["example.com"]);
    }

    #[test]
    fn test_parse_negative_trust_anchors_multiple_whitespace() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Resolve]\nNegativeTrustAnchors=a.com   b.com\tc.com\n");
        assert_eq!(
            config.negative_trust_anchors,
            vec!["a.com", "b.com", "c.com"]
        );
    }

    #[test]
    fn test_is_negative_trust_anchor_exact_match() {
        let config = ResolvedConfig {
            negative_trust_anchors: vec!["example.com".to_string()],
            ..Default::default()
        };
        assert!(config.is_negative_trust_anchor("example.com"));
    }

    #[test]
    fn test_is_negative_trust_anchor_subdomain_match() {
        let config = ResolvedConfig {
            negative_trust_anchors: vec!["example.com".to_string()],
            ..Default::default()
        };
        assert!(config.is_negative_trust_anchor("host.example.com"));
        assert!(config.is_negative_trust_anchor("deep.host.example.com"));
    }

    #[test]
    fn test_is_negative_trust_anchor_no_match() {
        let config = ResolvedConfig {
            negative_trust_anchors: vec!["example.com".to_string()],
            ..Default::default()
        };
        assert!(!config.is_negative_trust_anchor("other.com"));
        assert!(!config.is_negative_trust_anchor("notexample.com"));
        assert!(!config.is_negative_trust_anchor("com"));
    }

    #[test]
    fn test_is_negative_trust_anchor_case_insensitive() {
        let config = ResolvedConfig {
            negative_trust_anchors: vec!["example.com".to_string()],
            ..Default::default()
        };
        assert!(config.is_negative_trust_anchor("EXAMPLE.COM"));
        assert!(config.is_negative_trust_anchor("Host.Example.COM"));
    }

    #[test]
    fn test_is_negative_trust_anchor_trailing_dot() {
        let config = ResolvedConfig {
            negative_trust_anchors: vec!["example.com".to_string()],
            ..Default::default()
        };
        assert!(config.is_negative_trust_anchor("example.com."));
        assert!(config.is_negative_trust_anchor("host.example.com."));
    }

    #[test]
    fn test_is_negative_trust_anchor_empty_list() {
        let config = ResolvedConfig::default();
        assert!(!config.is_negative_trust_anchor("example.com"));
    }

    #[test]
    fn test_is_negative_trust_anchor_multiple_anchors() {
        let config = ResolvedConfig {
            negative_trust_anchors: vec![
                "corp.local".to_string(),
                "internal.example.com".to_string(),
            ],
            ..Default::default()
        };
        assert!(config.is_negative_trust_anchor("host.corp.local"));
        assert!(config.is_negative_trust_anchor("srv.internal.example.com"));
        assert!(!config.is_negative_trust_anchor("example.com"));
        assert!(!config.is_negative_trust_anchor("public.example.com"));
    }

    #[test]
    fn test_negative_trust_anchors_default_empty() {
        let config = ResolvedConfig::default();
        assert!(config.negative_trust_anchors.is_empty());
    }

    #[test]
    fn test_parse_negative_trust_anchors_wrong_section_ignored() {
        let mut config = ResolvedConfig::default();
        config.parse_config("[Other]\nNegativeTrustAnchors=example.com\n");
        assert!(config.negative_trust_anchors.is_empty());
    }

    #[test]
    fn test_parse_negative_trust_anchors_last_value_wins() {
        let mut config = ResolvedConfig::default();
        config.parse_config(
            "[Resolve]\nNegativeTrustAnchors=first.com\nNegativeTrustAnchors=second.com\n",
        );
        assert_eq!(config.negative_trust_anchors, vec!["second.com"]);
    }
}
