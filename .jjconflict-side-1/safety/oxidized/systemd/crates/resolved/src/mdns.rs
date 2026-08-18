#![allow(dead_code)]
//! Multicast DNS (mDNS) — RFC 6762.
//!
//! mDNS enables DNS-like name resolution on a local network segment without
//! a conventional unicast DNS server.  Hosts publish their own resource
//! records and respond to queries sent to a well-known multicast address.
//!
//! ## Addresses
//!
//! - IPv4: `224.0.0.251` port 5353
//! - IPv6: `ff02::fb` port 5353
//!
//! ## Protocol overview
//!
//! 1. A client sends a DNS-formatted query to the mDNS multicast address.
//! 2. Any host owning the requested records responds (multicast or unicast).
//! 3. The `.local` pseudo-TLD is reserved for mDNS use (RFC 6762 §3).
//! 4. Responses carry a cache-flush (CF) bit in the CLASS field to indicate
//!    that stale cached records should be replaced.
//! 5. Probing and announcing ensure uniqueness of records on the network.
//!
//! ## Scope
//!
//! mDNS is strictly link-local:
//! - IPv4 queries use TTL=255.
//! - IPv6 queries use hop-limit=255.
//! - Only the `.local` domain is resolved via mDNS.
//!
//! ## This module
//!
//! Provides:
//! - mDNS constants and message validation
//! - `MdnsResponder` — answers queries for locally published records
//! - `MdnsResolver` — sends queries and collects multicast responses
//! - Record publishing (A, AAAA, PTR, SRV, TXT)
//! - Cache-flush bit handling
//! - Socket construction helpers (IPv4 + IPv6 multicast join)
//! - Probe/announce state machine (simplified)

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::dns::{DnsHeader, HEADER_SIZE, RecordType, encode_name, parse_name};

// ── Constants ──────────────────────────────────────────────────────────────

/// mDNS port (RFC 6762 §1)
pub const MDNS_PORT: u16 = 5353;

/// mDNS IPv4 multicast address
pub const MDNS_MCAST_V4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 251);

/// mDNS IPv6 multicast address
pub const MDNS_MCAST_V6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x00fb);

/// mDNS IPv4 multicast socket address
pub const MDNS_MCAST_ADDR_V4: SocketAddr = SocketAddr::new(IpAddr::V4(MDNS_MCAST_V4), MDNS_PORT);

/// mDNS IPv6 multicast socket address
pub const MDNS_MCAST_ADDR_V6: SocketAddr = SocketAddr::new(IpAddr::V6(MDNS_MCAST_V6), MDNS_PORT);

/// Maximum mDNS UDP message size (RFC 6762 §17 — 9000 recommended)
pub const MAX_MDNS_UDP_SIZE: usize = 9000;

/// Default mDNS query timeout for one-shot resolution
pub const MDNS_QUERY_TIMEOUT: Duration = Duration::from_millis(2000);

/// Interval between initial queries (RFC 6762 §5.2: at least 1s apart)
pub const MDNS_QUERY_INTERVAL: Duration = Duration::from_secs(1);

/// Maximum number of mDNS query attempts
pub const MDNS_MAX_QUERIES: usize = 3;

/// TTL for mDNS IPv4 multicast (MUST be 255, RFC 6762 §11)
pub const MDNS_TTL_V4: u32 = 255;

/// Hop limit for mDNS IPv6 multicast (MUST be 255, RFC 6762 §11)
pub const MDNS_HOP_LIMIT_V6: u32 = 255;

/// Default TTL in mDNS responses for host records (120 seconds, RFC 6762 §11.3)
pub const MDNS_HOST_TTL: u32 = 120;

/// Default TTL for other mDNS records (75 minutes, RFC 6762 §11.3)
pub const MDNS_OTHER_TTL: u32 = 4500;

/// Goodbye TTL — a record with TTL=0 signals cache removal (RFC 6762 §10.1)
pub const MDNS_GOODBYE_TTL: u32 = 0;

/// The `.local` pseudo-TLD used by mDNS
pub const MDNS_LOCAL_DOMAIN: &str = "local";

/// Cache-flush bit mask in the CLASS field (RFC 6762 §10.2)
pub const CACHE_FLUSH_BIT: u16 = 0x8000;

/// DNS class IN
const CLASS_IN: u16 = 1;

/// Probing wait time (RFC 6762 §8.1 — 250ms between probes)
pub const PROBE_WAIT: Duration = Duration::from_millis(250);

/// Number of probe messages to send (RFC 6762 §8.1)
pub const PROBE_COUNT: usize = 3;

/// Announce interval after probing (RFC 6762 §8.3)
pub const ANNOUNCE_WAIT: Duration = Duration::from_secs(1);

/// Number of announcement messages (RFC 6762 §8.3 — at least 2)
pub const ANNOUNCE_COUNT: usize = 2;

// ── mDNS-specific validation ───────────────────────────────────────────────

/// Errors specific to mDNS message validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MdnsError {
    /// Message too short for a DNS header.
    TooShort,
    /// Non-zero opcode (must be 0 for mDNS).
    BadOpcode,
    /// Non-zero RCODE in a query.
    BadRcode,
    /// Query name could not be parsed.
    BadName,
    /// Name is not in the `.local` domain.
    NotLocalDomain,
    /// TTL != 255 in incoming packet (should be discarded per RFC 6762 §11).
    BadTtl,
}

impl fmt::Display for MdnsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(f, "mDNS message too short"),
            Self::BadOpcode => write!(f, "mDNS message has non-zero opcode"),
            Self::BadRcode => write!(f, "mDNS query has non-zero RCODE"),
            Self::BadName => write!(f, "mDNS query name could not be parsed"),
            Self::NotLocalDomain => write!(f, "mDNS query name is not in .local domain"),
            Self::BadTtl => write!(f, "mDNS message has unexpected TTL/hop-limit"),
        }
    }
}

/// Check whether a domain name is in the `.local` mDNS domain.
pub fn is_local_domain(name: &str) -> bool {
    let name = name.trim_end_matches('.').to_lowercase();
    name == MDNS_LOCAL_DOMAIN || name.ends_with(".local")
}

/// Extract the host part from a `.local` name (e.g. "myhost.local" → "myhost").
pub fn extract_local_hostname(name: &str) -> Option<&str> {
    let name = name.trim_end_matches('.');
    name.strip_suffix(".local")
        .or_else(|| {
            if name.eq_ignore_ascii_case("local") {
                Some("")
            } else {
                None
            }
        })
        .filter(|h| !h.is_empty())
}

// ── mDNS query parsing ────────────────────────────────────────────────────

/// A parsed mDNS query question.
#[derive(Debug, Clone)]
pub struct MdnsQuestion {
    /// The queried name (lowercased, no trailing dot).
    pub name: String,
    /// Query type.
    pub qtype: RecordType,
    /// Query class (with cache-flush bit masked out).
    pub qclass: u16,
    /// Whether the QU (unicast-response) bit was set (RFC 6762 §5.4).
    pub unicast_response: bool,
}

/// A parsed mDNS query message.
#[derive(Debug, Clone)]
pub struct MdnsQuery {
    /// Transaction ID (usually 0 for mDNS, but may be non-zero).
    pub id: u16,
    /// Questions in the query.
    pub questions: Vec<MdnsQuestion>,
    /// Known-answer records in the answer section (RFC 6762 §7.1).
    pub known_answer_count: u16,
    /// The raw message bytes.
    pub raw: Vec<u8>,
}

