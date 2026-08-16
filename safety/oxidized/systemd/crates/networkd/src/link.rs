//! Network link management via the Linux netlink (rtnetlink) interface.
//!
//! This module provides functions to:
//! - Enumerate network interfaces and their addresses
//! - Add/remove IPv4 addresses on interfaces
//! - Add/remove IPv4 routes
//! - Bring interfaces up/down
//! - Set MTU
//! - Read interface MAC addresses
//!
//! All operations use raw netlink sockets (AF_NETLINK / NETLINK_ROUTE) so
//! there is no dependency on external crates beyond `libc` and `nix`.

use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::Ipv4Addr;

// ---------------------------------------------------------------------------
// Netlink constants (from linux/netlink.h, linux/rtnetlink.h, linux/if.h)
// ---------------------------------------------------------------------------

const NETLINK_ROUTE: i32 = 0;

// Netlink message types (RTM_*)
const RTM_NEWLINK: u16 = 16;
const RTM_DELLINK: u16 = 17;
const RTM_GETLINK: u16 = 18;
const RTM_NEWADDR: u16 = 20;
const RTM_DELADDR: u16 = 21;
const RTM_GETADDR: u16 = 22;
const RTM_NEWROUTE: u16 = 24;
const RTM_DELROUTE: u16 = 25;
const RTM_GETROUTE: u16 = 26;
const RTM_NEWRULE: u16 = 32;
const RTM_DELRULE: u16 = 33;

// Netlink flags
const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_MULTI: u16 = 0x0002;
const NLM_F_ACK: u16 = 0x0004;
const NLM_F_ROOT: u16 = 0x0100;
const NLM_F_MATCH: u16 = 0x0200;
const NLM_F_DUMP: u16 = NLM_F_ROOT | NLM_F_MATCH;
const NLM_F_CREATE: u16 = 0x0400;
const NLM_F_EXCL: u16 = 0x0200;
const NLM_F_REPLACE: u16 = 0x0100;

// Netlink message type for errors/acks
const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;
const NLMSG_NOOP: u16 = 1;

// Address families
const AF_INET: u8 = 2;
const AF_INET6: u8 = 10;
const AF_UNSPEC: u8 = 0;

// Interface link attributes (IFLA_*)
const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_ADDRESS: u16 = 1; // hardware address
const IFLA_OPERSTATE: u16 = 16;

// Address attributes (IFA_*)
const IFA_ADDRESS: u16 = 1;
const IFA_LOCAL: u16 = 2;
const IFA_LABEL: u16 = 3;
const IFA_BROADCAST: u16 = 4;

// Route attributes (RTA_*)
const RTA_DST: u16 = 1;
const RTA_GATEWAY: u16 = 5;
const RTA_OIF: u16 = 4;
const RTA_PRIORITY: u16 = 6;
const RTA_TABLE: u16 = 15;
const RTA_PREFSRC: u16 = 7;

// Routing policy rule attributes (FRA_*)
const FRA_DST: u16 = 1;
const FRA_SRC: u16 = 2;
const FRA_IFNAME: u16 = 3; // incoming interface
const FRA_TABLE: u16 = 15;
const FRA_FWMARK: u16 = 10;
const FRA_FWMASK: u16 = 16;
const FRA_PRIORITY: u16 = 6;
const FRA_OIFNAME: u16 = 17;
const FRA_SUPPRESS_PREFIXLEN: u16 = 14;
const FRA_IP_PROTO: u16 = 19;
const FRA_SPORT_RANGE: u16 = 20;
const FRA_DPORT_RANGE: u16 = 21;
const FRA_UID_RANGE: u16 = 22;

// Route table
const RT_TABLE_MAIN: u8 = 254;

// Route protocol
const RTPROT_DHCP: u8 = 16;
const RTPROT_STATIC: u8 = 4;
const RTPROT_BOOT: u8 = 3;

// Route types
const RTN_UNICAST: u8 = 1;
const RTN_BLACKHOLE: u8 = 6;
const RTN_UNREACHABLE: u8 = 7;
const RTN_PROHIBIT: u8 = 8;

// FIB rule action (same codes as route types for the rule action field)
const FR_ACT_TO_TBL: u8 = 1; // look up in routing table
const FR_ACT_BLACKHOLE: u8 = RTN_BLACKHOLE;
const FR_ACT_UNREACHABLE: u8 = RTN_UNREACHABLE;
const FR_ACT_PROHIBIT: u8 = RTN_PROHIBIT;

// FIB rule flags
const FIB_RULE_INVERT: u32 = 0x00000002;

// Route scopes
const RT_SCOPE_UNIVERSE: u8 = 0;
const RT_SCOPE_LINK: u8 = 253;

// IFF_* flags
const IFF_UP: u32 = 0x1;
const IFF_RUNNING: u32 = 0x40;
const IFF_LOOPBACK: u32 = 0x8;

// Operstate values
const IF_OPER_UP: u8 = 6;
const IF_OPER_DOWN: u8 = 2;
const IF_OPER_UNKNOWN: u8 = 0;

// Netlink header size
const NLMSG_HDR_LEN: usize = 16;
const NLMSG_ALIGN: usize = 4;

// ifinfomsg size (family + pad + type + index + flags + change)
const IFINFOMSG_LEN: usize = 16;

// ifaddrmsg size
const IFADDRMSG_LEN: usize = 8;

// rtmsg size
const RTMSG_LEN: usize = 12;

// fib_rule_hdr size (same layout as rtmsg: family, dst_len, src_len, tos,
// table, res1, res2, action — 12 bytes)
const FIB_RULE_HDR_LEN: usize = 12;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// Information about a network interface (link).
#[derive(Debug, Clone)]
pub struct LinkInfo {
    /// Kernel interface index.
    pub index: u32,
    /// Interface name (e.g. `eth0`, `ens3`).
    pub name: String,
    /// Hardware (MAC) address as colon-separated hex string.
    pub mac: String,
    /// Raw MAC bytes.
    pub mac_bytes: Vec<u8>,
    /// MTU in bytes.
    pub mtu: u32,
    /// IFF_* flags.
    pub flags: u32,
    /// Operational state.
    pub operstate: u8,
}

impl LinkInfo {
    pub fn is_up(&self) -> bool {
        (self.flags & IFF_UP) != 0
    }

    pub fn is_running(&self) -> bool {
        (self.flags & IFF_RUNNING) != 0
    }

    pub fn is_loopback(&self) -> bool {
        (self.flags & IFF_LOOPBACK) != 0
    }

    /// Human-readable operational state.
    pub fn oper_state_str(&self) -> &'static str {
        match self.operstate {
            IF_OPER_UP => "up",
            IF_OPER_DOWN => "down",
            IF_OPER_UNKNOWN => "unknown",
            1 => "not-present",
            3 => "testing",
            4 => "dormant",
            5 => "lower-layer-down",
            _ => "unknown",
        }
    }

    /// Formatted MAC address for display.
    pub fn mac_display(&self) -> &str {
        &self.mac
    }
}

impl fmt::Display for LinkInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}: {} mac={} mtu={} state={}",
            self.index,
            self.name,
            self.mac,
            self.mtu,
            self.oper_state_str()
        )
    }
}

/// Information about an address assigned to an interface.
#[derive(Debug, Clone)]
pub struct AddrInfo {
    /// Interface index this address belongs to.
    pub ifindex: u32,
    /// Address family (AF_INET = 2, AF_INET6 = 10).
    pub family: u8,
    /// Prefix length.
    pub prefix_len: u8,
    /// The address (for IPv4, stored as Ipv4Addr).
    pub address: Ipv4Addr,
    /// Local address (may differ for point-to-point).
    pub local: Option<Ipv4Addr>,
    /// Broadcast address.
    pub broadcast: Option<Ipv4Addr>,
    /// Label (e.g. `eth0:1`).
    pub label: Option<String>,
    /// Scope.
    pub scope: u8,
}

impl fmt::Display for AddrInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}/{}", self.address, self.prefix_len)?;
        if let Some(ref brd) = self.broadcast {
            write!(f, " brd {brd}")?;
        }
        if let Some(ref label) = self.label {
            write!(f, " label {label}")?;
        }
        Ok(())
    }
}

/// Information about a route.
#[derive(Debug, Clone)]
pub struct RouteInfo {
    /// Destination network (0.0.0.0 for default route).
    pub destination: Ipv4Addr,
    /// Destination prefix length.
    pub dst_len: u8,
    /// Gateway address.
    pub gateway: Option<Ipv4Addr>,
    /// Output interface index.
    pub oif: Option<u32>,
    /// Route priority / metric.
    pub priority: Option<u32>,
    /// Route protocol (RTPROT_*).
    pub protocol: u8,
    /// Route scope.
    pub scope: u8,
    /// Route table.
    pub table: u8,
    /// Preferred source address.
    pub prefsrc: Option<Ipv4Addr>,
}

