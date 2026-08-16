//! NTS (Network Time Security) — RFC 8915
//!
//! Provides authenticated NTP via:
//! 1. NTS-KE (Key Establishment) over TLS 1.3 (TCP port 4460)
//! 2. NTS extension fields in NTP packets (AEAD-protected)
//!
//! Mandatory-to-implement AEAD: AEAD_AES_SIV_CMAC_256 (algorithm ID 15)

use std::fmt;
use std::io::{self, Read, Write};
use std::net::{SocketAddr, TcpStream, ToSocketAddrs};
use std::sync::Arc;
use std::time::Duration;

use aead::{Aead, KeyInit, Payload};
use aes_siv::Aes128SivAead; // 256-bit key = 2×AES-128 → AEAD_AES_SIV_CMAC_256
use rand::RngExt;

// ── Constants ──────────────────────────────────────────────────────────────

/// Default NTS-KE port (RFC 8915 §4)
pub const NTS_KE_PORT: u16 = 4460;

/// TLS ALPN protocol ID for NTS-KE (RFC 8915 §4)
const NTS_KE_ALPN: &[u8] = b"ntske/1";

/// NTP protocol ID in NTS-KE Next Protocol Negotiation (RFC 8915 §4.1.2)
const NTS_NEXT_PROTOCOL_NTPV4: u16 = 0;

/// AEAD_AES_SIV_CMAC_256 algorithm ID (RFC 8915 §4.1.5, IANA)
pub const AEAD_AES_SIV_CMAC_256: u16 = 15;

/// Key length for AEAD_AES_SIV_CMAC_256 (32 bytes = 256 bits)
const AEAD_KEY_LEN: usize = 32;

/// SIV tag/nonce length (16 bytes)
const SIV_TAG_LEN: usize = 16;

/// TLS exporter label (RFC 8915 §4.2)
const NTS_TLS_EXPORTER_LABEL: &str = "EXPORTER-network-time-security";

/// Exporter context byte for C2S key
const NTS_EXPORTER_CONTEXT_C2S: u8 = 0x00;

/// Exporter context byte for S2C key
const NTS_EXPORTER_CONTEXT_S2C: u8 = 0x01;

/// NTP extension field type: Unique Identifier (RFC 8915 §5.3)
pub const EXT_TYPE_UNIQUE_ID: u16 = 0x0104;

/// NTP extension field type: NTS Cookie (RFC 8915 §5.4)
pub const EXT_TYPE_NTS_COOKIE: u16 = 0x0204;

/// NTP extension field type: NTS Cookie Placeholder (RFC 8915 §5.5)
pub const EXT_TYPE_NTS_COOKIE_PLACEHOLDER: u16 = 0x0304;

/// NTP extension field type: NTS Authenticator (RFC 8915 §5.6)
pub const EXT_TYPE_NTS_AUTHENTICATOR: u16 = 0x0404;

/// Minimum unique identifier length (bytes)
const UNIQUE_ID_LEN: usize = 32;

/// Maximum number of cookies to store
const MAX_COOKIES: usize = 8;

/// NTS-KE TCP connect + TLS handshake timeout
const NTS_KE_TIMEOUT: Duration = Duration::from_secs(15);

/// Minimum NTP extension field length (type + length = 4 bytes, minimum total
/// length including value must be at least 16 per RFC 7822 §3, but RFC 8915
/// relaxes this for the last EF which may be 4 bytes of header only).
/// We enforce the 4-byte header minimum for parsing.
const EXT_FIELD_HEADER_LEN: usize = 4;

// ── Error types ────────────────────────────────────────────────────────────

/// NTS errors
#[derive(Debug)]
pub enum NtsError {
    /// TLS connection or handshake failure
    Tls(String),
    /// NTS-KE protocol error
    KeProtocol(String),
    /// NTS-KE server sent an error record
    KeServerError(u16),
    /// AEAD encryption/decryption failure
    Aead(String),
    /// No cookies available
    NoCookies,
    /// NTP extension field parsing error
    ExtensionField(String),
    /// I/O error
    Io(io::Error),
    /// Authentication failure (response validation)
    AuthFailed(String),
}

impl fmt::Display for NtsError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Tls(msg) => write!(f, "NTS TLS error: {msg}"),
            Self::KeProtocol(msg) => write!(f, "NTS-KE protocol error: {msg}"),
            Self::KeServerError(code) => write!(f, "NTS-KE server error: {code}"),
            Self::Aead(msg) => write!(f, "NTS AEAD error: {msg}"),
            Self::NoCookies => write!(f, "NTS: no cookies available"),
            Self::ExtensionField(msg) => write!(f, "NTS extension field error: {msg}"),
            Self::Io(e) => write!(f, "NTS I/O error: {e}"),
            Self::AuthFailed(msg) => write!(f, "NTS authentication failed: {msg}"),
        }
    }
}

impl std::error::Error for NtsError {}

impl From<io::Error> for NtsError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

// ── NTS-KE record types (RFC 8915 §4.1) ───────────────────────────────────

/// NTS-KE record type codes
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u16)]
pub enum NtsKeRecordType {
    /// End of Message (RFC 8915 §4.1.1)
    EndOfMessage = 0,
    /// Next Protocol Negotiation (RFC 8915 §4.1.2)
    NextProtocol = 1,
    /// Error (RFC 8915 §4.1.3)
    Error = 2,
    /// Warning (RFC 8915 §4.1.4)
    Warning = 3,
    /// AEAD Algorithm Negotiation (RFC 8915 §4.1.5)
    AeadAlgorithm = 4,
    /// New Cookie for NTPv4 (RFC 8915 §4.1.6)
    NewCookieForNtpv4 = 5,
    /// NTPv4 Server Negotiation (RFC 8915 §4.1.7)
    Ntpv4Server = 6,
    /// NTPv4 Port Negotiation (RFC 8915 §4.1.8)
    Ntpv4Port = 7,
}

impl NtsKeRecordType {
    fn from_u16(v: u16) -> Option<Self> {
        match v {
            0 => Some(Self::EndOfMessage),
            1 => Some(Self::NextProtocol),
            2 => Some(Self::Error),
            3 => Some(Self::Warning),
            4 => Some(Self::AeadAlgorithm),
            5 => Some(Self::NewCookieForNtpv4),
            6 => Some(Self::Ntpv4Server),
            7 => Some(Self::Ntpv4Port),
            _ => None,
        }
    }
}

/// A single NTS-KE record (RFC 8915 §4)
///
/// Wire format:
///   - Bit 0: Critical bit (1 = critical, 0 = non-critical)
///   - Bits 1-15: Record type number
///   - Bytes 2-3: Body length (network byte order)
///   - Remaining: Body data
#[derive(Debug, Clone)]
pub struct NtsKeRecord {
    pub critical: bool,
    pub record_type: u16,
    pub body: Vec<u8>,
}

impl NtsKeRecord {
    /// Create a new NTS-KE record
    pub fn new(critical: bool, record_type: NtsKeRecordType, body: Vec<u8>) -> Self {
        Self {
            critical,
            record_type: record_type as u16,
            body,
        }
    }

    /// Serialize the record to wire format
    pub fn to_bytes(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(4 + self.body.len());
        let type_and_critical = if self.critical {
            self.record_type | 0x8000
        } else {
            self.record_type
        };
        buf.extend_from_slice(&type_and_critical.to_be_bytes());
        buf.extend_from_slice(&(self.body.len() as u16).to_be_bytes());
        buf.extend_from_slice(&self.body);
        buf
    }

    /// Parse a single NTS-KE record from a byte slice.
    /// Returns the record and the number of bytes consumed.
    pub fn from_bytes(data: &[u8]) -> Result<(Self, usize), NtsError> {
        if data.len() < 4 {
            return Err(NtsError::KeProtocol("record too short".into()));
        }
        let first_word = u16::from_be_bytes([data[0], data[1]]);
        let critical = (first_word & 0x8000) != 0;
        let record_type = first_word & 0x7FFF;
        let body_len = u16::from_be_bytes([data[2], data[3]]) as usize;

        if data.len() < 4 + body_len {
            return Err(NtsError::KeProtocol(format!(
                "record body truncated: need {} bytes, have {}",
                body_len,
                data.len() - 4
            )));
        }

        let body = data[4..4 + body_len].to_vec();
        Ok((
            Self {
                critical,
                record_type,
                body,
            },
            4 + body_len,
        ))
    }
}

// ── NTS-KE client request building ─────────────────────────────────────────

/// Build the NTS-KE client request records.
///
/// The client sends:
///   1. Next Protocol Negotiation (NTPv4 = 0)
///   2. AEAD Algorithm Negotiation (AEAD_AES_SIV_CMAC_256 = 15)
///   3. End of Message
fn build_nts_ke_request() -> Vec<u8> {
    let mut buf = Vec::new();

    // Record 1: Next Protocol Negotiation — NTPv4 (protocol ID 0)
    let next_proto = NtsKeRecord::new(
        true, // critical
        NtsKeRecordType::NextProtocol,
        NTS_NEXT_PROTOCOL_NTPV4.to_be_bytes().to_vec(),
    );
    buf.extend_from_slice(&next_proto.to_bytes());

    // Record 2: AEAD Algorithm Negotiation — AEAD_AES_SIV_CMAC_256 (ID 15)
    let aead_algo = NtsKeRecord::new(
        true, // critical
        NtsKeRecordType::AeadAlgorithm,
        AEAD_AES_SIV_CMAC_256.to_be_bytes().to_vec(),
    );
    buf.extend_from_slice(&aead_algo.to_bytes());

    // Record 3: End of Message
    let eom = NtsKeRecord::new(true, NtsKeRecordType::EndOfMessage, Vec::new());
    buf.extend_from_slice(&eom.to_bytes());

    buf
}

/// Parse all NTS-KE response records from a byte buffer.
fn parse_nts_ke_records(data: &[u8]) -> Result<Vec<NtsKeRecord>, NtsError> {
    let mut records = Vec::new();
    let mut offset = 0;

    while offset < data.len() {
        let (record, consumed) = NtsKeRecord::from_bytes(&data[offset..])?;
        let is_eom = record.record_type == NtsKeRecordType::EndOfMessage as u16;
        records.push(record);
        offset += consumed;
        if is_eom {
            break;
        }
    }

    Ok(records)
}

