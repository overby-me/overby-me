//! `/etc/hosts` file reader and DNS response builder.
//!
//! This module provides an `EtcHosts` struct that parses `/etc/hosts` into
//! an in-memory lookup table and can answer DNS queries (A, AAAA, PTR, ANY)
//! directly from it — matching the behaviour of real systemd-resolved when
//! `ReadEtcHosts=yes` (the default).
//!
//! ## File format
//!
//! Each line contains an IP address followed by one or more hostnames
//! separated by whitespace.  Comments begin with `#`.  Both IPv4 and IPv6
//! addresses are supported.
//!
//! ```text
//! 127.0.0.1       localhost
//! ::1             localhost ip6-localhost ip6-loopback
//! 192.168.1.10    myhost.example.com myhost
//! ```
//!
//! ## Integration
//!
//! The resolver calls [`EtcHosts::lookup`] before forwarding a query to
//! upstream servers.  If it returns `Some(response)`, the response is sent
//! directly to the client without any upstream traffic.

use std::collections::BTreeMap;
use std::fmt;
use std::fs;
use std::io;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;
use std::time::SystemTime;

use crate::dns::{DnsMessage, HEADER_SIZE, RecordType, encode_name};

// ── Data types ────────────────────────────────────────────────────────────

/// A parsed `/etc/hosts` database.
#[derive(Debug, Clone)]
pub struct EtcHosts {
    /// Hostname → list of IPv4 addresses (lowercased key).
    ipv4: BTreeMap<String, Vec<Ipv4Addr>>,
    /// Hostname → list of IPv6 addresses (lowercased key).
    ipv6: BTreeMap<String, Vec<Ipv6Addr>>,
    /// IPv4 address → canonical hostname (first name on the line).
    reverse4: BTreeMap<Ipv4Addr, String>,
    /// IPv6 address → canonical hostname (first name on the line).
    reverse6: BTreeMap<Ipv6Addr, String>,
    /// Path to the hosts file.
    path: PathBuf,
    /// mtime of the file when it was last loaded (for change detection).
    mtime: Option<SystemTime>,
}

/// Statistics returned after parsing.
#[derive(Debug, Clone, Copy, Default)]
pub struct EtcHostsStats {
    /// Number of lines parsed (excluding blanks/comments).
    pub entries: usize,
    /// Number of distinct hostnames.
    pub hostnames: usize,
    /// Number of distinct IPv4 addresses.
    pub ipv4_addrs: usize,
    /// Number of distinct IPv6 addresses.
    pub ipv6_addrs: usize,
}

impl fmt::Display for EtcHostsStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} entries, {} hostnames, {} IPv4, {} IPv6",
            self.entries, self.hostnames, self.ipv4_addrs, self.ipv6_addrs,
        )
    }
}

// ── Parsing ───────────────────────────────────────────────────────────────

impl EtcHosts {
    /// Default path to the hosts file.
    pub const DEFAULT_PATH: &str = "/etc/hosts";

    /// Create an empty hosts database.
    pub fn new() -> Self {
        Self::with_path(Self::DEFAULT_PATH)
    }

    /// Create an empty hosts database pointing to a custom path.
    pub fn with_path(path: impl Into<PathBuf>) -> Self {
        Self {
            ipv4: BTreeMap::new(),
            ipv6: BTreeMap::new(),
            reverse4: BTreeMap::new(),
            reverse6: BTreeMap::new(),
            path: path.into(),
            mtime: None,
        }
    }

    /// Load (or reload) the hosts file from disk.
    ///
    /// Returns statistics about the loaded entries, or an error if the file
    /// cannot be read.  Parse errors on individual lines are silently skipped
    /// (matching real systemd behaviour).
    pub fn load(&mut self) -> io::Result<EtcHostsStats> {
        let content = fs::read_to_string(&self.path)?;
        let mtime = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        Ok(self.load_from_str(&content, mtime))
    }

