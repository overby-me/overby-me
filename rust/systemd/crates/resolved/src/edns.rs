#![allow(dead_code)]
//! EDNS0 (Extension Mechanisms for DNS) and ECS (EDNS Client Subnet) support.
//!
//! Implements RFC 6891 (EDNS0) OPT pseudo-record construction and parsing,
//! and RFC 7871 (EDNS Client Subnet) option for including client subnet
//! information in DNS queries to authoritative servers.
//!
//! ## EDNS0 (RFC 6891)
//!
//! EDNS0 extends the DNS protocol by adding an OPT pseudo-RR to the
//! additional section of DNS messages.  This allows:
//! - Larger UDP payload sizes (beyond the 512-byte limit)
//! - Extended RCODE and flags
//! - Carrying arbitrary options in the OPT RDATA
//!
//! ## ECS (RFC 7871)
//!
//! EDNS Client Subnet allows a recursive resolver to include a truncated
//! version of the client's IP address in queries sent to authoritative
//! servers, enabling better CDN/geo-DNS routing without revealing the
//! full client address.

use std::fmt;
use std::net::IpAddr;

use crate::dns::HEADER_SIZE;

// ── Constants ──────────────────────────────────────────────────────────────

/// OPT record type (RFC 6891 §6.1.1)
pub const RR_TYPE_OPT: u16 = 41;

/// Default EDNS0 advertised UDP payload size
pub const DEFAULT_EDNS_UDP_SIZE: u16 = 4096;

/// Minimum EDNS0 UDP payload size (RFC 6891 §6.2.3)
pub const MIN_EDNS_UDP_SIZE: u16 = 512;

/// Maximum EDNS0 UDP payload size
pub const MAX_EDNS_UDP_SIZE: u16 = 4096;

/// EDNS version we support
pub const EDNS_VERSION: u8 = 0;

/// EDNS option code for Client Subnet (RFC 7871)
pub const OPTION_CODE_ECS: u16 = 8;

/// EDNS option code for Cookie (RFC 7873)
pub const OPTION_CODE_COOKIE: u16 = 10;

/// EDNS option code for Padding (RFC 7830)
pub const OPTION_CODE_PADDING: u16 = 12;

/// Address family number: IPv4 (IANA)
pub const FAMILY_IPV4: u16 = 1;

/// Address family number: IPv6 (IANA)
pub const FAMILY_IPV6: u16 = 2;

/// Maximum source prefix length for ECS (IPv4)
pub const MAX_ECS_PREFIX_V4: u8 = 32;

/// Maximum source prefix length for ECS (IPv6)
pub const MAX_ECS_PREFIX_V6: u8 = 128;

/// Minimum OPT record size (empty RDATA): name(1) + type(2) + class(2) + ttl(4) + rdlen(2)
pub const OPT_FIXED_SIZE: usize = 11;

/// ECS option fixed header: option-code(2) + option-length(2) + family(2) + src_prefix(1) + scope_prefix(1)
pub const ECS_OPTION_HEADER_SIZE: usize = 8;

// ── EDNS0 flags ────────────────────────────────────────────────────────────

/// EDNS0 flags stored in the TTL field of the OPT record.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct EdnsFlags {
    /// DO (DNSSEC OK) bit — indicates the resolver can handle DNSSEC data.
    pub dnssec_ok: bool,
}

impl EdnsFlags {
    /// Encode flags into the upper 16 bits of the TTL field.
    pub fn to_u16(self) -> u16 {
        if self.dnssec_ok { 0x8000 } else { 0 }
    }

    /// Decode flags from the upper 16 bits of the TTL field.
    pub fn from_u16(val: u16) -> Self {
        Self {
            dnssec_ok: (val & 0x8000) != 0,
        }
    }
}

impl fmt::Display for EdnsFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dnssec_ok {
            write!(f, "DO")
        } else {
            write!(f, "(none)")
        }
    }
}

// ── EDNS Client Subnet ────────────────────────────────────────────────────

/// EDNS Client Subnet option (RFC 7871).
///
/// Contains a truncated client IP address prefix to allow authoritative
/// servers to return geographically appropriate answers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ClientSubnet {
    /// Address family (1 = IPv4, 2 = IPv6).
    pub family: u16,
    /// Number of significant bits of the address supplied by the client.
    pub source_prefix_length: u8,
    /// Number of significant bits used by the server for its answer scope
    /// (set in responses, 0 in queries).
    pub scope_prefix_length: u8,
    /// The leftmost `source_prefix_length` bits of the client address,
    /// zero-padded to a whole number of bytes.
    pub address: Vec<u8>,
}

impl ClientSubnet {
    /// Create an ECS option from an IP address and prefix length.
    ///
    /// The prefix length determines how many bits of the address are sent.
    /// For privacy, typical values are /24 for IPv4 and /56 for IPv6.
    pub fn new(addr: IpAddr, prefix_len: u8) -> Self {
        match addr {
            IpAddr::V4(v4) => {
                let prefix_len = prefix_len.min(MAX_ECS_PREFIX_V4);
                let octets = v4.octets();
                let addr_bytes = truncate_address(&octets, prefix_len);
                Self {
                    family: FAMILY_IPV4,
                    source_prefix_length: prefix_len,
                    scope_prefix_length: 0,
                    address: addr_bytes,
                }
            }
            IpAddr::V6(v6) => {
                let prefix_len = prefix_len.min(MAX_ECS_PREFIX_V6);
                let octets = v6.octets();
                let addr_bytes = truncate_address(&octets, prefix_len);
                Self {
                    family: FAMILY_IPV6,
                    source_prefix_length: prefix_len,
                    scope_prefix_length: 0,
                    address: addr_bytes,
                }
            }
        }
    }

    /// Encode this ECS option into wire format (option-code + option-data).
    pub fn to_bytes(&self) -> Vec<u8> {
        let addr_len = self.address.len();
        // Total option-data length: family(2) + src_prefix(1) + scope_prefix(1) + addr
        let option_data_len = 4 + addr_len;

        let mut buf = Vec::with_capacity(4 + option_data_len);
        // Option code
        buf.extend_from_slice(&OPTION_CODE_ECS.to_be_bytes());
        // Option length
        buf.extend_from_slice(&(option_data_len as u16).to_be_bytes());
        // Family
        buf.extend_from_slice(&self.family.to_be_bytes());
        // Source prefix length
        buf.push(self.source_prefix_length);
        // Scope prefix length
        buf.push(self.scope_prefix_length);
        // Address
        buf.extend_from_slice(&self.address);

        buf
    }