// ── NTS-KE result ──────────────────────────────────────────────────────────

/// Result of a successful NTS-KE exchange
#[derive(Debug, Clone)]
pub struct NtsKeResult {
    /// Client-to-server AEAD key
    pub c2s_key: Vec<u8>,
    /// Server-to-client AEAD key
    pub s2c_key: Vec<u8>,
    /// Negotiated AEAD algorithm
    pub aead_algorithm: u16,
    /// Cookies obtained from the server
    pub cookies: Vec<Vec<u8>>,
    /// NTP server to use (if server negotiation provided one; otherwise use
    /// the NTS-KE server hostname)
    pub ntp_server: Option<String>,
    /// NTP port to use (if port negotiation provided one; otherwise 123)
    pub ntp_port: Option<u16>,
}

// ── TLS key derivation (RFC 8915 §4.2) ────────────────────────────────────

/// Derive the C2S and S2C keys from a TLS 1.3 exporter.
///
/// Per RFC 8915 §4.2, the context is 5 bytes:
///   - Byte 0: 0x00 for C2S, 0x01 for S2C
///   - Bytes 1-2: AEAD algorithm in network byte order
///   - Bytes 3-4: reserved, set to 0x0000
fn derive_nts_keys(
    tls_conn: &rustls::ClientConnection,
    aead_algorithm: u16,
) -> Result<(Vec<u8>, Vec<u8>), NtsError> {
    let algo_bytes = aead_algorithm.to_be_bytes();

    // C2S key
    let c2s_context = [
        NTS_EXPORTER_CONTEXT_C2S,
        algo_bytes[0],
        algo_bytes[1],
        0x00,
        0x00,
    ];
    let mut c2s_key = vec![0u8; AEAD_KEY_LEN];
    tls_conn
        .export_keying_material(
            &mut c2s_key,
            NTS_TLS_EXPORTER_LABEL.as_bytes(),
            Some(&c2s_context),
        )
        .map_err(|e| NtsError::Tls(format!("C2S key export failed: {e}")))?;

    // S2C key
    let s2c_context = [
        NTS_EXPORTER_CONTEXT_S2C,
        algo_bytes[0],
        algo_bytes[1],
        0x00,
        0x00,
    ];
    let mut s2c_key = vec![0u8; AEAD_KEY_LEN];
    tls_conn
        .export_keying_material(
            &mut s2c_key,
            NTS_TLS_EXPORTER_LABEL.as_bytes(),
            Some(&s2c_context),
        )
        .map_err(|e| NtsError::Tls(format!("S2C key export failed: {e}")))?;

    Ok((c2s_key, s2c_key))
}

// ── NTS-KE exchange ────────────────────────────────────────────────────────

/// Build a rustls ClientConfig for NTS-KE (TLS 1.3, ALPN "ntske/1").
fn build_tls_config() -> Result<Arc<rustls::ClientConfig>, NtsError> {
    let mut root_store = rustls::RootCertStore::empty();
    root_store.extend(webpki_roots::TLS_SERVER_ROOTS.iter().cloned());

    let mut config = rustls::ClientConfig::builder()
        .with_root_certificates(root_store)
        .with_no_client_auth();

    config.alpn_protocols = vec![NTS_KE_ALPN.to_vec()];

    Ok(Arc::new(config))
}

/// Perform an NTS-KE exchange with the given server.
///
/// 1. Connect via TLS 1.3 to `server:port` (default port 4460)
/// 2. Send NTS-KE request records
/// 3. Receive and parse NTS-KE response records
/// 4. Derive C2S/S2C keys from TLS exporter
/// 5. Return cookies and keys
pub fn nts_ke_exchange(server: &str, port: u16) -> Result<NtsKeResult, NtsError> {
    let tls_config = build_tls_config()?;

    let server_name: rustls::pki_types::ServerName<'_> = server
        .try_into()
        .map_err(|e| NtsError::Tls(format!("invalid server name '{server}': {e}")))?;

    let mut tls_conn = rustls::ClientConnection::new(tls_config, server_name.to_owned())
        .map_err(|e| NtsError::Tls(format!("TLS client creation failed: {e}")))?;

    // TCP connect
    let addr_str = format!("{server}:{port}");
    let addrs: Vec<SocketAddr> = addr_str
        .to_socket_addrs()
        .map_err(|e| {
            NtsError::Io(io::Error::new(
                io::ErrorKind::NotFound,
                format!("resolve {addr_str}: {e}"),
            ))
        })?
        .collect();

    if addrs.is_empty() {
        return Err(NtsError::Io(io::Error::new(
            io::ErrorKind::NotFound,
            format!("could not resolve {addr_str}"),
        )));
    }

    let mut tcp_stream = None;
    for addr in &addrs {
        match TcpStream::connect_timeout(addr, NTS_KE_TIMEOUT) {
            Ok(s) => {
                s.set_read_timeout(Some(NTS_KE_TIMEOUT))?;
                s.set_write_timeout(Some(NTS_KE_TIMEOUT))?;
                tcp_stream = Some(s);
                break;
            }
            Err(e) => {
                log::debug!("NTS-KE: TCP connect to {addr} failed: {e}");
            }
        }
    }

    let mut tcp = tcp_stream.ok_or_else(|| {
        NtsError::Io(io::Error::new(
            io::ErrorKind::ConnectionRefused,
            format!("could not connect to any address for {addr_str}"),
        ))
    })?;

    // Complete TLS handshake
    let mut stream = rustls::Stream::new(&mut tls_conn, &mut tcp);

    // Send NTS-KE request
    let request = build_nts_ke_request();
    stream
        .write_all(&request)
        .map_err(|e| NtsError::Tls(format!("NTS-KE write failed: {e}")))?;
    stream
        .flush()
        .map_err(|e| NtsError::Tls(format!("NTS-KE flush failed: {e}")))?;

    // Read NTS-KE response
    let mut response_buf = vec![0u8; 16384];
    let mut total_read = 0;
    loop {
        match stream.read(&mut response_buf[total_read..]) {
            Ok(0) => break, // EOF
            Ok(n) => {
                total_read += n;
                // Check if we've seen an End of Message record
                if has_end_of_message(&response_buf[..total_read]) {
                    break;
                }
                if total_read >= response_buf.len() {
                    return Err(NtsError::KeProtocol("response too large".into()));
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                // Timeout — check if we have enough data
                if total_read > 0 && has_end_of_message(&response_buf[..total_read]) {
                    break;
                }
                return Err(NtsError::Tls("NTS-KE read timed out".into()));
            }
            Err(e) => {
                return Err(NtsError::Tls(format!("NTS-KE read failed: {e}")));
            }
        }
    }

    if total_read == 0 {
        return Err(NtsError::KeProtocol("empty NTS-KE response".into()));
    }

    // Parse response records
    let records = parse_nts_ke_records(&response_buf[..total_read])?;

    // Process records
    let mut got_next_proto = false;
    let mut aead_algorithm: Option<u16> = None;
    let mut cookies: Vec<Vec<u8>> = Vec::new();
    let mut ntp_server: Option<String> = None;
    let mut ntp_port: Option<u16> = None;

    for record in &records {
        let rtype = NtsKeRecordType::from_u16(record.record_type);
        match rtype {
            Some(NtsKeRecordType::EndOfMessage) => break,
            Some(NtsKeRecordType::NextProtocol) => {
                if record.body.len() < 2 {
                    return Err(NtsError::KeProtocol("NextProtocol body too short".into()));
                }
                let proto = u16::from_be_bytes([record.body[0], record.body[1]]);
                if proto != NTS_NEXT_PROTOCOL_NTPV4 {
                    return Err(NtsError::KeProtocol(format!(
                        "server selected unsupported protocol {proto}"
                    )));
                }
                got_next_proto = true;
            }
            Some(NtsKeRecordType::Error) => {
                let code = if record.body.len() >= 2 {
                    u16::from_be_bytes([record.body[0], record.body[1]])
                } else {
                    0
                };
                return Err(NtsError::KeServerError(code));
            }
            Some(NtsKeRecordType::Warning) => {
                let code = if record.body.len() >= 2 {
                    u16::from_be_bytes([record.body[0], record.body[1]])
                } else {
                    0
                };
                log::warn!("NTS-KE: server warning code {code}");
            }
            Some(NtsKeRecordType::AeadAlgorithm) => {
                if record.body.len() < 2 {
                    return Err(NtsError::KeProtocol("AEAD algorithm body too short".into()));
                }
                let algo = u16::from_be_bytes([record.body[0], record.body[1]]);
                if algo != AEAD_AES_SIV_CMAC_256 {
                    return Err(NtsError::KeProtocol(format!(
                        "server selected unsupported AEAD algorithm {algo}"
                    )));
                }
                aead_algorithm = Some(algo);
            }
            Some(NtsKeRecordType::NewCookieForNtpv4) => {
                if cookies.len() < MAX_COOKIES {
                    cookies.push(record.body.clone());
                }
            }
            Some(NtsKeRecordType::Ntpv4Server) => {
                // ASCII-encoded server name (RFC 8915 §4.1.7)
                if let Ok(s) = std::str::from_utf8(&record.body) {
                    ntp_server = Some(s.to_string());
                }
            }
            Some(NtsKeRecordType::Ntpv4Port) => {
                if record.body.len() >= 2 {
                    ntp_port = Some(u16::from_be_bytes([record.body[0], record.body[1]]));
                }
            }
            None => {
                if record.critical {
                    return Err(NtsError::KeProtocol(format!(
                        "unrecognized critical record type {}",
                        record.record_type
                    )));
                }
                // Non-critical unknown records are silently ignored
            }
        }
    }

    if !got_next_proto {
        return Err(NtsError::KeProtocol(
            "server did not confirm Next Protocol".into(),
        ));
    }

    let aead_algorithm = aead_algorithm
        .ok_or_else(|| NtsError::KeProtocol("server did not confirm AEAD algorithm".into()))?;

    if cookies.is_empty() {
        return Err(NtsError::KeProtocol("server provided no cookies".into()));
    }

    // Derive C2S and S2C keys from TLS exporter
    let (c2s_key, s2c_key) = derive_nts_keys(&tls_conn, aead_algorithm)?;

    log::info!(
        "NTS-KE: obtained {} cookie(s), AEAD={}{}{}",
        cookies.len(),
        aead_algorithm,
        ntp_server
            .as_ref()
            .map(|s| format!(", server={s}"))
            .unwrap_or_default(),
        ntp_port.map(|p| format!(", port={p}")).unwrap_or_default(),
    );

    Ok(NtsKeResult {
        c2s_key,
        s2c_key,
        aead_algorithm,
        cookies,
        ntp_server,
        ntp_port,
    })
}

/// Quick scan for End of Message record in raw NTS-KE response data.
fn has_end_of_message(data: &[u8]) -> bool {
    let mut offset = 0;
    while offset + 4 <= data.len() {
        let type_word = u16::from_be_bytes([data[offset], data[offset + 1]]);
        let rtype = type_word & 0x7FFF;
        let body_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;
        if rtype == NtsKeRecordType::EndOfMessage as u16 {
            return true;
        }
        let next = offset + 4 + body_len;
        if next <= offset {
            return false; // prevent infinite loop
        }
        offset = next;
    }
    false
}

// ── NTP Extension Fields (RFC 7822 / RFC 8915 §5) ─────────────────────────

/// A parsed NTP extension field
#[derive(Debug, Clone)]
pub struct NtpExtensionField {
    pub field_type: u16,
    pub value: Vec<u8>,
}

impl NtpExtensionField {
    /// Serialize to wire format (type 2 bytes + length 2 bytes + value padded
    /// to 4-byte boundary). Length field includes the 4-byte header.
    pub fn to_bytes(&self) -> Vec<u8> {
        let padded_value_len = (self.value.len() + 3) & !3; // round up to 4
        let total_len = 4 + padded_value_len;
        let mut buf = Vec::with_capacity(total_len);
        buf.extend_from_slice(&self.field_type.to_be_bytes());
        buf.extend_from_slice(&(total_len as u16).to_be_bytes());
        buf.extend_from_slice(&self.value);
        // Pad with zeros
        let padding = padded_value_len - self.value.len();
        buf.extend(std::iter::repeat_n(0u8, padding));
        buf
    }

    /// Parse extension fields from the bytes following the 48-byte NTP header.
    pub fn parse_all(data: &[u8]) -> Result<Vec<Self>, NtsError> {
        let mut fields = Vec::new();
        let mut offset = 0;

        while offset + EXT_FIELD_HEADER_LEN <= data.len() {
            let field_type = u16::from_be_bytes([data[offset], data[offset + 1]]);
            let total_len = u16::from_be_bytes([data[offset + 2], data[offset + 3]]) as usize;

            // RFC 7822: minimum extension field length is 4 (header-only for
            // the last field); for non-last fields it should be >= 16, but we
            // are lenient.
            if total_len < EXT_FIELD_HEADER_LEN {
                return Err(NtsError::ExtensionField(format!(
                    "extension field length {total_len} < minimum {EXT_FIELD_HEADER_LEN}"
                )));
            }

            if offset + total_len > data.len() {
                return Err(NtsError::ExtensionField(format!(
                    "extension field at offset {offset} extends beyond data (len={total_len}, remaining={})",
                    data.len() - offset
                )));
            }

            let value_len = total_len - EXT_FIELD_HEADER_LEN;
            let value = data
                [offset + EXT_FIELD_HEADER_LEN..offset + EXT_FIELD_HEADER_LEN + value_len]
                .to_vec();

            fields.push(Self { field_type, value });
            offset += total_len;
        }

        Ok(fields)
    }
}

// ── AEAD operations ────────────────────────────────────────────────────────

/// Encrypt with AEAD_AES_SIV_CMAC_256.
///
/// - `key`: 32-byte C2S key
/// - `nonce`: 16-byte nonce (random)
/// - `aad`: additional authenticated data (NTP header + preceding extension fields)
/// - `plaintext`: plaintext to encrypt (may be empty for NTS)
///
/// Returns nonce || ciphertext (ciphertext includes 16-byte SIV tag).
pub fn nts_aead_encrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    plaintext: &[u8],
) -> Result<Vec<u8>, NtsError> {
    if key.len() != AEAD_KEY_LEN {
        return Err(NtsError::Aead(format!(
            "invalid key length: {} (expected {AEAD_KEY_LEN})",
            key.len()
        )));
    }

    let cipher = Aes128SivAead::new_from_slice(key)
        .map_err(|e| NtsError::Aead(format!("cipher init: {e}")))?;

    // AES-SIV uses the nonce as one of the AAD components, and the `aead`
    // crate's SIV implementation takes a nonce parameter. We pass our
    // additional authenticated data via the `Payload` aad field.
    let aead_nonce = aead::generic_array::GenericArray::from_slice(nonce);
    let payload = Payload {
        msg: plaintext,
        aad,
    };

    let ciphertext = cipher
        .encrypt(aead_nonce, payload)
        .map_err(|e| NtsError::Aead(format!("encryption failed: {e}")))?;

    // Return nonce || ciphertext (ciphertext already has SIV tag prepended)
    let mut result = Vec::with_capacity(nonce.len() + ciphertext.len());
    result.extend_from_slice(nonce);
    result.extend_from_slice(&ciphertext);
    Ok(result)
}

