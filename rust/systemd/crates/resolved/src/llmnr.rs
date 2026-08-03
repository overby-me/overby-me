#![allow(dead_code)]
//! Link-Local Multicast Name Resolution (LLMNR) — RFC 4795.
//!
//! LLMNR allows hosts on the same local link to resolve each other's names
//! without a configured DNS server.  Queries are sent to a well-known
//! multicast address and the host owning the name responds directly
//! (unicast) to the querier.
//!
//! ## Addresses
//!
//! - IPv4: `224.0.0.252` port 5355
//! - IPv6: `ff02::1:3` port 5355
//!
//! ## Protocol overview
//!
//! 1. A client sends a DNS-formatted query to the LLMNR multicast address.
//! 2. Any host whose name matches responds via unicast to the querier.
//! 3. Queries MUST have QDCOUNT=1, OPCODE=0, TC=0, ANCOUNT=0, NSCOUNT=0.
//! 4. The responder sets the C (conflict) bit in the response when it
//!    detects a conflict (multiple owners of the same name).
//!
//! ## Scope
//!
//! LLMNR is strictly link-local:
//! - IPv4 queries use TTL=1 (not forwarded by routers).
//! - IPv6 queries use hop-limit=1 and link-local scope.
//! - Responses MUST NOT be cached across link boundaries.
//!
//! ## This module
//!
//! Provides:
//! - LLMNR constants and message validation
//! - `LlmnrResponder` — answers queries for the local hostname
//! - `LlmnrResolver` — sends queries and collects unicast responses
//! - Socket construction helpers (IPv4 + IPv6 multicast join)

use std::fmt;
use std::io;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr, UdpSocket};
use std::time::{Duration, Instant};

use crate::dns::{DnsHeader, HEADER_SIZE, RecordType, encode_name, parse_name};

// ── Constants ──────────────────────────────────────────────────────────────

/// LLMNR port (RFC 4795 §2)
pub const LLMNR_PORT: u16 = 5355;

/// LLMNR IPv4 multicast address
pub const LLMNR_MCAST_V4: Ipv4Addr = Ipv4Addr::new(224, 0, 0, 252);

/// LLMNR IPv6 multicast address
pub const LLMNR_MCAST_V6: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0x0001, 0x0003);

/// LLMNR IPv4 multicast socket address
pub const LLMNR_MCAST_ADDR_V4: SocketAddr = SocketAddr::new(IpAddr::V4(LLMNR_MCAST_V4), LLMNR_PORT);

/// LLMNR IPv6 multicast socket address
pub const LLMNR_MCAST_ADDR_V6: SocketAddr = SocketAddr::new(IpAddr::V6(LLMNR_MCAST_V6), LLMNR_PORT);

/// Maximum LLMNR UDP message size (RFC 4795 §2.1 — same as DNS)
pub const MAX_LLMNR_UDP_SIZE: usize = 512;

/// Default LLMNR query timeout (RFC 4795 §2.1 recommends LLMNR_TIMEOUT = 1s)
pub const LLMNR_TIMEOUT: Duration = Duration::from_millis(1000);

/// LLMNR jitter limit for responses (RFC 4795 §2.7 — 0–100ms)
pub const LLMNR_JITTER_MS: u64 = 100;

/// Maximum number of LLMNR query retransmissions (RFC 4795 §2.1)
pub const LLMNR_MAX_RETRIES: usize = 1;

/// TTL for LLMNR IPv4 multicast (MUST be 1)
pub const LLMNR_TTL_V4: u32 = 1;

/// Hop limit for LLMNR IPv6 multicast (MUST be 1)
pub const LLMNR_HOP_LIMIT_V6: u32 = 1;

/// Default TTL in LLMNR responses (30 seconds, matching Windows default)
pub const LLMNR_RESPONSE_TTL: u32 = 30;

/// DNS class IN
const CLASS_IN: u16 = 1;

/// Conflict (C) bit position in the flags field (bit 10 of the second flags
/// byte, i.e. byte 3 bit 2 counting from LSB).  In LLMNR the TC bit position
/// in standard DNS is re-used as the conflict flag for *responses*.
/// Actually per RFC 4795 §2.1.1 the C bit is at the same position as the TC
/// bit (byte 2, bit 1).  In queries C=0; in responses C=1 means conflict.
const CONFLICT_FLAG_BYTE: usize = 2;
const CONFLICT_FLAG_MASK: u8 = 0x02; // same position as TC in DNS

// ── Validation ─────────────────────────────────────────────────────────────

/// Errors specific to LLMNR message validation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LlmnrError {
    /// Message too short for a DNS/LLMNR header.
    TooShort,
    /// Not a query (QR=1).
    NotAQuery,
    /// Non-zero opcode (must be 0 for LLMNR).
    BadOpcode,
    /// QDCOUNT != 1 (LLMNR mandates exactly one question).
    BadQuestionCount,
    /// Non-zero ANCOUNT/NSCOUNT in query.
    NonZeroCounts,
    /// Query name could not be parsed.
    BadName,
    /// Multi-label name (LLMNR only resolves single-label names).
    MultiLabelName,
    /// TC bit set in query (not allowed per RFC 4795 §2.1).
    TruncatedQuery,
}

impl fmt::Display for LlmnrError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::TooShort => write!(f, "LLMNR message too short"),
            Self::NotAQuery => write!(f, "LLMNR message is not a query"),
            Self::BadOpcode => write!(f, "LLMNR query has non-zero opcode"),
            Self::BadQuestionCount => write!(f, "LLMNR query QDCOUNT != 1"),
            Self::NonZeroCounts => write!(f, "LLMNR query has non-zero AN/NS count"),
            Self::BadName => write!(f, "LLMNR query name could not be parsed"),
            Self::MultiLabelName => write!(f, "LLMNR query name has multiple labels"),
            Self::TruncatedQuery => write!(f, "LLMNR query has TC bit set"),
        }
    }
}

/// A validated LLMNR query.
#[derive(Debug, Clone)]
pub struct LlmnrQuery {
    /// Transaction ID.
    pub id: u16,
    /// The queried single-label name (lowercased).
    pub name: String,
    /// Query type (A, AAAA, ANY, …).
    pub qtype: RecordType,
    /// Query class (normally IN).
    pub qclass: u16,
    /// The raw query bytes.
    pub raw: Vec<u8>,
}