/// Parse an mDNS query message.
///
/// Unlike unicast DNS, mDNS queries may contain multiple questions and
/// may have known-answer records in the answer section (RFC 6762 §7.1).
pub fn parse_mdns_query(data: &[u8]) -> Result<MdnsQuery, MdnsError> {
    if data.len() < HEADER_SIZE {
        return Err(MdnsError::TooShort);
    }

    let header = DnsHeader::parse(data).map_err(|_| MdnsError::TooShort)?;

    // Must be a query (QR=0)
    if header.qr {
        // This is a response, not a query — not an error per se, but
        // callers should use parse_mdns_response() instead.
        return Err(MdnsError::BadOpcode);
    }

    // Opcode must be 0
    if u8::from(header.opcode) != 0 {
        return Err(MdnsError::BadOpcode);
    }

    // Parse questions
    let mut offset = HEADER_SIZE;
    let mut questions = Vec::with_capacity(header.qdcount as usize);

    for _ in 0..header.qdcount {
        if offset >= data.len() {
            return Err(MdnsError::TooShort);
        }

        let (name, name_end) = parse_name(data, offset).map_err(|_| MdnsError::BadName)?;
        if name_end + 4 > data.len() {
            return Err(MdnsError::TooShort);
        }

        let qtype_raw = u16::from_be_bytes([data[name_end], data[name_end + 1]]);
        let qclass_raw = u16::from_be_bytes([data[name_end + 2], data[name_end + 3]]);

        // The QU bit is the top bit of the QCLASS field (RFC 6762 §5.4)
        let unicast_response = (qclass_raw & CACHE_FLUSH_BIT) != 0;
        let qclass = qclass_raw & !CACHE_FLUSH_BIT;

        let trimmed = name.trim_end_matches('.').to_lowercase();

        questions.push(MdnsQuestion {
            name: trimmed,
            qtype: RecordType::from_u16(qtype_raw),
            qclass,
            unicast_response,
        });

        offset = name_end + 4;
    }

    Ok(MdnsQuery {
        id: header.id,
        questions,
        known_answer_count: header.ancount,
        raw: data.to_vec(),
    })
}

// ── mDNS resource records ──────────────────────────────────────────────────

/// An mDNS resource record for publishing.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MdnsRecord {
    /// Record name (fully qualified, e.g. "myhost.local").
    pub name: String,
    /// Record type.
    pub rtype: RecordType,
    /// Record class (normally IN=1).
    pub rclass: u16,
    /// TTL in seconds.
    pub ttl: u32,
    /// Record data.
    pub rdata: Vec<u8>,
    /// Whether this record should set the cache-flush bit when published.
    pub cache_flush: bool,
}

impl MdnsRecord {
    /// Create an A record for a `.local` hostname.
    pub fn a(hostname: &str, addr: Ipv4Addr) -> Self {
        let name = format_local_name(hostname);
        Self {
            name,
            rtype: RecordType::A,
            rclass: CLASS_IN,
            ttl: MDNS_HOST_TTL,
            rdata: addr.octets().to_vec(),
            cache_flush: true,
        }
    }

    /// Create an AAAA record for a `.local` hostname.
    pub fn aaaa(hostname: &str, addr: Ipv6Addr) -> Self {
        let name = format_local_name(hostname);
        Self {
            name,
            rtype: RecordType::AAAA,
            rclass: CLASS_IN,
            ttl: MDNS_HOST_TTL,
            rdata: addr.octets().to_vec(),
            cache_flush: true,
        }
    }

    /// Create a PTR record (e.g. for reverse lookups or service browsing).
    pub fn ptr(name: &str, target: &str, ttl: u32) -> Option<Self> {
        let rdata = encode_name(target).ok()?;
        Some(Self {
            name: name.to_lowercase(),
            rtype: RecordType::PTR,
            rclass: CLASS_IN,
            ttl,
            rdata,
            cache_flush: false, // PTR records are typically shared
        })
    }

    /// Create a SRV record for a service instance.
    pub fn srv(name: &str, priority: u16, weight: u16, port: u16, target: &str) -> Option<Self> {
        let target_encoded = encode_name(target).ok()?;
        let mut rdata = Vec::with_capacity(6 + target_encoded.len());
        rdata.extend_from_slice(&priority.to_be_bytes());
        rdata.extend_from_slice(&weight.to_be_bytes());
        rdata.extend_from_slice(&port.to_be_bytes());
        rdata.extend_from_slice(&target_encoded);
        Some(Self {
            name: name.to_lowercase(),
            rtype: RecordType::SRV,
            rclass: CLASS_IN,
            ttl: MDNS_OTHER_TTL,
            rdata,
            cache_flush: true,
        })
    }

    /// Create a TXT record for a service instance.
    pub fn txt(name: &str, entries: &[(&str, &str)]) -> Self {
        let mut rdata = Vec::new();
        for (key, value) in entries {
            let entry = if value.is_empty() {
                key.to_string()
            } else {
                format!("{}={}", key, value)
            };
            let len = entry.len().min(255);
            rdata.push(len as u8);
            rdata.extend_from_slice(&entry.as_bytes()[..len]);
        }
        // RFC 6763 §6.1: empty TXT record MUST contain a single zero byte
        if rdata.is_empty() {
            rdata.push(0);
        }
        Self {
            name: name.to_lowercase(),
            rtype: RecordType::TXT,
            rclass: CLASS_IN,
            ttl: MDNS_OTHER_TTL,
            rdata,
            cache_flush: true,
        }
    }

    /// Create a "goodbye" version of this record (TTL=0) for cache eviction.
    pub fn goodbye(&self) -> Self {
        let mut rec = self.clone();
        rec.ttl = MDNS_GOODBYE_TTL;
        rec
    }

    /// Encode this record into DNS wire format for inclusion in a message.
    ///
    /// The CLASS field will have the cache-flush bit set if `self.cache_flush`
    /// is true.
    pub fn to_wire(&self) -> Option<Vec<u8>> {
        let encoded_name = encode_name(&self.name).ok()?;
        let class_wire = if self.cache_flush {
            self.rclass | CACHE_FLUSH_BIT
        } else {
            self.rclass
        };

        let mut buf = Vec::with_capacity(encoded_name.len() + 10 + self.rdata.len());
        buf.extend_from_slice(&encoded_name);
        buf.extend_from_slice(&self.rtype.to_u16().to_be_bytes());
        buf.extend_from_slice(&class_wire.to_be_bytes());
        buf.extend_from_slice(&self.ttl.to_be_bytes());
        buf.extend_from_slice(&(self.rdata.len() as u16).to_be_bytes());
        buf.extend_from_slice(&self.rdata);

        Some(buf)
    }

    /// The wire size of this record.
    pub fn wire_size(&self) -> usize {
        // name encoding size is variable, approximate with len + 2
        self.name.len() + 2 + 10 + self.rdata.len()
    }
}

impl fmt::Display for MdnsRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} {} {} TTL={}{}",
            self.name,
            self.rtype,
            if self.rclass == CLASS_IN { "IN" } else { "??" },
            self.ttl,
            if self.cache_flush { " CF" } else { "" },
        )
    }
}

// ── Response construction ──────────────────────────────────────────────────

/// Build an mDNS response message containing the given answer records.
///
/// mDNS responses:
/// - Have QR=1, AA=1 (authoritative)
/// - ID=0 (RFC 6762 §6)
/// - QDCOUNT=0 (no question section in unsolicited responses)
pub fn build_mdns_response(records: &[MdnsRecord]) -> Option<Vec<u8>> {
    if records.is_empty() {
        return None;
    }

    // Encode all answer records
    let mut answer_data = Vec::new();
    let mut ancount: u16 = 0;
    for rec in records {
        if let Some(wire) = rec.to_wire() {
            answer_data.extend_from_slice(&wire);
            ancount = ancount.checked_add(1)?;
        }
    }

    if ancount == 0 {
        return None;
    }

    let mut buf = Vec::with_capacity(HEADER_SIZE + answer_data.len());

    // Header
    buf.extend_from_slice(&0u16.to_be_bytes()); // ID = 0
    buf.extend_from_slice(&[0x84, 0x00]); // QR=1, AA=1, RCODE=0
    buf.extend_from_slice(&0u16.to_be_bytes()); // QDCOUNT = 0
    buf.extend_from_slice(&ancount.to_be_bytes()); // ANCOUNT
    buf.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT = 0
    buf.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT = 0

    // Answer section
    buf.extend_from_slice(&answer_data);

    Some(buf)
}

