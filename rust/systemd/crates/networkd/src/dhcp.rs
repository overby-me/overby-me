//! DHCPv4 client implementation.
//!
//! Implements the DHCP protocol (RFC 2131) using raw UDP sockets.
//! The client goes through the standard DORA flow:
//!   Discover → Offer → Request → Ack
//!
//! After obtaining a lease the client tracks renewal (T1) and rebinding (T2)
//! timers and re-negotiates as needed.

use std::fmt;
use std::net::Ipv4Addr;
use std::time::{Duration, Instant};

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DHCP_SERVER_PORT: u16 = 67;
const DHCP_CLIENT_PORT: u16 = 68;

const BOOTP_REQUEST: u8 = 1;
const BOOTP_REPLY: u8 = 2;

const HTYPE_ETHERNET: u8 = 1;
const HLEN_ETHERNET: u8 = 6;

/// Magic cookie that starts the options section (RFC 2131 §3).
const MAGIC_COOKIE: [u8; 4] = [99, 130, 83, 99];

// DHCP message types (option 53).
const DHCP_DISCOVER: u8 = 1;
const DHCP_OFFER: u8 = 2;
const DHCP_REQUEST: u8 = 3;
const DHCP_DECLINE: u8 = 4;
const DHCP_ACK: u8 = 5;
const DHCP_NAK: u8 = 6;
const DHCP_RELEASE: u8 = 7;
const DHCP_INFORM: u8 = 8;

// Common option codes.
const OPT_PAD: u8 = 0;
const OPT_SUBNET_MASK: u8 = 1;
const OPT_ROUTER: u8 = 3;
const OPT_DNS: u8 = 6;
const OPT_HOSTNAME: u8 = 12;
const OPT_DOMAIN_NAME: u8 = 15;
const OPT_BROADCAST: u8 = 28;
const OPT_NTP: u8 = 42;
const OPT_REQUESTED_IP: u8 = 50;
const OPT_LEASE_TIME: u8 = 51;
const OPT_MESSAGE_TYPE: u8 = 53;
const OPT_SERVER_ID: u8 = 54;
const OPT_PARAMETER_LIST: u8 = 55;
const OPT_RENEWAL_TIME: u8 = 58;
const OPT_REBINDING_TIME: u8 = 59;
const OPT_CLIENT_ID: u8 = 61;
const OPT_CLASSLESS_ROUTES: u8 = 121;
const OPT_END: u8 = 255;

// Minimum DHCP packet size (BOOTP fixed header + magic cookie).
const MIN_PACKET_SIZE: usize = 240;

// ---------------------------------------------------------------------------
// DHCP Lease
// ---------------------------------------------------------------------------

/// A successfully obtained DHCP lease.
#[derive(Debug, Clone)]
pub struct DhcpLease {
    /// Assigned IPv4 address.
    pub address: Ipv4Addr,

    /// Subnet mask (e.g. 255.255.255.0).
    pub subnet_mask: Ipv4Addr,

    /// Default gateway (router) addresses.
    pub routers: Vec<Ipv4Addr>,

    /// DNS server addresses.
    pub dns_servers: Vec<Ipv4Addr>,

    /// NTP server addresses.
    pub ntp_servers: Vec<Ipv4Addr>,

    /// Broadcast address.
    pub broadcast: Option<Ipv4Addr>,

    /// Hostname offered by the server.
    pub hostname: Option<String>,

    /// Domain name offered by the server.
    pub domain_name: Option<String>,

    /// Classless static routes (RFC 3442): Vec<(destination, prefix_len, gateway)>.
    pub classless_routes: Vec<(Ipv4Addr, u8, Ipv4Addr)>,

    /// Server identifier (the DHCP server's IP).
    pub server_id: Ipv4Addr,

    /// Lease duration in seconds.
    pub lease_time: u32,

    /// T1 renewal time in seconds (defaults to lease_time / 2).
    pub renewal_time: u32,

    /// T2 rebinding time in seconds (defaults to lease_time * 7/8).
    pub rebinding_time: u32,

    /// MTU offered by the server.
    pub mtu: Option<u16>,

    /// When the lease was obtained (monotonic).
    pub obtained_at: Instant,
}

impl DhcpLease {
    /// Prefix length derived from the subnet mask.
    pub fn prefix_len(&self) -> u8 {
        let mask = u32::from(self.subnet_mask);
        mask.count_ones() as u8
    }

    /// How long until the lease expires.
    pub fn remaining(&self) -> Duration {
        let elapsed = self.obtained_at.elapsed();
        let total = Duration::from_secs(u64::from(self.lease_time));
        total.saturating_sub(elapsed)
    }

    /// Whether the lease has expired.
    pub fn is_expired(&self) -> bool {
        self.remaining() == Duration::ZERO
    }

    /// Whether T1 (renewal) time has been reached.
    pub fn needs_renewal(&self) -> bool {
        let elapsed = self.obtained_at.elapsed();
        elapsed >= Duration::from_secs(u64::from(self.renewal_time))
    }

    /// Whether T2 (rebinding) time has been reached.
    pub fn needs_rebinding(&self) -> bool {
        let elapsed = self.obtained_at.elapsed();
        elapsed >= Duration::from_secs(u64::from(self.rebinding_time))
    }
}

impl fmt::Display for DhcpLease {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}/{} gw={} dns=[{}] lease={}s server={}",
            self.address,
            self.prefix_len(),
            self.routers
                .first()
                .map(|r| r.to_string())
                .unwrap_or_else(|| "none".into()),
            self.dns_servers
                .iter()
                .map(|d| d.to_string())
                .collect::<Vec<_>>()
                .join(", "),
            self.lease_time,
            self.server_id,
        )
    }
}

// ---------------------------------------------------------------------------
// DHCP Client State
// ---------------------------------------------------------------------------

/// Current state of the DHCP client state machine.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DhcpState {
    /// Initial state — nothing sent yet.
    Init,
    /// DISCOVER sent, waiting for OFFER.
    Selecting,
    /// REQUEST sent (after OFFER), waiting for ACK.
    Requesting,
    /// Lease obtained and active.
    Bound,
    /// T1 reached, unicast REQUEST sent to renew.
    Renewing,
    /// T2 reached, broadcast REQUEST sent to rebind.
    Rebinding,
}

impl fmt::Display for DhcpState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Init => write!(f, "INIT"),
            Self::Selecting => write!(f, "SELECTING"),
            Self::Requesting => write!(f, "REQUESTING"),
            Self::Bound => write!(f, "BOUND"),
            Self::Renewing => write!(f, "RENEWING"),
            Self::Rebinding => write!(f, "REBINDING"),
        }
    }
}

// ---------------------------------------------------------------------------
// DHCP Client Configuration
// ---------------------------------------------------------------------------

/// Configuration for the DHCP client.
#[derive(Debug, Clone)]
pub struct DhcpClientConfig {
    /// Interface index.
    pub ifindex: u32,

    /// Interface name (for logging).
    pub ifname: String,

    /// Hardware (MAC) address (6 bytes for Ethernet).
    pub mac: [u8; 6],

    /// Hostname to send in option 12.
    pub hostname: Option<String>,

    /// Vendor class identifier (option 60).
    pub vendor_class_id: Option<String>,

    /// Client identifier mode: `mac` or `duid`.
    pub client_identifier: ClientIdMode,

    /// Whether to request broadcast replies.
    pub request_broadcast: bool,

    /// Route metric for DHCP-learned routes.
    pub route_metric: u32,

    /// Maximum number of discover attempts (0 = unlimited).
    pub max_attempts: u32,

    /// Options to request in the parameter-request list.
    pub request_options: Vec<u8>,
}