/// Validate and parse an incoming LLMNR query.
///
/// Returns `Err` if the message does not conform to RFC 4795 §2.1
/// requirements for a query.
pub fn parse_llmnr_query(data: &[u8]) -> Result<LlmnrQuery, LlmnrError> {
    if data.len() < HEADER_SIZE {
        return Err(LlmnrError::TooShort);
    }

    let header = DnsHeader::parse(data).map_err(|_| LlmnrError::TooShort)?;

    // Must be a query (QR=0)
    if header.qr {
        return Err(LlmnrError::NotAQuery);
    }

    // Opcode must be 0 (standard query)
    if u8::from(header.opcode) != 0 {
        return Err(LlmnrError::BadOpcode);
    }

    // TC must be 0 in queries
    if header.tc {
        return Err(LlmnrError::TruncatedQuery);
    }

    // Exactly one question
    if header.qdcount != 1 {
        return Err(LlmnrError::BadQuestionCount);
    }

    // ANCOUNT and NSCOUNT must be 0
    if header.ancount != 0 || header.nscount != 0 {
        return Err(LlmnrError::NonZeroCounts);
    }

    // Parse the question name
    let (name, name_end) = parse_name(data, HEADER_SIZE).map_err(|_| LlmnrError::BadName)?;

    // LLMNR only resolves single-label names (no dots, excluding trailing dot)
    let trimmed = name.trim_end_matches('.');
    if trimmed.contains('.') {
        return Err(LlmnrError::MultiLabelName);
    }

    // Parse QTYPE and QCLASS
    if name_end + 4 > data.len() {
        return Err(LlmnrError::TooShort);
    }
    let qtype_raw = u16::from_be_bytes([data[name_end], data[name_end + 1]]);
    let qclass = u16::from_be_bytes([data[name_end + 2], data[name_end + 3]]);

    Ok(LlmnrQuery {
        id: header.id,
        name: trimmed.to_lowercase(),
        qtype: RecordType::from_u16(qtype_raw),
        qclass,
        raw: data.to_vec(),
    })
}

// ── Response construction ──────────────────────────────────────────────────

/// Build an LLMNR response for a single A record.
pub fn build_llmnr_a_response(query: &[u8], addr: Ipv4Addr) -> Option<Vec<u8>> {
    build_llmnr_response(query, RecordType::A, &addr.octets(), LLMNR_RESPONSE_TTL)
}

/// Build an LLMNR response for a single AAAA record.
pub fn build_llmnr_aaaa_response(query: &[u8], addr: Ipv6Addr) -> Option<Vec<u8>> {
    build_llmnr_response(query, RecordType::AAAA, &addr.octets(), LLMNR_RESPONSE_TTL)
}

/// Build a generic LLMNR response carrying one answer RR.
///
/// Copies the question section from the query and appends an answer RR
/// with the given record type and RDATA.
fn build_llmnr_response(
    query: &[u8],
    rtype: RecordType,
    rdata: &[u8],
    ttl: u32,
) -> Option<Vec<u8>> {
    if query.len() < HEADER_SIZE {
        return None;
    }

    let header = DnsHeader::parse(query).ok()?;
    if header.qdcount != 1 {
        return None;
    }

    // Locate end of question section
    let (_, name_end) = parse_name(query, HEADER_SIZE).ok()?;
    if name_end + 4 > query.len() {
        return None;
    }
    let question_end = name_end + 4; // past QTYPE + QCLASS

    // Build response header
    let mut resp = Vec::with_capacity(question_end + 64);

    // Copy header bytes and set flags
    resp.extend_from_slice(&query[..HEADER_SIZE]);
    // QR=1, keep opcode/RD from query
    resp[2] = (query[2] & 0x79) | 0x80; // QR=1
    resp[3] = 0x00; // RA=0, RCODE=0 (NOERROR)
    // QDCOUNT = 1
    resp[4] = 0;
    resp[5] = 1;
    // ANCOUNT = 1
    resp[6] = 0;
    resp[7] = 1;
    // NSCOUNT = 0
    resp[8] = 0;
    resp[9] = 0;
    // ARCOUNT = 0
    resp[10] = 0;
    resp[11] = 0;

    // Copy question section
    resp.extend_from_slice(&query[HEADER_SIZE..question_end]);

    // Build answer RR: use a compression pointer to the name in the question
    // section (offset HEADER_SIZE = 12 → 0xC00C).
    resp.push(0xC0);
    resp.push(0x0C);
    // TYPE
    resp.extend_from_slice(&rtype.to_u16().to_be_bytes());
    // CLASS IN
    resp.extend_from_slice(&CLASS_IN.to_be_bytes());
    // TTL
    resp.extend_from_slice(&ttl.to_be_bytes());
    // RDLENGTH
    resp.extend_from_slice(&(rdata.len() as u16).to_be_bytes());
    // RDATA
    resp.extend_from_slice(rdata);

    Some(resp)
}

/// Build an LLMNR negative response (RCODE=NOERROR, ANCOUNT=0).
/// Used when we are authoritative for the name but have no matching records
/// for the requested type (e.g. asked AAAA but we only have A).
fn build_llmnr_empty_response(query: &[u8]) -> Option<Vec<u8>> {
    if query.len() < HEADER_SIZE {
        return None;
    }

    let header = DnsHeader::parse(query).ok()?;
    if header.qdcount != 1 {
        return None;
    }

    let (_, name_end) = parse_name(query, HEADER_SIZE).ok()?;
    if name_end + 4 > query.len() {
        return None;
    }
    let question_end = name_end + 4;

    let mut resp = Vec::with_capacity(question_end);
    resp.extend_from_slice(&query[..HEADER_SIZE]);
    resp[2] = (query[2] & 0x79) | 0x80;
    resp[3] = 0x00;
    // QDCOUNT = 1
    resp[4] = 0;
    resp[5] = 1;
    // ANCOUNT = 0
    resp[6] = 0;
    resp[7] = 0;
    resp[8] = 0;
    resp[9] = 0;
    resp[10] = 0;
    resp[11] = 0;

    resp.extend_from_slice(&query[HEADER_SIZE..question_end]);
    Some(resp)
}

// ── Responder ──────────────────────────────────────────────────────────────

/// Configuration for the LLMNR responder.
#[derive(Debug, Clone)]
pub struct LlmnrResponderConfig {
    /// The hostname this node responds to (single-label, lowercased).
    pub hostname: String,
    /// IPv4 addresses to answer with for A queries.
    pub ipv4_addrs: Vec<Ipv4Addr>,
    /// IPv6 addresses to answer with for AAAA queries.
    pub ipv6_addrs: Vec<Ipv6Addr>,
    /// TTL for our responses.
    pub ttl: u32,
}

impl LlmnrResponderConfig {
    /// Create a responder config from the system hostname and addresses.
    pub fn new(hostname: &str, ipv4: Vec<Ipv4Addr>, ipv6: Vec<Ipv6Addr>) -> Self {
        // Extract first label only
        let label = hostname
            .split('.')
            .next()
            .unwrap_or(hostname)
            .to_lowercase();
        Self {
            hostname: label,
            ipv4_addrs: ipv4,
            ipv6_addrs: ipv6,
            ttl: LLMNR_RESPONSE_TTL,
        }
    }
}