/// Build an mDNS response to a specific query, including the question section.
///
/// This is used for unicast responses (QU bit set) where the response is
/// sent directly to the querier and should echo the question.
pub fn build_mdns_unicast_response(
    query_id: u16,
    questions: &[MdnsQuestion],
    answers: &[MdnsRecord],
) -> Option<Vec<u8>> {
    if answers.is_empty() {
        return None;
    }

    // Encode questions
    let mut question_data = Vec::new();
    let mut qdcount: u16 = 0;
    for q in questions {
        let encoded = encode_name(&q.name).ok()?;
        question_data.extend_from_slice(&encoded);
        question_data.extend_from_slice(&q.qtype.to_u16().to_be_bytes());
        let qclass_wire = if q.unicast_response {
            q.qclass | CACHE_FLUSH_BIT
        } else {
            q.qclass
        };
        question_data.extend_from_slice(&qclass_wire.to_be_bytes());
        qdcount = qdcount.checked_add(1)?;
    }

    // Encode answers
    let mut answer_data = Vec::new();
    let mut ancount: u16 = 0;
    for rec in answers {
        if let Some(wire) = rec.to_wire() {
            answer_data.extend_from_slice(&wire);
            ancount = ancount.checked_add(1)?;
        }
    }

    if ancount == 0 {
        return None;
    }

    let mut buf = Vec::with_capacity(HEADER_SIZE + question_data.len() + answer_data.len());

    // Header
    buf.extend_from_slice(&query_id.to_be_bytes()); // echo query ID
    buf.extend_from_slice(&[0x84, 0x00]); // QR=1, AA=1
    buf.extend_from_slice(&qdcount.to_be_bytes());
    buf.extend_from_slice(&ancount.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT = 0
    buf.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT = 0

    buf.extend_from_slice(&question_data);
    buf.extend_from_slice(&answer_data);

    Some(buf)
}

/// Build an mDNS query message.
pub fn build_mdns_query(questions: &[(String, RecordType)]) -> Option<Vec<u8>> {
    if questions.is_empty() {
        return None;
    }

    let mut question_data = Vec::new();
    let mut qdcount: u16 = 0;

    for (name, qtype) in questions {
        let encoded = encode_name(name).ok()?;
        question_data.extend_from_slice(&encoded);
        question_data.extend_from_slice(&qtype.to_u16().to_be_bytes());
        question_data.extend_from_slice(&CLASS_IN.to_be_bytes());
        qdcount = qdcount.checked_add(1)?;
    }

    let mut buf = Vec::with_capacity(HEADER_SIZE + question_data.len());

    // Header: ID=0 for mDNS queries (RFC 6762 §18.1)
    buf.extend_from_slice(&0u16.to_be_bytes());
    buf.extend_from_slice(&[0x00, 0x00]); // Flags: query
    buf.extend_from_slice(&qdcount.to_be_bytes());
    buf.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT
    buf.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT
    buf.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT

    buf.extend_from_slice(&question_data);

    Some(buf)
}

/// Build an mDNS probe message (RFC 6762 §8.1).
///
/// A probe is a query with the proposed records in the Authority section,
/// allowing other hosts to detect conflicts.
pub fn build_mdns_probe(name: &str, proposed_records: &[MdnsRecord]) -> Option<Vec<u8>> {
    let encoded_name = encode_name(name).ok()?;

    // Encode proposed records for authority section
    let mut auth_data = Vec::new();
    let mut nscount: u16 = 0;
    for rec in proposed_records {
        if let Some(wire) = rec.to_wire() {
            auth_data.extend_from_slice(&wire);
            nscount = nscount.checked_add(1)?;
        }
    }

    let mut buf = Vec::with_capacity(HEADER_SIZE + encoded_name.len() + 4 + auth_data.len());

    // Header
    buf.extend_from_slice(&0u16.to_be_bytes()); // ID = 0
    buf.extend_from_slice(&[0x00, 0x00]); // Flags: query
    buf.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT = 1
    buf.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT = 0
    buf.extend_from_slice(&nscount.to_be_bytes()); // NSCOUNT
    buf.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT = 0

    // Question: ANY type, IN class, QU bit set
    buf.extend_from_slice(&encoded_name);
    buf.extend_from_slice(&RecordType::ANY.to_u16().to_be_bytes());
    buf.extend_from_slice(&(CLASS_IN | CACHE_FLUSH_BIT).to_be_bytes()); // QU=1

    // Authority section with proposed records
    buf.extend_from_slice(&auth_data);

    Some(buf)
}

// ── Responder ──────────────────────────────────────────────────────────────

/// State of the mDNS record publication process.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublishState {
    /// Waiting to start probing.
    Idle,
    /// Currently probing for uniqueness.
    Probing { probe_count: usize },
    /// Announcing (post-probe).
    Announcing { announce_count: usize },
    /// Successfully published and responding to queries.
    Published,
    /// A conflict was detected; the name needs to be changed.
    Conflict,
}

impl fmt::Display for PublishState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Idle => write!(f, "idle"),
            Self::Probing { probe_count } => write!(f, "probing ({}/{})", probe_count, PROBE_COUNT),
            Self::Announcing { announce_count } => {
                write!(f, "announcing ({}/{})", announce_count, ANNOUNCE_COUNT)
            }
            Self::Published => write!(f, "published"),
            Self::Conflict => write!(f, "conflict"),
        }
    }
}

/// mDNS responder that manages locally published records and responds
/// to incoming mDNS queries.
pub struct MdnsResponder {
    /// The local hostname (without `.local` suffix).
    hostname: String,
    /// Published records indexed by (lowercased name, record type).
    records: HashMap<(String, u16), Vec<MdnsRecord>>,
    /// Current publication state.
    state: PublishState,
    /// When the last probe/announce was sent.
    last_action: Option<Instant>,
}

impl MdnsResponder {
    /// Create a new mDNS responder for the given hostname.
    pub fn new(hostname: &str) -> Self {
        Self {
            hostname: hostname.to_lowercase(),
            records: HashMap::new(),
            state: PublishState::Idle,
            last_action: None,
        }
    }

    /// Get the current hostname.
    pub fn hostname(&self) -> &str {
        &self.hostname
    }

    /// Get the current publication state.
    pub fn state(&self) -> PublishState {
        self.state
    }

    /// Add a record to be published.
    pub fn add_record(&mut self, record: MdnsRecord) {
        let key = (record.name.to_lowercase(), record.rtype.to_u16());
        self.records.entry(key).or_default().push(record);
        // Adding records resets to idle so we can re-probe
        if self.state == PublishState::Published {
            self.state = PublishState::Idle;
        }
    }

    /// Remove all records.
    pub fn clear_records(&mut self) {
        self.records.clear();
        self.state = PublishState::Idle;
    }

    /// Set up standard host records (A and AAAA) for the current hostname.
    pub fn set_host_records(&mut self, ipv4: &[Ipv4Addr], ipv6: &[Ipv6Addr]) {
        // Remove old host records
        let local_name = format_local_name(&self.hostname);
        let a_key = (local_name.clone(), RecordType::A.to_u16());
        let aaaa_key = (local_name.clone(), RecordType::AAAA.to_u16());
        self.records.remove(&a_key);
        self.records.remove(&aaaa_key);

        // Add new ones
        for addr in ipv4 {
            self.add_record(MdnsRecord::a(&self.hostname, *addr));
        }
        for addr in ipv6 {
            self.add_record(MdnsRecord::aaaa(&self.hostname, *addr));
        }
    }

    /// Handle an incoming mDNS query and produce a response if we have
    /// matching records.
    ///
    /// Returns `Some((response_bytes, unicast))` where `unicast` indicates
    /// whether the response should be sent via unicast (QU query) or
    /// multicast.
    pub fn handle_query(&self, data: &[u8]) -> Option<(Vec<u8>, bool)> {
        // Only respond if we're in Published state
        if self.state != PublishState::Published {
            return None;
        }

        let query = parse_mdns_query(data).ok()?;
        if query.questions.is_empty() {
            return None;
        }

        let mut answers = Vec::new();
        let mut any_unicast = false;

        for q in &query.questions {
            if q.unicast_response {
                any_unicast = true;
            }

            // Look up records matching the question
            let key = (q.name.clone(), q.qtype.to_u16());
            if let Some(recs) = self.records.get(&key) {
                answers.extend(recs.iter().cloned());
            }

            // For ANY queries, collect all records for the name
            if q.qtype == RecordType::ANY {
                for ((name, _), recs) in &self.records {
                    if *name == q.name {
                        answers.extend(recs.iter().cloned());
                    }
                }
            }
        }

        if answers.is_empty() {
            return None;
        }

        // Deduplicate
        answers.dedup_by(|a, b| a.name == b.name && a.rtype == b.rtype && a.rdata == b.rdata);

        if any_unicast {
            // Unicast response with question section
            let resp = build_mdns_unicast_response(query.id, &query.questions, &answers)?;
            Some((resp, true))
        } else {
            // Multicast response
            let resp = build_mdns_response(&answers)?;
            Some((resp, false))
        }
    }