impl Default for DhcpClientConfig {
    fn default() -> Self {
        Self {
            ifindex: 0,
            ifname: String::new(),
            mac: [0; 6],
            hostname: None,
            vendor_class_id: None,
            client_identifier: ClientIdMode::Mac,
            request_broadcast: false,
            route_metric: 1024,
            max_attempts: 0,
            request_options: vec![
                OPT_SUBNET_MASK,
                OPT_ROUTER,
                OPT_DNS,
                OPT_HOSTNAME,
                OPT_DOMAIN_NAME,
                OPT_BROADCAST,
                OPT_NTP,
                OPT_LEASE_TIME,
                OPT_RENEWAL_TIME,
                OPT_REBINDING_TIME,
                OPT_CLASSLESS_ROUTES,
            ],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClientIdMode {
    Mac,
    Duid,
}

// ---------------------------------------------------------------------------
// DHCP Packet builder / parser
// ---------------------------------------------------------------------------

/// Raw DHCP packet representation.
#[derive(Clone)]
pub struct DhcpPacket {
    pub op: u8,
    pub htype: u8,
    pub hlen: u8,
    pub hops: u8,
    pub xid: u32,
    pub secs: u16,
    pub flags: u16,
    pub ciaddr: Ipv4Addr,
    pub yiaddr: Ipv4Addr,
    pub siaddr: Ipv4Addr,
    pub giaddr: Ipv4Addr,
    pub chaddr: [u8; 16],
    pub sname: [u8; 64],
    pub file: [u8; 128],
    pub options: Vec<DhcpOption>,
}

impl fmt::Debug for DhcpPacket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("DhcpPacket")
            .field("op", &self.op)
            .field("xid", &format_args!("{:#010x}", self.xid))
            .field("ciaddr", &self.ciaddr)
            .field("yiaddr", &self.yiaddr)
            .field("siaddr", &self.siaddr)
            .field("options", &self.options)
            .finish()
    }
}

impl DhcpPacket {
    /// Create a new client request packet with default fields.
    pub fn new_request(xid: u32, mac: &[u8; 6]) -> Self {
        let mut chaddr = [0u8; 16];
        chaddr[..6].copy_from_slice(mac);
        Self {
            op: BOOTP_REQUEST,
            htype: HTYPE_ETHERNET,
            hlen: HLEN_ETHERNET,
            hops: 0,
            xid,
            secs: 0,
            flags: 0,
            ciaddr: Ipv4Addr::UNSPECIFIED,
            yiaddr: Ipv4Addr::UNSPECIFIED,
            siaddr: Ipv4Addr::UNSPECIFIED,
            giaddr: Ipv4Addr::UNSPECIFIED,
            chaddr,
            sname: [0; 64],
            file: [0; 128],
            options: Vec::new(),
        }
    }

    /// Get the DHCP message type from options.
    pub fn message_type(&self) -> Option<u8> {
        self.options.iter().find_map(|opt| {
            if opt.code == OPT_MESSAGE_TYPE && !opt.data.is_empty() {
                Some(opt.data[0])
            } else {
                None
            }
        })
    }

    /// Get the server identifier from options.
    pub fn server_id(&self) -> Option<Ipv4Addr> {
        self.options.iter().find_map(|opt| {
            if opt.code == OPT_SERVER_ID && opt.data.len() == 4 {
                Some(Ipv4Addr::new(
                    opt.data[0],
                    opt.data[1],
                    opt.data[2],
                    opt.data[3],
                ))
            } else {
                None
            }
        })
    }

    /// Get the requested IP address from options.
    pub fn get_option_ipv4(&self, code: u8) -> Option<Ipv4Addr> {
        self.options.iter().find_map(|opt| {
            if opt.code == code && opt.data.len() == 4 {
                Some(Ipv4Addr::new(
                    opt.data[0],
                    opt.data[1],
                    opt.data[2],
                    opt.data[3],
                ))
            } else {
                None
            }
        })
    }

    /// Get a list of IPv4 addresses from an option (e.g. DNS, routers).
    pub fn get_option_ipv4_list(&self, code: u8) -> Vec<Ipv4Addr> {
        let mut addrs = Vec::new();
        for opt in &self.options {
            if opt.code == code && opt.data.len() >= 4 && opt.data.len() % 4 == 0 {
                for chunk in opt.data.chunks_exact(4) {
                    addrs.push(Ipv4Addr::new(chunk[0], chunk[1], chunk[2], chunk[3]));
                }
            }
        }
        addrs
    }

    /// Get a u32 from an option (e.g. lease time).
    pub fn get_option_u32(&self, code: u8) -> Option<u32> {
        self.options.iter().find_map(|opt| {
            if opt.code == code && opt.data.len() == 4 {
                Some(u32::from_be_bytes([
                    opt.data[0],
                    opt.data[1],
                    opt.data[2],
                    opt.data[3],
                ]))
            } else {
                None
            }
        })
    }

    /// Get a string from an option.
    pub fn get_option_string(&self, code: u8) -> Option<String> {
        self.options.iter().find_map(|opt| {
            if opt.code == code && !opt.data.is_empty() {
                Some(String::from_utf8_lossy(&opt.data).to_string())
            } else {
                None
            }
        })
    }

    /// Parse classless static routes (RFC 3442, option 121).
    pub fn get_classless_routes(&self) -> Vec<(Ipv4Addr, u8, Ipv4Addr)> {
        let mut routes = Vec::new();
        for opt in &self.options {
            if opt.code == OPT_CLASSLESS_ROUTES {
                let data = &opt.data;
                let mut i = 0;
                while i < data.len() {
                    if i >= data.len() {
                        break;
                    }
                    let prefix_len = data[i];
                    i += 1;
                    // RFC 3442: an IPv4 destination prefix length is 0-32. A
                    // hostile or buggy DHCP server can send a larger value, for
                    // which `octets` (below) would exceed the 4-byte `dest`
                    // buffer and panic the `dest[..octets]` copy. Reject it; the
                    // variable-length encoding cannot be resynced past a bad
                    // entry, so stop parsing this option.
                    if prefix_len > 32 {
                        break;
                    }
                    // Number of significant octets in the destination (0-4).
                    let octets = prefix_len.div_ceil(8) as usize;
                    if i + octets + 4 > data.len() {
                        break;
                    }
                    let mut dest = [0u8; 4];
                    dest[..octets].copy_from_slice(&data[i..i + octets]);
                    i += octets;
                    let gw = Ipv4Addr::new(data[i], data[i + 1], data[i + 2], data[i + 3]);
                    i += 4;
                    routes.push((Ipv4Addr::from(dest), prefix_len, gw));
                }
            }
        }
        routes
    }

    /// Serialize the packet to bytes.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::with_capacity(576);

        buf.push(self.op);
        buf.push(self.htype);
        buf.push(self.hlen);
        buf.push(self.hops);
        buf.extend_from_slice(&self.xid.to_be_bytes());
        buf.extend_from_slice(&self.secs.to_be_bytes());
        buf.extend_from_slice(&self.flags.to_be_bytes());
        buf.extend_from_slice(&self.ciaddr.octets());
        buf.extend_from_slice(&self.yiaddr.octets());
        buf.extend_from_slice(&self.siaddr.octets());
        buf.extend_from_slice(&self.giaddr.octets());
        buf.extend_from_slice(&self.chaddr);
        buf.extend_from_slice(&self.sname);
        buf.extend_from_slice(&self.file);

        // Magic cookie.
        buf.extend_from_slice(&MAGIC_COOKIE);

        // Options.
        for opt in &self.options {
            buf.push(opt.code);
            if opt.code == OPT_PAD || opt.code == OPT_END {
                continue;
            }
            buf.push(opt.data.len() as u8);
            buf.extend_from_slice(&opt.data);
        }

        // End marker.
        buf.push(OPT_END);

        // Pad to minimum 300 bytes (BOOTP minimum).
        while buf.len() < 300 {
            buf.push(0);
        }

        buf
    }