    /// Parse hosts entries from a string (for testing or custom sources).
    pub fn load_from_str(&mut self, content: &str, mtime: Option<SystemTime>) -> EtcHostsStats {
        // Clear previous data.
        self.ipv4.clear();
        self.ipv6.clear();
        self.reverse4.clear();
        self.reverse6.clear();
        self.mtime = mtime;

        let mut entries = 0u64;

        for line in content.lines() {
            // Strip comments.
            let line = match line.split_once('#') {
                Some((before, _)) => before,
                None => line,
            };
            let line = line.trim();
            if line.is_empty() {
                continue;
            }

            let mut parts = line.split_whitespace();
            let addr_str = match parts.next() {
                Some(a) => a,
                None => continue,
            };

            // Collect all hostnames for this line.
            let names: Vec<String> = parts.map(|n| n.to_ascii_lowercase()).collect();
            if names.is_empty() {
                continue; // IP without hostnames — skip
            }

            entries += 1;

            // Parse as IPv4 first, then IPv6.
            if let Ok(v4) = addr_str.parse::<Ipv4Addr>() {
                // First name is the canonical name for reverse lookups.
                self.reverse4.entry(v4).or_insert_with(|| names[0].clone());

                for name in &names {
                    self.ipv4.entry(name.clone()).or_default().push(v4);
                }
            } else if let Ok(v6) = addr_str.parse::<Ipv6Addr>() {
                self.reverse6.entry(v6).or_insert_with(|| names[0].clone());

                for name in &names {
                    self.ipv6.entry(name.clone()).or_default().push(v6);
                }
            }
            // Lines with unparsable addresses are silently skipped.
        }

        EtcHostsStats {
            entries: entries as usize,
            hostnames: {
                let mut all: std::collections::BTreeSet<&String> =
                    std::collections::BTreeSet::new();
                for k in self.ipv4.keys() {
                    all.insert(k);
                }
                for k in self.ipv6.keys() {
                    all.insert(k);
                }
                all.len()
            },
            ipv4_addrs: self.reverse4.len(),
            ipv6_addrs: self.reverse6.len(),
        }
    }

    /// Check whether the on-disk file has changed since the last load.
    pub fn has_changed(&self) -> bool {
        let disk_mtime = fs::metadata(&self.path).and_then(|m| m.modified()).ok();
        disk_mtime != self.mtime
    }

    /// Return the number of unique hostnames.
    pub fn hostname_count(&self) -> usize {
        let mut all: std::collections::BTreeSet<&String> = std::collections::BTreeSet::new();
        for k in self.ipv4.keys() {
            all.insert(k);
        }
        for k in self.ipv6.keys() {
            all.insert(k);
        }
        all.len()
    }

    /// Return true if the database is empty.
    pub fn is_empty(&self) -> bool {
        self.ipv4.is_empty() && self.ipv6.is_empty()
    }

    /// Look up IPv4 addresses for a hostname (case-insensitive).
    pub fn lookup_v4(&self, name: &str) -> Option<&[Ipv4Addr]> {
        self.ipv4
            .get(&name.to_ascii_lowercase())
            .map(|v| v.as_slice())
    }

    /// Look up IPv6 addresses for a hostname (case-insensitive).
    pub fn lookup_v6(&self, name: &str) -> Option<&[Ipv6Addr]> {
        self.ipv6
            .get(&name.to_ascii_lowercase())
            .map(|v| v.as_slice())
    }

    /// Reverse-lookup: find the canonical hostname for an IPv4 address.
    pub fn reverse_v4(&self, addr: &Ipv4Addr) -> Option<&str> {
        self.reverse4.get(addr).map(|s| s.as_str())
    }

    /// Reverse-lookup: find the canonical hostname for an IPv6 address.
    pub fn reverse_v6(&self, addr: &Ipv6Addr) -> Option<&str> {
        self.reverse6.get(addr).map(|s| s.as_str())
    }

    // ── DNS response building ─────────────────────────────────────────

    /// Try to answer a raw DNS query from the hosts database.
    ///
    /// Returns `Some(response_bytes)` if the query matched an `/etc/hosts`
    /// entry, or `None` if the query should be forwarded upstream.
    ///
    /// Handles A (1), AAAA (28), PTR (12), and ANY (255) query types.
    pub fn lookup(&self, query: &[u8]) -> Option<Vec<u8>> {
        let msg = DnsMessage::parse(query).ok()?;
        if !msg.is_query() || msg.questions.is_empty() {
            return None;
        }

        let q = &msg.questions[0];
        let qname = q.name.to_ascii_lowercase();

        match q.qtype {
            RecordType::A => {
                let addrs = self.lookup_v4(&qname)?;
                if addrs.is_empty() {
                    return None;
                }
                Some(build_a_response(query, &qname, addrs))
            }
            RecordType::AAAA => {
                let addrs = self.lookup_v6(&qname)?;
                if addrs.is_empty() {
                    return None;
                }
                Some(build_aaaa_response(query, &qname, addrs))
            }
            RecordType::PTR => {
                let hostname = self.resolve_ptr(&qname)?;
                Some(build_ptr_response(query, &qname, hostname))
            }
            RecordType::ANY => {
                let v4 = self.lookup_v4(&qname).unwrap_or(&[]);
                let v6 = self.lookup_v6(&qname).unwrap_or(&[]);
                if v4.is_empty() && v6.is_empty() {
                    return None;
                }
                Some(build_any_response(query, &qname, v4, v6))
            }
            _ => None,
        }
    }