    /// Advance the probe/announce state machine.
    ///
    /// Returns an optional message to send (multicast) and the recommended
    /// delay before calling `advance()` again.
    pub fn advance(&mut self) -> (Option<Vec<u8>>, Duration) {
        let now = Instant::now();

        match self.state {
            PublishState::Idle => {
                self.state = PublishState::Probing { probe_count: 0 };
                self.last_action = Some(now);
                // Build and send first probe
                let msg = self.build_probe();
                (msg, PROBE_WAIT)
            }
            PublishState::Probing { probe_count } => {
                if let Some(last) = self.last_action
                    && now.duration_since(last) < PROBE_WAIT
                {
                    let remaining = PROBE_WAIT - now.duration_since(last);
                    return (None, remaining);
                }

                let next_count = probe_count + 1;
                if next_count >= PROBE_COUNT {
                    // Probing complete, start announcing
                    self.state = PublishState::Announcing { announce_count: 0 };
                    self.last_action = Some(now);
                    let msg = self.build_announcement();
                    (msg, ANNOUNCE_WAIT)
                } else {
                    self.state = PublishState::Probing {
                        probe_count: next_count,
                    };
                    self.last_action = Some(now);
                    let msg = self.build_probe();
                    (msg, PROBE_WAIT)
                }
            }
            PublishState::Announcing { announce_count } => {
                if let Some(last) = self.last_action
                    && now.duration_since(last) < ANNOUNCE_WAIT
                {
                    let remaining = ANNOUNCE_WAIT - now.duration_since(last);
                    return (None, remaining);
                }

                let next_count = announce_count + 1;
                if next_count >= ANNOUNCE_COUNT {
                    self.state = PublishState::Published;
                    self.last_action = Some(now);
                    (None, Duration::from_secs(60))
                } else {
                    self.state = PublishState::Announcing {
                        announce_count: next_count,
                    };
                    self.last_action = Some(now);
                    let msg = self.build_announcement();
                    (msg, ANNOUNCE_WAIT)
                }
            }
            PublishState::Published => (None, Duration::from_secs(60)),
            PublishState::Conflict => (None, Duration::from_secs(1)),
        }
    }

    /// Signal that a conflict was detected.
    pub fn signal_conflict(&mut self) {
        self.state = PublishState::Conflict;
    }

    /// Build goodbye messages for all records (TTL=0).
    pub fn build_goodbye(&self) -> Option<Vec<u8>> {
        let goodbyes: Vec<MdnsRecord> = self
            .records
            .values()
            .flatten()
            .map(|r| r.goodbye())
            .collect();

        if goodbyes.is_empty() {
            return None;
        }

        build_mdns_response(&goodbyes)
    }

    /// Get all published records.
    pub fn all_records(&self) -> Vec<&MdnsRecord> {
        self.records.values().flatten().collect()
    }

    /// Get the record count.
    pub fn record_count(&self) -> usize {
        self.records.values().map(|v| v.len()).sum()
    }

    /// Build a probe message for our hostname.
    fn build_probe(&self) -> Option<Vec<u8>> {
        let local_name = format_local_name(&self.hostname);
        let proposed: Vec<MdnsRecord> = self.records.values().flatten().cloned().collect();
        build_mdns_probe(&local_name, &proposed)
    }

    /// Build an announcement (unsolicited response with all our records).
    fn build_announcement(&self) -> Option<Vec<u8>> {
        let all: Vec<MdnsRecord> = self.records.values().flatten().cloned().collect();
        build_mdns_response(&all)
    }
}

// ── Resolver ───────────────────────────────────────────────────────────────

/// Result of an mDNS resolution.
#[derive(Debug, Clone)]
pub struct MdnsResult {
    /// The name that was queried.
    pub name: String,
    /// Resolved IPv4 addresses.
    pub ipv4: Vec<Ipv4Addr>,
    /// Resolved IPv6 addresses.
    pub ipv6: Vec<Ipv6Addr>,
    /// Additional records received.
    pub additional_records: Vec<MdnsRecord>,
    /// Source address of the responder(s).
    pub responders: Vec<SocketAddr>,
}

/// mDNS resolver that sends multicast queries and collects responses.
pub struct MdnsResolver {
    /// Query timeout per attempt.
    timeout: Duration,
    /// Maximum number of query attempts.
    max_queries: usize,
}

impl MdnsResolver {
    /// Create a new mDNS resolver with default settings.
    pub fn new() -> Self {
        Self {
            timeout: MDNS_QUERY_TIMEOUT,
            max_queries: MDNS_MAX_QUERIES,
        }
    }

    /// Set the query timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Set the maximum number of query attempts.
    pub fn with_max_queries(mut self, max: usize) -> Self {
        self.max_queries = max.max(1);
        self
    }

    /// Build an mDNS query for a `.local` name.
    ///
    /// Automatically appends `.local` if not present.
    pub fn build_query(&self, name: &str, qtype: RecordType) -> Option<Vec<u8>> {
        let local_name = if is_local_domain(name) {
            name.trim_end_matches('.').to_lowercase()
        } else {
            format!("{}.local", name.trim_end_matches('.').to_lowercase())
        };

        build_mdns_query(&[(local_name, qtype)])
    }

    /// Resolve a `.local` name using a provided UDP socket.
    ///
    /// Sends the query to the mDNS multicast address and collects responses.
    pub fn resolve(
        &self,
        socket: &UdpSocket,
        name: &str,
        dest: SocketAddr,
    ) -> io::Result<MdnsResult> {
        let local_name = if is_local_domain(name) {
            name.trim_end_matches('.').to_lowercase()
        } else {
            format!("{}.local", name.trim_end_matches('.').to_lowercase())
        };

        // Build A and AAAA queries
        let query_a = build_mdns_query(&[(local_name.clone(), RecordType::A)]);
        let query_aaaa = build_mdns_query(&[(local_name.clone(), RecordType::AAAA)]);

        let mut result = MdnsResult {
            name: local_name,
            ipv4: Vec::new(),
            ipv6: Vec::new(),
            additional_records: Vec::new(),
            responders: Vec::new(),
        };

        socket.set_read_timeout(Some(self.timeout))?;

        for _ in 0..self.max_queries {
            if let Some(ref q) = query_a {
                socket.send_to(q, dest)?;
            }
            if let Some(ref q) = query_aaaa {
                socket.send_to(q, dest)?;
            }

            let deadline = Instant::now() + self.timeout;
            let mut buf = [0u8; MAX_MDNS_UDP_SIZE];

            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let _ = socket.set_read_timeout(Some(remaining));

                match socket.recv_from(&mut buf) {
                    Ok((len, src)) => {
                        if let Some(parsed) = parse_mdns_response_records(&buf[..len]) {
                            result.responders.push(src);
                            for rec in parsed {
                                match rec.rtype {
                                    RecordType::A if rec.rdata.len() == 4 => {
                                        let addr = Ipv4Addr::new(
                                            rec.rdata[0],
                                            rec.rdata[1],
                                            rec.rdata[2],
                                            rec.rdata[3],
                                        );
                                        if !result.ipv4.contains(&addr) {
                                            result.ipv4.push(addr);
                                        }
                                    }
                                    RecordType::AAAA if rec.rdata.len() == 16 => {
                                        let mut octets = [0u8; 16];
                                        octets.copy_from_slice(&rec.rdata);
                                        let addr = Ipv6Addr::from(octets);
                                        if !result.ipv6.contains(&addr) {
                                            result.ipv6.push(addr);
                                        }
                                    }
                                    _ => {
                                        result.additional_records.push(rec);
                                    }
                                }
                            }
                        }
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(e) => return Err(e),
                }
            }

            if !result.ipv4.is_empty() || !result.ipv6.is_empty() {
                break;
            }
        }

        Ok(result)
    }
}

impl Default for MdnsResolver {
    fn default() -> Self {
        Self::new()
    }
}

// ── Response parsing ───────────────────────────────────────────────────────

