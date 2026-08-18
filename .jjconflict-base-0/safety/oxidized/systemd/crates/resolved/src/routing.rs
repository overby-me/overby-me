//! DNS routing module — split DNS support.
//!
//! Real systemd-resolved routes DNS queries to different upstream servers based
//! on per-link "routing domains" (prefixed with `~` in configuration).  This
//! module implements that logic:
//!
//! - **Routing domains** (`~example.com`): queries whose name is a subdomain of
//!   `example.com` are sent to the link's DNS servers.
//! - **Catch-all routing domain** (`~.`): matches every query.
//! - **Default-route links**: links marked `DefaultRoute=yes` receive queries
//!   that don't match any specific routing domain.
//! - **Fallback**: when no routing/default-route match exists, the global DNS
//!   and then fallback DNS servers are used (matching the existing behaviour).
//!
//! Longest-suffix match wins.  If multiple links share the same longest match,
//! all their servers are merged (round-robin across links).

use std::fmt;
use std::net::SocketAddr;

use crate::config::ResolvedConfig;

// ── Route entry ────────────────────────────────────────────────────────────

/// A single routing domain → server-set mapping.
#[derive(Debug, Clone)]
struct DnsRoute {
    /// The routing domain, lowercased, no `~` prefix, no trailing dot.
    /// Empty string means catch-all (`~.`).
    domain: String,
    /// Number of labels in `domain` (0 for catch-all).
    label_count: usize,
    /// Upstream servers for this route.
    servers: Vec<SocketAddr>,
    /// Originating link name (for logging / debugging).
    link_name: String,
}

// ── Router ─────────────────────────────────────────────────────────────────

/// DNS query router that selects upstream servers based on routing domains.
///
/// Build from a [`ResolvedConfig`] via [`DnsRouter::from_config`] and call
/// [`DnsRouter::servers_for_name`] to resolve which upstreams should handle a
/// particular query name.
#[derive(Debug, Clone)]
pub struct DnsRouter {
    /// All routing-domain routes (sorted longest-suffix-first is not required;
    /// we scan linearly and pick the best match).
    routes: Vec<DnsRoute>,
    /// Servers from links that have `DefaultRoute=yes` but no routing domains.
    default_route_servers: Vec<SocketAddr>,
    /// Global DNS servers (from `resolved.conf` `DNS=`).
    global_servers: Vec<SocketAddr>,
    /// Fallback DNS servers (from `resolved.conf` `FallbackDNS=`).
    fallback_servers: Vec<SocketAddr>,
}

impl DnsRouter {
    /// Build a router from the current resolved configuration.
    pub fn from_config(config: &ResolvedConfig) -> Self {
        let mut routes: Vec<DnsRoute> = Vec::new();
        let mut default_route_servers: Vec<SocketAddr> = Vec::new();

        // Collect per-link routing domains.
        for link in &config.link_dns {
            if link.dns_servers.is_empty() {
                continue;
            }

            let link_servers: Vec<SocketAddr> =
                link.dns_servers.iter().map(|s| s.socket_addr()).collect();

            let mut has_routing_domain = false;

            for raw_domain in &link.domains {
                let trimmed = raw_domain.trim();
                if let Some(routing) = trimmed.strip_prefix('~') {
                    has_routing_domain = true;
                    let domain = normalize_domain(routing);
                    let label_count = if domain.is_empty() {
                        0 // catch-all
                    } else {
                        domain.split('.').count()
                    };
                    routes.push(DnsRoute {
                        domain,
                        label_count,
                        servers: link_servers.clone(),
                        link_name: link.ifname.clone(),
                    });
                }
                // Non-routing domains (search domains) are not added to the
                // routing table — they only affect resolv.conf generation.
            }

            // Links marked as default route that have no routing domains
            // receive unmatched queries.
            if link.default_route && !has_routing_domain {
                for s in &link_servers {
                    if !default_route_servers.contains(s) {
                        default_route_servers.push(*s);
                    }
                }
            }
        }

        // Also consider global routing domains from `Domains=` in
        // resolved.conf.  These use the global DNS servers.
        let global_dns: Vec<SocketAddr> = config.dns.iter().map(|s| s.socket_addr()).collect();
        for raw_domain in &config.domains {
            let trimmed = raw_domain.trim();
            if let Some(routing) = trimmed.strip_prefix('~')
                && !global_dns.is_empty()
            {
                let domain = normalize_domain(routing);
                let label_count = if domain.is_empty() {
                    0
                } else {
                    domain.split('.').count()
                };
                routes.push(DnsRoute {
                    domain,
                    label_count,
                    servers: global_dns.clone(),
                    link_name: "global".to_string(),
                });
            }
        }

        let fallback_dns: Vec<SocketAddr> = config
            .fallback_dns
            .iter()
            .map(|s| s.socket_addr())
            .collect();

        DnsRouter {
            routes,
            default_route_servers,
            global_servers: global_dns,
            fallback_servers: fallback_dns,
        }
    }