/// Decrypt with AEAD_AES_SIV_CMAC_256.
///
/// - `key`: 32-byte S2C key
/// - `nonce`: 16-byte nonce (extracted from authenticator)
/// - `aad`: additional authenticated data
/// - `ciphertext`: ciphertext to decrypt (includes SIV tag)
///
/// Returns decrypted plaintext.
pub fn nts_aead_decrypt(
    key: &[u8],
    nonce: &[u8],
    aad: &[u8],
    ciphertext: &[u8],
) -> Result<Vec<u8>, NtsError> {
    if key.len() != AEAD_KEY_LEN {
        return Err(NtsError::Aead(format!(
            "invalid key length: {} (expected {AEAD_KEY_LEN})",
            key.len()
        )));
    }

    let cipher = Aes128SivAead::new_from_slice(key)
        .map_err(|e| NtsError::Aead(format!("cipher init: {e}")))?;

    let aead_nonce = aead::generic_array::GenericArray::from_slice(nonce);
    let payload = Payload {
        msg: ciphertext,
        aad,
    };

    cipher
        .decrypt(aead_nonce, payload)
        .map_err(|e| NtsError::Aead(format!("decryption failed: {e}")))
}

// ── Cookie jar ─────────────────────────────────────────────────────────────

/// Manages NTS cookies with automatic replenishment.
#[derive(Debug, Clone)]
pub struct NtsCookieJar {
    cookies: Vec<Vec<u8>>,
}

#[allow(dead_code)]
impl NtsCookieJar {
    /// Create a new cookie jar from initial cookies
    pub fn new(cookies: Vec<Vec<u8>>) -> Self {
        Self { cookies }
    }

    /// Take a cookie for use in an NTP request. Returns `None` if empty.
    pub fn take(&mut self) -> Option<Vec<u8>> {
        self.cookies.pop()
    }

    /// Add a cookie (received in an NTP response).
    pub fn add(&mut self, cookie: Vec<u8>) {
        if self.cookies.len() < MAX_COOKIES {
            self.cookies.push(cookie);
        }
    }

    /// Number of cookies available.
    pub fn len(&self) -> usize {
        self.cookies.len()
    }

    /// Whether the jar is empty.
    pub fn is_empty(&self) -> bool {
        self.cookies.is_empty()
    }

    /// Replace all cookies (e.g., after a fresh NTS-KE exchange).
    pub fn replace(&mut self, cookies: Vec<Vec<u8>>) {
        self.cookies = cookies;
    }
}

// ── NTS-protected NTP request/response ─────────────────────────────────────

/// Build the extension fields for an NTS-protected NTP request.
///
/// The order of extension fields matters:
///   1. Unique Identifier (plaintext, for anti-replay)
///   2. NTS Cookie (one cookie consumed from the jar)
///   3. NTS Cookie Placeholder(s) (request additional cookies, same length as
///      the provided cookie)
///   4. NTS Authenticator (AEAD-encrypts the preceding extension fields)
///
/// `ntp_header` is the 48-byte NTP header that precedes the extension fields.
/// It is included in the AEAD additional authenticated data.
///
/// Returns `(unique_id, extension_fields_bytes)` where unique_id can be used
/// to verify the response.
pub fn build_nts_request_extensions(
    c2s_key: &[u8],
    cookie: &[u8],
    ntp_header: &[u8],
    extra_cookie_placeholders: usize,
) -> Result<(Vec<u8>, Vec<u8>), NtsError> {
    let mut rng = rand::rng();

    // 1. Unique Identifier — random 32 bytes
    let mut unique_id = vec![0u8; UNIQUE_ID_LEN];
    rng.fill(&mut unique_id[..]);

    let uid_ef = NtpExtensionField {
        field_type: EXT_TYPE_UNIQUE_ID,
        value: unique_id.clone(),
    };

    // 2. NTS Cookie
    let cookie_ef = NtpExtensionField {
        field_type: EXT_TYPE_NTS_COOKIE,
        value: cookie.to_vec(),
    };

    // 3. NTS Cookie Placeholders (request fresh cookies from server)
    let cookie_placeholder_ef = NtpExtensionField {
        field_type: EXT_TYPE_NTS_COOKIE_PLACEHOLDER,
        value: vec![0u8; cookie.len()], // same length as actual cookie
    };

    // Serialize extension fields that go before the authenticator
    let mut plaintext_efs = Vec::new();
    plaintext_efs.extend_from_slice(&uid_ef.to_bytes());
    plaintext_efs.extend_from_slice(&cookie_ef.to_bytes());
    for _ in 0..extra_cookie_placeholders {
        plaintext_efs.extend_from_slice(&cookie_placeholder_ef.to_bytes());
    }

    // AAD = NTP header || extension fields before authenticator
    let mut aad = Vec::with_capacity(ntp_header.len() + plaintext_efs.len());
    aad.extend_from_slice(ntp_header);
    aad.extend_from_slice(&plaintext_efs);

    // Generate random nonce (16 bytes for AES-SIV)
    let mut nonce = [0u8; SIV_TAG_LEN];
    rng.fill(&mut nonce);

    // AEAD encrypt — for the request, the encrypted part can be empty (no
    // encrypted extension fields needed from client side).
    let encrypted = nts_aead_encrypt(c2s_key, &nonce, &aad, &[])?;

    // Build authenticator value: nonce_len (2) || nonce || ciphertext_len (2) || ciphertext
    let nonce_len = nonce.len() as u16;
    // ciphertext = encrypted minus the nonce prefix we concatenated in nts_aead_encrypt
    // Actually nts_aead_encrypt returns nonce || ciphertext, but we constructed nonce separately.
    // The ciphertext portion is everything after the nonce.
    let ciphertext = &encrypted[nonce.len()..];
    let ciphertext_len = ciphertext.len() as u16;

    let mut auth_value = Vec::new();
    auth_value.extend_from_slice(&nonce_len.to_be_bytes());
    auth_value.extend_from_slice(&nonce);
    auth_value.extend_from_slice(&ciphertext_len.to_be_bytes());
    auth_value.extend_from_slice(ciphertext);

    let auth_ef = NtpExtensionField {
        field_type: EXT_TYPE_NTS_AUTHENTICATOR,
        value: auth_value,
    };

    // Final extension fields bytes
    let mut result = plaintext_efs;
    result.extend_from_slice(&auth_ef.to_bytes());

    Ok((unique_id, result))
}

