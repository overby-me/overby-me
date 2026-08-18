#![allow(dead_code)]
//! DNSSEC validation — RFC 4033, RFC 4034, RFC 4035.
//!
//! This module provides DNSSEC record type parsing, signature verification
//! structures, and a validation framework for DNS responses.  It implements
//! the data structures and algorithms needed to validate DNSSEC-signed
//! responses from upstream resolvers.
//!
//! ## DNSSEC overview
//!
//! DNSSEC adds cryptographic signatures to DNS records, allowing resolvers
//! to verify that responses have not been tampered with.  The chain of trust
//! starts from a configured trust anchor (typically the root zone's KSK)
//! and follows DS→DNSKEY→RRSIG chains down to the queried zone.
//!
//! ## Record types
//!
//! - `DNSKEY` (48) — public key for a zone (RFC 4034 §2)
//! - `RRSIG`  (46) — signature over an RRset (RFC 4034 §3)
//! - `DS`     (43) — delegation signer linking parent to child (RFC 4034 §5)
//! - `NSEC`   (47) — authenticated denial of existence (RFC 4034 §4)
//! - `NSEC3`  (50) — hashed denial of existence (RFC 5155)
//!
//! ## Algorithms
//!
//! Supported algorithm numbers (RFC 8624):
//! - 8:  RSA/SHA-256
//! - 10: RSA/SHA-512
//! - 13: ECDSA Curve P-256 with SHA-256
//! - 14: ECDSA Curve P-384 with SHA-384
//! - 15: Ed25519
//! - 16: Ed448
//!
//! ## Digest types (for DS records)
//!
//! - 1: SHA-1 (deprecated but still seen)
//! - 2: SHA-256
//! - 4: SHA-384
//!
//! ## This module
//!
//! Provides:
//! - DNSSEC record parsing (DNSKEY, RRSIG, DS, NSEC, NSEC3)
//! - Algorithm and digest type enumerations
//! - Trust anchor management
//! - Validation result types
//! - RRset canonical ordering and signing helpers
//! - Wire-format canonical name encoding
//! - A `DnssecValidator` that checks chains of trust

use std::collections::HashMap;
use std::fmt;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crate::dns::RecordType;

// ── Constants ──────────────────────────────────────────────────────────────

/// DNSKEY record type (RFC 4034 §2)
pub const RR_TYPE_DNSKEY: u16 = 48;

/// RRSIG record type (RFC 4034 §3)
pub const RR_TYPE_RRSIG: u16 = 46;

/// DS record type (RFC 4034 §5)
pub const RR_TYPE_DS: u16 = 43;

/// NSEC record type (RFC 4034 §4)
pub const RR_TYPE_NSEC: u16 = 47;

/// NSEC3 record type (RFC 5155)
pub const RR_TYPE_NSEC3: u16 = 50;

/// NSEC3PARAM record type (RFC 5155)
pub const RR_TYPE_NSEC3PARAM: u16 = 51;

/// DNSKEY flag: Zone Key (bit 7, RFC 4034 §2.1.1)
pub const DNSKEY_FLAG_ZONE_KEY: u16 = 0x0100;

/// DNSKEY flag: Secure Entry Point / Key Signing Key (bit 15, RFC 4034 §2.1.1)
pub const DNSKEY_FLAG_SEP: u16 = 0x0001;

/// DNSKEY flag: Revoke (bit 8, RFC 5011 §3)
pub const DNSKEY_FLAG_REVOKE: u16 = 0x0080;

/// DNSKEY protocol value (always 3 per RFC 4034 §2.1.2)
pub const DNSKEY_PROTOCOL: u8 = 3;

/// Maximum RRSIG signature inception/expiration skew we tolerate (seconds).
/// Real systemd-resolved uses ~1 hour; we use 5 minutes for testing.
pub const SIGNATURE_JITTER_SECS: u64 = 300;

/// Maximum chain depth to prevent loops in validation.
pub const MAX_VALIDATION_DEPTH: usize = 16;

// ── DNSSEC algorithms ──────────────────────────────────────────────────────

/// DNSSEC algorithm numbers (RFC 8624, IANA registry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DnssecAlgorithm {
    /// Delete DS (RFC 8078)
    Delete,
    /// RSA/MD5 (deprecated, MUST NOT use)
    RsaMd5,
    /// Diffie-Hellman (RFC 2539, not for signing)
    DiffieHellman,
    /// DSA/SHA-1 (RFC 2536, MUST NOT use)
    DsaSha1,
    /// RSA/SHA-1 (RFC 3110, NOT RECOMMENDED)
    RsaSha1,
    /// DSA-NSEC3-SHA1 (RFC 5155, MUST NOT use)
    DsaNsec3Sha1,
    /// RSA/SHA-1-NSEC3 (RFC 5155, NOT RECOMMENDED)
    RsaSha1Nsec3,
    /// RSA/SHA-256 (RFC 5702, MUST support)
    RsaSha256,
    /// RSA/SHA-512 (RFC 5702, MUST support)
    RsaSha512,
    /// ECDSA P-256/SHA-256 (RFC 6605, MUST support)
    EcdsaP256Sha256,
    /// ECDSA P-384/SHA-384 (RFC 6605, MAY support)
    EcdsaP384Sha384,
    /// Ed25519 (RFC 8080, RECOMMENDED)
    Ed25519,
    /// Ed448 (RFC 8080, MAY support)
    Ed448,
    /// Indirect key (RFC 4034)
    Indirect,
    /// Private algorithm (domain name)
    PrivateDns,
    /// Private algorithm (OID)
    PrivateOid,
    /// Unknown algorithm number.
    Unknown(u8),
}

impl DnssecAlgorithm {
    /// Parse from the IANA algorithm number.
    pub fn from_u8(n: u8) -> Self {
        match n {
            0 => Self::Delete,
            1 => Self::RsaMd5,
            2 => Self::DiffieHellman,
            3 => Self::DsaSha1,
            5 => Self::RsaSha1,
            6 => Self::DsaNsec3Sha1,
            7 => Self::RsaSha1Nsec3,
            8 => Self::RsaSha256,
            10 => Self::RsaSha512,
            13 => Self::EcdsaP256Sha256,
            14 => Self::EcdsaP384Sha384,
            15 => Self::Ed25519,
            16 => Self::Ed448,
            252 => Self::Indirect,
            253 => Self::PrivateDns,
            254 => Self::PrivateOid,
            n => Self::Unknown(n),
        }
    }

    /// Convert to the IANA algorithm number.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Delete => 0,
            Self::RsaMd5 => 1,
            Self::DiffieHellman => 2,
            Self::DsaSha1 => 3,
            Self::RsaSha1 => 5,
            Self::DsaNsec3Sha1 => 6,
            Self::RsaSha1Nsec3 => 7,
            Self::RsaSha256 => 8,
            Self::RsaSha512 => 10,
            Self::EcdsaP256Sha256 => 13,
            Self::EcdsaP384Sha384 => 14,
            Self::Ed25519 => 15,
            Self::Ed448 => 16,
            Self::Indirect => 252,
            Self::PrivateDns => 253,
            Self::PrivateOid => 254,
            Self::Unknown(n) => n,
        }
    }

    /// Whether this algorithm is considered secure per RFC 8624.
    pub fn is_supported(self) -> bool {
        matches!(
            self,
            Self::RsaSha256
                | Self::RsaSha512
                | Self::EcdsaP256Sha256
                | Self::EcdsaP384Sha384
                | Self::Ed25519
                | Self::Ed448
        )
    }

    /// Whether this algorithm is explicitly deprecated / MUST NOT use.
    pub fn is_deprecated(self) -> bool {
        matches!(self, Self::RsaMd5 | Self::DsaSha1 | Self::DsaNsec3Sha1)
    }

    /// Mnemonic name for the algorithm.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::Delete => "DELETE",
            Self::RsaMd5 => "RSAMD5",
            Self::DiffieHellman => "DH",
            Self::DsaSha1 => "DSA",
            Self::RsaSha1 => "RSASHA1",
            Self::DsaNsec3Sha1 => "DSA-NSEC3-SHA1",
            Self::RsaSha1Nsec3 => "RSASHA1-NSEC3-SHA1",
            Self::RsaSha256 => "RSASHA256",
            Self::RsaSha512 => "RSASHA512",
            Self::EcdsaP256Sha256 => "ECDSAP256SHA256",
            Self::EcdsaP384Sha384 => "ECDSAP384SHA384",
            Self::Ed25519 => "ED25519",
            Self::Ed448 => "ED448",
            Self::Indirect => "INDIRECT",
            Self::PrivateDns => "PRIVATEDNS",
            Self::PrivateOid => "PRIVATEOID",
            Self::Unknown(_) => "UNKNOWN",
        }
    }
}

impl fmt::Display for DnssecAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.mnemonic(), self.to_u8())
    }
}

// ── DS digest types ────────────────────────────────────────────────────────

/// Digest types used in DS records (RFC 4034 §5.1.3, IANA registry).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum DigestType {
    /// SHA-1 (RFC 3658) — deprecated but still in use.
    Sha1,
    /// SHA-256 (RFC 4509) — MUST support.
    Sha256,
    /// GOST R 34.11-94 (RFC 5933) — not widely used.
    GostR34_11_94,
    /// SHA-384 (RFC 6605) — MAY support.
    Sha384,
    /// Unknown digest type.
    Unknown(u8),
}

impl DigestType {
    /// Parse from the IANA digest type number.
    pub fn from_u8(n: u8) -> Self {
        match n {
            1 => Self::Sha1,
            2 => Self::Sha256,
            3 => Self::GostR34_11_94,
            4 => Self::Sha384,
            n => Self::Unknown(n),
        }
    }

    /// Convert to the IANA digest type number.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Sha1 => 1,
            Self::Sha256 => 2,
            Self::GostR34_11_94 => 3,
            Self::Sha384 => 4,
            Self::Unknown(n) => n,
        }
    }

    /// Whether this digest type is considered secure.
    pub fn is_supported(self) -> bool {
        matches!(self, Self::Sha256 | Self::Sha384)
    }

    /// The expected digest length in bytes.
    pub fn digest_length(self) -> Option<usize> {
        match self {
            Self::Sha1 => Some(20),
            Self::Sha256 => Some(32),
            Self::Sha384 => Some(48),
            _ => None,
        }
    }

    /// Mnemonic name.
    pub fn mnemonic(self) -> &'static str {
        match self {
            Self::Sha1 => "SHA-1",
            Self::Sha256 => "SHA-256",
            Self::GostR34_11_94 => "GOST",
            Self::Sha384 => "SHA-384",
            Self::Unknown(_) => "UNKNOWN",
        }
    }
}

impl fmt::Display for DigestType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}({})", self.mnemonic(), self.to_u8())
    }
}

// ── NSEC3 hash algorithms ──────────────────────────────────────────────────

/// Hash algorithms used in NSEC3 records (RFC 5155 §11).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Nsec3HashAlgorithm {
    /// SHA-1 (RFC 5155)
    Sha1,
    /// Unknown algorithm.
    Unknown(u8),
}

impl Nsec3HashAlgorithm {
    /// Parse from the IANA hash algorithm number.
    pub fn from_u8(n: u8) -> Self {
        match n {
            1 => Self::Sha1,
            n => Self::Unknown(n),
        }
    }

    /// Convert to the IANA number.
    pub fn to_u8(self) -> u8 {
        match self {
            Self::Sha1 => 1,
            Self::Unknown(n) => n,
        }
    }
}

impl fmt::Display for Nsec3HashAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Sha1 => write!(f, "SHA-1(1)"),
            Self::Unknown(n) => write!(f, "UNKNOWN({})", n),
        }
    }
}