/// Parse resource records from an mDNS response message.
fn parse_mdns_response_records(data: &[u8]) -> Option<Vec<MdnsRecord>> {
    if data.len() < HEADER_SIZE {
        return None;
    }

    let header = DnsHeader::parse(data).ok()?;

    // Must be a response
    if !header.qr {
        return None;
    }

    let mut offset = HEADER_SIZE;

    // Skip question section
    for _ in 0..header.qdcount {
        let (_, name_end) = parse_name(data, offset).ok()?;
        offset = name_end + 4;
        if offset > data.len() {
            return None;
        }
    }

    // Parse answer + authority + additional records
    let total_rrs = header.ancount as usize + header.nscount as usize + header.arcount as usize;
    let mut records = Vec::new();

    for _ in 0..total_rrs {
        if offset >= data.len() {
            break;
        }

        let (name, name_end) = parse_name(data, offset).ok()?;
        if name_end + 10 > data.len() {
            break;
        }

        let rtype = u16::from_be_bytes([data[name_end], data[name_end + 1]]);
        let rclass_raw = u16::from_be_bytes([data[name_end + 2], data[name_end + 3]]);
        let ttl = u32::from_be_bytes([
            data[name_end + 4],
            data[name_end + 5],
            data[name_end + 6],
            data[name_end + 7],
        ]);
        let rdlen = u16::from_be_bytes([data[name_end + 8], data[name_end + 9]]) as usize;
        let rdata_start = name_end + 10;
        let rdata_end = rdata_start + rdlen;

        if rdata_end > data.len() {
            break;
        }

        let cache_flush = (rclass_raw & CACHE_FLUSH_BIT) != 0;
        let rclass = rclass_raw & !CACHE_FLUSH_BIT;

        records.push(MdnsRecord {
            name: name.trim_end_matches('.').to_lowercase(),
            rtype: RecordType::from_u16(rtype),
            rclass,
            ttl,
            rdata: data[rdata_start..rdata_end].to_vec(),
            cache_flush,
        });

        offset = rdata_end;
    }

    Some(records)
}

// ── Socket helpers ─────────────────────────────────────────────────────────

/// Bind a UDP socket for mDNS on the IPv4 multicast address.
pub fn bind_mdns_ipv4() -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        MDNS_PORT,
    ))?;
    socket.set_multicast_loop_v4(true)?; // mDNS needs loopback for local queries
    socket.set_multicast_ttl_v4(MDNS_TTL_V4)?;
    socket.join_multicast_v4(&MDNS_MCAST_V4, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_nonblocking(false)?;
    Ok(socket)
}

/// Bind a UDP socket for mDNS on the IPv6 multicast address.
pub fn bind_mdns_ipv6() -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddr::new(
        IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        MDNS_PORT,
    ))?;
    socket.set_multicast_loop_v6(true)?;
    socket.join_multicast_v6(&MDNS_MCAST_V6, 0)?;
    socket.set_nonblocking(false)?;
    Ok(socket)
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Format a hostname into a `.local` FQDN.
fn format_local_name(hostname: &str) -> String {
    let hostname = hostname.trim_end_matches('.').to_lowercase();
    if is_local_domain(&hostname) {
        hostname
    } else {
        format!("{}.{}", hostname, MDNS_LOCAL_DOMAIN)
    }
}

/// Construct the reverse (PTR) name for an IPv4 address in `.local`.
/// This uses the standard `in-addr.arpa` format, not mDNS-specific.
pub fn reverse_ipv4_name(addr: Ipv4Addr) -> String {
    let o = addr.octets();
    format!("{}.{}.{}.{}.in-addr.arpa", o[3], o[2], o[1], o[0])
}

/// Construct the reverse (PTR) name for an IPv6 address.
pub fn reverse_ipv6_name(addr: Ipv6Addr) -> String {
    let octets = addr.octets();
    let mut nibbles = Vec::with_capacity(64);
    for byte in octets.iter().rev() {
        nibbles.push(format!("{:x}", byte & 0x0f));
        nibbles.push(format!("{:x}", (byte >> 4) & 0x0f));
    }
    format!("{}.ip6.arpa", nibbles.join("."))
}

// ── Statistics ──────────────────────────────────────────────────────────────

/// mDNS daemon statistics.
#[derive(Debug, Clone, Default)]
pub struct MdnsStats {
    /// Number of mDNS queries received.
    pub queries_received: u64,
    /// Number of queries we responded to.
    pub queries_answered: u64,
    /// Number of queries ignored (not our records).
    pub queries_ignored: u64,
    /// Number of outgoing mDNS resolution queries sent.
    pub resolution_queries_sent: u64,
    /// Number of resolution responses received.
    pub resolution_responses: u64,
    /// Number of probes sent.
    pub probes_sent: u64,
    /// Number of announcements sent.
    pub announcements_sent: u64,
    /// Number of conflicts detected.
    pub conflicts_detected: u64,
    /// Number of goodbyes sent.
    pub goodbyes_sent: u64,
}