impl fmt::Display for RouteInfo {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.dst_len == 0 {
            write!(f, "default")?;
        } else {
            write!(f, "{}/{}", self.destination, self.dst_len)?;
        }
        if let Some(gw) = self.gateway {
            write!(f, " via {gw}")?;
        }
        if let Some(oif) = self.oif {
            write!(f, " dev idx={oif}")?;
        }
        if let Some(prio) = self.priority {
            write!(f, " metric {prio}")?;
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Netlink socket wrapper
// ---------------------------------------------------------------------------

/// A netlink route socket for communicating with the kernel.
pub struct NetlinkSocket {
    fd: i32,
    seq: u32,
    pub(crate) pid: u32,
}

impl NetlinkSocket {
    /// Open a new NETLINK_ROUTE socket.
    pub fn open() -> io::Result<Self> {
        let fd = unsafe {
            libc::socket(
                libc::AF_NETLINK,
                libc::SOCK_RAW | libc::SOCK_CLOEXEC,
                NETLINK_ROUTE,
            )
        };
        if fd < 0 {
            return Err(io::Error::last_os_error());
        }

        // Bind to an auto-assigned port.
        let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        addr.nl_family = libc::AF_NETLINK as u16;
        addr.nl_pid = 0; // Let kernel assign.
        addr.nl_groups = 0;

        let ret = unsafe {
            libc::bind(
                fd,
                &addr as *const libc::sockaddr_nl as *const libc::sockaddr,
                std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t,
            )
        };
        if ret < 0 {
            let err = io::Error::last_os_error();
            unsafe { libc::close(fd) };
            return Err(err);
        }

        // Read back the assigned PID.
        let mut bound_addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
        let mut addrlen = std::mem::size_of::<libc::sockaddr_nl>() as libc::socklen_t;
        unsafe {
            libc::getsockname(
                fd,
                &mut bound_addr as *mut libc::sockaddr_nl as *mut libc::sockaddr,
                &mut addrlen,
            );
        }

        // Set a receive timeout so we don't block forever.
        let tv = libc::timeval {
            tv_sec: 5,
            tv_usec: 0,
        };
        unsafe {
            libc::setsockopt(
                fd,
                libc::SOL_SOCKET,
                libc::SO_RCVTIMEO,
                &tv as *const libc::timeval as *const libc::c_void,
                std::mem::size_of::<libc::timeval>() as libc::socklen_t,
            );
        }

        Ok(Self {
            fd,
            seq: 1,
            pid: bound_addr.nl_pid,
        })
    }

    /// Send a netlink message and collect all response messages.
    pub(crate) fn request(&mut self, msg: &[u8]) -> io::Result<Vec<Vec<u8>>> {
        // Send.
        let sent =
            unsafe { libc::send(self.fd, msg.as_ptr() as *const libc::c_void, msg.len(), 0) };
        if sent < 0 {
            return Err(io::Error::last_os_error());
        }

        // Receive response(s).
        let mut responses = Vec::new();
        let mut buf = vec![0u8; 65536];

        loop {
            let n =
                unsafe { libc::recv(self.fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
            if n < 0 {
                let err = io::Error::last_os_error();
                if err.kind() == io::ErrorKind::WouldBlock || err.kind() == io::ErrorKind::TimedOut
                {
                    break;
                }
                return Err(err);
            }
            let n = n as usize;
            if n == 0 {
                break;
            }

            // Parse individual netlink messages from the received buffer.
            let mut offset = 0;
            let mut done = false;
            while offset + NLMSG_HDR_LEN <= n {
                let nlmsg_len =
                    u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap()) as usize;
                let nlmsg_type =
                    u16::from_ne_bytes(buf[offset + 4..offset + 6].try_into().unwrap());
                let nlmsg_flags =
                    u16::from_ne_bytes(buf[offset + 6..offset + 8].try_into().unwrap());

                if nlmsg_len < NLMSG_HDR_LEN || offset + nlmsg_len > n {
                    break;
                }

                match nlmsg_type {
                    NLMSG_DONE => {
                        done = true;
                        break;
                    }
                    NLMSG_ERROR => {
                        // Error message: 16-byte nlmsghdr + 4-byte errno + original header.
                        if nlmsg_len >= NLMSG_HDR_LEN + 4 {
                            let errno = i32::from_ne_bytes(
                                buf[offset + NLMSG_HDR_LEN..offset + NLMSG_HDR_LEN + 4]
                                    .try_into()
                                    .unwrap(),
                            );
                            if errno < 0 {
                                return Err(io::Error::from_raw_os_error(-errno));
                            }
                            // errno == 0 means ACK.
                        }
                        done = true;
                        break;
                    }
                    NLMSG_NOOP => {}
                    _ => {
                        responses.push(buf[offset..offset + nlmsg_len].to_vec());
                    }
                }

                // Check if this is a multi-part message.
                if (nlmsg_flags & NLM_F_MULTI) == 0 && nlmsg_type != NLMSG_DONE {
                    // Single message, may still need to read more (ACK).
                }

                offset += nlmsg_align(nlmsg_len);
            }

            if done {
                break;
            }
        }

        Ok(responses)
    }

    pub(crate) fn next_seq(&mut self) -> u32 {
        let s = self.seq;
        self.seq = self.seq.wrapping_add(1);
        s
    }
}

impl Drop for NetlinkSocket {
    fn drop(&mut self) {
        unsafe {
            libc::close(self.fd);
        }
    }
}

// ---------------------------------------------------------------------------
// Public API — queries
// ---------------------------------------------------------------------------

/// Enumerate all network interfaces.
pub fn list_links() -> io::Result<Vec<LinkInfo>> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    // Build RTM_GETLINK dump request.
    let msg_len = NLMSG_HDR_LEN + IFINFOMSG_LEN;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    // nlmsghdr
    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_GETLINK);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_DUMP);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // ifinfomsg — family = AF_UNSPEC
    msg[NLMSG_HDR_LEN] = AF_UNSPEC;

    let responses = nl.request(&msg)?;
    let mut links = Vec::new();

    for resp in &responses {
        if resp.len() < NLMSG_HDR_LEN + IFINFOMSG_LEN {
            continue;
        }
        let nlmsg_type = u16::from_ne_bytes(resp[4..6].try_into().unwrap());
        if nlmsg_type != RTM_NEWLINK {
            continue;
        }

        let payload = &resp[NLMSG_HDR_LEN..];
        let _family = payload[0];
        let _pad = payload[1];
        let _ifi_type = u16::from_ne_bytes(payload[2..4].try_into().unwrap());
        let ifi_index = i32::from_ne_bytes(payload[4..8].try_into().unwrap()) as u32;
        let ifi_flags = u32::from_ne_bytes(payload[8..12].try_into().unwrap());

        let attrs = parse_rtattrs(&payload[IFINFOMSG_LEN..]);

        let name = attrs
            .get(&IFLA_IFNAME)
            .and_then(|d| {
                let s = std::str::from_utf8(d).ok()?;
                Some(s.trim_end_matches('\0').to_string())
            })
            .unwrap_or_default();

        let mac_bytes = attrs.get(&IFLA_ADDRESS).cloned().unwrap_or_default();
        let mac = mac_bytes
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<Vec<_>>()
            .join(":");

        let mtu = attrs
            .get(&IFLA_MTU)
            .and_then(|d| {
                if d.len() >= 4 {
                    Some(u32::from_ne_bytes(d[..4].try_into().unwrap()))
                } else {
                    None
                }
            })
            .unwrap_or(0);

        let operstate = attrs
            .get(&IFLA_OPERSTATE)
            .and_then(|d| d.first().copied())
            .unwrap_or(IF_OPER_UNKNOWN);

        links.push(LinkInfo {
            index: ifi_index,
            name,
            mac,
            mac_bytes,
            mtu,
            flags: ifi_flags,
            operstate,
        });
    }

    Ok(links)
}

/// Enumerate all IPv4 addresses on all interfaces.
pub fn list_addresses() -> io::Result<Vec<AddrInfo>> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    let msg_len = NLMSG_HDR_LEN + IFADDRMSG_LEN;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_GETADDR);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_DUMP);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // ifaddrmsg — family = AF_INET for IPv4 only.
    msg[NLMSG_HDR_LEN] = AF_INET;

    let responses = nl.request(&msg)?;
    let mut addrs = Vec::new();

    for resp in &responses {
        if resp.len() < NLMSG_HDR_LEN + IFADDRMSG_LEN {
            continue;
        }
        let nlmsg_type = u16::from_ne_bytes(resp[4..6].try_into().unwrap());
        if nlmsg_type != RTM_NEWADDR {
            continue;
        }

        let payload = &resp[NLMSG_HDR_LEN..];
        let family = payload[0];
        if family != AF_INET {
            continue;
        }
        let prefix_len = payload[1];
        let _flags = payload[2];
        let scope = payload[3];
        let ifindex = u32::from_ne_bytes(payload[4..8].try_into().unwrap());

        let attrs = parse_rtattrs(&payload[IFADDRMSG_LEN..]);

        let address = attrs
            .get(&IFA_ADDRESS)
            .and_then(|d| {
                if d.len() >= 4 {
                    Some(Ipv4Addr::new(d[0], d[1], d[2], d[3]))
                } else {
                    None
                }
            })
            .unwrap_or(Ipv4Addr::UNSPECIFIED);

        let local = attrs.get(&IFA_LOCAL).and_then(|d| {
            if d.len() >= 4 {
                Some(Ipv4Addr::new(d[0], d[1], d[2], d[3]))
            } else {
                None
            }
        });

        let broadcast = attrs.get(&IFA_BROADCAST).and_then(|d| {
            if d.len() >= 4 {
                Some(Ipv4Addr::new(d[0], d[1], d[2], d[3]))
            } else {
                None
            }
        });

        let label = attrs.get(&IFA_LABEL).and_then(|d| {
            std::str::from_utf8(d)
                .ok()
                .map(|s| s.trim_end_matches('\0').to_string())
        });

        addrs.push(AddrInfo {
            ifindex,
            family,
            prefix_len,
            address,
            local,
            broadcast,
            label,
            scope,
        });
    }

    Ok(addrs)
}