/// Parsed NTS response data extracted from NTP extension fields.
#[derive(Debug)]
#[allow(dead_code)]
pub struct NtsResponseData {
    /// The Unique Identifier echoed back by the server
    pub unique_id: Vec<u8>,
    /// New cookies provided by the server (decrypted from authenticator)
    pub new_cookies: Vec<Vec<u8>>,
    /// Decrypted extension fields from the authenticator
    pub decrypted_extensions: Vec<NtpExtensionField>,
}

/// Verify and parse an NTS-protected NTP response.
///
/// - `s2c_key`: the server-to-client AEAD key
/// - `ntp_packet`: the full NTP response packet (>= 48 bytes with extensions)
/// - `expected_unique_id`: the unique ID we sent in the request
///
/// Verifies the AEAD authenticator and extracts new cookies.
pub fn verify_nts_response(
    s2c_key: &[u8],
    ntp_packet: &[u8],
    expected_unique_id: &[u8],
) -> Result<NtsResponseData, NtsError> {
    if ntp_packet.len() < 48 {
        return Err(NtsError::ExtensionField(
            "packet too short for NTP header".into(),
        ));
    }

    let ntp_header = &ntp_packet[..48];
    let ext_data = &ntp_packet[48..];

    let fields = NtpExtensionField::parse_all(ext_data)?;

    // Find the Unique Identifier
    let uid_field = fields
        .iter()
        .find(|f| f.field_type == EXT_TYPE_UNIQUE_ID)
        .ok_or_else(|| NtsError::AuthFailed("response missing Unique Identifier".into()))?;

    if uid_field.value != expected_unique_id {
        return Err(NtsError::AuthFailed(
            "Unique Identifier mismatch — possible replay or spoofing".into(),
        ));
    }

    // Find the NTS Authenticator — must be the last extension field
    let auth_field = fields
        .iter()
        .rfind(|f| f.field_type == EXT_TYPE_NTS_AUTHENTICATOR)
        .ok_or_else(|| NtsError::AuthFailed("response missing NTS Authenticator".into()))?;

    // Parse authenticator value: nonce_len (2) || nonce || ciphertext_len (2) || ciphertext
    let auth = &auth_field.value;
    if auth.len() < 4 {
        return Err(NtsError::AuthFailed("authenticator value too short".into()));
    }

    let nonce_len = u16::from_be_bytes([auth[0], auth[1]]) as usize;
    if auth.len() < 2 + nonce_len + 2 {
        return Err(NtsError::AuthFailed("authenticator nonce truncated".into()));
    }
    let nonce = &auth[2..2 + nonce_len];
    let ct_len = u16::from_be_bytes([auth[2 + nonce_len], auth[3 + nonce_len]]) as usize;
    let ct_start = 4 + nonce_len;
    if auth.len() < ct_start + ct_len {
        return Err(NtsError::AuthFailed(
            "authenticator ciphertext truncated".into(),
        ));
    }
    let ciphertext = &auth[ct_start..ct_start + ct_len];

    // AAD = NTP header || all extension fields before the authenticator
    // We need to reconstruct the bytes of all EFs before the authenticator
    let mut aad = Vec::new();
    aad.extend_from_slice(ntp_header);

    // Serialize all fields before the authenticator for AAD
    for f in &fields {
        if f.field_type == EXT_TYPE_NTS_AUTHENTICATOR {
            break; // authenticator not included in AAD
        }
        aad.extend_from_slice(&f.to_bytes());
    }

    // Decrypt
    let plaintext = nts_aead_decrypt(s2c_key, nonce, &aad, ciphertext)?;

    // Parse decrypted extension fields (may contain new cookies)
    let decrypted_fields = if plaintext.is_empty() {
        Vec::new()
    } else {
        NtpExtensionField::parse_all(&plaintext)?
    };

    // Also collect plaintext NTS Cookie extension fields (server may send
    // new cookies outside the authenticator too, though RFC 8915 recommends
    // encrypting them).
    let mut new_cookies: Vec<Vec<u8>> = Vec::new();

    // Cookies from plaintext extension fields
    for f in &fields {
        if f.field_type == EXT_TYPE_NTS_COOKIE {
            // Note: the first cookie EF in the response is the one echoed
            // back from our request. Additional ones are new cookies.
            // However, in practice servers encrypt new cookies inside the
            // authenticator, so we mostly collect from decrypted fields.
        }
    }

    // Cookies from decrypted extension fields
    for f in &decrypted_fields {
        if f.field_type == EXT_TYPE_NTS_COOKIE {
            new_cookies.push(f.value.clone());
        }
    }

    Ok(NtsResponseData {
        unique_id: uid_field.value.clone(),
        new_cookies,
        decrypted_extensions: decrypted_fields,
    })
}

// ── NTS state management ───────────────────────────────────────────────────

/// Complete NTS state for a server association.
#[derive(Debug, Clone)]
pub struct NtsState {
    /// Client-to-server AEAD key
    pub c2s_key: Vec<u8>,
    /// Server-to-client AEAD key
    pub s2c_key: Vec<u8>,
    /// Negotiated AEAD algorithm
    #[allow(dead_code)]
    pub aead_algorithm: u16,
    /// Cookie jar
    pub cookie_jar: NtsCookieJar,
    /// NTS-KE server hostname (for re-keying)
    pub ke_server: String,
    /// NTS-KE server port
    #[allow(dead_code)]
    pub ke_port: u16,
    /// NTP server to use (may differ from KE server after negotiation)
    pub ntp_server: Option<String>,
    /// NTP port to use
    pub ntp_port: Option<u16>,
}

impl NtsState {
    /// Perform initial NTS-KE exchange and create the state.
    pub fn establish(ke_server: &str, ke_port: u16) -> Result<Self, NtsError> {
        let result = nts_ke_exchange(ke_server, ke_port)?;
        Ok(Self {
            c2s_key: result.c2s_key,
            s2c_key: result.s2c_key,
            aead_algorithm: result.aead_algorithm,
            cookie_jar: NtsCookieJar::new(result.cookies),
            ke_server: ke_server.to_string(),
            ke_port,
            ntp_server: result.ntp_server,
            ntp_port: result.ntp_port,
        })
    }

    /// Re-key: perform a new NTS-KE exchange, refreshing cookies and keys.
    #[allow(dead_code)]
    pub fn rekey(&mut self) -> Result<(), NtsError> {
        let result = nts_ke_exchange(&self.ke_server, self.ke_port)?;
        self.c2s_key = result.c2s_key;
        self.s2c_key = result.s2c_key;
        self.aead_algorithm = result.aead_algorithm;
        self.cookie_jar.replace(result.cookies);
        self.ntp_server = result.ntp_server;
        self.ntp_port = result.ntp_port;
        Ok(())
    }

    /// Get the effective NTP server hostname.
    pub fn effective_ntp_server(&self) -> &str {
        self.ntp_server.as_deref().unwrap_or(&self.ke_server)
    }

    /// Get the effective NTP port.
    pub fn effective_ntp_port(&self) -> u16 {
        self.ntp_port.unwrap_or(123)
    }

    /// Whether we have cookies available for NTP queries.
    pub fn has_cookies(&self) -> bool {
        !self.cookie_jar.is_empty()
    }

    /// Take a cookie and build NTS extension fields for an NTP request.
    ///
    /// Returns `(unique_id, extension_field_bytes)`.
    pub fn build_request_extensions(
        &mut self,
        ntp_header: &[u8],
    ) -> Result<(Vec<u8>, Vec<u8>), NtsError> {
        let cookie = self.cookie_jar.take().ok_or(NtsError::NoCookies)?;

        // Request enough cookie placeholders to replenish toward MAX_COOKIES
        let wanted = MAX_COOKIES.saturating_sub(self.cookie_jar.len() + 1);
        // Limit to a reasonable number per query
        let placeholders = wanted.min(7);

        build_nts_request_extensions(&self.c2s_key, &cookie, ntp_header, placeholders)
    }

    /// Verify an NTS-protected NTP response and replenish cookies.
    pub fn verify_response(
        &mut self,
        ntp_packet: &[u8],
        expected_unique_id: &[u8],
    ) -> Result<(), NtsError> {
        let response_data = verify_nts_response(&self.s2c_key, ntp_packet, expected_unique_id)?;

        // Add new cookies to the jar
        for cookie in response_data.new_cookies {
            self.cookie_jar.add(cookie);
        }

        log::debug!(
            "NTS: verified response, {} cookie(s) in jar",
            self.cookie_jar.len()
        );

        Ok(())
    }
}

