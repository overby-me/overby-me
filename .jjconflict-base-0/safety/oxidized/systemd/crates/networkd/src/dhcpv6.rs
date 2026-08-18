//! DHCPv6 client implementation (RFC 8415).
//!
//! Implements the four-message exchange (Solicit → Advertise → Request → Reply)
//! and the two-message exchange for stateless DHCPv6 (Information-Request → Reply).
//! Supports IA_NA for address assignment, IA_PD for prefix delegation (RFC 8415 §6.3),
//! DNS recursive name server option, DNS domain search list option, and DUID-based
//! client identification.

use std::fmt;
use std::net::Ipv6Addr;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// DHCPv6 constants (RFC 8415)
// ---------------------------------------------------------------------------

/// DHCPv6 server port (destination for client messages).
const DHCPV6_SERVER_PORT: u16 = 547;
/// DHCPv6 client port (source for client messages / destination for server messages).
const DHCPV6_CLIENT_PORT: u16 = 546;

/// All DHCP relay agents and servers multicast address (ff02::1:2).
const ALL_DHCP_RELAY_AGENTS_AND_SERVERS: Ipv6Addr = Ipv6Addr::new(0xff02, 0, 0, 0, 0, 0, 1, 2);

// Message types (RFC 8415 §7.3)
const MSG_SOLICIT: u8 = 1;
const MSG_ADVERTISE: u8 = 2;
const MSG_REQUEST: u8 = 3;
const MSG_CONFIRM: u8 = 4;
const MSG_RENEW: u8 = 5;
const MSG_REBIND: u8 = 6;
const MSG_REPLY: u8 = 7;
const MSG_RELEASE: u8 = 8;
const MSG_DECLINE: u8 = 9;
const MSG_RECONFIGURE: u8 = 10;
const MSG_INFORMATION_REQUEST: u8 = 11;

// Option codes (RFC 8415 §21, RFC 3646)
const OPT_CLIENTID: u16 = 1;
const OPT_SERVERID: u16 = 2;
const OPT_IA_NA: u16 = 3;
const OPT_IA_TA: u16 = 4;
const OPT_IAADDR: u16 = 5;
const OPT_ORO: u16 = 6;
const OPT_PREFERENCE: u16 = 7;
const OPT_ELAPSED_TIME: u16 = 8;
const OPT_STATUS_CODE: u16 = 13;
const OPT_RAPID_COMMIT: u16 = 14;
const OPT_DNS_SERVERS: u16 = 23; // RFC 3646
const OPT_DOMAIN_LIST: u16 = 24; // RFC 3646
const OPT_IA_PD: u16 = 25;
const OPT_IAPREFIX: u16 = 26;
const OPT_INFORMATION_REFRESH_TIME: u16 = 32;
const OPT_SOL_MAX_RT: u16 = 82;
const OPT_INF_MAX_RT: u16 = 83;

// Status codes (RFC 8415 §21.13)
const STATUS_SUCCESS: u16 = 0;
const STATUS_UNSPEC_FAIL: u16 = 1;
const STATUS_NO_ADDRS_AVAIL: u16 = 2;
const STATUS_NO_BINDING: u16 = 3;
const STATUS_NOT_ON_LINK: u16 = 4;
const STATUS_USE_MULTICAST: u16 = 5;
const STATUS_NO_PREFIX_AVAIL: u16 = 6;

// DUID types (RFC 8415 §11)
const DUID_LLT: u16 = 1; // Link-layer address plus time
const DUID_EN: u16 = 2; // Assigned by vendor based on Enterprise Number
const DUID_LL: u16 = 3; // Link-layer address
const DUID_UUID: u16 = 4; // UUID-based

/// Hardware type for Ethernet (RFC 826).
const HW_TYPE_ETHERNET: u16 = 1;

/// Minimum DHCPv6 message size: 4 bytes (type + transaction ID).
const MIN_MSG_LEN: usize = 4;

// Default retransmission parameters (RFC 8415 §7.6)
const SOL_TIMEOUT: Duration = Duration::from_secs(1);
const SOL_MAX_RT: Duration = Duration::from_secs(3600);
const REQ_TIMEOUT: Duration = Duration::from_secs(1);
const REQ_MAX_RT: Duration = Duration::from_secs(30);
const REQ_MAX_RC: u32 = 10;
const REN_TIMEOUT: Duration = Duration::from_secs(10);
const REN_MAX_RT: Duration = Duration::from_secs(600);
const REB_TIMEOUT: Duration = Duration::from_secs(10);
const REB_MAX_RT: Duration = Duration::from_secs(600);
const INF_TIMEOUT: Duration = Duration::from_secs(1);
const INF_MAX_RT: Duration = Duration::from_secs(3600);
const REL_TIMEOUT: Duration = Duration::from_secs(1);
const REL_MAX_RC: u32 = 4;

// ---------------------------------------------------------------------------
// DUID (DHCP Unique Identifier)
// ---------------------------------------------------------------------------

/// A DUID identifies a DHCPv6 client or server.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Duid {
    pub data: Vec<u8>,
}

impl Duid {
    /// Create a DUID-LL (link-layer address) from a MAC address.
    pub fn from_mac(mac: &[u8; 6]) -> Self {
        let mut data = Vec::with_capacity(2 + 2 + 6);
        // DUID type = DUID-LL (3)
        data.extend_from_slice(&DUID_LL.to_be_bytes());
        // Hardware type = Ethernet (1)
        data.extend_from_slice(&HW_TYPE_ETHERNET.to_be_bytes());
        // Link-layer address
        data.extend_from_slice(mac);
        Self { data }
    }

    /// Create a DUID-LLT (link-layer address plus time) from a MAC and timestamp.
    /// The `time` is seconds since 2000-01-01 00:00:00 UTC (DHCPv6 epoch).
    pub fn from_mac_time(mac: &[u8; 6], time: u32) -> Self {
        let mut data = Vec::with_capacity(2 + 2 + 4 + 6);
        // DUID type = DUID-LLT (1)
        data.extend_from_slice(&DUID_LLT.to_be_bytes());
        // Hardware type = Ethernet (1)
        data.extend_from_slice(&HW_TYPE_ETHERNET.to_be_bytes());
        // Time value
        data.extend_from_slice(&time.to_be_bytes());
        // Link-layer address
        data.extend_from_slice(mac);
        Self { data }
    }

    /// Create a DUID from raw bytes (e.g. parsed from a server response).
    pub fn from_bytes(data: Vec<u8>) -> Self {
        Self { data }
    }

    /// Encode as DHCPv6 option payload.
    pub fn as_bytes(&self) -> &[u8] {
        &self.data
    }

    /// DUID type code.
    pub fn duid_type(&self) -> Option<u16> {
        if self.data.len() >= 2 {
            Some(u16::from_be_bytes([self.data[0], self.data[1]]))
        } else {
            None
        }
    }
}

impl fmt::Display for Duid {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, b) in self.data.iter().enumerate() {
            if i > 0 {
                write!(f, ":")?;
            }
            write!(f, "{:02x}", b)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// IA_NA (Identity Association for Non-temporary Addresses)
// ---------------------------------------------------------------------------

/// An address binding within an IA_NA.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IaAddress {
    /// IPv6 address.
    pub address: Ipv6Addr,
    /// Preferred lifetime in seconds.
    pub preferred_lifetime: u32,
    /// Valid lifetime in seconds.
    pub valid_lifetime: u32,
}

impl fmt::Display for IaAddress {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} preferred={}s valid={}s",
            self.address, self.preferred_lifetime, self.valid_lifetime
        )
    }
}

/// Identity Association for Non-temporary Addresses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IaNa {
    /// IA identifier (chosen by client, must be consistent across restarts).
    pub iaid: u32,
    /// T1: time at which the client contacts the server to extend lifetimes.
    pub t1: u32,
    /// T2: time at which the client contacts any server to extend lifetimes.
    pub t2: u32,
    /// Address bindings.
    pub addresses: Vec<IaAddress>,
    /// Status code from within the IA_NA (if present).
    pub status: Option<(u16, String)>,
}

impl IaNa {
    /// Create a new IA_NA with the given IAID and no addresses.
    pub fn new(iaid: u32) -> Self {
        Self {
            iaid,
            t1: 0,
            t2: 0,
            addresses: Vec::new(),
            status: None,
        }
    }
}

impl fmt::Display for IaNa {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IA_NA(iaid={:#010x} T1={}s T2={}s",
            self.iaid, self.t1, self.t2
        )?;
        for addr in &self.addresses {
            write!(f, " {}", addr)?;
        }
        if let Some((code, ref msg)) = self.status {
            write!(f, " status={}:{}", code, msg)?;
        }
        write!(f, ")")
    }
}

// ---------------------------------------------------------------------------
// IA_PD (Identity Association for Prefix Delegation) — RFC 8415 §6.3
// ---------------------------------------------------------------------------

/// A delegated prefix within an IA_PD.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IaPrefix {
    /// Preferred lifetime in seconds.
    pub preferred_lifetime: u32,
    /// Valid lifetime in seconds.
    pub valid_lifetime: u32,
    /// Prefix length (e.g. 48, 56, 64).
    pub prefix_length: u8,
    /// Delegated prefix (network address portion).
    pub prefix: Ipv6Addr,
}

impl fmt::Display for IaPrefix {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{} preferred={}s valid={}s",
            self.prefix, self.prefix_length, self.preferred_lifetime, self.valid_lifetime
        )
    }
}

/// Identity Association for Prefix Delegation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IaPd {
    /// IA identifier (chosen by client, must be consistent across restarts).
    pub iaid: u32,
    /// T1: time at which the client contacts the server to extend lifetimes.
    pub t1: u32,
    /// T2: time at which the client contacts any server to extend lifetimes.
    pub t2: u32,
    /// Delegated prefix bindings.
    pub prefixes: Vec<IaPrefix>,
    /// Status code from within the IA_PD (if present).
    pub status: Option<(u16, String)>,
}

impl IaPd {
    /// Create a new IA_PD with the given IAID and no prefixes.
    pub fn new(iaid: u32) -> Self {
        Self {
            iaid,
            t1: 0,
            t2: 0,
            prefixes: Vec::new(),
            status: None,
        }
    }
}

impl fmt::Display for IaPd {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "IA_PD(iaid={:#010x} T1={}s T2={}s",
            self.iaid, self.t1, self.t2
        )?;
        for pfx in &self.prefixes {
            write!(f, " {}", pfx)?;
        }
        if let Some((code, ref msg)) = self.status {
            write!(f, " status={}:{}", code, msg)?;
        }
        write!(f, ")")
    }
}

// ---------------------------------------------------------------------------
// DHCPv6 Lease
// ---------------------------------------------------------------------------

/// A DHCPv6 lease obtained from a server.
#[derive(Debug, Clone)]
pub struct Dhcpv6Lease {
    /// The IA_NA with assigned addresses.
    pub ia_na: IaNa,

    /// The IA_PD with delegated prefixes (empty if PD not requested/granted).
    pub ia_pd: Option<IaPd>,

    /// Server DUID.
    pub server_id: Duid,

    /// DNS recursive name servers (option 23).
    pub dns_servers: Vec<Ipv6Addr>,

    /// DNS search domain list (option 24).
    pub domains: Vec<String>,

    /// Server preference (0-255, higher is better).
    pub preference: u8,

    /// Timestamp when the lease was obtained.
    pub obtained_at: Instant,
}

impl Dhcpv6Lease {
    /// T1 time — earliest T1 across IA_NA and IA_PD (time to start Renew).
    pub fn t1(&self) -> Duration {
        let na_t1 = self.t1_from_ia_na();
        let pd_t1 = self.t1_from_ia_pd();
        match (na_t1, pd_t1) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => Duration::from_secs(1800), // Fallback.
        }
    }

    fn t1_from_ia_na(&self) -> Option<Duration> {
        if self.ia_na.addresses.is_empty() {
            return None;
        }
        if self.ia_na.t1 == 0 || self.ia_na.t1 == 0xFFFFFFFF {
            // Use 0.5 * shortest preferred lifetime as a reasonable default.
            let min_pref = self
                .ia_na
                .addresses
                .iter()
                .map(|a| a.preferred_lifetime)
                .filter(|&t| t > 0 && t != 0xFFFFFFFF)
                .min()
                .unwrap_or(3600);
            Some(Duration::from_secs((min_pref / 2) as u64))
        } else {
            Some(Duration::from_secs(self.ia_na.t1 as u64))
        }
    }

    fn t1_from_ia_pd(&self) -> Option<Duration> {
        let ia_pd = self.ia_pd.as_ref()?;
        if ia_pd.prefixes.is_empty() {
            return None;
        }
        if ia_pd.t1 == 0 || ia_pd.t1 == 0xFFFFFFFF {
            let min_pref = ia_pd
                .prefixes
                .iter()
                .map(|p| p.preferred_lifetime)
                .filter(|&t| t > 0 && t != 0xFFFFFFFF)
                .min()
                .unwrap_or(3600);
            Some(Duration::from_secs((min_pref / 2) as u64))
        } else {
            Some(Duration::from_secs(ia_pd.t1 as u64))
        }
    }

    /// T2 time — earliest T2 across IA_NA and IA_PD (time to start Rebind).
    pub fn t2(&self) -> Duration {
        let na_t2 = self.t2_from_ia_na();
        let pd_t2 = self.t2_from_ia_pd();
        match (na_t2, pd_t2) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => Duration::from_secs(2880), // Fallback.
        }
    }

    fn t2_from_ia_na(&self) -> Option<Duration> {
        if self.ia_na.addresses.is_empty() {
            return None;
        }
        if self.ia_na.t2 == 0 || self.ia_na.t2 == 0xFFFFFFFF {
            let min_pref = self
                .ia_na
                .addresses
                .iter()
                .map(|a| a.preferred_lifetime)
                .filter(|&t| t > 0 && t != 0xFFFFFFFF)
                .min()
                .unwrap_or(3600);
            Some(Duration::from_secs((min_pref * 4 / 5) as u64))
        } else {
            Some(Duration::from_secs(self.ia_na.t2 as u64))
        }
    }

    fn t2_from_ia_pd(&self) -> Option<Duration> {
        let ia_pd = self.ia_pd.as_ref()?;
        if ia_pd.prefixes.is_empty() {
            return None;
        }
        if ia_pd.t2 == 0 || ia_pd.t2 == 0xFFFFFFFF {
            let min_pref = ia_pd
                .prefixes
                .iter()
                .map(|p| p.preferred_lifetime)
                .filter(|&t| t > 0 && t != 0xFFFFFFFF)
                .min()
                .unwrap_or(3600);
            Some(Duration::from_secs((min_pref * 4 / 5) as u64))
        } else {
            Some(Duration::from_secs(ia_pd.t2 as u64))
        }
    }

    /// Whether the T1 renewal time has been reached.
    pub fn needs_renewal(&self) -> bool {
        self.obtained_at.elapsed() >= self.t1()
    }

    /// Whether the T2 rebinding time has been reached.
    pub fn needs_rebinding(&self) -> bool {
        self.obtained_at.elapsed() >= self.t2()
    }

    /// Whether all addresses and prefixes in the lease have expired.
    pub fn is_expired(&self) -> bool {
        let elapsed = self.obtained_at.elapsed().as_secs() as u32;
        let na_expired = self.ia_na.addresses.is_empty()
            || self
                .ia_na
                .addresses
                .iter()
                .all(|a| a.valid_lifetime != 0xFFFFFFFF && elapsed >= a.valid_lifetime);
        let pd_expired = match &self.ia_pd {
            Some(pd) => {
                pd.prefixes.is_empty()
                    || pd
                        .prefixes
                        .iter()
                        .all(|p| p.valid_lifetime != 0xFFFFFFFF && elapsed >= p.valid_lifetime)
            }
            None => true,
        };
        na_expired && pd_expired
    }

    /// Remaining time until the first address or prefix expires.
    pub fn remaining(&self) -> Duration {
        let elapsed = self.obtained_at.elapsed().as_secs() as u32;
        let na_remaining = self
            .ia_na
            .addresses
            .iter()
            .map(|a| {
                if a.valid_lifetime == 0xFFFFFFFF {
                    u32::MAX
                } else {
                    a.valid_lifetime.saturating_sub(elapsed)
                }
            })
            .min();
        let pd_remaining = self.ia_pd.as_ref().and_then(|pd| {
            pd.prefixes
                .iter()
                .map(|p| {
                    if p.valid_lifetime == 0xFFFFFFFF {
                        u32::MAX
                    } else {
                        p.valid_lifetime.saturating_sub(elapsed)
                    }
                })
                .min()
        });
        let min_valid = match (na_remaining, pd_remaining) {
            (Some(a), Some(b)) => a.min(b),
            (Some(a), None) => a,
            (None, Some(b)) => b,
            (None, None) => 0,
        };
        Duration::from_secs(min_valid as u64)
    }

    /// The primary (first) assigned IPv6 address, if any.
    pub fn primary_address(&self) -> Option<Ipv6Addr> {
        self.ia_na.addresses.first().map(|a| a.address)
    }

    /// The delegated prefixes, if any.
    pub fn delegated_prefixes(&self) -> &[IaPrefix] {
        match &self.ia_pd {
            Some(pd) => &pd.prefixes,
            None => &[],
        }
    }

    /// Whether this lease contains delegated prefixes.
    pub fn has_prefix_delegation(&self) -> bool {
        self.ia_pd
            .as_ref()
            .map(|pd| !pd.prefixes.is_empty())
            .unwrap_or(false)
    }
}