/// Enumerate all IPv4 routes (main table).
pub fn list_routes() -> io::Result<Vec<RouteInfo>> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    let msg_len = NLMSG_HDR_LEN + RTMSG_LEN;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_GETROUTE);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_DUMP);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // rtmsg — family = AF_INET.
    let rthdr = NLMSG_HDR_LEN;
    msg[rthdr] = AF_INET; // rtm_family
    msg[rthdr + 4] = RT_TABLE_MAIN; // rtm_table

    let responses = nl.request(&msg)?;
    let mut routes = Vec::new();

    for resp in &responses {
        if resp.len() < NLMSG_HDR_LEN + RTMSG_LEN {
            continue;
        }
        let nlmsg_type = u16::from_ne_bytes(resp[4..6].try_into().unwrap());
        if nlmsg_type != RTM_NEWROUTE {
            continue;
        }

        let payload = &resp[NLMSG_HDR_LEN..];
        let family = payload[0];
        if family != AF_INET {
            continue;
        }
        let dst_len = payload[1];
        let _src_len = payload[2];
        let _tos = payload[3];
        let table = payload[4];
        let protocol = payload[5];
        let scope = payload[6];
        let _route_type = payload[7];

        // Only main table routes.
        if table != RT_TABLE_MAIN {
            continue;
        }

        let attrs = parse_rtattrs(&payload[RTMSG_LEN..]);

        let destination = attrs
            .get(&RTA_DST)
            .and_then(|d| {
                if d.len() >= 4 {
                    Some(Ipv4Addr::new(d[0], d[1], d[2], d[3]))
                } else {
                    None
                }
            })
            .unwrap_or(Ipv4Addr::UNSPECIFIED);

        let gateway = attrs.get(&RTA_GATEWAY).and_then(|d| {
            if d.len() >= 4 {
                Some(Ipv4Addr::new(d[0], d[1], d[2], d[3]))
            } else {
                None
            }
        });

        let oif = attrs.get(&RTA_OIF).and_then(|d| {
            if d.len() >= 4 {
                Some(u32::from_ne_bytes(d[..4].try_into().unwrap()))
            } else {
                None
            }
        });

        let priority = attrs.get(&RTA_PRIORITY).and_then(|d| {
            if d.len() >= 4 {
                Some(u32::from_ne_bytes(d[..4].try_into().unwrap()))
            } else {
                None
            }
        });

        let prefsrc = attrs.get(&RTA_PREFSRC).and_then(|d| {
            if d.len() >= 4 {
                Some(Ipv4Addr::new(d[0], d[1], d[2], d[3]))
            } else {
                None
            }
        });

        routes.push(RouteInfo {
            destination,
            dst_len,
            gateway,
            oif,
            priority,
            protocol,
            scope,
            table,
            prefsrc,
        });
    }

    Ok(routes)
}

/// Find a link by name.
pub fn find_link_by_name(name: &str) -> io::Result<Option<LinkInfo>> {
    let links = list_links()?;
    Ok(links.into_iter().find(|l| l.name == name))
}

/// Find a link by index.
pub fn find_link_by_index(index: u32) -> io::Result<Option<LinkInfo>> {
    let links = list_links()?;
    Ok(links.into_iter().find(|l| l.index == index))
}

/// Get all IPv4 addresses for a specific interface index.
pub fn get_addresses_for_link(ifindex: u32) -> io::Result<Vec<AddrInfo>> {
    let addrs = list_addresses()?;
    Ok(addrs.into_iter().filter(|a| a.ifindex == ifindex).collect())
}

/// Build a map from interface index to interface name.
pub fn build_ifindex_name_map() -> io::Result<HashMap<u32, String>> {
    let links = list_links()?;
    Ok(links.into_iter().map(|l| (l.index, l.name)).collect())
}

// ---------------------------------------------------------------------------
// Public API — modifications
// ---------------------------------------------------------------------------

/// Set a network interface up or down.
pub fn set_link_up(ifindex: u32, up: bool) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    let msg_len = NLMSG_HDR_LEN + IFINFOMSG_LEN;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_NEWLINK);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_ACK);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // ifinfomsg
    let ifi = NLMSG_HDR_LEN;
    msg[ifi] = AF_UNSPEC;
    put_i32(&mut msg, ifi + 4, ifindex as i32); // ifi_index
    let flags: u32 = if up { IFF_UP } else { 0 };
    put_u32(&mut msg, ifi + 8, flags); // ifi_flags
    put_u32(&mut msg, ifi + 12, IFF_UP); // ifi_change (mask)

    nl.request(&msg)?;
    Ok(())
}

/// Set the MTU on a network interface.
pub fn set_link_mtu(ifindex: u32, mtu: u32) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    let attr_len = rta_aligned_len(4);
    let msg_len = NLMSG_HDR_LEN + IFINFOMSG_LEN + attr_len;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_NEWLINK);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_ACK);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    let ifi = NLMSG_HDR_LEN;
    msg[ifi] = AF_UNSPEC;
    put_i32(&mut msg, ifi + 4, ifindex as i32);

    // IFLA_MTU attribute
    let attr_off = NLMSG_HDR_LEN + IFINFOMSG_LEN;
    put_rta_u32(&mut msg, attr_off, IFLA_MTU, mtu);

    nl.request(&msg)?;
    Ok(())
}

/// Add an IPv4 address to an interface.
pub fn add_address(
    ifindex: u32,
    address: Ipv4Addr,
    prefix_len: u8,
    broadcast: Option<Ipv4Addr>,
) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    // Calculate attribute sizes
    let addr_attr_len = rta_aligned_len(4); // IFA_LOCAL
    let addr2_attr_len = rta_aligned_len(4); // IFA_ADDRESS
    let brd_attr_len = if broadcast.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let msg_len = NLMSG_HDR_LEN + IFADDRMSG_LEN + addr_attr_len + addr2_attr_len + brd_attr_len;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_NEWADDR);
    put_u16(
        &mut msg,
        6,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
    );
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // ifaddrmsg
    let ifa = NLMSG_HDR_LEN;
    msg[ifa] = AF_INET; // ifa_family
    msg[ifa + 1] = prefix_len; // ifa_prefixlen
    msg[ifa + 2] = 0; // ifa_flags
    msg[ifa + 3] = RT_SCOPE_UNIVERSE; // ifa_scope
    put_u32(&mut msg, ifa + 4, ifindex); // ifa_index

    // IFA_LOCAL
    let mut off = NLMSG_HDR_LEN + IFADDRMSG_LEN;
    put_rta_ipv4(&mut msg, off, IFA_LOCAL, address);
    off += addr_attr_len;

    // IFA_ADDRESS
    put_rta_ipv4(&mut msg, off, IFA_ADDRESS, address);
    off += addr2_attr_len;

    // IFA_BROADCAST
    if let Some(brd) = broadcast {
        put_rta_ipv4(&mut msg, off, IFA_BROADCAST, brd);
    }

    nl.request(&msg)?;
    Ok(())
}

/// Remove an IPv4 address from an interface.
pub fn del_address(ifindex: u32, address: Ipv4Addr, prefix_len: u8) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    let addr_attr_len = rta_aligned_len(4);
    let msg_len = NLMSG_HDR_LEN + IFADDRMSG_LEN + addr_attr_len;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_DELADDR);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_ACK);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    let ifa = NLMSG_HDR_LEN;
    msg[ifa] = AF_INET;
    msg[ifa + 1] = prefix_len;
    msg[ifa + 3] = RT_SCOPE_UNIVERSE;
    put_u32(&mut msg, ifa + 4, ifindex);

    put_rta_ipv4(&mut msg, NLMSG_HDR_LEN + IFADDRMSG_LEN, IFA_LOCAL, address);

    nl.request(&msg)?;
    Ok(())
}