    /// Parse an ECS option from option-data bytes (after option-code and
    /// option-length have been consumed).
    pub fn parse(data: &[u8]) -> Option<Self> {
        // Minimum: family(2) + src_prefix(1) + scope_prefix(1) = 4
        if data.len() < 4 {
            return None;
        }

        let family = u16::from_be_bytes([data[0], data[1]]);
        let source_prefix_length = data[2];
        let scope_prefix_length = data[3];

        // Validate family and prefix length
        match family {
            FAMILY_IPV4 if source_prefix_length > MAX_ECS_PREFIX_V4 => return None,
            FAMILY_IPV6 if source_prefix_length > MAX_ECS_PREFIX_V6 => return None,
            FAMILY_IPV4 | FAMILY_IPV6 => {}
            _ => return None,
        }

        let expected_addr_bytes = prefix_byte_count(source_prefix_length);
        if data.len() < 4 + expected_addr_bytes {
            return None;
        }

        let address = data[4..4 + expected_addr_bytes].to_vec();

        Some(Self {
            family,
            source_prefix_length,
            scope_prefix_length,
            address,
        })
    }

    /// The wire size of the full option (option-code + option-length + option-data).
    pub fn wire_size(&self) -> usize {
        ECS_OPTION_HEADER_SIZE + self.address.len()
    }
}

impl fmt::Display for ClientSubnet {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let family_str = match self.family {
            FAMILY_IPV4 => "IPv4",
            FAMILY_IPV6 => "IPv6",
            _ => "unknown",
        };
        write!(
            f,
            "ECS {}/{} (scope /{})",
            family_str, self.source_prefix_length, self.scope_prefix_length
        )
    }
}

// ── EDNS0 OPT pseudo-record ───────────────────────────────────────────────

/// A parsed or constructed EDNS0 OPT record.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OptRecord {
    /// Advertised UDP payload size (stored in the CLASS field).
    pub udp_payload_size: u16,
    /// Extended RCODE (upper 8 bits, stored in TTL byte 0).
    pub extended_rcode: u8,
    /// EDNS version (stored in TTL byte 1).
    pub version: u8,
    /// EDNS flags (stored in TTL bytes 2-3).
    pub flags: EdnsFlags,
    /// EDNS options carried in RDATA.
    pub options: Vec<EdnsOption>,
}

/// A single EDNS option (generic).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EdnsOption {
    /// Option code.
    pub code: u16,
    /// Option data.
    pub data: Vec<u8>,
}

impl OptRecord {
    /// Create a new OPT record with default settings and no options.
    pub fn new() -> Self {
        Self {
            udp_payload_size: DEFAULT_EDNS_UDP_SIZE,
            extended_rcode: 0,
            version: EDNS_VERSION,
            flags: EdnsFlags::default(),
            options: Vec::new(),
        }
    }

    /// Create an OPT record with the DO (DNSSEC OK) flag set.
    pub fn with_dnssec_ok(mut self) -> Self {
        self.flags.dnssec_ok = true;
        self
    }

    /// Set the advertised UDP payload size.
    pub fn with_udp_size(mut self, size: u16) -> Self {
        self.udp_payload_size = size.max(MIN_EDNS_UDP_SIZE);
        self
    }

    /// Add an ECS option.
    pub fn with_client_subnet(mut self, ecs: ClientSubnet) -> Self {
        let data = {
            let mut d = Vec::new();
            d.extend_from_slice(&ecs.family.to_be_bytes());
            d.push(ecs.source_prefix_length);
            d.push(ecs.scope_prefix_length);
            d.extend_from_slice(&ecs.address);
            d
        };
        self.options.push(EdnsOption {
            code: OPTION_CODE_ECS,
            data,
        });
        self
    }

    /// Add a padding option (RFC 7830) to pad the message to a desired size.
    pub fn with_padding(mut self, padding_len: u16) -> Self {
        self.options.push(EdnsOption {
            code: OPTION_CODE_PADDING,
            data: vec![0u8; padding_len as usize],
        });
        self
    }

    /// Add a generic EDNS option.
    pub fn with_option(mut self, code: u16, data: Vec<u8>) -> Self {
        self.options.push(EdnsOption { code, data });
        self
    }

    /// Encode this OPT record into DNS wire format (a complete RR in the
    /// additional section).
    ///
    /// Wire format (RFC 6891 §6.1.2):
    /// ```text
    ///   NAME  = 0x00 (root domain)
    ///   TYPE  = 41 (OPT)
    ///   CLASS = UDP payload size
    ///   TTL   = extended-rcode(8) | version(8) | flags(16)
    ///   RDLEN = length of RDATA
    ///   RDATA = sequence of {option-code(2), option-length(2), option-data(*)}
    /// ```
    pub fn to_bytes(&self) -> Vec<u8> {
        // Build RDATA first
        let mut rdata = Vec::new();
        for opt in &self.options {
            rdata.extend_from_slice(&opt.code.to_be_bytes());
            rdata.extend_from_slice(&(opt.data.len() as u16).to_be_bytes());
            rdata.extend_from_slice(&opt.data);
        }

        let rdlen = rdata.len() as u16;
        let ttl = self.ttl_field();

        let mut buf = Vec::with_capacity(OPT_FIXED_SIZE + rdata.len());
        // Name: root (single zero byte)
        buf.push(0x00);
        // Type: OPT (41)
        buf.extend_from_slice(&RR_TYPE_OPT.to_be_bytes());
        // Class: UDP payload size
        buf.extend_from_slice(&self.udp_payload_size.to_be_bytes());
        // TTL: extended-rcode | version | flags
        buf.extend_from_slice(&ttl.to_be_bytes());
        // RDLEN
        buf.extend_from_slice(&rdlen.to_be_bytes());
        // RDATA
        buf.extend_from_slice(&rdata);

        buf
    }