impl fmt::Display for Dhcpv6Lease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DHCPv6 lease")?;
        for addr in &self.ia_na.addresses {
            write!(f, " addr={}", addr.address)?;
        }
        if let Some(ref pd) = self.ia_pd {
            for pfx in &pd.prefixes {
                write!(f, " prefix={}/{}", pfx.prefix, pfx.prefix_length)?;
            }
        }
        if !self.dns_servers.is_empty() {
            write!(f, " dns={:?}", self.dns_servers)?;
        }
        if !self.domains.is_empty() {
            write!(f, " domains={:?}", self.domains)?;
        }
        write!(f, " T1={}s T2={}s", self.ia_na.t1, self.ia_na.t2)?;
        if let Some(ref pd) = self.ia_pd {
            write!(f, " PD-T1={}s PD-T2={}s", pd.t1, pd.t2)?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// DHCPv6 State Machine
// ---------------------------------------------------------------------------

/// DHCPv6 client states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Dhcpv6State {
    /// Initial state — will send Solicit.
    Init,
    /// Waiting for Advertise after Solicit.
    Soliciting,
    /// Waiting for Reply after Request.
    Requesting,
    /// Lease obtained.
    Bound,
    /// Renewing lease with the same server (unicast or multicast).
    Renewing,
    /// Rebinding — contacting any server.
    Rebinding,
    /// Stateless: waiting for Reply after Information-Request.
    InformationRequesting,
    /// Stateless: have received configuration information.
    InformationReceived,
}

impl fmt::Display for Dhcpv6State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => write!(f, "INIT"),
            Self::Soliciting => write!(f, "SOLICITING"),
            Self::Requesting => write!(f, "REQUESTING"),
            Self::Bound => write!(f, "BOUND"),
            Self::Renewing => write!(f, "RENEWING"),
            Self::Rebinding => write!(f, "REBINDING"),
            Self::InformationRequesting => write!(f, "INFORMATION-REQUESTING"),
            Self::InformationReceived => write!(f, "INFORMATION-RECEIVED"),
        }
    }
}

// ---------------------------------------------------------------------------
// DHCPv6 Message
// ---------------------------------------------------------------------------

/// A parsed DHCPv6 option.
#[derive(Debug, Clone)]
pub struct Dhcpv6Option {
    pub code: u16,
    pub data: Vec<u8>,
}

/// A DHCPv6 message (client ↔ server).
#[derive(Debug, Clone)]
pub struct Dhcpv6Message {
    /// Message type.
    pub msg_type: u8,
    /// Transaction ID (24-bit).
    pub transaction_id: [u8; 3],
    /// Options.
    pub options: Vec<Dhcpv6Option>,
}

impl Dhcpv6Message {
    pub fn new(msg_type: u8, transaction_id: [u8; 3]) -> Self {
        Self {
            msg_type,
            transaction_id,
            options: Vec::new(),
        }
    }

    pub fn add_option(&mut self, code: u16, data: Vec<u8>) {
        self.options.push(Dhcpv6Option { code, data });
    }

    pub fn find_option(&self, code: u16) -> Option<&Dhcpv6Option> {
        self.options.iter().find(|o| o.code == code)
    }

    pub fn find_options(&self, code: u16) -> Vec<&Dhcpv6Option> {
        self.options.iter().filter(|o| o.code == code).collect()
    }

    pub fn server_id(&self) -> Option<Duid> {
        self.find_option(OPT_SERVERID)
            .map(|o| Duid::from_bytes(o.data.clone()))
    }

    pub fn client_id(&self) -> Option<Duid> {
        self.find_option(OPT_CLIENTID)
            .map(|o| Duid::from_bytes(o.data.clone()))
    }

    pub fn preference(&self) -> u8 {
        self.find_option(OPT_PREFERENCE)
            .and_then(|o| o.data.first().copied())
            .unwrap_or(0)
    }

    pub fn status_code(&self) -> Option<(u16, String)> {
        self.find_option(OPT_STATUS_CODE)
            .and_then(parse_status_code_option)
    }

    pub fn parse_ia_nas(&self) -> Vec<IaNa> {
        self.find_options(OPT_IA_NA)
            .into_iter()
            .filter_map(|o| parse_ia_na(&o.data))
            .collect()
    }

    /// Parse all IA_PD options from this message.
    pub fn parse_ia_pds(&self) -> Vec<IaPd> {
        self.find_options(OPT_IA_PD)
            .into_iter()
            .filter_map(|o| parse_ia_pd(&o.data))
            .collect()
    }

    pub fn dns_servers(&self) -> Vec<Ipv6Addr> {
        self.find_option(OPT_DNS_SERVERS)
            .map(|o| parse_ipv6_list(&o.data))
            .unwrap_or_default()
    }

    pub fn domain_list(&self) -> Vec<String> {
        self.find_option(OPT_DOMAIN_LIST)
            .map(|o| parse_dns_labels(&o.data))
            .unwrap_or_default()
    }

    pub fn has_rapid_commit(&self) -> bool {
        self.find_option(OPT_RAPID_COMMIT).is_some()
    }

    /// Serialize to wire format.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(256);
        buf.push(self.msg_type);
        buf.extend_from_slice(&self.transaction_id);

        for opt in &self.options {
            buf.extend_from_slice(&opt.code.to_be_bytes());
            buf.extend_from_slice(&(opt.data.len() as u16).to_be_bytes());
            buf.extend_from_slice(&opt.data);
        }

        buf
    }

    /// Parse from wire format.
    pub fn parse(data: &[u8]) -> Option<Self> {
        if data.len() < MIN_MSG_LEN {
            return None;
        }

        let msg_type = data[0];
        let transaction_id = [data[1], data[2], data[3]];

        let options = parse_options(&data[4..])?;

        Some(Self {
            msg_type,
            transaction_id,
            options,
        })
    }
}

impl fmt::Display for Dhcpv6Message {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DHCPv6 {} tid={:02x}{:02x}{:02x} ({} options)",
            dhcpv6_message_type_name(self.msg_type),
            self.transaction_id[0],
            self.transaction_id[1],
            self.transaction_id[2],
            self.options.len()
        )
    }
}

// ---------------------------------------------------------------------------
// DHCPv6 Client Configuration
// ---------------------------------------------------------------------------

/// Configuration for the DHCPv6 client.
#[derive(Debug, Clone)]
pub struct Dhcpv6ClientConfig {
    /// Interface index.
    pub ifindex: u32,
    /// Interface name.
    pub ifname: String,
    /// MAC address (for DUID generation).
    pub mac: [u8; 6],
    /// Whether to request addresses (stateful mode, M flag).
    /// When false, only requests configuration (stateless, O flag).
    pub request_addresses: bool,
    /// Whether to request prefix delegation (IA_PD).
    pub request_prefix_delegation: bool,
    /// Prefix delegation hint: requested prefix length (0 = no hint).
    /// Common values: 48, 56, 60, 64.
    pub prefix_hint: u8,
    /// Prefix delegation hint: requested prefix address (:: = no hint).
    pub prefix_hint_address: Ipv6Addr,
    /// Use Rapid Commit (2-message exchange).
    pub rapid_commit: bool,
    /// Maximum attempts (0 = unlimited).
    pub max_attempts: u32,
    /// Request options (ORO).
    pub request_options: Vec<u16>,
}

impl Default for Dhcpv6ClientConfig {
    fn default() -> Self {
        Self {
            ifindex: 0,
            ifname: String::new(),
            mac: [0; 6],
            request_addresses: true,
            request_prefix_delegation: false,
            prefix_hint: 0,
            prefix_hint_address: Ipv6Addr::UNSPECIFIED,
            rapid_commit: false,
            max_attempts: 0,
            request_options: vec![OPT_DNS_SERVERS, OPT_DOMAIN_LIST],
        }
    }
}

// ---------------------------------------------------------------------------
// DHCPv6 Client
// ---------------------------------------------------------------------------

/// Stateful DHCPv6 client implementing the four-message exchange
/// (Solicit → Advertise → Request → Reply) and stateless
/// (Information-Request → Reply) mode.  Supports IA_PD for prefix delegation.
#[derive(Debug)]
pub struct Dhcpv6Client {
    /// Client configuration.
    pub config: Dhcpv6ClientConfig,
    /// Current state.
    pub state: Dhcpv6State,
    /// Client DUID.
    pub client_duid: Duid,
    /// Transaction ID for the current exchange.
    pub transaction_id: [u8; 3],
    /// IAID for IA_NA requests (derived from ifindex).
    pub iaid: u32,
    /// IAID for IA_PD requests (derived from ifindex + 1 to avoid collision).
    pub pd_iaid: u32,
    /// Best Advertise received during Soliciting (highest preference).
    pub best_advertise: Option<Dhcpv6Message>,
    /// Current lease.
    pub lease: Option<Dhcpv6Lease>,
    /// Transmission attempt counter.
    pub attempts: u32,
    /// When the last packet was sent.
    pub last_send: Option<Instant>,
    /// When the exchange started (for elapsed time option).
    pub exchange_start: Option<Instant>,
    /// Current retransmission timeout (for binary exponential backoff).
    pub current_rt: Duration,
    /// Stateless DNS servers (from Information-Request reply).
    pub stateless_dns: Vec<Ipv6Addr>,
    /// Stateless domains (from Information-Request reply).
    pub stateless_domains: Vec<String>,
    /// Information refresh time (for stateless mode).
    pub info_refresh_time: Option<Duration>,
}

impl Dhcpv6Client {
    /// Create a new DHCPv6 client.
    pub fn new(config: Dhcpv6ClientConfig) -> Self {
        let client_duid = Duid::from_mac(&config.mac);
        let iaid = config.ifindex;
        // Use ifindex + 1 for PD IAID to avoid collision with IA_NA IAID.
        let pd_iaid = config.ifindex.wrapping_add(1);
        let tid = generate_transaction_id();

        // Both stateful and stateless start in Init;
        // stateless switches to InformationRequesting on first next_packet().
        let initial_state = Dhcpv6State::Init;

        Self {
            config,
            state: initial_state,
            client_duid,
            transaction_id: tid,
            iaid,
            pd_iaid,
            best_advertise: None,
            lease: None,
            attempts: 0,
            last_send: None,
            exchange_start: None,
            current_rt: SOL_TIMEOUT,
            stateless_dns: Vec::new(),
            stateless_domains: Vec::new(),
            info_refresh_time: None,
        }
    }

    /// Build the next outgoing packet based on current state.
    /// Returns `None` if no packet should be sent.
    pub fn next_packet(&mut self) -> Option<Vec<u8>> {
        if self.config.request_addresses || self.config.request_prefix_delegation {
            self.next_packet_stateful()
        } else {
            self.next_packet_stateless()
        }
    }

    /// Build the next packet for stateful (address assignment) mode.
    fn next_packet_stateful(&mut self) -> Option<Vec<u8>> {
        match self.state {
            Dhcpv6State::Init => {
                self.transaction_id = generate_transaction_id();
                self.state = Dhcpv6State::Soliciting;
                self.attempts = 1;
                self.last_send = Some(Instant::now());
                self.exchange_start = Some(Instant::now());
                self.current_rt = SOL_TIMEOUT;
                self.best_advertise = None;
                Some(self.build_solicit().serialize())
            }
            Dhcpv6State::Soliciting => {
                // Retransmit Solicit.
                self.attempts += 1;
                self.last_send = Some(Instant::now());
                self.current_rt = retransmit_timeout(self.current_rt, SOL_MAX_RT);
                Some(self.build_solicit().serialize())
            }
            Dhcpv6State::Requesting => {
                if self.attempts >= REQ_MAX_RC {
                    // Max retransmissions reached, restart with Solicit.
                    log::warn!(
                        "{}: DHCPv6 Request max retransmissions reached, restarting",
                        self.config.ifname
                    );
                    self.state = Dhcpv6State::Init;
                    self.best_advertise = None;
                    self.attempts = 0;
                    return None;
                }
                self.attempts += 1;
                self.last_send = Some(Instant::now());
                self.current_rt = retransmit_timeout(self.current_rt, REQ_MAX_RT);
                if let Some(ref adv) = self.best_advertise {
                    Some(self.build_request(adv.clone()).serialize())
                } else {
                    self.state = Dhcpv6State::Init;
                    None
                }
            }
            Dhcpv6State::Bound => {
                // Check if we need to renew.
                if let Some(ref lease) = self.lease
                    && lease.needs_renewal()
                {
                    self.state = Dhcpv6State::Renewing;
                    self.transaction_id = generate_transaction_id();
                    self.attempts = 1;
                    self.last_send = Some(Instant::now());
                    self.exchange_start = Some(Instant::now());
                    self.current_rt = REN_TIMEOUT;
                    return Some(self.build_renew().serialize());
                }
                None
            }
            Dhcpv6State::Renewing => {
                if let Some(ref lease) = self.lease {
                    if lease.needs_rebinding() {
                        // Move to Rebind.
                        self.state = Dhcpv6State::Rebinding;
                        self.transaction_id = generate_transaction_id();
                        self.attempts = 1;
                        self.last_send = Some(Instant::now());
                        self.exchange_start = Some(Instant::now());
                        self.current_rt = REB_TIMEOUT;
                        return Some(self.build_rebind().serialize());
                    }
                    if lease.is_expired() {
                        self.state = Dhcpv6State::Init;
                        self.lease = None;
                        return None;
                    }
                }
                self.attempts += 1;
                self.last_send = Some(Instant::now());
                self.current_rt = retransmit_timeout(self.current_rt, REN_MAX_RT);
                Some(self.build_renew().serialize())
            }
            Dhcpv6State::Rebinding => {
                if let Some(ref lease) = self.lease {
                    if lease.is_expired() {
                        log::warn!("{}: DHCPv6 lease expired during rebind", self.config.ifname);
                        self.state = Dhcpv6State::Init;
                        self.lease = None;
                        return None;
                    }
                } else {
                    self.state = Dhcpv6State::Init;
                    return None;
                }
                self.attempts += 1;
                self.last_send = Some(Instant::now());
                self.current_rt = retransmit_timeout(self.current_rt, REB_MAX_RT);
                Some(self.build_rebind().serialize())
            }
            _ => None,
        }
    }

    /// Build the next packet for stateless (information-only) mode.
    fn next_packet_stateless(&mut self) -> Option<Vec<u8>> {
        match self.state {
            Dhcpv6State::Init => {
                self.transaction_id = generate_transaction_id();
                self.state = Dhcpv6State::InformationRequesting;
                self.attempts = 1;
                self.last_send = Some(Instant::now());
                self.exchange_start = Some(Instant::now());
                self.current_rt = INF_TIMEOUT;
                Some(self.build_information_request().serialize())
            }
            Dhcpv6State::InformationRequesting => {
                self.attempts += 1;
                self.last_send = Some(Instant::now());
                self.current_rt = retransmit_timeout(self.current_rt, INF_MAX_RT);
                Some(self.build_information_request().serialize())
            }
            Dhcpv6State::InformationReceived => {
                // Check if information refresh time has elapsed.
                if let Some(refresh) = self.info_refresh_time
                    && let Some(start) = self.exchange_start
                    && start.elapsed() >= refresh
                {
                    self.state = Dhcpv6State::Init;
                    return None; // Will re-request on next call.
                }
                None
            }
            _ => None,
        }
    }

    /// Process an incoming DHCPv6 message.
    /// Returns `Some(lease)` if a new lease was obtained or renewed (stateful),
    /// or `Some(lease)` with empty addresses for stateless info received.
    pub fn process_reply(&mut self, data: &[u8]) -> Option<Dhcpv6Lease> {
        let msg = Dhcpv6Message::parse(data)?;

        // Verify transaction ID matches.
        if msg.transaction_id != self.transaction_id {
            log::trace!(
                "{}: ignoring DHCPv6 message with wrong tid",
                self.config.ifname
            );
            return None;
        }

        // Verify client ID matches.
        if let Some(cid) = msg.client_id()
            && cid != self.client_duid
        {
            log::trace!(
                "{}: ignoring DHCPv6 message with wrong client DUID",
                self.config.ifname
            );
            return None;
        }

        // Check top-level status code.
        if let Some((code, ref status_msg)) = msg.status_code()
            && code != STATUS_SUCCESS
        {
            log::warn!(
                "{}: DHCPv6 {} status={} ({}): {}",
                self.config.ifname,
                dhcpv6_message_type_name(msg.msg_type),
                code,
                status_code_name(code),
                status_msg
            );
            if code == STATUS_USE_MULTICAST {
                // Server wants us to use multicast; we already do. Retry.
                return None;
            }
            if code == STATUS_NO_ADDRS_AVAIL || code == STATUS_NO_PREFIX_AVAIL {
                // No addresses available; restart after a delay.
                self.state = Dhcpv6State::Init;
                self.attempts = 0;
                return None;
            }
            if code == STATUS_NOT_ON_LINK {
                // Our address is not valid for this link, restart.
                self.state = Dhcpv6State::Init;
                self.lease = None;
                self.attempts = 0;
                return None;
            }
        }

        match (self.state, msg.msg_type) {
            // Soliciting: expect Advertise.
            (Dhcpv6State::Soliciting, MSG_ADVERTISE) => {
                self.handle_advertise(msg);
                None
            }
            // Soliciting: Rapid Commit Reply.
            (Dhcpv6State::Soliciting, MSG_REPLY) if msg.has_rapid_commit() => {
                self.handle_reply(msg)
            }
            // Requesting: expect Reply.
            (Dhcpv6State::Requesting, MSG_REPLY) => self.handle_reply(msg),
            // Renewing: expect Reply.
            (Dhcpv6State::Renewing, MSG_REPLY) => self.handle_reply(msg),
            // Rebinding: expect Reply.
            (Dhcpv6State::Rebinding, MSG_REPLY) => self.handle_reply(msg),
            // Information-Requesting: expect Reply.
            (Dhcpv6State::InformationRequesting, MSG_REPLY) => self.handle_information_reply(msg),
            _ => {
                log::trace!(
                    "{}: ignoring DHCPv6 {} in state {}",
                    self.config.ifname,
                    dhcpv6_message_type_name(msg.msg_type),
                    self.state
                );
                None
            }
        }
    }