// ── DNSKEY record ──────────────────────────────────────────────────────────

/// A parsed DNSKEY record (RFC 4034 §2).
///
/// RDATA wire format:
/// ```text
///   Flags(2) | Protocol(1) | Algorithm(1) | Public Key(variable)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DnskeyRecord {
    /// Owner name of the DNSKEY record.
    pub name: String,
    /// Flags field.
    pub flags: u16,
    /// Protocol (always 3).
    pub protocol: u8,
    /// Algorithm number.
    pub algorithm: DnssecAlgorithm,
    /// Raw public key bytes.
    pub public_key: Vec<u8>,
    /// Original TTL from the RRset.
    pub ttl: u32,
}

impl DnskeyRecord {
    /// Parse a DNSKEY record from its RDATA.
    pub fn parse(name: &str, ttl: u32, rdata: &[u8]) -> Option<Self> {
        // Minimum: flags(2) + protocol(1) + algorithm(1) = 4 bytes
        if rdata.len() < 4 {
            return None;
        }

        let flags = u16::from_be_bytes([rdata[0], rdata[1]]);
        let protocol = rdata[2];
        let algorithm = DnssecAlgorithm::from_u8(rdata[3]);
        let public_key = rdata[4..].to_vec();

        Some(Self {
            name: name.to_lowercase(),
            flags,
            protocol,
            algorithm,
            public_key,
            ttl,
        })
    }

    /// Whether this is a Zone Signing Key (ZSK).
    ///
    /// A ZSK has the Zone Key flag set but NOT the SEP flag.
    pub fn is_zone_key(&self) -> bool {
        (self.flags & DNSKEY_FLAG_ZONE_KEY) != 0
    }

    /// Whether this is a Key Signing Key (KSK) / Secure Entry Point.
    pub fn is_sep(&self) -> bool {
        (self.flags & DNSKEY_FLAG_SEP) != 0
    }

    /// Whether the REVOKE flag is set (RFC 5011).
    pub fn is_revoked(&self) -> bool {
        (self.flags & DNSKEY_FLAG_REVOKE) != 0
    }

    /// Compute the key tag for this DNSKEY (RFC 4034 Appendix B).
    ///
    /// The key tag is a 16-bit value used to efficiently select the
    /// correct DNSKEY for RRSIG verification.
    pub fn key_tag(&self) -> u16 {
        let rdata = self.to_rdata();
        compute_key_tag(&rdata)
    }

    /// Encode the DNSKEY RDATA to wire format.
    pub fn to_rdata(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.public_key.len());
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.push(self.protocol);
        buf.push(self.algorithm.to_u8());
        buf.extend_from_slice(&self.public_key);
        buf
    }
}

impl fmt::Display for DnskeyRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} DNSKEY {} {} {} (tag={}{}{})",
            self.name,
            self.flags,
            self.protocol,
            self.algorithm,
            self.key_tag(),
            if self.is_sep() { " SEP" } else { "" },
            if self.is_revoked() { " REVOKED" } else { "" },
        )
    }
}

// ── RRSIG record ───────────────────────────────────────────────────────────

/// A parsed RRSIG record (RFC 4034 §3).
///
/// RDATA wire format:
/// ```text
///   Type Covered(2) | Algorithm(1) | Labels(1) | Original TTL(4) |
///   Signature Expiration(4) | Signature Inception(4) | Key Tag(2) |
///   Signer's Name(variable) | Signature(variable)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RrsigRecord {
    /// Owner name of the RRSIG record.
    pub name: String,
    /// The RR type covered by this signature.
    pub type_covered: u16,
    /// Algorithm used for the signature.
    pub algorithm: DnssecAlgorithm,
    /// Number of labels in the original owner name (for wildcard detection).
    pub labels: u8,
    /// Original TTL of the covered RRset.
    pub original_ttl: u32,
    /// Signature expiration time (seconds since Unix epoch).
    pub sig_expiration: u32,
    /// Signature inception time (seconds since Unix epoch).
    pub sig_inception: u32,
    /// Key tag of the DNSKEY that created this signature.
    pub key_tag: u16,
    /// The signer's domain name.
    pub signer_name: String,
    /// The raw cryptographic signature bytes.
    pub signature: Vec<u8>,
    /// TTL from the RR.
    pub ttl: u32,
}

impl RrsigRecord {
    /// Parse an RRSIG record from its RDATA.
    ///
    /// `rdata` must start at the beginning of the RRSIG RDATA (after the
    /// standard RR header NAME/TYPE/CLASS/TTL/RDLENGTH).
    pub fn parse(name: &str, ttl: u32, rdata: &[u8]) -> Option<Self> {
        // Minimum fixed fields: 18 bytes before signer's name
        if rdata.len() < 18 {
            return None;
        }

        let type_covered = u16::from_be_bytes([rdata[0], rdata[1]]);
        let algorithm = DnssecAlgorithm::from_u8(rdata[2]);
        let labels = rdata[3];
        let original_ttl = u32::from_be_bytes([rdata[4], rdata[5], rdata[6], rdata[7]]);
        let sig_expiration = u32::from_be_bytes([rdata[8], rdata[9], rdata[10], rdata[11]]);
        let sig_inception = u32::from_be_bytes([rdata[12], rdata[13], rdata[14], rdata[15]]);
        let key_tag = u16::from_be_bytes([rdata[16], rdata[17]]);

        // Parse the signer's name (uncompressed in RRSIG RDATA)
        let (signer_name, name_end) = parse_name_uncompressed(rdata, 18)?;

        if name_end > rdata.len() {
            return None;
        }

        let signature = rdata[name_end..].to_vec();

        Some(Self {
            name: name.to_lowercase(),
            type_covered,
            algorithm,
            labels,
            original_ttl,
            sig_expiration,
            sig_inception,
            key_tag,
            signer_name: signer_name.to_lowercase(),
            signature,
            ttl,
        })
    }

    /// Check whether the signature is currently valid (within the
    /// inception–expiration window, with a configurable jitter).
    pub fn is_time_valid(&self) -> bool {
        self.is_time_valid_at(current_unix_time())
    }

    /// Check time validity against a specific Unix timestamp.
    pub fn is_time_valid_at(&self, now: u32) -> bool {
        let inception = self
            .sig_inception
            .saturating_sub(SIGNATURE_JITTER_SECS as u32);
        let expiration = self
            .sig_expiration
            .saturating_add(SIGNATURE_JITTER_SECS as u32);
        now >= inception && now <= expiration
    }

    /// Whether this signature covers the given RR type.
    pub fn covers(&self, rr_type: u16) -> bool {
        self.type_covered == rr_type
    }

    /// Whether the owner name may have been synthesized from a wildcard.
    ///
    /// If the number of labels in the RRSIG is less than the number of
    /// labels in the owner name, the response was generated from a
    /// wildcard (RFC 4034 §3.1.3).
    pub fn is_wildcard(&self) -> bool {
        let owner_labels = count_labels(&self.name);
        self.labels < owner_labels
    }

    /// Encode the RRSIG RDATA prefix (everything except the signature)
    /// for use as input to the signature verification algorithm.
    ///
    /// Per RFC 4034 §3.1.8.1, the signature covers:
    ///   RRSIG_RDATA_prefix | RR(1) | RR(2) | ...
    pub fn sig_data_prefix(&self) -> Option<Vec<u8>> {
        let signer_wire = canonical_name_wire(&self.signer_name)?;

        let mut buf = Vec::with_capacity(18 + signer_wire.len());
        buf.extend_from_slice(&self.type_covered.to_be_bytes());
        buf.push(self.algorithm.to_u8());
        buf.push(self.labels);
        buf.extend_from_slice(&self.original_ttl.to_be_bytes());
        buf.extend_from_slice(&self.sig_expiration.to_be_bytes());
        buf.extend_from_slice(&self.sig_inception.to_be_bytes());
        buf.extend_from_slice(&self.key_tag.to_be_bytes());
        buf.extend_from_slice(&signer_wire);

        Some(buf)
    }
}

impl fmt::Display for RrsigRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} RRSIG {} {} {} {} {} {} {} {}",
            self.name,
            self.type_covered,
            self.algorithm,
            self.labels,
            self.original_ttl,
            self.sig_expiration,
            self.sig_inception,
            self.key_tag,
            self.signer_name,
        )
    }
}

// ── DS record ──────────────────────────────────────────────────────────────

/// A parsed DS (Delegation Signer) record (RFC 4034 §5).
///
/// RDATA wire format:
/// ```text
///   Key Tag(2) | Algorithm(1) | Digest Type(1) | Digest(variable)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DsRecord {
    /// Owner name (the child zone name).
    pub name: String,
    /// Key tag of the DNSKEY this DS refers to.
    pub key_tag: u16,
    /// Algorithm of the DNSKEY this DS refers to.
    pub algorithm: DnssecAlgorithm,
    /// Digest type used.
    pub digest_type: DigestType,
    /// The digest bytes.
    pub digest: Vec<u8>,
    /// TTL from the RR.
    pub ttl: u32,
}

impl DsRecord {
    /// Parse a DS record from its RDATA.
    pub fn parse(name: &str, ttl: u32, rdata: &[u8]) -> Option<Self> {
        // Minimum: key_tag(2) + algorithm(1) + digest_type(1) + at least 1 byte digest
        if rdata.len() < 5 {
            return None;
        }

        let key_tag = u16::from_be_bytes([rdata[0], rdata[1]]);
        let algorithm = DnssecAlgorithm::from_u8(rdata[2]);
        let digest_type = DigestType::from_u8(rdata[3]);
        let digest = rdata[4..].to_vec();

        Some(Self {
            name: name.to_lowercase(),
            key_tag,
            algorithm,
            digest_type,
            digest,
            ttl,
        })
    }

    /// Verify that this DS record matches a given DNSKEY record.
    ///
    /// The DS digest is computed over: owner_name_wire || DNSKEY_RDATA.
    /// This function computes the expected digest and compares it to
    /// `self.digest`.
    ///
    /// Note: actual hash computation requires a crypto library. This
    /// method returns `None` if the digest type is unsupported, `Some(true)`
    /// if the digests match, and `Some(false)` if they don't.
    pub fn matches_dnskey(&self, dnskey: &DnskeyRecord) -> Option<bool> {
        // Check basic compatibility
        if dnskey.algorithm != self.algorithm {
            return Some(false);
        }
        if dnskey.key_tag() != self.key_tag {
            return Some(false);
        }

        // We need the owner name in canonical wire format + DNSKEY RDATA
        let owner_wire = canonical_name_wire(&dnskey.name)?;
        let dnskey_rdata = dnskey.to_rdata();

        let mut hash_input = Vec::with_capacity(owner_wire.len() + dnskey_rdata.len());
        hash_input.extend_from_slice(&owner_wire);
        hash_input.extend_from_slice(&dnskey_rdata);

        // The actual hashing would require a crypto library.
        // For now we validate the structure and defer to a crypto backend.
        let expected_len = self.digest_type.digest_length()?;
        if self.digest.len() != expected_len {
            return Some(false);
        }

        // Return None to indicate "we validated the structure but can't
        // compute the hash without a crypto library."  A real implementation
        // would compute SHA-256/SHA-384 here and compare.
        None
    }

    /// Encode the DS RDATA to wire format.
    pub fn to_rdata(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.digest.len());
        buf.extend_from_slice(&self.key_tag.to_be_bytes());
        buf.push(self.algorithm.to_u8());
        buf.push(self.digest_type.to_u8());
        buf.extend_from_slice(&self.digest);
        buf
    }
}