    /// Parse an OPT record from a slice starting at the TYPE field.
    /// Assumes the NAME byte (0x00) has already been consumed and validated.
    ///
    /// `data` should start with: TYPE(2) CLASS(2) TTL(4) RDLEN(2) RDATA(*)
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < 10 {
            return None;
        }

        let rr_type = u16::from_be_bytes([data[0], data[1]]);
        if rr_type != RR_TYPE_OPT {
            return None;
        }

        let udp_payload_size = u16::from_be_bytes([data[2], data[3]]);
        let extended_rcode = data[4];
        let version = data[5];
        let flags = EdnsFlags::from_u16(u16::from_be_bytes([data[6], data[7]]));
        let rdlen = u16::from_be_bytes([data[8], data[9]]) as usize;

        if data.len() < 10 + rdlen {
            return None;
        }

        let rdata = &data[10..10 + rdlen];
        let options = parse_edns_options(rdata);

        Some(Self {
            udp_payload_size,
            extended_rcode,
            version,
            flags,
            options,
        })
    }

    /// Extract the ECS option from this OPT record, if present.
    pub fn client_subnet(&self) -> Option<ClientSubnet> {
        for opt in &self.options {
            if opt.code == OPTION_CODE_ECS {
                return ClientSubnet::parse(&opt.data);
            }
        }
        None
    }

    /// The total wire size of this OPT record.
    pub fn wire_size(&self) -> usize {
        let rdata_len: usize = self
            .options
            .iter()
            .map(|o| 4 + o.data.len()) // option-code(2) + option-length(2) + data
            .sum();
        OPT_FIXED_SIZE + rdata_len
    }

    /// Encode the TTL field: extended-rcode(8) | version(8) | flags(16).
    fn ttl_field(&self) -> u32 {
        let flags_u16 = self.flags.to_u16();
        ((self.extended_rcode as u32) << 24) | ((self.version as u32) << 16) | (flags_u16 as u32)
    }
}

impl Default for OptRecord {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Display for OptRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "OPT udp={} version={} flags={} options={}",
            self.udp_payload_size,
            self.version,
            self.flags,
            self.options.len()
        )
    }
}

// ── Query manipulation ─────────────────────────────────────────────────────

/// Append an OPT record to a DNS query message.
///
/// This adds the OPT record to the additional section and increments ARCOUNT.
/// If the message already has an OPT record in the additional section, this
/// function returns the message unchanged.
pub fn append_opt_to_query(query: &[u8], opt: &OptRecord) -> Option<Vec<u8>> {
    if query.len() < HEADER_SIZE {
        return None;
    }

    // Check if there's already an OPT in the additional section
    if has_opt_record(query) {
        return Some(query.to_vec());
    }

    let opt_bytes = opt.to_bytes();

    let mut result = Vec::with_capacity(query.len() + opt_bytes.len());
    result.extend_from_slice(query);
    result.extend_from_slice(&opt_bytes);

    // Increment ARCOUNT
    let arcount = u16::from_be_bytes([result[10], result[11]]);
    let new_arcount = arcount.checked_add(1)?;
    result[10..12].copy_from_slice(&new_arcount.to_be_bytes());

    Some(result)
}

/// Strip any OPT record from a DNS message's additional section.
///
/// This is useful when forwarding responses to clients that didn't send
/// EDNS0 in their query.  Returns the modified message.
pub fn strip_opt_from_message(msg: &[u8]) -> Option<Vec<u8>> {
    if msg.len() < HEADER_SIZE {
        return None;
    }

    let arcount = u16::from_be_bytes([msg[10], msg[11]]);
    if arcount == 0 {
        return Some(msg.to_vec());
    }

    // We need to walk through the message to find the additional section.
    // Skip: header + questions + answers + authority
    let qdcount = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let ancount = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    let nscount = u16::from_be_bytes([msg[8], msg[9]]) as usize;

    let mut offset = HEADER_SIZE;

    // Skip question section
    for _ in 0..qdcount {
        offset = skip_name_in_message(msg, offset)?;
        offset = offset.checked_add(4)?; // QTYPE + QCLASS
        if offset > msg.len() {
            return None;
        }
    }

    // Skip answer + authority RRs
    for _ in 0..(ancount + nscount) {
        offset = skip_rr(msg, offset)?;
    }

    // Now at the start of the additional section.
    let additional_start = offset;
    let mut new_arcount = arcount;
    let mut result = msg[..additional_start].to_vec();

    // Walk through additional records, copying all except OPT
    let mut pos = additional_start;
    for _ in 0..arcount {
        let rr_start = pos;
        let name_end = skip_name_in_message(msg, pos)?;
        if name_end + 10 > msg.len() {
            return None;
        }
        let rr_type = u16::from_be_bytes([msg[name_end], msg[name_end + 1]]);
        let rdlen = u16::from_be_bytes([msg[name_end + 8], msg[name_end + 9]]) as usize;
        let rr_end = name_end + 10 + rdlen;
        if rr_end > msg.len() {
            return None;
        }

        if rr_type == RR_TYPE_OPT {
            new_arcount = new_arcount.saturating_sub(1);
        } else {
            result.extend_from_slice(&msg[rr_start..rr_end]);
        }

        pos = rr_end;
    }

    // Update ARCOUNT
    result[10..12].copy_from_slice(&new_arcount.to_be_bytes());

    Some(result)
}

/// Check if a DNS message already contains an OPT record.
pub fn has_opt_record(msg: &[u8]) -> bool {
    if msg.len() < HEADER_SIZE {
        return false;
    }

    let arcount = u16::from_be_bytes([msg[10], msg[11]]);
    if arcount == 0 {
        return false;
    }

    let qdcount = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let ancount = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    let nscount = u16::from_be_bytes([msg[8], msg[9]]) as usize;

    let mut offset = HEADER_SIZE;

    // Skip question section
    for _ in 0..qdcount {
        if let Some(new_offset) = skip_name_in_message(msg, offset) {
            offset = new_offset + 4;
        } else {
            return false;
        }
    }

    // Skip answer + authority
    for _ in 0..(ancount + nscount) {
        if let Some(new_offset) = skip_rr(msg, offset) {
            offset = new_offset;
        } else {
            return false;
        }
    }

    // Check additional records for OPT
    for _ in 0..arcount {
        if let Some(name_end) = skip_name_in_message(msg, offset) {
            if name_end + 10 <= msg.len() {
                let rr_type = u16::from_be_bytes([msg[name_end], msg[name_end + 1]]);
                if rr_type == RR_TYPE_OPT {
                    return true;
                }
                let rdlen = u16::from_be_bytes([msg[name_end + 8], msg[name_end + 9]]) as usize;
                offset = name_end + 10 + rdlen;
            } else {
                return false;
            }
        } else {
            return false;
        }
    }

    false
}