/// LLMNR responder that listens for multicast queries on the local link
/// and answers with the local hostname's addresses.
pub struct LlmnrResponder {
    config: LlmnrResponderConfig,
}

impl LlmnrResponder {
    /// Create a new LLMNR responder.
    pub fn new(config: LlmnrResponderConfig) -> Self {
        Self { config }
    }

    /// Update the responder's hostname.
    pub fn set_hostname(&mut self, hostname: &str) {
        self.config.hostname = hostname
            .split('.')
            .next()
            .unwrap_or(hostname)
            .to_lowercase();
    }

    /// Update the responder's addresses.
    pub fn set_addresses(&mut self, ipv4: Vec<Ipv4Addr>, ipv6: Vec<Ipv6Addr>) {
        self.config.ipv4_addrs = ipv4;
        self.config.ipv6_addrs = ipv6;
    }

    /// Handle an incoming LLMNR query.
    ///
    /// Returns `Some(response_bytes)` if the query is for our hostname and
    /// we have matching records; `None` if the query is not for us or is
    /// invalid.
    pub fn handle_query(&self, data: &[u8]) -> Option<Vec<u8>> {
        let query = parse_llmnr_query(data).ok()?;

        // Only respond if the name matches our hostname (case-insensitive)
        if query.name != self.config.hostname {
            return None;
        }

        // Only respond to class IN (or ANY)
        if query.qclass != CLASS_IN && query.qclass != 255 {
            return None;
        }

        match query.qtype {
            RecordType::A => {
                let addr = self.config.ipv4_addrs.first()?;
                build_llmnr_response(data, RecordType::A, &addr.octets(), self.config.ttl)
            }
            RecordType::AAAA => {
                let addr = self.config.ipv6_addrs.first()?;
                build_llmnr_response(data, RecordType::AAAA, &addr.octets(), self.config.ttl)
            }
            RecordType::ANY => {
                // Respond with the first available address (prefer A)
                if let Some(addr) = self.config.ipv4_addrs.first() {
                    build_llmnr_response(data, RecordType::A, &addr.octets(), self.config.ttl)
                } else if let Some(addr) = self.config.ipv6_addrs.first() {
                    build_llmnr_response(data, RecordType::AAAA, &addr.octets(), self.config.ttl)
                } else {
                    build_llmnr_empty_response(data)
                }
            }
            _ => {
                // Respond with empty answer to indicate we own the name
                // but don't have the requested record type.
                build_llmnr_empty_response(data)
            }
        }
    }

    /// Get the hostname this responder answers for.
    pub fn hostname(&self) -> &str {
        &self.config.hostname
    }

    /// Get the current config.
    pub fn config(&self) -> &LlmnrResponderConfig {
        &self.config
    }
}

// ── Resolver ───────────────────────────────────────────────────────────────

/// Result of an LLMNR resolution attempt.
#[derive(Debug, Clone)]
pub struct LlmnrResult {
    /// The name that was queried.
    pub name: String,
    /// Resolved IPv4 addresses.
    pub ipv4: Vec<Ipv4Addr>,
    /// Resolved IPv6 addresses.
    pub ipv6: Vec<Ipv6Addr>,
    /// Whether a conflict was detected (C bit in response).
    pub conflict: bool,
    /// The source address of the responder.
    pub responder: Option<SocketAddr>,
}

/// LLMNR resolver that sends multicast queries and collects responses.
pub struct LlmnrResolver {
    /// Query timeout.
    timeout: Duration,
    /// Maximum retries.
    max_retries: usize,
    /// Next transaction ID.
    next_id: u16,
}

impl LlmnrResolver {
    /// Create a new LLMNR resolver with default settings.
    pub fn new() -> Self {
        Self {
            timeout: LLMNR_TIMEOUT,
            max_retries: LLMNR_MAX_RETRIES,
            next_id: 1,
        }
    }

    /// Create a resolver with custom timeout.
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// Build an LLMNR query for a single-label name.
    pub fn build_query(&mut self, name: &str, qtype: RecordType) -> Option<Vec<u8>> {
        let trimmed = name.trim_end_matches('.');
        if trimmed.is_empty() || trimmed.contains('.') {
            return None;
        }

        let encoded_name = encode_name(trimmed).ok()?;

        let id = self.next_id;
        self.next_id = self.next_id.wrapping_add(1);

        let mut buf = Vec::with_capacity(HEADER_SIZE + encoded_name.len() + 4);

        // Header
        buf.extend_from_slice(&id.to_be_bytes()); // ID
        buf.extend_from_slice(&[0x00, 0x00]); // Flags: query, opcode=0, no flags
        buf.extend_from_slice(&1u16.to_be_bytes()); // QDCOUNT = 1
        buf.extend_from_slice(&0u16.to_be_bytes()); // ANCOUNT = 0
        buf.extend_from_slice(&0u16.to_be_bytes()); // NSCOUNT = 0
        buf.extend_from_slice(&0u16.to_be_bytes()); // ARCOUNT = 0

        // Question
        buf.extend_from_slice(&encoded_name);
        buf.extend_from_slice(&qtype.to_u16().to_be_bytes()); // QTYPE
        buf.extend_from_slice(&CLASS_IN.to_be_bytes()); // QCLASS = IN

        Some(buf)
    }

    /// Resolve a single-label name via LLMNR using a provided socket.
    ///
    /// Sends the query to the LLMNR multicast address and waits for a
    /// unicast response.  Returns `Ok(result)` with any addresses found,
    /// or `Err` on I/O failure.
    pub fn resolve(
        &mut self,
        socket: &UdpSocket,
        name: &str,
        dest: SocketAddr,
    ) -> io::Result<LlmnrResult> {
        // Build both A and AAAA queries
        let query_a = self.build_query(name, RecordType::A);
        let query_aaaa = self.build_query(name, RecordType::AAAA);

        let mut result = LlmnrResult {
            name: name.to_lowercase(),
            ipv4: Vec::new(),
            ipv6: Vec::new(),
            conflict: false,
            responder: None,
        };

        socket.set_read_timeout(Some(self.timeout))?;

        for attempt in 0..=self.max_retries {
            // Send A query
            if let Some(ref q) = query_a {
                socket.send_to(q, dest)?;
            }
            // Send AAAA query
            if let Some(ref q) = query_aaaa {
                socket.send_to(q, dest)?;
            }

            let deadline = Instant::now() + self.timeout;

            // Collect responses until timeout
            let mut buf = [0u8; MAX_LLMNR_UDP_SIZE];
            while Instant::now() < deadline {
                let remaining = deadline.saturating_duration_since(Instant::now());
                if remaining.is_zero() {
                    break;
                }
                let _ = socket.set_read_timeout(Some(remaining));

                match socket.recv_from(&mut buf) {
                    Ok((len, src)) => {
                        if let Some(parsed) = parse_llmnr_response(&buf[..len]) {
                            result.responder = Some(src);
                            if parsed.conflict {
                                result.conflict = true;
                            }
                            result.ipv4.extend(parsed.ipv4);
                            result.ipv6.extend(parsed.ipv6);
                        }
                    }
                    Err(ref e)
                        if e.kind() == io::ErrorKind::WouldBlock
                            || e.kind() == io::ErrorKind::TimedOut =>
                    {
                        break;
                    }
                    Err(e) => {
                        if attempt == self.max_retries {
                            return Err(e);
                        }
                        break;
                    }
                }
            }

            if !result.ipv4.is_empty() || !result.ipv6.is_empty() {
                break;
            }
        }

        // Deduplicate
        result.ipv4.sort();
        result.ipv4.dedup();
        result.ipv6.sort();
        result.ipv6.dedup();

        Ok(result)
    }
}