    /// Parse a DHCP packet from bytes.
    pub fn parse(data: &[u8]) -> Result<Self, String> {
        if data.len() < MIN_PACKET_SIZE {
            return Err(format!(
                "packet too short: {} < {} bytes",
                data.len(),
                MIN_PACKET_SIZE
            ));
        }

        let op = data[0];
        let htype = data[1];
        let hlen = data[2];
        let hops = data[3];
        let xid = u32::from_be_bytes([data[4], data[5], data[6], data[7]]);
        let secs = u16::from_be_bytes([data[8], data[9]]);
        let flags = u16::from_be_bytes([data[10], data[11]]);
        let ciaddr = Ipv4Addr::new(data[12], data[13], data[14], data[15]);
        let yiaddr = Ipv4Addr::new(data[16], data[17], data[18], data[19]);
        let siaddr = Ipv4Addr::new(data[20], data[21], data[22], data[23]);
        let giaddr = Ipv4Addr::new(data[24], data[25], data[26], data[27]);

        let mut chaddr = [0u8; 16];
        chaddr.copy_from_slice(&data[28..44]);

        let mut sname = [0u8; 64];
        sname.copy_from_slice(&data[44..108]);

        let mut file = [0u8; 128];
        file.copy_from_slice(&data[108..236]);

        // Verify magic cookie.
        if data[236..240] != MAGIC_COOKIE {
            return Err("invalid magic cookie".to_string());
        }

        // Parse options.
        let options = parse_options(&data[240..])?;

        Ok(Self {
            op,
            htype,
            hlen,
            hops,
            xid,
            secs,
            flags,
            ciaddr,
            yiaddr,
            siaddr,
            giaddr,
            chaddr,
            sname,
            file,
            options,
        })
    }

    /// Build a DHCPDISCOVER message.
    pub fn build_discover(config: &DhcpClientConfig, xid: u32) -> Self {
        let mut pkt = Self::new_request(xid, &config.mac);
        if config.request_broadcast {
            pkt.flags = 0x8000; // BROADCAST flag.
        }

        // Option 53: DHCP Message Type = DISCOVER.
        pkt.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_DISCOVER],
        });

        // Option 61: Client Identifier.
        add_client_id(&mut pkt.options, config);

        // Option 12: Hostname.
        if let Some(ref hostname) = config.hostname {
            pkt.options.push(DhcpOption {
                code: OPT_HOSTNAME,
                data: hostname.as_bytes().to_vec(),
            });
        }

        // Option 55: Parameter Request List.
        if !config.request_options.is_empty() {
            pkt.options.push(DhcpOption {
                code: OPT_PARAMETER_LIST,
                data: config.request_options.clone(),
            });
        }

        pkt
    }

    /// Build a DHCPREQUEST message (in response to an OFFER, or for renewal).
    pub fn build_request(
        config: &DhcpClientConfig,
        xid: u32,
        offered_ip: Ipv4Addr,
        server_id: Option<Ipv4Addr>,
        ciaddr: Ipv4Addr,
    ) -> Self {
        let mut pkt = Self::new_request(xid, &config.mac);
        pkt.ciaddr = ciaddr;

        if config.request_broadcast {
            pkt.flags = 0x8000;
        }

        // Option 53: DHCP Message Type = REQUEST.
        pkt.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_REQUEST],
        });

        // Option 61: Client Identifier.
        add_client_id(&mut pkt.options, config);

        // Option 50: Requested IP Address (only in SELECTING/REBOOTING, not RENEWING).
        if ciaddr == Ipv4Addr::UNSPECIFIED {
            pkt.options.push(DhcpOption {
                code: OPT_REQUESTED_IP,
                data: offered_ip.octets().to_vec(),
            });
        }

        // Option 54: Server Identifier (only in SELECTING state).
        if let Some(sid) = server_id {
            pkt.options.push(DhcpOption {
                code: OPT_SERVER_ID,
                data: sid.octets().to_vec(),
            });
        }

        // Option 12: Hostname.
        if let Some(ref hostname) = config.hostname {
            pkt.options.push(DhcpOption {
                code: OPT_HOSTNAME,
                data: hostname.as_bytes().to_vec(),
            });
        }

        // Option 55: Parameter Request List.
        if !config.request_options.is_empty() {
            pkt.options.push(DhcpOption {
                code: OPT_PARAMETER_LIST,
                data: config.request_options.clone(),
            });
        }

        pkt
    }

    /// Build a DHCPRELEASE message.
    pub fn build_release(
        config: &DhcpClientConfig,
        xid: u32,
        client_ip: Ipv4Addr,
        server_id: Ipv4Addr,
    ) -> Self {
        let mut pkt = Self::new_request(xid, &config.mac);
        pkt.ciaddr = client_ip;

        pkt.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_RELEASE],
        });

        pkt.options.push(DhcpOption {
            code: OPT_SERVER_ID,
            data: server_id.octets().to_vec(),
        });

        add_client_id(&mut pkt.options, config);

        pkt
    }

    /// Build a DHCPDECLINE message.
    pub fn build_decline(
        config: &DhcpClientConfig,
        xid: u32,
        declined_ip: Ipv4Addr,
        server_id: Ipv4Addr,
    ) -> Self {
        let mut pkt = Self::new_request(xid, &config.mac);

        pkt.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_DECLINE],
        });

        pkt.options.push(DhcpOption {
            code: OPT_REQUESTED_IP,
            data: declined_ip.octets().to_vec(),
        });

        pkt.options.push(DhcpOption {
            code: OPT_SERVER_ID,
            data: server_id.octets().to_vec(),
        });

        add_client_id(&mut pkt.options, config);

        pkt
    }

    /// Build a DHCPINFORM message.
    pub fn build_inform(config: &DhcpClientConfig, xid: u32, client_ip: Ipv4Addr) -> Self {
        let mut pkt = Self::new_request(xid, &config.mac);
        pkt.ciaddr = client_ip;

        pkt.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_INFORM],
        });

        add_client_id(&mut pkt.options, config);

        if let Some(ref hostname) = config.hostname {
            pkt.options.push(DhcpOption {
                code: OPT_HOSTNAME,
                data: hostname.as_bytes().to_vec(),
            });
        }

        if !config.request_options.is_empty() {
            pkt.options.push(DhcpOption {
                code: OPT_PARAMETER_LIST,
                data: config.request_options.clone(),
            });
        }

        pkt
    }

    /// Extract a [`DhcpLease`] from an ACK packet.
    pub fn to_lease(&self) -> Result<DhcpLease, String> {
        if self.message_type() != Some(DHCP_ACK) {
            return Err("not a DHCPACK".to_string());
        }

        let address = self.yiaddr;
        if address == Ipv4Addr::UNSPECIFIED {
            return Err("ACK has no yiaddr".to_string());
        }

        let subnet_mask = self
            .get_option_ipv4(OPT_SUBNET_MASK)
            .unwrap_or(Ipv4Addr::new(255, 255, 255, 0));
        let routers = self.get_option_ipv4_list(OPT_ROUTER);
        let dns_servers = self.get_option_ipv4_list(OPT_DNS);
        let ntp_servers = self.get_option_ipv4_list(OPT_NTP);
        let broadcast = self.get_option_ipv4(OPT_BROADCAST);
        let hostname = self.get_option_string(OPT_HOSTNAME);
        let domain_name = self.get_option_string(OPT_DOMAIN_NAME);
        let classless_routes = self.get_classless_routes();
        let server_id = self.server_id().unwrap_or(self.siaddr);
        let lease_time = self.get_option_u32(OPT_LEASE_TIME).unwrap_or(3600);
        let renewal_time = self
            .get_option_u32(OPT_RENEWAL_TIME)
            .unwrap_or(lease_time / 2);
        let rebinding_time = self
            .get_option_u32(OPT_REBINDING_TIME)
            .unwrap_or(lease_time * 7 / 8);

        // MTU (option 26).
        let mtu = self.options.iter().find_map(|opt| {
            if opt.code == 26 && opt.data.len() == 2 {
                Some(u16::from_be_bytes([opt.data[0], opt.data[1]]))
            } else {
                None
            }
        });

        Ok(DhcpLease {
            address,
            subnet_mask,
            routers,
            dns_servers,
            ntp_servers,
            broadcast,
            hostname,
            domain_name,
            classless_routes,
            server_id,
            lease_time,
            renewal_time,
            rebinding_time,
            mtu,
            obtained_at: Instant::now(),
        })
    }
}

// ---------------------------------------------------------------------------
// Options
// ---------------------------------------------------------------------------

/// A single DHCP option (type-length-value).
#[derive(Clone)]
pub struct DhcpOption {
    pub code: u8,
    pub data: Vec<u8>,
}

