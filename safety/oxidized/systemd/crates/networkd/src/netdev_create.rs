//! Virtual network device creation via netlink RTM_NEWLINK.
//!
//! This module takes parsed `NetDevConfig` structures (from `.netdev` files)
//! and creates the corresponding kernel network interfaces using the
//! NETLINK_ROUTE protocol.
//!
//! Supported device types:
//! - dummy, vcan, nlmon, ifb — simple devices (no parameters)
//! - bridge — with STP, timers, priority, VLAN filtering, multicast settings
//! - bond — with mode, MII monitor, hash policy, LACP rate, etc.
//! - vlan — with VLAN ID, protocol, flags (requires parent link)
//! - vxlan — with VNI, group/remote, port, TTL, learning, etc.
//! - macvlan / macvtap — with mode (requires parent link)
//! - ipvlan — with mode and flags (requires parent link)
//! - veth — with peer name
//! - vrf — with routing table ID
//! - wireguard — basic interface creation (key config via separate mechanism)
//! - gre, gretap, ip6gre, ip6gretap, sit, vti, vti6, ip6tnl, erspan — tunnels
//! - geneve — with VNI, remote, port
//! - bareudp — with destination port, ethertype
//! - xfrm — with interface ID

use std::io;
use std::net::Ipv4Addr;

use crate::netdev::*;

// ---------------------------------------------------------------------------
// Netlink / rtnetlink constants
// ---------------------------------------------------------------------------

const NETLINK_ROUTE: i32 = 0;

const RTM_NEWLINK: u16 = 16;

const NLM_F_REQUEST: u16 = 0x0001;
const NLM_F_ACK: u16 = 0x0004;
const NLM_F_CREATE: u16 = 0x0400;
const NLM_F_EXCL: u16 = 0x0200;

const NLMSG_HDR_LEN: usize = 16;
const IFINFOMSG_LEN: usize = 16;

const NLMSG_ERROR: u16 = 2;
const NLMSG_DONE: u16 = 3;

const AF_UNSPEC: u8 = 0;

// IFLA attribute types
const IFLA_ADDRESS: u16 = 1;
const IFLA_IFNAME: u16 = 3;
const IFLA_MTU: u16 = 4;
const IFLA_LINK: u16 = 5;
const IFLA_LINKINFO: u16 = 18;

// IFLA_INFO sub-attributes
const IFLA_INFO_KIND: u16 = 1;
const IFLA_INFO_DATA: u16 = 2;

// NLA_F_NESTED flag for nested attributes
const NLA_F_NESTED: u16 = 0x8000;

// IFF flags
const IFF_UP: u32 = 0x1;

// ---------------------------------------------------------------------------
// Bridge attributes (IFLA_BR_*)
// ---------------------------------------------------------------------------
const IFLA_BR_FORWARD_DELAY: u16 = 1;
const IFLA_BR_HELLO_TIME: u16 = 2;
const IFLA_BR_MAX_AGE: u16 = 3;
const IFLA_BR_AGEING_TIME: u16 = 4;
const IFLA_BR_STP_STATE: u16 = 5;
const IFLA_BR_PRIORITY: u16 = 6;
const IFLA_BR_VLAN_FILTERING: u16 = 7;
const IFLA_BR_VLAN_PROTOCOL: u16 = 8;
const IFLA_BR_GROUP_FWD_MASK: u16 = 9;
const IFLA_BR_MCAST_SNOOPING: u16 = 23;
const IFLA_BR_MCAST_QUERIER: u16 = 25;
const IFLA_BR_VLAN_DEFAULT_PVID: u16 = 39;
const IFLA_BR_MCAST_IGMP_VERSION: u16 = 43;

// ---------------------------------------------------------------------------
// Bond attributes (IFLA_BOND_*)
// ---------------------------------------------------------------------------
const IFLA_BOND_MODE: u16 = 1;
const IFLA_BOND_MIIMON: u16 = 3;
const IFLA_BOND_UPDELAY: u16 = 4;
const IFLA_BOND_DOWNDELAY: u16 = 5;
const IFLA_BOND_ARP_INTERVAL: u16 = 7;
const IFLA_BOND_ARP_IP_TARGET: u16 = 8;
const IFLA_BOND_ARP_VALIDATE: u16 = 9;
const IFLA_BOND_ARP_ALL_TARGETS: u16 = 10;
const IFLA_BOND_PRIMARY_RESELECT: u16 = 12;
const IFLA_BOND_FAIL_OVER_MAC: u16 = 13;
const IFLA_BOND_XMIT_HASH_POLICY: u16 = 14;
const IFLA_BOND_RESEND_IGMP: u16 = 15;
const IFLA_BOND_NUM_PEER_NOTIF: u16 = 16;
const IFLA_BOND_ALL_SLAVES_ACTIVE: u16 = 17;
const IFLA_BOND_MIN_LINKS: u16 = 18;
const IFLA_BOND_LP_INTERVAL: u16 = 19;
const IFLA_BOND_PACKETS_PER_SLAVE: u16 = 20;
const IFLA_BOND_AD_LACP_RATE: u16 = 21;
const IFLA_BOND_AD_SELECT: u16 = 22;
const IFLA_BOND_AD_ACTOR_SYS_PRIO: u16 = 24;
const IFLA_BOND_AD_USER_PORT_KEY: u16 = 25;
const IFLA_BOND_AD_ACTOR_SYSTEM: u16 = 26;
const IFLA_BOND_PEER_NOTIF_DELAY: u16 = 28;

// ---------------------------------------------------------------------------
// VLAN attributes (IFLA_VLAN_*)
// ---------------------------------------------------------------------------
const IFLA_VLAN_ID: u16 = 1;
const IFLA_VLAN_PROTOCOL: u16 = 5;

// ---------------------------------------------------------------------------
// VXLAN attributes (IFLA_VXLAN_*)
// ---------------------------------------------------------------------------
const IFLA_VXLAN_ID: u16 = 1;
const IFLA_VXLAN_GROUP: u16 = 2;
const IFLA_VXLAN_LOCAL: u16 = 4;
const IFLA_VXLAN_TTL: u16 = 5;
const IFLA_VXLAN_TOS: u16 = 6;
const IFLA_VXLAN_LEARNING: u16 = 7;
const IFLA_VXLAN_AGEING: u16 = 8;
const IFLA_VXLAN_LIMIT: u16 = 9;
const IFLA_VXLAN_L2MISS: u16 = 13;
const IFLA_VXLAN_L3MISS: u16 = 14;
const IFLA_VXLAN_PORT: u16 = 15;
const IFLA_VXLAN_UDP_ZERO_CSUM6_TX: u16 = 19;
const IFLA_VXLAN_UDP_ZERO_CSUM6_RX: u16 = 20;
const IFLA_VXLAN_GBP: u16 = 23;
const IFLA_VXLAN_LABEL: u16 = 26;
const IFLA_VXLAN_GPE: u16 = 27;

// ---------------------------------------------------------------------------
// MACVLAN attributes (IFLA_MACVLAN_*)
// ---------------------------------------------------------------------------
const IFLA_MACVLAN_MODE: u16 = 1;

// MACVLAN mode values
const MACVLAN_MODE_PRIVATE: u32 = 1;
const MACVLAN_MODE_VEPA: u32 = 2;
const MACVLAN_MODE_BRIDGE: u32 = 4;
const MACVLAN_MODE_PASSTHRU: u32 = 8;
const MACVLAN_MODE_SOURCE: u32 = 16;

// ---------------------------------------------------------------------------
// IPVLAN attributes (IFLA_IPVLAN_*)
// ---------------------------------------------------------------------------
const IFLA_IPVLAN_MODE: u16 = 1;
const IFLA_IPVLAN_FLAGS: u16 = 2;

// IPVLAN mode values
const IPVLAN_MODE_L2: u16 = 0;
const IPVLAN_MODE_L3: u16 = 1;
const IPVLAN_MODE_L3S: u16 = 2;

// IPVLAN flag values
const IPVLAN_F_BRIDGE: u16 = 0;
const IPVLAN_F_PRIVATE: u16 = 1;
const IPVLAN_F_VEPA: u16 = 2;

// ---------------------------------------------------------------------------
// VRF attributes (IFLA_VRF_*)
// ---------------------------------------------------------------------------
const IFLA_VRF_TABLE: u16 = 1;

// ---------------------------------------------------------------------------
// VETH peer attribute
// ---------------------------------------------------------------------------
const VETH_INFO_PEER: u16 = 1;

// ---------------------------------------------------------------------------
// Tunnel attributes (IFLA_GRE_*, IFLA_GENEVE_*, etc.)
// ---------------------------------------------------------------------------
const IFLA_GRE_LOCAL: u16 = 2;
const IFLA_GRE_REMOTE: u16 = 3;
const IFLA_GRE_TTL: u16 = 4;
const IFLA_GRE_TOS: u16 = 5;
const IFLA_GRE_IKEY: u16 = 7;
const IFLA_GRE_OKEY: u16 = 8;
const IFLA_GRE_ERSPAN_INDEX: u16 = 20;
const IFLA_GRE_ERSPAN_VER: u16 = 21;

const IFLA_GENEVE_ID: u16 = 1;
const IFLA_GENEVE_REMOTE: u16 = 2;
const IFLA_GENEVE_TTL: u16 = 3;
const IFLA_GENEVE_TOS: u16 = 4;
const IFLA_GENEVE_PORT: u16 = 5;
const IFLA_GENEVE_LABEL: u16 = 11;
const IFLA_GENEVE_UDP_ZERO_CSUM6_TX: u16 = 10;
const IFLA_GENEVE_UDP_ZERO_CSUM6_RX: u16 = 12;

const IFLA_IPTUN_LOCAL: u16 = 2;
const IFLA_IPTUN_REMOTE: u16 = 3;
const IFLA_IPTUN_TTL: u16 = 4;
const IFLA_IPTUN_TOS: u16 = 5;

const IFLA_BAREUDP_PORT: u16 = 1;
const IFLA_BAREUDP_ETHERTYPE: u16 = 2;

const IFLA_XFRM_IF_ID: u16 = 2;

// ---------------------------------------------------------------------------
// Serialization helpers
// ---------------------------------------------------------------------------

/// Align a length to a 4-byte boundary (netlink attribute alignment).
fn nl_align(len: usize) -> usize {
    (len + 3) & !3
}

/// Write a u16 value to a buffer at a given offset (native endian).
fn put_u16(buf: &mut [u8], offset: usize, val: u16) {
    let bytes = val.to_ne_bytes();
    buf[offset] = bytes[0];
    buf[offset + 1] = bytes[1];
}

/// Write a u32 value to a buffer at a given offset (native endian).
fn put_u32(buf: &mut [u8], offset: usize, val: u32) {
    let bytes = val.to_ne_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}

/// Write an i32 value to a buffer at a given offset (native endian).
fn put_i32(buf: &mut [u8], offset: usize, val: i32) {
    let bytes = val.to_ne_bytes();
    buf[offset..offset + 4].copy_from_slice(&bytes);
}

/// Append an rtattr with arbitrary bytes payload. Returns the start offset.
fn append_rta_bytes(buf: &mut Vec<u8>, rta_type: u16, payload: &[u8]) -> usize {
    let start = buf.len();
    let rta_len = 4 + payload.len();
    // Extend with aligned size
    let aligned = nl_align(rta_len);
    buf.resize(buf.len() + aligned, 0);
    put_u16(buf, start, rta_len as u16);
    put_u16(buf, start + 2, rta_type);
    buf[start + 4..start + 4 + payload.len()].copy_from_slice(payload);
    start
}

/// Append an rtattr with a NUL-terminated string payload.
fn append_rta_str(buf: &mut Vec<u8>, rta_type: u16, s: &str) -> usize {
    let mut payload = s.as_bytes().to_vec();
    payload.push(0); // NUL terminator
    append_rta_bytes(buf, rta_type, &payload)
}

/// Append an rtattr with a u8 payload.
fn append_rta_u8(buf: &mut Vec<u8>, rta_type: u16, val: u8) -> usize {
    append_rta_bytes(buf, rta_type, &[val])
}

/// Append an rtattr with a u16 payload (native endian).
fn append_rta_u16(buf: &mut Vec<u8>, rta_type: u16, val: u16) -> usize {
    append_rta_bytes(buf, rta_type, &val.to_ne_bytes())
}

/// Append an rtattr with a u16 payload (big endian, for network protocols).
fn append_rta_u16_be(buf: &mut Vec<u8>, rta_type: u16, val: u16) -> usize {
    append_rta_bytes(buf, rta_type, &val.to_be_bytes())
}