impl fmt::Display for DsRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} DS {} {} {} ({}B digest)",
            self.name,
            self.key_tag,
            self.algorithm,
            self.digest_type,
            self.digest.len(),
        )
    }
}

// ── NSEC record ────────────────────────────────────────────────────────────

/// A parsed NSEC record (RFC 4034 §4).
///
/// RDATA wire format:
/// ```text
///   Next Domain Name(variable) | Type Bit Maps(variable)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NsecRecord {
    /// Owner name.
    pub name: String,
    /// The next owner name in canonical order.
    pub next_name: String,
    /// The set of RR types that exist at the owner name.
    pub types: Vec<u16>,
    /// TTL.
    pub ttl: u32,
}

impl NsecRecord {
    /// Parse an NSEC record from its RDATA.
    pub fn parse(name: &str, ttl: u32, rdata: &[u8]) -> Option<Self> {
        if rdata.is_empty() {
            return None;
        }

        // Parse next domain name (uncompressed)
        let (next_name, name_end) = parse_name_uncompressed(rdata, 0)?;

        // Parse type bit maps
        let types = parse_type_bit_maps(&rdata[name_end..])?;

        Some(Self {
            name: name.to_lowercase(),
            next_name: next_name.to_lowercase(),
            types,
            ttl,
        })
    }

    /// Check whether a given RR type exists at the owner name.
    pub fn has_type(&self, rr_type: u16) -> bool {
        self.types.contains(&rr_type)
    }

    /// Check whether this NSEC record proves that a given name does not
    /// exist (the name falls between `self.name` and `self.next_name`
    /// in canonical order).
    pub fn denies_name(&self, qname: &str) -> bool {
        let qname = qname.trim_end_matches('.').to_lowercase();
        let owner = self.name.trim_end_matches('.').to_lowercase();
        let next = self.next_name.trim_end_matches('.').to_lowercase();

        if owner == next {
            // Single-record zone or last NSEC wrapping around
            return qname != owner;
        }

        // Standard case: owner < qname < next in canonical order
        if canonical_name_order(&owner, &next) < std::cmp::Ordering::Equal {
            // Normal range
            canonical_name_order(&owner, &qname) == std::cmp::Ordering::Less
                && canonical_name_order(&qname, &next) == std::cmp::Ordering::Less
        } else {
            // Wrap-around: owner > next means the zone's last NSEC
            canonical_name_order(&owner, &qname) == std::cmp::Ordering::Less
                || canonical_name_order(&qname, &next) == std::cmp::Ordering::Less
        }
    }

    /// Check whether this NSEC proves that a given RR type does not exist
    /// at the owner name (the name matches but the type is not in the bitmap).
    pub fn denies_type(&self, qname: &str, rr_type: u16) -> bool {
        let qname = qname.trim_end_matches('.').to_lowercase();
        let owner = self.name.trim_end_matches('.').to_lowercase();
        qname == owner && !self.has_type(rr_type)
    }
}

impl fmt::Display for NsecRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let type_strs: Vec<String> = self
            .types
            .iter()
            .map(|t| format!("{}", RecordType::from_u16(*t)))
            .collect();
        write!(
            f,
            "{} NSEC {} ({})",
            self.name,
            self.next_name,
            type_strs.join(" "),
        )
    }
}

// ── NSEC3 record ───────────────────────────────────────────────────────────

/// A parsed NSEC3 record (RFC 5155).
///
/// RDATA wire format:
/// ```text
///   Hash Algorithm(1) | Flags(1) | Iterations(2) | Salt Length(1) |
///   Salt(variable) | Hash Length(1) | Next Hashed Owner(variable) |
///   Type Bit Maps(variable)
/// ```
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Nsec3Record {
    /// Owner name.
    pub name: String,
    /// Hash algorithm.
    pub hash_algorithm: Nsec3HashAlgorithm,
    /// Flags (bit 0 = Opt-Out).
    pub flags: u8,
    /// Number of additional hash iterations.
    pub iterations: u16,
    /// Salt value.
    pub salt: Vec<u8>,
    /// Next hashed owner name (binary, base32hex-encoded in presentation).
    pub next_hashed_owner: Vec<u8>,
    /// RR types that exist at the original owner name.
    pub types: Vec<u16>,
    /// TTL.
    pub ttl: u32,
}

impl Nsec3Record {
    /// Parse an NSEC3 record from its RDATA.
    pub fn parse(name: &str, ttl: u32, rdata: &[u8]) -> Option<Self> {
        // Minimum: hash_alg(1) + flags(1) + iterations(2) + salt_len(1) = 5
        if rdata.len() < 5 {
            return None;
        }

        let hash_algorithm = Nsec3HashAlgorithm::from_u8(rdata[0]);
        let flags = rdata[1];
        let iterations = u16::from_be_bytes([rdata[2], rdata[3]]);
        let salt_len = rdata[4] as usize;

        let mut offset = 5;
        if offset + salt_len > rdata.len() {
            return None;
        }
        let salt = rdata[offset..offset + salt_len].to_vec();
        offset += salt_len;

        if offset >= rdata.len() {
            return None;
        }
        let hash_len = rdata[offset] as usize;
        offset += 1;

        if offset + hash_len > rdata.len() {
            return None;
        }
        let next_hashed_owner = rdata[offset..offset + hash_len].to_vec();
        offset += hash_len;

        let types = parse_type_bit_maps(&rdata[offset..]).unwrap_or_default();

        Some(Self {
            name: name.to_lowercase(),
            hash_algorithm,
            flags,
            iterations,
            salt,
            next_hashed_owner,
            types,
            ttl,
        })
    }

    /// Whether the Opt-Out flag is set (RFC 5155 §3.1.2.1).
    pub fn is_opt_out(&self) -> bool {
        (self.flags & 0x01) != 0
    }

    /// Check whether a given RR type exists.
    pub fn has_type(&self, rr_type: u16) -> bool {
        self.types.contains(&rr_type)
    }
}

impl fmt::Display for Nsec3Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} NSEC3 {} {} {} (salt={}B, hash={}B, {} types{})",
            self.name,
            self.hash_algorithm,
            self.flags,
            self.iterations,
            self.salt.len(),
            self.next_hashed_owner.len(),
            self.types.len(),
            if self.is_opt_out() { " opt-out" } else { "" },
        )
    }
}

// ── Trust anchor ───────────────────────────────────────────────────────────

/// A DNSSEC trust anchor — a known-good DNSKEY or DS record.
///
/// The root zone trust anchor is typically built in or configured.
/// systemd-resolved reads trust anchors from `/etc/dnssec-trust-anchors.d/`
/// and `/usr/lib/dnssec-trust-anchors.d/`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TrustAnchor {
    /// A trusted DNSKEY (usually the root zone KSK).
    DnsKey(DnskeyRecord),
    /// A trusted DS record.
    Ds(DsRecord),
}

impl TrustAnchor {
    /// Get the zone name this anchor is for.
    pub fn zone(&self) -> &str {
        match self {
            Self::DnsKey(k) => &k.name,
            Self::Ds(d) => &d.name,
        }
    }

    /// Get the key tag (for matching against RRSIG or DS records).
    pub fn key_tag(&self) -> u16 {
        match self {
            Self::DnsKey(k) => k.key_tag(),
            Self::Ds(d) => d.key_tag,
        }
    }

    /// Get the algorithm.
    pub fn algorithm(&self) -> DnssecAlgorithm {
        match self {
            Self::DnsKey(k) => k.algorithm,
            Self::Ds(d) => d.algorithm,
        }
    }
}

impl fmt::Display for TrustAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::DnsKey(k) => write!(f, "TrustAnchor(DNSKEY {} tag={})", k.name, k.key_tag()),
            Self::Ds(d) => write!(f, "TrustAnchor(DS {} tag={})", d.name, d.key_tag),
        }
    }
}

// ── Trust anchor store ─────────────────────────────────────────────────────

/// A store of DNSSEC trust anchors.
///
/// Provides lookup by zone name to find the trust anchor(s) for
/// validating a particular zone's DNSKEY set.
#[derive(Debug, Clone, Default)]
pub struct TrustAnchorStore {
    /// Trust anchors indexed by lowercased zone name.
    anchors: HashMap<String, Vec<TrustAnchor>>,
}

impl TrustAnchorStore {
    /// Create an empty trust anchor store.
    pub fn new() -> Self {
        Self::default()
    }

    /// Create a store with the built-in root trust anchor.
    ///
    /// This uses the well-known root zone KSK key tag 20326
    /// (the 2017 root KSK, algorithm 8 RSA/SHA-256).
    ///
    /// In a production implementation, the actual public key bytes would
    /// be included.  Here we include the metadata only.
    pub fn with_root_anchor() -> Self {
        let mut store = Self::new();
        store.add(TrustAnchor::Ds(DsRecord {
            name: ".".to_string(),
            key_tag: 20326,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            // Root zone KSK DS digest (placeholder — real value is 64 hex chars / 32 bytes)
            digest: vec![
                0xE0, 0x6D, 0x44, 0xB8, 0x0B, 0x8F, 0x1D, 0x39, 0xA9, 0x5C, 0x0B, 0x0D, 0x7C, 0x65,
                0xD0, 0x84, 0x58, 0xE8, 0x80, 0x40, 0x9B, 0xBC, 0x68, 0x34, 0x57, 0x10, 0x42, 0x37,
                0xC7, 0xF8, 0xEC, 0x8D,
            ],
            ttl: 86400,
        }));
        store
    }

    /// Add a trust anchor.
    pub fn add(&mut self, anchor: TrustAnchor) {
        let zone = anchor.zone().trim_end_matches('.').to_lowercase();
        let zone = if zone.is_empty() {
            ".".to_string()
        } else {
            zone
        };
        self.anchors.entry(zone).or_default().push(anchor);
    }

    /// Look up trust anchors for a zone.
    pub fn get(&self, zone: &str) -> Option<&[TrustAnchor]> {
        let zone = zone.trim_end_matches('.').to_lowercase();
        let zone = if zone.is_empty() {
            ".".to_string()
        } else {
            zone
        };
        self.anchors.get(&zone).map(|v| v.as_slice())
    }

    /// Check if there is a trust anchor for the given zone.
    pub fn has_anchor(&self, zone: &str) -> bool {
        self.get(zone).is_some()
    }

    /// Find the closest enclosing trust anchor for a given name.
    ///
    /// Walks up the name hierarchy looking for a trust anchor.
    /// For example, for `host.example.com.` it checks:
    /// `host.example.com`, `example.com`, `com`, `.`
    pub fn find_closest(&self, name: &str) -> Option<(&str, &[TrustAnchor])> {
        let name = name.trim_end_matches('.').to_lowercase();

        // Check the name itself
        if let Some(_anchors) = self.get(&name) {
            // Need to return a reference with the right lifetime...
            let _key = if name.is_empty() { "." } else { &name };
            // Look up again to get the right reference
            let normalized = if name.is_empty() {
                ".".to_string()
            } else {
                name.clone()
            };
            if let Some((k, v)) = self.anchors.get_key_value(&normalized) {
                return Some((k.as_str(), v.as_slice()));
            }
        }

        // Walk up the hierarchy
        let mut search = name.as_str();
        loop {
            if let Some(dot_pos) = search.find('.') {
                search = &search[dot_pos + 1..];
                let normalized = if search.is_empty() {
                    ".".to_string()
                } else {
                    search.to_string()
                };
                if let Some((k, v)) = self.anchors.get_key_value(&normalized) {
                    return Some((k.as_str(), v.as_slice()));
                }
            } else {
                // Check root
                if let Some((k, v)) = self.anchors.get_key_value(".") {
                    return Some((k.as_str(), v.as_slice()));
                }
                return None;
            }
        }
    }