/// Add an IPv4 route.
pub fn add_route(
    destination: Ipv4Addr,
    dst_prefix_len: u8,
    gateway: Option<Ipv4Addr>,
    ifindex: u32,
    metric: Option<u32>,
    protocol: u8,
) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    // Calculate size of route attributes.
    let dst_len = if dst_prefix_len > 0 {
        rta_aligned_len(4)
    } else {
        0
    };
    let gw_len = if gateway.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let oif_len = rta_aligned_len(4);
    let metric_len = if metric.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };

    let msg_len = NLMSG_HDR_LEN + RTMSG_LEN + dst_len + gw_len + oif_len + metric_len;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_NEWROUTE);
    put_u16(
        &mut msg,
        6,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
    );
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // rtmsg
    let rt = NLMSG_HDR_LEN;
    msg[rt] = AF_INET; // rtm_family
    msg[rt + 1] = dst_prefix_len; // rtm_dst_len
    msg[rt + 2] = 0; // rtm_src_len
    msg[rt + 3] = 0; // rtm_tos
    msg[rt + 4] = RT_TABLE_MAIN; // rtm_table
    msg[rt + 5] = protocol; // rtm_protocol
    msg[rt + 6] = if gateway.is_some() {
        RT_SCOPE_UNIVERSE
    } else {
        RT_SCOPE_LINK
    }; // rtm_scope
    msg[rt + 7] = RTN_UNICAST; // rtm_type

    let mut off = NLMSG_HDR_LEN + RTMSG_LEN;

    // RTA_DST
    if dst_prefix_len > 0 {
        put_rta_ipv4(&mut msg, off, RTA_DST, destination);
        off += dst_len;
    }

    // RTA_GATEWAY
    if let Some(gw) = gateway {
        put_rta_ipv4(&mut msg, off, RTA_GATEWAY, gw);
        off += gw_len;
    }

    // RTA_OIF
    put_rta_u32(&mut msg, off, RTA_OIF, ifindex);
    off += oif_len;

    // RTA_PRIORITY (metric)
    if let Some(m) = metric {
        put_rta_u32(&mut msg, off, RTA_PRIORITY, m);
    }

    nl.request(&msg)?;
    Ok(())
}

/// Delete an IPv4 route.
pub fn del_route(
    destination: Ipv4Addr,
    dst_prefix_len: u8,
    gateway: Option<Ipv4Addr>,
    ifindex: u32,
) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    let dst_len = if dst_prefix_len > 0 {
        rta_aligned_len(4)
    } else {
        0
    };
    let gw_len = if gateway.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let oif_len = rta_aligned_len(4);

    let msg_len = NLMSG_HDR_LEN + RTMSG_LEN + dst_len + gw_len + oif_len;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_DELROUTE);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_ACK);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    let rt = NLMSG_HDR_LEN;
    msg[rt] = AF_INET;
    msg[rt + 1] = dst_prefix_len;
    msg[rt + 4] = RT_TABLE_MAIN;
    msg[rt + 6] = if gateway.is_some() {
        RT_SCOPE_UNIVERSE
    } else {
        RT_SCOPE_LINK
    };
    msg[rt + 7] = RTN_UNICAST;

    let mut off = NLMSG_HDR_LEN + RTMSG_LEN;

    if dst_prefix_len > 0 {
        put_rta_ipv4(&mut msg, off, RTA_DST, destination);
        off += dst_len;
    }

    if let Some(gw) = gateway {
        put_rta_ipv4(&mut msg, off, RTA_GATEWAY, gw);
        off += gw_len;
    }

    put_rta_u32(&mut msg, off, RTA_OIF, ifindex);

    nl.request(&msg)?;
    Ok(())
}

/// Add a default route via the given gateway.
pub fn add_default_route(
    gateway: Ipv4Addr,
    ifindex: u32,
    metric: Option<u32>,
    protocol: u8,
) -> io::Result<()> {
    add_route(
        Ipv4Addr::UNSPECIFIED,
        0,
        Some(gateway),
        ifindex,
        metric,
        protocol,
    )
}

/// Delete all IPv4 addresses on an interface (except link-local).
pub fn flush_addresses(ifindex: u32) -> io::Result<()> {
    let addrs = get_addresses_for_link(ifindex)?;
    for addr in addrs {
        if addr.address.is_link_local() {
            continue;
        }
        if let Err(e) = del_address(ifindex, addr.address, addr.prefix_len) {
            log::warn!(
                "Failed to remove address {}/{} from ifindex {}: {}",
                addr.address,
                addr.prefix_len,
                ifindex,
                e
            );
        }
    }
    Ok(())
}

/// Flush all routes going through a specific interface.
pub fn flush_routes(ifindex: u32) -> io::Result<()> {
    let routes = list_routes()?;
    for route in routes {
        if route.oif == Some(ifindex)
            && let Err(e) = del_route(route.destination, route.dst_len, route.gateway, ifindex)
        {
            log::warn!(
                "Failed to remove route {} via {:?} on ifindex {}: {}",
                route.destination,
                route.gateway,
                ifindex,
                e
            );
        }
    }
    Ok(())
}

/// Write DNS server addresses to /run/systemd/resolve/resolv.conf.
///
/// When systemd-resolved is running it owns `/run/systemd/resolve/` and
/// reads per-link DNS configuration from networkd's state files in
/// `/run/systemd/netif/links/`.  In that case networkd must **not** write
/// to resolved's directory — doing so causes "Permission denied" errors
/// because resolved restricts the directory to its own user.
///
/// We detect resolved by checking for `/run/systemd/resolve/stub-resolv.conf`
/// which resolved creates on startup.
pub fn write_resolv_conf(
    dns_servers: &[Ipv4Addr],
    dns6_servers: &[std::net::Ipv6Addr],
    search_domains: &[String],
) -> io::Result<()> {
    // If systemd-resolved is running it manages resolv.conf itself.
    // It picks up per-link DNS from /run/systemd/netif/links/ state files
    // that networkd already writes, so we can safely skip.
    let stub = std::path::Path::new("/run/systemd/resolve/stub-resolv.conf");
    if stub.exists() {
        log::debug!(
            "systemd-resolved is running (stub-resolv.conf exists), \
             skipping resolv.conf write — resolved reads per-link DNS from netif state files"
        );
        return Ok(());
    }

    let dir = std::path::Path::new("/run/systemd/resolve");
    if !dir.exists() {
        std::fs::create_dir_all(dir)?;
    }

    let mut content = String::new();
    content.push_str("# Generated by systemd-networkd\n");
    if !search_domains.is_empty() {
        content.push_str("search");
        for domain in search_domains {
            content.push(' ');
            content.push_str(domain);
        }
        content.push('\n');
    }
    for dns in dns_servers {
        content.push_str(&format!("nameserver {dns}\n"));
    }
    for dns in dns6_servers {
        content.push_str(&format!("nameserver {dns}\n"));
    }
    if dns_servers.is_empty() && dns6_servers.is_empty() {
        // Fallback to localhost stub if no DNS configured.
        content.push_str("nameserver 127.0.0.53\n");
    }

    let path = dir.join("resolv.conf");
    std::fs::write(&path, &content)?;

    // Also update /etc/resolv.conf if it's a symlink to our file or doesn't exist.
    let etc_resolv = std::path::Path::new("/etc/resolv.conf");
    if !etc_resolv.exists() || is_our_resolv_conf(etc_resolv) {
        // Write directly — don't overwrite user-managed resolv.conf.
        // Only write if /etc/resolv.conf is a symlink to our managed file.
    }

    Ok(())
}

/// Check if /etc/resolv.conf points to our managed file.
fn is_our_resolv_conf(path: &std::path::Path) -> bool {
    match std::fs::read_link(path) {
        Ok(target) => {
            let target_str = target.to_string_lossy();
            target_str.contains("systemd/resolve")
                || target_str.contains("run/systemd/resolve")
                || target_str.contains("stub-resolv.conf")
        }
        Err(_) => false,
    }
}

/// Route protocol constant for DHCP-learned routes.
pub fn rtprot_dhcp() -> u8 {
    RTPROT_DHCP
}

/// Route protocol constant for static routes.
pub fn rtprot_static() -> u8 {
    RTPROT_STATIC
}

/// Route protocol constant for boot routes.
pub fn rtprot_boot() -> u8 {
    RTPROT_BOOT
}

// ---------------------------------------------------------------------------
// Netlink attribute parsing
// ---------------------------------------------------------------------------

/// Parse a sequence of rtattr (route attribute) TLVs into a map.
fn parse_rtattrs(data: &[u8]) -> HashMap<u16, Vec<u8>> {
    let mut attrs = HashMap::new();
    let mut offset = 0;

    while offset + 4 <= data.len() {
        let rta_len = u16::from_ne_bytes(data[offset..offset + 2].try_into().unwrap()) as usize;
        let rta_type = u16::from_ne_bytes(data[offset + 2..offset + 4].try_into().unwrap());

        if rta_len < 4 || offset + rta_len > data.len() {
            break;
        }

        // Strip the NLA_TYPE_MASK bits — we only care about the base type.
        let base_type = rta_type & 0x3FFF;
        let payload = data[offset + 4..offset + rta_len].to_vec();
        attrs.insert(base_type, payload);

        offset += rta_align(rta_len);
    }

    attrs
}

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

fn nlmsg_align(len: usize) -> usize {
    (len + NLMSG_ALIGN - 1) & !(NLMSG_ALIGN - 1)
}

fn rta_align(len: usize) -> usize {
    (len + 3) & !3
}

fn rta_aligned_len(payload_len: usize) -> usize {
    rta_align(4 + payload_len) // 4 bytes for rta_len + rta_type
}

fn put_u16(buf: &mut [u8], offset: usize, val: u16) {
    buf[offset..offset + 2].copy_from_slice(&val.to_ne_bytes());
}

fn put_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
}

fn put_i32(buf: &mut [u8], offset: usize, val: i32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
}