    /// Parse a PTR query name and look up the canonical hostname.
    ///
    /// IPv4 PTR names look like `4.3.2.1.in-addr.arpa`.
    /// IPv6 PTR names look like `<nibbles>.ip6.arpa`.
    fn resolve_ptr(&self, qname: &str) -> Option<&str> {
        let qname = qname.strip_suffix('.').unwrap_or(qname);

        if let Some(rest) = qname
            .strip_suffix(".in-addr.arpa")
            .or_else(|| qname.strip_suffix(".IN-ADDR.ARPA"))
            .or_else(|| {
                let lower = qname.to_ascii_lowercase();
                if lower.ends_with(".in-addr.arpa") {
                    // We already lowercased qname at the call site, but
                    // for robustness, handle mixed case too.
                    None
                } else {
                    None
                }
            })
        {
            // IPv4: reverse the dotted octets.
            let octets: Vec<&str> = rest.split('.').collect();
            if octets.len() == 4 {
                let addr_str = format!("{}.{}.{}.{}", octets[3], octets[2], octets[1], octets[0]);
                if let Ok(addr) = addr_str.parse::<Ipv4Addr>() {
                    return self.reverse_v4(&addr);
                }
            }
        }

        // Try case-insensitive match for in-addr.arpa
        {
            let lower = qname.to_ascii_lowercase();
            if let Some(rest) = lower.strip_suffix(".in-addr.arpa") {
                let octets: Vec<&str> = rest.split('.').collect();
                if octets.len() == 4 {
                    let addr_str =
                        format!("{}.{}.{}.{}", octets[3], octets[2], octets[1], octets[0]);
                    if let Ok(addr) = addr_str.parse::<Ipv4Addr>() {
                        return self.reverse_v4(&addr);
                    }
                }
            }

            if let Some(rest) = lower.strip_suffix(".ip6.arpa") {
                // IPv6: nibbles are separated by dots, reversed.
                let nibbles: Vec<&str> = rest.split('.').collect();
                if nibbles.len() == 32 && nibbles.iter().all(|n| n.len() == 1) {
                    // Reconstruct the IPv6 address.
                    let hex: String = nibbles.iter().rev().copied().collect();
                    // Insert colons every 4 characters.
                    let mut groups = Vec::with_capacity(8);
                    for chunk in hex.as_bytes().chunks(4) {
                        groups.push(std::str::from_utf8(chunk).unwrap_or("0000"));
                    }
                    let addr_str = groups.join(":");
                    if let Ok(addr) = addr_str.parse::<Ipv6Addr>() {
                        return self.reverse_v6(&addr);
                    }
                }
            }
        }

        None
    }
}

impl Default for EtcHosts {
    fn default() -> Self {
        Self::new()
    }
}

// ── DNS response builders ─────────────────────────────────────────────────

/// Default TTL for /etc/hosts entries (0 = do not cache, matching systemd).
const ETC_HOSTS_TTL: u32 = 0;

/// Build a DNS response header from a query.
///
/// Returns a mutable `Vec<u8>` containing the response header and the
/// original question section, ready to have answer RRs appended.
fn build_response_header(query: &[u8], ancount: u16) -> Vec<u8> {
    let mut resp = query.to_vec();
    if resp.len() < HEADER_SIZE {
        // Shouldn't happen — caller already validated, but be safe.
        resp.resize(HEADER_SIZE, 0);
    }

    // QR=1, AA=1, keep opcode and RD from query.
    resp[2] = (query[2] & 0x78) | 0x84; // QR=1 (0x80) | AA=1 (0x04), keep opcode
    // If the query had RD=1, keep it.
    if query.len() > 2 && query[2] & 0x01 != 0 {
        resp[2] |= 0x01; // RD=1
    }
    resp[3] = 0x80; // RA=1, RCODE=0 (NOERROR)

    // Set answer count.
    resp[6] = (ancount >> 8) as u8;
    resp[7] = (ancount & 0xFF) as u8;
    // Zero authority and additional counts.
    resp[8..10].copy_from_slice(&[0, 0]);
    resp[10..12].copy_from_slice(&[0, 0]);

    resp
}

/// Append an A record to a response buffer.
fn append_a_record(buf: &mut Vec<u8>, name: &str, addr: Ipv4Addr) {
    if let Ok(encoded) = encode_name(name) {
        buf.extend_from_slice(&encoded);
        buf.extend_from_slice(&RecordType::A.to_u16().to_be_bytes()); // TYPE
        buf.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN
        buf.extend_from_slice(&ETC_HOSTS_TTL.to_be_bytes()); // TTL
        buf.extend_from_slice(&4u16.to_be_bytes()); // RDLENGTH
        buf.extend_from_slice(&addr.octets()); // RDATA
    }
}

/// Append an AAAA record to a response buffer.
fn append_aaaa_record(buf: &mut Vec<u8>, name: &str, addr: Ipv6Addr) {
    if let Ok(encoded) = encode_name(name) {
        buf.extend_from_slice(&encoded);
        buf.extend_from_slice(&RecordType::AAAA.to_u16().to_be_bytes()); // TYPE
        buf.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN
        buf.extend_from_slice(&ETC_HOSTS_TTL.to_be_bytes()); // TTL
        buf.extend_from_slice(&16u16.to_be_bytes()); // RDLENGTH
        buf.extend_from_slice(&addr.octets()); // RDATA
    }
}