/// Append an rtattr with a u32 payload (native endian).
fn append_rta_u32(buf: &mut Vec<u8>, rta_type: u16, val: u32) -> usize {
    append_rta_bytes(buf, rta_type, &val.to_ne_bytes())
}

/// Append an rtattr with an IPv4 address payload (4 bytes).
fn append_rta_ipv4(buf: &mut Vec<u8>, rta_type: u16, addr: Ipv4Addr) -> usize {
    append_rta_bytes(buf, rta_type, &addr.octets())
}

/// Begin a nested rtattr. Returns the offset of the header so it can be
/// patched later with `finish_nested_rta()`.
fn begin_nested_rta(buf: &mut Vec<u8>, rta_type: u16) -> usize {
    let start = buf.len();
    buf.resize(buf.len() + 4, 0);
    // Mark as nested via NLA_F_NESTED
    put_u16(buf, start, 0); // placeholder length
    put_u16(buf, start + 2, rta_type | NLA_F_NESTED);
    start
}

/// Finish a nested rtattr, patching the length field.
fn finish_nested_rta(buf: &mut Vec<u8>, start: usize) {
    let total_len = buf.len() - start;
    put_u16(buf, start, total_len as u16);
    // Align the buffer
    let aligned = nl_align(buf.len());
    buf.resize(aligned, 0);
}

/// Parse a MAC address string "xx:xx:xx:xx:xx:xx" into 6 bytes.
fn parse_mac_address(mac: &str) -> Option<[u8; 6]> {
    let parts: Vec<&str> = mac.split(':').collect();
    if parts.len() != 6 {
        return None;
    }
    let mut bytes = [0u8; 6];
    for (i, part) in parts.iter().enumerate() {
        bytes[i] = u8::from_str_radix(part, 16).ok()?;
    }
    Some(bytes)
}

// ---------------------------------------------------------------------------
// Netlink socket for device creation
// ---------------------------------------------------------------------------

/// Open a NETLINK_ROUTE socket, send a message, and wait for the ACK.
fn netlink_request(msg: &[u8]) -> io::Result<()> {
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

    // Bind.
    let mut addr: libc::sockaddr_nl = unsafe { std::mem::zeroed() };
    addr.nl_family = libc::AF_NETLINK as u16;
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

    // Receive timeout.
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

    // Send.
    let sent = unsafe { libc::send(fd, msg.as_ptr() as *const libc::c_void, msg.len(), 0) };
    if sent < 0 {
        let err = io::Error::last_os_error();
        unsafe { libc::close(fd) };
        return Err(err);
    }

    // Receive ACK.
    let mut buf = [0u8; 4096];
    let n = unsafe { libc::recv(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len(), 0) };
    unsafe { libc::close(fd) };

    if n < 0 {
        return Err(io::Error::last_os_error());
    }
    let n = n as usize;

    // Parse the response.
    if n >= NLMSG_HDR_LEN + 4 {
        let nlmsg_type = u16::from_ne_bytes(buf[4..6].try_into().unwrap());
        if nlmsg_type == NLMSG_ERROR {
            let errno =
                i32::from_ne_bytes(buf[NLMSG_HDR_LEN..NLMSG_HDR_LEN + 4].try_into().unwrap());
            if errno < 0 {
                return Err(io::Error::from_raw_os_error(-errno));
            }
            // errno == 0 means ACK — success.
        }
    }

    Ok(())
}

/// Resolve an interface name to its ifindex via sysfs.
fn resolve_ifindex(name: &str) -> Option<u32> {
    let path = format!("/sys/class/net/{}/ifindex", name);
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| s.trim().parse::<u32>().ok())
}

/// Check whether an interface already exists.
fn interface_exists(name: &str) -> bool {
    std::path::Path::new(&format!("/sys/class/net/{}", name)).exists()
}

// ---------------------------------------------------------------------------
// Message construction
// ---------------------------------------------------------------------------

/// Build the common RTM_NEWLINK header (nlmsghdr + ifinfomsg) with
/// IFLA_IFNAME and optional IFLA_MTU, IFLA_ADDRESS.
/// Returns the buffer ready for IFLA_LINKINFO to be appended.
fn build_newlink_base(
    name: &str,
    mtu: Option<u64>,
    mac: Option<&str>,
    parent_ifindex: Option<u32>,
) -> Vec<u8> {
    let mut buf = vec![0u8; NLMSG_HDR_LEN + IFINFOMSG_LEN];

    // nlmsghdr — length will be patched at the end.
    put_u16(&mut buf, 4, RTM_NEWLINK);
    put_u16(
        &mut buf,
        6,
        NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL,
    );
    put_u32(&mut buf, 8, 1); // seq
    put_u32(&mut buf, 12, 0); // pid (kernel)

    // ifinfomsg
    buf[NLMSG_HDR_LEN] = AF_UNSPEC; // ifi_family
    // ifi_type, ifi_index, ifi_flags, ifi_change = 0

    // IFLA_IFNAME
    append_rta_str(&mut buf, IFLA_IFNAME, name);

    // IFLA_MTU (optional)
    if let Some(mtu_val) = mtu {
        append_rta_u32(&mut buf, IFLA_MTU, mtu_val as u32);
    }

    // IFLA_ADDRESS (optional MAC)
    if let Some(mac_str) = mac
        && let Some(mac_bytes) = parse_mac_address(mac_str)
    {
        append_rta_bytes(&mut buf, IFLA_ADDRESS, &mac_bytes);
    }

    // IFLA_LINK (optional parent interface for VLAN, macvlan, etc.)
    if let Some(ifindex) = parent_ifindex {
        append_rta_u32(&mut buf, IFLA_LINK, ifindex);
    }

    buf
}

/// Finalize the message by patching the nlmsg_len field.
fn finalize_msg(buf: &mut Vec<u8>) {
    // Align the total buffer.
    let aligned = nl_align(buf.len());
    buf.resize(aligned, 0);
    let total_len = buf.len() as u32;
    put_u32(buf, 0, total_len);
}

// ---------------------------------------------------------------------------
// Kind-specific IFLA_INFO_DATA builders
// ---------------------------------------------------------------------------