    /// Resolve the upstream DNS servers for a query name.
    ///
    /// Returns the list of servers that should handle this query, considering
    /// routing domains (longest-suffix match), default-route links, global
    /// DNS, and fallback DNS — in that priority order.
    pub fn servers_for_name(&self, query_name: &str) -> Vec<SocketAddr> {
        let name = normalize_domain(query_name);

        // 1. Find the best (longest-suffix) routing domain match.
        let mut best_label_count: usize = 0;
        let mut best_servers: Vec<SocketAddr> = Vec::new();
        let mut have_catch_all: Vec<SocketAddr> = Vec::new();

        for route in &self.routes {
            if route.label_count == 0 {
                // Catch-all — remember but don't prefer over specific matches.
                for s in &route.servers {
                    if !have_catch_all.contains(s) {
                        have_catch_all.push(*s);
                    }
                }
                continue;
            }

            if name_matches_domain(&name, &route.domain) {
                if route.label_count > best_label_count {
                    // New best match — replace.
                    best_label_count = route.label_count;
                    best_servers = route.servers.clone();
                } else if route.label_count == best_label_count {
                    // Same specificity — merge servers.
                    for s in &route.servers {
                        if !best_servers.contains(s) {
                            best_servers.push(*s);
                        }
                    }
                }
            }
        }

        if !best_servers.is_empty() {
            return best_servers;
        }

        // 2. Catch-all routing domain.
        if !have_catch_all.is_empty() {
            return have_catch_all;
        }

        // 3. Default-route links.
        if !self.default_route_servers.is_empty() {
            return self.default_route_servers.clone();
        }

        // 4. Global DNS servers.
        if !self.global_servers.is_empty() {
            return self.global_servers.clone();
        }

        // 5. Fallback DNS.
        self.fallback_servers.clone()
    }

    /// Build a flat list of all known upstream servers (any route).  This is
    /// used as the fallback when the query name cannot be extracted from the
    /// DNS message.
    pub fn all_servers(&self) -> Vec<SocketAddr> {
        let mut all = Vec::new();

        // Collect from routes.
        for route in &self.routes {
            for s in &route.servers {
                if !all.contains(s) {
                    all.push(*s);
                }
            }
        }

        // Default-route.
        for s in &self.default_route_servers {
            if !all.contains(s) {
                all.push(*s);
            }
        }

        // Global.
        for s in &self.global_servers {
            if !all.contains(s) {
                all.push(*s);
            }
        }

        // Fallback.
        for s in &self.fallback_servers {
            if !all.contains(s) {
                all.push(*s);
            }
        }

        all
    }

    /// Returns `true` if the router has any routing-domain entries.
    pub fn has_routing_domains(&self) -> bool {
        !self.routes.is_empty()
    }

    /// Number of routing-domain entries.
    pub fn route_count(&self) -> usize {
        self.routes.len()
    }
}

impl fmt::Display for DnsRouter {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DnsRouter({} routes, {} default-route servers, {} global, {} fallback)",
            self.routes.len(),
            self.default_route_servers.len(),
            self.global_servers.len(),
            self.fallback_servers.len(),
        )
    }
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Normalize a domain name: lowercase, strip trailing dot.
fn normalize_domain(s: &str) -> String {
    let s = s.trim().to_ascii_lowercase();
    if s == "." {
        return String::new(); // root / catch-all
    }
    s.trim_end_matches('.').to_string()
}