impl fmt::Debug for DhcpOption {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Opt({}, {} bytes", self.code, self.data.len())?;
        match self.code {
            OPT_MESSAGE_TYPE if !self.data.is_empty() => {
                let name = match self.data[0] {
                    DHCP_DISCOVER => "DISCOVER",
                    DHCP_OFFER => "OFFER",
                    DHCP_REQUEST => "REQUEST",
                    DHCP_DECLINE => "DECLINE",
                    DHCP_ACK => "ACK",
                    DHCP_NAK => "NAK",
                    DHCP_RELEASE => "RELEASE",
                    DHCP_INFORM => "INFORM",
                    other => return write!(f, ", type={other})"),
                };
                write!(f, ", {name})")
            }
            OPT_SERVER_ID | OPT_SUBNET_MASK | OPT_ROUTER | OPT_BROADCAST | OPT_REQUESTED_IP
                if self.data.len() == 4 =>
            {
                let ip = Ipv4Addr::new(self.data[0], self.data[1], self.data[2], self.data[3]);
                write!(f, ", {ip})")
            }
            OPT_LEASE_TIME | OPT_RENEWAL_TIME | OPT_REBINDING_TIME if self.data.len() == 4 => {
                let val =
                    u32::from_be_bytes([self.data[0], self.data[1], self.data[2], self.data[3]]);
                write!(f, ", {val}s)")
            }
            _ => write!(f, ")"),
        }
    }
}

fn parse_options(data: &[u8]) -> Result<Vec<DhcpOption>, String> {
    let mut options = Vec::new();
    let mut i = 0;
    while i < data.len() {
        let code = data[i];
        match code {
            OPT_PAD => {
                i += 1;
                continue;
            }
            OPT_END => break,
            _ => {
                i += 1;
                if i >= data.len() {
                    break;
                }
                let len = data[i] as usize;
                i += 1;
                if i + len > data.len() {
                    return Err(format!(
                        "option {} truncated: need {} bytes at offset {}",
                        code, len, i
                    ));
                }
                options.push(DhcpOption {
                    code,
                    data: data[i..i + len].to_vec(),
                });
                i += len;
            }
        }
    }
    Ok(options)
}

fn add_client_id(options: &mut Vec<DhcpOption>, config: &DhcpClientConfig) {
    match config.client_identifier {
        ClientIdMode::Mac => {
            let mut id = vec![HTYPE_ETHERNET]; // type = ethernet
            id.extend_from_slice(&config.mac);
            options.push(DhcpOption {
                code: OPT_CLIENT_ID,
                data: id,
            });
        }
        ClientIdMode::Duid => {
            // DUID-LL (type 3): 2-byte type + 2-byte hardware type + MAC.
            let mut id = vec![0, 3, 0, HTYPE_ETHERNET]; // DUID-LL, ethernet
            id.extend_from_slice(&config.mac);
            // Prepend the "type" byte for option 61 (0xff = DUID).
            let mut opt_data = vec![0xff];
            opt_data.extend(id);
            options.push(DhcpOption {
                code: OPT_CLIENT_ID,
                data: opt_data,
            });
        }
    }
}

// ---------------------------------------------------------------------------
// UDP/IP raw socket helpers
// ---------------------------------------------------------------------------

/// Build a UDP/IP packet for sending DHCP via raw socket.
/// Used when the client has no IP address yet and must send via broadcast.
pub fn build_udp_ip_packet(
    src_ip: Ipv4Addr,
    dst_ip: Ipv4Addr,
    src_port: u16,
    dst_port: u16,
    payload: &[u8],
) -> Vec<u8> {
    let ip_header_len = 20u16;
    let udp_header_len = 8u16;
    let total_len = ip_header_len + udp_header_len + payload.len() as u16;

    let mut pkt = Vec::with_capacity(total_len as usize);

    // IPv4 header (20 bytes, no options).
    pkt.push(0x45); // version=4, ihl=5
    pkt.push(0x10); // DSCP=0, ECN=0 (low-delay TOS)
    pkt.extend_from_slice(&total_len.to_be_bytes());
    pkt.extend_from_slice(&[0x00, 0x00]); // identification
    pkt.extend_from_slice(&[0x00, 0x00]); // flags + fragment offset
    pkt.push(64); // TTL
    pkt.push(17); // protocol = UDP
    pkt.extend_from_slice(&[0x00, 0x00]); // header checksum (filled below)
    pkt.extend_from_slice(&src_ip.octets());
    pkt.extend_from_slice(&dst_ip.octets());

    // Compute IP header checksum.
    let ip_cksum = ip_checksum(&pkt[..20]);
    pkt[10] = (ip_cksum >> 8) as u8;
    pkt[11] = (ip_cksum & 0xff) as u8;

    // UDP header (8 bytes).
    let udp_len = udp_header_len + payload.len() as u16;
    pkt.extend_from_slice(&src_port.to_be_bytes());
    pkt.extend_from_slice(&dst_port.to_be_bytes());
    pkt.extend_from_slice(&udp_len.to_be_bytes());
    pkt.extend_from_slice(&[0x00, 0x00]); // UDP checksum (optional for IPv4, set to 0)

    // Payload.
    pkt.extend_from_slice(payload);

    pkt
}

/// Extract DHCP payload from a raw IP+UDP packet.
/// Returns `None` if the packet is not a valid UDP packet on the expected port.
pub fn extract_dhcp_payload(data: &[u8], expected_port: u16) -> Option<Vec<u8>> {
    if data.len() < 28 {
        return None; // Too short for IP + UDP headers.
    }

    let version = (data[0] >> 4) & 0xf;
    if version != 4 {
        return None;
    }

    let ihl = (data[0] & 0xf) as usize * 4;
    if data.len() < ihl + 8 {
        return None;
    }

    let protocol = data[9];
    if protocol != 17 {
        // Not UDP.
        return None;
    }

    let udp_start = ihl;
    let _src_port = u16::from_be_bytes([data[udp_start], data[udp_start + 1]]);
    let dst_port = u16::from_be_bytes([data[udp_start + 2], data[udp_start + 3]]);
    let udp_len = u16::from_be_bytes([data[udp_start + 4], data[udp_start + 5]]) as usize;

    if dst_port != expected_port {
        return None;
    }

    let payload_start = udp_start + 8;
    let payload_end = udp_start + udp_len;
    if payload_end > data.len() || payload_start > payload_end {
        return None;
    }

    Some(data[payload_start..payload_end].to_vec())
}

fn ip_checksum(header: &[u8]) -> u16 {
    let mut sum = 0u32;
    let mut i = 0;
    while i + 1 < header.len() {
        let word = u16::from_be_bytes([header[i], header[i + 1]]);
        sum += u32::from(word);
        i += 2;
    }
    if i < header.len() {
        sum += u32::from(header[i]) << 8;
    }
    while (sum >> 16) != 0 {
        sum = (sum & 0xffff) + (sum >> 16);
    }
    !(sum as u16)
}

// ---------------------------------------------------------------------------
// Transaction ID generation
// ---------------------------------------------------------------------------

/// Generate a pseudo-random transaction ID from the MAC and current time.
pub fn generate_xid(mac: &[u8; 6]) -> u32 {
    // Simple hash of MAC + timestamp for uniqueness.
    let ts = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    let mut h: u32 = 0x811c_9dc5; // FNV-1a offset basis
    for &b in mac {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193); // FNV prime
    }
    for b in ts.to_le_bytes() {
        h ^= u32::from(b);
        h = h.wrapping_mul(0x0100_0193);
    }
    h
}

// ---------------------------------------------------------------------------
// DHCP client state machine
// ---------------------------------------------------------------------------

/// High-level DHCP client that manages the DORA state machine.
#[derive(Debug)]
pub struct DhcpClient {
    pub config: DhcpClientConfig,
    pub state: DhcpState,
    pub xid: u32,
    pub lease: Option<DhcpLease>,
    pub last_offer: Option<DhcpPacket>,
    pub attempts: u32,
    pub last_send: Option<Instant>,
}