/// Append a PTR record to a response buffer.
fn append_ptr_record(buf: &mut Vec<u8>, qname: &str, hostname: &str) {
    if let Ok(encoded_qname) = encode_name(qname)
        && let Ok(encoded_hostname) = encode_name(hostname)
    {
        buf.extend_from_slice(&encoded_qname);
        buf.extend_from_slice(&RecordType::PTR.to_u16().to_be_bytes()); // TYPE
        buf.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN
        buf.extend_from_slice(&ETC_HOSTS_TTL.to_be_bytes()); // TTL
        let rdlength = encoded_hostname.len() as u16;
        buf.extend_from_slice(&rdlength.to_be_bytes()); // RDLENGTH
        buf.extend_from_slice(&encoded_hostname); // RDATA
    }
}

/// Build a complete A response.
fn build_a_response(query: &[u8], qname: &str, addrs: &[Ipv4Addr]) -> Vec<u8> {
    let mut resp = build_response_header(query, addrs.len() as u16);
    for addr in addrs {
        append_a_record(&mut resp, qname, *addr);
    }
    resp
}

/// Build a complete AAAA response.
fn build_aaaa_response(query: &[u8], qname: &str, addrs: &[Ipv6Addr]) -> Vec<u8> {
    let mut resp = build_response_header(query, addrs.len() as u16);
    for addr in addrs {
        append_aaaa_record(&mut resp, qname, *addr);
    }
    resp
}

/// Build a complete PTR response.
fn build_ptr_response(query: &[u8], qname: &str, hostname: &str) -> Vec<u8> {
    let mut resp = build_response_header(query, 1);
    append_ptr_record(&mut resp, qname, hostname);
    resp
}

/// Build a response with both A and AAAA records (for ANY queries).
fn build_any_response(query: &[u8], qname: &str, v4: &[Ipv4Addr], v6: &[Ipv6Addr]) -> Vec<u8> {
    let ancount = v4.len() + v6.len();
    let mut resp = build_response_header(query, ancount as u16);
    for addr in v4 {
        append_a_record(&mut resp, qname, *addr);
    }
    for addr in v6 {
        append_aaaa_record(&mut resp, qname, *addr);
    }
    resp
}

/// Build a DNS query packet for testing purposes.
#[cfg(test)]
fn build_test_query(name: &str, qtype: RecordType) -> Vec<u8> {
    let mut buf = vec![0u8; HEADER_SIZE];
    // ID = 0x1234
    buf[0] = 0x12;
    buf[1] = 0x34;
    // QR=0 (query), RD=1
    buf[2] = 0x01;
    buf[3] = 0x00;
    // QDCOUNT=1
    buf[4] = 0x00;
    buf[5] = 0x01;

    // Encode question
    if let Ok(encoded) = encode_name(name) {
        buf.extend_from_slice(&encoded);
    }
    buf.extend_from_slice(&qtype.to_u16().to_be_bytes());
    buf.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN

    buf
}

// ── Test support ──────────────────────────────────────────────────────────

/// Test helpers exposed for integration tests in other modules.
#[cfg(test)]
pub mod tests_support {
    use super::*;