// ── NTS-protected SNTP query ───────────────────────────────────────────────

use std::net::UdpSocket;

/// Perform an NTS-protected SNTP query.
///
/// This is the NTS equivalent of the plain `sntp_query()` function.
/// Returns `(ntp_packet_bytes, origin_ts, t4)` similar to the plain version,
/// but with NTS authentication.
pub fn nts_sntp_query(
    addr: SocketAddr,
    nts_state: &mut NtsState,
) -> Result<(Vec<u8>, super::NtpTimestamp, std::time::SystemTime), NtsError> {
    use std::time::SystemTime;

    let bind_addr: SocketAddr = if addr.is_ipv4() {
        "0.0.0.0:0".parse().unwrap()
    } else {
        "[::]:0".parse().unwrap()
    };

    let socket = UdpSocket::bind(bind_addr)?;
    socket.set_read_timeout(Some(Duration::from_secs(5)))?;
    socket.set_write_timeout(Some(Duration::from_secs(5)))?;

    // Build standard NTP client request header (48 bytes)
    let request = super::NtpPacket::new_client_request();
    let origin_ts = request.transmit_ts;
    let ntp_header = request.to_bytes();

    // Build NTS extension fields
    let (unique_id, ext_bytes) = nts_state.build_request_extensions(&ntp_header)?;

    // Combine NTP header + extension fields
    let mut packet = Vec::with_capacity(ntp_header.len() + ext_bytes.len());
    packet.extend_from_slice(&ntp_header);
    packet.extend_from_slice(&ext_bytes);

    socket.send_to(&packet, addr)?;

    let mut buf = [0u8; 4096]; // NTS responses can be larger due to extension fields
    let (n, _from) = socket.recv_from(&mut buf)?;

    let t4 = SystemTime::now();

    if n < 48 {
        return Err(NtsError::ExtensionField("NTP response too short".into()));
    }

    // Verify NTS authentication
    nts_state.verify_response(&buf[..n], &unique_id)?;

    Ok((buf[..n].to_vec(), origin_ts, t4))
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── NTS-KE record serialization/parsing ────────────────────────────────

    #[test]
    fn test_nts_ke_record_roundtrip() {
        let record = NtsKeRecord::new(true, NtsKeRecordType::NextProtocol, vec![0x00, 0x00]);
        let bytes = record.to_bytes();
        let (parsed, consumed) = NtsKeRecord::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, bytes.len());
        assert!(parsed.critical);
        assert_eq!(parsed.record_type, NtsKeRecordType::NextProtocol as u16);
        assert_eq!(parsed.body, vec![0x00, 0x00]);
    }

    #[test]
    fn test_nts_ke_record_non_critical() {
        let record = NtsKeRecord {
            critical: false,
            record_type: NtsKeRecordType::Warning as u16,
            body: vec![0x00, 0x01],
        };
        let bytes = record.to_bytes();
        // First two bytes should NOT have the critical bit set
        assert_eq!(bytes[0] & 0x80, 0x00);
        let (parsed, _) = NtsKeRecord::from_bytes(&bytes).unwrap();
        assert!(!parsed.critical);
        assert_eq!(parsed.record_type, NtsKeRecordType::Warning as u16);
    }

    #[test]
    fn test_nts_ke_record_critical_bit() {
        let record = NtsKeRecord::new(true, NtsKeRecordType::AeadAlgorithm, vec![0x00, 0x0F]);
        let bytes = record.to_bytes();
        // First byte should have critical bit set
        assert_ne!(bytes[0] & 0x80, 0x00);
    }

    #[test]
    fn test_nts_ke_record_empty_body() {
        let record = NtsKeRecord::new(true, NtsKeRecordType::EndOfMessage, Vec::new());
        let bytes = record.to_bytes();
        assert_eq!(bytes.len(), 4); // just header
        let (parsed, consumed) = NtsKeRecord::from_bytes(&bytes).unwrap();
        assert_eq!(consumed, 4);
        assert!(parsed.body.is_empty());
        assert_eq!(parsed.record_type, NtsKeRecordType::EndOfMessage as u16);
    }

    #[test]
    fn test_nts_ke_record_from_bytes_too_short() {
        let result = NtsKeRecord::from_bytes(&[0x80, 0x01]);
        assert!(result.is_err());
    }

    #[test]
    fn test_nts_ke_record_from_bytes_body_truncated() {
        // Header says body is 10 bytes but only 2 are provided
        let data = [0x80, 0x01, 0x00, 0x0A, 0x00, 0x00];
        let result = NtsKeRecord::from_bytes(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_nts_ke_record_type_from_u16() {
        assert_eq!(
            NtsKeRecordType::from_u16(0),
            Some(NtsKeRecordType::EndOfMessage)
        );
        assert_eq!(
            NtsKeRecordType::from_u16(1),
            Some(NtsKeRecordType::NextProtocol)
        );
        assert_eq!(NtsKeRecordType::from_u16(2), Some(NtsKeRecordType::Error));
        assert_eq!(NtsKeRecordType::from_u16(3), Some(NtsKeRecordType::Warning));
        assert_eq!(
            NtsKeRecordType::from_u16(4),
            Some(NtsKeRecordType::AeadAlgorithm)
        );
        assert_eq!(
            NtsKeRecordType::from_u16(5),
            Some(NtsKeRecordType::NewCookieForNtpv4)
        );
        assert_eq!(
            NtsKeRecordType::from_u16(6),
            Some(NtsKeRecordType::Ntpv4Server)
        );
        assert_eq!(
            NtsKeRecordType::from_u16(7),
            Some(NtsKeRecordType::Ntpv4Port)
        );
        assert_eq!(NtsKeRecordType::from_u16(99), None);
    }

    // ── NTS-KE request building ────────────────────────────────────────────

    #[test]
    fn test_build_nts_ke_request() {
        let request = build_nts_ke_request();
        let records = parse_nts_ke_records(&request).unwrap();
        assert_eq!(records.len(), 3);

        // First: Next Protocol
        assert_eq!(records[0].record_type, NtsKeRecordType::NextProtocol as u16);
        assert!(records[0].critical);
        assert_eq!(records[0].body, NTS_NEXT_PROTOCOL_NTPV4.to_be_bytes());

        // Second: AEAD Algorithm
        assert_eq!(
            records[1].record_type,
            NtsKeRecordType::AeadAlgorithm as u16
        );
        assert!(records[1].critical);
        assert_eq!(records[1].body, AEAD_AES_SIV_CMAC_256.to_be_bytes());

        // Third: End of Message
        assert_eq!(records[2].record_type, NtsKeRecordType::EndOfMessage as u16);
        assert!(records[2].critical);
        assert!(records[2].body.is_empty());
    }

    // ── NTS-KE response parsing ────────────────────────────────────────────

    #[test]
    fn test_parse_nts_ke_records_multi() {
        // Build a mock server response
        let mut data = Vec::new();

        // Next Protocol
        let np = NtsKeRecord::new(true, NtsKeRecordType::NextProtocol, vec![0x00, 0x00]);
        data.extend_from_slice(&np.to_bytes());

        // AEAD
        let ae = NtsKeRecord::new(
            true,
            NtsKeRecordType::AeadAlgorithm,
            AEAD_AES_SIV_CMAC_256.to_be_bytes().to_vec(),
        );
        data.extend_from_slice(&ae.to_bytes());

        // Cookie
        let cookie = NtsKeRecord::new(
            false,
            NtsKeRecordType::NewCookieForNtpv4,
            vec![0xDE, 0xAD, 0xBE, 0xEF],
        );
        data.extend_from_slice(&cookie.to_bytes());

        // End
        let eom = NtsKeRecord::new(true, NtsKeRecordType::EndOfMessage, Vec::new());
        data.extend_from_slice(&eom.to_bytes());

        let records = parse_nts_ke_records(&data).unwrap();
        assert_eq!(records.len(), 4);
        assert_eq!(records[2].body, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    }

    #[test]
    fn test_parse_nts_ke_records_stops_at_eom() {
        let mut data = Vec::new();
        let eom = NtsKeRecord::new(true, NtsKeRecordType::EndOfMessage, Vec::new());
        data.extend_from_slice(&eom.to_bytes());
        // Extra garbage after EOM
        data.extend_from_slice(&[0xFF; 20]);

        let records = parse_nts_ke_records(&data).unwrap();
        assert_eq!(records.len(), 1);
    }

    // ── has_end_of_message ─────────────────────────────────────────────────

    #[test]
    fn test_has_end_of_message_true() {
        let eom = NtsKeRecord::new(true, NtsKeRecordType::EndOfMessage, Vec::new());
        let data = eom.to_bytes();
        assert!(has_end_of_message(&data));
    }

    #[test]
    fn test_has_end_of_message_false() {
        let np = NtsKeRecord::new(true, NtsKeRecordType::NextProtocol, vec![0x00, 0x00]);
        let data = np.to_bytes();
        assert!(!has_end_of_message(&data));
    }

    #[test]
    fn test_has_end_of_message_empty() {
        assert!(!has_end_of_message(&[]));
    }

    #[test]
    fn test_has_end_of_message_after_records() {
        let mut data = Vec::new();
        let np = NtsKeRecord::new(true, NtsKeRecordType::NextProtocol, vec![0x00, 0x00]);
        data.extend_from_slice(&np.to_bytes());
        let ae = NtsKeRecord::new(true, NtsKeRecordType::AeadAlgorithm, vec![0x00, 0x0F]);
        data.extend_from_slice(&ae.to_bytes());
        assert!(!has_end_of_message(&data));

        let eom = NtsKeRecord::new(true, NtsKeRecordType::EndOfMessage, Vec::new());
        data.extend_from_slice(&eom.to_bytes());
        assert!(has_end_of_message(&data));
    }

    // ── NTP Extension Field serialization/parsing ──────────────────────────

    #[test]
    fn test_extension_field_roundtrip() {
        let ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: vec![1, 2, 3, 4, 5, 6, 7, 8],
        };
        let bytes = ef.to_bytes();
        // Length should include header (4) + value (8) = 12
        assert_eq!(bytes.len(), 12);
        // Type
        assert_eq!(u16::from_be_bytes([bytes[0], bytes[1]]), EXT_TYPE_UNIQUE_ID);
        // Length
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 12);

        let parsed = NtpExtensionField::parse_all(&bytes).unwrap();
        assert_eq!(parsed.len(), 1);
        assert_eq!(parsed[0].field_type, EXT_TYPE_UNIQUE_ID);
        assert_eq!(parsed[0].value, vec![1, 2, 3, 4, 5, 6, 7, 8]);
    }

    #[test]
    fn test_extension_field_padding() {
        // Value of 5 bytes should be padded to 8 (next multiple of 4)
        let ef = NtpExtensionField {
            field_type: 0x1234,
            value: vec![1, 2, 3, 4, 5],
        };
        let bytes = ef.to_bytes();
        // 4 header + 8 padded value = 12
        assert_eq!(bytes.len(), 12);
        // Length field is total: 12
        assert_eq!(u16::from_be_bytes([bytes[2], bytes[3]]), 12);
        // Padding bytes should be zero
        assert_eq!(bytes[9], 0);
        assert_eq!(bytes[10], 0);
        assert_eq!(bytes[11], 0);
    }

    #[test]
    fn test_extension_field_parse_multiple() {
        let ef1 = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: vec![0xAA; 8],
        };
        let ef2 = NtpExtensionField {
            field_type: EXT_TYPE_NTS_COOKIE,
            value: vec![0xBB; 16],
        };

        let mut data = Vec::new();
        data.extend_from_slice(&ef1.to_bytes());
        data.extend_from_slice(&ef2.to_bytes());

        let parsed = NtpExtensionField::parse_all(&data).unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].field_type, EXT_TYPE_UNIQUE_ID);
        assert_eq!(parsed[0].value.len(), 8);
        assert_eq!(parsed[1].field_type, EXT_TYPE_NTS_COOKIE);
        assert_eq!(parsed[1].value.len(), 16);
    }

    #[test]
    fn test_extension_field_parse_empty() {
        let parsed = NtpExtensionField::parse_all(&[]).unwrap();
        assert!(parsed.is_empty());
    }

    #[test]
    fn test_extension_field_parse_truncated() {
        // Header says 100 bytes but only 20 available
        let mut data = [0u8; 20];
        data[0] = 0x01;
        data[1] = 0x04; // type
        data[2] = 0x00;
        data[3] = 100; // length = 100
        let result = NtpExtensionField::parse_all(&data);
        assert!(result.is_err());
    }

    #[test]
    fn test_extension_field_parse_length_too_small() {
        // Length < 4 is invalid
        let data = [0x01, 0x04, 0x00, 0x02]; // length = 2
        let result = NtpExtensionField::parse_all(&data);
        assert!(result.is_err());
    }

    // ── AEAD encrypt/decrypt ───────────────────────────────────────────────

    #[test]
    fn test_aead_encrypt_decrypt_roundtrip() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let nonce = [0x01u8; SIV_TAG_LEN];
        let aad = b"additional authenticated data";
        let plaintext = b"hello NTS world!";

        let encrypted = nts_aead_encrypt(&key, &nonce, aad, plaintext).unwrap();
        // encrypted = nonce || ciphertext
        assert!(encrypted.len() > nonce.len());

        let ciphertext = &encrypted[nonce.len()..];
        let decrypted = nts_aead_decrypt(&key, &nonce, aad, ciphertext).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_aead_encrypt_empty_plaintext() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let nonce = [0x01u8; SIV_TAG_LEN];
        let aad = b"NTP header here";

        let encrypted = nts_aead_encrypt(&key, &nonce, aad, &[]).unwrap();
        let ciphertext = &encrypted[nonce.len()..];
        // SIV tag is always present even for empty plaintext
        assert_eq!(ciphertext.len(), SIV_TAG_LEN);

        let decrypted = nts_aead_decrypt(&key, &nonce, aad, ciphertext).unwrap();
        assert!(decrypted.is_empty());
    }

    #[test]
    fn test_aead_decrypt_wrong_key() {
        let key1 = [0x42u8; AEAD_KEY_LEN];
        let key2 = [0x43u8; AEAD_KEY_LEN];
        let nonce = [0x01u8; SIV_TAG_LEN];
        let aad = b"test aad";
        let plaintext = b"secret";

        let encrypted = nts_aead_encrypt(&key1, &nonce, aad, plaintext).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        let result = nts_aead_decrypt(&key2, &nonce, aad, ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_aead_decrypt_wrong_aad() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let nonce = [0x01u8; SIV_TAG_LEN];
        let aad1 = b"correct aad";
        let aad2 = b"wrong aad";
        let plaintext = b"secret";

        let encrypted = nts_aead_encrypt(&key, &nonce, aad1, plaintext).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        let result = nts_aead_decrypt(&key, &nonce, aad2, ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_aead_decrypt_tampered_ciphertext() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let nonce = [0x01u8; SIV_TAG_LEN];
        let aad = b"test aad";
        let plaintext = b"hello";

        let encrypted = nts_aead_encrypt(&key, &nonce, aad, plaintext).unwrap();
        let mut ciphertext = encrypted[nonce.len()..].to_vec();
        // Flip a bit
        if !ciphertext.is_empty() {
            ciphertext[0] ^= 0x01;
        }

        let result = nts_aead_decrypt(&key, &nonce, aad, &ciphertext);
        assert!(result.is_err());
    }

    #[test]
    fn test_aead_invalid_key_length() {
        let key = [0x42u8; 16]; // wrong length
        let nonce = [0x01u8; SIV_TAG_LEN];

        let result = nts_aead_encrypt(&key, &nonce, b"", b"");
        assert!(result.is_err());
        match result.unwrap_err() {
            NtsError::Aead(msg) => assert!(msg.contains("key length")),
            other => panic!("expected Aead error, got: {other}"),
        }
    }

    // ── Cookie jar ─────────────────────────────────────────────────────────

    #[test]
    fn test_cookie_jar_new() {
        let jar = NtsCookieJar::new(vec![vec![1, 2, 3], vec![4, 5, 6]]);
        assert_eq!(jar.len(), 2);
        assert!(!jar.is_empty());
    }

    #[test]
    fn test_cookie_jar_take() {
        let mut jar = NtsCookieJar::new(vec![vec![1], vec![2]]);
        let c1 = jar.take().unwrap();
        assert_eq!(c1, vec![2]); // LIFO order (pop from end)
        let c2 = jar.take().unwrap();
        assert_eq!(c2, vec![1]);
        assert!(jar.take().is_none());
        assert!(jar.is_empty());
    }

    #[test]
    fn test_cookie_jar_add() {
        let mut jar = NtsCookieJar::new(Vec::new());
        assert!(jar.is_empty());

        jar.add(vec![0xAA]);
        assert_eq!(jar.len(), 1);

        jar.add(vec![0xBB]);
        assert_eq!(jar.len(), 2);
    }

    #[test]
    fn test_cookie_jar_max_cookies() {
        let mut jar = NtsCookieJar::new(Vec::new());
        for i in 0..MAX_COOKIES + 5 {
            jar.add(vec![i as u8]);
        }
        // Should be capped at MAX_COOKIES
        assert_eq!(jar.len(), MAX_COOKIES);
    }

    #[test]
    fn test_cookie_jar_replace() {
        let mut jar = NtsCookieJar::new(vec![vec![1], vec![2]]);
        jar.replace(vec![vec![10], vec![20], vec![30]]);
        assert_eq!(jar.len(), 3);
        assert_eq!(jar.take().unwrap(), vec![30]);
    }

    // ── NTS request extension field building ───────────────────────────────

    #[test]
    fn test_build_nts_request_extensions_basic() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let cookie = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];
        let ntp_header = [0u8; 48]; // dummy header

        let (unique_id, ext_bytes) =
            build_nts_request_extensions(&key, &cookie, &ntp_header, 0).unwrap();

        assert_eq!(unique_id.len(), UNIQUE_ID_LEN);

        // Parse the generated extension fields
        let fields = NtpExtensionField::parse_all(&ext_bytes).unwrap();
        assert!(fields.len() >= 3); // UID + Cookie + Authenticator

        // First field: Unique Identifier
        assert_eq!(fields[0].field_type, EXT_TYPE_UNIQUE_ID);
        assert_eq!(fields[0].value, unique_id);

        // Second field: NTS Cookie
        assert_eq!(fields[1].field_type, EXT_TYPE_NTS_COOKIE);
        assert_eq!(fields[1].value, cookie);

        // Last field: NTS Authenticator
        let last = fields.last().unwrap();
        assert_eq!(last.field_type, EXT_TYPE_NTS_AUTHENTICATOR);
    }

    #[test]
    fn test_build_nts_request_extensions_with_placeholders() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let cookie = vec![0xAA; 32];
        let ntp_header = [0u8; 48];

        let (_, ext_bytes) = build_nts_request_extensions(&key, &cookie, &ntp_header, 3).unwrap();

        let fields = NtpExtensionField::parse_all(&ext_bytes).unwrap();
        // UID + Cookie + 3 Placeholders + Authenticator = 6
        assert_eq!(fields.len(), 6);

        assert_eq!(fields[0].field_type, EXT_TYPE_UNIQUE_ID);
        assert_eq!(fields[1].field_type, EXT_TYPE_NTS_COOKIE);
        assert_eq!(fields[2].field_type, EXT_TYPE_NTS_COOKIE_PLACEHOLDER);
        assert_eq!(fields[3].field_type, EXT_TYPE_NTS_COOKIE_PLACEHOLDER);
        assert_eq!(fields[4].field_type, EXT_TYPE_NTS_COOKIE_PLACEHOLDER);
        assert_eq!(fields[5].field_type, EXT_TYPE_NTS_AUTHENTICATOR);

        // Placeholders should be same length as cookie
        assert_eq!(fields[2].value.len(), cookie.len());
    }

    #[test]
    fn test_build_nts_request_unique_ids_differ() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let cookie = vec![0xAA; 16];
        let ntp_header = [0u8; 48];

        let (uid1, _) = build_nts_request_extensions(&key, &cookie, &ntp_header, 0).unwrap();
        let (uid2, _) = build_nts_request_extensions(&key, &cookie, &ntp_header, 0).unwrap();

        // Two calls should produce different unique IDs (random)
        assert_ne!(uid1, uid2);
    }

    // ── NTS response verification ──────────────────────────────────────────

    #[test]
    fn test_verify_nts_response_missing_header() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let short_packet = [0u8; 20]; // too short for NTP header
        let result = verify_nts_response(&key, &short_packet, &[0u8; 32]);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_nts_response_no_extensions() {
        let key = [0x42u8; AEAD_KEY_LEN];
        let packet = [0u8; 48]; // NTP header only, no extensions
        let result = verify_nts_response(&key, &packet, &[0u8; 32]);
        assert!(result.is_err()); // missing Unique Identifier
    }

    #[test]
    fn test_verify_nts_response_uid_mismatch() {
        let _c2s_key = [0x42u8; AEAD_KEY_LEN];
        let s2c_key = [0x43u8; AEAD_KEY_LEN];
        let ntp_header = [0u8; 48];

        // Build a "response" with wrong unique ID
        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: vec![0xFF; UNIQUE_ID_LEN],
        };
        // Add a dummy authenticator
        let mut aad = Vec::new();
        aad.extend_from_slice(&ntp_header);
        aad.extend_from_slice(&uid_ef.to_bytes());
        let nonce = [0x01u8; SIV_TAG_LEN];
        let encrypted = nts_aead_encrypt(&s2c_key, &nonce, &aad, &[]).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        let mut auth_value = Vec::new();
        auth_value.extend_from_slice(&(SIV_TAG_LEN as u16).to_be_bytes());
        auth_value.extend_from_slice(&nonce);
        auth_value.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
        auth_value.extend_from_slice(ciphertext);
        let auth_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_AUTHENTICATOR,
            value: auth_value,
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&ntp_header);
        packet.extend_from_slice(&uid_ef.to_bytes());
        packet.extend_from_slice(&auth_ef.to_bytes());

        let expected_uid = vec![0xAA; UNIQUE_ID_LEN]; // different!
        let result = verify_nts_response(&s2c_key, &packet, &expected_uid);
        assert!(result.is_err());
        match result.unwrap_err() {
            NtsError::AuthFailed(msg) => assert!(msg.contains("mismatch")),
            other => panic!("expected AuthFailed, got: {other}"),
        }
    }

    #[test]
    fn test_verify_nts_response_valid_empty_encrypted() {
        // Build a valid NTS-protected response with empty encrypted extension fields
        let s2c_key = [0x43u8; AEAD_KEY_LEN];
        let ntp_header = [0u8; 48];
        let unique_id = vec![0xBB; UNIQUE_ID_LEN];

        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: unique_id.clone(),
        };

        // Build AAD
        let mut aad = Vec::new();
        aad.extend_from_slice(&ntp_header);
        aad.extend_from_slice(&uid_ef.to_bytes());

        // Encrypt empty plaintext
        let nonce = [0x07u8; SIV_TAG_LEN];
        let encrypted = nts_aead_encrypt(&s2c_key, &nonce, &aad, &[]).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        // Build authenticator
        let mut auth_value = Vec::new();
        auth_value.extend_from_slice(&(SIV_TAG_LEN as u16).to_be_bytes());
        auth_value.extend_from_slice(&nonce);
        auth_value.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
        auth_value.extend_from_slice(ciphertext);
        let auth_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_AUTHENTICATOR,
            value: auth_value,
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&ntp_header);
        packet.extend_from_slice(&uid_ef.to_bytes());
        packet.extend_from_slice(&auth_ef.to_bytes());

        let result = verify_nts_response(&s2c_key, &packet, &unique_id).unwrap();
        assert_eq!(result.unique_id, unique_id);
        assert!(result.new_cookies.is_empty());
        assert!(result.decrypted_extensions.is_empty());
    }

    #[test]
    fn test_verify_nts_response_with_encrypted_cookies() {
        let s2c_key = [0x43u8; AEAD_KEY_LEN];
        let ntp_header = [0u8; 48];
        let unique_id = vec![0xCC; UNIQUE_ID_LEN];

        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: unique_id.clone(),
        };

        let mut aad = Vec::new();
        aad.extend_from_slice(&ntp_header);
        aad.extend_from_slice(&uid_ef.to_bytes());

        // Build encrypted extension fields containing new cookies
        let new_cookie = NtpExtensionField {
            field_type: EXT_TYPE_NTS_COOKIE,
            value: vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04],
        };
        let encrypted_plaintext = new_cookie.to_bytes();

        let nonce = [0x09u8; SIV_TAG_LEN];
        let encrypted = nts_aead_encrypt(&s2c_key, &nonce, &aad, &encrypted_plaintext).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        let mut auth_value = Vec::new();
        auth_value.extend_from_slice(&(SIV_TAG_LEN as u16).to_be_bytes());
        auth_value.extend_from_slice(&nonce);
        auth_value.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
        auth_value.extend_from_slice(ciphertext);
        let auth_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_AUTHENTICATOR,
            value: auth_value,
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&ntp_header);
        packet.extend_from_slice(&uid_ef.to_bytes());
        packet.extend_from_slice(&auth_ef.to_bytes());

        let result = verify_nts_response(&s2c_key, &packet, &unique_id).unwrap();
        assert_eq!(result.new_cookies.len(), 1);
        assert_eq!(
            result.new_cookies[0],
            vec![0xDE, 0xAD, 0xBE, 0xEF, 0x01, 0x02, 0x03, 0x04]
        );
    }

    #[test]
    fn test_verify_nts_response_wrong_s2c_key() {
        let s2c_key_good = [0x43u8; AEAD_KEY_LEN];
        let s2c_key_bad = [0x44u8; AEAD_KEY_LEN];
        let ntp_header = [0u8; 48];
        let unique_id = vec![0xDD; UNIQUE_ID_LEN];

        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: unique_id.clone(),
        };

        let mut aad = Vec::new();
        aad.extend_from_slice(&ntp_header);
        aad.extend_from_slice(&uid_ef.to_bytes());

        let nonce = [0x0Au8; SIV_TAG_LEN];
        let encrypted = nts_aead_encrypt(&s2c_key_good, &nonce, &aad, &[]).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        let mut auth_value = Vec::new();
        auth_value.extend_from_slice(&(SIV_TAG_LEN as u16).to_be_bytes());
        auth_value.extend_from_slice(&nonce);
        auth_value.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
        auth_value.extend_from_slice(ciphertext);
        let auth_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_AUTHENTICATOR,
            value: auth_value,
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&ntp_header);
        packet.extend_from_slice(&uid_ef.to_bytes());
        packet.extend_from_slice(&auth_ef.to_bytes());

        // Should fail with wrong key
        let result = verify_nts_response(&s2c_key_bad, &packet, &unique_id);
        assert!(result.is_err());
    }

    // ── NtsState ───────────────────────────────────────────────────────────

    #[test]
    fn test_nts_state_effective_server_default() {
        let state = NtsState {
            c2s_key: vec![0; AEAD_KEY_LEN],
            s2c_key: vec![0; AEAD_KEY_LEN],
            aead_algorithm: AEAD_AES_SIV_CMAC_256,
            cookie_jar: NtsCookieJar::new(vec![vec![0xAA; 16]]),
            ke_server: "ke.example.com".to_string(),
            ke_port: NTS_KE_PORT,
            ntp_server: None,
            ntp_port: None,
        };
        assert_eq!(state.effective_ntp_server(), "ke.example.com");
        assert_eq!(state.effective_ntp_port(), 123);
    }

    #[test]
    fn test_nts_state_effective_server_negotiated() {
        let state = NtsState {
            c2s_key: vec![0; AEAD_KEY_LEN],
            s2c_key: vec![0; AEAD_KEY_LEN],
            aead_algorithm: AEAD_AES_SIV_CMAC_256,
            cookie_jar: NtsCookieJar::new(Vec::new()),
            ke_server: "ke.example.com".to_string(),
            ke_port: NTS_KE_PORT,
            ntp_server: Some("ntp.example.com".to_string()),
            ntp_port: Some(4123),
        };
        assert_eq!(state.effective_ntp_server(), "ntp.example.com");
        assert_eq!(state.effective_ntp_port(), 4123);
    }

    #[test]
    fn test_nts_state_has_cookies() {
        let mut state = NtsState {
            c2s_key: vec![0; AEAD_KEY_LEN],
            s2c_key: vec![0; AEAD_KEY_LEN],
            aead_algorithm: AEAD_AES_SIV_CMAC_256,
            cookie_jar: NtsCookieJar::new(vec![vec![1], vec![2]]),
            ke_server: "test".to_string(),
            ke_port: NTS_KE_PORT,
            ntp_server: None,
            ntp_port: None,
        };
        assert!(state.has_cookies());
        state.cookie_jar.take();
        state.cookie_jar.take();
        assert!(!state.has_cookies());
    }

    #[test]
    fn test_nts_state_build_and_verify_roundtrip() {
        let c2s_key = vec![0x42u8; AEAD_KEY_LEN];
        let s2c_key = vec![0x43u8; AEAD_KEY_LEN];
        let cookie = vec![0xDE, 0xAD, 0xBE, 0xEF, 0xCA, 0xFE, 0xBA, 0xBE];

        let mut state = NtsState {
            c2s_key: c2s_key.clone(),
            s2c_key: s2c_key.clone(),
            aead_algorithm: AEAD_AES_SIV_CMAC_256,
            cookie_jar: NtsCookieJar::new(vec![cookie.clone()]),
            ke_server: "test".to_string(),
            ke_port: NTS_KE_PORT,
            ntp_server: None,
            ntp_port: None,
        };

        // Build NTP header
        let ntp_header = [0u8; 48];

        // Build request extensions
        let (unique_id, ext_bytes) = state.build_request_extensions(&ntp_header).unwrap();
        assert!(!unique_id.is_empty());
        assert!(!ext_bytes.is_empty());

        // Cookie should be consumed
        assert!(state.cookie_jar.is_empty());

        // Now simulate a server response: we need to build a valid one
        // with the server's s2c_key
        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: unique_id.clone(),
        };
        // Server provides a new cookie in encrypted extensions
        let new_cookie_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_COOKIE,
            value: vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88],
        };
        let encrypted_plaintext = new_cookie_ef.to_bytes();

        let mut aad = Vec::new();
        aad.extend_from_slice(&ntp_header);
        aad.extend_from_slice(&uid_ef.to_bytes());

        let nonce = [0x0Bu8; SIV_TAG_LEN];
        let encrypted = nts_aead_encrypt(&s2c_key, &nonce, &aad, &encrypted_plaintext).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        let mut auth_value = Vec::new();
        auth_value.extend_from_slice(&(SIV_TAG_LEN as u16).to_be_bytes());
        auth_value.extend_from_slice(&nonce);
        auth_value.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
        auth_value.extend_from_slice(ciphertext);
        let auth_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_AUTHENTICATOR,
            value: auth_value,
        };

        let mut response_packet = Vec::new();
        response_packet.extend_from_slice(&ntp_header);
        response_packet.extend_from_slice(&uid_ef.to_bytes());
        response_packet.extend_from_slice(&auth_ef.to_bytes());

        // Verify the response
        state.verify_response(&response_packet, &unique_id).unwrap();

        // Cookie jar should now have the new cookie
        assert_eq!(state.cookie_jar.len(), 1);
        let replenished = state.cookie_jar.take().unwrap();
        assert_eq!(
            replenished,
            vec![0x11, 0x22, 0x33, 0x44, 0x55, 0x66, 0x77, 0x88]
        );
    }

    #[test]
    fn test_nts_state_no_cookies_error() {
        let mut state = NtsState {
            c2s_key: vec![0; AEAD_KEY_LEN],
            s2c_key: vec![0; AEAD_KEY_LEN],
            aead_algorithm: AEAD_AES_SIV_CMAC_256,
            cookie_jar: NtsCookieJar::new(Vec::new()), // empty!
            ke_server: "test".to_string(),
            ke_port: NTS_KE_PORT,
            ntp_server: None,
            ntp_port: None,
        };

        let result = state.build_request_extensions(&[0u8; 48]);
        assert!(result.is_err());
        match result.unwrap_err() {
            NtsError::NoCookies => {}
            other => panic!("expected NoCookies, got: {other}"),
        }
    }

    // ── NtsError display ───────────────────────────────────────────────────

    #[test]
    fn test_nts_error_display() {
        let e = NtsError::Tls("test".into());
        assert!(e.to_string().contains("TLS"));

        let e = NtsError::KeProtocol("bad".into());
        assert!(e.to_string().contains("protocol"));

        let e = NtsError::KeServerError(42);
        assert!(e.to_string().contains("42"));

        let e = NtsError::Aead("decrypt".into());
        assert!(e.to_string().contains("AEAD"));

        let e = NtsError::NoCookies;
        assert!(e.to_string().contains("cookie"));

        let e = NtsError::ExtensionField("parse".into());
        assert!(e.to_string().contains("extension"));

        let e = NtsError::Io(io::Error::other("test"));
        assert!(e.to_string().contains("I/O"));

        let e = NtsError::AuthFailed("mismatch".into());
        assert!(e.to_string().contains("authentication"));
    }

    #[test]
    fn test_nts_error_from_io() {
        let io_err = io::Error::new(io::ErrorKind::ConnectionRefused, "refused");
        let nts_err: NtsError = io_err.into();
        match nts_err {
            NtsError::Io(e) => assert_eq!(e.kind(), io::ErrorKind::ConnectionRefused),
            other => panic!("expected Io, got: {other}"),
        }
    }

    // ── Constants sanity checks ────────────────────────────────────────────

    #[test]
    fn test_nts_constants() {
        assert_eq!(NTS_KE_PORT, 4460);
        assert_eq!(NTS_KE_ALPN, b"ntske/1");
        assert_eq!(NTS_NEXT_PROTOCOL_NTPV4, 0);
        assert_eq!(AEAD_AES_SIV_CMAC_256, 15);
        assert_eq!(AEAD_KEY_LEN, 32);
        assert_eq!(SIV_TAG_LEN, 16);
        assert_eq!(EXT_TYPE_UNIQUE_ID, 0x0104);
        assert_eq!(EXT_TYPE_NTS_COOKIE, 0x0204);
        assert_eq!(EXT_TYPE_NTS_COOKIE_PLACEHOLDER, 0x0304);
        assert_eq!(EXT_TYPE_NTS_AUTHENTICATOR, 0x0404);
        assert_eq!(UNIQUE_ID_LEN, 32);
        assert_eq!(MAX_COOKIES, 8);
    }

    #[test]
    fn test_nts_tls_exporter_label() {
        assert_eq!(NTS_TLS_EXPORTER_LABEL, "EXPORTER-network-time-security");
    }

    #[test]
    fn test_nts_exporter_context_bytes() {
        assert_eq!(NTS_EXPORTER_CONTEXT_C2S, 0x00);
        assert_eq!(NTS_EXPORTER_CONTEXT_S2C, 0x01);
    }

    // ── Extension field type values per RFC 8915 ───────────────────────────

    #[test]
    fn test_extension_field_types_rfc8915() {
        // Unique Identifier: Field Type 0x0104
        assert_eq!(EXT_TYPE_UNIQUE_ID, 0x0104);
        // NTS Cookie: Field Type 0x0204
        assert_eq!(EXT_TYPE_NTS_COOKIE, 0x0204);
        // NTS Cookie Placeholder: Field Type 0x0304
        assert_eq!(EXT_TYPE_NTS_COOKIE_PLACEHOLDER, 0x0304);
        // NTS Authenticator and Encrypted Extension Fields: Field Type 0x0404
        assert_eq!(EXT_TYPE_NTS_AUTHENTICATOR, 0x0404);
    }

    // ── Multiple cookies in encrypted response ─────────────────────────────

    #[test]
    fn test_verify_nts_response_multiple_encrypted_cookies() {
        let s2c_key = [0x50u8; AEAD_KEY_LEN];
        let ntp_header = [0u8; 48];
        let unique_id = vec![0xEE; UNIQUE_ID_LEN];

        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: unique_id.clone(),
        };

        let mut aad = Vec::new();
        aad.extend_from_slice(&ntp_header);
        aad.extend_from_slice(&uid_ef.to_bytes());

        // Build encrypted extension fields with 3 new cookies
        let mut encrypted_plaintext = Vec::new();
        for i in 0..3u8 {
            let cookie_ef = NtpExtensionField {
                field_type: EXT_TYPE_NTS_COOKIE,
                value: vec![i + 1; 8],
            };
            encrypted_plaintext.extend_from_slice(&cookie_ef.to_bytes());
        }

        let nonce = [0x0Cu8; SIV_TAG_LEN];
        let encrypted = nts_aead_encrypt(&s2c_key, &nonce, &aad, &encrypted_plaintext).unwrap();
        let ciphertext = &encrypted[nonce.len()..];

        let mut auth_value = Vec::new();
        auth_value.extend_from_slice(&(SIV_TAG_LEN as u16).to_be_bytes());
        auth_value.extend_from_slice(&nonce);
        auth_value.extend_from_slice(&(ciphertext.len() as u16).to_be_bytes());
        auth_value.extend_from_slice(ciphertext);
        let auth_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_AUTHENTICATOR,
            value: auth_value,
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&ntp_header);
        packet.extend_from_slice(&uid_ef.to_bytes());
        packet.extend_from_slice(&auth_ef.to_bytes());

        let result = verify_nts_response(&s2c_key, &packet, &unique_id).unwrap();
        assert_eq!(result.new_cookies.len(), 3);
        assert_eq!(result.new_cookies[0], vec![1u8; 8]);
        assert_eq!(result.new_cookies[1], vec![2u8; 8]);
        assert_eq!(result.new_cookies[2], vec![3u8; 8]);
    }

    // ── Authenticator parsing edge cases ───────────────────────────────────

    #[test]
    fn test_verify_nts_response_authenticator_too_short() {
        let s2c_key = [0x42u8; AEAD_KEY_LEN];
        let ntp_header = [0u8; 48];
        let unique_id = vec![0xAA; UNIQUE_ID_LEN];

        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: unique_id.clone(),
        };
        // Authenticator with truncated value
        let auth_ef = NtpExtensionField {
            field_type: EXT_TYPE_NTS_AUTHENTICATOR,
            value: vec![0x00, 0x10], // nonce_len=16 but no nonce data
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&ntp_header);
        packet.extend_from_slice(&uid_ef.to_bytes());
        packet.extend_from_slice(&auth_ef.to_bytes());

        let result = verify_nts_response(&s2c_key, &packet, &unique_id);
        assert!(result.is_err());
    }

    #[test]
    fn test_verify_nts_response_missing_authenticator() {
        let s2c_key = [0x42u8; AEAD_KEY_LEN];
        let ntp_header = [0u8; 48];
        let unique_id = vec![0xAA; UNIQUE_ID_LEN];

        let uid_ef = NtpExtensionField {
            field_type: EXT_TYPE_UNIQUE_ID,
            value: unique_id.clone(),
        };

        let mut packet = Vec::new();
        packet.extend_from_slice(&ntp_header);
        packet.extend_from_slice(&uid_ef.to_bytes());
        // No authenticator field

        let result = verify_nts_response(&s2c_key, &packet, &unique_id);
        assert!(result.is_err());
        match result.unwrap_err() {
            NtsError::AuthFailed(msg) => assert!(msg.contains("Authenticator")),
            other => panic!("expected AuthFailed, got: {other}"),
        }
    }
}