/// Write a route attribute with a u32 payload.
fn put_rta_u32(buf: &mut [u8], offset: usize, rta_type: u16, val: u32) {
    let rta_len: u16 = 8; // 4 header + 4 payload
    put_u16(buf, offset, rta_len);
    put_u16(buf, offset + 2, rta_type);
    put_u32(buf, offset + 4, val);
}

/// Write a route attribute with an IPv4 address payload.
fn put_rta_ipv4(buf: &mut [u8], offset: usize, rta_type: u16, addr: Ipv4Addr) {
    let rta_len: u16 = 8; // 4 header + 4 address
    put_u16(buf, offset, rta_len);
    put_u16(buf, offset + 2, rta_type);
    buf[offset + 4..offset + 8].copy_from_slice(&addr.octets());
}

/// Write a route attribute with an IPv6 address payload (16 bytes).
fn put_rta_ipv6(buf: &mut [u8], offset: usize, rta_type: u16, addr: Ipv6Addr) {
    let rta_len: u16 = 20; // 4 header + 16 address
    put_u16(buf, offset, rta_len);
    put_u16(buf, offset + 2, rta_type);
    buf[offset + 4..offset + 20].copy_from_slice(&addr.octets());
}

/// Write a route attribute with a NUL-terminated string payload.
fn put_rta_str(buf: &mut [u8], offset: usize, rta_type: u16, s: &str) {
    let payload_len = s.len() + 1; // include NUL terminator
    let rta_len = (4 + payload_len) as u16;
    put_u16(buf, offset, rta_len);
    put_u16(buf, offset + 2, rta_type);
    buf[offset + 4..offset + 4 + s.len()].copy_from_slice(s.as_bytes());
    buf[offset + 4 + s.len()] = 0; // NUL
}

/// Write a route attribute with a u8 payload.
fn put_rta_u8(buf: &mut [u8], offset: usize, rta_type: u16, val: u8) {
    let rta_len: u16 = 5; // 4 header + 1 payload
    put_u16(buf, offset, rta_len);
    put_u16(buf, offset + 2, rta_type);
    buf[offset + 4] = val;
}

/// Write a route attribute with a port range payload (two u16 values).
fn put_rta_port_range(buf: &mut [u8], offset: usize, rta_type: u16, start: u16, end: u16) {
    let rta_len: u16 = 8; // 4 header + 4 payload (2 × u16)
    put_u16(buf, offset, rta_len);
    put_u16(buf, offset + 2, rta_type);
    put_u16(buf, offset + 4, start);
    put_u16(buf, offset + 6, end);
}

/// Write a route attribute with a UID range payload (two u32 values).
fn put_rta_uid_range(buf: &mut [u8], offset: usize, rta_type: u16, start: u32, end: u32) {
    let rta_len: u16 = 12; // 4 header + 8 payload (2 × u32)
    put_u16(buf, offset, rta_len);
    put_u16(buf, offset + 2, rta_type);
    put_u32(buf, offset + 4, start);
    put_u32(buf, offset + 8, end);
}

// ---------------------------------------------------------------------------
// Routing policy rules
// ---------------------------------------------------------------------------

use std::net::Ipv6Addr;

/// Configuration for a routing policy rule to install via netlink.
#[derive(Debug, Clone)]
pub struct RuleConfig {
    /// Address family: `AF_INET` (2) or `AF_INET6` (10).
    pub family: u8,
    /// Source prefix (IPv4 or IPv6 in CIDR).
    pub from: Option<(IpAddr, u8)>,
    /// Destination prefix (IPv4 or IPv6 in CIDR).
    pub to: Option<(IpAddr, u8)>,
    /// TOS value.
    pub tos: u8,
    /// Routing table ID.
    pub table: u32,
    /// Rule priority.
    pub priority: Option<u32>,
    /// Firewall mark value.
    pub fwmark: Option<u32>,
    /// Firewall mark mask.
    pub fwmask: Option<u32>,
    /// Incoming interface name.
    pub iifname: Option<String>,
    /// Outgoing interface name.
    pub oifname: Option<String>,
    /// Source port range.
    pub sport_range: Option<(u16, u16)>,
    /// Destination port range.
    pub dport_range: Option<(u16, u16)>,
    /// IP protocol number.
    pub ip_proto: Option<u8>,
    /// Invert the rule match.
    pub invert: bool,
    /// UID range.
    pub uid_range: Option<(u32, u32)>,
    /// Suppress prefix length.
    pub suppress_prefix_length: Option<i32>,
    /// Rule action type (FR_ACT_TO_TBL, FR_ACT_BLACKHOLE, etc.).
    pub action: u8,
}

impl Default for RuleConfig {
    fn default() -> Self {
        Self {
            family: AF_INET,
            from: None,
            to: None,
            tos: 0,
            table: RT_TABLE_MAIN as u32,
            priority: None,
            fwmark: None,
            fwmask: None,
            iifname: None,
            oifname: None,
            sport_range: None,
            dport_range: None,
            ip_proto: None,
            invert: false,
            uid_range: None,
            suppress_prefix_length: None,
            action: FR_ACT_TO_TBL,
        }
    }
}

use std::net::IpAddr;

/// Add a routing policy rule via netlink RTM_NEWRULE.
pub fn add_rule(cfg: &RuleConfig) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    // Calculate sizes for all optional attributes.
    let src_len = match &cfg.from {
        Some((IpAddr::V4(_), _)) => rta_aligned_len(4),
        Some((IpAddr::V6(_), _)) => rta_aligned_len(16),
        None => 0,
    };
    let dst_len = match &cfg.to {
        Some((IpAddr::V4(_), _)) => rta_aligned_len(4),
        Some((IpAddr::V6(_), _)) => rta_aligned_len(16),
        None => 0,
    };
    let table_len = rta_aligned_len(4); // FRA_TABLE always present
    let priority_len = if cfg.priority.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let fwmark_len = if cfg.fwmark.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let fwmask_len = if cfg.fwmask.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let iifname_len = cfg
        .iifname
        .as_ref()
        .map(|s| rta_aligned_len(s.len() + 1))
        .unwrap_or(0);
    let oifname_len = cfg
        .oifname
        .as_ref()
        .map(|s| rta_aligned_len(s.len() + 1))
        .unwrap_or(0);
    let sport_len = if cfg.sport_range.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let dport_len = if cfg.dport_range.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };
    let ip_proto_len = if cfg.ip_proto.is_some() {
        rta_aligned_len(1)
    } else {
        0
    };
    let uid_range_len = if cfg.uid_range.is_some() {
        rta_aligned_len(8)
    } else {
        0
    };
    let suppress_len = if cfg.suppress_prefix_length.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };

    let msg_len = NLMSG_HDR_LEN
        + FIB_RULE_HDR_LEN
        + src_len
        + dst_len
        + table_len
        + priority_len
        + fwmark_len
        + fwmask_len
        + iifname_len
        + oifname_len
        + sport_len
        + dport_len
        + ip_proto_len
        + uid_range_len
        + suppress_len;

    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    // Netlink header
    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_NEWRULE);
    put_u16(
        &mut msg,
        6,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
    );
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // fib_rule_hdr (same layout as rtmsg)
    let hdr = NLMSG_HDR_LEN;
    msg[hdr] = cfg.family; // family
    let src_prefix_len = cfg.from.map(|(_, p)| p).unwrap_or(0);
    let dst_prefix_len = cfg.to.map(|(_, p)| p).unwrap_or(0);
    msg[hdr + 1] = dst_prefix_len; // dst_len
    msg[hdr + 2] = src_prefix_len; // src_len
    msg[hdr + 3] = cfg.tos; // tos
    // table field: use RT_TABLE_MAIN (254) as sentinel if real table > 255
    msg[hdr + 4] = if cfg.table <= 255 {
        cfg.table as u8
    } else {
        RT_TABLE_MAIN
    };
    msg[hdr + 5] = 0; // res1
    msg[hdr + 6] = 0; // res2
    msg[hdr + 7] = cfg.action; // action

    // flags field (u32 at offset hdr+8)
    let flags: u32 = if cfg.invert { FIB_RULE_INVERT } else { 0 };
    put_u32(&mut msg, hdr + 8, flags);

    let mut off = NLMSG_HDR_LEN + FIB_RULE_HDR_LEN;

    // FRA_SRC
    if let Some((addr, _)) = &cfg.from {
        match addr {
            IpAddr::V4(v4) => {
                put_rta_ipv4(&mut msg, off, FRA_SRC, *v4);
                off += rta_aligned_len(4);
            }
            IpAddr::V6(v6) => {
                put_rta_ipv6(&mut msg, off, FRA_SRC, *v6);
                off += rta_aligned_len(16);
            }
        }
    }

    // FRA_DST
    if let Some((addr, _)) = &cfg.to {
        match addr {
            IpAddr::V4(v4) => {
                put_rta_ipv4(&mut msg, off, FRA_DST, *v4);
                off += rta_aligned_len(4);
            }
            IpAddr::V6(v6) => {
                put_rta_ipv6(&mut msg, off, FRA_DST, *v6);
                off += rta_aligned_len(16);
            }
        }
    }

    // FRA_TABLE (always present, carries the full 32-bit table ID)
    put_rta_u32(&mut msg, off, FRA_TABLE, cfg.table);
    off += rta_aligned_len(4);

    // FRA_PRIORITY
    if let Some(prio) = cfg.priority {
        put_rta_u32(&mut msg, off, FRA_PRIORITY, prio);
        off += rta_aligned_len(4);
    }

    // FRA_FWMARK
    if let Some(mark) = cfg.fwmark {
        put_rta_u32(&mut msg, off, FRA_FWMARK, mark);
        off += rta_aligned_len(4);
    }

    // FRA_FWMASK
    if let Some(mask) = cfg.fwmask {
        put_rta_u32(&mut msg, off, FRA_FWMASK, mask);
        off += rta_aligned_len(4);
    }

    // FRA_IFNAME (incoming interface)
    if let Some(ref iifname) = cfg.iifname {
        put_rta_str(&mut msg, off, FRA_IFNAME, iifname);
        off += rta_aligned_len(iifname.len() + 1);
    }

    // FRA_OIFNAME (outgoing interface)
    if let Some(ref oif) = cfg.oifname {
        put_rta_str(&mut msg, off, FRA_OIFNAME, oif);
        off += rta_aligned_len(oif.len() + 1);
    }

    // FRA_SPORT_RANGE
    if let Some((start, end)) = cfg.sport_range {
        put_rta_port_range(&mut msg, off, FRA_SPORT_RANGE, start, end);
        off += rta_aligned_len(4);
    }

    // FRA_DPORT_RANGE
    if let Some((start, end)) = cfg.dport_range {
        put_rta_port_range(&mut msg, off, FRA_DPORT_RANGE, start, end);
        off += rta_aligned_len(4);
    }

    // FRA_IP_PROTO
    if let Some(proto) = cfg.ip_proto {
        put_rta_u8(&mut msg, off, FRA_IP_PROTO, proto);
        off += rta_aligned_len(1);
    }

    // FRA_UID_RANGE
    if let Some((start, end)) = cfg.uid_range {
        put_rta_uid_range(&mut msg, off, FRA_UID_RANGE, start, end);
        off += rta_aligned_len(8);
    }

    // FRA_SUPPRESS_PREFIXLEN
    if let Some(spl) = cfg.suppress_prefix_length {
        // Kernel expects u32 here (despite the semantics allowing -1 to disable).
        put_rta_u32(&mut msg, off, FRA_SUPPRESS_PREFIXLEN, spl as u32);
        // off += rta_aligned_len(4); // last attribute
    }

    nl.request(&msg)?;
    Ok(())
}