impl Default for LlmnrResolver {
    fn default() -> Self {
        Self::new()
    }
}

// ── Response parsing ───────────────────────────────────────────────────────

/// Parsed content from an LLMNR response.
#[derive(Debug, Clone)]
struct ParsedLlmnrResponse {
    ipv4: Vec<Ipv4Addr>,
    ipv6: Vec<Ipv6Addr>,
    conflict: bool,
}

/// Parse an LLMNR response and extract addresses.
fn parse_llmnr_response(data: &[u8]) -> Option<ParsedLlmnrResponse> {
    if data.len() < HEADER_SIZE {
        return None;
    }

    let header = DnsHeader::parse(data).ok()?;

    // Must be a response
    if !header.qr {
        return None;
    }

    // Check RCODE — only NOERROR responses contain useful data
    if !matches!(header.rcode, crate::dns::Rcode::NoError) {
        return None;
    }

    // Detect conflict bit (same position as TC)
    let conflict = (data[CONFLICT_FLAG_BYTE] & CONFLICT_FLAG_MASK) != 0;

    let mut offset = HEADER_SIZE;

    // Skip question section
    for _ in 0..header.qdcount {
        let (_, name_end) = parse_name(data, offset).ok()?;
        offset = name_end + 4; // QTYPE + QCLASS
        if offset > data.len() {
            return None;
        }
    }

    // Parse answer section
    let mut ipv4 = Vec::new();
    let mut ipv6 = Vec::new();

    for _ in 0..header.ancount {
        if offset >= data.len() {
            break;
        }

        let (_, name_end) = parse_name(data, offset).ok()?;
        if name_end + 10 > data.len() {
            break;
        }

        let rtype = u16::from_be_bytes([data[name_end], data[name_end + 1]]);
        let _rclass = u16::from_be_bytes([data[name_end + 2], data[name_end + 3]]);
        let _ttl = u32::from_be_bytes([
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

        match rtype {
            1 if rdlen == 4 => {
                // A record
                let addr = Ipv4Addr::new(
                    data[rdata_start],
                    data[rdata_start + 1],
                    data[rdata_start + 2],
                    data[rdata_start + 3],
                );
                ipv4.push(addr);
            }
            28 if rdlen == 16 => {
                // AAAA record
                let mut octets = [0u8; 16];
                octets.copy_from_slice(&data[rdata_start..rdata_end]);
                ipv6.push(Ipv6Addr::from(octets));
            }
            _ => {
                // Skip unknown record types
            }
        }

        offset = rdata_end;
    }

    Some(ParsedLlmnrResponse {
        ipv4,
        ipv6,
        conflict,
    })
}

// ── Socket helpers ─────────────────────────────────────────────────────────

/// Bind a UDP socket for LLMNR on the IPv4 multicast address.
///
/// Joins the LLMNR multicast group on all interfaces (INADDR_ANY).
/// The socket has SO_REUSEADDR set and is bound to 0.0.0.0:5355.
pub fn bind_llmnr_ipv4() -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddr::new(
        IpAddr::V4(Ipv4Addr::UNSPECIFIED),
        LLMNR_PORT,
    ))?;
    socket.set_multicast_loop_v4(false)?;
    socket.set_multicast_ttl_v4(LLMNR_TTL_V4)?;
    socket.join_multicast_v4(&LLMNR_MCAST_V4, &Ipv4Addr::UNSPECIFIED)?;
    socket.set_nonblocking(false)?;
    Ok(socket)
}

/// Bind a UDP socket for LLMNR on the IPv6 multicast address.
///
/// Joins the LLMNR multicast group on interface index 0 (all interfaces).
pub fn bind_llmnr_ipv6() -> io::Result<UdpSocket> {
    let socket = UdpSocket::bind(SocketAddr::new(
        IpAddr::V6(Ipv6Addr::UNSPECIFIED),
        LLMNR_PORT,
    ))?;
    socket.set_multicast_loop_v6(false)?;
    // Join on all interfaces (ifindex 0)
    socket.join_multicast_v6(&LLMNR_MCAST_V6, 0)?;
    socket.set_nonblocking(false)?;
    Ok(socket)
}

// ── LLMNR scope check ──────────────────────────────────────────────────────

/// Check whether an address is link-local (and thus valid for LLMNR).
pub fn is_link_local(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => {
            // 169.254.0.0/16
            v4.octets()[0] == 169 && v4.octets()[1] == 254
        }
        IpAddr::V6(v6) => {
            // fe80::/10
            let seg = v6.segments();
            (seg[0] & 0xffc0) == 0xfe80
        }
    }
}

/// Check whether an address is a valid LLMNR responder source.
///
/// LLMNR responses should come from link-local or same-subnet addresses.
/// In practice we accept any non-loopback, non-multicast address.
pub fn is_valid_llmnr_source(addr: &IpAddr) -> bool {
    match addr {
        IpAddr::V4(v4) => !v4.is_loopback() && !v4.is_multicast() && !v4.is_unspecified(),
        IpAddr::V6(v6) => !v6.is_loopback() && !v6.is_multicast() && v6.segments()[0] != 0,
    }
}

// ── Statistics ──────────────────────────────────────────────────────────────

/// LLMNR daemon statistics.
#[derive(Debug, Clone, Default)]
pub struct LlmnrStats {
    /// Number of LLMNR queries received.
    pub queries_received: u64,
    /// Number of queries we responded to (matched our hostname).
    pub queries_answered: u64,
    /// Number of queries ignored (not our name / invalid).
    pub queries_ignored: u64,
    /// Number of outgoing LLMNR resolution queries sent.
    pub resolution_queries_sent: u64,
    /// Number of resolution responses received.
    pub resolution_responses: u64,
    /// Number of conflicts detected.
    pub conflicts_detected: u64,
}