impl DhcpClient {
    pub fn new(config: DhcpClientConfig) -> Self {
        let xid = generate_xid(&config.mac);
        Self {
            config,
            state: DhcpState::Init,
            xid,
            lease: None,
            last_offer: None,
            attempts: 0,
            last_send: None,
        }
    }

    /// Build the next outgoing packet based on current state.
    /// Returns `None` if no packet should be sent (e.g. already bound and
    /// not yet at renewal time).
    pub fn next_packet(&mut self) -> Option<DhcpPacket> {
        match self.state {
            DhcpState::Init => {
                self.xid = generate_xid(&self.config.mac);
                self.state = DhcpState::Selecting;
                self.attempts += 1;
                self.last_send = Some(Instant::now());
                Some(DhcpPacket::build_discover(&self.config, self.xid))
            }
            DhcpState::Selecting => {
                // Retransmit discover.
                self.attempts += 1;
                self.last_send = Some(Instant::now());
                Some(DhcpPacket::build_discover(&self.config, self.xid))
            }
            DhcpState::Requesting => {
                // Retransmit request.
                if let Some(ref offer) = self.last_offer {
                    self.attempts += 1;
                    self.last_send = Some(Instant::now());
                    Some(DhcpPacket::build_request(
                        &self.config,
                        self.xid,
                        offer.yiaddr,
                        offer.server_id(),
                        Ipv4Addr::UNSPECIFIED,
                    ))
                } else {
                    // Lost the offer, restart.
                    self.state = DhcpState::Init;
                    self.attempts = 0;
                    None
                }
            }
            DhcpState::Bound => {
                // Check if we need to renew.
                if let Some(ref lease) = self.lease
                    && lease.needs_renewal()
                {
                    self.state = DhcpState::Renewing;
                    self.xid = generate_xid(&self.config.mac);
                    self.last_send = Some(Instant::now());
                    return Some(DhcpPacket::build_request(
                        &self.config,
                        self.xid,
                        lease.address,
                        Some(lease.server_id),
                        lease.address,
                    ));
                }
                None
            }
            DhcpState::Renewing => {
                if let Some(ref lease) = self.lease {
                    if lease.needs_rebinding() {
                        self.state = DhcpState::Rebinding;
                    }
                    self.last_send = Some(Instant::now());
                    Some(DhcpPacket::build_request(
                        &self.config,
                        self.xid,
                        lease.address,
                        Some(lease.server_id),
                        lease.address,
                    ))
                } else {
                    self.state = DhcpState::Init;
                    None
                }
            }
            DhcpState::Rebinding => {
                if let Some(ref lease) = self.lease {
                    if lease.is_expired() {
                        self.state = DhcpState::Init;
                        self.lease = None;
                        return None;
                    }
                    self.last_send = Some(Instant::now());
                    // Broadcast request without server ID.
                    Some(DhcpPacket::build_request(
                        &self.config,
                        self.xid,
                        lease.address,
                        None,
                        lease.address,
                    ))
                } else {
                    self.state = DhcpState::Init;
                    None
                }
            }
        }
    }

    /// Process an incoming DHCP packet. Returns `Some(lease)` if a new lease
    /// was obtained or renewed.
    pub fn process_reply(&mut self, pkt: &DhcpPacket) -> Option<DhcpLease> {
        // Must be a reply with our XID and MAC.
        if pkt.op != BOOTP_REPLY {
            return None;
        }
        if pkt.xid != self.xid {
            return None;
        }
        if pkt.chaddr[..6] != self.config.mac {
            return None;
        }

        let msg_type = pkt.message_type()?;

        match (self.state, msg_type) {
            (DhcpState::Selecting, DHCP_OFFER) => {
                log::info!(
                    "{}: received OFFER {} from {:?}",
                    self.config.ifname,
                    pkt.yiaddr,
                    pkt.server_id()
                );
                self.last_offer = Some(pkt.clone());
                self.state = DhcpState::Requesting;
                self.attempts = 0;
                // The caller should call next_packet() to get the REQUEST.
                None
            }
            (DhcpState::Requesting, DHCP_ACK) => match pkt.to_lease() {
                Ok(lease) => {
                    log::info!("{}: received ACK — lease {}", self.config.ifname, lease);
                    self.state = DhcpState::Bound;
                    self.lease = Some(lease.clone());
                    self.attempts = 0;
                    Some(lease)
                }
                Err(e) => {
                    log::warn!("{}: invalid ACK: {}", self.config.ifname, e);
                    None
                }
            },
            (DhcpState::Requesting, DHCP_NAK) => {
                log::warn!("{}: received NAK, restarting", self.config.ifname);
                self.state = DhcpState::Init;
                self.last_offer = None;
                self.attempts = 0;
                None
            }
            (DhcpState::Renewing | DhcpState::Rebinding, DHCP_ACK) => match pkt.to_lease() {
                Ok(lease) => {
                    log::info!("{}: lease renewed — {}", self.config.ifname, lease);
                    self.state = DhcpState::Bound;
                    self.lease = Some(lease.clone());
                    self.attempts = 0;
                    Some(lease)
                }
                Err(e) => {
                    log::warn!("{}: invalid renewal ACK: {}", self.config.ifname, e);
                    None
                }
            },
            (DhcpState::Renewing | DhcpState::Rebinding, DHCP_NAK) => {
                log::warn!("{}: renewal NAK, restarting", self.config.ifname);
                self.state = DhcpState::Init;
                self.lease = None;
                self.last_offer = None;
                self.attempts = 0;
                None
            }
            _ => {
                log::trace!(
                    "{}: ignoring message type {} in state {}",
                    self.config.ifname,
                    msg_type,
                    self.state
                );
                None
            }
        }
    }

    /// Compute the retransmission timeout based on the number of attempts.
    /// Uses exponential backoff: 4s, 8s, 16s, 32s, 64s (capped).
    pub fn retransmit_timeout(&self) -> Duration {
        let base = Duration::from_secs(4);
        let factor = 1u64 << self.attempts.min(4);
        base * factor as u32
    }

    /// Whether the maximum number of attempts has been reached.
    pub fn max_attempts_reached(&self) -> bool {
        self.config.max_attempts > 0 && self.attempts >= self.config.max_attempts
    }

    /// Build a RELEASE packet for the current lease.
    pub fn build_release(&self) -> Option<DhcpPacket> {
        self.lease.as_ref().map(|lease| {
            DhcpPacket::build_release(
                &self.config,
                generate_xid(&self.config.mac),
                lease.address,
                lease.server_id,
            )
        })
    }
}

// ---------------------------------------------------------------------------
// DHCP message type name helper
// ---------------------------------------------------------------------------