    /// Handle an Advertise message during Soliciting.
    fn handle_advertise(&mut self, msg: Dhcpv6Message) {
        let preference = msg.preference();

        log::info!(
            "{}: received DHCPv6 Advertise (preference={}) from {:?}",
            self.config.ifname,
            preference,
            msg.server_id()
        );

        // Check if this is better than what we have.
        let dominated = match &self.best_advertise {
            Some(best) => preference > best.preference(),
            None => true,
        };

        if dominated {
            self.best_advertise = Some(msg.clone());
        }

        // If preference is 255, immediately proceed (RFC 8415 §18.2.1).
        // Otherwise, we could wait for more Advertise messages, but for
        // simplicity we accept the first one (matching systemd's behavior).
        if preference == 255 || self.best_advertise.is_some() {
            // Transition to Requesting.
            let adv = self.best_advertise.clone().unwrap();
            self.state = Dhcpv6State::Requesting;
            self.transaction_id = generate_transaction_id();
            self.attempts = 1;
            self.last_send = Some(Instant::now());
            self.exchange_start = Some(Instant::now());
            self.current_rt = REQ_TIMEOUT;

            // The actual Request is built by next_packet() but we also
            // generate it here for immediate sending.
            // (The caller should call next_packet() to get it.)
            let _ = adv;
        }
    }

    /// Handle a Reply message (to Request, Renew, or Rebind).
    fn handle_reply(&mut self, msg: Dhcpv6Message) -> Option<Dhcpv6Lease> {
        let ia_nas = msg.parse_ia_nas();

        // For stateful mode, we need at least one IA_NA with an address.
        let ia_na = if self.config.request_addresses {
            let mut best_ia: Option<IaNa> = None;
            for ia in ia_nas {
                // Check IA-level status.
                if let Some((code, ref _msg)) = ia.status
                    && code != STATUS_SUCCESS
                {
                    log::warn!(
                        "{}: IA_NA iaid={:#010x} status={} ({})",
                        self.config.ifname,
                        ia.iaid,
                        code,
                        status_code_name(code)
                    );
                    continue;
                }
                if !ia.addresses.is_empty() && (best_ia.is_none() || ia.iaid == self.iaid) {
                    best_ia = Some(ia);
                }
            }
            match best_ia {
                Some(ia) => ia,
                None => {
                    // If we also requested PD, don't fail just because IA_NA
                    // is missing — the server may only grant prefixes.
                    if !self.config.request_prefix_delegation {
                        log::warn!("{}: DHCPv6 Reply has no usable IA_NA", self.config.ifname);
                        if self.state == Dhcpv6State::Requesting {
                            self.state = Dhcpv6State::Init;
                            self.best_advertise = None;
                        }
                        return None;
                    }
                    IaNa::new(self.iaid)
                }
            }
        } else {
            IaNa::new(self.iaid) // Empty IA_NA for stateless / PD-only.
        };

        // Parse IA_PD options from the reply.
        let ia_pd = if self.config.request_prefix_delegation {
            let ia_pds = msg.parse_ia_pds();
            let mut best_pd: Option<IaPd> = None;
            for pd in ia_pds {
                // Check IA-level status.
                if let Some((code, ref _msg)) = pd.status
                    && code != STATUS_SUCCESS
                {
                    log::warn!(
                        "{}: IA_PD iaid={:#010x} status={} ({})",
                        self.config.ifname,
                        pd.iaid,
                        code,
                        status_code_name(code)
                    );
                    if code == STATUS_NO_PREFIX_AVAIL {
                        log::info!(
                            "{}: server has no prefixes available for delegation",
                            self.config.ifname
                        );
                    }
                    continue;
                }
                if !pd.prefixes.is_empty() && (best_pd.is_none() || pd.iaid == self.pd_iaid) {
                    best_pd = Some(pd);
                }
            }
            if let Some(ref pd) = best_pd {
                log::info!(
                    "{}: DHCPv6 prefix delegation obtained: {}",
                    self.config.ifname,
                    pd
                );
            }
            best_pd
        } else {
            None
        };

        // We need at least addresses or prefixes for a successful lease.
        if ia_na.addresses.is_empty() && ia_pd.as_ref().is_none_or(|pd| pd.prefixes.is_empty()) {
            log::warn!(
                "{}: DHCPv6 Reply has no usable addresses or prefixes",
                self.config.ifname
            );
            if self.state == Dhcpv6State::Requesting {
                self.state = Dhcpv6State::Init;
                self.best_advertise = None;
            }
            return None;
        }

        let server_id = msg
            .server_id()
            .unwrap_or_else(|| Duid::from_bytes(Vec::new()));
        let dns_servers = msg.dns_servers();
        let domains = msg.domain_list();

        let lease = Dhcpv6Lease {
            ia_na,
            ia_pd,
            server_id,
            dns_servers,
            domains,
            preference: msg.preference(),
            obtained_at: Instant::now(),
        };

        log::info!("{}: DHCPv6 lease obtained: {}", self.config.ifname, lease);

        self.state = Dhcpv6State::Bound;
        self.lease = Some(lease.clone());
        self.attempts = 0;

        Some(lease)
    }

    /// Handle a Reply to an Information-Request.
    fn handle_information_reply(&mut self, msg: Dhcpv6Message) -> Option<Dhcpv6Lease> {
        let dns_servers = msg.dns_servers();
        let domains = msg.domain_list();

        // Information Refresh Time (option 32).
        if let Some(opt) = msg.find_option(OPT_INFORMATION_REFRESH_TIME)
            && opt.data.len() >= 4
        {
            let secs = u32::from_be_bytes([opt.data[0], opt.data[1], opt.data[2], opt.data[3]]);
            self.info_refresh_time = Some(Duration::from_secs(secs as u64));
        }

        log::info!(
            "{}: DHCPv6 information received: dns={:?} domains={:?}",
            self.config.ifname,
            dns_servers,
            domains
        );

        self.stateless_dns = dns_servers.clone();
        self.stateless_domains = domains.clone();
        self.state = Dhcpv6State::InformationReceived;
        self.attempts = 0;

        // Return a pseudo-lease with the info for the caller to use.
        let server_id = msg
            .server_id()
            .unwrap_or_else(|| Duid::from_bytes(Vec::new()));
        Some(Dhcpv6Lease {
            ia_na: IaNa::new(self.iaid),
            ia_pd: None,
            server_id,
            dns_servers,
            domains,
            preference: 0,
            obtained_at: Instant::now(),
        })
    }

    /// Compute the retransmission timeout based on the number of attempts.
    pub fn retransmit_timeout(&self) -> Duration {
        self.current_rt
    }

    /// Whether the maximum number of attempts has been reached.
    pub fn max_attempts_reached(&self) -> bool {
        self.config.max_attempts > 0 && self.attempts >= self.config.max_attempts
    }

    // -----------------------------------------------------------------------
    // Message builders
    // -----------------------------------------------------------------------

    /// Build a Solicit message.
    fn build_solicit(&self) -> Dhcpv6Message {
        let mut msg = Dhcpv6Message::new(MSG_SOLICIT, self.transaction_id);

        // Client ID.
        msg.add_option(OPT_CLIENTID, self.client_duid.data.clone());

        // Elapsed time.
        msg.add_option(OPT_ELAPSED_TIME, self.elapsed_time_value());

        // IA_NA (request an address).
        if self.config.request_addresses {
            msg.add_option(OPT_IA_NA, build_ia_na_option(self.iaid, 0, 0, &[]));
        }

        // IA_PD (request prefix delegation).
        if self.config.request_prefix_delegation {
            let hint = if self.config.prefix_hint > 0 {
                Some(IaPrefix {
                    preferred_lifetime: 0,
                    valid_lifetime: 0,
                    prefix_length: self.config.prefix_hint,
                    prefix: self.config.prefix_hint_address,
                })
            } else {
                None
            };
            let hints: Vec<&IaPrefix> = hint.iter().collect();
            msg.add_option(OPT_IA_PD, build_ia_pd_option(self.pd_iaid, 0, 0, &hints));
        }

        // Rapid Commit (if configured).
        if self.config.rapid_commit {
            msg.add_option(OPT_RAPID_COMMIT, Vec::new());
        }

        // Option Request Option (ORO).
        if !self.config.request_options.is_empty() {
            msg.add_option(OPT_ORO, build_oro(&self.config.request_options));
        }

        msg
    }

    /// Build a Request message (in response to an Advertise).
    fn build_request(&self, advertise: Dhcpv6Message) -> Dhcpv6Message {
        let mut msg = Dhcpv6Message::new(MSG_REQUEST, self.transaction_id);

        // Client ID.
        msg.add_option(OPT_CLIENTID, self.client_duid.data.clone());

        // Server ID (from the Advertise).
        if let Some(sid) = advertise.server_id() {
            msg.add_option(OPT_SERVERID, sid.data);
        }

        // Elapsed time.
        msg.add_option(OPT_ELAPSED_TIME, self.elapsed_time_value());

        // IA_NA — include addresses from the Advertise if available.
        if self.config.request_addresses {
            let adv_ia_nas = advertise.parse_ia_nas();
            let ia_addrs: Vec<&IaAddress> = adv_ia_nas
                .iter()
                .flat_map(|ia| ia.addresses.iter())
                .collect();

            if ia_addrs.is_empty() {
                msg.add_option(OPT_IA_NA, build_ia_na_option(self.iaid, 0, 0, &[]));
            } else {
                msg.add_option(OPT_IA_NA, build_ia_na_option(self.iaid, 0, 0, &ia_addrs));
            }
        }

        // IA_PD — include prefixes from the Advertise if available.
        if self.config.request_prefix_delegation {
            let adv_ia_pds = advertise.parse_ia_pds();
            let ia_prefixes: Vec<&IaPrefix> = adv_ia_pds
                .iter()
                .flat_map(|pd| pd.prefixes.iter())
                .collect();

            if ia_prefixes.is_empty() {
                let hint = if self.config.prefix_hint > 0 {
                    Some(IaPrefix {
                        preferred_lifetime: 0,
                        valid_lifetime: 0,
                        prefix_length: self.config.prefix_hint,
                        prefix: self.config.prefix_hint_address,
                    })
                } else {
                    None
                };
                let hints: Vec<&IaPrefix> = hint.iter().collect();
                msg.add_option(OPT_IA_PD, build_ia_pd_option(self.pd_iaid, 0, 0, &hints));
            } else {
                msg.add_option(
                    OPT_IA_PD,
                    build_ia_pd_option(self.pd_iaid, 0, 0, &ia_prefixes),
                );
            }
        }

        // ORO.
        if !self.config.request_options.is_empty() {
            msg.add_option(OPT_ORO, build_oro(&self.config.request_options));
        }

        msg
    }

    /// Build a Renew message.
    fn build_renew(&self) -> Dhcpv6Message {
        let mut msg = Dhcpv6Message::new(MSG_RENEW, self.transaction_id);

        // Client ID.
        msg.add_option(OPT_CLIENTID, self.client_duid.data.clone());

        // Server ID (from the lease).
        if let Some(ref lease) = self.lease {
            msg.add_option(OPT_SERVERID, lease.server_id.data.clone());
        }

        // Elapsed time.
        msg.add_option(OPT_ELAPSED_TIME, self.elapsed_time_value());

        // IA_NA with current addresses.
        if let Some(ref lease) = self.lease {
            let addrs: Vec<&IaAddress> = lease.ia_na.addresses.iter().collect();
            msg.add_option(
                OPT_IA_NA,
                build_ia_na_option(self.iaid, lease.ia_na.t1, lease.ia_na.t2, &addrs),
            );
        } else if self.config.request_addresses {
            msg.add_option(OPT_IA_NA, build_ia_na_option(self.iaid, 0, 0, &[]));
        }

        // IA_PD with current prefixes.
        if self.config.request_prefix_delegation {
            if let Some(ref lease) = self.lease
                && let Some(ref pd) = lease.ia_pd
            {
                let pfxs: Vec<&IaPrefix> = pd.prefixes.iter().collect();
                msg.add_option(
                    OPT_IA_PD,
                    build_ia_pd_option(self.pd_iaid, pd.t1, pd.t2, &pfxs),
                );
            } else {
                msg.add_option(OPT_IA_PD, build_ia_pd_option(self.pd_iaid, 0, 0, &[]));
            }
        }

        // ORO.
        if !self.config.request_options.is_empty() {
            msg.add_option(OPT_ORO, build_oro(&self.config.request_options));
        }

        msg
    }

    /// Build a Rebind message.
    fn build_rebind(&self) -> Dhcpv6Message {
        let mut msg = Dhcpv6Message::new(MSG_REBIND, self.transaction_id);

        // Client ID.
        msg.add_option(OPT_CLIENTID, self.client_duid.data.clone());

        // Elapsed time.
        msg.add_option(OPT_ELAPSED_TIME, self.elapsed_time_value());

        // IA_NA with current addresses (no Server ID for Rebind).
        if let Some(ref lease) = self.lease {
            let addrs: Vec<&IaAddress> = lease.ia_na.addresses.iter().collect();
            msg.add_option(
                OPT_IA_NA,
                build_ia_na_option(self.iaid, lease.ia_na.t1, lease.ia_na.t2, &addrs),
            );
        } else if self.config.request_addresses {
            msg.add_option(OPT_IA_NA, build_ia_na_option(self.iaid, 0, 0, &[]));
        }

        // IA_PD with current prefixes (no Server ID for Rebind).
        if self.config.request_prefix_delegation {
            if let Some(ref lease) = self.lease
                && let Some(ref pd) = lease.ia_pd
            {
                let pfxs: Vec<&IaPrefix> = pd.prefixes.iter().collect();
                msg.add_option(
                    OPT_IA_PD,
                    build_ia_pd_option(self.pd_iaid, pd.t1, pd.t2, &pfxs),
                );
            } else {
                msg.add_option(OPT_IA_PD, build_ia_pd_option(self.pd_iaid, 0, 0, &[]));
            }
        }

        // ORO.
        if !self.config.request_options.is_empty() {
            msg.add_option(OPT_ORO, build_oro(&self.config.request_options));
        }

        msg
    }

    /// Build a Release message.
    pub fn build_release(&self) -> Option<Vec<u8>> {
        let lease = self.lease.as_ref()?;
        let mut msg = Dhcpv6Message::new(MSG_RELEASE, generate_transaction_id());

        // Client ID.
        msg.add_option(OPT_CLIENTID, self.client_duid.data.clone());

        // Server ID.
        msg.add_option(OPT_SERVERID, lease.server_id.data.clone());

        // IA_NA with current addresses.
        if !lease.ia_na.addresses.is_empty() {
            let addrs: Vec<&IaAddress> = lease.ia_na.addresses.iter().collect();
            msg.add_option(OPT_IA_NA, build_ia_na_option(self.iaid, 0, 0, &addrs));
        }

        // IA_PD with current prefixes.
        if let Some(ref pd) = lease.ia_pd
            && !pd.prefixes.is_empty()
        {
            let pfxs: Vec<&IaPrefix> = pd.prefixes.iter().collect();
            msg.add_option(OPT_IA_PD, build_ia_pd_option(self.pd_iaid, 0, 0, &pfxs));
        }

        Some(msg.serialize())
    }

    /// Build an Information-Request message (stateless DHCPv6).
    fn build_information_request(&self) -> Dhcpv6Message {
        let mut msg = Dhcpv6Message::new(MSG_INFORMATION_REQUEST, self.transaction_id);

        // Client ID.
        msg.add_option(OPT_CLIENTID, self.client_duid.data.clone());

        // Elapsed time.
        msg.add_option(OPT_ELAPSED_TIME, self.elapsed_time_value());

        // ORO.
        let mut oro_options = self.config.request_options.clone();
        if !oro_options.contains(&OPT_INFORMATION_REFRESH_TIME) {
            oro_options.push(OPT_INFORMATION_REFRESH_TIME);
        }
        msg.add_option(OPT_ORO, build_oro(&oro_options));

        msg
    }

    /// Compute the elapsed time option value (in hundredths of a second).
    fn elapsed_time_value(&self) -> Vec<u8> {
        let elapsed = match self.exchange_start {
            Some(start) => {
                let ms = start.elapsed().as_millis() as u64;
                let centisecs = ms / 10;
                // Capped at 0xFFFF (65535 centiseconds ≈ 655 seconds).
                centisecs.min(0xFFFF) as u16
            }
            None => 0,
        };
        elapsed.to_be_bytes().to_vec()
    }
}

// ---------------------------------------------------------------------------
// Option encoding helpers
// ---------------------------------------------------------------------------

/// Build an IA_NA option payload.
fn build_ia_na_option(iaid: u32, t1: u32, t2: u32, addresses: &[&IaAddress]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12 + addresses.len() * 28);
    buf.extend_from_slice(&iaid.to_be_bytes());
    buf.extend_from_slice(&t1.to_be_bytes());
    buf.extend_from_slice(&t2.to_be_bytes());

    for addr in addresses {
        // IA Address sub-option.
        let ia_addr_data = build_ia_addr(addr);
        buf.extend_from_slice(&OPT_IAADDR.to_be_bytes());
        buf.extend_from_slice(&(ia_addr_data.len() as u16).to_be_bytes());
        buf.extend_from_slice(&ia_addr_data);
    }

    buf
}

/// Build an IA Address sub-option payload.
fn build_ia_addr(addr: &IaAddress) -> Vec<u8> {
    let mut buf = Vec::with_capacity(24);
    buf.extend_from_slice(&addr.address.octets());
    buf.extend_from_slice(&addr.preferred_lifetime.to_be_bytes());
    buf.extend_from_slice(&addr.valid_lifetime.to_be_bytes());
    // No sub-options.
    buf
}

/// Build an IA_PD option payload (RFC 8415 §21.21).
///
/// Wire format:
///   IAID (4 bytes) + T1 (4 bytes) + T2 (4 bytes) + IA Prefix sub-options
fn build_ia_pd_option(iaid: u32, t1: u32, t2: u32, prefixes: &[&IaPrefix]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(12 + prefixes.len() * 29);
    buf.extend_from_slice(&iaid.to_be_bytes());
    buf.extend_from_slice(&t1.to_be_bytes());
    buf.extend_from_slice(&t2.to_be_bytes());

    for pfx in prefixes {
        // IA Prefix sub-option (option 26).
        let ia_prefix_data = build_ia_prefix(pfx);
        buf.extend_from_slice(&OPT_IAPREFIX.to_be_bytes());
        buf.extend_from_slice(&(ia_prefix_data.len() as u16).to_be_bytes());
        buf.extend_from_slice(&ia_prefix_data);
    }

    buf
}