    /// Build a DNS query packet for testing purposes.
    pub fn build_test_query(name: &str, qtype: RecordType) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_SIZE];
        // ID = 0x1234
        buf[0] = 0x12;
        buf[1] = 0x34;
        // QR=0 (query), RD=1
        buf[2] = 0x01;
        buf[3] = 0x00;
        // QDCOUNT=1
        buf[4] = 0x00;
        buf[5] = 0x01;

        // Encode question
        if let Ok(encoded) = encode_name(name) {
            buf.extend_from_slice(&encoded);
        }
        buf.extend_from_slice(&qtype.to_u16().to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN

        buf
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    const SAMPLE_HOSTS: &str = "\
# /etc/hosts: static lookup table
#
# <ip-address>  <hostname.domain.org>  <hostname>

127.0.0.1       localhost
::1             localhost ip6-localhost ip6-loopback
ff02::1         ip6-allnodes
ff02::2         ip6-allrouters

192.168.1.10    myhost.example.com myhost
10.0.0.1        gateway.local gateway
2001:db8::1     myhost.example.com
";

    fn make_hosts(content: &str) -> EtcHosts {
        let mut hosts = EtcHosts::new();
        hosts.load_from_str(content, None);
        hosts
    }

    // ── Parsing tests ─────────────────────────────────────────────────

    #[test]
    fn test_parse_empty() {
        let hosts = make_hosts("");
        assert!(hosts.is_empty());
        assert_eq!(hosts.hostname_count(), 0);
    }

    #[test]
    fn test_parse_comments_only() {
        let hosts = make_hosts("# this is a comment\n# another\n");
        assert!(hosts.is_empty());
    }

    #[test]
    fn test_parse_blank_lines() {
        let hosts = make_hosts("\n\n\n");
        assert!(hosts.is_empty());
    }

    #[test]
    fn test_parse_basic_ipv4() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        assert_eq!(
            hosts.lookup_v4("localhost"),
            Some([Ipv4Addr::LOCALHOST].as_slice()),
        );
    }

    #[test]
    fn test_parse_basic_ipv6() {
        let hosts = make_hosts("::1 localhost\n");
        assert_eq!(
            hosts.lookup_v6("localhost"),
            Some([Ipv6Addr::LOCALHOST].as_slice()),
        );
    }

    #[test]
    fn test_parse_multiple_names_per_line() {
        let hosts = make_hosts("192.168.1.1 alpha bravo charlie\n");
        let addr = Ipv4Addr::new(192, 168, 1, 1);
        assert_eq!(hosts.lookup_v4("alpha"), Some([addr].as_slice()));
        assert_eq!(hosts.lookup_v4("bravo"), Some([addr].as_slice()));
        assert_eq!(hosts.lookup_v4("charlie"), Some([addr].as_slice()));
    }

    #[test]
    fn test_parse_multiple_addrs_for_same_name() {
        let content = "192.168.1.1 myhost\n10.0.0.1 myhost\n";
        let hosts = make_hosts(content);
        let addrs = hosts.lookup_v4("myhost").unwrap();
        assert_eq!(addrs.len(), 2);
        assert!(addrs.contains(&Ipv4Addr::new(192, 168, 1, 1)));
        assert!(addrs.contains(&Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn test_parse_case_insensitive() {
        let hosts = make_hosts("127.0.0.1 MyHost\n");
        assert!(hosts.lookup_v4("myhost").is_some());
        assert!(hosts.lookup_v4("MYHOST").is_some());
        assert!(hosts.lookup_v4("MyHost").is_some());
    }

    #[test]
    fn test_parse_inline_comment() {
        let hosts = make_hosts("127.0.0.1 localhost # loopback\n");
        assert!(hosts.lookup_v4("localhost").is_some());
        // The "loopback" after # should NOT be a hostname.
        assert!(hosts.lookup_v4("loopback").is_none());
        // Neither should "#"
        assert!(hosts.lookup_v4("#").is_none());
    }

    #[test]
    fn test_parse_tabs_and_spaces() {
        let hosts = make_hosts("127.0.0.1\t\tlocalhost\t  myhost\n");
        assert!(hosts.lookup_v4("localhost").is_some());
        assert!(hosts.lookup_v4("myhost").is_some());
    }

    #[test]
    fn test_parse_ip_without_hostname_skipped() {
        let hosts = make_hosts("127.0.0.1\n");
        assert!(hosts.is_empty());
    }

    #[test]
    fn test_parse_invalid_ip_skipped() {
        let hosts = make_hosts("999.999.999.999 badhost\n127.0.0.1 goodhost\n");
        assert!(hosts.lookup_v4("badhost").is_none());
        assert!(hosts.lookup_v4("goodhost").is_some());
    }

    #[test]
    fn test_parse_sample_hosts() {
        let hosts = make_hosts(SAMPLE_HOSTS);

        // IPv4
        assert!(hosts.lookup_v4("localhost").is_some());
        assert!(hosts.lookup_v4("myhost.example.com").is_some());
        assert!(hosts.lookup_v4("myhost").is_some());
        assert!(hosts.lookup_v4("gateway.local").is_some());
        assert!(hosts.lookup_v4("gateway").is_some());

        // IPv6
        assert!(hosts.lookup_v6("localhost").is_some());
        assert!(hosts.lookup_v6("ip6-localhost").is_some());
        assert!(hosts.lookup_v6("ip6-loopback").is_some());
        assert!(hosts.lookup_v6("ip6-allnodes").is_some());
        assert!(hosts.lookup_v6("myhost.example.com").is_some());
    }

    #[test]
    fn test_parse_stats() {
        let mut hosts = EtcHosts::new();
        let stats = hosts.load_from_str(SAMPLE_HOSTS, None);
        assert_eq!(stats.entries, 7); // 7 non-blank non-comment lines with hostnames
        assert!(stats.hostnames > 0);
        assert!(stats.ipv4_addrs > 0);
        assert!(stats.ipv6_addrs > 0);
    }

    #[test]
    fn test_stats_display() {
        let stats = EtcHostsStats {
            entries: 10,
            hostnames: 5,
            ipv4_addrs: 3,
            ipv6_addrs: 2,
        };
        let s = format!("{}", stats);
        assert!(s.contains("10 entries"));
        assert!(s.contains("5 hostnames"));
    }

    // ── Reverse lookup tests ──────────────────────────────────────────

    #[test]
    fn test_reverse_v4() {
        let hosts = make_hosts("192.168.1.10 myhost.example.com myhost\n");
        assert_eq!(
            hosts.reverse_v4(&Ipv4Addr::new(192, 168, 1, 10)),
            Some("myhost.example.com"),
        );
    }

    #[test]
    fn test_reverse_v4_first_name_is_canonical() {
        let hosts = make_hosts("10.0.0.1 canonical alias1 alias2\n");
        assert_eq!(
            hosts.reverse_v4(&Ipv4Addr::new(10, 0, 0, 1)),
            Some("canonical"),
        );
    }

    #[test]
    fn test_reverse_v4_first_entry_wins() {
        let content = "10.0.0.1 first\n10.0.0.1 second\n";
        let hosts = make_hosts(content);
        // First line's name should be the canonical one.
        assert_eq!(hosts.reverse_v4(&Ipv4Addr::new(10, 0, 0, 1)), Some("first"),);
    }

    #[test]
    fn test_reverse_v6() {
        let hosts = make_hosts("::1 localhost ip6-localhost\n");
        assert_eq!(hosts.reverse_v6(&Ipv6Addr::LOCALHOST), Some("localhost"),);
    }

    #[test]
    fn test_reverse_not_found() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        assert_eq!(hosts.reverse_v4(&Ipv4Addr::new(10, 0, 0, 1)), None);
        assert_eq!(hosts.reverse_v6(&Ipv6Addr::LOCALHOST), None);
    }

    // ── PTR resolution ────────────────────────────────────────────────

    #[test]
    fn test_resolve_ptr_ipv4() {
        let hosts = make_hosts("192.168.1.10 myhost\n");
        assert_eq!(
            hosts.resolve_ptr("10.1.168.192.in-addr.arpa"),
            Some("myhost"),
        );
    }

    #[test]
    fn test_resolve_ptr_ipv4_with_trailing_dot() {
        let hosts = make_hosts("192.168.1.10 myhost\n");
        assert_eq!(
            hosts.resolve_ptr("10.1.168.192.in-addr.arpa."),
            Some("myhost"),
        );
    }

    #[test]
    fn test_resolve_ptr_ipv6() {
        let hosts = make_hosts("::1 localhost\n");
        // ::1 in nibble format: 1.0.0.0. ... .0.0.0.0.ip6.arpa
        let ptr = "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.ip6.arpa";
        assert_eq!(hosts.resolve_ptr(ptr), Some("localhost"));
    }

    #[test]
    fn test_resolve_ptr_not_found() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        assert_eq!(hosts.resolve_ptr("1.0.0.10.in-addr.arpa"), None,);
    }

    #[test]
    fn test_resolve_ptr_invalid_format() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        assert_eq!(hosts.resolve_ptr("not-a-ptr-name"), None);
    }

    #[test]
    fn test_resolve_ptr_too_few_octets() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        assert_eq!(hosts.resolve_ptr("1.0.0.in-addr.arpa"), None);
    }

    // ── DNS response building (integration) ───────────────────────────

    #[test]
    fn test_lookup_a_query() {
        let hosts = make_hosts("192.168.1.10 myhost\n");
        let query = build_test_query("myhost", RecordType::A);
        let response = hosts.lookup(&query).expect("should match");

        // Verify it's a valid response.
        assert!(response.len() >= HEADER_SIZE);
        // QR=1
        assert_ne!(response[2] & 0x80, 0);
        // AA=1
        assert_ne!(response[2] & 0x04, 0);
        // RCODE=0
        assert_eq!(response[3] & 0x0F, 0);
        // ANCOUNT=1
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
    }

    #[test]
    fn test_lookup_a_query_multiple_addrs() {
        let content = "192.168.1.10 myhost\n10.0.0.1 myhost\n";
        let hosts = make_hosts(content);
        let query = build_test_query("myhost", RecordType::A);
        let response = hosts.lookup(&query).expect("should match");
        // ANCOUNT=2
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 2);
    }

    #[test]
    fn test_lookup_aaaa_query() {
        let hosts = make_hosts("::1 localhost\n");
        let query = build_test_query("localhost", RecordType::AAAA);
        let response = hosts.lookup(&query).expect("should match");
        assert!(response.len() >= HEADER_SIZE);
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
    }

    #[test]
    fn test_lookup_ptr_query_ipv4() {
        let hosts = make_hosts("192.168.1.10 myhost.example.com\n");
        let query = build_test_query("10.1.168.192.in-addr.arpa", RecordType::PTR);
        let response = hosts.lookup(&query).expect("should match");
        assert!(response.len() >= HEADER_SIZE);
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
    }

    #[test]
    fn test_lookup_any_query() {
        let content = "192.168.1.10 dual\n::1 dual\n";
        let hosts = make_hosts(content);
        let query = build_test_query("dual", RecordType::ANY);
        let response = hosts.lookup(&query).expect("should match");
        // Should have both A and AAAA records.
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 2);
    }

    #[test]
    fn test_lookup_any_query_ipv4_only() {
        let hosts = make_hosts("192.168.1.10 v4only\n");
        let query = build_test_query("v4only", RecordType::ANY);
        let response = hosts.lookup(&query).expect("should match");
        assert_eq!(u16::from_be_bytes([response[6], response[7]]), 1);
    }

    #[test]
    fn test_lookup_miss() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        let query = build_test_query("unknown.host", RecordType::A);
        assert!(hosts.lookup(&query).is_none());
    }

    #[test]
    fn test_lookup_aaaa_miss_for_v4_only_host() {
        let hosts = make_hosts("192.168.1.10 v4only\n");
        let query = build_test_query("v4only", RecordType::AAAA);
        assert!(hosts.lookup(&query).is_none());
    }

    #[test]
    fn test_lookup_a_miss_for_v6_only_host() {
        let hosts = make_hosts("::1 v6only\n");
        let query = build_test_query("v6only", RecordType::A);
        assert!(hosts.lookup(&query).is_none());
    }

    #[test]
    fn test_lookup_case_insensitive() {
        let hosts = make_hosts("127.0.0.1 MyHost\n");
        let query = build_test_query("myhost", RecordType::A);
        assert!(hosts.lookup(&query).is_some());

        let query = build_test_query("MYHOST", RecordType::A);
        assert!(hosts.lookup(&query).is_some());
    }

    #[test]
    fn test_lookup_unsupported_qtype() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        let query = build_test_query("localhost", RecordType::MX);
        assert!(hosts.lookup(&query).is_none());
    }

    #[test]
    fn test_lookup_malformed_query() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        // Too short to be a valid DNS message.
        let short = vec![0u8; 4];
        assert!(hosts.lookup(&short).is_none());
    }

    #[test]
    fn test_lookup_preserves_query_id() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        let mut query = build_test_query("localhost", RecordType::A);
        query[0] = 0xAB;
        query[1] = 0xCD;
        let response = hosts.lookup(&query).expect("should match");
        // ID should match the query.
        assert_eq!(response[0], 0xAB);
        assert_eq!(response[1], 0xCD);
    }

    #[test]
    fn test_lookup_preserves_rd_flag() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        let query = build_test_query("localhost", RecordType::A);
        // Our test builder sets RD=1.
        let response = hosts.lookup(&query).expect("should match");
        // RD should be preserved.
        assert_ne!(response[2] & 0x01, 0);
    }

    #[test]
    fn test_lookup_response_has_ra() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        let query = build_test_query("localhost", RecordType::A);
        let response = hosts.lookup(&query).expect("should match");
        // RA=1
        assert_ne!(response[3] & 0x80, 0);
    }

    // ── Response builder low-level tests ──────────────────────────────

    #[test]
    fn test_build_response_header_basic() {
        let query = build_test_query("test", RecordType::A);
        let resp = build_response_header(&query, 3);
        // ANCOUNT = 3
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 3);
        // NSCOUNT = 0
        assert_eq!(u16::from_be_bytes([resp[8], resp[9]]), 0);
        // ARCOUNT = 0
        assert_eq!(u16::from_be_bytes([resp[10], resp[11]]), 0);
    }

    #[test]
    fn test_a_record_rdata() {
        let hosts = make_hosts("10.20.30.40 testhost\n");
        let query = build_test_query("testhost", RecordType::A);
        let response = hosts.lookup(&query).expect("should match");

        // Find the A record RDATA (4 bytes of IP address).
        // The response contains: header + question + answer RR.
        // We need to find the 4 bytes 10.20.30.40.
        let ip_bytes: [u8; 4] = [10, 20, 30, 40];
        let found = response.windows(4).any(|w| w == ip_bytes);
        assert!(found, "Response should contain the IP address bytes");
    }

    #[test]
    fn test_aaaa_record_rdata() {
        let hosts = make_hosts("2001:db8::1 testhost\n");
        let query = build_test_query("testhost", RecordType::AAAA);
        let response = hosts.lookup(&query).expect("should match");

        let addr: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let ip_bytes = addr.octets();
        let found = response.windows(16).any(|w| w == ip_bytes);
        assert!(found, "Response should contain the IPv6 address bytes");
    }

    // ── Reload / change detection ─────────────────────────────────────

    #[test]
    fn test_reload_clears_old_data() {
        let mut hosts = EtcHosts::new();
        hosts.load_from_str("127.0.0.1 oldhost\n", None);
        assert!(hosts.lookup_v4("oldhost").is_some());

        hosts.load_from_str("127.0.0.1 newhost\n", None);
        assert!(hosts.lookup_v4("oldhost").is_none());
        assert!(hosts.lookup_v4("newhost").is_some());
    }

    #[test]
    fn test_has_changed_no_file() {
        let hosts = EtcHosts::with_path("/nonexistent/hosts");
        // No mtime recorded and file doesn't exist — should detect change
        // (None != None is false, so this returns false).
        assert!(!hosts.has_changed());
    }

    #[test]
    fn test_has_changed_with_temp_file() {
        use std::io::Write;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        std::fs::write(&path, "127.0.0.1 test\n").unwrap();

        let mut hosts = EtcHosts::with_path(&path);
        hosts.load().unwrap();
        assert!(!hosts.has_changed());

        // Touch the file to change mtime.
        std::thread::sleep(std::time::Duration::from_millis(50));
        let mut f = fs::OpenOptions::new().write(true).open(&path).unwrap();
        f.write_all(b"127.0.0.1 test2\n").unwrap();
        f.flush().unwrap();
        drop(f);

        // mtime should differ now.
        assert!(hosts.has_changed());
    }

    #[test]
    fn test_load_nonexistent_file() {
        let mut hosts = EtcHosts::with_path("/nonexistent/hosts");
        assert!(hosts.load().is_err());
    }

    #[test]
    fn test_load_real_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("hosts");
        std::fs::write(
            &path,
            "127.0.0.1 localhost\n::1 localhost\n192.168.1.1 server\n",
        )
        .unwrap();

        let mut hosts = EtcHosts::with_path(&path);
        let stats = hosts.load().unwrap();
        assert_eq!(stats.entries, 3);
        assert!(hosts.lookup_v4("localhost").is_some());
        assert!(hosts.lookup_v6("localhost").is_some());
        assert!(hosts.lookup_v4("server").is_some());
    }

    // ── Edge cases ────────────────────────────────────────────────────

    #[test]
    fn test_parse_trailing_whitespace() {
        let hosts = make_hosts("127.0.0.1 localhost   \n");
        assert!(hosts.lookup_v4("localhost").is_some());
    }

    #[test]
    fn test_parse_leading_whitespace() {
        let hosts = make_hosts("  127.0.0.1 localhost\n");
        assert!(hosts.lookup_v4("localhost").is_some());
    }

    #[test]
    fn test_parse_no_trailing_newline() {
        let hosts = make_hosts("127.0.0.1 localhost");
        assert!(hosts.lookup_v4("localhost").is_some());
    }

    #[test]
    fn test_parse_crlf() {
        let hosts = make_hosts("127.0.0.1 localhost\r\n::1 localhost\r\n");
        assert!(hosts.lookup_v4("localhost").is_some());
        assert!(hosts.lookup_v6("localhost").is_some());
    }

    #[test]
    fn test_parse_ipv6_full() {
        let hosts = make_hosts("2001:0db8:0000:0000:0000:0000:0000:0001 full\n");
        let expected: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let addrs = hosts.lookup_v6("full").unwrap();
        assert_eq!(addrs[0], expected);
    }

    #[test]
    fn test_parse_ipv6_link_local() {
        let hosts = make_hosts("fe80::1%eth0 linklocal\n");
        // Zone IDs are not valid in /etc/hosts and Rust's parser rejects them.
        assert!(hosts.lookup_v6("linklocal").is_none());
    }

    #[test]
    fn test_default_impl() {
        let hosts = EtcHosts::default();
        assert!(hosts.is_empty());
        assert_eq!(hosts.path, Path::new("/etc/hosts"));
    }

    #[test]
    fn test_with_path() {
        let hosts = EtcHosts::with_path("/custom/hosts");
        assert_eq!(hosts.path, Path::new("/custom/hosts"));
    }

    #[test]
    fn test_ptr_ipv6_2001_db8() {
        let hosts = make_hosts("2001:db8::1 myv6host\n");
        // 2001:0db8:0000:0000:0000:0000:0000:0001 in nibble form:
        let ptr = "1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2.ip6.arpa";
        assert_eq!(hosts.resolve_ptr(ptr), Some("myv6host"));
    }

    #[test]
    fn test_lookup_any_empty() {
        let hosts = make_hosts("127.0.0.1 localhost\n");
        let query = build_test_query("nonexistent", RecordType::ANY);
        assert!(hosts.lookup(&query).is_none());
    }

    #[test]
    fn test_lookup_ptr_miss_ipv6() {
        let hosts = make_hosts("::1 localhost\n");
        // Wrong address: ::2 instead of ::1
        let ptr = "2.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.ip6.arpa";
        assert!(hosts.resolve_ptr(ptr).is_none());
    }

    #[test]
    fn test_build_test_query_valid() {
        let query = build_test_query("example.com", RecordType::A);
        let msg = DnsMessage::parse(&query).unwrap();
        assert!(msg.is_query());
        assert_eq!(msg.questions.len(), 1);
        assert_eq!(msg.questions[0].qtype, RecordType::A);
    }

    #[test]
    fn test_etc_hosts_ttl_is_zero() {
        // systemd-resolved uses TTL=0 for /etc/hosts entries to prevent
        // downstream caching of what are meant to be local overrides.
        assert_eq!(ETC_HOSTS_TTL, 0);
    }
}