impl fmt::Display for LlmnrStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "LLMNR: {} received, {} answered, {} ignored, {} resolutions sent, {} responses, {} conflicts",
            self.queries_received,
            self.queries_answered,
            self.queries_ignored,
            self.resolution_queries_sent,
            self.resolution_responses,
            self.conflicts_detected,
        )
    }
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    /// Build a simple LLMNR query for a single-label name.
    fn build_test_llmnr_query(name: &str, qtype: u16) -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);

        // Header: ID=0x1234, flags=0 (query), QDCOUNT=1
        buf.extend_from_slice(&[0x12, 0x34]); // ID
        buf.extend_from_slice(&[0x00, 0x00]); // Flags
        buf.extend_from_slice(&[0x00, 0x01]); // QDCOUNT
        buf.extend_from_slice(&[0x00, 0x00]); // ANCOUNT
        buf.extend_from_slice(&[0x00, 0x00]); // NSCOUNT
        buf.extend_from_slice(&[0x00, 0x00]); // ARCOUNT

        // Question: encode single label
        buf.push(name.len() as u8);
        buf.extend_from_slice(name.as_bytes());
        buf.push(0x00); // root

        buf.extend_from_slice(&qtype.to_be_bytes()); // QTYPE
        buf.extend_from_slice(&1u16.to_be_bytes()); // QCLASS = IN

        buf
    }

    /// Build a multi-label LLMNR query (should be rejected).
    fn build_multi_label_query() -> Vec<u8> {
        let mut buf = Vec::with_capacity(64);
        buf.extend_from_slice(&[0x12, 0x34]);
        buf.extend_from_slice(&[0x00, 0x00]);
        buf.extend_from_slice(&[0x00, 0x01]);
        buf.extend_from_slice(&[0x00, 0x00]);
        buf.extend_from_slice(&[0x00, 0x00]);
        buf.extend_from_slice(&[0x00, 0x00]);

        // "host.local" — two labels
        buf.push(4);
        buf.extend_from_slice(b"host");
        buf.push(5);
        buf.extend_from_slice(b"local");
        buf.push(0);

        buf.extend_from_slice(&1u16.to_be_bytes());
        buf.extend_from_slice(&1u16.to_be_bytes());

        buf
    }

    // ── parse_llmnr_query tests ────────────────────────────────────────

    #[test]
    fn fuzz_llmnr_parsers_never_panic() {
        // Robustness net (task #22): the LLMNR parsers consume attacker-
        // controlled multicast packets. parse_llmnr_query and
        // parse_llmnr_response must never panic or hang on malformed input.
        let handle = std::thread::spawn(|| {
            let seed = build_test_llmnr_query("myhost", 1);
            let mut state: u64 = 0x11_11_a0a0_5eed_1234;
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
                    let _ = parse_llmnr_query(&input);
                    let _ = parse_llmnr_response(&input);
                });
                assert!(res.is_ok(), "llmnr parser panicked on: {buf:?}");
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "llmnr fuzz did not finish in 30s -- a malformed packet hangs the parser"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        handle.join().expect("fuzz worker panicked");
    }

    #[test]
    fn test_parse_valid_query() {
        let data = build_test_llmnr_query("myhost", 1);
        let query = parse_llmnr_query(&data).unwrap();
        assert_eq!(query.name, "myhost");
        assert_eq!(query.id, 0x1234);
        assert_eq!(query.qtype, RecordType::A);
        assert_eq!(query.qclass, 1);
    }

    #[test]
    fn test_parse_query_aaaa() {
        let data = build_test_llmnr_query("printer", 28);
        let query = parse_llmnr_query(&data).unwrap();
        assert_eq!(query.name, "printer");
        assert_eq!(query.qtype, RecordType::AAAA);
    }

    #[test]
    fn test_parse_query_any() {
        let data = build_test_llmnr_query("server", 255);
        let query = parse_llmnr_query(&data).unwrap();
        assert_eq!(query.qtype, RecordType::ANY);
    }

    #[test]
    fn test_parse_query_case_insensitive() {
        let data = build_test_llmnr_query("MyHost", 1);
        let query = parse_llmnr_query(&data).unwrap();
        assert_eq!(query.name, "myhost");
    }

    #[test]
    fn test_parse_query_too_short() {
        assert_eq!(
            parse_llmnr_query(&[0; 5]).unwrap_err(),
            LlmnrError::TooShort
        );
    }

    #[test]
    fn test_parse_query_not_a_query() {
        let mut data = build_test_llmnr_query("host", 1);
        data[2] |= 0x80; // Set QR=1 (response)
        assert_eq!(parse_llmnr_query(&data).unwrap_err(), LlmnrError::NotAQuery);
    }

    #[test]
    fn test_parse_query_bad_opcode() {
        let mut data = build_test_llmnr_query("host", 1);
        data[2] |= 0x08; // Set opcode to 1
        assert_eq!(parse_llmnr_query(&data).unwrap_err(), LlmnrError::BadOpcode);
    }

    #[test]
    fn test_parse_query_truncated() {
        let mut data = build_test_llmnr_query("host", 1);
        data[2] |= 0x02; // Set TC=1
        assert_eq!(
            parse_llmnr_query(&data).unwrap_err(),
            LlmnrError::TruncatedQuery
        );
    }

    #[test]
    fn test_parse_query_wrong_qdcount() {
        let mut data = build_test_llmnr_query("host", 1);
        data[5] = 2; // QDCOUNT = 2
        assert_eq!(
            parse_llmnr_query(&data).unwrap_err(),
            LlmnrError::BadQuestionCount
        );
    }

    #[test]
    fn test_parse_query_zero_qdcount() {
        let mut data = build_test_llmnr_query("host", 1);
        data[5] = 0;
        assert_eq!(
            parse_llmnr_query(&data).unwrap_err(),
            LlmnrError::BadQuestionCount
        );
    }

    #[test]
    fn test_parse_query_nonzero_ancount() {
        let mut data = build_test_llmnr_query("host", 1);
        data[7] = 1; // ANCOUNT = 1
        assert_eq!(
            parse_llmnr_query(&data).unwrap_err(),
            LlmnrError::NonZeroCounts
        );
    }

    #[test]
    fn test_parse_query_nonzero_nscount() {
        let mut data = build_test_llmnr_query("host", 1);
        data[9] = 1; // NSCOUNT = 1
        assert_eq!(
            parse_llmnr_query(&data).unwrap_err(),
            LlmnrError::NonZeroCounts
        );
    }

    #[test]
    fn test_parse_query_multi_label_rejected() {
        let data = build_multi_label_query();
        assert_eq!(
            parse_llmnr_query(&data).unwrap_err(),
            LlmnrError::MultiLabelName
        );
    }

    #[test]
    fn test_parse_query_truncated_question() {
        // Header only, no question data
        let data = [0x12, 0x34, 0x00, 0x00, 0x00, 0x01, 0, 0, 0, 0, 0, 0];
        assert!(parse_llmnr_query(&data).is_err());
    }

    // ── Response building tests ────────────────────────────────────────

    #[test]
    fn test_build_a_response() {
        let query = build_test_llmnr_query("myhost", 1);
        let resp = build_llmnr_a_response(&query, Ipv4Addr::new(192, 168, 1, 10)).unwrap();

        // Verify it's a response
        assert_ne!(resp[2] & 0x80, 0, "QR should be 1");
        // ANCOUNT = 1
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 1);
        // Check the response contains RDATA with the IP
        assert!(resp.len() > 20);
    }

    #[test]
    fn test_build_aaaa_response() {
        let query = build_test_llmnr_query("myhost", 28);
        let addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        let resp = build_llmnr_aaaa_response(&query, addr).unwrap();

        assert_ne!(resp[2] & 0x80, 0);
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 1);
    }

    #[test]
    fn test_build_response_preserves_id() {
        let query = build_test_llmnr_query("test", 1);
        let resp = build_llmnr_a_response(&query, Ipv4Addr::new(10, 0, 0, 1)).unwrap();
        assert_eq!(resp[0], 0x12);
        assert_eq!(resp[1], 0x34);
    }

    #[test]
    fn test_build_response_too_short() {
        assert!(build_llmnr_a_response(&[0; 5], Ipv4Addr::LOCALHOST).is_none());
    }

    #[test]
    fn test_build_empty_response() {
        let query = build_test_llmnr_query("myhost", 1);
        let resp = build_llmnr_empty_response(&query).unwrap();
        assert_ne!(resp[2] & 0x80, 0);
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 0); // ANCOUNT=0
    }

    #[test]
    fn test_build_response_rcode_noerror() {
        let query = build_test_llmnr_query("host", 1);
        let resp = build_llmnr_a_response(&query, Ipv4Addr::new(10, 0, 0, 1)).unwrap();
        let rcode = resp[3] & 0x0F;
        assert_eq!(rcode, 0); // NOERROR
    }

    // ── Responder tests ────────────────────────────────────────────────

    #[test]
    fn test_responder_matches_hostname() {
        let config =
            LlmnrResponderConfig::new("myhost", vec![Ipv4Addr::new(192, 168, 1, 10)], vec![]);
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("myhost", 1);
        assert!(responder.handle_query(&query).is_some());
    }

    #[test]
    fn test_responder_case_insensitive() {
        let config = LlmnrResponderConfig::new("MyHost", vec![Ipv4Addr::new(10, 0, 0, 1)], vec![]);
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("MYHOST", 1);
        assert!(responder.handle_query(&query).is_some());
    }

    #[test]
    fn test_responder_ignores_other_name() {
        let config = LlmnrResponderConfig::new("myhost", vec![Ipv4Addr::new(10, 0, 0, 1)], vec![]);
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("otherhost", 1);
        assert!(responder.handle_query(&query).is_none());
    }

    #[test]
    fn test_responder_a_query() {
        let config = LlmnrResponderConfig::new(
            "host",
            vec![Ipv4Addr::new(192, 168, 1, 1)],
            vec![Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)],
        );
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("host", 1);
        let resp = responder.handle_query(&query).unwrap();
        // Should be an A response
        assert!(resp.len() > HEADER_SIZE);
    }

    #[test]
    fn test_responder_aaaa_query() {
        let config = LlmnrResponderConfig::new(
            "host",
            vec![],
            vec![Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1)],
        );
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("host", 28);
        let resp = responder.handle_query(&query).unwrap();
        assert!(resp.len() > HEADER_SIZE);
    }

    #[test]
    fn test_responder_a_query_no_ipv4() {
        let config = LlmnrResponderConfig::new("host", vec![], vec![Ipv6Addr::LOCALHOST]);
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("host", 1);
        // No IPv4 addresses → None for A query
        assert!(responder.handle_query(&query).is_none());
    }

    #[test]
    fn test_responder_any_query_prefers_a() {
        let config = LlmnrResponderConfig::new(
            "host",
            vec![Ipv4Addr::new(10, 0, 0, 1)],
            vec![Ipv6Addr::LOCALHOST],
        );
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("host", 255); // ANY
        let resp = responder.handle_query(&query).unwrap();
        assert!(resp.len() > HEADER_SIZE);
    }

    #[test]
    fn test_responder_any_query_no_addrs() {
        let config = LlmnrResponderConfig::new("host", vec![], vec![]);
        let responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("host", 255);
        let resp = responder.handle_query(&query).unwrap();
        // Empty response (ANCOUNT=0)
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 0);
    }

    #[test]
    fn test_responder_unsupported_type() {
        let config = LlmnrResponderConfig::new("host", vec![Ipv4Addr::LOCALHOST], vec![]);
        let responder = LlmnrResponder::new(config);

        // MX query
        let query = build_test_llmnr_query("host", 15);
        let resp = responder.handle_query(&query).unwrap();
        // Empty but valid response
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 0);
    }

    #[test]
    fn test_responder_ignores_non_in_class() {
        let config = LlmnrResponderConfig::new("host", vec![Ipv4Addr::LOCALHOST], vec![]);
        let responder = LlmnrResponder::new(config);

        let mut query = build_test_llmnr_query("host", 1);
        // Change QCLASS to CH (3)
        let qclass_offset = query.len() - 2;
        query[qclass_offset] = 0;
        query[qclass_offset + 1] = 3;

        assert!(responder.handle_query(&query).is_none());
    }

    #[test]
    fn test_responder_accepts_any_class() {
        let config = LlmnrResponderConfig::new("host", vec![Ipv4Addr::LOCALHOST], vec![]);
        let responder = LlmnrResponder::new(config);

        let mut query = build_test_llmnr_query("host", 1);
        // Change QCLASS to ANY (255)
        let qclass_offset = query.len() - 2;
        query[qclass_offset] = 0;
        query[qclass_offset + 1] = 255;

        assert!(responder.handle_query(&query).is_some());
    }

    #[test]
    fn test_responder_set_hostname() {
        let config = LlmnrResponderConfig::new("old", vec![Ipv4Addr::LOCALHOST], vec![]);
        let mut responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("new", 1);
        assert!(responder.handle_query(&query).is_none());

        responder.set_hostname("new");
        assert!(responder.handle_query(&query).is_some());
    }

    #[test]
    fn test_responder_set_hostname_strips_domain() {
        let config = LlmnrResponderConfig::new("host", vec![Ipv4Addr::LOCALHOST], vec![]);
        let mut responder = LlmnrResponder::new(config);
        responder.set_hostname("newhost.example.com");
        assert_eq!(responder.hostname(), "newhost");
    }

    #[test]
    fn test_responder_set_addresses() {
        let config = LlmnrResponderConfig::new("host", vec![], vec![]);
        let mut responder = LlmnrResponder::new(config);

        let query = build_test_llmnr_query("host", 1);
        assert!(responder.handle_query(&query).is_none());

        responder.set_addresses(vec![Ipv4Addr::new(10, 0, 0, 1)], vec![]);
        assert!(responder.handle_query(&query).is_some());
    }

    #[test]
    fn test_responder_config_new_strips_domain() {
        let config = LlmnrResponderConfig::new("myhost.example.com", vec![], vec![]);
        assert_eq!(config.hostname, "myhost");
    }

    #[test]
    fn test_responder_config_new_lowercases() {
        let config = LlmnrResponderConfig::new("BIGHOST", vec![], vec![]);
        assert_eq!(config.hostname, "bighost");
    }

    // ── Response parsing tests ─────────────────────────────────────────

    #[test]
    fn test_parse_response_a() {
        let query = build_test_llmnr_query("host", 1);
        let resp = build_llmnr_a_response(&query, Ipv4Addr::new(192, 168, 1, 10)).unwrap();
        let parsed = parse_llmnr_response(&resp).unwrap();
        assert_eq!(parsed.ipv4, vec![Ipv4Addr::new(192, 168, 1, 10)]);
        assert!(parsed.ipv6.is_empty());
        assert!(!parsed.conflict);
    }

    #[test]
    fn test_parse_response_aaaa() {
        let query = build_test_llmnr_query("host", 28);
        let addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0, 0, 0, 1);
        let resp = build_llmnr_aaaa_response(&query, addr).unwrap();
        let parsed = parse_llmnr_response(&resp).unwrap();
        assert!(parsed.ipv4.is_empty());
        assert_eq!(parsed.ipv6, vec![addr]);
    }

    #[test]
    fn test_parse_response_too_short() {
        assert!(parse_llmnr_response(&[0; 5]).is_none());
    }

    #[test]
    fn test_parse_response_query_ignored() {
        let query = build_test_llmnr_query("host", 1);
        assert!(parse_llmnr_response(&query).is_none());
    }

    #[test]
    fn test_parse_response_conflict_flag() {
        let query = build_test_llmnr_query("host", 1);
        let mut resp = build_llmnr_a_response(&query, Ipv4Addr::new(10, 0, 0, 1)).unwrap();
        // Set the conflict/TC bit
        resp[CONFLICT_FLAG_BYTE] |= CONFLICT_FLAG_MASK;
        let parsed = parse_llmnr_response(&resp).unwrap();
        assert!(parsed.conflict);
    }

    #[test]
    fn test_parse_response_empty_answer() {
        let query = build_test_llmnr_query("host", 1);
        let resp = build_llmnr_empty_response(&query).unwrap();
        let parsed = parse_llmnr_response(&resp).unwrap();
        assert!(parsed.ipv4.is_empty());
        assert!(parsed.ipv6.is_empty());
    }

    // ── Resolver tests ─────────────────────────────────────────────────

    #[test]
    fn test_resolver_build_query_a() {
        let mut resolver = LlmnrResolver::new();
        let query = resolver.build_query("myhost", RecordType::A).unwrap();

        assert!(query.len() >= HEADER_SIZE);
        // QDCOUNT = 1
        assert_eq!(u16::from_be_bytes([query[4], query[5]]), 1);
        // Flags = 0 (standard query)
        assert_eq!(query[2], 0);
        assert_eq!(query[3], 0);
    }

    #[test]
    fn test_resolver_build_query_aaaa() {
        let mut resolver = LlmnrResolver::new();
        let query = resolver.build_query("printer", RecordType::AAAA).unwrap();
        assert!(query.len() >= HEADER_SIZE);
    }

    #[test]
    fn test_resolver_build_query_empty_name() {
        let mut resolver = LlmnrResolver::new();
        assert!(resolver.build_query("", RecordType::A).is_none());
    }

    #[test]
    fn test_resolver_build_query_multi_label_rejected() {
        let mut resolver = LlmnrResolver::new();
        assert!(resolver.build_query("host.local", RecordType::A).is_none());
    }

    #[test]
    fn test_resolver_build_query_trailing_dot_stripped() {
        let mut resolver = LlmnrResolver::new();
        // Single label with trailing dot should work
        let query = resolver.build_query("host.", RecordType::A);
        // "host." → trimmed to "host" → single label, OK
        assert!(query.is_some());
    }

    #[test]
    fn test_resolver_incrementing_ids() {
        let mut resolver = LlmnrResolver::new();
        let q1 = resolver.build_query("host", RecordType::A).unwrap();
        let q2 = resolver.build_query("host", RecordType::A).unwrap();
        let id1 = u16::from_be_bytes([q1[0], q1[1]]);
        let id2 = u16::from_be_bytes([q2[0], q2[1]]);
        assert_eq!(id2, id1 + 1);
    }

    #[test]
    fn test_resolver_default() {
        let resolver = LlmnrResolver::default();
        assert_eq!(resolver.timeout, LLMNR_TIMEOUT);
        assert_eq!(resolver.max_retries, LLMNR_MAX_RETRIES);
    }

    #[test]
    fn test_resolver_with_timeout() {
        let resolver = LlmnrResolver::new().with_timeout(Duration::from_millis(500));
        assert_eq!(resolver.timeout, Duration::from_millis(500));
    }

    // ── Scope check tests ──────────────────────────────────────────────

    #[test]
    fn test_is_link_local_ipv4() {
        assert!(is_link_local(&IpAddr::V4(Ipv4Addr::new(169, 254, 1, 1))));
        assert!(!is_link_local(&IpAddr::V4(Ipv4Addr::new(192, 168, 1, 1))));
        assert!(!is_link_local(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
    }

    #[test]
    fn test_is_link_local_ipv6() {
        assert!(is_link_local(&IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(!is_link_local(&IpAddr::V6(Ipv6Addr::new(
            0x2001, 0xdb8, 0, 0, 0, 0, 0, 1
        ))));
        assert!(!is_link_local(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
    }

    #[test]
    fn test_is_valid_source_ipv4() {
        assert!(is_valid_llmnr_source(&IpAddr::V4(Ipv4Addr::new(
            192, 168, 1, 1
        ))));
        assert!(!is_valid_llmnr_source(&IpAddr::V4(Ipv4Addr::LOCALHOST)));
        assert!(!is_valid_llmnr_source(&IpAddr::V4(Ipv4Addr::new(
            224, 0, 0, 252
        ))));
        assert!(!is_valid_llmnr_source(&IpAddr::V4(Ipv4Addr::UNSPECIFIED)));
    }

    #[test]
    fn test_is_valid_source_ipv6() {
        assert!(is_valid_llmnr_source(&IpAddr::V6(Ipv6Addr::new(
            0xfe80, 0, 0, 0, 0, 0, 0, 1
        ))));
        assert!(!is_valid_llmnr_source(&IpAddr::V6(Ipv6Addr::LOCALHOST)));
        assert!(!is_valid_llmnr_source(&IpAddr::V6(LLMNR_MCAST_V6)));
    }

    // ── Constants tests ────────────────────────────────────────────────

    #[test]
    fn test_constants() {
        assert_eq!(LLMNR_PORT, 5355);
        assert_eq!(LLMNR_MCAST_V4, Ipv4Addr::new(224, 0, 0, 252));
        assert_eq!(
            LLMNR_MCAST_V6,
            Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 0x0001, 0x0003)
        );
        assert_eq!(MAX_LLMNR_UDP_SIZE, 512);
        assert_eq!(LLMNR_TTL_V4, 1);
        assert_eq!(LLMNR_HOP_LIMIT_V6, 1);
        assert_eq!(LLMNR_RESPONSE_TTL, 30);
    }

    #[test]
    fn test_llmnr_mcast_addrs() {
        assert_eq!(LLMNR_MCAST_ADDR_V4.port(), LLMNR_PORT);
        assert_eq!(LLMNR_MCAST_ADDR_V6.port(), LLMNR_PORT);
    }

    // ── LlmnrError display tests ──────────────────────────────────────

    #[test]
    fn test_error_display() {
        assert_eq!(
            format!("{}", LlmnrError::TooShort),
            "LLMNR message too short"
        );
        assert_eq!(
            format!("{}", LlmnrError::NotAQuery),
            "LLMNR message is not a query"
        );
        assert_eq!(
            format!("{}", LlmnrError::BadOpcode),
            "LLMNR query has non-zero opcode"
        );
        assert_eq!(
            format!("{}", LlmnrError::BadQuestionCount),
            "LLMNR query QDCOUNT != 1"
        );
        assert_eq!(
            format!("{}", LlmnrError::NonZeroCounts),
            "LLMNR query has non-zero AN/NS count"
        );
        assert_eq!(
            format!("{}", LlmnrError::BadName),
            "LLMNR query name could not be parsed"
        );
        assert_eq!(
            format!("{}", LlmnrError::MultiLabelName),
            "LLMNR query name has multiple labels"
        );
        assert_eq!(
            format!("{}", LlmnrError::TruncatedQuery),
            "LLMNR query has TC bit set"
        );
    }

    // ── LlmnrStats tests ──────────────────────────────────────────────

    #[test]
    fn test_stats_default() {
        let stats = LlmnrStats::default();
        assert_eq!(stats.queries_received, 0);
        assert_eq!(stats.queries_answered, 0);
        assert_eq!(stats.queries_ignored, 0);
        assert_eq!(stats.conflicts_detected, 0);
    }

    #[test]
    fn test_stats_display() {
        let stats = LlmnrStats {
            queries_received: 100,
            queries_answered: 80,
            queries_ignored: 20,
            resolution_queries_sent: 5,
            resolution_responses: 3,
            conflicts_detected: 1,
        };
        let s = format!("{}", stats);
        assert!(s.contains("100"));
        assert!(s.contains("80"));
        assert!(s.contains("20"));
        assert!(s.contains("1"));
    }

    // ── LlmnrResult tests ─────────────────────────────────────────────

    #[test]
    fn test_llmnr_result_default_like() {
        let result = LlmnrResult {
            name: "host".to_string(),
            ipv4: vec![],
            ipv6: vec![],
            conflict: false,
            responder: None,
        };
        assert!(result.ipv4.is_empty());
        assert!(result.ipv6.is_empty());
        assert!(!result.conflict);
        assert!(result.responder.is_none());
    }

    // ── Integration: query → responder → parse_response ────────────────

    #[test]
    fn test_query_response_roundtrip_a() {
        let mut resolver = LlmnrResolver::new();
        let query_bytes = resolver.build_query("mypc", RecordType::A).unwrap();

        let config =
            LlmnrResponderConfig::new("mypc", vec![Ipv4Addr::new(192, 168, 1, 42)], vec![]);
        let responder = LlmnrResponder::new(config);

        let resp = responder.handle_query(&query_bytes).unwrap();
        let parsed = parse_llmnr_response(&resp).unwrap();

        assert_eq!(parsed.ipv4, vec![Ipv4Addr::new(192, 168, 1, 42)]);
        assert!(parsed.ipv6.is_empty());
        assert!(!parsed.conflict);
    }

    #[test]
    fn test_query_response_roundtrip_aaaa() {
        let mut resolver = LlmnrResolver::new();
        let query_bytes = resolver.build_query("mypc", RecordType::AAAA).unwrap();

        let addr = Ipv6Addr::new(0xfe80, 0, 0, 0, 0x1234, 0, 0, 0x5678);
        let config = LlmnrResponderConfig::new("mypc", vec![], vec![addr]);
        let responder = LlmnrResponder::new(config);

        let resp = responder.handle_query(&query_bytes).unwrap();
        let parsed = parse_llmnr_response(&resp).unwrap();

        assert!(parsed.ipv4.is_empty());
        assert_eq!(parsed.ipv6, vec![addr]);
    }

    #[test]
    fn test_query_response_wrong_name() {
        let mut resolver = LlmnrResolver::new();
        let query_bytes = resolver.build_query("other", RecordType::A).unwrap();

        let config = LlmnrResponderConfig::new("mypc", vec![Ipv4Addr::LOCALHOST], vec![]);
        let responder = LlmnrResponder::new(config);

        assert!(responder.handle_query(&query_bytes).is_none());
    }

    #[test]
    fn test_query_valid_llmnr_structure() {
        let mut resolver = LlmnrResolver::new();
        let query = resolver.build_query("test", RecordType::A).unwrap();

        // Verify it passes LLMNR validation
        let parsed = parse_llmnr_query(&query).unwrap();
        assert_eq!(parsed.name, "test");
        assert_eq!(parsed.qtype, RecordType::A);
    }

    #[test]
    fn test_generic_response_builder() {
        let query = build_test_llmnr_query("host", 1);
        let rdata = [10u8, 0, 0, 1]; // 10.0.0.1
        let resp = build_llmnr_response(&query, RecordType::A, &rdata, 60).unwrap();

        assert_ne!(resp[2] & 0x80, 0); // QR=1
        assert_eq!(u16::from_be_bytes([resp[6], resp[7]]), 1); // ANCOUNT=1
    }
}