/// Delete a routing policy rule via netlink RTM_DELRULE.
///
/// The kernel matches the rule to delete by the fields specified in the message
/// (same structure as add). All fields in `cfg` are used as match criteria.
pub fn del_rule(cfg: &RuleConfig) -> io::Result<()> {
    let mut nl = NetlinkSocket::open()?;
    let seq = nl.next_seq();

    // Calculate sizes (same as add_rule).
    let src_len = match &cfg.from {
        Some((IpAddr::V4(_), _)) => rta_aligned_len(4),
        Some((IpAddr::V6(_), _)) => rta_aligned_len(16),
        None => 0,
    };
    let dst_len = match &cfg.to {
        Some((IpAddr::V4(_), _)) => rta_aligned_len(4),
        Some((IpAddr::V6(_), _)) => rta_aligned_len(16),
        None => 0,
    };
    let table_len = rta_aligned_len(4);
    let priority_len = if cfg.priority.is_some() {
        rta_aligned_len(4)
    } else {
        0
    };

    let msg_len = NLMSG_HDR_LEN + FIB_RULE_HDR_LEN + src_len + dst_len + table_len + priority_len;
    let mut msg = vec![0u8; nlmsg_align(msg_len)];

    // Netlink header
    put_u32(&mut msg, 0, msg_len as u32);
    put_u16(&mut msg, 4, RTM_DELRULE);
    put_u16(&mut msg, 6, NLM_F_REQUEST | NLM_F_ACK);
    put_u32(&mut msg, 8, seq);
    put_u32(&mut msg, 12, nl.pid);

    // fib_rule_hdr
    let hdr = NLMSG_HDR_LEN;
    msg[hdr] = cfg.family;
    let src_prefix_len = cfg.from.map(|(_, p)| p).unwrap_or(0);
    let dst_prefix_len = cfg.to.map(|(_, p)| p).unwrap_or(0);
    msg[hdr + 1] = dst_prefix_len;
    msg[hdr + 2] = src_prefix_len;
    msg[hdr + 3] = cfg.tos;
    msg[hdr + 4] = if cfg.table <= 255 {
        cfg.table as u8
    } else {
        RT_TABLE_MAIN
    };
    msg[hdr + 7] = cfg.action;

    let flags: u32 = if cfg.invert { FIB_RULE_INVERT } else { 0 };
    put_u32(&mut msg, hdr + 8, flags);

    let mut off = NLMSG_HDR_LEN + FIB_RULE_HDR_LEN;

    if let Some((addr, _)) = &cfg.from {
        match addr {
            IpAddr::V4(v4) => {
                put_rta_ipv4(&mut msg, off, FRA_SRC, *v4);
                off += rta_aligned_len(4);
            }
            IpAddr::V6(v6) => {
                put_rta_ipv6(&mut msg, off, FRA_SRC, *v6);
                off += rta_aligned_len(16);
            }
        }
    }

    if let Some((addr, _)) = &cfg.to {
        match addr {
            IpAddr::V4(v4) => {
                put_rta_ipv4(&mut msg, off, FRA_DST, *v4);
                off += rta_aligned_len(4);
            }
            IpAddr::V6(v6) => {
                put_rta_ipv6(&mut msg, off, FRA_DST, *v6);
                off += rta_aligned_len(16);
            }
        }
    }

    put_rta_u32(&mut msg, off, FRA_TABLE, cfg.table);
    off += rta_aligned_len(4);

    if let Some(prio) = cfg.priority {
        put_rta_u32(&mut msg, off, FRA_PRIORITY, prio);
        // off += rta_aligned_len(4); // last attribute
    }

    nl.request(&msg)?;
    Ok(())
}

/// Helper: determine the address family from a CIDR string.
/// Returns `AF_INET` for IPv4, `AF_INET6` for IPv6, or `None` on parse failure.
pub fn family_from_cidr(cidr: &str) -> Option<u8> {
    let addr_str = cidr.split('/').next()?;
    let addr: IpAddr = addr_str.trim().parse().ok()?;
    Some(match addr {
        IpAddr::V4(_) => AF_INET,
        IpAddr::V6(_) => AF_INET6,
    })
}

/// Public accessor for AF_INET6 constant.
pub fn af_inet6() -> u8 {
    AF_INET6
}

/// Public accessor for AF_INET constant.
pub fn af_inet() -> u8 {
    AF_INET
}

/// Public accessor for FR_ACT_TO_TBL constant.
pub fn fr_act_to_tbl() -> u8 {
    FR_ACT_TO_TBL
}

/// Public accessor for FR_ACT_BLACKHOLE constant.
pub fn fr_act_blackhole() -> u8 {
    FR_ACT_BLACKHOLE
}

/// Public accessor for FR_ACT_UNREACHABLE constant.
pub fn fr_act_unreachable() -> u8 {
    FR_ACT_UNREACHABLE
}