impl fmt::Display for MdnsStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mDNS: {} received, {} answered, {} ignored, {} resolutions, {} conflicts, {} goodbyes",
            self.queries_received,
            self.queries_answered,
            self.queries_ignored,
            self.resolution_queries_sent,
            self.conflicts_detected,
            self.goodbyes_sent,
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Helper ─────────────────────────────────────────────────────────

    /// Build a minimal mDNS query for testing.
    fn build_test_mdns_query(name: &str, qtype: u16) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);

        // Header: ID=0, flags=0, QDCOUNT=1
        buf.extend_from_slice(&[0x00, 0x00]); // ID
        buf.extend_from_slice(&[0x00, 0x00]); // Flags
        buf.extend_from_slice(&[0x00, 0x01]); // QDCOUNT
        buf.extend_from_slice(&[0x00, 0x00]); // ANCOUNT
        buf.extend_from_slice(&[0x00, 0x00]); // NSCOUNT
        buf.extend_from_slice(&[0x00, 0x00]); // ARCOUNT

        // Question: encode name
        for label in name.split('.') {
            if label.is_empty() {
                continue;
            }
            buf.push(label.len() as u8);
            buf.extend_from_slice(label.as_bytes());
        }
        buf.push(0x00); // root

        buf.extend_from_slice(&qtype.to_be_bytes()); // QTYPE
        buf.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = IN

        buf
    }

    // ── is_local_domain tests ──────────────────────────────────────────

    #[test]
    fn test_is_local_domain() {
        assert!(is_local_domain("myhost.local"));
        assert!(is_local_domain("myhost.local."));
        assert!(is_local_domain("sub.host.local"));
        assert!(is_local_domain("local"));
        assert!(is_local_domain("LOCAL"));
        assert!(is_local_domain("MyHost.LOCAL"));
        assert!(!is_local_domain("example.com"));
        assert!(!is_local_domain("notlocal"));
        assert!(!is_local_domain("local.example.com"));
    }

    #[test]
    fn test_extract_local_hostname() {
        assert_eq!(extract_local_hostname("myhost.local"), Some("myhost"));
        assert_eq!(extract_local_hostname("myhost.local."), Some("myhost"));
        assert_eq!(extract_local_hostname("sub.host.local"), Some("sub.host"));
        assert_eq!(extract_local_hostname("local"), None); // empty host part filtered
        assert_eq!(extract_local_hostname("example.com"), None);
    }

    // ── MdnsError display tests ────────────────────────────────────────

    #[test]
    fn test_error_display() {
        assert_eq!(format!("{}", MdnsError::TooShort), "mDNS message too short");
        assert_eq!(
            format!("{}", MdnsError::BadOpcode),
            "mDNS message has non-zero opcode"
        );
        assert_eq!(
            format!("{}", MdnsError::BadRcode),
            "mDNS query has non-zero RCODE"
        );
        assert_eq!(
            format!("{}", MdnsError::BadName),
            "mDNS query name could not be parsed"
        );
        assert_eq!(
            format!("{}", MdnsError::NotLocalDomain),
            "mDNS query name is not in .local domain"
        );
        assert_eq!(
            format!("{}", MdnsError::BadTtl),
            "mDNS message has unexpected TTL/hop-limit"
        );
    }

    // ── parse_mdns_query tests ─────────────────────────────────────────

    #[test]
    fn test_parse_query_basic() {
        let data = build_test_mdns_query("myhost.local", 1);
        let query = parse_mdns_query(&data).unwrap();
        assert_eq!(query.questions.len(), 1);
        assert_eq!(query.questions[0].name, "myhost.local");
        assert_eq!(query.questions[0].qtype, RecordType::A);
        assert_eq!(query.questions[0].qclass, CLASS_IN);
        assert!(!query.questions[0].unicast_response);
    }

    #[test]
    fn fuzz_mdns_parsers_never_panic() {
        // Robustness net (task #22): the mDNS parsers consume attacker-
        // controlled multicast packets, so parse_mdns_query and the response
        // record parser must never panic or hang on malformed input.
        // Deterministic seeded mutation of a valid query + random packets, with
        // injected 0xC0 compression pointers; each parse in catch_unwind on a
        // worker thread joined under a wall-clock budget (vs infinite loops).
        let handle = std::thread::spawn(|| {
            let seed = build_test_mdns_query("myhost.local", 1);
            let mut state: u64 = 0x00c0_ffee_1234_5678;
            let mut next = || {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u32
            };
            for iter in 0..50_000u32 {
                let mut buf = if iter % 2 == 0 {
                    seed.clone()
                } else {
                    let len = (next() % 128) as usize;
                    (0..len).map(|_| (next() & 0xff) as u8).collect::<Vec<u8>>()
                };
                let muts = 1 + (next() % 10) as usize;
                for _ in 0..muts {
                    if buf.is_empty() {
                        break;
                    }
                    match next() % 6 {
                        0 => {
                            let i = (next() as usize) % buf.len();
                            buf[i] ^= (next() & 0xff) as u8;
                        }
                        1 => {
                            let i = (next() as usize) % buf.len();
                            buf.remove(i);
                        }
                        2 => {
                            let i = (next() as usize) % (buf.len() + 1);
                            buf.insert(i, (next() & 0xff) as u8);
                        }
                        3 => {
                            let i = (next() as usize) % buf.len();
                            buf.truncate(i);
                        }
                        4 => {
                            let i = (next() as usize) % buf.len();
                            buf[i] = 0xC0;
                        }
                        _ => {
                            let i = (next() as usize) % buf.len();
                            let b = buf[i];
                            buf.insert(i, b);
                        }
                    }
                }
                let input = buf.clone();
                let res = std::panic::catch_unwind(move || {
                    let _ = parse_mdns_query(&input);
                    let _ = parse_mdns_response_records(&input);
                });
                assert!(res.is_ok(), "mdns parser panicked on: {buf:?}");
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "mdns fuzz did not finish in 30s -- a malformed packet hangs the parser"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        handle.join().expect("fuzz worker panicked");
    }

    #[test]
    fn test_parse_query_aaaa() {
        let data = build_test_mdns_query("printer.local", 28);
        let query = parse_mdns_query(&data).unwrap();
        assert_eq!(query.questions[0].qtype, RecordType::AAAA);
    }

    #[test]
    fn test_parse_query_qu_bit() {
        let mut data = build_test_mdns_query("host.local", 1);
        // Set the QU bit (top bit of QCLASS)
        let last = data.len();
        data[last - 2] |= 0x80;
        let query = parse_mdns_query(&data).unwrap();
        assert!(query.questions[0].unicast_response);
        assert_eq!(query.questions[0].qclass, CLASS_IN); // QU bit stripped
    }

    #[test]
    fn test_parse_query_too_short() {
        assert_eq!(parse_mdns_query(&[0; 5]).unwrap_err(), MdnsError::TooShort);
    }

    #[test]
    fn test_parse_query_response_rejected() {
        let mut data = build_test_mdns_query("host.local", 1);
        data[2] |= 0x80; // QR=1
        assert!(parse_mdns_query(&data).is_err());
    }

    #[test]
    fn test_parse_query_bad_opcode() {
        let mut data = build_test_mdns_query("host.local", 1);
        data[2] |= 0x08; // opcode=1
        assert_eq!(parse_mdns_query(&data).unwrap_err(), MdnsError::BadOpcode);
    }

    #[test]
    fn test_parse_query_case_insensitive() {
        let data = build_test_mdns_query("MyHost.LOCAL", 1);
        let query = parse_mdns_query(&data).unwrap();
        assert_eq!(query.questions[0].name, "myhost.local");
    }

    // ── MdnsRecord tests ───────────────────────────────────────────────

    #[test]
    fn test_record_a() {
        let rec = MdnsRecord::a("myhost", Ipv4Addr::new(192, 168, 1, 10));
        assert_eq!(rec.name, "myhost.local");
        assert_eq!(rec.rtype, RecordType::A);
        assert_eq!(rec.rdata, vec![192, 168, 1, 10]);
        assert_eq!(rec.ttl, MDNS_HOST_TTL);
        assert!(rec.cache_flush);
    }

    #[test]
    fn test_record_aaaa() {
        let addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        let rec = MdnsRecord::aaaa("myhost", addr);
        assert_eq!(rec.name, "myhost.local");
        assert_eq!(rec.rtype, RecordType::AAAA);
        assert_eq!(rec.rdata.len(), 16);
        assert!(rec.cache_flush);
    }

    #[test]
    fn test_record_ptr() {
        let rec = MdnsRecord::ptr(
            "_http._tcp.local",
            "My Web._http._tcp.local",
            MDNS_OTHER_TTL,
        )
        .unwrap();
        assert_eq!(rec.rtype, RecordType::PTR);
        assert!(!rec.cache_flush); // PTR records are shared
    }

    #[test]
    fn test_record_srv() {
        let rec = MdnsRecord::srv("My Web._http._tcp.local", 0, 0, 80, "myhost.local").unwrap();
        assert_eq!(rec.rtype, RecordType::SRV);
        assert!(rec.cache_flush);
        // SRV RDATA: priority(2) + weight(2) + port(2) + target name
        assert!(rec.rdata.len() >= 6);
    }

    #[test]
    fn test_record_txt() {
        let rec = MdnsRecord::txt(
            "My Web._http._tcp.local",
            &[("path", "/index.html"), ("version", "1.0")],
        );
        assert_eq!(rec.rtype, RecordType::TXT);
        assert!(rec.cache_flush);
        assert!(!rec.rdata.is_empty());
    }

    #[test]
    fn test_record_txt_empty() {
        let rec = MdnsRecord::txt("service.local", &[]);
        // RFC 6763 §6.1: must contain at least one byte (zero)
        assert_eq!(rec.rdata, vec![0]);
    }

    #[test]
    fn test_record_goodbye() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let goodbye = rec.goodbye();
        assert_eq!(goodbye.ttl, MDNS_GOODBYE_TTL);
        assert_eq!(goodbye.name, rec.name);
        assert_eq!(goodbye.rdata, rec.rdata);
    }

    #[test]
    fn test_record_to_wire() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let wire = rec.to_wire().unwrap();
        // Should contain encoded name + TYPE(2) + CLASS(2) + TTL(4) + RDLEN(2) + RDATA(4)
        assert!(wire.len() > 14);
    }

    #[test]
    fn test_record_to_wire_cache_flush_bit() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let wire = rec.to_wire().unwrap();

        // Find the CLASS field: after encoded name + TYPE(2)
        // "host.local" → 4+host + 5+local + 0 = 12 bytes for name
        // The name encoding for "host.local" is: \x04host\x05local\x00
        let name_len = 4 + 1 + 5 + 1 + 1; // label(4) + "host" + label(5) + "local" + root
        let class_offset = name_len + 2; // after TYPE
        let class_val = u16::from_be_bytes([wire[class_offset], wire[class_offset + 1]]);
        assert_ne!(
            class_val & CACHE_FLUSH_BIT,
            0,
            "cache-flush bit should be set"
        );
    }

    #[test]
    fn test_record_display() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let s = format!("{}", rec);
        assert!(s.contains("host.local"));
        assert!(s.contains("A"));
        assert!(s.contains("IN"));
        assert!(s.contains("CF"));
    }

    #[test]
    fn test_record_display_no_cache_flush() {
        let rec = MdnsRecord::ptr("_http._tcp.local", "svc.local", 4500).unwrap();
        let s = format!("{}", rec);
        assert!(!s.contains("CF"));
    }

    // ── Response building tests ────────────────────────────────────────

    #[test]
    fn test_build_response_single_record() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let resp = build_mdns_response(&[rec]).unwrap();

        assert!(resp.len() > HEADER_SIZE);
        // QR=1, AA=1
        assert_eq!(resp[2] & 0x80, 0x80); // QR
        assert_eq!(resp[2] & 0x04, 0x04); // AA
        // ID=0
        assert_eq!(u16::from_be_bytes([resp[0], resp[1]]), 0);
        // QDCOUNT=0
        assert_eq!(u16::from_be_bytes([resp[4], resp[5]]), 0);
        // ANCOUNT=1
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 1);
    }

    #[test]
    fn test_build_response_multiple_records() {
        let a = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let aaaa = MdnsRecord::aaaa("host", Ipv6Addr::LOCALHOST);
        let resp = build_mdns_response(&[a, aaaa]).unwrap();
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 2);
    }

    #[test]
    fn test_build_response_empty() {
        assert!(build_mdns_response(&[]).is_none());
    }

    #[test]
    fn test_build_unicast_response() {
        let q = MdnsQuestion {
            name: "host.local".to_string(),
            qtype: RecordType::A,
            qclass: CLASS_IN,
            unicast_response: true,
        };
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let resp = build_mdns_unicast_response(0x1234, &[q], &[rec]).unwrap();

        // ID should echo query
        assert_eq!(u16::from_be_bytes([resp[0], resp[1]]), 0x1234);
        // QDCOUNT=1
        assert_eq!(u16::from_be_bytes([resp[4], resp[5]]), 1);
        // ANCOUNT=1
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 1);
    }

    #[test]
    fn test_build_unicast_response_empty_answers() {
        let q = MdnsQuestion {
            name: "host.local".to_string(),
            qtype: RecordType::A,
            qclass: CLASS_IN,
            unicast_response: true,
        };
        assert!(build_mdns_unicast_response(0, &[q], &[]).is_none());
    }

    #[test]
    fn test_build_mdns_query() {
        let msg = build_mdns_query(&[("host.local".to_string(), RecordType::A)]).unwrap();
        assert!(msg.len() > HEADER_SIZE);
        // ID=0
        assert_eq!(u16::from_be_bytes([msg[0], msg[1]]), 0);
        // QR=0
        assert_eq!(msg[2] & 0x80, 0);
        // QDCOUNT=1
        assert_eq!(u16::from_be_bytes([msg[4], msg[5]]), 1);
    }

    #[test]
    fn test_build_mdns_query_empty() {
        assert!(build_mdns_query(&[]).is_none());
    }

    #[test]
    fn test_build_probe() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let probe = build_mdns_probe("host.local", &[rec]).unwrap();
        assert!(probe.len() > HEADER_SIZE);
        // QDCOUNT=1
        assert_eq!(u16::from_be_bytes([probe[4], probe[5]]), 1);
        // NSCOUNT=1
        assert_eq!(u16::from_be_bytes([probe[8], probe[9]]), 1);
    }

    // ── Responder tests ────────────────────────────────────────────────

    #[test]
    fn test_responder_new() {
        let resp = MdnsResponder::new("MyHost");
        assert_eq!(resp.hostname(), "myhost");
        assert_eq!(resp.state(), PublishState::Idle);
        assert_eq!(resp.record_count(), 0);
    }

    #[test]
    fn test_responder_add_record() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));
        assert_eq!(resp.record_count(), 1);
    }

    #[test]
    fn test_responder_set_host_records() {
        let mut resp = MdnsResponder::new("host");
        resp.set_host_records(
            &[Ipv4Addr::new(10, 0, 0, 1)],
            &[Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)],
        );
        assert_eq!(resp.record_count(), 2);
    }

    #[test]
    fn test_responder_clear_records() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::LOCALHOST));
        resp.clear_records();
        assert_eq!(resp.record_count(), 0);
        assert_eq!(resp.state(), PublishState::Idle);
    }

    #[test]
    fn test_responder_handle_query_when_not_published() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));
        // State is Idle, not Published — should not respond
        let query = build_test_mdns_query("host.local", 1);
        assert!(resp.handle_query(&query).is_none());
    }

    #[test]
    fn test_responder_handle_query_when_published() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));
        resp.state = PublishState::Published;

        let query = build_test_mdns_query("host.local", 1);
        let (response, unicast) = resp.handle_query(&query).unwrap();
        assert!(!unicast);
        assert!(response.len() > HEADER_SIZE);
    }

    #[test]
    fn test_responder_handle_query_wrong_name() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));
        resp.state = PublishState::Published;

        let query = build_test_mdns_query("other.local", 1);
        assert!(resp.handle_query(&query).is_none());
    }

    #[test]
    fn test_responder_handle_query_qu_bit() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));
        resp.state = PublishState::Published;

        let mut query = build_test_mdns_query("host.local", 1);
        // Set QU bit
        let last = query.len();
        query[last - 2] |= 0x80;

        let (_, unicast) = resp.handle_query(&query).unwrap();
        assert!(unicast);
    }

    #[test]
    fn test_responder_advance_idle_to_probing() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));

        let (msg, delay) = resp.advance();
        assert!(msg.is_some()); // probe message
        assert_eq!(delay, PROBE_WAIT);
        assert!(matches!(resp.state(), PublishState::Probing { .. }));
    }

    #[test]
    fn test_responder_advance_through_probing() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));

        // First probe
        resp.advance();
        assert!(matches!(
            resp.state(),
            PublishState::Probing { probe_count: 0 }
        ));

        // Force time advancement by setting last_action to the past
        resp.last_action = Some(Instant::now() - PROBE_WAIT * 2);

        // Second probe
        resp.advance();
        assert!(matches!(
            resp.state(),
            PublishState::Probing { probe_count: 1 }
        ));

        resp.last_action = Some(Instant::now() - PROBE_WAIT * 2);

        // Third probe → should transition to Announcing
        resp.advance();
        // After probe_count reaches PROBE_COUNT (3), should transition
        // Since initial count is 0 and we increment: 0→1→2→(>=3 announces)
        // Actually: advance sets probe_count to next_count which is current+1.
        // probe_count=0: advance → next_count=1 (not >= 3)
        // probe_count=1: advance → next_count=2 (not >= 3)
        resp.last_action = Some(Instant::now() - PROBE_WAIT * 2);
        resp.advance();
        // probe_count=2: advance → next_count=3 (>= 3) → Announcing
        assert!(
            matches!(resp.state(), PublishState::Announcing { .. }),
            "Expected Announcing, got {:?}",
            resp.state()
        );
    }

    #[test]
    fn test_responder_signal_conflict() {
        let mut resp = MdnsResponder::new("host");
        resp.state = PublishState::Published;
        resp.signal_conflict();
        assert_eq!(resp.state(), PublishState::Conflict);
    }

    #[test]
    fn test_responder_build_goodbye() {
        let mut resp = MdnsResponder::new("host");
        resp.add_record(MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1)));

        let goodbye = resp.build_goodbye().unwrap();
        assert!(goodbye.len() > HEADER_SIZE);
    }

    #[test]
    fn test_responder_build_goodbye_empty() {
        let resp = MdnsResponder::new("host");
        assert!(resp.build_goodbye().is_none());
    }

    // ── Resolver tests ─────────────────────────────────────────────────

    #[test]
    fn test_resolver_new() {
        let r = MdnsResolver::new();
        assert_eq!(r.timeout, MDNS_QUERY_TIMEOUT);
        assert_eq!(r.max_queries, MDNS_MAX_QUERIES);
    }

    #[test]
    fn test_resolver_default() {
        let r = MdnsResolver::default();
        assert_eq!(r.timeout, MDNS_QUERY_TIMEOUT);
    }

    #[test]
    fn test_resolver_with_timeout() {
        let r = MdnsResolver::new().with_timeout(Duration::from_millis(500));
        assert_eq!(r.timeout, Duration::from_millis(500));
    }

    #[test]
    fn test_resolver_with_max_queries() {
        let r = MdnsResolver::new().with_max_queries(5);
        assert_eq!(r.max_queries, 5);
    }

    #[test]
    fn test_resolver_with_max_queries_min_1() {
        let r = MdnsResolver::new().with_max_queries(0);
        assert_eq!(r.max_queries, 1);
    }

    #[test]
    fn test_resolver_build_query_local() {
        let r = MdnsResolver::new();
        let q = r.build_query("host.local", RecordType::A).unwrap();
        assert!(q.len() > HEADER_SIZE);
    }

    #[test]
    fn test_resolver_build_query_auto_local() {
        let r = MdnsResolver::new();
        let q = r.build_query("host", RecordType::A).unwrap();
        assert!(q.len() > HEADER_SIZE);
    }

    // ── Response parsing tests ─────────────────────────────────────────

    #[test]
    fn test_parse_response_a_record() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let resp = build_mdns_response(&[rec]).unwrap();
        let parsed = parse_mdns_response_records(&resp).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].name, "host.local");
        assert_eq!(parsed[0].rtype, RecordType::A);
        assert_eq!(parsed[0].rdata, vec![10, 0, 0, 1]);
        assert!(parsed[0].cache_flush);
    }

    #[test]
    fn test_parse_response_aaaa_record() {
        let addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        let rec = MdnsRecord::aaaa("host", addr);
        let resp = build_mdns_response(&[rec]).unwrap();
        let parsed = parse_mdns_response_records(&resp).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].rtype, RecordType::AAAA);
        assert_eq!(parsed[0].rdata.len(), 16);
    }

    #[test]
    fn test_parse_response_multiple_records() {
        let a = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let aaaa = MdnsRecord::aaaa("host", Ipv6Addr::LOCALHOST);
        let resp = build_mdns_response(&[a, aaaa]).unwrap();
        let parsed = parse_mdns_response_records(&resp).unwrap();
        assert_eq!(parsed.len(), 2);
    }

    #[test]
    fn test_parse_response_too_short() {
        assert!(parse_mdns_response_records(&[0; 5]).is_none());
    }

    #[test]
    fn test_parse_response_query_rejected() {
        let query = build_test_mdns_query("host.local", 1);
        assert!(parse_mdns_response_records(&query).is_none());
    }

    #[test]
    fn test_parse_response_cache_flush_bit() {
        let rec = MdnsRecord::a("host", Ipv4Addr::new(10, 0, 0, 1));
        let resp = build_mdns_response(&[rec]).unwrap();
        let parsed = parse_mdns_response_records(&resp).unwrap();
        assert!(parsed[0].cache_flush);
        assert_eq!(parsed[0].rclass, CLASS_IN);
    }

    #[test]
    fn test_parse_response_no_cache_flush() {
        let rec = MdnsRecord::ptr("_http._tcp.local", "svc.local", 4500).unwrap();
        let resp = build_mdns_response(&[rec]).unwrap();
        let parsed = parse_mdns_response_records(&resp).unwrap();
        assert!(!parsed[0].cache_flush);
    }

    // ── format_local_name tests ────────────────────────────────────────

    #[test]
    fn test_format_local_name_plain() {
        assert_eq!(format_local_name("myhost"), "myhost.local");
    }

    #[test]
    fn test_format_local_name_already_local() {
        assert_eq!(format_local_name("myhost.local"), "myhost.local");
    }

    #[test]
    fn test_format_local_name_trailing_dot() {
        assert_eq!(format_local_name("myhost."), "myhost.local");
    }

    #[test]
    fn test_format_local_name_uppercase() {
        assert_eq!(format_local_name("MyHost"), "myhost.local");
    }

    // ── Reverse name tests ─────────────────────────────────────────────

    #[test]
    fn test_reverse_ipv4_name() {
        assert_eq!(
            reverse_ipv4_name(Ipv4Addr::new(10, 0, 0, 1)),
            "1.0.0.10.in-addr.arpa"
        );
    }

    #[test]
    fn test_reverse_ipv6_name() {
        let addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        let name = reverse_ipv6_name(addr);
        assert!(name.ends_with(".ip6.arpa"));
        assert!(
            name.starts_with("1.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.0.8.b.d.0.1.0.0.2")
        );
    }

    // ── PublishState display tests ─────────────────────────────────────

    #[test]
    fn test_publish_state_display() {
        assert_eq!(format!("{}", PublishState::Idle), "idle");
        assert_eq!(
            format!("{}", PublishState::Probing { probe_count: 1 }),
            "probing (1/3)"
        );
        assert_eq!(
            format!("{}", PublishState::Announcing { announce_count: 0 }),
            "announcing (0/2)"
        );
        assert_eq!(format!("{}", PublishState::Published), "published");
        assert_eq!(format!("{}", PublishState::Conflict), "conflict");
    }

    // ── Stats tests ────────────────────────────────────────────────────

    #[test]
    fn test_stats_default() {
        let stats = MdnsStats::default();
        assert_eq!(stats.queries_received, 0);
        assert_eq!(stats.queries_answered, 0);
        assert_eq!(stats.conflicts_detected, 0);
    }

    #[test]
    fn test_stats_display() {
        let stats = MdnsStats {
            queries_received: 50,
            queries_answered: 30,
            queries_ignored: 20,
            resolution_queries_sent: 10,
            resolution_responses: 8,
            probes_sent: 3,
            announcements_sent: 2,
            conflicts_detected: 0,
            goodbyes_sent: 1,
        };
        let s = format!("{}", stats);
        assert!(s.contains("50"));
        assert!(s.contains("30"));
        assert!(s.contains("mDNS"));
    }

    // ── Constants tests ────────────────────────────────────────────────

    #[test]
    fn test_constants() {
        assert_eq!(MDNS_PORT, 5353);
        assert_eq!(MDNS_MCAST_V4, Ipv4Addr::new(224, 0, 0, 251));
        assert_eq!(
            MDNS_MCAST_V6,
            Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0, 0x00fb)
        );
        assert_eq!(MDNS_TTL_V4, 255);
        assert_eq!(MDNS_HOP_LIMIT_V6, 255);
        assert_eq!(MDNS_HOST_TTL, 120);
        assert_eq!(MDNS_OTHER_TTL, 4500);
        assert_eq!(MDNS_GOODBYE_TTL, 0);
        assert_eq!(CACHE_FLUSH_BIT, 0x8000);
    }

    #[test]
    fn test_mdns_mcast_addrs() {
        assert_eq!(MDNS_MCAST_ADDR_V4.port(), MDNS_PORT);
        assert_eq!(MDNS_MCAST_ADDR_V6.port(), MDNS_PORT);
    }

    // ── Integration: query → responder → parse ─────────────────────────

    #[test]
    fn test_roundtrip_a_record() {
        let mut responder = MdnsResponder::new("mypc");
        responder.add_record(MdnsRecord::a("mypc", Ipv4Addr::new(192, 168, 1, 42)));
        responder.state = PublishState::Published;

        let query = build_test_mdns_query("mypc.local", 1);
        let (resp_bytes, _) = responder.handle_query(&query).unwrap();
        let parsed = parse_mdns_response_records(&resp_bytes).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].rtype, RecordType::A);
        assert_eq!(parsed[0].rdata, vec![192, 168, 1, 42]);
    }

    #[test]
    fn test_roundtrip_aaaa_record() {
        let addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0x1234, 0, 0, 0x5678);
        let mut responder = MdnsResponder::new("mypc");
        responder.add_record(MdnsRecord::aaaa("mypc", addr));
        responder.state = PublishState::Published;

        let query = build_test_mdns_query("mypc.local", 28);
        let (resp_bytes, _) = responder.handle_query(&query).unwrap();
        let parsed = parse_mdns_response_records(&resp_bytes).unwrap();

        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].rtype, RecordType::AAAA);
        let mut octets = [0u8; 16];
        octets.copy_from_slice(&parsed[0].rdata);
        assert_eq!(Ipv6Addr::from(octets), addr);
    }

    #[test]
    fn test_roundtrip_any_query() {
        let mut responder = MdnsResponder::new("mypc");
        responder.add_record(MdnsRecord::a("mypc", Ipv4Addr::new(10, 0, 0, 1)));
        responder.add_record(MdnsRecord::aaaa("mypc", Ipv6Addr::LOCALHOST));
        responder.state = PublishState::Published;

        let query = build_test_mdns_query("mypc.local", 255); // ANY
        let (resp_bytes, _) = responder.handle_query(&query).unwrap();
        let parsed = parse_mdns_response_records(&resp_bytes).unwrap();
        assert!(parsed.len() >= 2);
    }
}