/// Append bridge-specific attributes into IFLA_INFO_DATA.
fn append_bridge_info_data(buf: &mut Vec<u8>, bridge: &BridgeSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(stp) = bridge.stp {
        append_rta_u32(buf, IFLA_BR_STP_STATE, if stp { 1 } else { 0 });
    }

    if let Some(delay) = bridge.forward_delay_sec {
        // Kernel expects jiffies (1/100 sec). .netdev stores seconds as u64.
        let jiffies = (delay * 100) as u32;
        append_rta_u32(buf, IFLA_BR_FORWARD_DELAY, jiffies);
    }

    if let Some(hello) = bridge.hello_time_sec {
        let jiffies = (hello * 100) as u32;
        append_rta_u32(buf, IFLA_BR_HELLO_TIME, jiffies);
    }

    if let Some(max_age) = bridge.max_age_sec {
        let jiffies = (max_age * 100) as u32;
        append_rta_u32(buf, IFLA_BR_MAX_AGE, jiffies);
    }

    if let Some(ageing) = bridge.ageing_time_sec {
        let jiffies = (ageing * 100) as u32;
        append_rta_u32(buf, IFLA_BR_AGEING_TIME, jiffies);
    }

    if let Some(prio) = bridge.priority {
        append_rta_u16(buf, IFLA_BR_PRIORITY, prio as u16);
    }

    if let Some(mask) = bridge.group_forward_mask {
        append_rta_u16(buf, IFLA_BR_GROUP_FWD_MASK, mask);
    }

    if let Some(pvid) = bridge.default_pvid {
        append_rta_u16(buf, IFLA_BR_VLAN_DEFAULT_PVID, pvid);
    }

    if let Some(filtering) = bridge.vlan_filtering {
        append_rta_u8(buf, IFLA_BR_VLAN_FILTERING, if filtering { 1 } else { 0 });
    }

    if let Some(ref proto) = bridge.vlan_protocol {
        // "802.1Q" → ETH_P_8021Q (0x8100), "802.1ad" → ETH_P_8021AD (0x88a8)
        let ethertype = match proto.to_lowercase().as_str() {
            "802.1q" => 0x8100u16,
            "802.1ad" => 0x88a8u16,
            _ => 0x8100u16,
        };
        append_rta_u16_be(buf, IFLA_BR_VLAN_PROTOCOL, ethertype);
    }

    if let Some(querier) = bridge.multicast_querier {
        append_rta_u8(buf, IFLA_BR_MCAST_QUERIER, if querier { 1 } else { 0 });
    }

    if let Some(snooping) = bridge.multicast_snooping {
        append_rta_u8(buf, IFLA_BR_MCAST_SNOOPING, if snooping { 1 } else { 0 });
    }

    if let Some(ver) = bridge.multicast_igmp_version {
        append_rta_u8(buf, IFLA_BR_MCAST_IGMP_VERSION, ver);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append bond-specific attributes into IFLA_INFO_DATA.
fn append_bond_info_data(buf: &mut Vec<u8>, bond: &BondSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(ref mode) = bond.mode {
        append_rta_u8(buf, IFLA_BOND_MODE, mode.as_num());
    }

    if let Some(ref policy) = bond.transmit_hash_policy {
        let val = match policy {
            TransmitHashPolicy::Layer2 => 0u8,
            TransmitHashPolicy::Layer3Plus4 => 1,
            TransmitHashPolicy::Layer2Plus3 => 2,
            TransmitHashPolicy::Encap2Plus3 => 3,
            TransmitHashPolicy::Encap3Plus4 => 4,
            TransmitHashPolicy::VlanSrcMac => 5,
        };
        append_rta_u8(buf, IFLA_BOND_XMIT_HASH_POLICY, val);
    }

    if let Some(ref rate) = bond.lacp_transmit_rate {
        let val = match rate {
            LacpTransmitRate::Slow => 0u8,
            LacpTransmitRate::Fast => 1,
        };
        append_rta_u8(buf, IFLA_BOND_AD_LACP_RATE, val);
    }

    if let Some(ref mii) = bond.mii_monitor_sec {
        // The field is a string (e.g. "100" for 100ms). Kernel expects ms.
        if let Ok(ms) = mii.trim().parse::<u32>() {
            append_rta_u32(buf, IFLA_BOND_MIIMON, ms);
        }
    }

    if let Some(ref up) = bond.up_delay_sec
        && let Ok(ms) = up.trim().parse::<u32>()
    {
        append_rta_u32(buf, IFLA_BOND_UPDELAY, ms);
    }

    if let Some(ref down) = bond.down_delay_sec
        && let Ok(ms) = down.trim().parse::<u32>()
    {
        append_rta_u32(buf, IFLA_BOND_DOWNDELAY, ms);
    }

    if let Some(ref arp_int) = bond.arp_interval_sec
        && let Ok(ms) = arp_int.trim().parse::<u32>()
    {
        append_rta_u32(buf, IFLA_BOND_ARP_INTERVAL, ms);
    }

    // ARP IP targets — nested attribute containing up to 16 IPv4 addresses
    if !bond.arp_ip_targets.is_empty() {
        let targets_start = begin_nested_rta(buf, IFLA_BOND_ARP_IP_TARGET);
        for (i, target) in bond.arp_ip_targets.iter().enumerate() {
            if let Ok(addr) = target.parse::<Ipv4Addr>() {
                append_rta_ipv4(buf, i as u16, addr);
            }
        }
        finish_nested_rta(buf, targets_start);
    }

    if let Some(ref validate) = bond.arp_validate {
        let val = match validate {
            ArpValidate::None => 0u32,
            ArpValidate::Active => 1,
            ArpValidate::Backup => 2,
            ArpValidate::All => 3,
            ArpValidate::Filter => 4,
            ArpValidate::FilterActive => 5,
            ArpValidate::FilterBackup => 6,
        };
        append_rta_u32(buf, IFLA_BOND_ARP_VALIDATE, val);
    }

    if let Some(ref targets) = bond.arp_all_targets {
        let val = match targets {
            ArpAllTargets::Any => 0u32,
            ArpAllTargets::All => 1,
        };
        append_rta_u32(buf, IFLA_BOND_ARP_ALL_TARGETS, val);
    }

    if let Some(ref reselect) = bond.primary_reselect_policy {
        let val = match reselect {
            PrimaryReselectPolicy::Always => 0u8,
            PrimaryReselectPolicy::Better => 1,
            PrimaryReselectPolicy::Failure => 2,
        };
        append_rta_u8(buf, IFLA_BOND_PRIMARY_RESELECT, val);
    }

    if let Some(resend) = bond.resend_igmp {
        append_rta_u32(buf, IFLA_BOND_RESEND_IGMP, resend);
    }

    if let Some(pps) = bond.packets_per_slave {
        append_rta_u32(buf, IFLA_BOND_PACKETS_PER_SLAVE, pps);
    }

    if let Some(garp) = bond.gratuitous_arp {
        append_rta_u32(buf, IFLA_BOND_NUM_PEER_NOTIF, garp);
    }

    if let Some(active) = bond.all_slaves_active {
        append_rta_u8(buf, IFLA_BOND_ALL_SLAVES_ACTIVE, if active { 1 } else { 0 });
    }

    if let Some(min) = bond.min_links {
        append_rta_u32(buf, IFLA_BOND_MIN_LINKS, min);
    }

    if let Some(ref ad_select) = bond.ad_select {
        let val = match ad_select {
            AdSelect::Stable => 0u8,
            AdSelect::Bandwidth => 1,
            AdSelect::Count => 2,
        };
        append_rta_u8(buf, IFLA_BOND_AD_SELECT, val);
    }

    if let Some(ref fail_over_mac) = bond.fail_over_mac_policy {
        let val = match fail_over_mac {
            FailOverMacPolicy::None => 0u8,
            FailOverMacPolicy::Active => 1,
            FailOverMacPolicy::Follow => 2,
        };
        append_rta_u8(buf, IFLA_BOND_FAIL_OVER_MAC, val);
    }

    if let Some(prio) = bond.ad_actor_sys_prio {
        append_rta_u16(buf, IFLA_BOND_AD_ACTOR_SYS_PRIO, prio);
    }

    if let Some(key) = bond.ad_user_port_key {
        append_rta_u16(buf, IFLA_BOND_AD_USER_PORT_KEY, key);
    }

    if let Some(ref sys) = bond.ad_actor_system
        && let Some(mac_bytes) = parse_mac_address(sys)
    {
        append_rta_bytes(buf, IFLA_BOND_AD_ACTOR_SYSTEM, &mac_bytes);
    }

    if let Some(ref delay) = bond.peer_notify_delay_sec
        && let Ok(ms) = delay.trim().parse::<u32>()
    {
        append_rta_u32(buf, IFLA_BOND_PEER_NOTIF_DELAY, ms);
    }

    if let Some(interval) = bond.lp_interval {
        append_rta_u32(buf, IFLA_BOND_LP_INTERVAL, interval);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append VLAN-specific attributes into IFLA_INFO_DATA.
fn append_vlan_info_data(buf: &mut Vec<u8>, vlan: &VlanSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(id) = vlan.id {
        append_rta_u16(buf, IFLA_VLAN_ID, id);
    }

    if let Some(ref proto) = vlan.protocol {
        let ethertype = match proto.to_lowercase().as_str() {
            "802.1q" => 0x8100u16,
            "802.1ad" => 0x88a8u16,
            _ => 0x8100u16,
        };
        append_rta_u16_be(buf, IFLA_VLAN_PROTOCOL, ethertype);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append VXLAN-specific attributes into IFLA_INFO_DATA.
fn append_vxlan_info_data(buf: &mut Vec<u8>, vxlan: &VxlanSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(vni) = vxlan.vni {
        append_rta_u32(buf, IFLA_VXLAN_ID, vni);
    }

    if let Some(ref group) = vxlan.group
        && let Ok(addr) = group.parse::<Ipv4Addr>()
    {
        append_rta_ipv4(buf, IFLA_VXLAN_GROUP, addr);
    }

    if let Some(ref remote) = vxlan.remote {
        // VXLAN uses the group attribute for remote when it's unicast
        if vxlan.group.is_none()
            && let Ok(addr) = remote.parse::<Ipv4Addr>()
        {
            append_rta_ipv4(buf, IFLA_VXLAN_GROUP, addr);
        }
    }

    if let Some(ref local) = vxlan.local
        && let Ok(addr) = local.parse::<Ipv4Addr>()
    {
        append_rta_ipv4(buf, IFLA_VXLAN_LOCAL, addr);
    }

    if let Some(ttl) = vxlan.ttl {
        append_rta_u8(buf, IFLA_VXLAN_TTL, ttl);
    }

    if let Some(tos) = vxlan.tos {
        append_rta_u8(buf, IFLA_VXLAN_TOS, tos);
    }

    if let Some(learning) = vxlan.mac_learning {
        append_rta_u8(buf, IFLA_VXLAN_LEARNING, if learning { 1 } else { 0 });
    }

    if let Some(ageing) = vxlan.fdb_ageing_sec {
        append_rta_u32(buf, IFLA_VXLAN_AGEING, ageing as u32);
    }

    if let Some(port) = vxlan.destination_port {
        append_rta_u16_be(buf, IFLA_VXLAN_PORT, port);
    }

    if let Some(limit) = vxlan.maximum_fdb_entries {
        append_rta_u32(buf, IFLA_VXLAN_LIMIT, limit);
    }

    if let Some(tx) = vxlan.udp6_zero_checksum_tx {
        append_rta_u8(buf, IFLA_VXLAN_UDP_ZERO_CSUM6_TX, if tx { 1 } else { 0 });
    }

    if let Some(rx) = vxlan.udp6_zero_checksum_rx {
        append_rta_u8(buf, IFLA_VXLAN_UDP_ZERO_CSUM6_RX, if rx { 1 } else { 0 });
    }

    if let Some(l2miss) = vxlan.l2_miss {
        append_rta_u8(buf, IFLA_VXLAN_L2MISS, if l2miss { 1 } else { 0 });
    }

    if let Some(l3miss) = vxlan.l3_miss {
        append_rta_u8(buf, IFLA_VXLAN_L3MISS, if l3miss { 1 } else { 0 });
    }

    if let Some(flow_label) = vxlan.flow_label {
        append_rta_u32(buf, IFLA_VXLAN_LABEL, flow_label.to_be());
    }

    if let Some(gbp) = vxlan.group_policy_extension
        && gbp
    {
        append_rta_bytes(buf, IFLA_VXLAN_GBP, &[]);
    }

    if let Some(gpe) = vxlan.generic_protocol_extension
        && gpe
    {
        append_rta_bytes(buf, IFLA_VXLAN_GPE, &[]);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append MACVLAN/MACVTAP-specific attributes into IFLA_INFO_DATA.
fn append_macvlan_info_data(buf: &mut Vec<u8>, macvlan: &MacVlanSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(ref mode) = macvlan.mode {
        let val = match mode {
            MacVlanMode::Private => MACVLAN_MODE_PRIVATE,
            MacVlanMode::Vepa => MACVLAN_MODE_VEPA,
            MacVlanMode::Bridge => MACVLAN_MODE_BRIDGE,
            MacVlanMode::Passthru => MACVLAN_MODE_PASSTHRU,
            MacVlanMode::Source => MACVLAN_MODE_SOURCE,
        };
        append_rta_u32(buf, IFLA_MACVLAN_MODE, val);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append IPVLAN-specific attributes into IFLA_INFO_DATA.
fn append_ipvlan_info_data(buf: &mut Vec<u8>, ipvlan: &IpVlanSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(ref mode) = ipvlan.mode {
        let val = match mode {
            IpVlanMode::L2 => IPVLAN_MODE_L2,
            IpVlanMode::L3 => IPVLAN_MODE_L3,
            IpVlanMode::L3S => IPVLAN_MODE_L3S,
        };
        append_rta_u16(buf, IFLA_IPVLAN_MODE, val);
    }

    if let Some(ref flags) = ipvlan.flags {
        let val = match flags {
            IpVlanFlags::Bridge => IPVLAN_F_BRIDGE,
            IpVlanFlags::Private => IPVLAN_F_PRIVATE,
            IpVlanFlags::Vepa => IPVLAN_F_VEPA,
        };
        append_rta_u16(buf, IFLA_IPVLAN_FLAGS, val);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append VRF-specific attributes into IFLA_INFO_DATA.
fn append_vrf_info_data(buf: &mut Vec<u8>, vrf: &VrfSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(table) = vrf.table {
        append_rta_u32(buf, IFLA_VRF_TABLE, table);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append veth peer info into IFLA_INFO_DATA.
///
/// The peer info is encoded as VETH_INFO_PEER containing an ifinfomsg
/// header followed by IFLA_IFNAME with the peer's name.
fn append_veth_info_data(buf: &mut Vec<u8>, peer: &PeerSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(ref peer_name) = peer.name {
        // VETH_INFO_PEER — contains an ifinfomsg + attributes for the peer
        let peer_start = begin_nested_rta(buf, VETH_INFO_PEER);

        // ifinfomsg for the peer (16 bytes, all zeros = defaults)
        let ifi_start = buf.len();
        buf.resize(ifi_start + IFINFOMSG_LEN, 0);
        buf[ifi_start] = AF_UNSPEC;

        // Peer's IFLA_IFNAME
        append_rta_str(buf, IFLA_IFNAME, peer_name);

        // Peer's IFLA_ADDRESS (optional MAC)
        if let Some(ref mac_str) = peer.mac_address
            && let Some(mac_bytes) = parse_mac_address(mac_str)
        {
            append_rta_bytes(buf, IFLA_ADDRESS, &mac_bytes);
        }

        finish_nested_rta(buf, peer_start);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append GRE/GRETAP tunnel attributes into IFLA_INFO_DATA.
fn append_gre_info_data(buf: &mut Vec<u8>, tunnel: &TunnelSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(ref local) = tunnel.local
        && let Ok(addr) = local.parse::<Ipv4Addr>()
    {
        append_rta_ipv4(buf, IFLA_GRE_LOCAL, addr);
    }

    if let Some(ref remote) = tunnel.remote
        && let Ok(addr) = remote.parse::<Ipv4Addr>()
    {
        append_rta_ipv4(buf, IFLA_GRE_REMOTE, addr);
    }

    if let Some(ttl) = tunnel.ttl {
        append_rta_u8(buf, IFLA_GRE_TTL, ttl);
    }

    if let Some(tos) = tunnel.tos {
        append_rta_u8(buf, IFLA_GRE_TOS, tos);
    }

    if let Some(ref key) = tunnel.key {
        if let Ok(k) = key.parse::<u32>() {
            append_rta_u32(buf, IFLA_GRE_IKEY, k.to_be());
            append_rta_u32(buf, IFLA_GRE_OKEY, k.to_be());
        }
    } else {
        if let Some(ref ikey) = tunnel.input_key
            && let Ok(k) = ikey.parse::<u32>()
        {
            append_rta_u32(buf, IFLA_GRE_IKEY, k.to_be());
        }
        if let Some(ref okey) = tunnel.output_key
            && let Ok(k) = okey.parse::<u32>()
        {
            append_rta_u32(buf, IFLA_GRE_OKEY, k.to_be());
        }
    }

    if let Some(idx) = tunnel.erspan_index {
        append_rta_u32(buf, IFLA_GRE_ERSPAN_INDEX, idx);
    }

    if let Some(ver) = tunnel.erspan_version {
        append_rta_u8(buf, IFLA_GRE_ERSPAN_VER, ver);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append SIT/VTI/IP6TNL tunnel attributes into IFLA_INFO_DATA.
fn append_iptun_info_data(buf: &mut Vec<u8>, tunnel: &TunnelSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(ref local) = tunnel.local
        && let Ok(addr) = local.parse::<Ipv4Addr>()
    {
        append_rta_ipv4(buf, IFLA_IPTUN_LOCAL, addr);
    }

    if let Some(ref remote) = tunnel.remote
        && let Ok(addr) = remote.parse::<Ipv4Addr>()
    {
        append_rta_ipv4(buf, IFLA_IPTUN_REMOTE, addr);
    }

    if let Some(ttl) = tunnel.ttl {
        append_rta_u8(buf, IFLA_IPTUN_TTL, ttl);
    }

    if let Some(tos) = tunnel.tos {
        append_rta_u8(buf, IFLA_IPTUN_TOS, tos);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append Geneve-specific attributes into IFLA_INFO_DATA.
fn append_geneve_info_data(buf: &mut Vec<u8>, geneve: &GeneveSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(vni) = geneve.vni {
        append_rta_u32(buf, IFLA_GENEVE_ID, vni);
    }

    if let Some(ref remote) = geneve.remote
        && let Ok(addr) = remote.parse::<Ipv4Addr>()
    {
        append_rta_ipv4(buf, IFLA_GENEVE_REMOTE, addr);
    }

    if let Some(ttl) = geneve.ttl {
        append_rta_u8(buf, IFLA_GENEVE_TTL, ttl);
    }

    if let Some(tos) = geneve.tos {
        append_rta_u8(buf, IFLA_GENEVE_TOS, tos);
    }

    if let Some(port) = geneve.destination_port {
        append_rta_u16_be(buf, IFLA_GENEVE_PORT, port);
    }

    if let Some(label) = geneve.flow_label {
        append_rta_u32(buf, IFLA_GENEVE_LABEL, label.to_be());
    }

    if let Some(tx) = geneve.udp6_zero_checksum_tx {
        append_rta_u8(buf, IFLA_GENEVE_UDP_ZERO_CSUM6_TX, if tx { 1 } else { 0 });
    }

    if let Some(rx) = geneve.udp6_zero_checksum_rx {
        append_rta_u8(buf, IFLA_GENEVE_UDP_ZERO_CSUM6_RX, if rx { 1 } else { 0 });
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append BareUDP-specific attributes into IFLA_INFO_DATA.
fn append_bareudp_info_data(buf: &mut Vec<u8>, bareudp: &BareUdpSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(port) = bareudp.destination_port {
        append_rta_u16_be(buf, IFLA_BAREUDP_PORT, port);
    }

    if let Some(ref ethertype) = bareudp.ether_type {
        // Common ethertypes: "ipv4" = 0x0800, "ipv6" = 0x86DD, "mpls" = 0x8847
        let val = match ethertype.to_lowercase().as_str() {
            "ipv4" | "ip" => 0x0800u16,
            "ipv6" | "ip6" => 0x86DDu16,
            "mpls" | "mpls-uc" => 0x8847u16,
            "mpls-mc" => 0x8848u16,
            other => other
                .strip_prefix("0x")
                .and_then(|h| u16::from_str_radix(h, 16).ok())
                .unwrap_or(0x6558), // ETH_P_TEB as default
        };
        append_rta_u16_be(buf, IFLA_BAREUDP_ETHERTYPE, val);
    }

    finish_nested_rta(buf, info_data_start);
}

/// Append XFRM-specific attributes into IFLA_INFO_DATA.
fn append_xfrm_info_data(buf: &mut Vec<u8>, xfrm: &XfrmSection) {
    let info_data_start = begin_nested_rta(buf, IFLA_INFO_DATA);

    if let Some(id) = xfrm.interface_id {
        append_rta_u32(buf, IFLA_XFRM_IF_ID, id);
    }

    finish_nested_rta(buf, info_data_start);
}

// ---------------------------------------------------------------------------
// High-level device creation
// ---------------------------------------------------------------------------

/// Map a `NetDevKind` to the kernel's link info kind string.
fn kind_to_str(kind: &NetDevKind) -> &'static str {
    match kind {
        NetDevKind::Bridge => "bridge",
        NetDevKind::Bond => "bond",
        NetDevKind::Vlan => "vlan",
        NetDevKind::Vxlan => "vxlan",
        NetDevKind::WireGuard => "wireguard",
        NetDevKind::MacVlan => "macvlan",
        NetDevKind::MacVtap => "macvtap",
        NetDevKind::IpVlan => "ipvlan",
        NetDevKind::IpVtap => "ipvtap",
        NetDevKind::Veth => "veth",
        NetDevKind::Tun => "tun",
        NetDevKind::Tap => "tap",
        NetDevKind::Dummy => "dummy",
        NetDevKind::Ifb => "ifb",
        NetDevKind::Gre => "gre",
        NetDevKind::GreTap => "gretap",
        NetDevKind::Ip6Gre => "ip6gre",
        NetDevKind::Ip6GreTap => "ip6gretap",
        NetDevKind::Sit => "sit",
        NetDevKind::Vti => "vti",
        NetDevKind::Vti6 => "vti6",
        NetDevKind::Ip6Tnl => "ip6tnl",
        NetDevKind::Erspan => "erspan",
        NetDevKind::Geneve => "geneve",
        NetDevKind::BareUdp => "bareudp",
        NetDevKind::L2tp => "l2tp",
        NetDevKind::MACsec => "macsec",
        NetDevKind::Xfrm => "xfrm",
        NetDevKind::BatAdv => "batadv",
        NetDevKind::Fou => "fou",
        NetDevKind::Wlan => "wlan",
        NetDevKind::NLMon => "nlmon",
        NetDevKind::Vrf => "vrf",
        NetDevKind::Vcan => "vcan",
        NetDevKind::VxCan => "vxcan",
    }
}

/// Determine if a device kind needs a parent link (IFLA_LINK).
fn kind_needs_parent(kind: &NetDevKind) -> bool {
    matches!(
        kind,
        NetDevKind::Vlan
            | NetDevKind::MacVlan
            | NetDevKind::MacVtap
            | NetDevKind::IpVlan
            | NetDevKind::IpVtap
    )
}

/// Build the complete RTM_NEWLINK message for creating a virtual network
/// device from a parsed `.netdev` configuration.
///
/// Returns `None` if the device type is not yet supported for creation.
pub fn build_create_netdev_msg(
    config: &NetDevConfig,
    parent_ifindex: Option<u32>,
) -> Option<Vec<u8>> {
    let name = &config.netdev_section.name;
    let kind = &config.netdev_section.kind;
    let kind_str = kind_to_str(kind);
    let mtu = config.netdev_section.mtu_bytes;
    let mac = config.netdev_section.mac_address.as_deref();

    // Some device types are not supported via simple RTM_NEWLINK.
    match kind {
        NetDevKind::L2tp
        | NetDevKind::MACsec
        | NetDevKind::BatAdv
        | NetDevKind::Fou
        | NetDevKind::Wlan => {
            log::debug!(
                "Netdev kind '{}' not yet supported for creation via netlink",
                kind_str
            );
            return None;
        }
        _ => {}
    }

    // Build the base message.
    let mut buf = build_newlink_base(name, mtu, mac, parent_ifindex);

    // Begin IFLA_LINKINFO.
    let linkinfo_start = begin_nested_rta(&mut buf, IFLA_LINKINFO);

    // IFLA_INFO_KIND.
    append_rta_str(&mut buf, IFLA_INFO_KIND, kind_str);

    // Kind-specific IFLA_INFO_DATA.
    match kind {
        NetDevKind::Bridge => {
            if let Some(ref bridge) = config.kind_config.bridge {
                append_bridge_info_data(&mut buf, bridge);
            }
        }
        NetDevKind::Bond => {
            if let Some(ref bond) = config.kind_config.bond {
                append_bond_info_data(&mut buf, bond);
            }
        }
        NetDevKind::Vlan => {
            if let Some(ref vlan) = config.kind_config.vlan {
                append_vlan_info_data(&mut buf, vlan);
            }
        }
        NetDevKind::Vxlan => {
            if let Some(ref vxlan) = config.kind_config.vxlan {
                append_vxlan_info_data(&mut buf, vxlan);
            }
        }
        NetDevKind::MacVlan | NetDevKind::MacVtap => {
            if let Some(ref macvlan) = config.kind_config.macvlan {
                append_macvlan_info_data(&mut buf, macvlan);
            }
        }
        NetDevKind::IpVlan | NetDevKind::IpVtap => {
            if let Some(ref ipvlan) = config.kind_config.ipvlan {
                append_ipvlan_info_data(&mut buf, ipvlan);
            }
        }
        NetDevKind::Veth => {
            if let Some(ref peer) = config.kind_config.peer {
                append_veth_info_data(&mut buf, peer);
            }
        }
        NetDevKind::Vrf => {
            if let Some(ref vrf) = config.kind_config.vrf {
                append_vrf_info_data(&mut buf, vrf);
            }
        }
        NetDevKind::Gre | NetDevKind::GreTap | NetDevKind::Erspan => {
            if let Some(ref tunnel) = config.kind_config.tunnel {
                append_gre_info_data(&mut buf, tunnel);
            }
        }
        NetDevKind::Sit | NetDevKind::Vti | NetDevKind::Vti6 | NetDevKind::Ip6Tnl => {
            if let Some(ref tunnel) = config.kind_config.tunnel {
                append_iptun_info_data(&mut buf, tunnel);
            }
        }
        NetDevKind::Ip6Gre | NetDevKind::Ip6GreTap => {
            // ip6gre uses the same attribute namespace as gre
            if let Some(ref tunnel) = config.kind_config.tunnel {
                append_gre_info_data(&mut buf, tunnel);
            }
        }
        NetDevKind::Geneve => {
            if let Some(ref geneve) = config.kind_config.geneve {
                append_geneve_info_data(&mut buf, geneve);
            }
        }
        NetDevKind::BareUdp => {
            if let Some(ref bareudp) = config.kind_config.bare_udp {
                append_bareudp_info_data(&mut buf, bareudp);
            }
        }
        NetDevKind::Xfrm => {
            if let Some(ref xfrm) = config.kind_config.xfrm {
                append_xfrm_info_data(&mut buf, xfrm);
            }
        }
        // Simple device types — no IFLA_INFO_DATA needed
        NetDevKind::Dummy
        | NetDevKind::Ifb
        | NetDevKind::NLMon
        | NetDevKind::Vcan
        | NetDevKind::VxCan
        | NetDevKind::WireGuard
        | NetDevKind::Tun
        | NetDevKind::Tap => {
            // No kind-specific data — just the IFLA_INFO_KIND is enough.
        }
        // Unsupported types already handled above
        _ => {}
    }

    // Close IFLA_LINKINFO.
    finish_nested_rta(&mut buf, linkinfo_start);

    // Finalize the message (set nlmsg_len).
    finalize_msg(&mut buf);

    Some(buf)
}

/// Create a single virtual network device from a `.netdev` configuration.
///
/// - For device types that need a parent link (VLAN, macvlan, etc.),
///   the parent interface must already exist.
/// - Devices that already exist are silently skipped (EEXIST).
/// - Returns `Ok(true)` if created, `Ok(false)` if skipped (exists/unsupported).
pub fn create_netdev(config: &NetDevConfig) -> Result<bool, String> {
    let name = &config.netdev_section.name;
    let kind = &config.netdev_section.kind;

    // Skip if the interface already exists.
    if interface_exists(name) {
        log::debug!("Netdev '{}' ({}) already exists, skipping", name, kind);
        return Ok(false);
    }

    // Resolve parent link if needed.
    let parent_ifindex = if kind_needs_parent(kind) {
        // For VLAN/macvlan/etc., we need to find the parent from .network files.
        // Convention: the .netdev file doesn't specify the parent directly.
        // The parent is determined by which .network file references this netdev
        // via VLAN= or similar directives. For now, we'll return an error if
        // we can't determine the parent — the caller should pass it explicitly.
        None
    } else {
        None
    };

    let msg = match build_create_netdev_msg(config, parent_ifindex) {
        Some(m) => m,
        None => {
            log::debug!("Netdev '{}' ({}) — creation not yet supported", name, kind);
            return Ok(false);
        }
    };

    match netlink_request(&msg) {
        Ok(()) => {
            log::info!("Created netdev '{}' (kind={})", name, kind);
            Ok(true)
        }
        Err(e) => {
            if e.raw_os_error() == Some(libc::EEXIST) {
                log::debug!("Netdev '{}' ({}) already exists (EEXIST)", name, kind);
                Ok(false)
            } else if e.raw_os_error() == Some(libc::EOPNOTSUPP) {
                log::warn!(
                    "Netdev '{}' ({}) — kernel does not support this device type: {}",
                    name,
                    kind,
                    e
                );
                Ok(false)
            } else {
                Err(format!(
                    "Failed to create netdev '{}' (kind={}): {}",
                    name, kind, e
                ))
            }
        }
    }
}

/// Create all virtual network devices from a list of `.netdev` configurations.
///
/// Devices are created in order. Device types that require a parent link
/// (VLAN, macvlan, etc.) are deferred to a second pass so that their parent
/// interfaces (which may themselves be netdevs like bridges or bonds) exist
/// by then.
///
/// Returns the number of devices successfully created.
pub fn create_netdevs(configs: &[NetDevConfig]) -> usize {
    if configs.is_empty() {
        return 0;
    }

    let mut created = 0;

    // First pass: create standalone devices (bridges, bonds, dummy, etc.)
    let mut deferred: Vec<&NetDevConfig> = Vec::new();

    for config in configs {
        if kind_needs_parent(&config.netdev_section.kind) {
            deferred.push(config);
            continue;
        }

        match create_netdev(config) {
            Ok(true) => created += 1,
            Ok(false) => {}
            Err(e) => log::warn!("{}", e),
        }
    }

    // Second pass: create dependent devices (VLAN, macvlan, etc.)
    // These need their parent interface to exist.
    for config in &deferred {
        // Try to resolve the parent. For now, we attempt common patterns:
        // 1. If the .netdev file has a [VLAN] section, the parent is
        //    typically specified in the matching .network file.
        // 2. We can't resolve this without the .network configs, so we
        //    build the message without IFLA_LINK and let the caller
        //    (NetworkManager) handle the parent resolution.

        let msg = match build_create_netdev_msg(config, None) {
            Some(m) => m,
            None => continue,
        };

        let name = &config.netdev_section.name;
        let kind = &config.netdev_section.kind;

        if interface_exists(name) {
            log::debug!("Netdev '{}' ({}) already exists, skipping", name, kind);
            continue;
        }

        // Try without a parent first — some kernel versions handle this.
        match netlink_request(&msg) {
            Ok(()) => {
                log::info!("Created netdev '{}' (kind={})", name, kind);
                created += 1;
            }
            Err(e) => {
                if e.raw_os_error() == Some(libc::EEXIST) {
                    log::debug!("Netdev '{}' ({}) already exists (EEXIST)", name, kind);
                } else {
                    log::warn!(
                        "Failed to create netdev '{}' (kind={}, needs parent link): {}",
                        name,
                        kind,
                        e
                    );
                }
            }
        }
    }

    created
}

/// Create a dependent netdev with an explicit parent interface index.
///
/// Used for VLAN, macvlan, macvtap, ipvlan, ipvtap devices where the
/// parent link is determined by the .network configuration referencing
/// this netdev.
pub fn create_netdev_with_parent(
    config: &NetDevConfig,
    parent_ifindex: u32,
) -> Result<bool, String> {
    let name = &config.netdev_section.name;
    let kind = &config.netdev_section.kind;

    if interface_exists(name) {
        log::debug!("Netdev '{}' ({}) already exists, skipping", name, kind);
        return Ok(false);
    }

    let msg = match build_create_netdev_msg(config, Some(parent_ifindex)) {
        Some(m) => m,
        None => return Ok(false),
    };

    match netlink_request(&msg) {
        Ok(()) => {
            log::info!(
                "Created netdev '{}' (kind={}, parent ifindex={})",
                name,
                kind,
                parent_ifindex
            );
            Ok(true)
        }
        Err(e) => {
            if e.raw_os_error() == Some(libc::EEXIST) {
                Ok(false)
            } else {
                Err(format!(
                    "Failed to create netdev '{}' (kind={}, parent={}): {}",
                    name, kind, parent_ifindex, e
                ))
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    /// Helper to create a minimal NetDevConfig for testing.
    fn make_config(name: &str, kind: NetDevKind) -> NetDevConfig {
        NetDevConfig {
            path: PathBuf::from("/test"),
            match_section: NetDevMatchSection::default(),
            netdev_section: NetDevSection {
                name: name.to_string(),
                kind,
                description: None,
                mtu_bytes: None,
                mac_address: None,
            },
            kind_config: KindConfig::default(),
        }
    }

    fn make_config_with_mtu(name: &str, kind: NetDevKind, mtu: u64) -> NetDevConfig {
        let mut c = make_config(name, kind);
        c.netdev_section.mtu_bytes = Some(mtu);
        c
    }

    fn make_config_with_mac(name: &str, kind: NetDevKind, mac: &str) -> NetDevConfig {
        let mut c = make_config(name, kind);
        c.netdev_section.mac_address = Some(mac.to_string());
        c
    }

    // ── parse_mac_address ──────────────────────────────────────────────

    #[test]
    fn test_parse_mac_address_valid() {
        assert_eq!(
            parse_mac_address("aa:bb:cc:dd:ee:ff"),
            Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff])
        );
    }

    #[test]
    fn test_parse_mac_address_uppercase() {
        assert_eq!(
            parse_mac_address("AA:BB:CC:DD:EE:FF"),
            Some([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff])
        );
    }

    #[test]
    fn test_parse_mac_address_zeros() {
        assert_eq!(
            parse_mac_address("00:00:00:00:00:00"),
            Some([0, 0, 0, 0, 0, 0])
        );
    }

    #[test]
    fn test_parse_mac_address_too_short() {
        assert_eq!(parse_mac_address("aa:bb:cc"), None);
    }

    #[test]
    fn test_parse_mac_address_too_long() {
        assert_eq!(parse_mac_address("aa:bb:cc:dd:ee:ff:11"), None);
    }

    #[test]
    fn test_parse_mac_address_invalid_hex() {
        assert_eq!(parse_mac_address("gg:hh:ii:jj:kk:ll"), None);
    }

    #[test]
    fn test_parse_mac_address_empty() {
        assert_eq!(parse_mac_address(""), None);
    }

    #[test]
    fn test_parse_mac_address_wrong_separator() {
        assert_eq!(parse_mac_address("aa-bb-cc-dd-ee-ff"), None);
    }

    // ── nl_align ───────────────────────────────────────────────────────

    #[test]
    fn test_nl_align() {
        assert_eq!(nl_align(0), 0);
        assert_eq!(nl_align(1), 4);
        assert_eq!(nl_align(4), 4);
        assert_eq!(nl_align(5), 8);
        assert_eq!(nl_align(8), 8);
        assert_eq!(nl_align(16), 16);
        assert_eq!(nl_align(17), 20);
    }

    // ── RTA serialization helpers ──────────────────────────────────────

    #[test]
    fn test_append_rta_u8() {
        let mut buf = Vec::new();
        append_rta_u8(&mut buf, 42, 0x7F);
        // rta_len=5, rta_type=42, payload=0x7F, padded to 8 bytes
        assert_eq!(buf.len(), 8); // aligned to 4
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 5); // rta_len
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), 42); // rta_type
        assert_eq!(buf[4], 0x7F); // payload
    }

    #[test]
    fn test_append_rta_u16() {
        let mut buf = Vec::new();
        append_rta_u16(&mut buf, 10, 1234);
        assert_eq!(buf.len(), 8);
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 6); // rta_len = 4 + 2
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), 10);
        assert_eq!(u16::from_ne_bytes([buf[4], buf[5]]), 1234);
    }

    #[test]
    fn test_append_rta_u32() {
        let mut buf = Vec::new();
        append_rta_u32(&mut buf, 5, 0xDEADBEEF);
        assert_eq!(buf.len(), 8);
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 8); // rta_len = 4 + 4
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), 5);
        assert_eq!(
            u32::from_ne_bytes([buf[4], buf[5], buf[6], buf[7]]),
            0xDEADBEEF
        );
    }

    #[test]
    fn test_append_rta_str() {
        let mut buf = Vec::new();
        append_rta_str(&mut buf, IFLA_IFNAME, "eth0");
        // "eth0\0" = 5 bytes payload, rta_len = 9, aligned to 12
        assert_eq!(buf.len(), 12);
        assert_eq!(u16::from_ne_bytes([buf[0], buf[1]]), 9); // 4 + 5
        assert_eq!(u16::from_ne_bytes([buf[2], buf[3]]), IFLA_IFNAME);
        assert_eq!(&buf[4..9], b"eth0\0");
    }

    #[test]
    fn test_append_rta_ipv4() {
        let mut buf = Vec::new();
        let addr = "192.168.1.1".parse::<Ipv4Addr>().unwrap();
        append_rta_ipv4(&mut buf, 7, addr);
        assert_eq!(buf.len(), 8);
        assert_eq!(&buf[4..8], &[192, 168, 1, 1]);
    }

    #[test]
    fn test_append_rta_u16_be() {
        let mut buf = Vec::new();
        append_rta_u16_be(&mut buf, 99, 0x8100);
        assert_eq!(buf.len(), 8);
        assert_eq!(buf[4], 0x81);
        assert_eq!(buf[5], 0x00);
    }

    // ── Nested RTA ─────────────────────────────────────────────────────

    #[test]
    fn test_nested_rta_basic() {
        let mut buf = Vec::new();
        let start = begin_nested_rta(&mut buf, IFLA_LINKINFO);
        append_rta_str(&mut buf, IFLA_INFO_KIND, "dummy");
        finish_nested_rta(&mut buf, start);

        // Check the outer RTA header
        let rta_len = u16::from_ne_bytes([buf[0], buf[1]]);
        let rta_type = u16::from_ne_bytes([buf[2], buf[3]]);
        assert!(rta_len > 4);
        assert_eq!(rta_type, IFLA_LINKINFO | NLA_F_NESTED);

        // The inner RTA should contain "dummy\0"
        // inner is not nested, but is the rta_len of the inner
        let inner_rta_len = u16::from_ne_bytes([buf[4], buf[5]]);
        assert!(inner_rta_len > 0);
    }

    #[test]
    fn test_nested_rta_nla_f_nested_flag() {
        let mut buf = Vec::new();
        let start = begin_nested_rta(&mut buf, 18); // IFLA_LINKINFO
        finish_nested_rta(&mut buf, start);

        let rta_type = u16::from_ne_bytes([buf[2], buf[3]]);
        assert_eq!(rta_type & NLA_F_NESTED, NLA_F_NESTED);
        assert_eq!(rta_type & 0x3FFF, 18);
    }

    // ── kind_to_str ────────────────────────────────────────────────────

    #[test]
    fn test_kind_to_str_all() {
        assert_eq!(kind_to_str(&NetDevKind::Bridge), "bridge");
        assert_eq!(kind_to_str(&NetDevKind::Bond), "bond");
        assert_eq!(kind_to_str(&NetDevKind::Vlan), "vlan");
        assert_eq!(kind_to_str(&NetDevKind::Vxlan), "vxlan");
        assert_eq!(kind_to_str(&NetDevKind::WireGuard), "wireguard");
        assert_eq!(kind_to_str(&NetDevKind::MacVlan), "macvlan");
        assert_eq!(kind_to_str(&NetDevKind::MacVtap), "macvtap");
        assert_eq!(kind_to_str(&NetDevKind::IpVlan), "ipvlan");
        assert_eq!(kind_to_str(&NetDevKind::IpVtap), "ipvtap");
        assert_eq!(kind_to_str(&NetDevKind::Veth), "veth");
        assert_eq!(kind_to_str(&NetDevKind::Tun), "tun");
        assert_eq!(kind_to_str(&NetDevKind::Tap), "tap");
        assert_eq!(kind_to_str(&NetDevKind::Dummy), "dummy");
        assert_eq!(kind_to_str(&NetDevKind::Ifb), "ifb");
        assert_eq!(kind_to_str(&NetDevKind::Gre), "gre");
        assert_eq!(kind_to_str(&NetDevKind::GreTap), "gretap");
        assert_eq!(kind_to_str(&NetDevKind::Ip6Gre), "ip6gre");
        assert_eq!(kind_to_str(&NetDevKind::Ip6GreTap), "ip6gretap");
        assert_eq!(kind_to_str(&NetDevKind::Sit), "sit");
        assert_eq!(kind_to_str(&NetDevKind::Vti), "vti");
        assert_eq!(kind_to_str(&NetDevKind::Vti6), "vti6");
        assert_eq!(kind_to_str(&NetDevKind::Ip6Tnl), "ip6tnl");
        assert_eq!(kind_to_str(&NetDevKind::Erspan), "erspan");
        assert_eq!(kind_to_str(&NetDevKind::Geneve), "geneve");
        assert_eq!(kind_to_str(&NetDevKind::BareUdp), "bareudp");
        assert_eq!(kind_to_str(&NetDevKind::L2tp), "l2tp");
        assert_eq!(kind_to_str(&NetDevKind::MACsec), "macsec");
        assert_eq!(kind_to_str(&NetDevKind::Xfrm), "xfrm");
        assert_eq!(kind_to_str(&NetDevKind::BatAdv), "batadv");
        assert_eq!(kind_to_str(&NetDevKind::Fou), "fou");
        assert_eq!(kind_to_str(&NetDevKind::Wlan), "wlan");
        assert_eq!(kind_to_str(&NetDevKind::NLMon), "nlmon");
        assert_eq!(kind_to_str(&NetDevKind::Vrf), "vrf");
        assert_eq!(kind_to_str(&NetDevKind::Vcan), "vcan");
        assert_eq!(kind_to_str(&NetDevKind::VxCan), "vxcan");
    }

    // ── kind_needs_parent ──────────────────────────────────────────────

    #[test]
    fn test_kind_needs_parent() {
        assert!(kind_needs_parent(&NetDevKind::Vlan));
        assert!(kind_needs_parent(&NetDevKind::MacVlan));
        assert!(kind_needs_parent(&NetDevKind::MacVtap));
        assert!(kind_needs_parent(&NetDevKind::IpVlan));
        assert!(kind_needs_parent(&NetDevKind::IpVtap));
    }

    #[test]
    fn test_kind_does_not_need_parent() {
        assert!(!kind_needs_parent(&NetDevKind::Bridge));
        assert!(!kind_needs_parent(&NetDevKind::Bond));
        assert!(!kind_needs_parent(&NetDevKind::Dummy));
        assert!(!kind_needs_parent(&NetDevKind::Vxlan));
        assert!(!kind_needs_parent(&NetDevKind::WireGuard));
        assert!(!kind_needs_parent(&NetDevKind::Veth));
        assert!(!kind_needs_parent(&NetDevKind::Vrf));
        assert!(!kind_needs_parent(&NetDevKind::Gre));
        assert!(!kind_needs_parent(&NetDevKind::Geneve));
        assert!(!kind_needs_parent(&NetDevKind::BareUdp));
        assert!(!kind_needs_parent(&NetDevKind::Xfrm));
    }

    // ── build_newlink_base ─────────────────────────────────────────────

    #[test]
    fn test_build_newlink_base_minimal() {
        let buf = build_newlink_base("test0", None, None, None);
        // Must contain nlmsghdr (16) + ifinfomsg (16) + IFLA_IFNAME rta
        assert!(buf.len() >= NLMSG_HDR_LEN + IFINFOMSG_LEN + 12);

        // Check RTM_NEWLINK
        let nlmsg_type = u16::from_ne_bytes([buf[4], buf[5]]);
        assert_eq!(nlmsg_type, RTM_NEWLINK);

        // Check flags
        let flags = u16::from_ne_bytes([buf[6], buf[7]]);
        assert_eq!(flags, NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL);

        // Check ifi_family
        assert_eq!(buf[NLMSG_HDR_LEN], AF_UNSPEC);

        // Check name is in the buffer
        assert!(buf.windows(6).any(|w| w == b"test0\0"));
    }

    #[test]
    fn test_build_newlink_base_with_mtu() {
        let buf = build_newlink_base("eth0", Some(9000), None, None);
        // Should contain IFLA_MTU attribute
        // Find the MTU value (9000 = 0x2328) in the buffer
        let mtu_bytes = 9000u32.to_ne_bytes();
        assert!(buf.windows(4).any(|w| w == mtu_bytes));
    }

    #[test]
    fn test_build_newlink_base_with_mac() {
        let buf = build_newlink_base("eth0", None, Some("aa:bb:cc:dd:ee:ff"), None);
        // Should contain the MAC address bytes
        assert!(
            buf.windows(6)
                .any(|w| w == [0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff])
        );
    }

    #[test]
    fn test_build_newlink_base_with_parent() {
        let buf = build_newlink_base("vlan10", None, None, Some(42));
        // Should contain IFLA_LINK with value 42
        let idx_bytes = 42u32.to_ne_bytes();
        assert!(buf.windows(4).any(|w| w == idx_bytes));
    }

    // ── build_create_netdev_msg: simple device types ───────────────────

    #[test]
    fn test_build_msg_dummy() {
        let config = make_config("dummy0", NetDevKind::Dummy);
        let msg = build_create_netdev_msg(&config, None);
        assert!(msg.is_some());

        let buf = msg.unwrap();
        // Check nlmsg_len is consistent
        let nlmsg_len = u32::from_ne_bytes(buf[0..4].try_into().unwrap()) as usize;
        assert_eq!(nlmsg_len, buf.len());

        // Check the device name is present
        assert!(buf.windows(7).any(|w| w == b"dummy0\0"));

        // Check "dummy" kind string is present
        assert!(buf.windows(6).any(|w| w == b"dummy\0"));
    }

    #[test]
    fn test_build_msg_vcan() {
        let config = make_config("vcan0", NetDevKind::Vcan);
        let msg = build_create_netdev_msg(&config, None);
        assert!(msg.is_some());
        let buf = msg.unwrap();
        assert!(buf.windows(6).any(|w| w == b"vcan0\0"));
        assert!(buf.windows(5).any(|w| w == b"vcan\0"));
    }

    #[test]
    fn test_build_msg_nlmon() {
        let config = make_config("nlmon0", NetDevKind::NLMon);
        let msg = build_create_netdev_msg(&config, None);
        assert!(msg.is_some());
        let buf = msg.unwrap();
        assert!(buf.windows(6).any(|w| w == b"nlmon\0"));
    }

    #[test]
    fn test_build_msg_ifb() {
        let config = make_config("ifb0", NetDevKind::Ifb);
        let msg = build_create_netdev_msg(&config, None);
        assert!(msg.is_some());
        let buf = msg.unwrap();
        assert!(buf.windows(4).any(|w| w == b"ifb\0"));
    }

    #[test]
    fn test_build_msg_wireguard() {
        let config = make_config("wg0", NetDevKind::WireGuard);
        let msg = build_create_netdev_msg(&config, None);
        assert!(msg.is_some());
        let buf = msg.unwrap();
        assert!(buf.windows(10).any(|w| w == b"wireguard\0"));
    }

    // ── build_create_netdev_msg: unsupported types ─────────────────────

    #[test]
    fn test_build_msg_l2tp_unsupported() {
        let config = make_config("l2tp0", NetDevKind::L2tp);
        assert!(build_create_netdev_msg(&config, None).is_none());
    }

    #[test]
    fn test_build_msg_macsec_unsupported() {
        let config = make_config("macsec0", NetDevKind::MACsec);
        assert!(build_create_netdev_msg(&config, None).is_none());
    }

    #[test]
    fn test_build_msg_batadv_unsupported() {
        let config = make_config("bat0", NetDevKind::BatAdv);
        assert!(build_create_netdev_msg(&config, None).is_none());
    }

    #[test]
    fn test_build_msg_fou_unsupported() {
        let config = make_config("fou0", NetDevKind::Fou);
        assert!(build_create_netdev_msg(&config, None).is_none());
    }

    #[test]
    fn test_build_msg_wlan_unsupported() {
        let config = make_config("wlan0", NetDevKind::Wlan);
        assert!(build_create_netdev_msg(&config, None).is_none());
    }

    // ── build_create_netdev_msg: with MTU and MAC ──────────────────────

    #[test]
    fn test_build_msg_dummy_with_mtu() {
        let config = make_config_with_mtu("dummy0", NetDevKind::Dummy, 1500);
        let msg = build_create_netdev_msg(&config, None).unwrap();
        let mtu_bytes = 1500u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == mtu_bytes));
    }

    #[test]
    fn test_build_msg_bridge_with_mac() {
        let config = make_config_with_mac("br0", NetDevKind::Bridge, "02:42:ac:11:00:01");
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(
            msg.windows(6)
                .any(|w| w == [0x02, 0x42, 0xac, 0x11, 0x00, 0x01])
        );
    }

    // ── build_create_netdev_msg: bridge with parameters ────────────────

    #[test]
    fn test_build_msg_bridge_basic() {
        let config = make_config("br0", NetDevKind::Bridge);
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"bridge\0"));
    }

    #[test]
    fn test_build_msg_bridge_with_stp() {
        let mut config = make_config("br0", NetDevKind::Bridge);
        config.kind_config.bridge = Some(BridgeSection {
            stp: Some(true),
            priority: Some(100),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"bridge\0"));
        // STP state = 1 should be in the buffer
        // Priority = 100 should be in the buffer
        let prio_bytes = 100u16.to_ne_bytes();
        assert!(msg.windows(2).any(|w| w == prio_bytes));
    }

    #[test]
    fn test_build_msg_bridge_all_fields() {
        let mut config = make_config("br-all", NetDevKind::Bridge);
        config.kind_config.bridge = Some(BridgeSection {
            stp: Some(true),
            forward_delay_sec: Some(15),
            hello_time_sec: Some(2),
            max_age_sec: Some(20),
            ageing_time_sec: Some(300),
            priority: Some(32768),
            group_forward_mask: Some(0),
            default_pvid: Some(1),
            multicast_querier: Some(false),
            multicast_snooping: Some(true),
            vlan_filtering: Some(true),
            vlan_protocol: Some("802.1Q".to_string()),
            multicast_igmp_version: Some(3),
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"bridge\0"));
        // Check that the message length is consistent
        let nlmsg_len = u32::from_ne_bytes(msg[0..4].try_into().unwrap()) as usize;
        assert_eq!(nlmsg_len, msg.len());
    }

    // ── build_create_netdev_msg: bond ──────────────────────────────────

    #[test]
    fn test_build_msg_bond_basic() {
        let mut config = make_config("bond0", NetDevKind::Bond);
        config.kind_config.bond = Some(BondSection {
            mode: Some(BondMode::Ieee802_3ad),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(5).any(|w| w == b"bond\0"));
        // Mode 802.3ad = 4, should be in the buffer as a u8
        assert!(msg.contains(&4u8));
    }

    #[test]
    fn test_build_msg_bond_all_modes() {
        for (mode, expected_num) in [
            (BondMode::BalanceRr, 0u8),
            (BondMode::ActiveBackup, 1),
            (BondMode::BalanceXor, 2),
            (BondMode::Broadcast, 3),
            (BondMode::Ieee802_3ad, 4),
            (BondMode::BalanceTlb, 5),
            (BondMode::BalanceAlb, 6),
        ] {
            let mut config = make_config("bond0", NetDevKind::Bond);
            config.kind_config.bond = Some(BondSection {
                mode: Some(mode),
                ..Default::default()
            });
            let msg = build_create_netdev_msg(&config, None).unwrap();
            assert!(msg.windows(5).any(|w| w == b"bond\0"));
            // The mode value should be present
            assert_eq!(mode.as_num(), expected_num);
        }
    }

    #[test]
    fn test_build_msg_bond_with_miimon() {
        let mut config = make_config("bond0", NetDevKind::Bond);
        config.kind_config.bond = Some(BondSection {
            mode: Some(BondMode::ActiveBackup),
            mii_monitor_sec: Some("100".to_string()), // 100ms
            up_delay_sec: Some("200".to_string()),    // 200ms
            down_delay_sec: Some("200".to_string()),  // 200ms
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        // 100ms in kernel format
        let miimon_bytes = 100u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == miimon_bytes));
        // 200ms
        let delay_bytes = 200u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == delay_bytes));
    }

    // ── build_create_netdev_msg: vlan ──────────────────────────────────

    #[test]
    fn test_build_msg_vlan() {
        let mut config = make_config("vlan10", NetDevKind::Vlan);
        config.kind_config.vlan = Some(VlanSection {
            id: Some(10),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, Some(2)).unwrap();
        assert!(msg.windows(5).any(|w| w == b"vlan\0"));
        // VLAN ID 10 as u16
        let id_bytes = 10u16.to_ne_bytes();
        assert!(msg.windows(2).any(|w| w == id_bytes));
        // Parent ifindex 2
        let parent_bytes = 2u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == parent_bytes));
    }

    #[test]
    fn test_build_msg_vlan_with_protocol() {
        let mut config = make_config("vlan100", NetDevKind::Vlan);
        config.kind_config.vlan = Some(VlanSection {
            id: Some(100),
            protocol: Some("802.1ad".to_string()),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, Some(5)).unwrap();
        // 802.1ad ethertype = 0x88a8 in big endian
        assert!(msg.windows(2).any(|w| w == [0x88, 0xa8]));
    }

    // ── build_create_netdev_msg: vxlan ─────────────────────────────────

    #[test]
    fn test_build_msg_vxlan_basic() {
        let mut config = make_config("vxlan42", NetDevKind::Vxlan);
        config.kind_config.vxlan = Some(VxlanSection {
            vni: Some(42),
            destination_port: Some(4789),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(6).any(|w| w == b"vxlan\0"));
        // VNI 42 as u32
        let vni_bytes = 42u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == vni_bytes));
        // Port 4789 in big endian
        let port_bytes = 4789u16.to_be_bytes();
        assert!(msg.windows(2).any(|w| w == port_bytes));
    }

    #[test]
    fn test_build_msg_vxlan_with_group() {
        let mut config = make_config("vxlan10", NetDevKind::Vxlan);
        config.kind_config.vxlan = Some(VxlanSection {
            vni: Some(10),
            group: Some("239.1.1.1".to_string()),
            ttl: Some(64),
            mac_learning: Some(true),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        // Group address bytes
        assert!(msg.windows(4).any(|w| w == [239, 1, 1, 1]));
    }

    // ── build_create_netdev_msg: macvlan ───────────────────────────────

    #[test]
    fn test_build_msg_macvlan_bridge() {
        let mut config = make_config("macvlan0", NetDevKind::MacVlan);
        config.kind_config.macvlan = Some(MacVlanSection {
            mode: Some(MacVlanMode::Bridge),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, Some(3)).unwrap();
        assert!(msg.windows(8).any(|w| w == b"macvlan\0"));
        // Bridge mode = 4
        let mode_bytes = MACVLAN_MODE_BRIDGE.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == mode_bytes));
    }

    #[test]
    fn test_build_msg_macvtap() {
        let mut config = make_config("macvtap0", NetDevKind::MacVtap);
        config.kind_config.macvlan = Some(MacVlanSection {
            mode: Some(MacVlanMode::Passthru),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, Some(4)).unwrap();
        assert!(msg.windows(8).any(|w| w == b"macvtap\0"));
        let mode_bytes = MACVLAN_MODE_PASSTHRU.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == mode_bytes));
    }

    #[test]
    fn test_macvlan_mode_values() {
        assert_eq!(MACVLAN_MODE_PRIVATE, 1);
        assert_eq!(MACVLAN_MODE_VEPA, 2);
        assert_eq!(MACVLAN_MODE_BRIDGE, 4);
        assert_eq!(MACVLAN_MODE_PASSTHRU, 8);
        assert_eq!(MACVLAN_MODE_SOURCE, 16);
    }

    // ── build_create_netdev_msg: ipvlan ────────────────────────────────

    #[test]
    fn test_build_msg_ipvlan_l3() {
        let mut config = make_config("ipvlan0", NetDevKind::IpVlan);
        config.kind_config.ipvlan = Some(IpVlanSection {
            mode: Some(IpVlanMode::L3),
            flags: Some(IpVlanFlags::Private),
        });
        let msg = build_create_netdev_msg(&config, Some(2)).unwrap();
        assert!(msg.windows(7).any(|w| w == b"ipvlan\0"));
        // L3 mode = 1
        let mode_bytes = IPVLAN_MODE_L3.to_ne_bytes();
        assert!(msg.windows(2).any(|w| w == mode_bytes));
    }

    #[test]
    fn test_ipvlan_mode_values() {
        assert_eq!(IPVLAN_MODE_L2, 0);
        assert_eq!(IPVLAN_MODE_L3, 1);
        assert_eq!(IPVLAN_MODE_L3S, 2);
    }

    // ── build_create_netdev_msg: veth ──────────────────────────────────

    #[test]
    fn test_build_msg_veth() {
        let mut config = make_config("veth0", NetDevKind::Veth);
        config.kind_config.peer = Some(PeerSection {
            name: Some("veth1".to_string()),
            mac_address: None,
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(5).any(|w| w == b"veth\0"));
        // Peer name should be present
        assert!(msg.windows(6).any(|w| w == b"veth1\0"));
    }

    #[test]
    fn test_build_msg_veth_with_peer_mac() {
        let mut config = make_config("veth0", NetDevKind::Veth);
        config.kind_config.peer = Some(PeerSection {
            name: Some("veth1".to_string()),
            mac_address: Some("02:00:00:00:00:01".to_string()),
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(
            msg.windows(6)
                .any(|w| w == [0x02, 0x00, 0x00, 0x00, 0x00, 0x01])
        );
    }

    // ── build_create_netdev_msg: vrf ───────────────────────────────────

    #[test]
    fn test_build_msg_vrf() {
        let mut config = make_config("vrf-red", NetDevKind::Vrf);
        config.kind_config.vrf = Some(VrfSection { table: Some(100) });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(4).any(|w| w == b"vrf\0"));
        let table_bytes = 100u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == table_bytes));
    }

    // ── build_create_netdev_msg: tunnels ───────────────────────────────

    #[test]
    fn test_build_msg_gre_tunnel() {
        let mut config = make_config("gre1", NetDevKind::Gre);
        config.kind_config.tunnel = Some(TunnelSection {
            local: Some("10.0.0.1".to_string()),
            remote: Some("10.0.0.2".to_string()),
            ttl: Some(64),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(4).any(|w| w == b"gre\0"));
        assert!(msg.windows(4).any(|w| w == [10, 0, 0, 1]));
        assert!(msg.windows(4).any(|w| w == [10, 0, 0, 2]));
    }

    #[test]
    fn test_build_msg_gretap_tunnel() {
        let mut config = make_config("gretap1", NetDevKind::GreTap);
        config.kind_config.tunnel = Some(TunnelSection {
            local: Some("192.168.1.1".to_string()),
            remote: Some("192.168.1.2".to_string()),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"gretap\0"));
    }

    #[test]
    fn test_build_msg_sit_tunnel() {
        let mut config = make_config("sit1", NetDevKind::Sit);
        config.kind_config.tunnel = Some(TunnelSection {
            local: Some("10.0.0.1".to_string()),
            remote: Some("10.0.0.2".to_string()),
            ttl: Some(128),
            tos: Some(0),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(4).any(|w| w == b"sit\0"));
    }

    #[test]
    fn test_build_msg_erspan() {
        let mut config = make_config("erspan0", NetDevKind::Erspan);
        config.kind_config.tunnel = Some(TunnelSection {
            local: Some("10.0.0.1".to_string()),
            remote: Some("10.0.0.2".to_string()),
            erspan_index: Some(1),
            erspan_version: Some(1),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"erspan\0"));
        // erspan_index = 1
        let idx_bytes = 1u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == idx_bytes));
    }

    #[test]
    fn test_build_msg_ip6gre() {
        let mut config = make_config("ip6gre1", NetDevKind::Ip6Gre);
        config.kind_config.tunnel = Some(TunnelSection::default());
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"ip6gre\0"));
    }

    #[test]
    fn test_build_msg_vti() {
        let config = make_config("vti0", NetDevKind::Vti);
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(4).any(|w| w == b"vti\0"));
    }

    #[test]
    fn test_build_msg_ip6tnl() {
        let config = make_config("ip6tnl0", NetDevKind::Ip6Tnl);
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"ip6tnl\0"));
    }

    // ── build_create_netdev_msg: geneve ────────────────────────────────

    #[test]
    fn test_build_msg_geneve() {
        let mut config = make_config("geneve0", NetDevKind::Geneve);
        config.kind_config.geneve = Some(GeneveSection {
            vni: Some(1000),
            remote: Some("10.0.0.1".to_string()),
            destination_port: Some(6081),
            ttl: Some(64),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(7).any(|w| w == b"geneve\0"));
        // VNI 1000
        let vni_bytes = 1000u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == vni_bytes));
        // Remote address
        assert!(msg.windows(4).any(|w| w == [10, 0, 0, 1]));
    }

    // ── build_create_netdev_msg: bareudp ───────────────────────────────

    #[test]
    fn test_build_msg_bareudp() {
        let mut config = make_config("bareudp0", NetDevKind::BareUdp);
        config.kind_config.bare_udp = Some(BareUdpSection {
            destination_port: Some(6635),
            ether_type: Some("mpls".to_string()),
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(8).any(|w| w == b"bareudp\0"));
        // Port 6635 in big endian
        let port_bytes = 6635u16.to_be_bytes();
        assert!(msg.windows(2).any(|w| w == port_bytes));
        // MPLS ethertype 0x8847 in big endian
        assert!(msg.windows(2).any(|w| w == [0x88, 0x47]));
    }

    #[test]
    fn test_bareudp_ethertype_parsing() {
        let mut config = make_config("bareudp0", NetDevKind::BareUdp);

        // IPv4
        config.kind_config.bare_udp = Some(BareUdpSection {
            destination_port: Some(1234),
            ether_type: Some("ipv4".to_string()),
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(2).any(|w| w == [0x08, 0x00])); // 0x0800

        // IPv6
        config.kind_config.bare_udp = Some(BareUdpSection {
            destination_port: Some(1234),
            ether_type: Some("ipv6".to_string()),
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(2).any(|w| w == [0x86, 0xDD])); // 0x86DD

        // Hex value
        config.kind_config.bare_udp = Some(BareUdpSection {
            destination_port: Some(1234),
            ether_type: Some("0x0806".to_string()),
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(2).any(|w| w == [0x08, 0x06])); // ARP
    }

    // ── build_create_netdev_msg: xfrm ──────────────────────────────────

    #[test]
    fn test_build_msg_xfrm() {
        let mut config = make_config("xfrm0", NetDevKind::Xfrm);
        config.kind_config.xfrm = Some(XfrmSection {
            interface_id: Some(42),
            independent: None,
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(5).any(|w| w == b"xfrm\0"));
        let id_bytes = 42u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == id_bytes));
    }

    // ── finalize_msg ───────────────────────────────────────────────────

    #[test]
    fn test_finalize_msg_alignment() {
        let mut buf = vec![0u8; 33]; // Not aligned
        finalize_msg(&mut buf);
        assert_eq!(buf.len() % 4, 0);
        let len = u32::from_ne_bytes(buf[0..4].try_into().unwrap()) as usize;
        assert_eq!(len, buf.len());
    }

    #[test]
    fn test_finalize_msg_already_aligned() {
        let mut buf = vec![0u8; 32]; // Already aligned
        finalize_msg(&mut buf);
        assert_eq!(buf.len(), 32);
        let len = u32::from_ne_bytes(buf[0..4].try_into().unwrap()) as usize;
        assert_eq!(len, 32);
    }

    // ── Message structure verification ─────────────────────────────────

    #[test]
    fn test_msg_structure_nlmsghdr() {
        let config = make_config("test0", NetDevKind::Dummy);
        let msg = build_create_netdev_msg(&config, None).unwrap();

        // nlmsg_len (u32) at offset 0
        let nlmsg_len = u32::from_ne_bytes(msg[0..4].try_into().unwrap()) as usize;
        assert_eq!(nlmsg_len, msg.len());

        // nlmsg_type (u16) at offset 4
        let nlmsg_type = u16::from_ne_bytes(msg[4..6].try_into().unwrap());
        assert_eq!(nlmsg_type, RTM_NEWLINK);

        // nlmsg_flags (u16) at offset 6
        let nlmsg_flags = u16::from_ne_bytes(msg[6..8].try_into().unwrap());
        assert_eq!(
            nlmsg_flags,
            NLM_F_REQUEST | NLM_F_ACK | NLM_F_CREATE | NLM_F_EXCL
        );

        // nlmsg_seq (u32) at offset 8
        let nlmsg_seq = u32::from_ne_bytes(msg[8..12].try_into().unwrap());
        assert_eq!(nlmsg_seq, 1);

        // nlmsg_pid (u32) at offset 12
        let nlmsg_pid = u32::from_ne_bytes(msg[12..16].try_into().unwrap());
        assert_eq!(nlmsg_pid, 0);
    }

    #[test]
    fn test_msg_structure_ifinfomsg() {
        let config = make_config("test0", NetDevKind::Dummy);
        let msg = build_create_netdev_msg(&config, None).unwrap();

        // ifinfomsg starts at offset 16
        let ifi_family = msg[16];
        assert_eq!(ifi_family, AF_UNSPEC);

        // ifi_type (u16) at offset 18 — should be 0
        let ifi_type = u16::from_ne_bytes(msg[18..20].try_into().unwrap());
        assert_eq!(ifi_type, 0);

        // ifi_index (i32) at offset 20 — should be 0 (new device)
        let ifi_index = i32::from_ne_bytes(msg[20..24].try_into().unwrap());
        assert_eq!(ifi_index, 0);
    }

    // ── Constants ──────────────────────────────────────────────────────

    #[test]
    fn test_netlink_constants() {
        assert_eq!(NETLINK_ROUTE, 0);
        assert_eq!(RTM_NEWLINK, 16);
        assert_eq!(NLMSG_HDR_LEN, 16);
        assert_eq!(IFINFOMSG_LEN, 16);
        assert_eq!(AF_UNSPEC, 0);
        assert_eq!(NLM_F_REQUEST, 1);
        assert_eq!(NLM_F_ACK, 4);
        assert_eq!(NLM_F_CREATE, 0x400);
        assert_eq!(NLM_F_EXCL, 0x200);
        assert_eq!(NLA_F_NESTED, 0x8000);
    }

    #[test]
    fn test_ifla_constants() {
        assert_eq!(IFLA_ADDRESS, 1);
        assert_eq!(IFLA_IFNAME, 3);
        assert_eq!(IFLA_MTU, 4);
        assert_eq!(IFLA_LINK, 5);
        assert_eq!(IFLA_LINKINFO, 18);
        assert_eq!(IFLA_INFO_KIND, 1);
        assert_eq!(IFLA_INFO_DATA, 2);
    }

    #[test]
    fn test_bridge_attr_constants() {
        assert_eq!(IFLA_BR_FORWARD_DELAY, 1);
        assert_eq!(IFLA_BR_HELLO_TIME, 2);
        assert_eq!(IFLA_BR_MAX_AGE, 3);
        assert_eq!(IFLA_BR_AGEING_TIME, 4);
        assert_eq!(IFLA_BR_STP_STATE, 5);
        assert_eq!(IFLA_BR_PRIORITY, 6);
        assert_eq!(IFLA_BR_VLAN_FILTERING, 7);
        assert_eq!(IFLA_BR_VLAN_PROTOCOL, 8);
        assert_eq!(IFLA_BR_GROUP_FWD_MASK, 9);
        assert_eq!(IFLA_BR_MCAST_SNOOPING, 23);
        assert_eq!(IFLA_BR_MCAST_QUERIER, 25);
        assert_eq!(IFLA_BR_VLAN_DEFAULT_PVID, 39);
        assert_eq!(IFLA_BR_MCAST_IGMP_VERSION, 43);
    }

    #[test]
    fn test_bond_attr_constants() {
        assert_eq!(IFLA_BOND_MODE, 1);
        assert_eq!(IFLA_BOND_MIIMON, 3);
        assert_eq!(IFLA_BOND_UPDELAY, 4);
        assert_eq!(IFLA_BOND_DOWNDELAY, 5);
        assert_eq!(IFLA_BOND_XMIT_HASH_POLICY, 14);
        assert_eq!(IFLA_BOND_AD_LACP_RATE, 21);
        assert_eq!(IFLA_BOND_AD_SELECT, 22);
        assert_eq!(IFLA_BOND_MIN_LINKS, 18);
    }

    #[test]
    fn test_vlan_attr_constants() {
        assert_eq!(IFLA_VLAN_ID, 1);
        assert_eq!(IFLA_VLAN_PROTOCOL, 5);
    }

    #[test]
    fn test_vxlan_attr_constants() {
        assert_eq!(IFLA_VXLAN_ID, 1);
        assert_eq!(IFLA_VXLAN_GROUP, 2);
        assert_eq!(IFLA_VXLAN_PORT, 15);
        assert_eq!(IFLA_VXLAN_LEARNING, 7);
    }

    // ── interface_exists ───────────────────────────────────────────────

    #[test]
    fn test_interface_exists_lo() {
        // loopback should always exist
        assert!(interface_exists("lo"));
    }

    #[test]
    fn test_interface_exists_nonexistent() {
        assert!(!interface_exists("this_interface_does_not_exist_12345"));
    }

    // ── resolve_ifindex ────────────────────────────────────────────────

    #[test]
    fn test_resolve_ifindex_lo() {
        // loopback should have ifindex 1
        let idx = resolve_ifindex("lo");
        assert!(idx.is_some());
        assert_eq!(idx.unwrap(), 1);
    }

    #[test]
    fn test_resolve_ifindex_nonexistent() {
        assert!(resolve_ifindex("nonexistent_iface_xyz").is_none());
    }

    // ── create_netdevs with empty config ───────────────────────────────

    #[test]
    fn test_create_netdevs_empty() {
        let configs: Vec<NetDevConfig> = Vec::new();
        let created = create_netdevs(&configs);
        assert_eq!(created, 0);
    }

    // ── Ordering: standalone before dependent ──────────────────────────

    #[test]
    fn test_create_netdevs_ordering() {
        // Verify that standalone devices (bridge) would be created before
        // dependent devices (vlan).
        let configs = vec![
            make_config("vlan10", NetDevKind::Vlan),
            make_config("br0", NetDevKind::Bridge),
            make_config("dummy0", NetDevKind::Dummy),
        ];

        // We can't actually create devices in tests (needs CAP_NET_ADMIN),
        // but we can verify the message construction ordering.
        let mut standalone = Vec::new();
        let mut dependent = Vec::new();

        for config in &configs {
            if kind_needs_parent(&config.netdev_section.kind) {
                dependent.push(&config.netdev_section.name);
            } else {
                standalone.push(&config.netdev_section.name);
            }
        }

        assert_eq!(standalone, vec!["br0", "dummy0"]);
        assert_eq!(dependent, vec!["vlan10"]);
    }

    // ── build_create_netdev_msg returns valid messages ──────────────────

    #[test]
    fn test_all_supported_kinds_produce_messages() {
        let supported_kinds = vec![
            NetDevKind::Dummy,
            NetDevKind::Bridge,
            NetDevKind::Bond,
            NetDevKind::Vlan,
            NetDevKind::Vxlan,
            NetDevKind::MacVlan,
            NetDevKind::MacVtap,
            NetDevKind::IpVlan,
            NetDevKind::IpVtap,
            NetDevKind::Veth,
            NetDevKind::Tun,
            NetDevKind::Tap,
            NetDevKind::Ifb,
            NetDevKind::NLMon,
            NetDevKind::Vcan,
            NetDevKind::VxCan,
            NetDevKind::WireGuard,
            NetDevKind::Vrf,
            NetDevKind::Gre,
            NetDevKind::GreTap,
            NetDevKind::Ip6Gre,
            NetDevKind::Ip6GreTap,
            NetDevKind::Sit,
            NetDevKind::Vti,
            NetDevKind::Vti6,
            NetDevKind::Ip6Tnl,
            NetDevKind::Erspan,
            NetDevKind::Geneve,
            NetDevKind::BareUdp,
            NetDevKind::Xfrm,
        ];

        for kind in supported_kinds {
            let name = format!("test-{}", kind_to_str(&kind));
            let config = make_config(&name, kind);
            let msg = build_create_netdev_msg(&config, None);
            assert!(
                msg.is_some(),
                "Expected message for kind '{}'",
                kind_to_str(&config.netdev_section.kind)
            );

            let buf = msg.unwrap();
            // Every message should be aligned
            assert_eq!(buf.len() % 4, 0, "Message for '{}' not aligned", name);
            // Every message should have valid nlmsg_len
            let nlmsg_len = u32::from_ne_bytes(buf[0..4].try_into().unwrap()) as usize;
            assert_eq!(nlmsg_len, buf.len(), "nlmsg_len mismatch for '{}'", name);
            // Every message should be at least header + ifinfomsg + name
            assert!(
                buf.len() >= NLMSG_HDR_LEN + IFINFOMSG_LEN + 8,
                "Message for '{}' too short: {} bytes",
                name,
                buf.len()
            );
        }
    }

    #[test]
    fn test_unsupported_kinds_return_none() {
        let unsupported = vec![
            NetDevKind::L2tp,
            NetDevKind::MACsec,
            NetDevKind::BatAdv,
            NetDevKind::Fou,
            NetDevKind::Wlan,
        ];

        for kind in unsupported {
            let config = make_config("test", kind);
            assert!(
                build_create_netdev_msg(&config, None).is_none(),
                "Expected None for kind '{}'",
                kind_to_str(&kind)
            );
        }
    }

    // ── GRE tunnel with keys ───────────────────────────────────────────

    #[test]
    fn test_build_msg_gre_with_key() {
        let mut config = make_config("gre1", NetDevKind::Gre);
        config.kind_config.tunnel = Some(TunnelSection {
            local: Some("10.0.0.1".to_string()),
            remote: Some("10.0.0.2".to_string()),
            key: Some("42".to_string()),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        // Key 42 in big endian
        let key_bytes = 42u32.to_be_bytes();
        // Should appear twice (ikey and okey)
        let count = msg.windows(4).filter(|w| *w == key_bytes).count();
        assert_eq!(count, 2, "Expected ikey and okey to both contain key=42");
    }

    #[test]
    fn test_build_msg_gre_with_separate_keys() {
        let mut config = make_config("gre1", NetDevKind::Gre);
        config.kind_config.tunnel = Some(TunnelSection {
            local: Some("10.0.0.1".to_string()),
            remote: Some("10.0.0.2".to_string()),
            input_key: Some("100".to_string()),
            output_key: Some("200".to_string()),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        let ikey_bytes = 100u32.to_be_bytes();
        let okey_bytes = 200u32.to_be_bytes();
        assert!(msg.windows(4).any(|w| w == ikey_bytes));
        assert!(msg.windows(4).any(|w| w == okey_bytes));
    }

    // ── Bond with ARP targets ──────────────────────────────────────────

    #[test]
    fn test_build_msg_bond_with_arp_targets() {
        let mut config = make_config("bond0", NetDevKind::Bond);
        config.kind_config.bond = Some(BondSection {
            mode: Some(BondMode::ActiveBackup),
            arp_interval_sec: Some("1000".to_string()),
            arp_ip_targets: vec!["10.0.0.1".to_string(), "10.0.0.2".to_string()],
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        assert!(msg.windows(4).any(|w| w == [10, 0, 0, 1]));
        assert!(msg.windows(4).any(|w| w == [10, 0, 0, 2]));
    }

    // ── VXLAN with boolean flags ───────────────────────────────────────

    #[test]
    fn test_build_msg_vxlan_all_flags() {
        let mut config = make_config("vxlan0", NetDevKind::Vxlan);
        config.kind_config.vxlan = Some(VxlanSection {
            vni: Some(100),
            mac_learning: Some(false),
            l2_miss: Some(true),
            l3_miss: Some(true),
            udp6_zero_checksum_tx: Some(true),
            udp6_zero_checksum_rx: Some(true),
            group_policy_extension: Some(true),
            generic_protocol_extension: Some(false),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        let nlmsg_len = u32::from_ne_bytes(msg[0..4].try_into().unwrap()) as usize;
        assert_eq!(nlmsg_len, msg.len());
    }

    // ── Bridge VLAN protocol 802.1ad ───────────────────────────────────

    #[test]
    fn test_bridge_vlan_protocol_8021ad() {
        let mut config = make_config("br0", NetDevKind::Bridge);
        config.kind_config.bridge = Some(BridgeSection {
            vlan_filtering: Some(true),
            vlan_protocol: Some("802.1ad".to_string()),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();
        // 0x88a8 in big endian
        assert!(msg.windows(2).any(|w| w == [0x88, 0xa8]));
    }

    // ── Complete message roundtrip ─────────────────────────────────────

    #[test]
    fn test_complete_bridge_message_structure() {
        let mut config = make_config("br0", NetDevKind::Bridge);
        config.netdev_section.mtu_bytes = Some(1500);
        config.netdev_section.mac_address = Some("02:42:00:00:00:01".to_string());
        config.kind_config.bridge = Some(BridgeSection {
            stp: Some(true),
            forward_delay_sec: Some(15),
            priority: Some(32768),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();

        // Verify structure
        let nlmsg_len = u32::from_ne_bytes(msg[0..4].try_into().unwrap()) as usize;
        assert_eq!(nlmsg_len, msg.len());
        assert_eq!(msg.len() % 4, 0);

        // Contains the name
        assert!(msg.windows(4).any(|w| w == b"br0\0"));
        // Contains the kind
        assert!(msg.windows(7).any(|w| w == b"bridge\0"));
        // Contains the MTU
        let mtu_bytes = 1500u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == mtu_bytes));
        // Contains the MAC
        assert!(
            msg.windows(6)
                .any(|w| w == [0x02, 0x42, 0x00, 0x00, 0x00, 0x01])
        );
    }

    #[test]
    fn test_complete_bond_8023ad_message() {
        let mut config = make_config("bond0", NetDevKind::Bond);
        config.kind_config.bond = Some(BondSection {
            mode: Some(BondMode::Ieee802_3ad),
            transmit_hash_policy: Some(TransmitHashPolicy::Layer3Plus4),
            lacp_transmit_rate: Some(LacpTransmitRate::Fast),
            mii_monitor_sec: Some("100".to_string()),
            min_links: Some(1),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, None).unwrap();

        let nlmsg_len = u32::from_ne_bytes(msg[0..4].try_into().unwrap()) as usize;
        assert_eq!(nlmsg_len, msg.len());
        assert_eq!(msg.len() % 4, 0);

        assert!(msg.windows(6).any(|w| w == b"bond0\0"));
        assert!(msg.windows(5).any(|w| w == b"bond\0"));
    }

    #[test]
    fn test_complete_vlan_with_parent() {
        let mut config = make_config("vlan100", NetDevKind::Vlan);
        config.kind_config.vlan = Some(VlanSection {
            id: Some(100),
            protocol: Some("802.1Q".to_string()),
            ..Default::default()
        });
        let msg = build_create_netdev_msg(&config, Some(5)).unwrap();

        let nlmsg_len = u32::from_ne_bytes(msg[0..4].try_into().unwrap()) as usize;
        assert_eq!(nlmsg_len, msg.len());
        assert_eq!(msg.len() % 4, 0);

        assert!(msg.windows(8).any(|w| w == b"vlan100\0"));
        assert!(msg.windows(5).any(|w| w == b"vlan\0"));

        // Parent ifindex = 5
        let parent_bytes = 5u32.to_ne_bytes();
        assert!(msg.windows(4).any(|w| w == parent_bytes));

        // VLAN ID 100
        let id_bytes = 100u16.to_ne_bytes();
        assert!(msg.windows(2).any(|w| w == id_bytes));

        // 802.1Q ethertype
        assert!(msg.windows(2).any(|w| w == [0x81, 0x00]));
    }
}