/// Check whether `name` is equal to or a subdomain of `domain`.
///
/// Both must be pre-normalized (lowercased, no trailing dot).
///
/// Examples:
/// - `name_matches_domain("foo.example.com", "example.com")` → true
/// - `name_matches_domain("example.com", "example.com")` → true
/// - `name_matches_domain("notexample.com", "example.com")` → false
/// - `name_matches_domain("com", "example.com")` → false
fn name_matches_domain(name: &str, domain: &str) -> bool {
    if domain.is_empty() {
        return true; // catch-all
    }
    if name == domain {
        return true; // exact match
    }
    // Check suffix: name must end with ".domain"
    if let Some(prefix) = name.strip_suffix(domain) {
        prefix.ends_with('.')
    } else {
        false
    }
}

/// Extract the query name from raw DNS message bytes.
///
/// Returns `None` if the message is too short or the name cannot be parsed.
pub fn extract_query_name(query: &[u8]) -> Option<String> {
    use crate::dns::HEADER_SIZE;

    if query.len() < HEADER_SIZE {
        return None;
    }

    let qdcount = u16::from_be_bytes([query[4], query[5]]);
    if qdcount == 0 {
        return None;
    }

    // Parse the first question name starting at offset 12.
    match crate::dns::parse_name(query, HEADER_SIZE) {
        Ok((name, _offset)) => Some(name),
        Err(_) => None,
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{DnsServer, LinkDns, ResolvedConfig};
    use std::net::{Ipv4Addr, SocketAddr};

    fn addr(ip: [u8; 4], port: u16) -> SocketAddr {
        SocketAddr::new(
            std::net::IpAddr::V4(Ipv4Addr::new(ip[0], ip[1], ip[2], ip[3])),
            port,
        )
    }

    fn dns_server(ip: [u8; 4]) -> DnsServer {
        DnsServer::new(std::net::IpAddr::V4(Ipv4Addr::new(
            ip[0], ip[1], ip[2], ip[3],
        )))
    }

    fn make_link(
        ifindex: u32,
        name: &str,
        servers: Vec<DnsServer>,
        domains: Vec<&str>,
        default_route: bool,
    ) -> LinkDns {
        let mut link = LinkDns::new(ifindex, name.to_string());
        link.dns_servers = servers;
        link.domains = domains.iter().map(|d| d.to_string()).collect();
        link.default_route = default_route;
        link
    }

    // ── normalize_domain ───────────────────────────────────────────────

    #[test]
    fn test_normalize_domain_basic() {
        assert_eq!(normalize_domain("Example.COM"), "example.com");
    }

    #[test]
    fn test_normalize_domain_trailing_dot() {
        assert_eq!(normalize_domain("example.com."), "example.com");
    }

    #[test]
    fn test_normalize_domain_root() {
        assert_eq!(normalize_domain("."), "");
    }

    #[test]
    fn test_normalize_domain_empty() {
        assert_eq!(normalize_domain(""), "");
    }

    #[test]
    fn test_normalize_domain_whitespace() {
        assert_eq!(normalize_domain("  Example.COM  "), "example.com");
    }

    // ── name_matches_domain ────────────────────────────────────────────

    #[test]
    fn test_matches_exact() {
        assert!(name_matches_domain("example.com", "example.com"));
    }

    #[test]
    fn test_matches_subdomain() {
        assert!(name_matches_domain("foo.example.com", "example.com"));
    }

    #[test]
    fn test_matches_deep_subdomain() {
        assert!(name_matches_domain("a.b.c.example.com", "example.com"));
    }

    #[test]
    fn test_no_match_different_domain() {
        assert!(!name_matches_domain("notexample.com", "example.com"));
    }

    #[test]
    fn test_no_match_shorter() {
        assert!(!name_matches_domain("com", "example.com"));
    }

    #[test]
    fn test_matches_catch_all() {
        assert!(name_matches_domain("anything.test", ""));
    }

    #[test]
    fn test_no_match_partial_label() {
        // "fooexample.com" should NOT match "example.com"
        assert!(!name_matches_domain("fooexample.com", "example.com"));
    }

    #[test]
    fn test_matches_single_label_domain() {
        assert!(name_matches_domain("host.local", "local"));
    }

    // ── DnsRouter::from_config ─────────────────────────────────────────

    #[test]
    fn test_router_empty_config() {
        let config = ResolvedConfig::default();
        let router = DnsRouter::from_config(&config);
        assert!(!router.has_routing_domains());
        assert_eq!(router.route_count(), 0);
    }

    #[test]
    fn test_router_no_routing_domains_uses_global() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            ..Default::default()
        };
        let router = DnsRouter::from_config(&config);
        assert!(!router.has_routing_domains());

        let servers = router.servers_for_name("example.com");
        assert_eq!(servers, vec![addr([8, 8, 8, 8], 53)]);
    }

    #[test]
    fn test_router_fallback_when_no_global() {
        let config = ResolvedConfig {
            dns: vec![],
            fallback_dns: vec![dns_server([1, 1, 1, 1])],
            ..Default::default()
        };
        let router = DnsRouter::from_config(&config);

        let servers = router.servers_for_name("example.com");
        assert_eq!(servers, vec![addr([1, 1, 1, 1], 53)]);
    }

    #[test]
    fn test_router_single_routing_domain() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~corp.example.com"],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        assert!(router.has_routing_domains());
        assert_eq!(router.route_count(), 1);

        // Query matching the routing domain → link server.
        let servers = router.servers_for_name("host.corp.example.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);

        // Query NOT matching → global server.
        let servers = router.servers_for_name("www.google.com");
        assert_eq!(servers, vec![addr([8, 8, 8, 8], 53)]);
    }

    #[test]
    fn test_router_exact_routing_domain_match() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~example.com"],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // Exact match on the routing domain itself.
        let servers = router.servers_for_name("example.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_longest_suffix_wins() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![
                make_link(
                    2,
                    "vpn0",
                    vec![dns_server([10, 0, 0, 1])],
                    vec!["~example.com"],
                    false,
                ),
                make_link(
                    3,
                    "vpn1",
                    vec![dns_server([10, 1, 0, 1])],
                    vec!["~corp.example.com"],
                    false,
                ),
            ],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // "host.corp.example.com" matches both, but ~corp.example.com is more
        // specific (3 labels vs 2).
        let servers = router.servers_for_name("host.corp.example.com");
        assert_eq!(servers, vec![addr([10, 1, 0, 1], 53)]);

        // "host.example.com" matches only ~example.com.
        let servers = router.servers_for_name("host.example.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_same_specificity_merges_servers() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![
                make_link(
                    2,
                    "vpn0",
                    vec![dns_server([10, 0, 0, 1])],
                    vec!["~example.com"],
                    false,
                ),
                make_link(
                    3,
                    "vpn1",
                    vec![dns_server([10, 1, 0, 1])],
                    vec!["~example.com"],
                    false,
                ),
            ],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        let servers = router.servers_for_name("host.example.com");
        assert_eq!(servers.len(), 2);
        assert!(servers.contains(&addr([10, 0, 0, 1], 53)));
        assert!(servers.contains(&addr([10, 1, 0, 1], 53)));
    }

    #[test]
    fn test_router_catch_all_routing_domain() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "vpn0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~."],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        assert!(router.has_routing_domains());

        // Every query goes to the VPN link.
        let servers = router.servers_for_name("anything.test");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_specific_beats_catch_all() {
        let config = ResolvedConfig {
            link_dns: vec![
                make_link(
                    2,
                    "vpn0",
                    vec![dns_server([10, 0, 0, 1])],
                    vec!["~."],
                    false,
                ),
                make_link(
                    3,
                    "vpn1",
                    vec![dns_server([10, 1, 0, 1])],
                    vec!["~corp.local"],
                    false,
                ),
            ],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // Specific match goes to vpn1.
        let servers = router.servers_for_name("host.corp.local");
        assert_eq!(servers, vec![addr([10, 1, 0, 1], 53)]);

        // Everything else goes to vpn0 (catch-all).
        let servers = router.servers_for_name("www.google.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_default_route_link() {
        let config = ResolvedConfig {
            dns: vec![],
            fallback_dns: vec![],
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![dns_server([10, 0, 0, 1])],
                vec![], // no routing domains
                true,   // default_route = true
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // No routing domains → falls through to default-route servers.
        let servers = router.servers_for_name("anything.test");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_default_route_not_used_when_routing_matches() {
        let config = ResolvedConfig {
            dns: vec![],
            fallback_dns: vec![],
            link_dns: vec![
                make_link(
                    2,
                    "eth0",
                    vec![dns_server([10, 0, 0, 1])],
                    vec![],
                    true, // default_route
                ),
                make_link(
                    3,
                    "vpn0",
                    vec![dns_server([10, 1, 0, 1])],
                    vec!["~corp.local"],
                    false,
                ),
            ],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // Matching routing domain → VPN server.
        let servers = router.servers_for_name("host.corp.local");
        assert_eq!(servers, vec![addr([10, 1, 0, 1], 53)]);

        // Non-matching → default-route server.
        let servers = router.servers_for_name("www.google.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_search_domains_not_routing() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["example.com"], // search domain, NOT routing domain (no ~)
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // Search domains should NOT create routing entries.
        assert!(!router.has_routing_domains());

        // Queries go to global DNS.
        let servers = router.servers_for_name("host.example.com");
        assert_eq!(servers, vec![addr([8, 8, 8, 8], 53)]);
    }

    #[test]
    fn test_router_global_routing_domains() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            domains: vec!["~internal.test".to_string()],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        assert!(router.has_routing_domains());

        // Global routing domain uses global DNS servers.
        let servers = router.servers_for_name("host.internal.test");
        assert_eq!(servers, vec![addr([8, 8, 8, 8], 53)]);
    }

    #[test]
    fn test_router_link_without_servers_ignored() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![], // no DNS servers
                vec!["~corp.local"],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // Link has no servers, so routing domain should not be registered.
        assert!(!router.has_routing_domains());
    }

    #[test]
    fn test_router_all_servers() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            fallback_dns: vec![dns_server([1, 1, 1, 1])],
            link_dns: vec![make_link(
                2,
                "vpn0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~corp.local"],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        let all = router.all_servers();
        assert!(all.contains(&addr([10, 0, 0, 1], 53)));
        assert!(all.contains(&addr([8, 8, 8, 8], 53)));
        assert!(all.contains(&addr([1, 1, 1, 1], 53)));
    }

    #[test]
    fn test_router_all_servers_deduplicates() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            fallback_dns: vec![dns_server([8, 8, 8, 8])], // same as global
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![dns_server([8, 8, 8, 8])], // also same
                vec!["~."],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        let all = router.all_servers();
        assert_eq!(all.len(), 1);
    }

    #[test]
    fn test_router_display() {
        let config = ResolvedConfig::default();
        let router = DnsRouter::from_config(&config);
        let s = format!("{}", router);
        assert!(s.starts_with("DnsRouter("));
        assert!(s.contains("routes"));
    }

    #[test]
    fn test_router_case_insensitive_matching() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "vpn0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~Corp.Example.COM"],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        let servers = router.servers_for_name("Host.CORP.Example.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_trailing_dot_domain() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "vpn0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~example.com."],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        let servers = router.servers_for_name("host.example.com.");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_multiple_routing_domains_per_link() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "vpn0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~corp.local", "~internal.test"],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        assert_eq!(router.route_count(), 2);

        let servers = router.servers_for_name("host.corp.local");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);

        let servers = router.servers_for_name("host.internal.test");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);

        // Non-matching → global.
        let servers = router.servers_for_name("www.example.com");
        assert_eq!(servers, vec![addr([8, 8, 8, 8], 53)]);
    }

    #[test]
    fn test_router_mixed_search_and_routing_domains() {
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["search.example.com", "~route.example.com"],
                false,
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        // Only the routing domain should create a route.
        assert_eq!(router.route_count(), 1);

        let servers = router.servers_for_name("host.route.example.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);

        // Search domain does NOT route.
        let servers = router.servers_for_name("host.search.example.com");
        assert_eq!(servers, vec![addr([8, 8, 8, 8], 53)]);
    }

    #[test]
    fn test_router_empty_returns_empty() {
        let config = ResolvedConfig {
            dns: vec![],
            fallback_dns: vec![],
            link_dns: vec![],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        let servers = router.servers_for_name("anything.test");
        assert!(servers.is_empty());
    }

    #[test]
    fn test_router_priority_order() {
        // Verify: routing match > catch-all > default-route > global > fallback
        let config = ResolvedConfig {
            dns: vec![dns_server([8, 8, 8, 8])],          // global
            fallback_dns: vec![dns_server([1, 1, 1, 1])], // fallback
            link_dns: vec![
                make_link(
                    2,
                    "eth0",
                    vec![dns_server([192, 168, 0, 1])],
                    vec![],
                    true, // default route
                ),
                make_link(
                    3,
                    "vpn0",
                    vec![dns_server([10, 0, 0, 1])],
                    vec!["~."], // catch-all
                    false,
                ),
                make_link(
                    4,
                    "vpn1",
                    vec![dns_server([10, 1, 0, 1])],
                    vec!["~corp.local"], // specific routing
                    false,
                ),
            ],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);

        // 1. Specific routing domain match wins.
        let servers = router.servers_for_name("host.corp.local");
        assert_eq!(servers, vec![addr([10, 1, 0, 1], 53)]);

        // 2. Non-matching goes to catch-all.
        let servers = router.servers_for_name("www.google.com");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);
    }

    #[test]
    fn test_router_default_route_with_routing_domains_not_in_default() {
        // A link that has BOTH default_route=true AND routing domains:
        // the routing domains take effect, and it should NOT be in the
        // default_route_servers pool (it has routing domains so it's
        // considered a routing link).
        let config = ResolvedConfig {
            dns: vec![],
            fallback_dns: vec![],
            link_dns: vec![make_link(
                2,
                "eth0",
                vec![dns_server([10, 0, 0, 1])],
                vec!["~corp.local"],
                true, // default_route + routing domain
            )],
            ..Default::default()
        };

        let router = DnsRouter::from_config(&config);
        assert!(router.has_routing_domains());

        // Matching → routing servers.
        let servers = router.servers_for_name("host.corp.local");
        assert_eq!(servers, vec![addr([10, 0, 0, 1], 53)]);

        // Non-matching → default route NOT populated because the link
        // has routing domains, so it should be empty.
        let servers = router.servers_for_name("www.google.com");
        assert!(servers.is_empty());
    }

    // ── extract_query_name ─────────────────────────────────────────────

    #[test]
    fn test_extract_query_name_valid() {
        use crate::hosts::tests_support::build_test_query;
        let query = build_test_query("example.com", crate::dns::RecordType::A);
        let name = extract_query_name(&query);
        assert_eq!(name, Some("example.com".to_string()));
    }

    #[test]
    fn test_extract_query_name_too_short() {
        let short = vec![0u8; 4];
        assert_eq!(extract_query_name(&short), None);
    }

    #[test]
    fn test_extract_query_name_no_questions() {
        let mut query = vec![0u8; 12];
        query[4] = 0; // QDCOUNT=0
        query[5] = 0;
        assert_eq!(extract_query_name(&query), None);
    }

    #[test]
    fn test_extract_query_name_truncated_question() {
        let mut query = vec![0u8; 12];
        query[4] = 0;
        query[5] = 1; // QDCOUNT=1
        // No actual question data after header → parse_name should fail.
        assert_eq!(extract_query_name(&query), None);
    }

    #[test]
    fn test_extract_query_name_subdomain() {
        use crate::hosts::tests_support::build_test_query;
        let query = build_test_query("host.corp.example.com", crate::dns::RecordType::AAAA);
        let name = extract_query_name(&query);
        assert_eq!(name, Some("host.corp.example.com".to_string()));
    }
}