/// Return a human-readable name for a DHCP message type.
pub fn dhcp_message_type_name(t: u8) -> &'static str {
    match t {
        DHCP_DISCOVER => "DISCOVER",
        DHCP_OFFER => "OFFER",
        DHCP_REQUEST => "REQUEST",
        DHCP_DECLINE => "DECLINE",
        DHCP_ACK => "ACK",
        DHCP_NAK => "NAK",
        DHCP_RELEASE => "RELEASE",
        DHCP_INFORM => "INFORM",
        _ => "UNKNOWN",
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn test_mac() -> [u8; 6] {
        [0x52, 0x54, 0x00, 0x12, 0x34, 0x56]
    }

    fn test_config() -> DhcpClientConfig {
        DhcpClientConfig {
            ifindex: 2,
            ifname: "eth0".into(),
            mac: test_mac(),
            hostname: Some("testhost".into()),
            ..Default::default()
        }
    }

    #[test]
    fn test_build_and_parse_discover() {
        let config = test_config();
        let pkt = DhcpPacket::build_discover(&config, 0x12345678);
        assert_eq!(pkt.op, BOOTP_REQUEST);
        assert_eq!(pkt.xid, 0x12345678);
        assert_eq!(pkt.message_type(), Some(DHCP_DISCOVER));
        assert_eq!(&pkt.chaddr[..6], &test_mac());

        // Serialize and re-parse.
        let bytes = pkt.serialize();
        assert!(bytes.len() >= 300);

        let parsed = DhcpPacket::parse(&bytes).unwrap();
        assert_eq!(parsed.op, BOOTP_REQUEST);
        assert_eq!(parsed.xid, 0x12345678);
        assert_eq!(parsed.message_type(), Some(DHCP_DISCOVER));
    }

    #[test]
    fn test_build_and_parse_request() {
        let config = test_config();
        let pkt = DhcpPacket::build_request(
            &config,
            0xAABBCCDD,
            Ipv4Addr::new(192, 168, 1, 100),
            Some(Ipv4Addr::new(192, 168, 1, 1)),
            Ipv4Addr::UNSPECIFIED,
        );
        assert_eq!(pkt.message_type(), Some(DHCP_REQUEST));
        assert_eq!(
            pkt.get_option_ipv4(OPT_REQUESTED_IP),
            Some(Ipv4Addr::new(192, 168, 1, 100))
        );
        assert_eq!(pkt.server_id(), Some(Ipv4Addr::new(192, 168, 1, 1)));

        let bytes = pkt.serialize();
        let parsed = DhcpPacket::parse(&bytes).unwrap();
        assert_eq!(parsed.message_type(), Some(DHCP_REQUEST));
    }

    #[test]
    fn test_build_release() {
        let config = test_config();
        let pkt = DhcpPacket::build_release(
            &config,
            0x11111111,
            Ipv4Addr::new(10, 0, 0, 5),
            Ipv4Addr::new(10, 0, 0, 1),
        );
        assert_eq!(pkt.message_type(), Some(DHCP_RELEASE));
        assert_eq!(pkt.ciaddr, Ipv4Addr::new(10, 0, 0, 5));
        assert_eq!(pkt.server_id(), Some(Ipv4Addr::new(10, 0, 0, 1)));
    }

    #[test]
    fn test_build_decline() {
        let config = test_config();
        let pkt = DhcpPacket::build_decline(
            &config,
            0x22222222,
            Ipv4Addr::new(10, 0, 0, 5),
            Ipv4Addr::new(10, 0, 0, 1),
        );
        assert_eq!(pkt.message_type(), Some(DHCP_DECLINE));
    }

    #[test]
    fn test_build_inform() {
        let config = test_config();
        let pkt = DhcpPacket::build_inform(&config, 0x33333333, Ipv4Addr::new(10, 0, 0, 5));
        assert_eq!(pkt.message_type(), Some(DHCP_INFORM));
        assert_eq!(pkt.ciaddr, Ipv4Addr::new(10, 0, 0, 5));
    }

    #[test]
    fn test_parse_invalid_magic_cookie() {
        let data = vec![0u8; 300];
        // No valid magic cookie at offset 236.
        let result = DhcpPacket::parse(&data);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("magic cookie"));
    }

    #[test]
    fn test_parse_too_short() {
        let data = vec![0u8; 100];
        assert!(DhcpPacket::parse(&data).is_err());
    }

    #[test]
    fn test_option_ipv4_list() {
        let mut pkt = DhcpPacket::new_request(1, &test_mac());
        pkt.options.push(DhcpOption {
            code: OPT_DNS,
            data: vec![8, 8, 8, 8, 1, 1, 1, 1],
        });
        let dns = pkt.get_option_ipv4_list(OPT_DNS);
        assert_eq!(dns.len(), 2);
        assert_eq!(dns[0], Ipv4Addr::new(8, 8, 8, 8));
        assert_eq!(dns[1], Ipv4Addr::new(1, 1, 1, 1));
    }

    #[test]
    fn test_option_u32() {
        let mut pkt = DhcpPacket::new_request(1, &test_mac());
        pkt.options.push(DhcpOption {
            code: OPT_LEASE_TIME,
            data: vec![0, 0, 0x0E, 0x10], // 3600
        });
        assert_eq!(pkt.get_option_u32(OPT_LEASE_TIME), Some(3600));
    }

    #[test]
    fn test_option_string() {
        let mut pkt = DhcpPacket::new_request(1, &test_mac());
        pkt.options.push(DhcpOption {
            code: OPT_HOSTNAME,
            data: b"myhost".to_vec(),
        });
        assert_eq!(pkt.get_option_string(OPT_HOSTNAME), Some("myhost".into()));
    }

    #[test]
    fn test_classless_routes() {
        // Example: 0.0.0.0/0 via 10.0.0.1 and 10.0.0.0/8 via 10.0.0.2
        let mut data = Vec::new();
        // Default route: prefix_len=0, no dest octets, gw=10.0.0.1
        data.push(0);
        data.extend_from_slice(&[10, 0, 0, 1]);
        // 10.0.0.0/8: prefix_len=8, 1 dest octet (10), gw=10.0.0.2
        data.push(8);
        data.push(10);
        data.extend_from_slice(&[10, 0, 0, 2]);

        let mut pkt = DhcpPacket::new_request(1, &test_mac());
        pkt.options.push(DhcpOption {
            code: OPT_CLASSLESS_ROUTES,
            data,
        });

        let routes = pkt.get_classless_routes();
        assert_eq!(routes.len(), 2);
        assert_eq!(
            routes[0],
            (Ipv4Addr::new(0, 0, 0, 0), 0, Ipv4Addr::new(10, 0, 0, 1))
        );
        assert_eq!(
            routes[1],
            (Ipv4Addr::new(10, 0, 0, 0), 8, Ipv4Addr::new(10, 0, 0, 2))
        );
    }

    #[test]
    fn test_classless_routes_oversized_prefix_len_does_not_panic() {
        // RFC 3442 caps an IPv4 prefix length at 32. A hostile DHCP server can
        // send a larger value; before the guard, octets = ceil(prefix/8) could
        // reach 32 while `dest` is only [u8; 4], so `dest[..octets]` panicked
        // out of bounds (an attacker-controlled DoS). It must be rejected, not
        // crash. Each payload is long enough that the length check passed and
        // the old code reached the panicking copy.
        for bad_prefix in [33u8, 40, 64, 200, 255] {
            let octets = bad_prefix.div_ceil(8) as usize;
            let mut data = vec![bad_prefix];
            data.resize(1 + octets + 4, 0xab);
            let mut pkt = DhcpPacket::new_request(1, &test_mac());
            pkt.options.push(DhcpOption {
                code: OPT_CLASSLESS_ROUTES,
                data,
            });
            let routes = pkt.get_classless_routes();
            assert!(
                routes.is_empty(),
                "bad prefix {bad_prefix} should yield no routes"
            );
        }
    }

    #[test]
    fn test_classless_routes_valid_then_malformed_keeps_valid() {
        // A valid 10.0.0.0/8 route followed by an oversized-prefix entry: the
        // valid route parses; the malformed tail is dropped without panicking.
        let mut data = vec![8, 10, 10, 0, 0, 2]; // /8 via 10.0.0.2
        data.push(40); // invalid prefix length (> 32)
        data.resize(data.len() + 9, 0);
        let mut pkt = DhcpPacket::new_request(1, &test_mac());
        pkt.options.push(DhcpOption {
            code: OPT_CLASSLESS_ROUTES,
            data,
        });
        let routes = pkt.get_classless_routes();
        assert_eq!(routes.len(), 1);
        assert_eq!(
            routes[0],
            (Ipv4Addr::new(10, 0, 0, 0), 8, Ipv4Addr::new(10, 0, 0, 2))
        );
    }

    #[test]
    fn fuzz_dhcp_parse_and_interpreters_never_panic() {
        // Robustness net: DhcpPacket::parse and the option interpreters
        // (get_classless_routes, get_option_*, to_lease) consume attacker-
        // controlled lease bytes from a DHCP server and must never panic on
        // malformed input (the classless-route prefix_len>32 overflow was one
        // such bug). Deterministically seeded random plus mutated packets, each
        // run under catch_unwind on a worker thread joined within a wall-clock
        // budget so an infinite loop fails the test instead of hanging.
        let handle = std::thread::spawn(|| {
            let seed = {
                let mut pkt = DhcpPacket::new_request(0x1234, &test_mac());
                pkt.options.push(DhcpOption {
                    code: OPT_MESSAGE_TYPE,
                    data: vec![DHCP_ACK],
                });
                pkt.options.push(DhcpOption {
                    code: OPT_CLASSLESS_ROUTES,
                    data: vec![24, 192, 168, 1, 10, 0, 0, 1],
                });
                pkt.serialize()
            };
            let mut state: u64 = 0x0bad_c0de_1234_5678;
            let mut next = || {
                state = state
                    .wrapping_mul(6_364_136_223_846_793_005)
                    .wrapping_add(1_442_695_040_888_963_407);
                (state >> 33) as u32
            };
            for iter in 0..40_000u32 {
                let mut buf = if iter % 2 == 0 {
                    seed.clone()
                } else {
                    let len = (next() % 400) as usize;
                    (0..len).map(|_| (next() & 0xff) as u8).collect::<Vec<u8>>()
                };
                let muts = 1 + (next() % 12) as usize;
                for _ in 0..muts {
                    if buf.is_empty() {
                        break;
                    }
                    match next() % 4 {
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
                        _ => {
                            let i = (next() as usize) % buf.len();
                            buf[i] = (next() & 0xff) as u8;
                        }
                    }
                }
                let res = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    if let Ok(pkt) = DhcpPacket::parse(&buf) {
                        let _ = pkt.message_type();
                        let _ = pkt.get_classless_routes();
                        for code in [
                            OPT_SUBNET_MASK,
                            OPT_ROUTER,
                            OPT_DNS,
                            OPT_NTP,
                            OPT_BROADCAST,
                            OPT_HOSTNAME,
                            OPT_DOMAIN_NAME,
                            OPT_LEASE_TIME,
                            26u8,
                            121u8,
                            43u8,
                        ] {
                            let _ = pkt.get_option_ipv4(code);
                            let _ = pkt.get_option_ipv4_list(code);
                            let _ = pkt.get_option_u32(code);
                            let _ = pkt.get_option_string(code);
                        }
                        let _ = pkt.to_lease();
                    }
                }));
                assert!(res.is_ok(), "panicked on DHCP input: {buf:02x?}");
            }
        });
        let start = std::time::Instant::now();
        while !handle.is_finished() {
            assert!(
                start.elapsed() < std::time::Duration::from_secs(60),
                "DHCP fuzz did not finish in 60s (possible infinite loop)"
            );
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        handle.join().unwrap();
    }

    #[test]
    fn test_to_lease() {
        let mut pkt = DhcpPacket::new_request(1, &test_mac());
        pkt.op = BOOTP_REPLY;
        pkt.yiaddr = Ipv4Addr::new(192, 168, 1, 100);
        pkt.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_ACK],
        });
        pkt.options.push(DhcpOption {
            code: OPT_SUBNET_MASK,
            data: vec![255, 255, 255, 0],
        });
        pkt.options.push(DhcpOption {
            code: OPT_ROUTER,
            data: vec![192, 168, 1, 1],
        });
        pkt.options.push(DhcpOption {
            code: OPT_DNS,
            data: vec![8, 8, 8, 8],
        });
        pkt.options.push(DhcpOption {
            code: OPT_LEASE_TIME,
            data: 86400u32.to_be_bytes().to_vec(),
        });
        pkt.options.push(DhcpOption {
            code: OPT_SERVER_ID,
            data: vec![192, 168, 1, 1],
        });

        let lease = pkt.to_lease().unwrap();
        assert_eq!(lease.address, Ipv4Addr::new(192, 168, 1, 100));
        assert_eq!(lease.subnet_mask, Ipv4Addr::new(255, 255, 255, 0));
        assert_eq!(lease.prefix_len(), 24);
        assert_eq!(lease.routers, vec![Ipv4Addr::new(192, 168, 1, 1)]);
        assert_eq!(lease.dns_servers, vec![Ipv4Addr::new(8, 8, 8, 8)]);
        assert_eq!(lease.lease_time, 86400);
        assert_eq!(lease.renewal_time, 43200);
        assert_eq!(lease.rebinding_time, 75600);
        assert_eq!(lease.server_id, Ipv4Addr::new(192, 168, 1, 1));
    }

    #[test]
    fn test_to_lease_rejects_non_ack() {
        let mut pkt = DhcpPacket::new_request(1, &test_mac());
        pkt.op = BOOTP_REPLY;
        pkt.yiaddr = Ipv4Addr::new(192, 168, 1, 100);
        pkt.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_OFFER],
        });
        assert!(pkt.to_lease().is_err());
    }

    #[test]
    fn test_client_state_machine_discover() {
        let config = test_config();
        let mut client = DhcpClient::new(config);
        assert_eq!(client.state, DhcpState::Init);

        let pkt = client.next_packet().unwrap();
        assert_eq!(pkt.message_type(), Some(DHCP_DISCOVER));
        assert_eq!(client.state, DhcpState::Selecting);
    }

    #[test]
    fn test_client_state_machine_offer_to_request() {
        let config = test_config();
        let mut client = DhcpClient::new(config);

        let _discover = client.next_packet().unwrap();
        assert_eq!(client.state, DhcpState::Selecting);

        // Simulate receiving an OFFER.
        let mut offer = DhcpPacket::new_request(client.xid, &test_mac());
        offer.op = BOOTP_REPLY;
        offer.xid = client.xid;
        offer.yiaddr = Ipv4Addr::new(192, 168, 1, 100);
        offer.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_OFFER],
        });
        offer.options.push(DhcpOption {
            code: OPT_SERVER_ID,
            data: vec![192, 168, 1, 1],
        });

        let result = client.process_reply(&offer);
        assert!(result.is_none()); // OFFER doesn't produce a lease yet.
        assert_eq!(client.state, DhcpState::Requesting);

        // Now get the REQUEST packet.
        let request = client.next_packet().unwrap();
        assert_eq!(request.message_type(), Some(DHCP_REQUEST));
    }

    #[test]
    fn test_client_state_machine_ack_to_bound() {
        let config = test_config();
        let mut client = DhcpClient::new(config);

        let _discover = client.next_packet().unwrap();

        // Simulate OFFER.
        let mut offer = DhcpPacket::new_request(client.xid, &test_mac());
        offer.op = BOOTP_REPLY;
        offer.xid = client.xid;
        offer.yiaddr = Ipv4Addr::new(192, 168, 1, 100);
        offer.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_OFFER],
        });
        offer.options.push(DhcpOption {
            code: OPT_SERVER_ID,
            data: vec![192, 168, 1, 1],
        });
        client.process_reply(&offer);
        assert_eq!(client.state, DhcpState::Requesting);

        // Simulate ACK.
        let mut ack = DhcpPacket::new_request(client.xid, &test_mac());
        ack.op = BOOTP_REPLY;
        ack.xid = client.xid;
        ack.yiaddr = Ipv4Addr::new(192, 168, 1, 100);
        ack.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_ACK],
        });
        ack.options.push(DhcpOption {
            code: OPT_SUBNET_MASK,
            data: vec![255, 255, 255, 0],
        });
        ack.options.push(DhcpOption {
            code: OPT_ROUTER,
            data: vec![192, 168, 1, 1],
        });
        ack.options.push(DhcpOption {
            code: OPT_LEASE_TIME,
            data: 3600u32.to_be_bytes().to_vec(),
        });
        ack.options.push(DhcpOption {
            code: OPT_SERVER_ID,
            data: vec![192, 168, 1, 1],
        });

        let lease = client.process_reply(&ack);
        assert!(lease.is_some());
        assert_eq!(client.state, DhcpState::Bound);

        let lease = lease.unwrap();
        assert_eq!(lease.address, Ipv4Addr::new(192, 168, 1, 100));
        assert_eq!(lease.lease_time, 3600);
    }

    #[test]
    fn test_client_nak_restarts() {
        let config = test_config();
        let mut client = DhcpClient::new(config);

        let _discover = client.next_packet().unwrap();

        // Simulate OFFER.
        let mut offer = DhcpPacket::new_request(client.xid, &test_mac());
        offer.op = BOOTP_REPLY;
        offer.xid = client.xid;
        offer.yiaddr = Ipv4Addr::new(192, 168, 1, 100);
        offer.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_OFFER],
        });
        offer.options.push(DhcpOption {
            code: OPT_SERVER_ID,
            data: vec![192, 168, 1, 1],
        });
        client.process_reply(&offer);

        // Simulate NAK.
        let mut nak = DhcpPacket::new_request(client.xid, &test_mac());
        nak.op = BOOTP_REPLY;
        nak.xid = client.xid;
        nak.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_NAK],
        });
        let result = client.process_reply(&nak);
        assert!(result.is_none());
        assert_eq!(client.state, DhcpState::Init);
    }

    #[test]
    fn test_client_wrong_xid_ignored() {
        let config = test_config();
        let mut client = DhcpClient::new(config);
        let _discover = client.next_packet().unwrap();

        let mut offer = DhcpPacket::new_request(client.xid + 1, &test_mac());
        offer.op = BOOTP_REPLY;
        offer.xid = client.xid + 1; // Wrong XID.
        offer.options.push(DhcpOption {
            code: OPT_MESSAGE_TYPE,
            data: vec![DHCP_OFFER],
        });
        let result = client.process_reply(&offer);
        assert!(result.is_none());
        assert_eq!(client.state, DhcpState::Selecting);
    }

    #[test]
    fn test_retransmit_timeout_exponential() {
        let config = test_config();
        let mut client = DhcpClient::new(config);
        client.attempts = 0;
        assert_eq!(client.retransmit_timeout(), Duration::from_secs(4));
        client.attempts = 1;
        assert_eq!(client.retransmit_timeout(), Duration::from_secs(8));
        client.attempts = 2;
        assert_eq!(client.retransmit_timeout(), Duration::from_secs(16));
        client.attempts = 3;
        assert_eq!(client.retransmit_timeout(), Duration::from_secs(32));
        client.attempts = 4;
        assert_eq!(client.retransmit_timeout(), Duration::from_secs(64));
        client.attempts = 10;
        assert_eq!(client.retransmit_timeout(), Duration::from_secs(64));
    }

    #[test]
    fn test_max_attempts() {
        let mut config = test_config();
        config.max_attempts = 3;
        let mut client = DhcpClient::new(config);
        assert!(!client.max_attempts_reached());
        client.attempts = 3;
        assert!(client.max_attempts_reached());
    }

    #[test]
    fn test_generate_xid_different() {
        let mac = test_mac();
        let xid1 = generate_xid(&mac);
        // Sleep a tiny bit so the timestamp differs.
        std::thread::sleep(Duration::from_millis(1));
        let xid2 = generate_xid(&mac);
        // They should almost certainly differ (different nanosecond timestamp).
        // We can't guarantee it but it's extremely unlikely to collide.
        assert_ne!(xid1, xid2);
    }

    #[test]
    fn test_build_release_from_client() {
        let config = test_config();
        let mut client = DhcpClient::new(config);

        // No lease yet, should return None.
        assert!(client.build_release().is_none());

        // Give it a lease.
        client.lease = Some(DhcpLease {
            address: Ipv4Addr::new(10, 0, 0, 5),
            subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            routers: vec![Ipv4Addr::new(10, 0, 0, 1)],
            dns_servers: vec![],
            ntp_servers: vec![],
            broadcast: None,
            hostname: None,
            domain_name: None,
            classless_routes: vec![],
            server_id: Ipv4Addr::new(10, 0, 0, 1),
            lease_time: 3600,
            renewal_time: 1800,
            rebinding_time: 3150,
            mtu: None,
            obtained_at: Instant::now(),
        });

        let release = client.build_release().unwrap();
        assert_eq!(release.message_type(), Some(DHCP_RELEASE));
        assert_eq!(release.ciaddr, Ipv4Addr::new(10, 0, 0, 5));
    }

    #[test]
    fn test_udp_ip_packet_build_and_extract() {
        let payload = b"test dhcp data here for roundtrip";
        let pkt = build_udp_ip_packet(
            Ipv4Addr::UNSPECIFIED,
            Ipv4Addr::BROADCAST,
            DHCP_CLIENT_PORT,
            DHCP_SERVER_PORT,
            payload,
        );

        // Verify IP header.
        assert_eq!(pkt[0], 0x45); // IPv4, IHL=5
        assert_eq!(pkt[9], 17); // UDP protocol

        // Extract payload for client port (should fail since we sent to server).
        assert!(extract_dhcp_payload(&pkt, DHCP_CLIENT_PORT).is_none());

        // Extract payload for server port.
        let extracted = extract_dhcp_payload(&pkt, DHCP_SERVER_PORT).unwrap();
        assert_eq!(extracted, payload);
    }

    #[test]
    fn test_extract_short_packet() {
        assert!(extract_dhcp_payload(&[0; 10], DHCP_CLIENT_PORT).is_none());
    }

    #[test]
    fn test_extract_non_ipv4() {
        let mut data = vec![0u8; 50];
        data[0] = 0x60; // IPv6 version nibble.
        assert!(extract_dhcp_payload(&data, DHCP_CLIENT_PORT).is_none());
    }

    #[test]
    fn test_extract_non_udp() {
        let mut data = vec![0u8; 50];
        data[0] = 0x45; // IPv4
        data[9] = 6; // TCP
        assert!(extract_dhcp_payload(&data, DHCP_CLIENT_PORT).is_none());
    }

    #[test]
    fn test_ip_checksum() {
        // Example from RFC 1071.
        let header: [u8; 20] = [
            0x45, 0x00, 0x00, 0x73, 0x00, 0x00, 0x40, 0x00, 0x40, 0x11, 0x00, 0x00, 0xc0, 0xa8,
            0x00, 0x01, 0xc0, 0xa8, 0x00, 0xc7,
        ];
        let cksum = ip_checksum(&header);
        // Verify by putting checksum back in and recalculating.
        let mut with_cksum = header;
        with_cksum[10] = (cksum >> 8) as u8;
        with_cksum[11] = (cksum & 0xff) as u8;
        assert_eq!(ip_checksum(&with_cksum), 0);
    }

    #[test]
    fn test_lease_timing() {
        let lease = DhcpLease {
            address: Ipv4Addr::new(10, 0, 0, 5),
            subnet_mask: Ipv4Addr::new(255, 255, 255, 0),
            routers: vec![],
            dns_servers: vec![],
            ntp_servers: vec![],
            broadcast: None,
            hostname: None,
            domain_name: None,
            classless_routes: vec![],
            server_id: Ipv4Addr::new(10, 0, 0, 1),
            lease_time: 3600,
            renewal_time: 1800,
            rebinding_time: 3150,
            mtu: None,
            obtained_at: Instant::now(),
        };

        assert!(!lease.is_expired());
        assert!(!lease.needs_renewal());
        assert!(!lease.needs_rebinding());
        assert!(lease.remaining() > Duration::from_secs(3500));
    }

    #[test]
    fn test_dhcp_message_type_name() {
        assert_eq!(dhcp_message_type_name(DHCP_DISCOVER), "DISCOVER");
        assert_eq!(dhcp_message_type_name(DHCP_OFFER), "OFFER");
        assert_eq!(dhcp_message_type_name(DHCP_REQUEST), "REQUEST");
        assert_eq!(dhcp_message_type_name(DHCP_ACK), "ACK");
        assert_eq!(dhcp_message_type_name(DHCP_NAK), "NAK");
        assert_eq!(dhcp_message_type_name(DHCP_RELEASE), "RELEASE");
        assert_eq!(dhcp_message_type_name(99), "UNKNOWN");
    }

    #[test]
    fn test_serialize_roundtrip() {
        let config = test_config();
        let original = DhcpPacket::build_discover(&config, 0xDEADBEEF);
        let bytes = original.serialize();
        let parsed = DhcpPacket::parse(&bytes).unwrap();

        assert_eq!(parsed.op, original.op);
        assert_eq!(parsed.xid, original.xid);
        assert_eq!(parsed.message_type(), original.message_type());
        assert_eq!(parsed.chaddr, original.chaddr);
        assert_eq!(parsed.ciaddr, original.ciaddr);
        assert_eq!(parsed.yiaddr, original.yiaddr);
    }
}