    /// Get the total number of trust anchors.
    pub fn count(&self) -> usize {
        self.anchors.values().map(|v| v.len()).sum()
    }

    /// Remove all trust anchors for a zone.
    pub fn remove(&mut self, zone: &str) {
        let zone = zone.trim_end_matches('.').to_lowercase();
        let zone = if zone.is_empty() {
            ".".to_string()
        } else {
            zone
        };
        self.anchors.remove(&zone);
    }

    /// Clear all trust anchors.
    pub fn clear(&mut self) {
        self.anchors.clear();
    }
}

// ── Validation result ──────────────────────────────────────────────────────

/// The result of DNSSEC validation for a DNS response.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ValidationResult {
    /// The response is fully validated and authentic (AD bit should be set).
    Secure,
    /// The zone is not signed (no DNSKEY/RRSIG/DS found), but this is
    /// legitimate — the parent zone does not have a DS record for it.
    Insecure,
    /// Validation failed — signatures are invalid, expired, or the chain
    /// of trust is broken.  The response should not be trusted.
    Bogus,
    /// Validation could not be performed (no trust anchor, algorithm not
    /// supported, etc.).  The response is returned as-is.
    Indeterminate,
}

impl ValidationResult {
    /// Whether the response can be trusted (Secure or Insecure).
    pub fn is_trusted(self) -> bool {
        matches!(self, Self::Secure | Self::Insecure)
    }

    /// Whether the AD (Authenticated Data) bit should be set.
    pub fn should_set_ad(self) -> bool {
        self == Self::Secure
    }
}

impl fmt::Display for ValidationResult {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Secure => write!(f, "SECURE"),
            Self::Insecure => write!(f, "INSECURE"),
            Self::Bogus => write!(f, "BOGUS"),
            Self::Indeterminate => write!(f, "INDETERMINATE"),
        }
    }
}

// ── Validation errors ──────────────────────────────────────────────────────

/// Detailed DNSSEC validation error.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    /// No trust anchor found for the zone.
    NoTrustAnchor,
    /// No DNSKEY records found in the zone.
    NoDnskeys,
    /// No RRSIG records found covering the answer.
    NoRrsigs,
    /// Signature has expired.
    SignatureExpired,
    /// Signature is not yet valid.
    SignatureNotYetValid,
    /// Signature algorithm not supported.
    UnsupportedAlgorithm(DnssecAlgorithm),
    /// Key tag mismatch between RRSIG and DNSKEY.
    KeyTagMismatch,
    /// Cryptographic signature verification failed.
    SignatureInvalid,
    /// DS digest mismatch (DNSKEY does not match parent DS).
    DsMismatch,
    /// DS digest type not supported.
    UnsupportedDigest(DigestType),
    /// NSEC/NSEC3 records do not properly prove denial of existence.
    DenialOfExistenceFailed,
    /// Validation depth exceeded.
    DepthExceeded,
    /// Chain of trust is broken.
    ChainBroken(String),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoTrustAnchor => write!(f, "no trust anchor found"),
            Self::NoDnskeys => write!(f, "no DNSKEY records in zone"),
            Self::NoRrsigs => write!(f, "no RRSIG records covering the answer"),
            Self::SignatureExpired => write!(f, "RRSIG signature has expired"),
            Self::SignatureNotYetValid => write!(f, "RRSIG signature not yet valid"),
            Self::UnsupportedAlgorithm(alg) => {
                write!(f, "unsupported DNSSEC algorithm: {}", alg)
            }
            Self::KeyTagMismatch => write!(f, "RRSIG key tag does not match any DNSKEY"),
            Self::SignatureInvalid => write!(f, "cryptographic signature verification failed"),
            Self::DsMismatch => write!(f, "DNSKEY does not match parent DS record"),
            Self::UnsupportedDigest(dt) => write!(f, "unsupported DS digest type: {}", dt),
            Self::DenialOfExistenceFailed => {
                write!(f, "NSEC/NSEC3 denial of existence proof failed")
            }
            Self::DepthExceeded => write!(f, "DNSSEC validation depth exceeded"),
            Self::ChainBroken(msg) => write!(f, "chain of trust broken: {}", msg),
        }
    }
}

// ── Validator ──────────────────────────────────────────────────────────────

/// DNSSEC validator.
///
/// Validates DNS responses against configured trust anchors.  This is a
/// stateless validator — each call to `validate_response()` performs a
/// full validation without caching intermediate results.
///
/// Note: actual cryptographic verification requires a crypto library
/// (e.g. ring, openssl).  This implementation validates the DNSSEC
/// record structure, time validity, key tag matching, and chain structure,
/// but defers actual signature verification to a pluggable backend.
pub struct DnssecValidator {
    /// Trust anchor store.
    trust_anchors: TrustAnchorStore,
    /// Negative trust anchors — zones where DNSSEC validation is skipped.
    negative_trust_anchors: Vec<String>,
}

impl DnssecValidator {
    /// Create a new validator with the given trust anchors.
    pub fn new(trust_anchors: TrustAnchorStore) -> Self {
        Self {
            trust_anchors,
            negative_trust_anchors: Vec::new(),
        }
    }

    /// Create a validator with the built-in root trust anchor.
    pub fn with_root_anchor() -> Self {
        Self::new(TrustAnchorStore::with_root_anchor())
    }

    /// Add a negative trust anchor (NTA) for a domain.
    ///
    /// Queries for names under this domain will skip DNSSEC validation
    /// and return `ValidationResult::Insecure`.
    pub fn add_negative_trust_anchor(&mut self, domain: &str) {
        let domain = domain.trim_end_matches('.').to_lowercase();
        if !domain.is_empty() && !self.negative_trust_anchors.contains(&domain) {
            self.negative_trust_anchors.push(domain);
        }
    }

    /// Remove a negative trust anchor.
    pub fn remove_negative_trust_anchor(&mut self, domain: &str) {
        let domain = domain.trim_end_matches('.').to_lowercase();
        self.negative_trust_anchors.retain(|d| d != &domain);
    }

    /// Check if a name is under a negative trust anchor.
    pub fn is_negative_trust_anchor(&self, name: &str) -> bool {
        let name = name.trim_end_matches('.').to_lowercase();
        for nta in &self.negative_trust_anchors {
            if name == *nta || name.ends_with(&format!(".{}", nta)) {
                return true;
            }
        }
        false
    }

    /// Get a reference to the trust anchor store.
    pub fn trust_anchors(&self) -> &TrustAnchorStore {
        &self.trust_anchors
    }

    /// Get a mutable reference to the trust anchor store.
    pub fn trust_anchors_mut(&mut self) -> &mut TrustAnchorStore {
        &mut self.trust_anchors
    }

    /// Validate the DNSSEC status of a response for a given query name.
    ///
    /// This performs structural validation:
    /// 1. Check for negative trust anchors (→ Insecure)
    /// 2. Find the closest trust anchor
    /// 3. Check for RRSIG records in the answer
    /// 4. Validate RRSIG time validity
    /// 5. Match RRSIG key tags against available DNSKEYs
    /// 6. (Crypto verification delegated to backend)
    ///
    /// Returns the validation result and any errors encountered.
    pub fn validate_rrset(
        &self,
        qname: &str,
        rrsigs: &[RrsigRecord],
        dnskeys: &[DnskeyRecord],
    ) -> (ValidationResult, Vec<ValidationError>) {
        let mut errors = Vec::new();

        // Check negative trust anchors
        if self.is_negative_trust_anchor(qname) {
            return (ValidationResult::Insecure, errors);
        }

        // Need at least one RRSIG
        if rrsigs.is_empty() {
            // Check if the zone has a trust anchor — if not, it's insecure
            if self.trust_anchors.find_closest(qname).is_none() {
                return (ValidationResult::Insecure, errors);
            }
            errors.push(ValidationError::NoRrsigs);
            return (ValidationResult::Bogus, errors);
        }

        // Need at least one DNSKEY
        if dnskeys.is_empty() {
            errors.push(ValidationError::NoDnskeys);
            return (ValidationResult::Bogus, errors);
        }

        let now = current_unix_time();

        // Validate each RRSIG
        let mut any_valid = false;
        for rrsig in rrsigs {
            // Check time validity
            if !rrsig.is_time_valid_at(now) {
                if now > rrsig.sig_expiration {
                    errors.push(ValidationError::SignatureExpired);
                } else {
                    errors.push(ValidationError::SignatureNotYetValid);
                }
                continue;
            }

            // Check algorithm support
            if !rrsig.algorithm.is_supported() {
                errors.push(ValidationError::UnsupportedAlgorithm(rrsig.algorithm));
                continue;
            }

            // Find matching DNSKEY by key tag and algorithm
            let matching_key = dnskeys.iter().find(|k| {
                k.key_tag() == rrsig.key_tag && k.algorithm == rrsig.algorithm && k.is_zone_key()
            });

            if matching_key.is_none() {
                errors.push(ValidationError::KeyTagMismatch);
                continue;
            }

            // At this point, structural validation passes.
            // Actual signature verification would happen here with a crypto library.
            // For now, we mark it as structurally valid.
            any_valid = true;
        }

        if any_valid {
            // Structural validation passed; with a crypto backend this
            // would be Secure.  Without one, we report Indeterminate.
            (ValidationResult::Indeterminate, errors)
        } else if errors.is_empty() {
            (ValidationResult::Indeterminate, errors)
        } else {
            (ValidationResult::Bogus, errors)
        }
    }
}

// ── Helper functions ───────────────────────────────────────────────────────

/// Compute the key tag for a DNSKEY RDATA (RFC 4034 Appendix B).
///
/// This is a simple checksum over the RDATA used to quickly identify
/// which DNSKEY an RRSIG refers to.
pub fn compute_key_tag(rdata: &[u8]) -> u16 {
    let mut ac: u32 = 0;
    for (i, &byte) in rdata.iter().enumerate() {
        if i & 1 == 0 {
            ac += (byte as u32) << 8;
        } else {
            ac += byte as u32;
        }
    }
    ac += (ac >> 16) & 0xFFFF;
    (ac & 0xFFFF) as u16
}

/// Encode a domain name in canonical DNS wire format (RFC 4034 §6.1).
///
/// Canonical wire format: all labels lowercased, no compression,
/// terminated with a root label (zero byte).
pub fn canonical_name_wire(name: &str) -> Option<Vec<u8>> {
    let name = name.trim_end_matches('.');

    if name.is_empty() || name == "." {
        return Some(vec![0]);
    }

    let mut buf = Vec::with_capacity(name.len() + 2);
    for label in name.split('.') {
        let lower = label.to_lowercase();
        let len = lower.len();
        if len == 0 || len > 63 {
            return None;
        }
        buf.push(len as u8);
        buf.extend_from_slice(lower.as_bytes());
    }
    buf.push(0); // root label

    Some(buf)
}

/// Count the number of labels in a domain name.
///
/// The root "." has 0 labels.
/// "example.com" has 2 labels.
/// "www.example.com" has 3 labels.
pub fn count_labels(name: &str) -> u8 {
    let name = name.trim_end_matches('.');
    if name.is_empty() {
        return 0;
    }
    name.split('.').count() as u8
}

/// Compare two domain names in canonical DNS order (RFC 4034 §6.1).
///
/// Names are compared label-by-label from the rightmost (root) label
/// to the leftmost, with case-insensitive comparison.
pub fn canonical_name_order(a: &str, b: &str) -> std::cmp::Ordering {
    let a = a.trim_end_matches('.').to_lowercase();
    let b = b.trim_end_matches('.').to_lowercase();

    // Split into labels and reverse for right-to-left comparison
    let a_labels: Vec<&str> = a.split('.').rev().collect();
    let b_labels: Vec<&str> = b.split('.').rev().collect();

    let min_len = a_labels.len().min(b_labels.len());

    for i in 0..min_len {
        match a_labels[i].cmp(b_labels[i]) {
            std::cmp::Ordering::Equal => continue,
            other => return other,
        }
    }

    a_labels.len().cmp(&b_labels.len())
}