/// Build an IA Prefix sub-option payload (RFC 8415 §21.22).
///
/// Wire format:
///   preferred-lifetime (4) + valid-lifetime (4) + prefix-length (1) + prefix (16) = 25 bytes
fn build_ia_prefix(pfx: &IaPrefix) -> Vec<u8> {
    let mut buf = Vec::with_capacity(25);
    buf.extend_from_slice(&pfx.preferred_lifetime.to_be_bytes());
    buf.extend_from_slice(&pfx.valid_lifetime.to_be_bytes());
    buf.push(pfx.prefix_length);
    buf.extend_from_slice(&pfx.prefix.octets());
    // No sub-options.
    buf
}

/// Build the Option Request Option (ORO) payload.
fn build_oro(codes: &[u16]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(codes.len() * 2);
    for &code in codes {
        buf.extend_from_slice(&code.to_be_bytes());
    }
    buf
}

// ---------------------------------------------------------------------------
// Option parsing helpers
// ---------------------------------------------------------------------------

/// Parse options from a DHCPv6 message payload (after the 4-byte header).
fn parse_options(data: &[u8]) -> Option<Vec<Dhcpv6Option>> {
    let mut options = Vec::new();
    let mut pos = 0;

    while pos + 4 <= data.len() {
        let code = u16::from_be_bytes([data[pos], data[pos + 1]]);
        let len = u16::from_be_bytes([data[pos + 2], data[pos + 3]]) as usize;
        pos += 4;

        if pos + len > data.len() {
            // Truncated option — stop parsing but return what we have.
            break;
        }

        options.push(Dhcpv6Option {
            code,
            data: data[pos..pos + len].to_vec(),
        });

        pos += len;
    }

    Some(options)
}

/// Parse an IA_NA option from its data payload.
fn parse_ia_na(data: &[u8]) -> Option<IaNa> {
    if data.len() < 12 {
        return None;
    }

    let iaid = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let t1 = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let t2 = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);

    let mut ia_na = IaNa {
        iaid,
        t1,
        t2,
        addresses: Vec::new(),
        status: None,
    };

    // Parse sub-options within the IA_NA.
    let sub_data = &data[12..];
    if let Some(sub_opts) = parse_options(sub_data) {
        for opt in &sub_opts {
            match opt.code {
                OPT_IAADDR => {
                    if let Some(addr) = parse_ia_addr(&opt.data) {
                        ia_na.addresses.push(addr);
                    }
                }
                OPT_STATUS_CODE => {
                    if let Some(status) = parse_status_code_option(opt) {
                        ia_na.status = Some(status);
                    }
                }
                _ => {}
            }
        }
    }

    Some(ia_na)
}

/// Parse an IA Address sub-option from its data payload.
fn parse_ia_addr(data: &[u8]) -> Option<IaAddress> {
    // 16 bytes address + 4 bytes preferred + 4 bytes valid = 24 bytes minimum.
    if data.len() < 24 {
        return None;
    }

    let mut addr_bytes = [0u8; 16];
    addr_bytes.copy_from_slice(&data[0..16]);
    let address = Ipv6Addr::from(addr_bytes);

    let preferred_lifetime = u32::from_be_bytes([data[16], data[17], data[18], data[19]]);
    let valid_lifetime = u32::from_be_bytes([data[20], data[21], data[22], data[23]]);

    Some(IaAddress {
        address,
        preferred_lifetime,
        valid_lifetime,
    })
}

/// Parse an IA_PD option from its data payload (RFC 8415 §21.21).
///
/// Wire format:
///   IAID (4) + T1 (4) + T2 (4) + sub-options (IA Prefix, Status Code)
fn parse_ia_pd(data: &[u8]) -> Option<IaPd> {
    if data.len() < 12 {
        return None;
    }

    let iaid = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let t1 = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let t2 = u32::from_be_bytes([data[8], data[9], data[10], data[11]]);

    let mut ia_pd = IaPd {
        iaid,
        t1,
        t2,
        prefixes: Vec::new(),
        status: None,
    };

    // Parse sub-options within the IA_PD.
    let sub_data = &data[12..];
    if let Some(sub_opts) = parse_options(sub_data) {
        for opt in &sub_opts {
            match opt.code {
                OPT_IAPREFIX => {
                    if let Some(pfx) = parse_ia_prefix(&opt.data) {
                        ia_pd.prefixes.push(pfx);
                    }
                }
                OPT_STATUS_CODE => {
                    if let Some(status) = parse_status_code_option(opt) {
                        ia_pd.status = Some(status);
                    }
                }
                _ => {}
            }
        }
    }

    Some(ia_pd)
}

/// Parse an IA Prefix sub-option from its data payload (RFC 8415 §21.22).
///
/// Wire format:
///   preferred-lifetime (4) + valid-lifetime (4) + prefix-length (1) + prefix (16) = 25 bytes min
fn parse_ia_prefix(data: &[u8]) -> Option<IaPrefix> {
    if data.len() < 25 {
        return None;
    }

    let preferred_lifetime = u32::from_be_bytes([data[0], data[1], data[2], data[3]]);
    let valid_lifetime = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
    let prefix_length = data[8];

    let mut prefix_bytes = [0u8; 16];
    prefix_bytes.copy_from_slice(&data[9..25]);
    let prefix = Ipv6Addr::from(prefix_bytes);

    Some(IaPrefix {
        preferred_lifetime,
        valid_lifetime,
        prefix_length,
        prefix,
    })
}

/// Parse a Status Code option.
fn parse_status_code_option(opt: &Dhcpv6Option) -> Option<(u16, String)> {
    if opt.data.len() < 2 {
        return None;
    }
    let code = u16::from_be_bytes([opt.data[0], opt.data[1]]);
    let message = if opt.data.len() > 2 {
        String::from_utf8_lossy(&opt.data[2..]).to_string()
    } else {
        String::new()
    };
    Some((code, message))
}

/// Parse a list of IPv6 addresses from option data.
fn parse_ipv6_list(data: &[u8]) -> Vec<Ipv6Addr> {
    let mut addrs = Vec::new();
    let mut pos = 0;
    while pos + 16 <= data.len() {
        let mut bytes = [0u8; 16];
        bytes.copy_from_slice(&data[pos..pos + 16]);
        addrs.push(Ipv6Addr::from(bytes));
        pos += 16;
    }
    addrs
}

/// Parse DNS-encoded domain name labels from option data (RFC 1035 §3.1).
fn parse_dns_labels(data: &[u8]) -> Vec<String> {
    let mut domains = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let mut labels = Vec::new();
        loop {
            if pos >= data.len() {
                break;
            }
            let label_len = data[pos] as usize;
            pos += 1;
            if label_len == 0 {
                break; // Root label — end of this domain name.
            }
            if pos + label_len > data.len() {
                break; // Truncated.
            }
            if let Ok(label) = std::str::from_utf8(&data[pos..pos + label_len]) {
                labels.push(label.to_string());
            }
            pos += label_len;
        }
        if !labels.is_empty() {
            domains.push(labels.join("."));
        }
    }

    domains
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

/// Generate a random 24-bit transaction ID.
pub fn generate_transaction_id() -> [u8; 3] {
    // Use a simple deterministic approach seeded from /dev/urandom
    // or fall back to a time-based approach.
    let mut tid = [0u8; 3];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        use std::io::Read;
        let _ = f.read_exact(&mut tid);
    } else {
        // Fallback: use nanosecond timestamp.
        let t = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos();
        tid[0] = (t & 0xFF) as u8;
        tid[1] = ((t >> 8) & 0xFF) as u8;
        tid[2] = ((t >> 16) & 0xFF) as u8;
    }
    tid
}

/// Binary exponential backoff for retransmission timeout (RFC 8415 §15).
/// RT_new = 2 * RT_prev + RAND * RT_prev
/// where RAND is a random number in [-0.1, 0.1].
/// Simplified: RT_new ≈ 2 * RT_prev, capped at max_rt.
fn retransmit_timeout(current_rt: Duration, max_rt: Duration) -> Duration {
    // Double the current timeout (simplified; real impl adds jitter).
    let new_rt = current_rt.saturating_mul(2);
    if new_rt > max_rt { max_rt } else { new_rt }
}

/// Return the human-readable name for a DHCPv6 message type.
pub fn dhcpv6_message_type_name(msg_type: u8) -> &'static str {
    match msg_type {
        MSG_SOLICIT => "SOLICIT",
        MSG_ADVERTISE => "ADVERTISE",
        MSG_REQUEST => "REQUEST",
        MSG_CONFIRM => "CONFIRM",
        MSG_RENEW => "RENEW",
        MSG_REBIND => "REBIND",
        MSG_REPLY => "REPLY",
        MSG_RELEASE => "RELEASE",
        MSG_DECLINE => "DECLINE",
        MSG_RECONFIGURE => "RECONFIGURE",
        MSG_INFORMATION_REQUEST => "INFORMATION-REQUEST",
        _ => "UNKNOWN",
    }
}

/// Return the human-readable name for a DHCPv6 status code.
pub fn status_code_name(code: u16) -> &'static str {
    match code {
        STATUS_SUCCESS => "Success",
        STATUS_UNSPEC_FAIL => "UnspecFail",
        STATUS_NO_ADDRS_AVAIL => "NoAddrsAvail",
        STATUS_NO_BINDING => "NoBinding",
        STATUS_NOT_ON_LINK => "NotOnLink",
        STATUS_USE_MULTICAST => "UseMulticast",
        STATUS_NO_PREFIX_AVAIL => "NoPrefixAvail",
        _ => "Unknown",
    }
}

/// Return the DHCPv6 all-servers multicast address (ff02::1:2).
pub fn all_dhcp_servers() -> Ipv6Addr {
    ALL_DHCP_RELAY_AGENTS_AND_SERVERS
}

/// Return the DHCPv6 server port.
pub fn server_port() -> u16 {
    DHCPV6_SERVER_PORT
}

/// Return the DHCPv6 client port.
pub fn client_port() -> u16 {
    DHCPV6_CLIENT_PORT
}

// ---------------------------------------------------------------------------
// UDP socket helpers
// ---------------------------------------------------------------------------

/// Open a DHCPv6 client UDP socket bound to the link-local address on the
/// given interface. Returns the raw file descriptor.
///
/// The socket is bound to `[::]:546` (any address, client port) with
/// `SO_BINDTODEVICE` to restrict to the specified interface.
pub fn open_dhcpv6_socket(ifname: &str) -> Result<i32, String> {
    use std::mem;

    unsafe {
        let fd = libc::socket(libc::AF_INET6, libc::SOCK_DGRAM | libc::SOCK_NONBLOCK, 0);
        if fd < 0 {
            return Err(format!(
                "socket(AF_INET6, SOCK_DGRAM): {}",
                std::io::Error::last_os_error()
            ));
        }

        // SO_REUSEADDR.
        let one: libc::c_int = 1;
        libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_REUSEADDR,
            &one as *const _ as *const libc::c_void,
            mem::size_of::<libc::c_int>() as libc::socklen_t,
        );

        // SO_BINDTODEVICE.
        let ifname_bytes = ifname.as_bytes();
        if libc::setsockopt(
            fd,
            libc::SOL_SOCKET,
            libc::SO_BINDTODEVICE,
            ifname_bytes.as_ptr() as *const libc::c_void,
            ifname_bytes.len() as libc::socklen_t,
        ) < 0
        {
            let err = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(format!("SO_BINDTODEVICE({ifname}): {err}"));
        }

        // IPV6_V6ONLY — only IPv6 on this socket.
        libc::setsockopt(
            fd,
            libc::IPPROTO_IPV6,
            libc::IPV6_V6ONLY,
            &one as *const _ as *const libc::c_void,
            mem::size_of::<libc::c_int>() as libc::socklen_t,
        );

        // Bind to [::]:546.
        let mut addr: libc::sockaddr_in6 = mem::zeroed();
        addr.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        addr.sin6_port = DHCPV6_CLIENT_PORT.to_be();
        addr.sin6_addr = libc::in6_addr { s6_addr: [0; 16] };

        if libc::bind(
            fd,
            &addr as *const libc::sockaddr_in6 as *const libc::sockaddr,
            mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
        ) < 0
        {
            let err = std::io::Error::last_os_error();
            libc::close(fd);
            return Err(format!("bind([::]:546): {err}"));
        }

        Ok(fd)
    }
}

/// Send a DHCPv6 message to the all-servers multicast address on the given
/// interface (identified by its scope_id = ifindex).
pub fn send_dhcpv6(fd: i32, ifindex: u32, data: &[u8]) -> Result<(), String> {
    use std::mem;

    unsafe {
        let mut dest: libc::sockaddr_in6 = mem::zeroed();
        dest.sin6_family = libc::AF_INET6 as libc::sa_family_t;
        dest.sin6_port = DHCPV6_SERVER_PORT.to_be();
        dest.sin6_scope_id = ifindex;
        dest.sin6_addr = libc::in6_addr {
            s6_addr: ALL_DHCP_RELAY_AGENTS_AND_SERVERS.octets(),
        };

        let ret = libc::sendto(
            fd,
            data.as_ptr() as *const libc::c_void,
            data.len(),
            0,
            &dest as *const libc::sockaddr_in6 as *const libc::sockaddr,
            mem::size_of::<libc::sockaddr_in6>() as libc::socklen_t,
        );

        if ret < 0 {
            return Err(format!(
                "sendto(ff02::1:2): {}",
                std::io::Error::last_os_error()
            ));
        }

        Ok(())
    }
}