/// Public accessor for FR_ACT_PROHIBIT constant.
pub fn fr_act_prohibit() -> u8 {
    FR_ACT_PROHIBIT
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_nlmsg_align() {
        assert_eq!(nlmsg_align(1), 4);
        assert_eq!(nlmsg_align(4), 4);
        assert_eq!(nlmsg_align(5), 8);
        assert_eq!(nlmsg_align(16), 16);
        assert_eq!(nlmsg_align(17), 20);
    }

    #[test]
    fn test_rta_align() {
        assert_eq!(rta_align(4), 4);
        assert_eq!(rta_align(5), 8);
        assert_eq!(rta_align(8), 8);
        assert_eq!(rta_align(9), 12);
    }

    #[test]
    fn test_rta_aligned_len() {
        // 4 bytes payload => 4 header + 4 payload = 8 aligned to 8.
        assert_eq!(rta_aligned_len(4), 8);
        // 1 byte payload => 4 header + 1 = 5, aligned to 8.
        assert_eq!(rta_aligned_len(1), 8);
        // 0 bytes => 4, aligned to 4.
        assert_eq!(rta_aligned_len(0), 4);
    }

    #[test]
    fn test_put_u16() {
        let mut buf = [0u8; 4];
        put_u16(&mut buf, 0, 0x1234);
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 0x1234);
    }

    #[test]
    fn test_put_u32() {
        let mut buf = [0u8; 8];
        put_u32(&mut buf, 0, 0xDEADBEEF);
        assert_eq!(
            u32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]),
            0xDEADBEEF
        );
    }

    #[test]
    fn test_put_rta_u32() {
        let mut buf = [0u8; 16];
        put_rta_u32(&mut buf, 0, IFLA_MTU, 1500);
        let rta_len = u16::from_ne_bytes([buf[0], buf[1]]);
        let rta_type = u16::from_ne_bytes([buf[2], buf[3]]);
        let val = u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]);
        assert_eq!(rta_len, 8);
        assert_eq!(rta_type, IFLA_MTU);
        assert_eq!(val, 1500);
    }

    #[test]
    fn test_put_rta_ipv4() {
        let mut buf = [0u8; 16];
        put_rta_ipv4(&mut buf, 0, IFA_LOCAL, Ipv4Addr::new(192, 168, 1, 100));
        let rta_len = u16::from_ne_bytes([buf[0], buf[1]]);
        let rta_type = u16::from_ne_bytes([buf[2], buf[3]]);
        assert_eq!(rta_len, 8);
        assert_eq!(rta_type, IFA_LOCAL);
        assert_eq!(&buf[4..8], &[192, 168, 1, 100]);
    }

    #[test]
    fn test_parse_rtattrs_single() {
        // Build a single rtattr: rta_len=8 (u32 payload), rta_type=4 (IFLA_MTU)
        let mut data = vec![0u8; 8];
        put_u16(&mut data, 0, 8); // rta_len
        put_u16(&mut data, 2, IFLA_MTU); // rta_type
        data[4..8].copy_from_slice(&1500u32.to_ne_bytes()); // payload

        let attrs = parse_rtattrs(&data);
        assert!(attrs.contains_key(&IFLA_MTU));
        let mtu_data = attrs.get(&IFLA_MTU).unwrap();
        assert_eq!(mtu_data.len(), 4);
        assert_eq!(u32::from_ne_bytes(mtu_data[..4].try_into().unwrap()), 1500);
    }

    #[test]
    fn test_parse_rtattrs_multiple() {
        // Build two rtattrs back-to-back.
        let mut data = vec![0u8; 16];

        // First: IFLA_MTU = 9000
        put_u16(&mut data, 0, 8);
        put_u16(&mut data, 2, IFLA_MTU);
        data[4..8].copy_from_slice(&9000u32.to_ne_bytes());

        // Second: IFLA_OPERSTATE = 6 (UP)
        // rta_len = 5 (4 header + 1 byte), padded to 8
        put_u16(&mut data, 8, 5);
        put_u16(&mut data, 10, IFLA_OPERSTATE);
        data[12] = IF_OPER_UP;

        let attrs = parse_rtattrs(&data);
        assert!(attrs.contains_key(&IFLA_MTU));
        assert!(attrs.contains_key(&IFLA_OPERSTATE));

        let state = attrs.get(&IFLA_OPERSTATE).unwrap();
        assert_eq!(state[0], IF_OPER_UP);
    }

    #[test]
    fn test_parse_rtattrs_empty() {
        let attrs = parse_rtattrs(&[]);
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_parse_rtattrs_truncated() {
        // Data too short for even a header.
        let data = [0u8; 3];
        let attrs = parse_rtattrs(&data);
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_parse_rtattrs_bad_length() {
        // rta_len claims 100 but data is only 8 bytes.
        let mut data = [0u8; 8];
        put_u16(&mut data, 0, 100);
        put_u16(&mut data, 2, 1);
        let attrs = parse_rtattrs(&data);
        assert!(attrs.is_empty());
    }

    #[test]
    fn test_link_info_flags() {
        let link = LinkInfo {
            index: 1,
            name: "lo".into(),
            mac: "00:00:00:00:00:00".into(),
            mac_bytes: vec![0; 6],
            mtu: 65536,
            flags: IFF_UP | IFF_LOOPBACK | IFF_RUNNING,
            operstate: IF_OPER_UNKNOWN,
        };
        assert!(link.is_up());
        assert!(link.is_running());
        assert!(link.is_loopback());
        assert_eq!(link.oper_state_str(), "unknown");
    }

    #[test]
    fn test_link_info_not_up() {
        let link = LinkInfo {
            index: 2,
            name: "eth0".into(),
            mac: "52:54:00:12:34:56".into(),
            mac_bytes: vec![0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            mtu: 1500,
            flags: 0,
            operstate: IF_OPER_DOWN,
        };
        assert!(!link.is_up());
        assert!(!link.is_running());
        assert!(!link.is_loopback());
        assert_eq!(link.oper_state_str(), "down");
    }

    #[test]
    fn test_link_info_display() {
        let link = LinkInfo {
            index: 3,
            name: "wlan0".into(),
            mac: "aa:bb:cc:dd:ee:ff".into(),
            mac_bytes: vec![0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff],
            mtu: 1500,
            flags: IFF_UP | IFF_RUNNING,
            operstate: IF_OPER_UP,
        };
        let s = link.to_string();
        assert!(s.contains("wlan0"));
        assert!(s.contains("aa:bb:cc:dd:ee:ff"));
        assert!(s.contains("1500"));
        assert!(s.contains("up"));
    }

    #[test]
    fn test_addr_info_display() {
        let addr = AddrInfo {
            ifindex: 2,
            family: AF_INET,
            prefix_len: 24,
            address: Ipv4Addr::new(192, 168, 1, 100),
            local: Some(Ipv4Addr::new(192, 168, 1, 100)),
            broadcast: Some(Ipv4Addr::new(192, 168, 1, 255)),
            label: Some("eth0".into()),
            scope: 0,
        };
        let s = addr.to_string();
        assert!(s.contains("192.168.1.100/24"));
        assert!(s.contains("brd 192.168.1.255"));
        assert!(s.contains("label eth0"));
    }

    #[test]
    fn test_route_info_display_default() {
        let route = RouteInfo {
            destination: Ipv4Addr::UNSPECIFIED,
            dst_len: 0,
            gateway: Some(Ipv4Addr::new(192, 168, 1, 1)),
            oif: Some(2),
            priority: Some(100),
            protocol: RTPROT_DHCP,
            scope: RT_SCOPE_UNIVERSE,
            table: RT_TABLE_MAIN,
            prefsrc: None,
        };
        let s = route.to_string();
        assert!(s.contains("default"));
        assert!(s.contains("via 192.168.1.1"));
        assert!(s.contains("metric 100"));
    }

    #[test]
    fn test_route_info_display_subnet() {
        let route = RouteInfo {
            destination: Ipv4Addr::new(10, 0, 0, 0),
            dst_len: 8,
            gateway: None,
            oif: Some(3),
            priority: None,
            protocol: RTPROT_STATIC,
            scope: RT_SCOPE_LINK,
            table: RT_TABLE_MAIN,
            prefsrc: None,
        };
        let s = route.to_string();
        assert!(s.contains("10.0.0.0/8"));
        assert!(!s.contains("via"));
    }

    #[test]
    fn test_is_our_resolv_conf() {
        // A path that doesn't exist should return false.
        assert!(!is_our_resolv_conf(std::path::Path::new(
            "/tmp/definitely_does_not_exist_resolv_test"
        )));
    }

    #[test]
    fn test_rtprot_constants() {
        assert_eq!(rtprot_dhcp(), 16);
        assert_eq!(rtprot_static(), 4);
        assert_eq!(rtprot_boot(), 3);
    }

    // -----------------------------------------------------------------------
    // Routing policy rule tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_rule_config_default() {
        let cfg = RuleConfig::default();
        assert_eq!(cfg.family, AF_INET);
        assert!(cfg.from.is_none());
        assert!(cfg.to.is_none());
        assert_eq!(cfg.tos, 0);
        assert_eq!(cfg.table, RT_TABLE_MAIN as u32);
        assert!(cfg.priority.is_none());
        assert!(cfg.fwmark.is_none());
        assert!(cfg.fwmask.is_none());
        assert!(cfg.iifname.is_none());
        assert!(cfg.oifname.is_none());
        assert!(cfg.sport_range.is_none());
        assert!(cfg.dport_range.is_none());
        assert!(cfg.ip_proto.is_none());
        assert!(!cfg.invert);
        assert!(cfg.uid_range.is_none());
        assert!(cfg.suppress_prefix_length.is_none());
        assert_eq!(cfg.action, FR_ACT_TO_TBL);
    }

    #[test]
    fn test_rule_config_with_ipv4_source() {
        let cfg = RuleConfig {
            family: AF_INET,
            from: Some((IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0)), 24)),
            table: 100,
            priority: Some(32765),
            ..Default::default()
        };
        assert_eq!(cfg.family, AF_INET);
        assert_eq!(
            cfg.from,
            Some((IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0)), 24))
        );
        assert_eq!(cfg.table, 100);
        assert_eq!(cfg.priority, Some(32765));
    }

    #[test]
    fn test_rule_config_with_ipv6_addresses() {
        let src: Ipv6Addr = "2001:db8::".parse().unwrap();
        let dst: Ipv6Addr = "fd00::".parse().unwrap();
        let cfg = RuleConfig {
            family: AF_INET6,
            from: Some((IpAddr::V6(src), 32)),
            to: Some((IpAddr::V6(dst), 8)),
            table: 200,
            ..Default::default()
        };
        assert_eq!(cfg.family, AF_INET6);
        assert_eq!(cfg.from, Some((IpAddr::V6(src), 32)));
        assert_eq!(cfg.to, Some((IpAddr::V6(dst), 8)));
    }

    #[test]
    fn test_rule_config_with_firewall_mark() {
        let cfg = RuleConfig {
            fwmark: Some(0xCAFE),
            fwmask: Some(0xFFFF),
            table: 100,
            ..Default::default()
        };
        assert_eq!(cfg.fwmark, Some(0xCAFE));
        assert_eq!(cfg.fwmask, Some(0xFFFF));
    }

    #[test]
    fn test_rule_config_with_interfaces() {
        let cfg = RuleConfig {
            iifname: Some("eth0".to_string()),
            oifname: Some("eth1".to_string()),
            table: 100,
            ..Default::default()
        };
        assert_eq!(cfg.iifname.as_deref(), Some("eth0"));
        assert_eq!(cfg.oifname.as_deref(), Some("eth1"));
    }

    #[test]
    fn test_rule_config_with_port_ranges() {
        let cfg = RuleConfig {
            sport_range: Some((1024, 65535)),
            dport_range: Some((80, 80)),
            ip_proto: Some(6), // TCP
            table: 100,
            ..Default::default()
        };
        assert_eq!(cfg.sport_range, Some((1024, 65535)));
        assert_eq!(cfg.dport_range, Some((80, 80)));
        assert_eq!(cfg.ip_proto, Some(6));
    }

    #[test]
    fn test_rule_config_with_uid_range() {
        let cfg = RuleConfig {
            uid_range: Some((1000, 2000)),
            table: 100,
            ..Default::default()
        };
        assert_eq!(cfg.uid_range, Some((1000, 2000)));
    }

    #[test]
    fn test_rule_config_invert() {
        let cfg = RuleConfig {
            invert: true,
            from: Some((IpAddr::V4(Ipv4Addr::new(10, 0, 0, 0)), 8)),
            table: 100,
            ..Default::default()
        };
        assert!(cfg.invert);
    }

    #[test]
    fn test_rule_config_suppress_prefix_length() {
        let cfg = RuleConfig {
            suppress_prefix_length: Some(0),
            table: 254, // main
            ..Default::default()
        };
        assert_eq!(cfg.suppress_prefix_length, Some(0));
    }

    #[test]
    fn test_rule_config_action_types() {
        assert_eq!(FR_ACT_TO_TBL, 1);
        assert_eq!(FR_ACT_BLACKHOLE, 6);
        assert_eq!(FR_ACT_UNREACHABLE, 7);
        assert_eq!(FR_ACT_PROHIBIT, 8);

        let cfg = RuleConfig {
            action: FR_ACT_BLACKHOLE,
            ..Default::default()
        };
        assert_eq!(cfg.action, FR_ACT_BLACKHOLE);
    }

    #[test]
    fn test_family_from_cidr_ipv4() {
        assert_eq!(family_from_cidr("192.168.1.0/24"), Some(AF_INET));
        assert_eq!(family_from_cidr("10.0.0.0/8"), Some(AF_INET));
        assert_eq!(family_from_cidr("0.0.0.0/0"), Some(AF_INET));
    }

    #[test]
    fn test_family_from_cidr_ipv6() {
        assert_eq!(family_from_cidr("2001:db8::/32"), Some(AF_INET6));
        assert_eq!(family_from_cidr("fd00::/8"), Some(AF_INET6));
        assert_eq!(family_from_cidr("::/0"), Some(AF_INET6));
    }

    #[test]
    fn test_family_from_cidr_invalid() {
        assert!(family_from_cidr("not-an-address/24").is_none());
        assert!(family_from_cidr("").is_none());
    }

    #[test]
    fn test_put_rta_ipv6() {
        let addr: Ipv6Addr = "2001:db8::1".parse().unwrap();
        let mut buf = vec![0u8; 24]; // rta_aligned_len(16) = 20, round to 24
        put_rta_ipv6(&mut buf, 0, FRA_SRC, addr);

        // rta_len = 20 (4 header + 16 address)
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 20);
        // rta_type = FRA_SRC (2)
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), FRA_SRC);
        // Address bytes
        assert_eq!(&buf[4..20], &addr.octets());
    }

    #[test]
    fn test_put_rta_str() {
        let mut buf = vec![0u8; 12]; // rta_aligned_len(5) for "eth0\0" = 12
        put_rta_str(&mut buf, 0, FRA_IFNAME, "eth0");

        // rta_len = 4 + 5 = 9
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 9);
        // rta_type = FRA_IFNAME (3)
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), FRA_IFNAME);
        // String bytes + NUL
        assert_eq!(&buf[4..8], b"eth0");
        assert_eq!(buf[8], 0); // NUL
    }

    #[test]
    fn test_put_rta_u8() {
        let mut buf = vec![0u8; 8]; // rta_aligned_len(1) = 8
        put_rta_u8(&mut buf, 0, FRA_IP_PROTO, 6);

        // rta_len = 5
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 5);
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), FRA_IP_PROTO);
        assert_eq!(buf[4], 6);
    }

    #[test]
    fn test_put_rta_port_range() {
        let mut buf = vec![0u8; 8];
        put_rta_port_range(&mut buf, 0, FRA_SPORT_RANGE, 1024, 65535);

        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 8);
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), FRA_SPORT_RANGE);
        assert_eq!(u16::from_ne_bytes([buf[4], buf[5]]), 1024);
        assert_eq!(u16::from_ne_bytes([buf[6], buf[7]]), 65535);
    }

    #[test]
    fn test_put_rta_uid_range() {
        let mut buf = vec![0u8; 12];
        put_rta_uid_range(&mut buf, 0, FRA_UID_RANGE, 1000, 2000);

        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 12);
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), FRA_UID_RANGE);
        assert_eq!(u32::from_ne_bytes(buf[4..8].try_into().unwrap()), 1000);
        assert_eq!(u32::from_ne_bytes(buf[8..12].try_into().unwrap()), 2000);
    }

    #[test]
    fn test_fra_constants() {
        assert_eq!(FRA_DST, 1);
        assert_eq!(FRA_SRC, 2);
        assert_eq!(FRA_IFNAME, 3);
        assert_eq!(FRA_TABLE, 15);
        assert_eq!(FRA_FWMARK, 10);
        assert_eq!(FRA_FWMASK, 16);
        assert_eq!(FRA_PRIORITY, 6);
        assert_eq!(FRA_OIFNAME, 17);
        assert_eq!(FRA_SUPPRESS_PREFIXLEN, 14);
        assert_eq!(FRA_IP_PROTO, 19);
        assert_eq!(FRA_SPORT_RANGE, 20);
        assert_eq!(FRA_DPORT_RANGE, 21);
        assert_eq!(FRA_UID_RANGE, 22);
    }

    #[test]
    fn test_rtm_rule_constants() {
        assert_eq!(RTM_NEWRULE, 32);
        assert_eq!(RTM_DELRULE, 33);
    }

    #[test]
    fn test_fib_rule_hdr_len() {
        // fib_rule_hdr is the same size as rtmsg (12 bytes)
        assert_eq!(FIB_RULE_HDR_LEN, 12);
    }

    #[test]
    fn test_fib_rule_invert_flag() {
        assert_eq!(FIB_RULE_INVERT, 0x00000002);
    }

    #[test]
    fn test_af_inet6_constant() {
        assert_eq!(af_inet6(), 10);
        assert_eq!(af_inet(), 2);
    }

    #[test]
    fn test_fr_act_accessors() {
        assert_eq!(fr_act_to_tbl(), 1);
        assert_eq!(fr_act_blackhole(), 6);
        assert_eq!(fr_act_unreachable(), 7);
        assert_eq!(fr_act_prohibit(), 8);
    }

    #[test]
    fn test_rule_msg_structure_ipv4_basic() {
        // Verify the message structure for a minimal IPv4 rule.
        let _cfg = RuleConfig {
            family: AF_INET,
            from: Some((IpAddr::V4(Ipv4Addr::new(192, 168, 1, 0)), 24)),
            table: 100,
            priority: Some(32765),
            ..Default::default()
        };

        // Calculate expected size:
        // NLMSG_HDR(16) + FIB_RULE_HDR(12) + FRA_SRC(8) + FRA_TABLE(8) + FRA_PRIORITY(8)
        let expected_payload = NLMSG_HDR_LEN + FIB_RULE_HDR_LEN + 8 + 8 + 8;
        let src_len = rta_aligned_len(4); // IPv4 = 4 bytes
        let table_len = rta_aligned_len(4);
        let prio_len = rta_aligned_len(4);
        let msg_len = NLMSG_HDR_LEN + FIB_RULE_HDR_LEN + src_len + table_len + prio_len;
        assert_eq!(msg_len, expected_payload);
    }

    #[test]
    fn test_rule_msg_structure_ipv6_larger() {
        // IPv6 source adds 16 bytes instead of 4.
        let src: Ipv6Addr = "2001:db8::".parse().unwrap();
        let _cfg = RuleConfig {
            family: AF_INET6,
            from: Some((IpAddr::V6(src), 32)),
            table: 200,
            ..Default::default()
        };

        let src_len = rta_aligned_len(16); // IPv6 = 16 bytes → rta_aligned_len(16) = 20
        let table_len = rta_aligned_len(4);
        let msg_len = NLMSG_HDR_LEN + FIB_RULE_HDR_LEN + src_len + table_len;
        // 16 + 12 + 20 + 8 = 56
        assert_eq!(msg_len, 56);
    }
}