/// Parse a DNS name from uncompressed wire format (no pointers allowed).
///
/// This is stricter than the general `parse_name()` from the dns module
/// because DNSSEC record RDATA must not use compression.
fn parse_name_uncompressed(data: &[u8], start: usize) -> Option<(String, usize)> {
    let mut name = String::with_capacity(64);
    let mut offset = start;

    loop {
        if offset >= data.len() {
            return None;
        }

        let len = data[offset] as usize;

        if len == 0 {
            offset += 1;
            break;
        }

        // Compression pointers not allowed in DNSSEC RDATA
        if (len & 0xC0) != 0 {
            return None;
        }

        if len > 63 {
            return None;
        }

        offset += 1;
        if offset + len > data.len() {
            return None;
        }

        if !name.is_empty() {
            name.push('.');
        }

        for &b in &data[offset..offset + len] {
            name.push(b as char);
        }

        offset += len;

        if name.len() > 255 {
            return None;
        }
    }

    if name.is_empty() {
        name.push('.');
    }

    Some((name, offset))
}

/// Parse NSEC/NSEC3 type bit maps (RFC 4034 §4.1.2).
///
/// Type bit maps consist of one or more blocks, each with:
/// ```text
///   Window Block Number(1) | Bitmap Length(1) | Bitmap(variable)
/// ```
fn parse_type_bit_maps(data: &[u8]) -> Option<Vec<u16>> {
    let mut types = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        if offset + 2 > data.len() {
            break;
        }

        let window = data[offset] as u16;
        let bitmap_len = data[offset + 1] as usize;
        offset += 2;

        if bitmap_len == 0 || bitmap_len > 32 || offset + bitmap_len > data.len() {
            break;
        }

        for (byte_idx, &byte) in data[offset..offset + bitmap_len].iter().enumerate() {
            for bit in 0..8u16 {
                if (byte & (0x80 >> bit)) != 0 {
                    let rr_type = window * 256 + (byte_idx as u16) * 8 + bit;
                    types.push(rr_type);
                }
            }
        }

        offset += bitmap_len;
    }

    Some(types)
}

/// Encode a set of RR types into NSEC/NSEC3 type bit map format.
pub fn encode_type_bit_maps(types: &[u16]) -> Vec<u8> {
    if types.is_empty() {
        return Vec::new();
    }

    // Group types by window
    let mut windows: HashMap<u8, Vec<u16>> = HashMap::new();
    for &rr_type in types {
        let window = (rr_type / 256) as u8;
        windows.entry(window).or_default().push(rr_type);
    }

    let mut result = Vec::new();

    // Sort windows
    let mut sorted_windows: Vec<u8> = windows.keys().copied().collect();
    sorted_windows.sort();

    for window in sorted_windows {
        let window_types = &windows[&window];

        // Find the highest offset within this window
        let max_offset = window_types
            .iter()
            .map(|t| (t % 256) as usize)
            .max()
            .unwrap_or(0);

        let bitmap_len = (max_offset / 8) + 1;
        let mut bitmap = vec![0u8; bitmap_len];

        for &rr_type in window_types {
            let offset_in_window = (rr_type % 256) as usize;
            let byte_idx = offset_in_window / 8;
            let bit_idx = offset_in_window % 8;
            bitmap[byte_idx] |= 0x80 >> bit_idx;
        }

        result.push(window);
        result.push(bitmap_len as u8);
        result.extend_from_slice(&bitmap);
    }

    result
}