/// Try to receive a DHCPv6 message from the socket.
/// Returns `None` if no data is available (EAGAIN/EWOULDBLOCK).
pub fn recv_dhcpv6(fd: i32) -> Option<Vec<u8>> {
    let mut buf = vec![0u8; 65536];

    unsafe {
        let n = libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0);
        if n <= 0 {
            return None;
        }
        buf.truncate(n as usize);
        Some(buf)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- DUID tests --

    #[test]
    fn test_duid_from_mac() {
        let mac = [0x52, 0x54, 0x00, 0x12, 0x34, 0x56];
        let duid = Duid::from_mac(&mac);
        assert_eq!(duid.duid_type(), Some(DUID_LL));
        assert_eq!(duid.data.len(), 10); // 2 type + 2 hwtype + 6 mac
        // Type field.
        assert_eq!(u16::from_be_bytes([duid.data[0], duid.data[1]]), DUID_LL);
        // Hardware type field.
        assert_eq!(
            u16::from_be_bytes([duid.data[2], duid.data[3]]),
            HW_TYPE_ETHERNET
        );
        // MAC bytes.
        assert_eq!(&duid.data[4..10], &mac);
    }

    #[test]
    fn fuzz_dhcpv6_parsers_never_panic() {
        // Robustness net (task #22): DHCPv6 reply/advertise parsing consumes
        // attacker-controlled server packets. Dhcpv6Message::parse and its
        // option sub-parsers must never panic or hang on malformed input. The
        // sub-parsers are also driven directly, so deep paths are exercised
        // even when the top-level parse rejects random bytes early.
        let handle = std::thread::spawn(|| {
            let mut state: u64 = 0xd6cf_1234_abcd_5eed;
            let mut next = || {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u32
            };
            for _ in 0..50_000u32 {
                let len = (next() % 256) as usize;
                let input: Vec<u8> = (0..len).map(|_| (next() & 0xff) as u8).collect();
                let buf = input.clone();
                let res = std::panic::catch_unwind(move || {
                    let _ = Dhcpv6Message::parse(&input);
                    let _ = parse_options(&input);
                    let _ = parse_ia_na(&input);
                    let _ = parse_ia_pd(&input);
                    let _ = parse_ia_addr(&input);
                    let _ = parse_ia_prefix(&input);
                    let _ = parse_ipv6_list(&input);
                    let _ = parse_dns_labels(&input);
                });
                assert!(res.is_ok(), "dhcpv6 parser panicked on: {buf:?}");
            }
        });
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(30);
        while !handle.is_finished() {
            assert!(
                std::time::Instant::now() < deadline,
                "dhcpv6 fuzz did not finish in 30s -- a malformed packet hangs a parser"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        handle.join().expect("fuzz worker panicked");
    }

    #[test]
    fn test_duid_from_mac_time() {
        let mac = [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff];
        let duid = Duid::from_mac_time(&mac, 1000);
        assert_eq!(duid.duid_type(), Some(DUID_LLT));
        assert_eq!(duid.data.len(), 14); // 2 type + 2 hwtype + 4 time + 6 mac
        let time = u32::from_be_bytes([duid.data[4], duid.data[5], duid.data[6], duid.data[7]]);
        assert_eq!(time, 1000);
        assert_eq!(&duid.data[8..14], &mac);
    }

    #[test]
    fn test_duid_display() {
        let duid = Duid::from_bytes(vec![0x00, 0x03, 0x00, 0x01, 0xaa, 0xbb]);
        assert_eq!(format!("{duid}"), "00:03:00:01:aa:bb");
    }

    #[test]
    fn test_duid_from_bytes() {
        let data = vec![1, 2, 3, 4];
        let duid = Duid::from_bytes(data.clone());
        assert_eq!(duid.as_bytes(), &data);
    }

    #[test]
    fn test_duid_type_too_short() {
        let duid = Duid::from_bytes(vec![0x01]);
        assert_eq!(duid.duid_type(), None);
    }

    // -- IA_NA tests --

    #[test]
    fn test_ia_na_new() {
        let ia = IaNa::new(42);
        assert_eq!(ia.iaid, 42);
        assert_eq!(ia.t1, 0);
        assert_eq!(ia.t2, 0);
        assert!(ia.addresses.is_empty());
        assert!(ia.status.is_none());
    }

    #[test]
    fn test_ia_na_display() {
        let mut ia = IaNa::new(1);
        ia.t1 = 1800;
        ia.t2 = 2880;
        ia.addresses.push(IaAddress {
            address: "2001:db8::1".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        });
        let s = format!("{ia}");
        assert!(s.contains("IA_NA"));
        assert!(s.contains("T1=1800s"));
        assert!(s.contains("T2=2880s"));
        assert!(s.contains("2001:db8::1"));
    }

    #[test]
    fn test_ia_na_display_with_status() {
        let mut ia = IaNa::new(1);
        ia.status = Some((2, "NoAddrsAvail".to_string()));
        let s = format!("{ia}");
        assert!(s.contains("status=2:NoAddrsAvail"));
    }

    // -- IaAddress tests --

    #[test]
    fn test_ia_address_display() {
        let a = IaAddress {
            address: "2001:db8::1".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        };
        let s = format!("{a}");
        assert!(s.contains("2001:db8::1"));
        assert!(s.contains("preferred=3600s"));
        assert!(s.contains("valid=7200s"));
    }

    // -- Dhcpv6Lease tests --

    fn make_test_lease() -> Dhcpv6Lease {
        Dhcpv6Lease {
            ia_na: IaNa {
                iaid: 1,
                t1: 1800,
                t2: 2880,
                addresses: vec![IaAddress {
                    address: "2001:db8::1".parse().unwrap(),
                    preferred_lifetime: 3600,
                    valid_lifetime: 7200,
                }],
                status: None,
            },
            ia_pd: None,
            server_id: Duid::from_bytes(vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]),
            dns_servers: vec!["2001:4860:4860::8888".parse().unwrap()],
            domains: vec!["example.com".to_string()],
            preference: 128,
            obtained_at: Instant::now(),
        }
    }

    #[test]
    fn test_lease_t1() {
        let lease = make_test_lease();
        assert_eq!(lease.t1(), Duration::from_secs(1800));
    }

    #[test]
    fn test_lease_t2() {
        let lease = make_test_lease();
        assert_eq!(lease.t2(), Duration::from_secs(2880));
    }

    #[test]
    fn test_lease_t1_default_when_zero() {
        let mut lease = make_test_lease();
        lease.ia_na.t1 = 0;
        // Default: 0.5 * preferred_lifetime = 1800.
        assert_eq!(lease.t1(), Duration::from_secs(1800));
    }

    #[test]
    fn test_lease_t2_default_when_zero() {
        let mut lease = make_test_lease();
        lease.ia_na.t2 = 0;
        // Default: 0.8 * preferred_lifetime = 2880.
        assert_eq!(lease.t2(), Duration::from_secs(2880));
    }

    #[test]
    fn test_lease_t1_infinity() {
        let mut lease = make_test_lease();
        lease.ia_na.t1 = 0xFFFFFFFF;
        // Should use default calculation.
        assert_eq!(lease.t1(), Duration::from_secs(1800));
    }

    #[test]
    fn test_lease_not_expired_initially() {
        let lease = make_test_lease();
        assert!(!lease.is_expired());
    }

    #[test]
    fn test_lease_not_needs_renewal_initially() {
        let lease = make_test_lease();
        assert!(!lease.needs_renewal());
    }

    #[test]
    fn test_lease_primary_address() {
        let lease = make_test_lease();
        assert_eq!(
            lease.primary_address(),
            Some("2001:db8::1".parse().unwrap())
        );
    }

    #[test]
    fn test_lease_primary_address_empty() {
        let mut lease = make_test_lease();
        lease.ia_na.addresses.clear();
        assert_eq!(lease.primary_address(), None);
    }

    #[test]
    fn test_lease_display() {
        let lease = make_test_lease();
        let s = format!("{lease}");
        assert!(s.contains("DHCPv6 lease"));
        assert!(s.contains("2001:db8::1"));
        assert!(s.contains("T1=1800s"));
    }

    #[test]
    fn test_lease_remaining() {
        let lease = make_test_lease();
        let remaining = lease.remaining();
        // Should be close to valid_lifetime (7200s) since we just created it.
        assert!(remaining.as_secs() >= 7190);
    }

    // -- Dhcpv6State tests --

    #[test]
    fn test_state_display() {
        assert_eq!(format!("{}", Dhcpv6State::Init), "INIT");
        assert_eq!(format!("{}", Dhcpv6State::Soliciting), "SOLICITING");
        assert_eq!(format!("{}", Dhcpv6State::Requesting), "REQUESTING");
        assert_eq!(format!("{}", Dhcpv6State::Bound), "BOUND");
        assert_eq!(format!("{}", Dhcpv6State::Renewing), "RENEWING");
        assert_eq!(format!("{}", Dhcpv6State::Rebinding), "REBINDING");
        assert_eq!(
            format!("{}", Dhcpv6State::InformationRequesting),
            "INFORMATION-REQUESTING"
        );
        assert_eq!(
            format!("{}", Dhcpv6State::InformationReceived),
            "INFORMATION-RECEIVED"
        );
    }

    // -- Message serialization/parsing tests --

    #[test]
    fn test_message_serialize_and_parse() {
        let mut msg = Dhcpv6Message::new(MSG_SOLICIT, [0x11, 0x22, 0x33]);
        msg.add_option(
            OPT_CLIENTID,
            vec![0, 3, 0, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
        );
        msg.add_option(OPT_ELAPSED_TIME, vec![0, 0]);

        let data = msg.serialize();
        assert_eq!(data[0], MSG_SOLICIT);
        assert_eq!(&data[1..4], &[0x11, 0x22, 0x33]);

        let parsed = Dhcpv6Message::parse(&data).unwrap();
        assert_eq!(parsed.msg_type, MSG_SOLICIT);
        assert_eq!(parsed.transaction_id, [0x11, 0x22, 0x33]);
        assert_eq!(parsed.options.len(), 2);
        assert_eq!(parsed.options[0].code, OPT_CLIENTID);
        assert_eq!(parsed.options[1].code, OPT_ELAPSED_TIME);
    }

    #[test]
    fn test_message_parse_too_short() {
        assert!(Dhcpv6Message::parse(&[1, 2]).is_none());
        assert!(Dhcpv6Message::parse(&[1, 2, 3]).is_none());
    }

    #[test]
    fn test_message_parse_no_options() {
        let msg = Dhcpv6Message::parse(&[MSG_REPLY, 0xaa, 0xbb, 0xcc]).unwrap();
        assert_eq!(msg.msg_type, MSG_REPLY);
        assert_eq!(msg.transaction_id, [0xaa, 0xbb, 0xcc]);
        assert!(msg.options.is_empty());
    }

    #[test]
    fn test_message_parse_truncated_option() {
        // Option header says 10 bytes but only 3 bytes of data follow.
        let data = [MSG_REPLY, 0, 0, 0, 0, 1, 0, 10, 1, 2, 3];
        let msg = Dhcpv6Message::parse(&data).unwrap();
        // Truncated option is skipped, no options returned.
        assert!(msg.options.is_empty());
    }

    #[test]
    fn test_message_server_id() {
        let mut msg = Dhcpv6Message::new(MSG_ADVERTISE, [0; 3]);
        msg.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        let sid = msg.server_id().unwrap();
        assert_eq!(sid.duid_type(), Some(DUID_LL));
    }

    #[test]
    fn test_message_client_id() {
        let mut msg = Dhcpv6Message::new(MSG_SOLICIT, [0; 3]);
        let duid = Duid::from_mac(&[0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff]);
        msg.add_option(OPT_CLIENTID, duid.data.clone());
        let cid = msg.client_id().unwrap();
        assert_eq!(cid, duid);
    }

    #[test]
    fn test_message_preference() {
        let mut msg = Dhcpv6Message::new(MSG_ADVERTISE, [0; 3]);
        assert_eq!(msg.preference(), 0); // Default.
        msg.add_option(OPT_PREFERENCE, vec![200]);
        assert_eq!(msg.preference(), 200);
    }

    #[test]
    fn test_message_status_code() {
        let mut msg = Dhcpv6Message::new(MSG_REPLY, [0; 3]);
        assert!(msg.status_code().is_none());

        let mut status_data = vec![0, 0]; // Success.
        status_data.extend_from_slice(b"success");
        msg.add_option(OPT_STATUS_CODE, status_data);
        let (code, message) = msg.status_code().unwrap();
        assert_eq!(code, STATUS_SUCCESS);
        assert_eq!(message, "success");
    }

    #[test]
    fn test_message_rapid_commit() {
        let mut msg = Dhcpv6Message::new(MSG_SOLICIT, [0; 3]);
        assert!(!msg.has_rapid_commit());
        msg.add_option(OPT_RAPID_COMMIT, Vec::new());
        assert!(msg.has_rapid_commit());
    }

    #[test]
    fn test_message_display() {
        let msg = Dhcpv6Message::new(MSG_SOLICIT, [0x12, 0x34, 0x56]);
        let s = format!("{msg}");
        assert!(s.contains("SOLICIT"));
        assert!(s.contains("123456"));
    }

    // -- IA_NA parsing tests --

    #[test]
    fn test_parse_ia_na_basic() {
        let mut data = Vec::new();
        // IAID = 1.
        data.extend_from_slice(&1u32.to_be_bytes());
        // T1 = 1800.
        data.extend_from_slice(&1800u32.to_be_bytes());
        // T2 = 2880.
        data.extend_from_slice(&2880u32.to_be_bytes());

        let ia = parse_ia_na(&data).unwrap();
        assert_eq!(ia.iaid, 1);
        assert_eq!(ia.t1, 1800);
        assert_eq!(ia.t2, 2880);
        assert!(ia.addresses.is_empty());
    }

    #[test]
    fn test_parse_ia_na_with_address() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes()); // IAID
        data.extend_from_slice(&1800u32.to_be_bytes()); // T1
        data.extend_from_slice(&2880u32.to_be_bytes()); // T2

        // IA Address sub-option.
        let addr: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let mut ia_addr_data = Vec::new();
        ia_addr_data.extend_from_slice(&addr.octets());
        ia_addr_data.extend_from_slice(&3600u32.to_be_bytes()); // preferred
        ia_addr_data.extend_from_slice(&7200u32.to_be_bytes()); // valid

        data.extend_from_slice(&OPT_IAADDR.to_be_bytes());
        data.extend_from_slice(&(ia_addr_data.len() as u16).to_be_bytes());
        data.extend_from_slice(&ia_addr_data);

        let ia = parse_ia_na(&data).unwrap();
        assert_eq!(ia.addresses.len(), 1);
        assert_eq!(ia.addresses[0].address, addr);
        assert_eq!(ia.addresses[0].preferred_lifetime, 3600);
        assert_eq!(ia.addresses[0].valid_lifetime, 7200);
    }

    #[test]
    fn test_parse_ia_na_too_short() {
        assert!(parse_ia_na(&[0; 11]).is_none());
    }

    #[test]
    fn test_parse_ia_na_with_status() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes()); // IAID
        data.extend_from_slice(&0u32.to_be_bytes()); // T1
        data.extend_from_slice(&0u32.to_be_bytes()); // T2

        // Status Code sub-option: NoAddrsAvail (2).
        let mut status_data = vec![0, 2]; // code = 2
        status_data.extend_from_slice(b"no addresses");
        data.extend_from_slice(&OPT_STATUS_CODE.to_be_bytes());
        data.extend_from_slice(&(status_data.len() as u16).to_be_bytes());
        data.extend_from_slice(&status_data);

        let ia = parse_ia_na(&data).unwrap();
        assert!(ia.addresses.is_empty());
        let (code, msg) = ia.status.unwrap();
        assert_eq!(code, STATUS_NO_ADDRS_AVAIL);
        assert_eq!(msg, "no addresses");
    }

    // -- IA Address parsing tests --

    #[test]
    fn test_parse_ia_addr_basic() {
        let addr: Ipv6Addr = "2001:db8::42".parse().unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&addr.octets());
        data.extend_from_slice(&3600u32.to_be_bytes());
        data.extend_from_slice(&7200u32.to_be_bytes());

        let ia_addr = parse_ia_addr(&data).unwrap();
        assert_eq!(ia_addr.address, addr);
        assert_eq!(ia_addr.preferred_lifetime, 3600);
        assert_eq!(ia_addr.valid_lifetime, 7200);
    }

    #[test]
    fn test_parse_ia_addr_too_short() {
        assert!(parse_ia_addr(&[0; 23]).is_none());
    }

    #[test]
    fn test_parse_ia_addr_infinity() {
        let addr: Ipv6Addr = "fe80::1".parse().unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&addr.octets());
        data.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());
        data.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());

        let ia_addr = parse_ia_addr(&data).unwrap();
        assert_eq!(ia_addr.preferred_lifetime, 0xFFFFFFFF);
        assert_eq!(ia_addr.valid_lifetime, 0xFFFFFFFF);
    }

    // -- DNS option parsing tests --

    #[test]
    fn test_parse_ipv6_list_single() {
        let addr: Ipv6Addr = "2001:4860:4860::8888".parse().unwrap();
        let addrs = parse_ipv6_list(&addr.octets());
        assert_eq!(addrs.len(), 1);
        assert_eq!(addrs[0], addr);
    }

    #[test]
    fn test_parse_ipv6_list_multiple() {
        let a1: Ipv6Addr = "2001:4860:4860::8888".parse().unwrap();
        let a2: Ipv6Addr = "2001:4860:4860::8844".parse().unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&a1.octets());
        data.extend_from_slice(&a2.octets());
        let addrs = parse_ipv6_list(&data);
        assert_eq!(addrs.len(), 2);
        assert_eq!(addrs[0], a1);
        assert_eq!(addrs[1], a2);
    }

    #[test]
    fn test_parse_ipv6_list_empty() {
        assert!(parse_ipv6_list(&[]).is_empty());
    }

    #[test]
    fn test_parse_ipv6_list_truncated() {
        // 20 bytes = 1 full address + 4 extra bytes (not enough for second).
        let mut data = vec![0u8; 20];
        data[15] = 1; // Make the first address non-zero.
        let addrs = parse_ipv6_list(&data);
        assert_eq!(addrs.len(), 1);
    }

    #[test]
    fn test_parse_dns_labels_single() {
        // "example.com" encoded as DNS labels.
        let data = [
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ];
        let domains = parse_dns_labels(&data);
        assert_eq!(domains, vec!["example.com"]);
    }

    #[test]
    fn test_parse_dns_labels_multiple() {
        let mut data = Vec::new();
        // "example.com"
        data.extend_from_slice(&[
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ]);
        // "test.org"
        data.extend_from_slice(&[4, b't', b'e', b's', b't', 3, b'o', b'r', b'g', 0]);
        let domains = parse_dns_labels(&data);
        assert_eq!(domains, vec!["example.com", "test.org"]);
    }

    #[test]
    fn test_parse_dns_labels_empty() {
        assert!(parse_dns_labels(&[]).is_empty());
    }

    #[test]
    fn test_parse_dns_labels_root_only() {
        let data = [0]; // Just the root label.
        let domains = parse_dns_labels(&data);
        assert!(domains.is_empty());
    }

    #[test]
    fn test_parse_dns_labels_single_label() {
        let data = [4, b't', b'e', b's', b't', 0];
        let domains = parse_dns_labels(&data);
        assert_eq!(domains, vec!["test"]);
    }

    // -- Status code tests --

    #[test]
    fn test_status_code_name() {
        assert_eq!(status_code_name(STATUS_SUCCESS), "Success");
        assert_eq!(status_code_name(STATUS_UNSPEC_FAIL), "UnspecFail");
        assert_eq!(status_code_name(STATUS_NO_ADDRS_AVAIL), "NoAddrsAvail");
        assert_eq!(status_code_name(STATUS_NO_BINDING), "NoBinding");
        assert_eq!(status_code_name(STATUS_NOT_ON_LINK), "NotOnLink");
        assert_eq!(status_code_name(STATUS_USE_MULTICAST), "UseMulticast");
        assert_eq!(status_code_name(STATUS_NO_PREFIX_AVAIL), "NoPrefixAvail");
        assert_eq!(status_code_name(99), "Unknown");
    }

    #[test]
    fn test_parse_status_code_option_success() {
        let opt = Dhcpv6Option {
            code: OPT_STATUS_CODE,
            data: vec![0, 0, b'O', b'K'],
        };
        let (code, msg) = parse_status_code_option(&opt).unwrap();
        assert_eq!(code, STATUS_SUCCESS);
        assert_eq!(msg, "OK");
    }

    #[test]
    fn test_parse_status_code_option_no_message() {
        let opt = Dhcpv6Option {
            code: OPT_STATUS_CODE,
            data: vec![0, 2],
        };
        let (code, msg) = parse_status_code_option(&opt).unwrap();
        assert_eq!(code, STATUS_NO_ADDRS_AVAIL);
        assert_eq!(msg, "");
    }

    #[test]
    fn test_parse_status_code_option_too_short() {
        let opt = Dhcpv6Option {
            code: OPT_STATUS_CODE,
            data: vec![0],
        };
        assert!(parse_status_code_option(&opt).is_none());
    }

    // -- Message type name tests --

    #[test]
    fn test_message_type_names() {
        assert_eq!(dhcpv6_message_type_name(MSG_SOLICIT), "SOLICIT");
        assert_eq!(dhcpv6_message_type_name(MSG_ADVERTISE), "ADVERTISE");
        assert_eq!(dhcpv6_message_type_name(MSG_REQUEST), "REQUEST");
        assert_eq!(dhcpv6_message_type_name(MSG_CONFIRM), "CONFIRM");
        assert_eq!(dhcpv6_message_type_name(MSG_RENEW), "RENEW");
        assert_eq!(dhcpv6_message_type_name(MSG_REBIND), "REBIND");
        assert_eq!(dhcpv6_message_type_name(MSG_REPLY), "REPLY");
        assert_eq!(dhcpv6_message_type_name(MSG_RELEASE), "RELEASE");
        assert_eq!(dhcpv6_message_type_name(MSG_DECLINE), "DECLINE");
        assert_eq!(dhcpv6_message_type_name(MSG_RECONFIGURE), "RECONFIGURE");
        assert_eq!(
            dhcpv6_message_type_name(MSG_INFORMATION_REQUEST),
            "INFORMATION-REQUEST"
        );
        assert_eq!(dhcpv6_message_type_name(200), "UNKNOWN");
    }

    // -- Retransmit timeout tests --

    #[test]
    fn test_retransmit_timeout_basic() {
        let rt = retransmit_timeout(Duration::from_secs(1), Duration::from_secs(60));
        assert_eq!(rt, Duration::from_secs(2));
    }

    #[test]
    fn test_retransmit_timeout_doubling() {
        let rt1 = retransmit_timeout(Duration::from_secs(1), Duration::from_secs(60));
        let rt2 = retransmit_timeout(rt1, Duration::from_secs(60));
        let rt3 = retransmit_timeout(rt2, Duration::from_secs(60));
        assert_eq!(rt1, Duration::from_secs(2));
        assert_eq!(rt2, Duration::from_secs(4));
        assert_eq!(rt3, Duration::from_secs(8));
    }

    #[test]
    fn test_retransmit_timeout_capped_at_max() {
        let rt = retransmit_timeout(Duration::from_secs(40), Duration::from_secs(60));
        assert_eq!(rt, Duration::from_secs(60)); // Capped.
    }

    #[test]
    fn test_retransmit_timeout_already_at_max() {
        let rt = retransmit_timeout(Duration::from_secs(60), Duration::from_secs(60));
        assert_eq!(rt, Duration::from_secs(60));
    }

    // -- Transaction ID tests --

    #[test]
    fn test_generate_transaction_id_nonzero() {
        // With /dev/urandom available, should get some non-zero bytes
        // (extremely unlikely to get all zeros from random).
        let tid1 = generate_transaction_id();
        let tid2 = generate_transaction_id();
        // They should be different (again, extremely unlikely to be the same).
        // Don't assert this strictly as it's probabilistic,
        // but at least check the function doesn't panic.
        assert_eq!(tid1.len(), 3);
        assert_eq!(tid2.len(), 3);
    }

    // -- ORO building tests --

    #[test]
    fn test_build_oro_empty() {
        let data = build_oro(&[]);
        assert!(data.is_empty());
    }

    #[test]
    fn test_build_oro_single() {
        let data = build_oro(&[OPT_DNS_SERVERS]);
        assert_eq!(data, vec![0, 23]); // OPT_DNS_SERVERS = 23.
    }

    #[test]
    fn test_build_oro_multiple() {
        let data = build_oro(&[OPT_DNS_SERVERS, OPT_DOMAIN_LIST]);
        assert_eq!(data.len(), 4);
        assert_eq!(u16::from_be_bytes([data[0], data[1]]), OPT_DNS_SERVERS);
        assert_eq!(u16::from_be_bytes([data[2], data[3]]), OPT_DOMAIN_LIST);
    }

    // -- IA_NA option building tests --

    #[test]
    fn test_build_ia_na_option_empty() {
        let data = build_ia_na_option(42, 1800, 2880, &[]);
        assert_eq!(data.len(), 12);
        assert_eq!(u32::from_be_bytes([data[0], data[1], data[2], data[3]]), 42);
        assert_eq!(
            u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            1800
        );
        assert_eq!(
            u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
            2880
        );
    }

    #[test]
    fn test_build_ia_na_option_with_address() {
        let addr = IaAddress {
            address: "2001:db8::1".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        };
        let data = build_ia_na_option(1, 0, 0, &[&addr]);
        // 12 bytes header + 4 bytes sub-option header + 24 bytes IA addr = 40.
        assert_eq!(data.len(), 40);
        // Sub-option code should be OPT_IAADDR (5).
        assert_eq!(u16::from_be_bytes([data[12], data[13]]), OPT_IAADDR);
        // Sub-option length should be 24.
        assert_eq!(u16::from_be_bytes([data[14], data[15]]), 24);
    }

    // -- Client tests --

    fn make_test_config() -> Dhcpv6ClientConfig {
        Dhcpv6ClientConfig {
            ifindex: 2,
            ifname: "eth0".to_string(),
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            request_addresses: true,
            request_prefix_delegation: false,
            prefix_hint: 0,
            prefix_hint_address: Ipv6Addr::UNSPECIFIED,
            rapid_commit: false,
            max_attempts: 0,
            request_options: vec![OPT_DNS_SERVERS, OPT_DOMAIN_LIST],
        }
    }

    fn make_stateless_config() -> Dhcpv6ClientConfig {
        let mut config = make_test_config();
        config.request_addresses = false;
        config
    }

    #[test]
    fn test_client_new_stateful() {
        let client = Dhcpv6Client::new(make_test_config());
        assert_eq!(client.state, Dhcpv6State::Init);
        assert!(client.lease.is_none());
        assert!(client.best_advertise.is_none());
        assert_eq!(client.iaid, 2);
        assert_eq!(client.attempts, 0);
    }

    #[test]
    fn test_client_new_stateless() {
        let client = Dhcpv6Client::new(make_stateless_config());
        assert_eq!(client.state, Dhcpv6State::Init);
    }

    #[test]
    fn test_client_init_sends_solicit() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let pkt = client.next_packet().unwrap();
        assert_eq!(client.state, Dhcpv6State::Soliciting);
        assert_eq!(client.attempts, 1);
        // Parse the packet and verify it's a Solicit.
        let msg = Dhcpv6Message::parse(&pkt).unwrap();
        assert_eq!(msg.msg_type, MSG_SOLICIT);
        // Should have Client ID, Elapsed Time, IA_NA, ORO.
        assert!(msg.find_option(OPT_CLIENTID).is_some());
        assert!(msg.find_option(OPT_ELAPSED_TIME).is_some());
        assert!(msg.find_option(OPT_IA_NA).is_some());
        assert!(msg.find_option(OPT_ORO).is_some());
    }

    #[test]
    fn test_client_solicit_retransmit() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let _pkt1 = client.next_packet().unwrap();
        assert_eq!(client.state, Dhcpv6State::Soliciting);
        assert_eq!(client.attempts, 1);

        let _pkt2 = client.next_packet().unwrap();
        assert_eq!(client.state, Dhcpv6State::Soliciting);
        assert_eq!(client.attempts, 2);
    }

    #[test]
    fn test_client_stateless_sends_information_request() {
        let mut client = Dhcpv6Client::new(make_stateless_config());
        let pkt = client.next_packet().unwrap();
        assert_eq!(client.state, Dhcpv6State::InformationRequesting);
        let msg = Dhcpv6Message::parse(&pkt).unwrap();
        assert_eq!(msg.msg_type, MSG_INFORMATION_REQUEST);
        assert!(msg.find_option(OPT_CLIENTID).is_some());
        assert!(msg.find_option(OPT_ORO).is_some());
        // Should NOT have IA_NA.
        assert!(msg.find_option(OPT_IA_NA).is_none());
    }

    #[test]
    fn test_client_processes_advertise() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();

        // Build an Advertise response.
        let advertise = build_test_advertise(&solicit, &client.client_duid);
        let adv_data = advertise.serialize();

        let result = client.process_reply(&adv_data);
        assert!(result.is_none()); // Advertise doesn't return a lease.
        assert_eq!(client.state, Dhcpv6State::Requesting);
        assert!(client.best_advertise.is_some());
    }

    #[test]
    fn test_client_processes_reply_after_request() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();

        // Advertise.
        let advertise = build_test_advertise(&solicit, &client.client_duid);
        client.process_reply(&advertise.serialize());
        assert_eq!(client.state, Dhcpv6State::Requesting);

        // Get the Request packet.
        let req_pkt = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&req_pkt).unwrap();
        assert_eq!(request.msg_type, MSG_REQUEST);

        // Build a Reply.
        let reply = build_test_reply(&request, &client.client_duid);
        let lease = client.process_reply(&reply.serialize());
        assert!(lease.is_some());
        assert_eq!(client.state, Dhcpv6State::Bound);
        let lease = lease.unwrap();
        assert_eq!(lease.ia_na.addresses.len(), 1);
        assert_eq!(
            lease.ia_na.addresses[0].address,
            "2001:db8::100".parse::<Ipv6Addr>().unwrap()
        );
    }

    #[test]
    fn test_client_wrong_tid_ignored() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let _pkt = client.next_packet().unwrap();

        // Build a reply with a different transaction ID.
        let mut reply = Dhcpv6Message::new(MSG_ADVERTISE, [0xFF, 0xFE, 0xFD]);
        reply.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);

        let result = client.process_reply(&reply.serialize());
        assert!(result.is_none());
        assert_eq!(client.state, Dhcpv6State::Soliciting); // Still soliciting.
    }

    #[test]
    fn test_client_wrong_client_id_ignored() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();

        // Build an Advertise with wrong client ID.
        let mut adv = Dhcpv6Message::new(MSG_ADVERTISE, solicit.transaction_id);
        adv.add_option(
            OPT_CLIENTID,
            Duid::from_mac(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]).data,
        );
        adv.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);

        let result = client.process_reply(&adv.serialize());
        assert!(result.is_none());
        assert_eq!(client.state, Dhcpv6State::Soliciting);
    }

    #[test]
    fn test_client_bound_no_packet() {
        let mut client = Dhcpv6Client::new(make_test_config());
        // Manually set to bound with a fresh lease.
        client.state = Dhcpv6State::Bound;
        client.lease = Some(make_test_lease());
        let pkt = client.next_packet();
        assert!(pkt.is_none()); // Not time to renew yet.
    }

    #[test]
    fn test_client_build_release() {
        let mut client = Dhcpv6Client::new(make_test_config());
        assert!(client.build_release().is_none()); // No lease yet.

        client.lease = Some(make_test_lease());
        let release_data = client.build_release().unwrap();
        let msg = Dhcpv6Message::parse(&release_data).unwrap();
        assert_eq!(msg.msg_type, MSG_RELEASE);
        assert!(msg.find_option(OPT_CLIENTID).is_some());
        assert!(msg.find_option(OPT_SERVERID).is_some());
        assert!(msg.find_option(OPT_IA_NA).is_some());
    }

    #[test]
    fn test_client_max_attempts_not_reached() {
        let mut client = Dhcpv6Client::new(make_test_config());
        client.attempts = 5;
        assert!(!client.max_attempts_reached()); // max_attempts = 0 means unlimited.
    }

    #[test]
    fn test_client_max_attempts_reached() {
        let mut config = make_test_config();
        config.max_attempts = 3;
        let mut client = Dhcpv6Client::new(config);
        client.attempts = 3;
        assert!(client.max_attempts_reached());
    }

    #[test]
    fn test_client_retransmit_timeout() {
        let client = Dhcpv6Client::new(make_test_config());
        assert_eq!(client.retransmit_timeout(), SOL_TIMEOUT);
    }

    #[test]
    fn test_client_rapid_commit_solicit() {
        let mut config = make_test_config();
        config.rapid_commit = true;
        let mut client = Dhcpv6Client::new(config);
        let pkt = client.next_packet().unwrap();
        let msg = Dhcpv6Message::parse(&pkt).unwrap();
        assert_eq!(msg.msg_type, MSG_SOLICIT);
        assert!(msg.has_rapid_commit());
    }

    #[test]
    fn test_client_rapid_commit_reply() {
        let mut config = make_test_config();
        config.rapid_commit = true;
        let mut client = Dhcpv6Client::new(config);
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();

        // Server replies directly with Reply + Rapid Commit.
        let mut reply = Dhcpv6Message::new(MSG_REPLY, solicit.transaction_id);
        reply.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        reply.add_option(OPT_RAPID_COMMIT, Vec::new());
        // IA_NA with an address.
        let addr = IaAddress {
            address: "2001:db8::200".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        };
        reply.add_option(
            OPT_IA_NA,
            build_ia_na_option(client.iaid, 1800, 2880, &[&addr]),
        );

        let lease = client.process_reply(&reply.serialize());
        assert!(lease.is_some());
        assert_eq!(client.state, Dhcpv6State::Bound);
    }

    #[test]
    fn test_client_stateless_reply() {
        let mut client = Dhcpv6Client::new(make_stateless_config());
        let pkt = client.next_packet().unwrap();
        let info_req = Dhcpv6Message::parse(&pkt).unwrap();

        // Build a Reply with DNS info.
        let mut reply = Dhcpv6Message::new(MSG_REPLY, info_req.transaction_id);
        reply.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        let dns: Ipv6Addr = "2001:4860:4860::8888".parse().unwrap();
        reply.add_option(OPT_DNS_SERVERS, dns.octets().to_vec());
        let domain_data = [
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ];
        reply.add_option(OPT_DOMAIN_LIST, domain_data.to_vec());
        reply.add_option(
            OPT_INFORMATION_REFRESH_TIME,
            86400u32.to_be_bytes().to_vec(),
        );

        let result = client.process_reply(&reply.serialize());
        assert!(result.is_some());
        assert_eq!(client.state, Dhcpv6State::InformationReceived);
        assert_eq!(client.stateless_dns, vec![dns]);
        assert_eq!(client.stateless_domains, vec!["example.com"]);
        assert_eq!(client.info_refresh_time, Some(Duration::from_secs(86400)));
    }

    #[test]
    fn test_client_request_max_retransmissions() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();

        // Receive an Advertise.
        let advertise = build_test_advertise(&solicit, &client.client_duid);
        client.process_reply(&advertise.serialize());
        assert_eq!(client.state, Dhcpv6State::Requesting);

        // Exhaust retransmissions.
        client.attempts = REQ_MAX_RC;
        let result = client.next_packet();
        assert!(result.is_none());
        assert_eq!(client.state, Dhcpv6State::Init);
    }

    #[test]
    fn test_client_status_no_addrs_avail() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();

        // Reply with NoAddrsAvail status.
        let mut reply = Dhcpv6Message::new(MSG_ADVERTISE, solicit.transaction_id);
        reply.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        let mut status_data = Vec::new();
        status_data.extend_from_slice(&STATUS_NO_ADDRS_AVAIL.to_be_bytes());
        status_data.extend_from_slice(b"no addresses");
        reply.add_option(OPT_STATUS_CODE, status_data);

        let result = client.process_reply(&reply.serialize());
        assert!(result.is_none());
        assert_eq!(client.state, Dhcpv6State::Init);
    }

    // -- Elapsed time tests --

    #[test]
    fn test_elapsed_time_initially_zero() {
        let client = Dhcpv6Client::new(make_test_config());
        let val = client.elapsed_time_value();
        assert_eq!(val.len(), 2);
        assert_eq!(u16::from_be_bytes([val[0], val[1]]), 0);
    }

    // -- Constants tests --

    #[test]
    fn test_ports() {
        assert_eq!(server_port(), 547);
        assert_eq!(client_port(), 546);
    }

    #[test]
    fn test_all_dhcp_servers() {
        assert_eq!(all_dhcp_servers(), "ff02::1:2".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_sol_timeout_constants() {
        assert_eq!(SOL_TIMEOUT, Duration::from_secs(1));
        assert_eq!(SOL_MAX_RT, Duration::from_secs(3600));
    }

    #[test]
    fn test_req_constants() {
        assert_eq!(REQ_TIMEOUT, Duration::from_secs(1));
        assert_eq!(REQ_MAX_RT, Duration::from_secs(30));
        assert_eq!(REQ_MAX_RC, 10);
    }

    #[test]
    fn test_ren_constants() {
        assert_eq!(REN_TIMEOUT, Duration::from_secs(10));
        assert_eq!(REN_MAX_RT, Duration::from_secs(600));
    }

    #[test]
    fn test_reb_constants() {
        assert_eq!(REB_TIMEOUT, Duration::from_secs(10));
        assert_eq!(REB_MAX_RT, Duration::from_secs(600));
    }

    #[test]
    fn test_inf_constants() {
        assert_eq!(INF_TIMEOUT, Duration::from_secs(1));
        assert_eq!(INF_MAX_RT, Duration::from_secs(3600));
    }

    #[test]
    fn test_duid_constants() {
        assert_eq!(DUID_LLT, 1);
        assert_eq!(DUID_EN, 2);
        assert_eq!(DUID_LL, 3);
        assert_eq!(DUID_UUID, 4);
    }

    #[test]
    fn test_msg_type_constants() {
        assert_eq!(MSG_SOLICIT, 1);
        assert_eq!(MSG_ADVERTISE, 2);
        assert_eq!(MSG_REQUEST, 3);
        assert_eq!(MSG_CONFIRM, 4);
        assert_eq!(MSG_RENEW, 5);
        assert_eq!(MSG_REBIND, 6);
        assert_eq!(MSG_REPLY, 7);
        assert_eq!(MSG_RELEASE, 8);
        assert_eq!(MSG_DECLINE, 9);
        assert_eq!(MSG_RECONFIGURE, 10);
        assert_eq!(MSG_INFORMATION_REQUEST, 11);
    }

    #[test]
    fn test_option_code_constants() {
        assert_eq!(OPT_CLIENTID, 1);
        assert_eq!(OPT_SERVERID, 2);
        assert_eq!(OPT_IA_NA, 3);
        assert_eq!(OPT_IAADDR, 5);
        assert_eq!(OPT_ORO, 6);
        assert_eq!(OPT_PREFERENCE, 7);
        assert_eq!(OPT_ELAPSED_TIME, 8);
        assert_eq!(OPT_STATUS_CODE, 13);
        assert_eq!(OPT_RAPID_COMMIT, 14);
        assert_eq!(OPT_DNS_SERVERS, 23);
        assert_eq!(OPT_DOMAIN_LIST, 24);
        assert_eq!(OPT_IA_PD, 25);
        assert_eq!(OPT_INFORMATION_REFRESH_TIME, 32);
        assert_eq!(OPT_SOL_MAX_RT, 82);
        assert_eq!(OPT_INF_MAX_RT, 83);
    }

    #[test]
    fn test_default_config() {
        let config = Dhcpv6ClientConfig::default();
        assert_eq!(config.ifindex, 0);
        assert!(config.ifname.is_empty());
        assert!(config.request_addresses);
        assert!(!config.rapid_commit);
        assert_eq!(config.max_attempts, 0);
        assert_eq!(
            config.request_options,
            vec![OPT_DNS_SERVERS, OPT_DOMAIN_LIST]
        );
    }

    // -- Full exchange integration test --

    #[test]
    fn test_full_four_message_exchange() {
        let mut client = Dhcpv6Client::new(make_test_config());

        // 1. Client sends Solicit.
        let solicit_data = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&solicit_data).unwrap();
        assert_eq!(solicit.msg_type, MSG_SOLICIT);
        assert_eq!(client.state, Dhcpv6State::Soliciting);

        // 2. Server sends Advertise.
        let advertise = build_test_advertise(&solicit, &client.client_duid);
        let result = client.process_reply(&advertise.serialize());
        assert!(result.is_none()); // Advertise doesn't produce a lease.
        assert_eq!(client.state, Dhcpv6State::Requesting);

        // 3. Client sends Request.
        let request_data = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&request_data).unwrap();
        assert_eq!(request.msg_type, MSG_REQUEST);
        // Request should include Server ID from the Advertise.
        assert!(request.find_option(OPT_SERVERID).is_some());

        // 4. Server sends Reply.
        let reply = build_test_reply(&request, &client.client_duid);
        let lease = client.process_reply(&reply.serialize());
        assert!(lease.is_some());
        assert_eq!(client.state, Dhcpv6State::Bound);

        let lease = lease.unwrap();
        assert_eq!(lease.ia_na.addresses.len(), 1);
        assert_eq!(
            lease.ia_na.addresses[0].address,
            "2001:db8::100".parse::<Ipv6Addr>().unwrap()
        );
        assert_eq!(lease.dns_servers.len(), 1);
        assert_eq!(
            lease.dns_servers[0],
            "2001:4860:4860::8888".parse::<Ipv6Addr>().unwrap()
        );
        assert_eq!(lease.domains, vec!["example.com"]);

        // 5. While bound, no packet should be emitted.
        assert!(client.next_packet().is_none());
    }

    #[test]
    fn test_reply_without_addresses_in_requesting_state() {
        let mut client = Dhcpv6Client::new(make_test_config());
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();

        // Advertise.
        let advertise = build_test_advertise(&solicit, &client.client_duid);
        client.process_reply(&advertise.serialize());

        // Get Request.
        let req_pkt = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&req_pkt).unwrap();

        // Reply without IA_NA addresses.
        let mut reply = Dhcpv6Message::new(MSG_REPLY, request.transaction_id);
        reply.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        // IA_NA with no addresses.
        reply.add_option(OPT_IA_NA, build_ia_na_option(client.iaid, 0, 0, &[]));

        let lease = client.process_reply(&reply.serialize());
        assert!(lease.is_none());
        assert_eq!(client.state, Dhcpv6State::Init); // Restarts.
    }

    // -- Test helpers --

    /// Build a test Advertise message in response to a Solicit.
    fn build_test_advertise(solicit: &Dhcpv6Message, client_duid: &Duid) -> Dhcpv6Message {
        let mut adv = Dhcpv6Message::new(MSG_ADVERTISE, solicit.transaction_id);
        adv.add_option(OPT_CLIENTID, client_duid.data.clone());
        adv.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        adv.add_option(OPT_PREFERENCE, vec![100]);

        // IA_NA with an offered address.
        let addr = IaAddress {
            address: "2001:db8::100".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        };
        let ia_nas = solicit.parse_ia_nas();
        let iaid = ia_nas.first().map(|ia| ia.iaid).unwrap_or(1);
        adv.add_option(OPT_IA_NA, build_ia_na_option(iaid, 1800, 2880, &[&addr]));

        adv
    }

    /// Build a test Reply message in response to a Request.
    fn build_test_reply(request: &Dhcpv6Message, client_duid: &Duid) -> Dhcpv6Message {
        let mut reply = Dhcpv6Message::new(MSG_REPLY, request.transaction_id);
        reply.add_option(OPT_CLIENTID, client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);

        // IA_NA with the assigned address.
        let addr = IaAddress {
            address: "2001:db8::100".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        };
        let ia_nas = request.parse_ia_nas();
        let iaid = ia_nas.first().map(|ia| ia.iaid).unwrap_or(1);
        reply.add_option(OPT_IA_NA, build_ia_na_option(iaid, 1800, 2880, &[&addr]));

        // DNS servers.
        let dns: Ipv6Addr = "2001:4860:4860::8888".parse().unwrap();
        reply.add_option(OPT_DNS_SERVERS, dns.octets().to_vec());

        // Domain list.
        let domain_data = [
            7, b'e', b'x', b'a', b'm', b'p', b'l', b'e', 3, b'c', b'o', b'm', 0,
        ];
        reply.add_option(OPT_DOMAIN_LIST, domain_data.to_vec());

        reply
    }

    // ================================================================
    // DHCPv6 Prefix Delegation (IA_PD) tests
    // ================================================================

    // -- IaPrefix struct tests --

    #[test]
    fn test_ia_prefix_display() {
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 56,
            prefix: "2001:db8:abcd::".parse().unwrap(),
        };
        let s = format!("{pfx}");
        assert!(s.contains("2001:db8:abcd::/56"));
        assert!(s.contains("preferred=3600s"));
        assert!(s.contains("valid=7200s"));
    }

    // -- IaPd struct tests --

    #[test]
    fn test_ia_pd_new() {
        let pd = IaPd::new(42);
        assert_eq!(pd.iaid, 42);
        assert_eq!(pd.t1, 0);
        assert_eq!(pd.t2, 0);
        assert!(pd.prefixes.is_empty());
        assert!(pd.status.is_none());
    }

    #[test]
    fn test_ia_pd_display() {
        let mut pd = IaPd::new(0x100);
        pd.t1 = 1800;
        pd.t2 = 2880;
        pd.prefixes.push(IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 48,
            prefix: "2001:db8::".parse().unwrap(),
        });
        let s = format!("{pd}");
        assert!(s.contains("IA_PD"));
        assert!(s.contains("T1=1800s"));
        assert!(s.contains("T2=2880s"));
        assert!(s.contains("2001:db8::/48"));
    }

    #[test]
    fn test_ia_pd_display_with_status() {
        let mut pd = IaPd::new(1);
        pd.status = Some((6, "no prefix".to_string()));
        let s = format!("{pd}");
        assert!(s.contains("status=6:no prefix"));
    }

    // -- IA_PD parsing tests --

    #[test]
    fn test_parse_ia_pd_basic() {
        let mut data = Vec::new();
        data.extend_from_slice(&42u32.to_be_bytes()); // IAID
        data.extend_from_slice(&1800u32.to_be_bytes()); // T1
        data.extend_from_slice(&2880u32.to_be_bytes()); // T2

        let pd = parse_ia_pd(&data).unwrap();
        assert_eq!(pd.iaid, 42);
        assert_eq!(pd.t1, 1800);
        assert_eq!(pd.t2, 2880);
        assert!(pd.prefixes.is_empty());
        assert!(pd.status.is_none());
    }

    #[test]
    fn test_parse_ia_pd_with_prefix() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes()); // IAID
        data.extend_from_slice(&900u32.to_be_bytes()); // T1
        data.extend_from_slice(&1440u32.to_be_bytes()); // T2

        // IA Prefix sub-option.
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        let mut ia_prefix_data = Vec::new();
        ia_prefix_data.extend_from_slice(&3600u32.to_be_bytes()); // preferred
        ia_prefix_data.extend_from_slice(&7200u32.to_be_bytes()); // valid
        ia_prefix_data.push(56); // prefix length
        ia_prefix_data.extend_from_slice(&prefix.octets());

        data.extend_from_slice(&OPT_IAPREFIX.to_be_bytes());
        data.extend_from_slice(&(ia_prefix_data.len() as u16).to_be_bytes());
        data.extend_from_slice(&ia_prefix_data);

        let pd = parse_ia_pd(&data).unwrap();
        assert_eq!(pd.prefixes.len(), 1);
        assert_eq!(pd.prefixes[0].prefix, prefix);
        assert_eq!(pd.prefixes[0].prefix_length, 56);
        assert_eq!(pd.prefixes[0].preferred_lifetime, 3600);
        assert_eq!(pd.prefixes[0].valid_lifetime, 7200);
    }

    #[test]
    fn test_parse_ia_pd_with_multiple_prefixes() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());

        // First prefix: 2001:db8:1::/48.
        let pfx1: Ipv6Addr = "2001:db8:1::".parse().unwrap();
        let mut p1_data = Vec::new();
        p1_data.extend_from_slice(&3600u32.to_be_bytes());
        p1_data.extend_from_slice(&7200u32.to_be_bytes());
        p1_data.push(48);
        p1_data.extend_from_slice(&pfx1.octets());
        data.extend_from_slice(&OPT_IAPREFIX.to_be_bytes());
        data.extend_from_slice(&(p1_data.len() as u16).to_be_bytes());
        data.extend_from_slice(&p1_data);

        // Second prefix: 2001:db8:2::/48.
        let pfx2: Ipv6Addr = "2001:db8:2::".parse().unwrap();
        let mut p2_data = Vec::new();
        p2_data.extend_from_slice(&1800u32.to_be_bytes());
        p2_data.extend_from_slice(&3600u32.to_be_bytes());
        p2_data.push(48);
        p2_data.extend_from_slice(&pfx2.octets());
        data.extend_from_slice(&OPT_IAPREFIX.to_be_bytes());
        data.extend_from_slice(&(p2_data.len() as u16).to_be_bytes());
        data.extend_from_slice(&p2_data);

        let pd = parse_ia_pd(&data).unwrap();
        assert_eq!(pd.prefixes.len(), 2);
        assert_eq!(pd.prefixes[0].prefix, pfx1);
        assert_eq!(pd.prefixes[0].prefix_length, 48);
        assert_eq!(pd.prefixes[1].prefix, pfx2);
        assert_eq!(pd.prefixes[1].prefix_length, 48);
    }

    #[test]
    fn test_parse_ia_pd_too_short() {
        assert!(parse_ia_pd(&[0; 11]).is_none());
    }

    #[test]
    fn test_parse_ia_pd_with_status() {
        let mut data = Vec::new();
        data.extend_from_slice(&1u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());
        data.extend_from_slice(&0u32.to_be_bytes());

        // Status Code sub-option: NoPrefixAvail (6).
        let mut status_data = vec![0, 6]; // code = 6
        status_data.extend_from_slice(b"no prefix available");
        data.extend_from_slice(&OPT_STATUS_CODE.to_be_bytes());
        data.extend_from_slice(&(status_data.len() as u16).to_be_bytes());
        data.extend_from_slice(&status_data);

        let pd = parse_ia_pd(&data).unwrap();
        assert!(pd.prefixes.is_empty());
        let (code, msg) = pd.status.unwrap();
        assert_eq!(code, STATUS_NO_PREFIX_AVAIL);
        assert_eq!(msg, "no prefix available");
    }

    // -- IA Prefix parsing tests --

    #[test]
    fn test_parse_ia_prefix_basic() {
        let prefix: Ipv6Addr = "2001:db8:ff00::".parse().unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&3600u32.to_be_bytes()); // preferred
        data.extend_from_slice(&7200u32.to_be_bytes()); // valid
        data.push(48); // prefix length
        data.extend_from_slice(&prefix.octets());

        let pfx = parse_ia_prefix(&data).unwrap();
        assert_eq!(pfx.prefix, prefix);
        assert_eq!(pfx.prefix_length, 48);
        assert_eq!(pfx.preferred_lifetime, 3600);
        assert_eq!(pfx.valid_lifetime, 7200);
    }

    #[test]
    fn test_parse_ia_prefix_too_short() {
        assert!(parse_ia_prefix(&[0; 24]).is_none());
    }

    #[test]
    fn test_parse_ia_prefix_infinity() {
        let prefix: Ipv6Addr = "fd00::".parse().unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());
        data.extend_from_slice(&0xFFFFFFFFu32.to_be_bytes());
        data.push(64);
        data.extend_from_slice(&prefix.octets());

        let pfx = parse_ia_prefix(&data).unwrap();
        assert_eq!(pfx.preferred_lifetime, 0xFFFFFFFF);
        assert_eq!(pfx.valid_lifetime, 0xFFFFFFFF);
        assert_eq!(pfx.prefix_length, 64);
    }

    #[test]
    fn test_parse_ia_prefix_slash_56() {
        let prefix: Ipv6Addr = "2001:db8:abcd:ab00::".parse().unwrap();
        let mut data = Vec::new();
        data.extend_from_slice(&1800u32.to_be_bytes());
        data.extend_from_slice(&3600u32.to_be_bytes());
        data.push(56);
        data.extend_from_slice(&prefix.octets());

        let pfx = parse_ia_prefix(&data).unwrap();
        assert_eq!(pfx.prefix_length, 56);
        assert_eq!(pfx.prefix, prefix);
    }

    // -- IA_PD option building tests --

    #[test]
    fn test_build_ia_pd_option_empty() {
        let data = build_ia_pd_option(42, 1800, 2880, &[]);
        assert_eq!(data.len(), 12);
        assert_eq!(u32::from_be_bytes([data[0], data[1], data[2], data[3]]), 42);
        assert_eq!(
            u32::from_be_bytes([data[4], data[5], data[6], data[7]]),
            1800
        );
        assert_eq!(
            u32::from_be_bytes([data[8], data[9], data[10], data[11]]),
            2880
        );
    }

    #[test]
    fn test_build_ia_pd_option_with_prefix() {
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 56,
            prefix: "2001:db8:abcd::".parse().unwrap(),
        };
        let data = build_ia_pd_option(1, 0, 0, &[&pfx]);
        // 12 bytes header + 4 bytes sub-option header + 25 bytes IA prefix = 41.
        assert_eq!(data.len(), 41);
        // Sub-option code should be OPT_IAPREFIX (26).
        assert_eq!(u16::from_be_bytes([data[12], data[13]]), OPT_IAPREFIX);
        // Sub-option length should be 25.
        assert_eq!(u16::from_be_bytes([data[14], data[15]]), 25);
    }

    #[test]
    fn test_build_ia_pd_option_with_multiple_prefixes() {
        let pfx1 = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 48,
            prefix: "2001:db8:1::".parse().unwrap(),
        };
        let pfx2 = IaPrefix {
            preferred_lifetime: 1800,
            valid_lifetime: 3600,
            prefix_length: 64,
            prefix: "2001:db8:2::".parse().unwrap(),
        };
        let data = build_ia_pd_option(5, 900, 1440, &[&pfx1, &pfx2]);
        // 12 + (4+25) + (4+25) = 70.
        assert_eq!(data.len(), 70);
    }

    #[test]
    fn test_build_ia_prefix_roundtrip() {
        let original = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 56,
            prefix: "2001:db8:abcd::".parse().unwrap(),
        };
        let data = build_ia_prefix(&original);
        let parsed = parse_ia_prefix(&data).unwrap();
        assert_eq!(parsed, original);
    }

    #[test]
    fn test_build_ia_pd_roundtrip() {
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 48,
            prefix: "2001:db8::".parse().unwrap(),
        };
        let data = build_ia_pd_option(99, 1800, 2880, &[&pfx]);
        let parsed = parse_ia_pd(&data).unwrap();
        assert_eq!(parsed.iaid, 99);
        assert_eq!(parsed.t1, 1800);
        assert_eq!(parsed.t2, 2880);
        assert_eq!(parsed.prefixes.len(), 1);
        assert_eq!(parsed.prefixes[0], pfx);
    }

    // -- Message IA_PD parsing tests --

    #[test]
    fn test_message_parse_ia_pds() {
        let mut msg = Dhcpv6Message::new(MSG_REPLY, [0; 3]);
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 56,
            prefix: "2001:db8::".parse().unwrap(),
        };
        msg.add_option(OPT_IA_PD, build_ia_pd_option(1, 900, 1440, &[&pfx]));

        let pds = msg.parse_ia_pds();
        assert_eq!(pds.len(), 1);
        assert_eq!(pds[0].iaid, 1);
        assert_eq!(pds[0].t1, 900);
        assert_eq!(pds[0].t2, 1440);
        assert_eq!(pds[0].prefixes.len(), 1);
        assert_eq!(pds[0].prefixes[0].prefix_length, 56);
    }

    #[test]
    fn test_message_parse_ia_pds_empty() {
        let msg = Dhcpv6Message::new(MSG_REPLY, [0; 3]);
        assert!(msg.parse_ia_pds().is_empty());
    }

    // -- Client PD config tests --

    fn make_pd_config() -> Dhcpv6ClientConfig {
        Dhcpv6ClientConfig {
            ifindex: 2,
            ifname: "wan0".to_string(),
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            request_addresses: true,
            request_prefix_delegation: true,
            prefix_hint: 56,
            prefix_hint_address: Ipv6Addr::UNSPECIFIED,
            rapid_commit: false,
            max_attempts: 0,
            request_options: vec![OPT_DNS_SERVERS, OPT_DOMAIN_LIST],
        }
    }

    fn make_pd_only_config() -> Dhcpv6ClientConfig {
        Dhcpv6ClientConfig {
            ifindex: 3,
            ifname: "wan0".to_string(),
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            request_addresses: false,
            request_prefix_delegation: true,
            prefix_hint: 48,
            prefix_hint_address: Ipv6Addr::UNSPECIFIED,
            rapid_commit: false,
            max_attempts: 0,
            request_options: vec![OPT_DNS_SERVERS, OPT_DOMAIN_LIST],
        }
    }

    #[test]
    fn test_client_new_with_pd() {
        let client = Dhcpv6Client::new(make_pd_config());
        assert_eq!(client.iaid, 2);
        assert_eq!(client.pd_iaid, 3); // ifindex + 1
        assert!(client.config.request_prefix_delegation);
        assert_eq!(client.config.prefix_hint, 56);
    }

    #[test]
    fn test_client_pd_iaid_distinct_from_na_iaid() {
        let client = Dhcpv6Client::new(make_pd_config());
        assert_ne!(client.iaid, client.pd_iaid);
    }

    #[test]
    fn test_client_solicit_contains_ia_pd() {
        let mut client = Dhcpv6Client::new(make_pd_config());
        let pkt = client.next_packet().unwrap();
        let msg = Dhcpv6Message::parse(&pkt).unwrap();

        assert_eq!(msg.msg_type, MSG_SOLICIT);
        // Should have both IA_NA and IA_PD.
        assert!(msg.find_option(OPT_IA_NA).is_some());
        assert!(msg.find_option(OPT_IA_PD).is_some());

        // Parse the IA_PD and check the hint.
        let pds = msg.parse_ia_pds();
        assert_eq!(pds.len(), 1);
        assert_eq!(pds[0].iaid, client.pd_iaid);
        // The hint prefix should be present.
        assert_eq!(pds[0].prefixes.len(), 1);
        assert_eq!(pds[0].prefixes[0].prefix_length, 56);
    }

    #[test]
    fn test_client_solicit_pd_only_no_ia_na() {
        let mut client = Dhcpv6Client::new(make_pd_only_config());
        let pkt = client.next_packet().unwrap();
        let msg = Dhcpv6Message::parse(&pkt).unwrap();

        assert_eq!(msg.msg_type, MSG_SOLICIT);
        // Should NOT have IA_NA, but should have IA_PD.
        assert!(msg.find_option(OPT_IA_NA).is_none());
        assert!(msg.find_option(OPT_IA_PD).is_some());
    }

    #[test]
    fn test_client_solicit_pd_no_hint() {
        let mut config = make_pd_config();
        config.prefix_hint = 0; // No hint.
        let mut client = Dhcpv6Client::new(config);
        let pkt = client.next_packet().unwrap();
        let msg = Dhcpv6Message::parse(&pkt).unwrap();

        let pds = msg.parse_ia_pds();
        assert_eq!(pds.len(), 1);
        // No hint prefix sub-option.
        assert!(pds[0].prefixes.is_empty());
    }

    // -- Build a PD Advertise/Reply for testing --

    fn build_pd_advertise(
        solicit: &Dhcpv6Message,
        client_duid: &Duid,
        pd_iaid: u32,
    ) -> Dhcpv6Message {
        let mut adv = Dhcpv6Message::new(MSG_ADVERTISE, solicit.transaction_id);
        adv.add_option(OPT_CLIENTID, client_duid.data.clone());
        adv.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        adv.add_option(OPT_PREFERENCE, vec![200]);

        // IA_NA with an offered address.
        let addr = IaAddress {
            address: "2001:db8::100".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        };
        let ia_nas = solicit.parse_ia_nas();
        let na_iaid = ia_nas.first().map(|ia| ia.iaid).unwrap_or(1);
        adv.add_option(OPT_IA_NA, build_ia_na_option(na_iaid, 1800, 2880, &[&addr]));

        // IA_PD with a delegated prefix.
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 56,
            prefix: "2001:db8:abcd:ab00::".parse().unwrap(),
        };
        adv.add_option(OPT_IA_PD, build_ia_pd_option(pd_iaid, 900, 1440, &[&pfx]));

        adv
    }

    fn build_pd_reply(request: &Dhcpv6Message, client_duid: &Duid, pd_iaid: u32) -> Dhcpv6Message {
        let mut reply = Dhcpv6Message::new(MSG_REPLY, request.transaction_id);
        reply.add_option(OPT_CLIENTID, client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);

        // IA_NA with the assigned address.
        let addr = IaAddress {
            address: "2001:db8::100".parse().unwrap(),
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
        };
        let ia_nas = request.parse_ia_nas();
        let na_iaid = ia_nas.first().map(|ia| ia.iaid).unwrap_or(1);
        reply.add_option(OPT_IA_NA, build_ia_na_option(na_iaid, 1800, 2880, &[&addr]));

        // IA_PD with a delegated prefix.
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 56,
            prefix: "2001:db8:abcd:ab00::".parse().unwrap(),
        };
        reply.add_option(OPT_IA_PD, build_ia_pd_option(pd_iaid, 900, 1440, &[&pfx]));

        // DNS servers.
        let dns: Ipv6Addr = "2001:4860:4860::8888".parse().unwrap();
        reply.add_option(OPT_DNS_SERVERS, dns.octets().to_vec());

        reply
    }

    fn build_pd_only_reply(
        request: &Dhcpv6Message,
        client_duid: &Duid,
        pd_iaid: u32,
    ) -> Dhcpv6Message {
        let mut reply = Dhcpv6Message::new(MSG_REPLY, request.transaction_id);
        reply.add_option(OPT_CLIENTID, client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);

        // Only IA_PD, no IA_NA.
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 48,
            prefix: "2001:db8:ff00::".parse().unwrap(),
        };
        reply.add_option(OPT_IA_PD, build_ia_pd_option(pd_iaid, 900, 1440, &[&pfx]));

        reply
    }

    // -- Full PD exchange tests --

    #[test]
    fn test_client_full_pd_exchange() {
        let mut client = Dhcpv6Client::new(make_pd_config());
        let pd_iaid = client.pd_iaid;

        // 1. Client sends Solicit.
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();
        assert_eq!(solicit.msg_type, MSG_SOLICIT);

        // 2. Server sends Advertise with IA_NA + IA_PD.
        let advertise = build_pd_advertise(&solicit, &client.client_duid, pd_iaid);
        client.process_reply(&advertise.serialize());
        assert_eq!(client.state, Dhcpv6State::Requesting);

        // 3. Client sends Request.
        let req_pkt = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&req_pkt).unwrap();
        assert_eq!(request.msg_type, MSG_REQUEST);
        // Request should contain both IA_NA and IA_PD.
        assert!(request.find_option(OPT_IA_NA).is_some());
        assert!(request.find_option(OPT_IA_PD).is_some());

        // Verify the Request IA_PD contains the prefix from the Advertise.
        let req_pds = request.parse_ia_pds();
        assert_eq!(req_pds.len(), 1);
        assert_eq!(req_pds[0].prefixes.len(), 1);
        assert_eq!(req_pds[0].prefixes[0].prefix_length, 56);

        // 4. Server sends Reply with IA_NA + IA_PD.
        let reply = build_pd_reply(&request, &client.client_duid, pd_iaid);
        let lease = client.process_reply(&reply.serialize());
        assert!(lease.is_some());
        assert_eq!(client.state, Dhcpv6State::Bound);

        let lease = lease.unwrap();
        // Check IA_NA.
        assert_eq!(lease.ia_na.addresses.len(), 1);
        assert_eq!(
            lease.ia_na.addresses[0].address,
            "2001:db8::100".parse::<Ipv6Addr>().unwrap()
        );

        // Check IA_PD.
        assert!(lease.ia_pd.is_some());
        let pd = lease.ia_pd.as_ref().unwrap();
        assert_eq!(pd.prefixes.len(), 1);
        assert_eq!(pd.prefixes[0].prefix_length, 56);
        assert_eq!(
            pd.prefixes[0].prefix,
            "2001:db8:abcd:ab00::".parse::<Ipv6Addr>().unwrap()
        );
        assert_eq!(pd.t1, 900);
        assert_eq!(pd.t2, 1440);
    }

    #[test]
    fn test_client_pd_only_exchange() {
        let mut client = Dhcpv6Client::new(make_pd_only_config());
        let pd_iaid = client.pd_iaid;

        // 1. Solicit — should not have IA_NA.
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();
        assert!(solicit.find_option(OPT_IA_NA).is_none());
        assert!(solicit.find_option(OPT_IA_PD).is_some());

        // 2. Advertise with PD only (no IA_NA).
        let mut adv = Dhcpv6Message::new(MSG_ADVERTISE, solicit.transaction_id);
        adv.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        adv.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        adv.add_option(OPT_PREFERENCE, vec![255]);
        let pfx = IaPrefix {
            preferred_lifetime: 3600,
            valid_lifetime: 7200,
            prefix_length: 48,
            prefix: "2001:db8:ff00::".parse().unwrap(),
        };
        adv.add_option(OPT_IA_PD, build_ia_pd_option(pd_iaid, 900, 1440, &[&pfx]));
        client.process_reply(&adv.serialize());
        assert_eq!(client.state, Dhcpv6State::Requesting);

        // 3. Request — should not have IA_NA.
        let req_pkt = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&req_pkt).unwrap();
        assert!(request.find_option(OPT_IA_NA).is_none());
        assert!(request.find_option(OPT_IA_PD).is_some());

        // 4. Reply with PD only.
        let reply = build_pd_only_reply(&request, &client.client_duid, pd_iaid);
        let lease = client.process_reply(&reply.serialize());
        assert!(lease.is_some());
        assert_eq!(client.state, Dhcpv6State::Bound);

        let lease = lease.unwrap();
        assert!(lease.ia_na.addresses.is_empty()); // No addresses.
        assert!(lease.has_prefix_delegation());
        assert_eq!(lease.delegated_prefixes().len(), 1);
        assert_eq!(lease.delegated_prefixes()[0].prefix_length, 48);
    }

    #[test]
    fn test_client_pd_reply_no_prefix_falls_back() {
        let mut client = Dhcpv6Client::new(make_pd_only_config());
        let _pkt = client.next_packet().unwrap();

        // Advertise without IA_PD.
        let mut adv = Dhcpv6Message::new(MSG_ADVERTISE, client.transaction_id);
        adv.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        adv.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        adv.add_option(OPT_PREFERENCE, vec![255]);
        // IA_PD with no prefixes (just the header).
        adv.add_option(OPT_IA_PD, build_ia_pd_option(client.pd_iaid, 0, 0, &[]));
        client.process_reply(&adv.serialize());

        // Request.
        let req_pkt = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&req_pkt).unwrap();

        // Reply with empty IA_PD and no IA_NA.
        let mut reply = Dhcpv6Message::new(MSG_REPLY, request.transaction_id);
        reply.add_option(OPT_CLIENTID, client.client_duid.data.clone());
        reply.add_option(OPT_SERVERID, vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]);
        reply.add_option(OPT_IA_PD, build_ia_pd_option(client.pd_iaid, 0, 0, &[]));

        let lease = client.process_reply(&reply.serialize());
        // Should fail: no addresses AND no prefixes.
        assert!(lease.is_none());
        assert_eq!(client.state, Dhcpv6State::Init);
    }

    #[test]
    fn test_client_renew_includes_ia_pd() {
        let mut client = Dhcpv6Client::new(make_pd_config());
        let pd_iaid = client.pd_iaid;

        // Go through full exchange to get to Bound.
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();
        let advertise = build_pd_advertise(&solicit, &client.client_duid, pd_iaid);
        client.process_reply(&advertise.serialize());
        let req_pkt = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&req_pkt).unwrap();
        let reply = build_pd_reply(&request, &client.client_duid, pd_iaid);
        client.process_reply(&reply.serialize());
        assert_eq!(client.state, Dhcpv6State::Bound);

        // Simulate T1 expiry by manually setting state to Renewing.
        client.state = Dhcpv6State::Renewing;
        client.transaction_id = generate_transaction_id();
        client.attempts = 1;
        client.last_send = Some(Instant::now());
        client.exchange_start = Some(Instant::now());
        client.current_rt = REN_TIMEOUT;

        let renew_pkt = client.next_packet().unwrap();
        let renew_msg = Dhcpv6Message::parse(&renew_pkt).unwrap();
        assert_eq!(renew_msg.msg_type, MSG_RENEW);
        // Renew should include both IA_NA and IA_PD with current values.
        assert!(renew_msg.find_option(OPT_IA_NA).is_some());
        assert!(renew_msg.find_option(OPT_IA_PD).is_some());

        let renew_pds = renew_msg.parse_ia_pds();
        assert_eq!(renew_pds.len(), 1);
        assert_eq!(renew_pds[0].prefixes.len(), 1);
        assert_eq!(renew_pds[0].prefixes[0].prefix_length, 56);
    }

    #[test]
    fn test_client_release_includes_ia_pd() {
        let mut client = Dhcpv6Client::new(make_pd_config());
        let pd_iaid = client.pd_iaid;

        // Get to Bound state.
        let pkt = client.next_packet().unwrap();
        let solicit = Dhcpv6Message::parse(&pkt).unwrap();
        let advertise = build_pd_advertise(&solicit, &client.client_duid, pd_iaid);
        client.process_reply(&advertise.serialize());
        let req_pkt = client.next_packet().unwrap();
        let request = Dhcpv6Message::parse(&req_pkt).unwrap();
        let reply = build_pd_reply(&request, &client.client_duid, pd_iaid);
        client.process_reply(&reply.serialize());

        // Build release.
        let release_data = client.build_release().unwrap();
        let release_msg = Dhcpv6Message::parse(&release_data).unwrap();
        assert_eq!(release_msg.msg_type, MSG_RELEASE);

        // Release should include both IA_NA and IA_PD.
        assert!(release_msg.find_option(OPT_IA_NA).is_some());
        assert!(release_msg.find_option(OPT_IA_PD).is_some());
    }

    // -- Lease PD integration tests --

    fn make_pd_lease() -> Dhcpv6Lease {
        Dhcpv6Lease {
            ia_na: IaNa {
                iaid: 1,
                t1: 1800,
                t2: 2880,
                addresses: vec![IaAddress {
                    address: "2001:db8::1".parse().unwrap(),
                    preferred_lifetime: 3600,
                    valid_lifetime: 7200,
                }],
                status: None,
            },
            ia_pd: Some(IaPd {
                iaid: 2,
                t1: 900,
                t2: 1440,
                prefixes: vec![IaPrefix {
                    preferred_lifetime: 3600,
                    valid_lifetime: 7200,
                    prefix_length: 56,
                    prefix: "2001:db8:abcd::".parse().unwrap(),
                }],
                status: None,
            }),
            server_id: Duid::from_bytes(vec![0, 3, 0, 1, 1, 2, 3, 4, 5, 6]),
            dns_servers: vec![],
            domains: vec![],
            preference: 0,
            obtained_at: Instant::now(),
        }
    }

    #[test]
    fn test_lease_t1_uses_minimum_of_na_and_pd() {
        let lease = make_pd_lease();
        // IA_NA T1=1800, IA_PD T1=900. Should pick 900.
        assert_eq!(lease.t1(), Duration::from_secs(900));
    }

    #[test]
    fn test_lease_t2_uses_minimum_of_na_and_pd() {
        let lease = make_pd_lease();
        // IA_NA T2=2880, IA_PD T2=1440. Should pick 1440.
        assert_eq!(lease.t2(), Duration::from_secs(1440));
    }

    #[test]
    fn test_lease_t1_na_only_when_no_pd() {
        let mut lease = make_pd_lease();
        lease.ia_pd = None;
        assert_eq!(lease.t1(), Duration::from_secs(1800));
    }

    #[test]
    fn test_lease_t1_pd_only_when_no_addresses() {
        let mut lease = make_pd_lease();
        lease.ia_na.addresses.clear();
        // Only PD's T1=900.
        assert_eq!(lease.t1(), Duration::from_secs(900));
    }

    #[test]
    fn test_lease_has_prefix_delegation() {
        let lease = make_pd_lease();
        assert!(lease.has_prefix_delegation());
    }

    #[test]
    fn test_lease_no_prefix_delegation() {
        let mut lease = make_pd_lease();
        lease.ia_pd = None;
        assert!(!lease.has_prefix_delegation());
    }

    #[test]
    fn test_lease_delegated_prefixes() {
        let lease = make_pd_lease();
        let pfxs = lease.delegated_prefixes();
        assert_eq!(pfxs.len(), 1);
        assert_eq!(pfxs[0].prefix_length, 56);
    }

    #[test]
    fn test_lease_delegated_prefixes_empty_when_no_pd() {
        let mut lease = make_pd_lease();
        lease.ia_pd = None;
        assert!(lease.delegated_prefixes().is_empty());
    }

    #[test]
    fn test_lease_is_expired_considers_pd() {
        let mut lease = make_pd_lease();
        // Make NA addresses expired.
        lease.ia_na.addresses[0].valid_lifetime = 0;
        // But PD prefix still valid.
        // Not expired because PD prefix is still valid.
        assert!(!lease.is_expired());
    }

    #[test]
    fn test_lease_is_expired_when_both_expired() {
        let mut lease = make_pd_lease();
        lease.ia_na.addresses[0].valid_lifetime = 0;
        lease.ia_pd.as_mut().unwrap().prefixes[0].valid_lifetime = 0;
        assert!(lease.is_expired());
    }

    #[test]
    fn test_lease_remaining_considers_pd() {
        let mut lease = make_pd_lease();
        // NA valid=7200, PD valid=7200 — remaining should be close to 7200.
        let remaining = lease.remaining();
        assert!(remaining.as_secs() >= 7190);

        // Make PD have shorter valid lifetime.
        lease.ia_pd.as_mut().unwrap().prefixes[0].valid_lifetime = 1800;
        let remaining2 = lease.remaining();
        // Should use the shorter one (PD's 1800).
        assert!(remaining2.as_secs() <= 1801);
    }

    #[test]
    fn test_lease_display_includes_prefix() {
        let lease = make_pd_lease();
        let s = format!("{lease}");
        assert!(s.contains("prefix=2001:db8:abcd::/56"));
        assert!(s.contains("PD-T1=900s"));
        assert!(s.contains("PD-T2=1440s"));
    }

    #[test]
    fn test_lease_display_no_pd() {
        let mut lease = make_pd_lease();
        lease.ia_pd = None;
        let s = format!("{lease}");
        assert!(!s.contains("prefix="));
        assert!(!s.contains("PD-T1"));
    }

    // -- Default config PD fields --

    #[test]
    fn test_default_config_pd_disabled() {
        let config = Dhcpv6ClientConfig::default();
        assert!(!config.request_prefix_delegation);
        assert_eq!(config.prefix_hint, 0);
        assert_eq!(config.prefix_hint_address, Ipv6Addr::UNSPECIFIED);
    }

    // -- PD constant tests --

    #[test]
    fn test_opt_ia_pd_constant() {
        assert_eq!(OPT_IA_PD, 25);
    }

    #[test]
    fn test_opt_iaprefix_constant() {
        assert_eq!(OPT_IAPREFIX, 26);
    }

    #[test]
    fn test_status_no_prefix_avail_constant() {
        assert_eq!(STATUS_NO_PREFIX_AVAIL, 6);
    }
}