/// Extract the OPT record from a DNS message, if present.
pub fn extract_opt_record(msg: &[u8]) -> Option<OptRecord> {
    if msg.len() < HEADER_SIZE {
        return None;
    }

    let arcount = u16::from_be_bytes([msg[10], msg[11]]);
    if arcount == 0 {
        return None;
    }

    let qdcount = u16::from_be_bytes([msg[4], msg[5]]) as usize;
    let ancount = u16::from_be_bytes([msg[6], msg[7]]) as usize;
    let nscount = u16::from_be_bytes([msg[8], msg[9]]) as usize;

    let mut offset = HEADER_SIZE;

    // Skip question section
    for _ in 0..qdcount {
        offset = skip_name_in_message(msg, offset)?;
        offset = offset.checked_add(4)?;
        if offset > msg.len() {
            return None;
        }
    }

    // Skip answer + authority
    for _ in 0..(ancount + nscount) {
        offset = skip_rr(msg, offset)?;
    }

    // Search additional records for OPT
    for _ in 0..arcount {
        let name_end = skip_name_in_message(msg, offset)?;
        if name_end + 10 > msg.len() {
            return None;
        }
        let rr_type = u16::from_be_bytes([msg[name_end], msg[name_end + 1]]);
        let rdlen = u16::from_be_bytes([msg[name_end + 8], msg[name_end + 9]]) as usize;
        let rr_end = name_end + 10 + rdlen;
        if rr_end > msg.len() {
            return None;
        }

        if rr_type == RR_TYPE_OPT {
            // Verify the name is root (the byte before name_end should be 0x00
            // for a root name, but since skip_name_in_message handled it, and
            // OPT MUST have root name, just parse from name_end)
            return OptRecord::parse(&msg[name_end..rr_end]);
        }

        offset = rr_end;
    }

    None
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Truncate an address byte array to the given prefix length, returning
/// only the bytes needed (ceiling of prefix_len/8), with trailing bits
/// zeroed.
fn truncate_address(octets: &[u8], prefix_len: u8) -> Vec<u8> {
    let byte_count = prefix_byte_count(prefix_len);
    if byte_count == 0 {
        return Vec::new();
    }

    let mut result = Vec::with_capacity(byte_count);
    for i in 0..byte_count {
        if i < octets.len() {
            result.push(octets[i]);
        } else {
            result.push(0);
        }
    }

    // Zero out the trailing bits in the last byte
    let trailing_bits = (prefix_len % 8) as usize;
    if trailing_bits > 0 && !result.is_empty() {
        let mask = !((1u8 << (8 - trailing_bits)) - 1);
        let last = result.len() - 1;
        result[last] &= mask;
    }

    result
}

/// Calculate the number of bytes needed to hold `prefix_len` bits.
fn prefix_byte_count(prefix_len: u8) -> usize {
    (prefix_len as usize).div_ceil(8)
}

/// Parse EDNS options from an RDATA buffer.
fn parse_edns_options(data: &[u8]) -> Vec<EdnsOption> {
    let mut options = Vec::new();
    let mut offset = 0;

    while offset + 4 <= data.len() {
        let code = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let length = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        offset += 4;

        if offset + length > data.len() {
            break;
        }

        options.push(EdnsOption {
            code,
            data: data[offset..offset + length].to_vec(),
        });

        offset += length;
    }

    options
}

/// Skip a DNS name in a message, returning the offset after the name.
fn skip_name_in_message(msg: &[u8], start: usize) -> Option<usize> {
    let mut offset = start;
    let mut hops = 0;
    let mut end_offset: Option<usize> = None;

    loop {
        if offset >= msg.len() {
            return None;
        }

        let len = msg[offset] as usize;

        if len == 0 {
            if end_offset.is_none() {
                end_offset = Some(offset + 1);
            }
            break;
        }

        if (len & 0xC0) == 0xC0 {
            if offset + 1 >= msg.len() {
                return None;
            }
            if end_offset.is_none() {
                end_offset = Some(offset + 2);
            }
            let ptr = ((len & 0x3F) << 8) | (msg[offset + 1] as usize);
            offset = ptr;
            hops += 1;
            if hops > 128 {
                return None;
            }
            continue;
        }

        offset += 1 + len;
        if offset > msg.len() {
            return None;
        }
    }

    end_offset
}

/// Skip a complete resource record, returning the offset after the RR.
fn skip_rr(msg: &[u8], start: usize) -> Option<usize> {
    let name_end = skip_name_in_message(msg, start)?;
    // TYPE(2) + CLASS(2) + TTL(4) + RDLENGTH(2) = 10
    if name_end + 10 > msg.len() {
        return None;
    }
    let rdlen = u16::from_be_bytes([msg[name_end + 8], msg[name_end + 9]]) as usize;
    let rr_end = name_end + 10 + rdlen;
    if rr_end > msg.len() {
        return None;
    }
    Some(rr_end)
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{Ipv4Addr, Ipv6Addr};

    // ── EdnsFlags tests ────────────────────────────────────────────────

    #[test]
    fn test_edns_flags_default() {
        let flags = EdnsFlags::default();
        assert!(!flags.dnssec_ok);
        assert_eq!(flags.to_u16(), 0);
    }

    #[test]
    fn fuzz_edns_parsers_never_panic() {
        // Robustness net (task #22): EDNS option parsing consumes
        // attacker-controlled OPT-record data. ClientSubnet::parse,
        // OptRecord::parse and parse_edns_options must never panic or hang on
        // malformed input (random + mutated blobs; catch_unwind on a worker
        // thread joined under a wall-clock budget).
        let handle = std::thread::spawn(|| {
            let mut state: u64 = 0x0ed0_5eed_0ed0_5eed;
            let mut next = || {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u32
            };
            for _ in 0..50_000u32 {
                let len = (next() % 160) as usize;
                let mut buf: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
                let muts = (next() % 6) as usize;
                for _ in 0..muts {
                    if buf.is_empty() {
                        break;
                    }
                    let i = (next() as usize) % buf.len();
                    match next() % 3 {
                        0 => buf[i] ^= (next() & 0xff) as u8,
                        1 => buf.truncate(i),
                        _ => buf.insert(i, (next() & 0xff) as u8),
                    }
                }
                let input = buf.clone();
                let res = std::panic::catch_unwind(move || {
                    let _ = ClientSubnet::parse(&input);
                    let _ = OptRecord::parse(&input);
                    let _ = parse_edns_options(&input);
                });
                assert!(res.is_ok(), "edns parser panicked on: {buf:?}");
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "edns fuzz did not finish in 30s -- a malformed option hangs the parser"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        handle.join().expect("fuzz worker panicked");
    }

    #[test]
    fn test_edns_flags_do_bit() {
        let flags = EdnsFlags { dnssec_ok: true };
        assert_eq!(flags.to_u16(), 0x8000);
    }

    #[test]
    fn test_edns_flags_roundtrip() {
        let flags = EdnsFlags { dnssec_ok: true };
        let decoded = EdnsFlags::from_u16(flags.to_u16());
        assert_eq!(flags, decoded);

        let flags = EdnsFlags::default();
        let decoded = EdnsFlags::from_u16(flags.to_u16());
        assert_eq!(flags, decoded);
    }

    #[test]
    fn test_edns_flags_display() {
        assert_eq!(format!("{}", EdnsFlags { dnssec_ok: true }), "DO");
        assert_eq!(format!("{}", EdnsFlags::default()), "(none)");
    }

    // ── ClientSubnet tests ─────────────────────────────────────────────

    #[test]
    fn test_ecs_ipv4_full() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 32);
        assert_eq!(ecs.family, FAMILY_IPV4);
        assert_eq!(ecs.source_prefix_length, 32);
        assert_eq!(ecs.scope_prefix_length, 0);
        assert_eq!(ecs.address, vec![192, 168, 1, 100]);
    }

    #[test]
    fn test_ecs_ipv4_24() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 24);
        assert_eq!(ecs.source_prefix_length, 24);
        // Only 3 bytes needed for /24
        assert_eq!(ecs.address, vec![192, 168, 1]);
    }

    #[test]
    fn test_ecs_ipv4_20() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 20);
        assert_eq!(ecs.source_prefix_length, 20);
        // 3 bytes needed for /20, last byte masked
        assert_eq!(ecs.address, vec![192, 168, 0]);
    }

    #[test]
    fn test_ecs_ipv4_0() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 0);
        assert_eq!(ecs.source_prefix_length, 0);
        assert!(ecs.address.is_empty());
    }

    #[test]
    fn test_ecs_ipv6_56() {
        let addr = Ipv6Addr::new(0x2001, 0xdb8, 0xabcd, 0xef01, 0, 0, 0, 1);
        let ecs = ClientSubnet::new(IpAddr::V6(addr), 56);
        assert_eq!(ecs.family, FAMILY_IPV6);
        assert_eq!(ecs.source_prefix_length, 56);
        // 7 bytes for /56
        assert_eq!(ecs.address.len(), 7);
        assert_eq!(ecs.address[0], 0x20);
        assert_eq!(ecs.address[1], 0x01);
    }

    #[test]
    fn test_ecs_ipv6_full() {
        let addr = Ipv6Addr::new(0x2001, 0xdb8, 0, 0, 0, 0, 0, 1);
        let ecs = ClientSubnet::new(IpAddr::V6(addr), 128);
        assert_eq!(ecs.source_prefix_length, 128);
        assert_eq!(ecs.address.len(), 16);
    }

    #[test]
    fn test_ecs_ipv4_clamped_prefix() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 64);
        // Should be clamped to 32
        assert_eq!(ecs.source_prefix_length, 32);
    }

    #[test]
    fn test_ecs_ipv6_clamped_prefix() {
        let addr = Ipv6Addr::LOCALHOST;
        let ecs = ClientSubnet::new(IpAddr::V6(addr), 200);
        // Should be clamped to 128
        assert_eq!(ecs.source_prefix_length, 128);
    }

    #[test]
    fn test_ecs_wire_roundtrip() {
        let original = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(10, 20, 30, 40)), 24);
        let bytes = original.to_bytes();

        // Skip option-code(2) and option-length(2) to get option-data
        assert!(bytes.len() >= 4);
        let parsed = ClientSubnet::parse(&bytes[4..]).unwrap();

        assert_eq!(parsed.family, original.family);
        assert_eq!(parsed.source_prefix_length, original.source_prefix_length);
        assert_eq!(parsed.scope_prefix_length, original.scope_prefix_length);
        assert_eq!(parsed.address, original.address);
    }

    #[test]
    fn test_ecs_parse_too_short() {
        assert!(ClientSubnet::parse(&[0, 1, 24]).is_none());
    }

    #[test]
    fn test_ecs_parse_bad_family() {
        let data = [0, 99, 24, 0, 10, 20, 30];
        assert!(ClientSubnet::parse(&data).is_none());
    }

    #[test]
    fn test_ecs_parse_prefix_too_long_ipv4() {
        // family=1 (IPv4), prefix=33, scope=0
        let data = [0, 1, 33, 0, 10, 20, 30, 40, 50];
        assert!(ClientSubnet::parse(&data).is_none());
    }

    #[test]
    fn test_ecs_parse_prefix_too_long_ipv6() {
        // family=2 (IPv6), prefix=129, scope=0
        let mut data = vec![0, 2, 129, 0];
        data.extend_from_slice(&[0u8; 17]);
        assert!(ClientSubnet::parse(&data).is_none());
    }

    #[test]
    fn test_ecs_parse_truncated_address() {
        // family=1 (IPv4), prefix=24, scope=0, but only 2 address bytes
        let data = [0, 1, 24, 0, 10, 20];
        assert!(ClientSubnet::parse(&data).is_none());
    }

    #[test]
    fn test_ecs_display() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 24);
        assert_eq!(format!("{}", ecs), "ECS IPv4/24 (scope /0)");

        let ecs = ClientSubnet::new(IpAddr::V6(Ipv6Addr::LOCALHOST), 56);
        assert_eq!(format!("{}", ecs), "ECS IPv6/56 (scope /0)");
    }

    #[test]
    fn test_ecs_wire_size() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 1)), 24);
        // header(8) + 3 address bytes
        assert_eq!(ecs.wire_size(), 11);
    }

    // ── OptRecord tests ────────────────────────────────────────────────

    #[test]
    fn test_opt_record_new() {
        let opt = OptRecord::new();
        assert_eq!(opt.udp_payload_size, DEFAULT_EDNS_UDP_SIZE);
        assert_eq!(opt.version, EDNS_VERSION);
        assert!(!opt.flags.dnssec_ok);
        assert!(opt.options.is_empty());
    }

    #[test]
    fn test_opt_record_default() {
        let opt = OptRecord::default();
        assert_eq!(opt, OptRecord::new());
    }

    #[test]
    fn test_opt_record_with_dnssec_ok() {
        let opt = OptRecord::new().with_dnssec_ok();
        assert!(opt.flags.dnssec_ok);
    }

    #[test]
    fn test_opt_record_with_udp_size() {
        let opt = OptRecord::new().with_udp_size(1232);
        assert_eq!(opt.udp_payload_size, 1232);
    }

    #[test]
    fn test_opt_record_with_udp_size_clamped() {
        let opt = OptRecord::new().with_udp_size(100);
        assert_eq!(opt.udp_payload_size, MIN_EDNS_UDP_SIZE);
    }

    #[test]
    fn test_opt_record_with_ecs() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(192, 168, 0, 0)), 16);
        let opt = OptRecord::new().with_client_subnet(ecs);
        assert_eq!(opt.options.len(), 1);
        assert_eq!(opt.options[0].code, OPTION_CODE_ECS);
    }

    #[test]
    fn test_opt_record_with_padding() {
        let opt = OptRecord::new().with_padding(32);
        assert_eq!(opt.options.len(), 1);
        assert_eq!(opt.options[0].code, OPTION_CODE_PADDING);
        assert_eq!(opt.options[0].data.len(), 32);
    }

    #[test]
    fn test_opt_record_to_bytes_empty() {
        let opt = OptRecord::new();
        let bytes = opt.to_bytes();
        // name(1) + type(2) + class(2) + ttl(4) + rdlen(2) = 11
        assert_eq!(bytes.len(), OPT_FIXED_SIZE);
        // Name = root
        assert_eq!(bytes[0], 0x00);
        // Type = OPT (41)
        assert_eq!(u16::from_be_bytes([bytes[1], bytes[2]]), RR_TYPE_OPT);
        // Class = UDP payload size
        assert_eq!(
            u16::from_be_bytes([bytes[3], bytes[4]]),
            DEFAULT_EDNS_UDP_SIZE
        );
        // RDLEN = 0
        assert_eq!(u16::from_be_bytes([bytes[9], bytes[10]]), 0);
    }

    #[test]
    fn test_opt_record_roundtrip() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8);
        let original = OptRecord::new()
            .with_dnssec_ok()
            .with_udp_size(1232)
            .with_client_subnet(ecs);

        let bytes = original.to_bytes();
        // Parse skipping the name byte (0x00)
        let parsed = OptRecord::parse(&bytes[1..]).unwrap();

        assert_eq!(parsed.udp_payload_size, 1232);
        assert!(parsed.flags.dnssec_ok);
        assert_eq!(parsed.options.len(), 1);
        assert_eq!(parsed.options[0].code, OPTION_CODE_ECS);

        let parsed_ecs = parsed.client_subnet().unwrap();
        assert_eq!(parsed_ecs.family, FAMILY_IPV4);
        assert_eq!(parsed_ecs.source_prefix_length, 8);
        assert_eq!(parsed_ecs.address, vec![10]);
    }

    #[test]
    fn test_opt_record_parse_too_short() {
        assert!(OptRecord::parse(&[0; 5]).is_none());
    }

    #[test]
    fn test_opt_record_parse_wrong_type() {
        let mut data = [0u8; 10];
        // Set type to 1 (A record) instead of 41
        data[0] = 0;
        data[1] = 1;
        assert!(OptRecord::parse(&data).is_none());
    }

    #[test]
    fn test_opt_record_parse_truncated_rdata() {
        let mut data = [0u8; 10];
        data[0] = 0;
        data[1] = 41; // TYPE = OPT
        data[8] = 0;
        data[9] = 10; // RDLEN = 10, but no rdata follows
        assert!(OptRecord::parse(&data).is_none());
    }

    #[test]
    fn test_opt_record_ttl_field() {
        let opt = OptRecord {
            udp_payload_size: 4096,
            extended_rcode: 0x01,
            version: 0,
            flags: EdnsFlags { dnssec_ok: true },
            options: Vec::new(),
        };
        let ttl = opt.ttl_field();
        // extended_rcode=0x01 in bits 24-31, version=0 in bits 16-23, DO=0x8000 in bits 0-15
        assert_eq!(ttl, 0x01008000);
    }

    #[test]
    fn test_opt_record_wire_size_empty() {
        let opt = OptRecord::new();
        assert_eq!(opt.wire_size(), OPT_FIXED_SIZE);
    }

    #[test]
    fn test_opt_record_wire_size_with_options() {
        let opt = OptRecord::new()
            .with_option(1, vec![0; 10])
            .with_padding(20);
        // OPT_FIXED_SIZE + opt1(4 + 10) + padding(4 + 20) = 11 + 14 + 24 = 49
        assert_eq!(opt.wire_size(), OPT_FIXED_SIZE + 14 + 24);
    }

    #[test]
    fn test_opt_record_display() {
        let opt = OptRecord::new();
        let s = format!("{}", opt);
        assert!(s.contains("OPT"));
        assert!(s.contains("4096"));
    }

    #[test]
    fn test_opt_record_client_subnet_none() {
        let opt = OptRecord::new();
        assert!(opt.client_subnet().is_none());
    }

    #[test]
    fn test_opt_record_multiple_options() {
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8);
        let opt = OptRecord::new()
            .with_client_subnet(ecs)
            .with_padding(16)
            .with_option(99, vec![1, 2, 3]);
        assert_eq!(opt.options.len(), 3);
    }

    // ── Query manipulation tests ───────────────────────────────────────

    /// Build a minimal DNS query for testing.
    fn build_test_query(name: &str, qtype: u16) -> Vec<u8> {
        let mut buf = vec![0u8; HEADER_SIZE];
        // ID
        buf[0] = 0xAB;
        buf[1] = 0xCD;
        // Flags: standard query, RD=1
        buf[2] = 0x01;
        buf[3] = 0x00;
        // QDCOUNT = 1
        buf[4] = 0x00;
        buf[5] = 0x01;

        // Question: encode name
        for label in name.split('.') {
            if label.is_empty() {
                continue;
            }
            buf.push(label.len() as u8);
            buf.extend_from_slice(label.as_bytes());
        }
        buf.push(0x00); // root

        // QTYPE
        buf.extend_from_slice(&qtype.to_be_bytes());
        // QCLASS = IN (1)
        buf.extend_from_slice(&1u16.to_be_bytes());

        buf
    }

    #[test]
    fn test_append_opt_to_query() {
        let query = build_test_query("example.com", 1);
        let arcount_before = u16::from_be_bytes([query[10], query[11]]);
        assert_eq!(arcount_before, 0);

        let opt = OptRecord::new();
        let result = append_opt_to_query(&query, &opt).unwrap();

        let arcount_after = u16::from_be_bytes([result[10], result[11]]);
        assert_eq!(arcount_after, 1);
        assert!(result.len() > query.len());
    }

    #[test]
    fn test_append_opt_to_query_too_short() {
        assert!(append_opt_to_query(&[0; 5], &OptRecord::new()).is_none());
    }

    #[test]
    fn test_append_opt_idempotent() {
        let query = build_test_query("example.com", 1);
        let opt = OptRecord::new();
        let with_opt = append_opt_to_query(&query, &opt).unwrap();
        let again = append_opt_to_query(&with_opt, &opt).unwrap();
        // Should not add a second OPT
        assert_eq!(with_opt.len(), again.len());
        assert_eq!(
            u16::from_be_bytes([again[10], again[11]]),
            1 // ARCOUNT still 1
        );
    }

    #[test]
    fn test_has_opt_record_false() {
        let query = build_test_query("example.com", 1);
        assert!(!has_opt_record(&query));
    }

    #[test]
    fn test_has_opt_record_true() {
        let query = build_test_query("example.com", 1);
        let opt = OptRecord::new();
        let with_opt = append_opt_to_query(&query, &opt).unwrap();
        assert!(has_opt_record(&with_opt));
    }

    #[test]
    fn test_has_opt_record_too_short() {
        assert!(!has_opt_record(&[0; 5]));
    }

    #[test]
    fn test_extract_opt_record() {
        let query = build_test_query("example.com", 1);
        let opt = OptRecord::new().with_dnssec_ok().with_udp_size(1232);
        let with_opt = append_opt_to_query(&query, &opt).unwrap();

        let extracted = extract_opt_record(&with_opt).unwrap();
        assert_eq!(extracted.udp_payload_size, 1232);
        assert!(extracted.flags.dnssec_ok);
    }

    #[test]
    fn test_extract_opt_record_none() {
        let query = build_test_query("example.com", 1);
        assert!(extract_opt_record(&query).is_none());
    }

    #[test]
    fn test_extract_opt_record_too_short() {
        assert!(extract_opt_record(&[0; 5]).is_none());
    }

    #[test]
    fn test_strip_opt_from_message() {
        let query = build_test_query("example.com", 1);
        let opt = OptRecord::new();
        let with_opt = append_opt_to_query(&query, &opt).unwrap();
        assert!(has_opt_record(&with_opt));

        let stripped = strip_opt_from_message(&with_opt).unwrap();
        assert!(!has_opt_record(&stripped));
        assert_eq!(u16::from_be_bytes([stripped[10], stripped[11]]), 0);
    }

    #[test]
    fn test_strip_opt_no_additional() {
        let query = build_test_query("example.com", 1);
        let result = strip_opt_from_message(&query).unwrap();
        assert_eq!(result, query);
    }

    #[test]
    fn test_strip_opt_too_short() {
        assert!(strip_opt_from_message(&[0; 5]).is_none());
    }

    #[test]
    fn test_append_opt_with_ecs() {
        let query = build_test_query("cdn.example.com", 1);
        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(198, 51, 100, 0)), 24);
        let opt = OptRecord::new().with_client_subnet(ecs);
        let result = append_opt_to_query(&query, &opt).unwrap();

        let extracted = extract_opt_record(&result).unwrap();
        let parsed_ecs = extracted.client_subnet().unwrap();
        assert_eq!(parsed_ecs.family, FAMILY_IPV4);
        assert_eq!(parsed_ecs.source_prefix_length, 24);
        assert_eq!(parsed_ecs.address, vec![198, 51, 100]);
    }

    // ── Helper tests ───────────────────────────────────────────────────

    #[test]
    fn test_truncate_address_full() {
        let addr = [192, 168, 1, 100];
        assert_eq!(truncate_address(&addr, 32), vec![192, 168, 1, 100]);
    }

    #[test]
    fn test_truncate_address_24() {
        let addr = [192, 168, 1, 100];
        assert_eq!(truncate_address(&addr, 24), vec![192, 168, 1]);
    }

    #[test]
    fn test_truncate_address_20() {
        let addr = [192, 168, 255, 100];
        // /20 → 3 bytes, last byte has bottom 4 bits zeroed
        // 255 & 0xF0 = 240
        assert_eq!(truncate_address(&addr, 20), vec![192, 168, 240]);
    }

    #[test]
    fn test_truncate_address_0() {
        let addr = [192, 168, 1, 100];
        assert_eq!(truncate_address(&addr, 0), Vec::<u8>::new());
    }

    #[test]
    fn test_truncate_address_1() {
        let addr = [192, 168, 1, 100];
        // /1 → 1 byte, only MSB kept: 192 & 0x80 = 128
        assert_eq!(truncate_address(&addr, 1), vec![128]);
    }

    #[test]
    fn test_truncate_address_8() {
        let addr = [10, 20, 30, 40];
        assert_eq!(truncate_address(&addr, 8), vec![10]);
    }

    #[test]
    fn test_prefix_byte_count() {
        assert_eq!(prefix_byte_count(0), 0);
        assert_eq!(prefix_byte_count(1), 1);
        assert_eq!(prefix_byte_count(7), 1);
        assert_eq!(prefix_byte_count(8), 1);
        assert_eq!(prefix_byte_count(9), 2);
        assert_eq!(prefix_byte_count(16), 2);
        assert_eq!(prefix_byte_count(24), 3);
        assert_eq!(prefix_byte_count(32), 4);
        assert_eq!(prefix_byte_count(128), 16);
    }

    #[test]
    fn test_parse_edns_options_empty() {
        let opts = parse_edns_options(&[]);
        assert!(opts.is_empty());
    }

    #[test]
    fn test_parse_edns_options_single() {
        // Option code=8, length=7, data=7 bytes
        let mut data = Vec::new();
        data.extend_from_slice(&8u16.to_be_bytes());
        data.extend_from_slice(&3u16.to_be_bytes());
        data.extend_from_slice(&[1, 2, 3]);

        let opts = parse_edns_options(&data);
        assert_eq!(opts.len(), 1);
        assert_eq!(opts[0].code, 8);
        assert_eq!(opts[0].data, vec![1, 2, 3]);
    }

    #[test]
    fn test_parse_edns_options_multiple() {
        let mut data = Vec::new();
        // Option 1: code=8, length=2
        data.extend_from_slice(&8u16.to_be_bytes());
        data.extend_from_slice(&2u16.to_be_bytes());
        data.extend_from_slice(&[0xAA, 0xBB]);
        // Option 2: code=12, length=3
        data.extend_from_slice(&12u16.to_be_bytes());
        data.extend_from_slice(&3u16.to_be_bytes());
        data.extend_from_slice(&[0, 0, 0]);

        let opts = parse_edns_options(&data);
        assert_eq!(opts.len(), 2);
        assert_eq!(opts[0].code, 8);
        assert_eq!(opts[1].code, 12);
    }

    #[test]
    fn test_parse_edns_options_truncated() {
        // Option code=8, length=10, but only 3 bytes of data
        let mut data = Vec::new();
        data.extend_from_slice(&8u16.to_be_bytes());
        data.extend_from_slice(&10u16.to_be_bytes());
        data.extend_from_slice(&[1, 2, 3]);

        let opts = parse_edns_options(&data);
        // Truncated option is not included
        assert!(opts.is_empty());
    }

    #[test]
    fn test_skip_name_in_message_simple() {
        // "example.com" encoded
        let data = [
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ];
        assert_eq!(skip_name_in_message(&data, 0), Some(13));
    }

    #[test]
    fn test_skip_name_in_message_root() {
        let data = [0u8];
        assert_eq!(skip_name_in_message(&data, 0), Some(1));
    }

    #[test]
    fn test_skip_name_in_message_compression() {
        // A name with a compression pointer at offset 0 pointing to offset 5
        let data = [0xC0, 0x05, 0, 0, 0, 3, b'c', b'o', b'm', 0];
        assert_eq!(skip_name_in_message(&data, 0), Some(2));
    }

    #[test]
    fn test_skip_name_in_message_empty() {
        assert!(skip_name_in_message(&[], 0).is_none());
    }

    #[test]
    fn test_skip_rr_basic() {
        // Name: root (1 byte) + TYPE(2) + CLASS(2) + TTL(4) + RDLEN(2) + RDATA(4)
        let mut data = vec![0x00]; // root name
        data.extend_from_slice(&1u16.to_be_bytes()); // TYPE A
        data.extend_from_slice(&1u16.to_be_bytes()); // CLASS IN
        data.extend_from_slice(&300u32.to_be_bytes()); // TTL
        data.extend_from_slice(&4u16.to_be_bytes()); // RDLEN
        data.extend_from_slice(&[10, 0, 0, 1]); // RDATA

        assert_eq!(skip_rr(&data, 0), Some(15));
    }

    #[test]
    fn test_skip_rr_truncated() {
        let data = [0x00, 0, 1]; // root name + partial TYPE
        assert!(skip_rr(&data, 0).is_none());
    }

    // ── Integration-style tests ────────────────────────────────────────

    #[test]
    fn test_full_edns_flow_ipv4() {
        // Build a query, add OPT with ECS, verify extraction
        let query = build_test_query("geo.example.com", 1);

        let ecs = ClientSubnet::new(IpAddr::V4(Ipv4Addr::new(203, 0, 113, 0)), 24);
        let opt = OptRecord::new()
            .with_udp_size(1232)
            .with_dnssec_ok()
            .with_client_subnet(ecs);

        let edns_query = append_opt_to_query(&query, &opt).unwrap();
        assert!(has_opt_record(&edns_query));

        let extracted = extract_opt_record(&edns_query).unwrap();
        assert_eq!(extracted.udp_payload_size, 1232);
        assert!(extracted.flags.dnssec_ok);

        let extracted_ecs = extracted.client_subnet().unwrap();
        assert_eq!(extracted_ecs.family, FAMILY_IPV4);
        assert_eq!(extracted_ecs.source_prefix_length, 24);
        assert_eq!(extracted_ecs.address, vec![203, 0, 113]);

        // Strip OPT and verify
        let stripped = strip_opt_from_message(&edns_query).unwrap();
        assert!(!has_opt_record(&stripped));
    }

    #[test]
    fn test_full_edns_flow_ipv6() {
        let query = build_test_query("cdn.example.org", 28); // AAAA

        let addr = Ipv6Addr::new(0x2001, 0xdb8, 0xface, 0, 0, 0, 0, 0);
        let ecs = ClientSubnet::new(IpAddr::V6(addr), 48);
        let opt = OptRecord::new().with_client_subnet(ecs);

        let edns_query = append_opt_to_query(&query, &opt).unwrap();

        let extracted = extract_opt_record(&edns_query).unwrap();
        let extracted_ecs = extracted.client_subnet().unwrap();
        assert_eq!(extracted_ecs.family, FAMILY_IPV6);
        assert_eq!(extracted_ecs.source_prefix_length, 48);
        assert_eq!(extracted_ecs.address.len(), 6);
    }
}