/// Get the current time as seconds since Unix epoch.
fn current_unix_time() -> u32 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_secs() as u32
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── DnssecAlgorithm tests ──────────────────────────────────────────

    #[test]
    fn test_algorithm_roundtrip() {
        let algs = [0, 1, 2, 3, 5, 6, 7, 8, 10, 13, 14, 15, 16, 252, 253, 254];
        for n in algs {
            let alg = DnssecAlgorithm::from_u8(n);
            assert_eq!(alg.to_u8(), n);
        }
    }

    #[test]
    fn fuzz_dnssec_rdata_parsers_never_panic() {
        // Robustness net (task #22): DNSSEC rdata parsers (DNSKEY/RRSIG/DS/NSEC/
        // NSEC3, the type-bitmap and uncompressed-name helpers) parse
        // attacker-controlled RR data from DNS responses -- a historically buggy
        // offset/length surface. None must panic or hang on malformed rdata.
        let handle = std::thread::spawn(|| {
            let mut state: u64 = 0xd0ec_5ec5_1234_abcd;
            let mut next = || {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u32
            };
            for _ in 0..40_000u32 {
                let len = (next() % 256) as usize;
                let rdata: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
                let start = if rdata.is_empty() {
                    0
                } else {
                    (next() as usize) % (rdata.len() + 8)
                };
                let input = rdata.clone();
                let res = std::panic::catch_unwind(move || {
                    let _ = DnskeyRecord::parse("example.com", 3600, &input);
                    let _ = RrsigRecord::parse("example.com", 3600, &input);
                    let _ = DsRecord::parse("example.com", 3600, &input);
                    let _ = NsecRecord::parse("example.com", 3600, &input);
                    let _ = Nsec3Record::parse("example.com", 3600, &input);
                    let _ = parse_type_bit_maps(&input);
                    let _ = parse_name_uncompressed(&input, start);
                });
                assert!(
                    res.is_ok(),
                    "dnssec parser panicked: rdata={rdata:?} start={start}"
                );
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "dnssec fuzz did not finish in 30s -- a malformed rdata hangs a parser"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        handle.join().expect("fuzz worker panicked");
    }

    #[test]
    fn test_algorithm_unknown() {
        let alg = DnssecAlgorithm::from_u8(99);
        assert_eq!(alg, DnssecAlgorithm::Unknown(99));
        assert_eq!(alg.to_u8(), 99);
    }

    #[test]
    fn test_algorithm_is_supported() {
        assert!(DnssecAlgorithm::RsaSha256.is_supported());
        assert!(DnssecAlgorithm::RsaSha512.is_supported());
        assert!(DnssecAlgorithm::EcdsaP256Sha256.is_supported());
        assert!(DnssecAlgorithm::EcdsaP384Sha384.is_supported());
        assert!(DnssecAlgorithm::Ed25519.is_supported());
        assert!(DnssecAlgorithm::Ed448.is_supported());
        assert!(!DnssecAlgorithm::RsaMd5.is_supported());
        assert!(!DnssecAlgorithm::RsaSha1.is_supported());
        assert!(!DnssecAlgorithm::Unknown(99).is_supported());
    }

    #[test]
    fn test_algorithm_is_deprecated() {
        assert!(DnssecAlgorithm::RsaMd5.is_deprecated());
        assert!(DnssecAlgorithm::DsaSha1.is_deprecated());
        assert!(DnssecAlgorithm::DsaNsec3Sha1.is_deprecated());
        assert!(!DnssecAlgorithm::RsaSha256.is_deprecated());
        assert!(!DnssecAlgorithm::Ed25519.is_deprecated());
    }

    #[test]
    fn test_algorithm_mnemonic() {
        assert_eq!(DnssecAlgorithm::RsaSha256.mnemonic(), "RSASHA256");
        assert_eq!(DnssecAlgorithm::Ed25519.mnemonic(), "ED25519");
        assert_eq!(DnssecAlgorithm::Unknown(99).mnemonic(), "UNKNOWN");
    }

    #[test]
    fn test_algorithm_display() {
        assert_eq!(format!("{}", DnssecAlgorithm::RsaSha256), "RSASHA256(8)");
        assert_eq!(format!("{}", DnssecAlgorithm::Ed25519), "ED25519(15)");
    }

    // ── DigestType tests ───────────────────────────────────────────────

    #[test]
    fn test_digest_type_roundtrip() {
        for n in [1, 2, 3, 4] {
            let dt = DigestType::from_u8(n);
            assert_eq!(dt.to_u8(), n);
        }
    }

    #[test]
    fn test_digest_type_unknown() {
        let dt = DigestType::from_u8(99);
        assert_eq!(dt, DigestType::Unknown(99));
    }

    #[test]
    fn test_digest_type_is_supported() {
        assert!(!DigestType::Sha1.is_supported());
        assert!(DigestType::Sha256.is_supported());
        assert!(!DigestType::GostR34_11_94.is_supported());
        assert!(DigestType::Sha384.is_supported());
    }

    #[test]
    fn test_digest_type_length() {
        assert_eq!(DigestType::Sha1.digest_length(), Some(20));
        assert_eq!(DigestType::Sha256.digest_length(), Some(32));
        assert_eq!(DigestType::Sha384.digest_length(), Some(48));
        assert_eq!(DigestType::Unknown(99).digest_length(), None);
    }

    #[test]
    fn test_digest_type_display() {
        assert_eq!(format!("{}", DigestType::Sha256), "SHA-256(2)");
    }

    // ── Nsec3HashAlgorithm tests ───────────────────────────────────────

    #[test]
    fn test_nsec3_hash_roundtrip() {
        assert_eq!(Nsec3HashAlgorithm::from_u8(1), Nsec3HashAlgorithm::Sha1);
        assert_eq!(Nsec3HashAlgorithm::Sha1.to_u8(), 1);
    }

    #[test]
    fn test_nsec3_hash_display() {
        assert_eq!(format!("{}", Nsec3HashAlgorithm::Sha1), "SHA-1(1)");
        assert_eq!(format!("{}", Nsec3HashAlgorithm::Unknown(5)), "UNKNOWN(5)");
    }

    // ── DNSKEY parsing tests ───────────────────────────────────────────

    #[test]
    fn test_dnskey_parse_basic() {
        // flags=257 (ZSK+SEP), protocol=3, algorithm=8 (RSA/SHA-256)
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&257u16.to_be_bytes());
        rdata.push(3); // protocol
        rdata.push(8); // algorithm
        rdata.extend_from_slice(&[0xAA, 0xBB, 0xCC]); // public key

        let key = DnskeyRecord::parse("example.com", 3600, &rdata).unwrap();
        assert_eq!(key.name, "example.com");
        assert_eq!(key.flags, 257);
        assert_eq!(key.protocol, 3);
        assert_eq!(key.algorithm, DnssecAlgorithm::RsaSha256);
        assert_eq!(key.public_key, vec![0xAA, 0xBB, 0xCC]);
        assert!(key.is_zone_key());
        assert!(key.is_sep());
        assert!(!key.is_revoked());
    }

    #[test]
    fn test_dnskey_parse_too_short() {
        assert!(DnskeyRecord::parse(".", 3600, &[0, 1, 3]).is_none());
    }

    #[test]
    fn test_dnskey_parse_zsk_only() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&256u16.to_be_bytes()); // ZSK only (no SEP)
        rdata.push(3);
        rdata.push(13); // ECDSA P-256
        rdata.extend_from_slice(&[0x01, 0x02]);

        let key = DnskeyRecord::parse(".", 86400, &rdata).unwrap();
        assert!(key.is_zone_key());
        assert!(!key.is_sep());
    }

    #[test]
    fn test_dnskey_revoked() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(
            &(DNSKEY_FLAG_ZONE_KEY | DNSKEY_FLAG_SEP | DNSKEY_FLAG_REVOKE).to_be_bytes(),
        );
        rdata.push(3);
        rdata.push(8);
        rdata.extend_from_slice(&[1, 2, 3]);

        let key = DnskeyRecord::parse("example.com", 3600, &rdata).unwrap();
        assert!(key.is_revoked());
    }

    #[test]
    fn test_dnskey_key_tag() {
        // Simple test: key tag is a checksum of the RDATA
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&257u16.to_be_bytes());
        rdata.push(3);
        rdata.push(8);
        rdata.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);

        let key = DnskeyRecord::parse(".", 3600, &rdata).unwrap();
        let tag = key.key_tag();
        // Key tag should be deterministic
        assert_eq!(tag, key.key_tag());
        // And non-zero for non-trivial keys
        assert_ne!(tag, 0);
    }

    #[test]
    fn test_dnskey_to_rdata() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&257u16.to_be_bytes());
        rdata.push(3);
        rdata.push(8);
        rdata.extend_from_slice(&[0xAA, 0xBB]);

        let key = DnskeyRecord::parse(".", 3600, &rdata).unwrap();
        assert_eq!(key.to_rdata(), rdata);
    }

    #[test]
    fn test_dnskey_display() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&257u16.to_be_bytes());
        rdata.push(3);
        rdata.push(8);
        rdata.extend_from_slice(&[1]);

        let key = DnskeyRecord::parse("example.com", 3600, &rdata).unwrap();
        let s = format!("{}", key);
        assert!(s.contains("example.com"));
        assert!(s.contains("DNSKEY"));
        assert!(s.contains("SEP"));
    }

    // ── RRSIG parsing tests ────────────────────────────────────────────

    #[test]
    fn test_rrsig_parse_basic() {
        let mut rdata = Vec::new();
        // Type covered: A (1)
        rdata.extend_from_slice(&1u16.to_be_bytes());
        // Algorithm: RSA/SHA-256 (8)
        rdata.push(8);
        // Labels: 2
        rdata.push(2);
        // Original TTL: 3600
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        // Sig expiration: some future time
        rdata.extend_from_slice(&2000000000u32.to_be_bytes());
        // Sig inception: some past time
        rdata.extend_from_slice(&1700000000u32.to_be_bytes());
        // Key tag: 12345
        rdata.extend_from_slice(&12345u16.to_be_bytes());
        // Signer's name: "example.com" (uncompressed)
        rdata.push(7);
        rdata.extend_from_slice(b"example");
        rdata.push(3);
        rdata.extend_from_slice(b"com");
        rdata.push(0);
        // Signature: some bytes
        rdata.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);

        let sig = RrsigRecord::parse("www.example.com", 3600, &rdata).unwrap();
        assert_eq!(sig.type_covered, 1);
        assert_eq!(sig.algorithm, DnssecAlgorithm::RsaSha256);
        assert_eq!(sig.labels, 2);
        assert_eq!(sig.original_ttl, 3600);
        assert_eq!(sig.key_tag, 12345);
        assert_eq!(sig.signer_name, "example.com");
        assert_eq!(sig.signature, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_rrsig_parse_too_short() {
        assert!(RrsigRecord::parse(".", 3600, &[0; 10]).is_none());
    }

    #[test]
    fn test_rrsig_time_valid() {
        let now = current_unix_time();
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(8);
        rdata.push(2);
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        // Expiration: now + 1 hour
        rdata.extend_from_slice(&(now + 3600).to_be_bytes());
        // Inception: now - 1 hour
        rdata.extend_from_slice(&(now - 3600).to_be_bytes());
        rdata.extend_from_slice(&12345u16.to_be_bytes());
        rdata.push(0); // root signer name
        rdata.extend_from_slice(&[0xFF]);

        let sig = RrsigRecord::parse("test.", 3600, &rdata).unwrap();
        assert!(sig.is_time_valid_at(now));
    }

    #[test]
    fn test_rrsig_time_expired() {
        let now = current_unix_time();
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(8);
        rdata.push(2);
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        // Expiration: 1 hour ago (past the jitter window)
        rdata.extend_from_slice(&(now - 3600).to_be_bytes());
        rdata.extend_from_slice(&(now - 7200).to_be_bytes());
        rdata.extend_from_slice(&12345u16.to_be_bytes());
        rdata.push(0);
        rdata.extend_from_slice(&[0xFF]);

        let sig = RrsigRecord::parse("test.", 3600, &rdata).unwrap();
        assert!(!sig.is_time_valid_at(now));
    }

    #[test]
    fn test_rrsig_covers() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes()); // covers A
        rdata.push(8);
        rdata.push(2);
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        rdata.extend_from_slice(&2000000000u32.to_be_bytes());
        rdata.extend_from_slice(&1700000000u32.to_be_bytes());
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(0);
        rdata.extend_from_slice(&[0]);

        let sig = RrsigRecord::parse("test.", 3600, &rdata).unwrap();
        assert!(sig.covers(1)); // A
        assert!(!sig.covers(28)); // AAAA
    }

    #[test]
    fn test_rrsig_wildcard() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(8);
        rdata.push(1); // Labels = 1, but owner has 3 labels → wildcard
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        rdata.extend_from_slice(&2000000000u32.to_be_bytes());
        rdata.extend_from_slice(&1700000000u32.to_be_bytes());
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(0);

        let sig = RrsigRecord::parse("a.b.example.com", 3600, &rdata).unwrap();
        assert!(sig.is_wildcard());
    }

    #[test]
    fn test_rrsig_not_wildcard() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(8);
        rdata.push(2); // Labels = 2, matches "example.com"
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        rdata.extend_from_slice(&2000000000u32.to_be_bytes());
        rdata.extend_from_slice(&1700000000u32.to_be_bytes());
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(0);

        let sig = RrsigRecord::parse("example.com", 3600, &rdata).unwrap();
        assert!(!sig.is_wildcard());
    }

    #[test]
    fn test_rrsig_sig_data_prefix() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(8);
        rdata.push(2);
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        rdata.extend_from_slice(&2000000000u32.to_be_bytes());
        rdata.extend_from_slice(&1700000000u32.to_be_bytes());
        rdata.extend_from_slice(&12345u16.to_be_bytes());
        rdata.push(0); // root signer
        rdata.extend_from_slice(&[0xAB, 0xCD]);

        let sig = RrsigRecord::parse("example.com", 3600, &rdata).unwrap();
        let prefix = sig.sig_data_prefix().unwrap();
        // Should be 18 fixed bytes + 1 byte root name = 19 bytes
        assert_eq!(prefix.len(), 19);
    }

    #[test]
    fn test_rrsig_display() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(8);
        rdata.push(2);
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        rdata.extend_from_slice(&2000000000u32.to_be_bytes());
        rdata.extend_from_slice(&1700000000u32.to_be_bytes());
        rdata.extend_from_slice(&12345u16.to_be_bytes());
        rdata.push(0);

        let sig = RrsigRecord::parse("example.com", 3600, &rdata).unwrap();
        let s = format!("{}", sig);
        assert!(s.contains("RRSIG"));
        assert!(s.contains("12345"));
    }

    // ── DS parsing tests ───────────────────────────────────────────────

    #[test]
    fn test_ds_parse_basic() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&20326u16.to_be_bytes()); // key tag
        rdata.push(8); // algorithm
        rdata.push(2); // digest type SHA-256
        rdata.extend_from_slice(&[0xAA; 32]); // digest

        let ds = DsRecord::parse("example.com", 86400, &rdata).unwrap();
        assert_eq!(ds.key_tag, 20326);
        assert_eq!(ds.algorithm, DnssecAlgorithm::RsaSha256);
        assert_eq!(ds.digest_type, DigestType::Sha256);
        assert_eq!(ds.digest.len(), 32);
    }

    #[test]
    fn test_ds_parse_too_short() {
        assert!(DsRecord::parse(".", 3600, &[0, 1, 8, 2]).is_none());
    }

    #[test]
    fn test_ds_to_rdata() {
        let ds = DsRecord {
            name: "example.com".to_string(),
            key_tag: 12345,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0xBB; 32],
            ttl: 3600,
        };
        let rdata = ds.to_rdata();
        assert_eq!(rdata.len(), 4 + 32);
        assert_eq!(u16::from_be_bytes([rdata[0], rdata[1]]), 12345);
    }

    #[test]
    fn test_ds_display() {
        let ds = DsRecord {
            name: "example.com".to_string(),
            key_tag: 20326,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 3600,
        };
        let s = format!("{}", ds);
        assert!(s.contains("DS"));
        assert!(s.contains("20326"));
    }

    #[test]
    fn test_ds_matches_dnskey_basic_checks() {
        let mut dnskey_rdata = Vec::new();
        dnskey_rdata.extend_from_slice(&257u16.to_be_bytes());
        dnskey_rdata.push(3);
        dnskey_rdata.push(8); // RSA/SHA-256
        dnskey_rdata.extend_from_slice(&[1, 2, 3, 4]);
        let dnskey = DnskeyRecord::parse("example.com", 3600, &dnskey_rdata).unwrap();

        let ds = DsRecord {
            name: "example.com".to_string(),
            key_tag: dnskey.key_tag(),
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 3600,
        };

        // Should return None (can't verify without crypto) because structure matches
        assert_eq!(ds.matches_dnskey(&dnskey), None);
    }

    #[test]
    fn test_ds_matches_dnskey_wrong_algorithm() {
        let mut dnskey_rdata = Vec::new();
        dnskey_rdata.extend_from_slice(&257u16.to_be_bytes());
        dnskey_rdata.push(3);
        dnskey_rdata.push(13); // ECDSA
        dnskey_rdata.extend_from_slice(&[1]);
        let dnskey = DnskeyRecord::parse("example.com", 3600, &dnskey_rdata).unwrap();

        let ds = DsRecord {
            name: "example.com".to_string(),
            key_tag: dnskey.key_tag(),
            algorithm: DnssecAlgorithm::RsaSha256, // Different!
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 3600,
        };

        assert_eq!(ds.matches_dnskey(&dnskey), Some(false));
    }

    #[test]
    fn test_ds_matches_dnskey_wrong_digest_length() {
        let mut dnskey_rdata = Vec::new();
        dnskey_rdata.extend_from_slice(&257u16.to_be_bytes());
        dnskey_rdata.push(3);
        dnskey_rdata.push(8);
        dnskey_rdata.extend_from_slice(&[1, 2, 3, 4]);
        let dnskey = DnskeyRecord::parse("example.com", 3600, &dnskey_rdata).unwrap();

        let ds = DsRecord {
            name: "example.com".to_string(),
            key_tag: dnskey.key_tag(),
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 16], // Wrong length for SHA-256
            ttl: 3600,
        };

        assert_eq!(ds.matches_dnskey(&dnskey), Some(false));
    }

    // ── NSEC parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_nsec_parse_basic() {
        let mut rdata = Vec::new();
        // Next domain name: "beta.example.com"
        rdata.push(4);
        rdata.extend_from_slice(b"beta");
        rdata.push(7);
        rdata.extend_from_slice(b"example");
        rdata.push(3);
        rdata.extend_from_slice(b"com");
        rdata.push(0);

        // Type bit map: window 0, bitmap includes A(1) and AAAA(28) and RRSIG(46)
        rdata.push(0); // window 0
        rdata.push(7); // bitmap length 7 (covers up to type 55)
        let mut bitmap = [0u8; 7];
        bitmap[0] |= 0x40; // type 1 (A): byte 0, bit 1
        bitmap[3] |= 0x08; // type 28 (AAAA): byte 3, bit 4
        bitmap[5] |= 0x02; // type 46 (RRSIG): byte 5, bit 6
        rdata.extend_from_slice(&bitmap);

        let nsec = NsecRecord::parse("alpha.example.com", 3600, &rdata).unwrap();
        assert_eq!(nsec.name, "alpha.example.com");
        assert_eq!(nsec.next_name, "beta.example.com");
        assert!(nsec.has_type(1)); // A
        assert!(nsec.has_type(28)); // AAAA
        assert!(nsec.has_type(46)); // RRSIG
        assert!(!nsec.has_type(15)); // MX
    }

    #[test]
    fn test_nsec_parse_empty() {
        assert!(NsecRecord::parse(".", 3600, &[]).is_none());
    }

    #[test]
    fn test_nsec_denies_name() {
        let nsec = NsecRecord {
            name: "alpha.example.com".to_string(),
            next_name: "gamma.example.com".to_string(),
            types: vec![1, 28, 46],
            ttl: 3600,
        };

        // "beta" falls between "alpha" and "gamma"
        assert!(nsec.denies_name("beta.example.com"));
        // "alpha" is the owner, not denied
        assert!(!nsec.denies_name("alpha.example.com"));
        // "zzz" is after "gamma", not in range
        assert!(!nsec.denies_name("zzz.example.com"));
    }

    #[test]
    fn test_nsec_denies_type() {
        let nsec = NsecRecord {
            name: "host.example.com".to_string(),
            next_name: "other.example.com".to_string(),
            types: vec![1, 28], // A and AAAA
            ttl: 3600,
        };

        // MX type not in bitmap → denied
        assert!(nsec.denies_type("host.example.com", 15));
        // A type in bitmap → not denied
        assert!(!nsec.denies_type("host.example.com", 1));
        // Wrong name → not denied by this NSEC
        assert!(!nsec.denies_type("other.example.com", 15));
    }

    #[test]
    fn test_nsec_display() {
        let nsec = NsecRecord {
            name: "a.example.com".to_string(),
            next_name: "b.example.com".to_string(),
            types: vec![1, 28],
            ttl: 3600,
        };
        let s = format!("{}", nsec);
        assert!(s.contains("NSEC"));
        assert!(s.contains("b.example.com"));
    }

    // ── NSEC3 parsing tests ────────────────────────────────────────────

    #[test]
    fn test_nsec3_parse_basic() {
        let mut rdata = Vec::new();
        rdata.push(1); // hash algorithm SHA-1
        rdata.push(0); // flags
        rdata.extend_from_slice(&10u16.to_be_bytes()); // iterations
        rdata.push(4); // salt length
        rdata.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD]); // salt
        rdata.push(20); // hash length (SHA-1 = 20)
        rdata.extend_from_slice(&[0x11; 20]); // next hashed owner

        // Type bitmap: A(1) and AAAA(28)
        rdata.push(0); // window 0
        rdata.push(4); // bitmap length 4
        let mut bitmap = [0u8; 4];
        bitmap[0] |= 0x40; // type 1 (A)
        bitmap[3] |= 0x08; // type 28 (AAAA)
        rdata.extend_from_slice(&bitmap);

        let nsec3 = Nsec3Record::parse("hash.example.com", 3600, &rdata).unwrap();
        assert_eq!(nsec3.hash_algorithm, Nsec3HashAlgorithm::Sha1);
        assert_eq!(nsec3.flags, 0);
        assert_eq!(nsec3.iterations, 10);
        assert_eq!(nsec3.salt, vec![0xAA, 0xBB, 0xCC, 0xDD]);
        assert_eq!(nsec3.next_hashed_owner.len(), 20);
        assert!(nsec3.has_type(1));
        assert!(nsec3.has_type(28));
        assert!(!nsec3.is_opt_out());
    }

    #[test]
    fn test_nsec3_opt_out() {
        let mut rdata = Vec::new();
        rdata.push(1); // hash algorithm
        rdata.push(1); // flags: opt-out
        rdata.extend_from_slice(&0u16.to_be_bytes()); // iterations
        rdata.push(0); // no salt
        rdata.push(1); // hash length
        rdata.push(0xFF); // dummy hash

        let nsec3 = Nsec3Record::parse("hash.example.com", 3600, &rdata).unwrap();
        assert!(nsec3.is_opt_out());
    }

    #[test]
    fn test_nsec3_parse_too_short() {
        assert!(Nsec3Record::parse(".", 3600, &[1, 0, 0]).is_none());
    }

    #[test]
    fn test_nsec3_display() {
        let nsec3 = Nsec3Record {
            name: "hash.example.com".to_string(),
            hash_algorithm: Nsec3HashAlgorithm::Sha1,
            flags: 1,
            iterations: 10,
            salt: vec![0xAA],
            next_hashed_owner: vec![0xBB; 20],
            types: vec![1, 28, 46],
            ttl: 3600,
        };
        let s = format!("{}", nsec3);
        assert!(s.contains("NSEC3"));
        assert!(s.contains("opt-out"));
    }

    // ── TrustAnchor tests ──────────────────────────────────────────────

    #[test]
    fn test_trust_anchor_dnskey() {
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&257u16.to_be_bytes());
        rdata.push(3);
        rdata.push(8);
        rdata.extend_from_slice(&[1, 2, 3]);
        let key = DnskeyRecord::parse(".", 86400, &rdata).unwrap();

        let anchor = TrustAnchor::DnsKey(key.clone());
        assert_eq!(anchor.zone(), ".");
        assert_eq!(anchor.key_tag(), key.key_tag());
        assert_eq!(anchor.algorithm(), DnssecAlgorithm::RsaSha256);
    }

    #[test]
    fn test_trust_anchor_ds() {
        let ds = DsRecord {
            name: "example.com".to_string(),
            key_tag: 12345,
            algorithm: DnssecAlgorithm::EcdsaP256Sha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 3600,
        };

        let anchor = TrustAnchor::Ds(ds);
        assert_eq!(anchor.zone(), "example.com");
        assert_eq!(anchor.key_tag(), 12345);
    }

    #[test]
    fn test_trust_anchor_display() {
        let ds = DsRecord {
            name: ".".to_string(),
            key_tag: 20326,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 86400,
        };
        let s = format!("{}", TrustAnchor::Ds(ds));
        assert!(s.contains("20326"));
    }

    // ── TrustAnchorStore tests ─────────────────────────────────────────

    #[test]
    fn test_store_new_empty() {
        let store = TrustAnchorStore::new();
        assert_eq!(store.count(), 0);
        assert!(!store.has_anchor("."));
    }

    #[test]
    fn test_store_with_root_anchor() {
        let store = TrustAnchorStore::with_root_anchor();
        assert!(store.has_anchor("."));
        assert_eq!(store.count(), 1);
    }

    #[test]
    fn test_store_add_and_get() {
        let mut store = TrustAnchorStore::new();
        let ds = DsRecord {
            name: "example.com".to_string(),
            key_tag: 12345,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 3600,
        };
        store.add(TrustAnchor::Ds(ds));

        assert!(store.has_anchor("example.com"));
        assert!(!store.has_anchor("other.com"));
        assert_eq!(store.get("example.com").unwrap().len(), 1);
    }

    #[test]
    fn test_store_find_closest() {
        let mut store = TrustAnchorStore::new();
        let ds = DsRecord {
            name: ".".to_string(),
            key_tag: 20326,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 86400,
        };
        store.add(TrustAnchor::Ds(ds));

        // Any name should find the root anchor
        let result = store.find_closest("host.example.com");
        assert!(result.is_some());
        let (zone, anchors) = result.unwrap();
        assert_eq!(zone, ".");
        assert_eq!(anchors.len(), 1);
    }

    #[test]
    fn test_store_find_closest_specific() {
        let mut store = TrustAnchorStore::new();
        // Add root and a specific zone
        store.add(TrustAnchor::Ds(DsRecord {
            name: ".".to_string(),
            key_tag: 1,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 86400,
        }));
        store.add(TrustAnchor::Ds(DsRecord {
            name: "example.com".to_string(),
            key_tag: 2,
            algorithm: DnssecAlgorithm::RsaSha256,
            digest_type: DigestType::Sha256,
            digest: vec![0; 32],
            ttl: 3600,
        }));

        // Should find the more specific anchor
        let (zone, _) = store.find_closest("host.example.com").unwrap();
        assert_eq!(zone, "example.com");

        // Unrelated name should find root
        let (zone, _) = store.find_closest("host.other.org").unwrap();
        assert_eq!(zone, ".");
    }

    #[test]
    fn test_store_find_closest_no_anchor() {
        let store = TrustAnchorStore::new();
        assert!(store.find_closest("example.com").is_none());
    }

    #[test]
    fn test_store_remove() {
        let mut store = TrustAnchorStore::with_root_anchor();
        assert!(store.has_anchor("."));
        store.remove(".");
        assert!(!store.has_anchor("."));
    }

    #[test]
    fn test_store_clear() {
        let mut store = TrustAnchorStore::with_root_anchor();
        store.clear();
        assert_eq!(store.count(), 0);
    }

    // ── ValidationResult tests ─────────────────────────────────────────

    #[test]
    fn test_validation_result_trusted() {
        assert!(ValidationResult::Secure.is_trusted());
        assert!(ValidationResult::Insecure.is_trusted());
        assert!(!ValidationResult::Bogus.is_trusted());
        assert!(!ValidationResult::Indeterminate.is_trusted());
    }

    #[test]
    fn test_validation_result_ad_bit() {
        assert!(ValidationResult::Secure.should_set_ad());
        assert!(!ValidationResult::Insecure.should_set_ad());
        assert!(!ValidationResult::Bogus.should_set_ad());
    }

    #[test]
    fn test_validation_result_display() {
        assert_eq!(format!("{}", ValidationResult::Secure), "SECURE");
        assert_eq!(format!("{}", ValidationResult::Insecure), "INSECURE");
        assert_eq!(format!("{}", ValidationResult::Bogus), "BOGUS");
        assert_eq!(
            format!("{}", ValidationResult::Indeterminate),
            "INDETERMINATE"
        );
    }

    // ── ValidationError display tests ──────────────────────────────────

    #[test]
    fn test_validation_error_display() {
        assert!(format!("{}", ValidationError::NoTrustAnchor).contains("trust anchor"));
        assert!(format!("{}", ValidationError::NoDnskeys).contains("DNSKEY"));
        assert!(format!("{}", ValidationError::NoRrsigs).contains("RRSIG"));
        assert!(format!("{}", ValidationError::SignatureExpired).contains("expired"));
        assert!(format!("{}", ValidationError::SignatureNotYetValid).contains("not yet"));
        assert!(
            format!(
                "{}",
                ValidationError::UnsupportedAlgorithm(DnssecAlgorithm::RsaMd5)
            )
            .contains("unsupported")
        );
        assert!(format!("{}", ValidationError::KeyTagMismatch).contains("key tag"));
        assert!(format!("{}", ValidationError::SignatureInvalid).contains("signature"));
        assert!(format!("{}", ValidationError::DsMismatch).contains("DS"));
        assert!(format!("{}", ValidationError::DenialOfExistenceFailed).contains("denial"));
        assert!(format!("{}", ValidationError::DepthExceeded).contains("depth"));
        assert!(format!("{}", ValidationError::ChainBroken("test".to_string())).contains("broken"));
    }

    // ── Validator tests ────────────────────────────────────────────────

    #[test]
    fn test_validator_nta() {
        let mut validator = DnssecValidator::with_root_anchor();
        validator.add_negative_trust_anchor("broken.example.com");

        assert!(validator.is_negative_trust_anchor("broken.example.com"));
        assert!(validator.is_negative_trust_anchor("host.broken.example.com"));
        assert!(!validator.is_negative_trust_anchor("example.com"));
    }

    #[test]
    fn test_validator_nta_validation_returns_insecure() {
        let mut validator = DnssecValidator::with_root_anchor();
        validator.add_negative_trust_anchor("broken.example.com");

        let (result, errors) = validator.validate_rrset("host.broken.example.com", &[], &[]);
        assert_eq!(result, ValidationResult::Insecure);
        assert!(errors.is_empty());
    }

    #[test]
    fn test_validator_remove_nta() {
        let mut validator = DnssecValidator::with_root_anchor();
        validator.add_negative_trust_anchor("example.com");
        assert!(validator.is_negative_trust_anchor("example.com"));
        validator.remove_negative_trust_anchor("example.com");
        assert!(!validator.is_negative_trust_anchor("example.com"));
    }

    #[test]
    fn test_validator_no_rrsigs_no_anchor() {
        let validator = DnssecValidator::new(TrustAnchorStore::new());
        let (result, _) = validator.validate_rrset("example.com", &[], &[]);
        // No trust anchor → insecure
        assert_eq!(result, ValidationResult::Insecure);
    }

    #[test]
    fn test_validator_no_rrsigs_with_anchor() {
        let validator = DnssecValidator::with_root_anchor();
        let (result, errors) = validator.validate_rrset("example.com", &[], &[]);
        // Has trust anchor but no RRSIG → bogus
        assert_eq!(result, ValidationResult::Bogus);
        assert!(errors.contains(&ValidationError::NoRrsigs));
    }

    #[test]
    fn test_validator_no_dnskeys() {
        let validator = DnssecValidator::with_root_anchor();
        let now = current_unix_time();

        // Create an RRSIG
        let mut rdata = Vec::new();
        rdata.extend_from_slice(&1u16.to_be_bytes());
        rdata.push(8);
        rdata.push(2);
        rdata.extend_from_slice(&3600u32.to_be_bytes());
        rdata.extend_from_slice(&(now + 3600).to_be_bytes());
        rdata.extend_from_slice(&(now - 3600).to_be_bytes());
        rdata.extend_from_slice(&12345u16.to_be_bytes());
        rdata.push(0);
        rdata.extend_from_slice(&[0xFF]);
        let rrsig = RrsigRecord::parse("example.com", 3600, &rdata).unwrap();

        let (result, errors) = validator.validate_rrset("example.com", &[rrsig], &[]);
        assert_eq!(result, ValidationResult::Bogus);
        assert!(errors.contains(&ValidationError::NoDnskeys));
    }

    #[test]
    fn test_validator_matching_key_indeterminate() {
        let validator = DnssecValidator::with_root_anchor();
        let now = current_unix_time();

        // Create a DNSKEY
        let mut key_rdata = Vec::new();
        key_rdata.extend_from_slice(&256u16.to_be_bytes()); // ZSK
        key_rdata.push(3);
        key_rdata.push(8); // RSA/SHA-256
        key_rdata.extend_from_slice(&[1, 2, 3, 4]);
        let dnskey = DnskeyRecord::parse("example.com", 3600, &key_rdata).unwrap();

        // Create a matching RRSIG
        let mut sig_rdata = Vec::new();
        sig_rdata.extend_from_slice(&1u16.to_be_bytes()); // type covered: A
        sig_rdata.push(8); // algorithm
        sig_rdata.push(2); // labels
        sig_rdata.extend_from_slice(&3600u32.to_be_bytes());
        sig_rdata.extend_from_slice(&(now + 3600).to_be_bytes());
        sig_rdata.extend_from_slice(&(now - 3600).to_be_bytes());
        sig_rdata.extend_from_slice(&dnskey.key_tag().to_be_bytes());
        sig_rdata.push(0); // root signer
        sig_rdata.extend_from_slice(&[0xDE, 0xAD]);
        let rrsig = RrsigRecord::parse("example.com", 3600, &sig_rdata).unwrap();

        let (result, _) = validator.validate_rrset("example.com", &[rrsig], &[dnskey]);
        // Structure is valid but we can't do crypto → Indeterminate
        assert_eq!(result, ValidationResult::Indeterminate);
    }

    // ── Helper function tests ──────────────────────────────────────────

    #[test]
    fn test_compute_key_tag_deterministic() {
        let rdata = vec![0x01, 0x01, 0x03, 0x08, 0xAA, 0xBB];
        let tag1 = compute_key_tag(&rdata);
        let tag2 = compute_key_tag(&rdata);
        assert_eq!(tag1, tag2);
    }

    #[test]
    fn test_compute_key_tag_different_data() {
        let rdata1 = vec![0x01, 0x01, 0x03, 0x08, 0xAA];
        let rdata2 = vec![0x01, 0x01, 0x03, 0x08, 0xBB];
        assert_ne!(compute_key_tag(&rdata1), compute_key_tag(&rdata2));
    }

    #[test]
    fn test_compute_key_tag_empty() {
        assert_eq!(compute_key_tag(&[]), 0);
    }

    #[test]
    fn test_canonical_name_wire_root() {
        let wire = canonical_name_wire(".").unwrap();
        assert_eq!(wire, vec![0]);
    }

    #[test]
    fn test_canonical_name_wire_empty() {
        let wire = canonical_name_wire("").unwrap();
        assert_eq!(wire, vec![0]);
    }

    #[test]
    fn test_canonical_name_wire_simple() {
        let wire = canonical_name_wire("example.com").unwrap();
        assert_eq!(
            wire,
            vec![
                7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0
            ]
        );
    }

    #[test]
    fn test_canonical_name_wire_trailing_dot() {
        let wire = canonical_name_wire("example.com.").unwrap();
        assert_eq!(wire, canonical_name_wire("example.com").unwrap());
    }

    #[test]
    fn test_canonical_name_wire_uppercased_lowered() {
        let wire = canonical_name_wire("EXAMPLE.COM").unwrap();
        assert_eq!(wire, canonical_name_wire("example.com").unwrap());
    }

    #[test]
    fn test_count_labels_root() {
        assert_eq!(count_labels(""), 0);
        assert_eq!(count_labels("."), 0);
    }

    #[test]
    fn test_count_labels_tld() {
        assert_eq!(count_labels("com"), 1);
        assert_eq!(count_labels("com."), 1);
    }

    #[test]
    fn test_count_labels_domain() {
        assert_eq!(count_labels("example.com"), 2);
        assert_eq!(count_labels("www.example.com"), 3);
        assert_eq!(count_labels("a.b.c.d.e"), 5);
    }

    #[test]
    fn test_canonical_name_order_equal() {
        assert_eq!(
            canonical_name_order("example.com", "example.com"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_canonical_name_order_case_insensitive() {
        assert_eq!(
            canonical_name_order("EXAMPLE.COM", "example.com"),
            std::cmp::Ordering::Equal
        );
    }

    #[test]
    fn test_canonical_name_order_basic() {
        // RFC 4034 §6.1: comparison is right-to-left by label
        assert_eq!(
            canonical_name_order("a.example.com", "b.example.com"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_canonical_name_order_tld_difference() {
        assert_eq!(
            canonical_name_order("example.com", "example.org"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_canonical_name_order_prefix() {
        // "example.com" < "host.example.com" because fewer labels
        assert_eq!(
            canonical_name_order("example.com", "host.example.com"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_canonical_name_order_trailing_dot() {
        assert_eq!(
            canonical_name_order("example.com.", "example.com"),
            std::cmp::Ordering::Equal
        );
    }

    // ── parse_name_uncompressed tests ──────────────────────────────────

    #[test]
    fn test_parse_name_uncompressed_root() {
        let (name, end) = parse_name_uncompressed(&[0], 0).unwrap();
        assert_eq!(name, ".");
        assert_eq!(end, 1);
    }

    #[test]
    fn test_parse_name_uncompressed_simple() {
        let data = [3, b'c', b'o', b'm', 0];
        let (name, end) = parse_name_uncompressed(&data, 0).unwrap();
        assert_eq!(name, "com");
        assert_eq!(end, 5);
    }

    #[test]
    fn test_parse_name_uncompressed_multi_label() {
        let data = [
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ];
        let (name, end) = parse_name_uncompressed(&data, 0).unwrap();
        assert_eq!(name, "example.com");
        assert_eq!(end, 13);
    }

    #[test]
    fn test_parse_name_uncompressed_rejects_compression() {
        // A compression pointer (0xC0 0x00)
        let data = [0xC0, 0x00];
        assert!(parse_name_uncompressed(&data, 0).is_none());
    }

    #[test]
    fn test_parse_name_uncompressed_truncated() {
        let data = [3, b'c', b'o']; // says 3 bytes but only 2 available
        assert!(parse_name_uncompressed(&data, 0).is_none());
    }

    #[test]
    fn test_parse_name_uncompressed_empty() {
        assert!(parse_name_uncompressed(&[], 0).is_none());
    }

    // ── parse_type_bit_maps tests ──────────────────────────────────────

    #[test]
    fn test_parse_type_bit_maps_empty() {
        let types = parse_type_bit_maps(&[]).unwrap();
        assert!(types.is_empty());
    }

    #[test]
    fn test_parse_type_bit_maps_single_type() {
        // Window 0, bitmap length 1, bit 1 set (type A = 1)
        let data = [0, 1, 0x40];
        let types = parse_type_bit_maps(&data).unwrap();
        assert_eq!(types, vec![1]);
    }

    #[test]
    fn test_parse_type_bit_maps_multiple_types() {
        // Window 0, bitmap length 4
        let mut bitmap = [0u8; 4];
        bitmap[0] = 0x40; // type 1 (A)
        bitmap[3] = 0x08; // type 28 (AAAA)

        let mut data = vec![0, 4]; // window 0, length 4
        data.extend_from_slice(&bitmap);

        let types = parse_type_bit_maps(&data).unwrap();
        assert!(types.contains(&1));
        assert!(types.contains(&28));
        assert!(!types.contains(&2));
    }

    #[test]
    fn test_parse_type_bit_maps_multiple_windows() {
        let data = vec![
            // Window 0: type 1 (A)
            0, 1, 0x40, // Window 1: type 256 (would be bit 0 of window 1, byte 0, bit 0)
            1, 1, 0x80, // bit 0 → type 256
        ];

        let types = parse_type_bit_maps(&data).unwrap();
        assert!(types.contains(&1));
        assert!(types.contains(&256));
    }

    // ── encode_type_bit_maps tests ─────────────────────────────────────

    #[test]
    fn test_encode_type_bit_maps_empty() {
        assert!(encode_type_bit_maps(&[]).is_empty());
    }

    #[test]
    fn test_encode_type_bit_maps_roundtrip() {
        let original_types = vec![1, 2, 5, 6, 28, 46, 47, 48];
        let encoded = encode_type_bit_maps(&original_types);
        let decoded = parse_type_bit_maps(&encoded).unwrap();

        // Should contain all original types
        for t in &original_types {
            assert!(decoded.contains(t), "missing type {}", t);
        }
    }

    #[test]
    fn test_encode_type_bit_maps_single() {
        let encoded = encode_type_bit_maps(&[1]); // A record
        let decoded = parse_type_bit_maps(&encoded).unwrap();
        assert_eq!(decoded, vec![1]);
    }

    #[test]
    fn test_encode_type_bit_maps_high_type() {
        let encoded = encode_type_bit_maps(&[256]); // Window 1
        let decoded = parse_type_bit_maps(&encoded).unwrap();
        assert_eq!(decoded, vec![256]);
    }

    // ── current_unix_time test ─────────────────────────────────────────

    #[test]
    fn test_current_unix_time_nonzero() {
        let t = current_unix_time();
        // Should be after 2020-01-01
        assert!(t > 1577836800);
    }
}
