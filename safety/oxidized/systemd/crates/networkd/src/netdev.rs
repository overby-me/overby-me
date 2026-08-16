//! Parser for systemd `.netdev` configuration files.
//!
//! Supports the INI-style format used by systemd-networkd for defining
//! virtual network devices. Reference: systemd.netdev(5)
//!
//! Sections:
//! - `[NetDev]`       — general virtual device settings (Name, Kind, MTU, MAC)
//! - `[Bridge]`       — bridge-specific settings
//! - `[Bond]`         — bond-specific settings
//! - `[VLAN]`         — VLAN-specific settings
//! - `[VXLAN]`        — VXLAN-specific settings
//! - `[WireGuard]`    — WireGuard-specific settings
//! - `[WireGuardPeer]`— WireGuard peer entries (may appear multiple times)
//! - `[Tunnel]`       — tunnel-specific settings (GRE, SIT, VTI, IP6TNL, etc.)
//! - `[MACVLAN]`      — MACVLAN-specific settings
//! - `[MACVTAP]`      — MACVTAP-specific settings (same fields as MACVLAN)
//! - `[IPVLAN]`       — IPVLAN-specific settings
//! - `[Tun]`          — TUN device settings
//! - `[Tap]`          — TAP device settings (same fields as Tun)
//! - `[FOU]`          — Foo-over-UDP settings
//! - `[L2TP]`         — L2TPv3 settings
//! - `[L2TPSession]`  — L2TPv3 session entries (may appear multiple times)
//! - `[MACsec]`       — MACsec settings
//! - `[BatmanAdvanced]`— BATMAN Advanced settings
//! - `[Peer]`         — veth peer settings
//! - `[Match]`        — optional match section (Host, Virtualization, etc.)

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Top-level parsed netdev file
// ---------------------------------------------------------------------------

/// A parsed `.netdev` file.
#[derive(Debug, Clone)]
pub struct NetDevConfig {
    /// Original file path (for diagnostics).
    pub path: PathBuf,

    /// `[Match]` section — optional conditions for this netdev.
    pub match_section: NetDevMatchSection,

    /// `[NetDev]` section — general device settings.
    pub netdev_section: NetDevSection,

    /// Kind-specific configuration.
    pub kind_config: KindConfig,
}

// ---------------------------------------------------------------------------
// [Match]
// ---------------------------------------------------------------------------

/// Optional `[Match]` section in a `.netdev` file.
#[derive(Debug, Clone, Default)]
pub struct NetDevMatchSection {
    /// Match by hostname.
    pub host: Option<String>,
    /// Match by virtualization type.
    pub virtualization: Option<String>,
    /// Match by kernel command line.
    pub kernel_command_line: Option<String>,
    /// Match by kernel version.
    pub kernel_version: Option<String>,
    /// Match by architecture.
    pub architecture: Option<String>,
    /// Match by credential.
    pub credential: Option<String>,
}

// ---------------------------------------------------------------------------
// [NetDev]
// ---------------------------------------------------------------------------

/// The `[NetDev]` section — present in every `.netdev` file.
#[derive(Debug, Clone)]
pub struct NetDevSection {
    /// Name of the network device to create.
    pub name: String,
    /// Kind of virtual network device.
    pub kind: NetDevKind,
    /// Human-readable description.
    pub description: Option<String>,
    /// MTU of the device (with K/M/G suffix support).
    pub mtu_bytes: Option<u64>,
    /// Explicit MAC address to set.
    pub mac_address: Option<String>,
}

/// All supported virtual network device kinds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NetDevKind {
    Bridge,
    Bond,
    Vlan,
    Vxlan,
    WireGuard,
    MacVlan,
    MacVtap,
    IpVlan,
    IpVtap,
    Veth,
    Tun,
    Tap,
    Dummy,
    Ifb,
    Gre,
    GreTap,
    Ip6Gre,
    Ip6GreTap,
    Sit,
    Vti,
    Vti6,
    Ip6Tnl,
    Erspan,
    Geneve,
    BareUdp,
    L2tp,
    MACsec,
    Xfrm,
    BatAdv,
    Fou,
    Wlan,
    NLMon,
    Vrf,
    Vcan,
    VxCan,
}

impl NetDevKind {
    /// Parse a kind string (case-insensitive, matching systemd naming).
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bridge" => Some(Self::Bridge),
            "bond" => Some(Self::Bond),
            "vlan" => Some(Self::Vlan),
            "vxlan" => Some(Self::Vxlan),
            "wireguard" => Some(Self::WireGuard),
            "macvlan" => Some(Self::MacVlan),
            "macvtap" => Some(Self::MacVtap),
            "ipvlan" => Some(Self::IpVlan),
            "ipvtap" => Some(Self::IpVtap),
            "veth" => Some(Self::Veth),
            "tun" => Some(Self::Tun),
            "tap" => Some(Self::Tap),
            "dummy" => Some(Self::Dummy),
            "ifb" => Some(Self::Ifb),
            "gre" => Some(Self::Gre),
            "gretap" => Some(Self::GreTap),
            "ip6gre" => Some(Self::Ip6Gre),
            "ip6gretap" => Some(Self::Ip6GreTap),
            "sit" => Some(Self::Sit),
            "vti" => Some(Self::Vti),
            "vti6" => Some(Self::Vti6),
            "ip6tnl" => Some(Self::Ip6Tnl),
            "erspan" => Some(Self::Erspan),
            "geneve" => Some(Self::Geneve),
            "bareudp" => Some(Self::BareUdp),
            "l2tp" => Some(Self::L2tp),
            "macsec" => Some(Self::MACsec),
            "xfrm" => Some(Self::Xfrm),
            "batadv" => Some(Self::BatAdv),
            "fou" => Some(Self::Fou),
            "wlan" => Some(Self::Wlan),
            "nlmon" => Some(Self::NLMon),
            "vrf" => Some(Self::Vrf),
            "vcan" => Some(Self::Vcan),
            "vxcan" => Some(Self::VxCan),
            _ => None,
        }
    }

    /// Canonical string form matching systemd output.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Bond => "bond",
            Self::Vlan => "vlan",
            Self::Vxlan => "vxlan",
            Self::WireGuard => "wireguard",
            Self::MacVlan => "macvlan",
            Self::MacVtap => "macvtap",
            Self::IpVlan => "ipvlan",
            Self::IpVtap => "ipvtap",
            Self::Veth => "veth",
            Self::Tun => "tun",
            Self::Tap => "tap",
            Self::Dummy => "dummy",
            Self::Ifb => "ifb",
            Self::Gre => "gre",
            Self::GreTap => "gretap",
            Self::Ip6Gre => "ip6gre",
            Self::Ip6GreTap => "ip6gretap",
            Self::Sit => "sit",
            Self::Vti => "vti",
            Self::Vti6 => "vti6",
            Self::Ip6Tnl => "ip6tnl",
            Self::Erspan => "erspan",
            Self::Geneve => "geneve",
            Self::BareUdp => "bareudp",
            Self::L2tp => "l2tp",
            Self::MACsec => "macsec",
            Self::Xfrm => "xfrm",
            Self::BatAdv => "batadv",
            Self::Fou => "fou",
            Self::Wlan => "wlan",
            Self::NLMon => "nlmon",
            Self::Vrf => "vrf",
            Self::Vcan => "vcan",
            Self::VxCan => "vxcan",
        }
    }

    /// Returns true if this kind uses the `[Tunnel]` section.
    pub fn is_tunnel(&self) -> bool {
        matches!(
            self,
            Self::Gre
                | Self::GreTap
                | Self::Ip6Gre
                | Self::Ip6GreTap
                | Self::Sit
                | Self::Vti
                | Self::Vti6
                | Self::Ip6Tnl
                | Self::Erspan
        )
    }
}

impl fmt::Display for NetDevKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Kind-specific configuration
// ---------------------------------------------------------------------------

/// Kind-specific configuration extracted from the appropriate section.
#[derive(Debug, Clone, Default)]
pub struct KindConfig {
    pub bridge: Option<BridgeSection>,
    pub bond: Option<BondSection>,
    pub vlan: Option<VlanSection>,
    pub vxlan: Option<VxlanSection>,
    pub wireguard: Option<WireGuardSection>,
    pub wireguard_peers: Vec<WireGuardPeerSection>,
    pub tunnel: Option<TunnelSection>,
    pub macvlan: Option<MacVlanSection>,
    pub ipvlan: Option<IpVlanSection>,
    pub tun: Option<TunTapSection>,
    pub peer: Option<PeerSection>,
    pub fou: Option<FouSection>,
    pub geneve: Option<GeneveSection>,
    pub l2tp: Option<L2tpSection>,
    pub l2tp_sessions: Vec<L2tpSessionSection>,
    pub macsec: Option<MACsecSection>,
    pub batman: Option<BatmanAdvancedSection>,
    pub bare_udp: Option<BareUdpSection>,
    pub xfrm: Option<XfrmSection>,
    pub vrf: Option<VrfSection>,
}

// ---------------------------------------------------------------------------
// [Bridge]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct BridgeSection {
    /// Enable Spanning Tree Protocol.
    pub stp: Option<bool>,
    /// Forward delay in seconds.
    pub forward_delay_sec: Option<u64>,
    /// Hello time in seconds.
    pub hello_time_sec: Option<u64>,
    /// Maximum message age in seconds.
    pub max_age_sec: Option<u64>,
    /// MAC address ageing time in seconds.
    pub ageing_time_sec: Option<u64>,
    /// Bridge priority (0-65535).
    pub priority: Option<u32>,
    /// Group forward mask (bitmask).
    pub group_forward_mask: Option<u16>,
    /// Default port VLAN ID.
    pub default_pvid: Option<u16>,
    /// Enable multicast querier.
    pub multicast_querier: Option<bool>,
    /// Enable multicast snooping.
    pub multicast_snooping: Option<bool>,
    /// Enable VLAN filtering.
    pub vlan_filtering: Option<bool>,
    /// VLAN protocol (802.1Q or 802.1ad).
    pub vlan_protocol: Option<String>,
    /// Multicast IGMP version (2 or 3).
    pub multicast_igmp_version: Option<u8>,
}

// ---------------------------------------------------------------------------
// [Bond]
// ---------------------------------------------------------------------------

/// Bonding mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BondMode {
    BalanceRr,
    ActiveBackup,
    BalanceXor,
    Broadcast,
    Ieee802_3ad,
    BalanceTlb,
    BalanceAlb,
}

impl BondMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "balance_rr" | "balance-rr" | "0" => Some(Self::BalanceRr),
            "active_backup" | "active-backup" | "1" => Some(Self::ActiveBackup),
            "balance_xor" | "balance-xor" | "2" => Some(Self::BalanceXor),
            "broadcast" | "3" => Some(Self::Broadcast),
            "802.3ad" | "4" => Some(Self::Ieee802_3ad),
            "balance_tlb" | "balance-tlb" | "5" => Some(Self::BalanceTlb),
            "balance_alb" | "balance-alb" | "6" => Some(Self::BalanceAlb),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BalanceRr => "balance-rr",
            Self::ActiveBackup => "active-backup",
            Self::BalanceXor => "balance-xor",
            Self::Broadcast => "broadcast",
            Self::Ieee802_3ad => "802.3ad",
            Self::BalanceTlb => "balance-tlb",
            Self::BalanceAlb => "balance-alb",
        }
    }

    pub fn as_num(&self) -> u8 {
        match self {
            Self::BalanceRr => 0,
            Self::ActiveBackup => 1,
            Self::BalanceXor => 2,
            Self::Broadcast => 3,
            Self::Ieee802_3ad => 4,
            Self::BalanceTlb => 5,
            Self::BalanceAlb => 6,
        }
    }
}

impl fmt::Display for BondMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bond transmit hash policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransmitHashPolicy {
    Layer2,
    Layer3Plus4,
    Layer2Plus3,
    Encap2Plus3,
    Encap3Plus4,
    VlanSrcMac,
}

impl TransmitHashPolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s
            .to_lowercase()
            .replace('-', "_")
            .replace('+', "plus")
            .as_str()
        {
            "layer2" => Some(Self::Layer2),
            "layer3plus4" | "layer3+4" => Some(Self::Layer3Plus4),
            "layer2plus3" | "layer2+3" => Some(Self::Layer2Plus3),
            "encap2plus3" | "encap2+3" => Some(Self::Encap2Plus3),
            "encap3plus4" | "encap3+4" => Some(Self::Encap3Plus4),
            "vlan_srcmac" | "vlan+srcmac" | "vlan-srcmac" => Some(Self::VlanSrcMac),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Layer2 => "layer2",
            Self::Layer3Plus4 => "layer3+4",
            Self::Layer2Plus3 => "layer2+3",
            Self::Encap2Plus3 => "encap2+3",
            Self::Encap3Plus4 => "encap3+4",
            Self::VlanSrcMac => "vlan+srcmac",
        }
    }
}

impl fmt::Display for TransmitHashPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// LACP transmit rate.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LacpTransmitRate {
    Slow,
    Fast,
}

impl LacpTransmitRate {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "slow" | "0" => Some(Self::Slow),
            "fast" | "1" => Some(Self::Fast),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Slow => "slow",
            Self::Fast => "fast",
        }
    }
}

impl fmt::Display for LacpTransmitRate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bond ad-select policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdSelect {
    Stable,
    Bandwidth,
    Count,
}

impl AdSelect {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "stable" | "0" => Some(Self::Stable),
            "bandwidth" | "1" => Some(Self::Bandwidth),
            "count" | "2" => Some(Self::Count),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Stable => "stable",
            Self::Bandwidth => "bandwidth",
            Self::Count => "count",
        }
    }
}

impl fmt::Display for AdSelect {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bond fail-over MAC policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FailOverMacPolicy {
    None,
    Active,
    Follow,
}

impl FailOverMacPolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "none" | "0" => Some(Self::None),
            "active" | "1" => Some(Self::Active),
            "follow" | "2" => Some(Self::Follow),
            _ => ::std::option::Option::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Active => "active",
            Self::Follow => "follow",
        }
    }
}

impl fmt::Display for FailOverMacPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bond ARP validate mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpValidate {
    None,
    Active,
    Backup,
    All,
    Filter,
    FilterActive,
    FilterBackup,
}

impl ArpValidate {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "none" | "0" => Some(Self::None),
            "active" | "1" => Some(Self::Active),
            "backup" | "2" => Some(Self::Backup),
            "all" | "3" => Some(Self::All),
            "filter" | "4" => Some(Self::Filter),
            "filter_active" | "5" => Some(Self::FilterActive),
            "filter_backup" | "6" => Some(Self::FilterBackup),
            _ => ::std::option::Option::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Active => "active",
            Self::Backup => "backup",
            Self::All => "all",
            Self::Filter => "filter",
            Self::FilterActive => "filter_active",
            Self::FilterBackup => "filter_backup",
        }
    }
}

impl fmt::Display for ArpValidate {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bond ARP all-targets mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArpAllTargets {
    Any,
    All,
}

impl ArpAllTargets {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "any" | "0" => Some(Self::Any),
            "all" | "1" => Some(Self::All),
            _ => ::std::option::Option::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Any => "any",
            Self::All => "all",
        }
    }
}

impl fmt::Display for ArpAllTargets {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Bond primary reselect policy.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PrimaryReselectPolicy {
    Always,
    Better,
    Failure,
}

impl PrimaryReselectPolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "always" | "0" => Some(Self::Always),
            "better" | "1" => Some(Self::Better),
            "failure" | "2" => Some(Self::Failure),
            _ => ::std::option::Option::None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Always => "always",
            Self::Better => "better",
            Self::Failure => "failure",
        }
    }
}

impl fmt::Display for PrimaryReselectPolicy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct BondSection {
    /// Bonding mode.
    pub mode: Option<BondMode>,
    /// Transmit hash policy for load-balancing modes.
    pub transmit_hash_policy: Option<TransmitHashPolicy>,
    /// LACP rate (for 802.3ad mode).
    pub lacp_transmit_rate: Option<LacpTransmitRate>,
    /// MII link monitoring frequency in milliseconds.
    pub mii_monitor_sec: Option<String>,
    /// Delay before enabling a slave after link up (ms).
    pub up_delay_sec: Option<String>,
    /// Delay before disabling a slave after link down (ms).
    pub down_delay_sec: Option<String>,
    /// ARP monitoring interval in milliseconds.
    pub arp_interval_sec: Option<String>,
    /// ARP monitoring IP targets (space/comma separated).
    pub arp_ip_targets: Vec<String>,
    /// ARP validation mode.
    pub arp_validate: Option<ArpValidate>,
    /// ARP all-targets mode.
    pub arp_all_targets: Option<ArpAllTargets>,
    /// Primary reselect policy.
    pub primary_reselect_policy: Option<PrimaryReselectPolicy>,
    /// Number of IGMP reports to send after failover.
    pub resend_igmp: Option<u32>,
    /// Number of packets per slave for balance-rr.
    pub packets_per_slave: Option<u32>,
    /// Number of gratuitous ARP frames after failover.
    pub gratuitous_arp: Option<u32>,
    /// All-slaves-active mode.
    pub all_slaves_active: Option<bool>,
    /// Minimum number of active links.
    pub min_links: Option<u32>,
    /// Aggregator selection policy.
    pub ad_select: Option<AdSelect>,
    /// Fail-over MAC policy.
    pub fail_over_mac_policy: Option<FailOverMacPolicy>,
    /// Primary slave device name.
    pub primary: Option<String>,
    /// LACP actor system priority.
    pub ad_actor_sys_prio: Option<u16>,
    /// LACP user port key.
    pub ad_user_port_key: Option<u16>,
    /// LACP system MAC.
    pub ad_actor_system: Option<String>,
    /// Peer notification delay after failover.
    pub peer_notify_delay_sec: Option<String>,
    /// Learn packet interval for balance-tlb.
    pub lp_interval: Option<u32>,
}

// ---------------------------------------------------------------------------
// [VLAN]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct VlanSection {
    /// VLAN identifier (0–4094).
    pub id: Option<u16>,
    /// VLAN protocol (802.1Q or 802.1ad).
    pub protocol: Option<String>,
    /// GARP VLAN Registration Protocol.
    pub gvrp: Option<bool>,
    /// Multiple VLAN Registration Protocol.
    pub mvrp: Option<bool>,
    /// Loose binding mode.
    pub loose_binding: Option<bool>,
    /// Reorder header mode.
    pub reorder_header: Option<bool>,
    /// Egress QoS mappings ("FROM:TO" pairs, space-separated).
    pub egress_qos_maps: Vec<String>,
    /// Ingress QoS mappings ("FROM:TO" pairs, space-separated).
    pub ingress_qos_maps: Vec<String>,
}

// ---------------------------------------------------------------------------
// [VXLAN]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct VxlanSection {
    /// VXLAN Network Identifier.
    pub vni: Option<u32>,
    /// Remote endpoint address.
    pub remote: Option<String>,
    /// Multicast group address.
    pub group: Option<String>,
    /// Local source address.
    pub local: Option<String>,
    /// Type of Service byte.
    pub tos: Option<u8>,
    /// Time to Live (0 = inherit from inner packet).
    pub ttl: Option<u8>,
    /// Enable MAC address learning.
    pub mac_learning: Option<bool>,
    /// FDB ageing time in seconds.
    pub fdb_ageing_sec: Option<u64>,
    /// UDP destination port (default 4789).
    pub destination_port: Option<u16>,
    /// Maximum number of FDB entries.
    pub maximum_fdb_entries: Option<u32>,
    /// Accept UDP IPv6 with zero checksum.
    pub udp6_zero_checksum_rx: Option<bool>,
    /// Send UDP IPv6 with zero checksum.
    pub udp6_zero_checksum_tx: Option<bool>,
    /// UDP source port range ("LOW HIGH").
    pub port_range: Option<String>,
    /// IPv6 flow label.
    pub flow_label: Option<u32>,
    /// L2 miss notification.
    pub l2_miss: Option<bool>,
    /// L3 miss notification.
    pub l3_miss: Option<bool>,
    /// Reduce ARP proxy.
    pub reduce_arp_proxy: Option<bool>,
    /// Route short circuit.
    pub route_short_circuit: Option<bool>,
    /// Group policy extension.
    pub group_policy_extension: Option<bool>,
    /// Generic protocol extension.
    pub generic_protocol_extension: Option<bool>,
    /// Independent mode (not associated with an underlying device).
    pub independent: Option<bool>,
}

// ---------------------------------------------------------------------------
// [WireGuard] and [WireGuardPeer]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct WireGuardSection {
    /// Private key (base64).
    pub private_key: Option<String>,
    /// Path to file containing the private key.
    pub private_key_file: Option<String>,
    /// UDP listen port.
    pub listen_port: Option<u16>,
    /// Firewall mark.
    pub fw_mark: Option<u32>,
    /// Route table to add routes to.
    pub route_table: Option<String>,
    /// Route metric.
    pub route_metric: Option<u32>,
}

#[derive(Debug, Clone, Default)]
pub struct WireGuardPeerSection {
    /// Public key (base64).
    pub public_key: Option<String>,
    /// Preshared key (base64).
    pub preshared_key: Option<String>,
    /// Path to file containing the preshared key.
    pub preshared_key_file: Option<String>,
    /// Endpoint address ("HOST:PORT" or "[HOST]:PORT").
    pub endpoint: Option<String>,
    /// Comma-separated list of allowed IP/CIDR ranges.
    pub allowed_ips: Vec<String>,
    /// Persistent keepalive interval in seconds (0 = disabled).
    pub persistent_keepalive: Option<u16>,
    /// Route table for auto-generated routes.
    pub route_table: Option<String>,
    /// Route metric for auto-generated routes.
    pub route_metric: Option<u32>,
}

// ---------------------------------------------------------------------------
// [Tunnel]
// ---------------------------------------------------------------------------

/// Tunnel encapsulation type for FOU.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TunnelEncapsulation {
    FooOverUdp,
    GenericUdpEncapsulation,
}

impl TunnelEncapsulation {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace(['-', '_'], "").as_str() {
            "fooudp" | "foooverudp" => Some(Self::FooOverUdp),
            "gue" | "genericudpencapsulation" => Some(Self::GenericUdpEncapsulation),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::FooOverUdp => "FooOverUDP",
            Self::GenericUdpEncapsulation => "GenericUDPEncapsulation",
        }
    }
}

impl fmt::Display for TunnelEncapsulation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct TunnelSection {
    /// Local endpoint address.
    pub local: Option<String>,
    /// Remote endpoint address.
    pub remote: Option<String>,
    /// Type of Service byte.
    pub tos: Option<u8>,
    /// Time to Live.
    pub ttl: Option<u8>,
    /// Discover Path MTU.
    pub discover_path_mtu: Option<bool>,
    /// IPv6 flow label.
    pub ipv6_flow_label: Option<String>,
    /// Copy DF bit from inner to outer header.
    pub copy_dscp: Option<bool>,
    /// Shared key (for GRE).
    pub key: Option<String>,
    /// Input key (for GRE).
    pub input_key: Option<String>,
    /// Output key (for GRE).
    pub output_key: Option<String>,
    /// SIT tunnel mode (ipip, ip6ip, any).
    pub mode: Option<String>,
    /// Independent tunnel (not bound to underlying device).
    pub independent: Option<bool>,
    /// Allow packets with local remote address.
    pub allow_local_remote: Option<bool>,
    /// Foo-over-UDP encapsulation.
    pub foo_over_udp: Option<bool>,
    /// FOU destination port.
    pub fou_destination_port: Option<u16>,
    /// FOU source port.
    pub fou_source_port: Option<u16>,
    /// Encapsulation mode (FooOverUDP, GenericUDPEncapsulation).
    pub encapsulation: Option<TunnelEncapsulation>,
    /// ERSPAN index (for erspan kind).
    pub erspan_index: Option<u32>,
    /// ERSPAN version (1 or 2).
    pub erspan_version: Option<u8>,
    /// Serialize tunneled packets.
    pub serialize_tunneled_packets: Option<bool>,
    /// Assign a numeric identifier.
    pub assign_to_loopback: Option<bool>,
    /// Enable IPv6 Rapid Deployment (6rd) prefix.
    pub ipv6_rapid_deployment_prefix: Option<String>,
    /// Whether to add the tunnel interface to ISATAP.
    pub isatap: Option<bool>,
}

// ---------------------------------------------------------------------------
// [MACVLAN] / [MACVTAP]
// ---------------------------------------------------------------------------

/// MACVLAN/MACVTAP operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MacVlanMode {
    Private,
    Vepa,
    Bridge,
    Passthru,
    Source,
}

impl MacVlanMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "private" => Some(Self::Private),
            "vepa" => Some(Self::Vepa),
            "bridge" => Some(Self::Bridge),
            "passthru" | "passthrough" => Some(Self::Passthru),
            "source" => Some(Self::Source),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Private => "private",
            Self::Vepa => "vepa",
            Self::Bridge => "bridge",
            Self::Passthru => "passthru",
            Self::Source => "source",
        }
    }
}

impl fmt::Display for MacVlanMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct MacVlanSection {
    /// Operating mode.
    pub mode: Option<MacVlanMode>,
    /// Source MAC address (for source mode filtering).
    pub source_mac_address: Vec<String>,
    /// Broadcast/flood mode.
    pub broadcast_mode: Option<String>,
}

// ---------------------------------------------------------------------------
// [IPVLAN]
// ---------------------------------------------------------------------------

/// IPVLAN operating mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpVlanMode {
    L2,
    L3,
    L3S,
}

impl IpVlanMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_uppercase().as_str() {
            "L2" => Some(Self::L2),
            "L3" => Some(Self::L3),
            "L3S" => Some(Self::L3S),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::L2 => "L2",
            Self::L3 => "L3",
            Self::L3S => "L3S",
        }
    }
}

impl fmt::Display for IpVlanMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// IPVLAN flags.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IpVlanFlags {
    Bridge,
    Private,
    Vepa,
}

impl IpVlanFlags {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "bridge" => Some(Self::Bridge),
            "private" => Some(Self::Private),
            "vepa" => Some(Self::Vepa),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Bridge => "bridge",
            Self::Private => "private",
            Self::Vepa => "vepa",
        }
    }
}

impl fmt::Display for IpVlanFlags {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct IpVlanSection {
    /// Operating mode (L2, L3, L3S).
    pub mode: Option<IpVlanMode>,
    /// Flags (bridge, private, vepa).
    pub flags: Option<IpVlanFlags>,
}

// ---------------------------------------------------------------------------
// [Tun] / [Tap]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct TunTapSection {
    /// Enable multi-queue mode.
    pub multi_queue: Option<bool>,
    /// Include packet info (struct tun_pi).
    pub packet_info: Option<bool>,
    /// Enable virtio-net header.
    pub vnet_header: Option<bool>,
    /// User to own the device (name or UID).
    pub user: Option<String>,
    /// Group to own the device (name or GID).
    pub group: Option<String>,
    /// Keep carrier state when opening.
    pub keep_carrier: Option<bool>,
}

// ---------------------------------------------------------------------------
// [Peer] (for veth)
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct PeerSection {
    /// Name of the peer interface.
    pub name: Option<String>,
    /// MAC address of the peer interface.
    pub mac_address: Option<String>,
}

// ---------------------------------------------------------------------------
// [FOU]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct FouSection {
    /// Encapsulation type.
    pub encapsulation: Option<TunnelEncapsulation>,
    /// Local listening port.
    pub port: Option<u16>,
    /// Remote port.
    pub peer_port: Option<u16>,
    /// Local address.
    pub local: Option<String>,
    /// Peer address.
    pub peer: Option<String>,
    /// Encapsulated protocol number (e.g. 4 for IPIP, 47 for GRE).
    pub protocol: Option<u8>,
}

// ---------------------------------------------------------------------------
// [Geneve]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct GeneveSection {
    /// Virtual Network Identifier.
    pub vni: Option<u32>,
    /// Remote endpoint address.
    pub remote: Option<String>,
    /// Type of Service byte.
    pub tos: Option<u8>,
    /// Time to Live.
    pub ttl: Option<u8>,
    /// UDP destination port (default 6081).
    pub destination_port: Option<u16>,
    /// IPv6 flow label.
    pub flow_label: Option<u32>,
    /// Send UDP IPv6 with zero checksum.
    pub udp6_zero_checksum_tx: Option<bool>,
    /// Accept UDP IPv6 with zero checksum.
    pub udp6_zero_checksum_rx: Option<bool>,
}

// ---------------------------------------------------------------------------
// [L2TP] and [L2TPSession]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct L2tpSection {
    /// Tunnel ID.
    pub tunnel_id: Option<u32>,
    /// Peer tunnel ID.
    pub peer_tunnel_id: Option<u32>,
    /// Remote endpoint address.
    pub remote: Option<String>,
    /// Local source address.
    pub local: Option<String>,
    /// Encapsulation (udp or ip).
    pub encapsulation_type: Option<String>,
    /// UDP source port.
    pub udp_source_port: Option<u16>,
    /// UDP destination port.
    pub udp_destination_port: Option<u16>,
    /// Enable UDP checksum.
    pub udp_checksum: Option<bool>,
    /// Enable UDP 6 zero-checksum TX.
    pub udp6_checksum_tx: Option<bool>,
    /// Enable UDP 6 zero-checksum RX.
    pub udp6_checksum_rx: Option<bool>,
}

#[derive(Debug, Clone, Default)]
pub struct L2tpSessionSection {
    /// Session ID.
    pub session_id: Option<u32>,
    /// Peer session ID.
    pub peer_session_id: Option<u32>,
    /// Name of the created interface.
    pub name: Option<String>,
    /// Layer 2 specific sublayer type.
    pub l2_specific_header: Option<String>,
}

// ---------------------------------------------------------------------------
// [MACsec]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct MACsecSection {
    /// SCI port number.
    pub port: Option<u16>,
    /// Enable encryption.
    pub encrypt: Option<bool>,
}

// ---------------------------------------------------------------------------
// [BatmanAdvanced]
// ---------------------------------------------------------------------------

/// BATMAN gateway mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatmanGatewayMode {
    Off,
    Client,
    Server,
}

impl BatmanGatewayMode {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "off" => Some(Self::Off),
            "client" => Some(Self::Client),
            "server" => Some(Self::Server),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::Client => "client",
            Self::Server => "server",
        }
    }
}

impl fmt::Display for BatmanGatewayMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// BATMAN routing algorithm.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BatmanRoutingAlgorithm {
    BatmanV,
    BatmanIv,
}

impl BatmanRoutingAlgorithm {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "batman_v" | "batman-v" => Some(Self::BatmanV),
            "batman_iv" | "batman-iv" => Some(Self::BatmanIv),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::BatmanV => "batman-v",
            Self::BatmanIv => "batman-iv",
        }
    }
}

impl fmt::Display for BatmanRoutingAlgorithm {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, Default)]
pub struct BatmanAdvancedSection {
    /// Gateway mode.
    pub gateway_mode: Option<BatmanGatewayMode>,
    /// Enable packet aggregation.
    pub aggregation: Option<bool>,
    /// Enable bridge loop avoidance.
    pub bridge_loop_avoidance: Option<bool>,
    /// Enable distributed ARP table.
    pub distributed_arp_table: Option<bool>,
    /// Enable fragmentation.
    pub fragmentation: Option<bool>,
    /// Hop penalty (0-255).
    pub hop_penalty: Option<u8>,
    /// Originator interval in milliseconds.
    pub originator_interval_sec: Option<String>,
    /// Gateway bandwidth (downstream).
    pub gateway_bandwidth_down: Option<String>,
    /// Gateway bandwidth (upstream).
    pub gateway_bandwidth_up: Option<String>,
    /// Routing algorithm.
    pub routing_algorithm: Option<BatmanRoutingAlgorithm>,
}

// ---------------------------------------------------------------------------
// [BareUDP]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct BareUdpSection {
    /// Destination port.
    pub destination_port: Option<u16>,
    /// Encapsulated EtherType.
    pub ether_type: Option<String>,
}

// ---------------------------------------------------------------------------
// [Xfrm]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct XfrmSection {
    /// XFRM interface ID.
    pub interface_id: Option<u32>,
    /// Independent mode.
    pub independent: Option<bool>,
}

// ---------------------------------------------------------------------------
// [VRF]
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Default)]
pub struct VrfSection {
    /// Routing table ID for this VRF.
    pub table: Option<u32>,
}

// ===========================================================================
// Parsing
// ===========================================================================

/// Load all `.netdev` files from the standard search paths.
///
/// Files are read in lexicographic order; earlier directories take priority
/// over later ones (matching systemd-networkd behaviour):
///
/// 1. `/etc/systemd/network/`
/// 2. `/run/systemd/network/`
/// 3. `/usr/lib/systemd/network/`
/// 4. `/lib/systemd/network/`
///
/// Additionally, package-relative paths are searched (for NixOS).
pub fn load_netdev_configs() -> Vec<NetDevConfig> {
    let mut search_dirs = vec![
        PathBuf::from("/etc/systemd/network"),
        PathBuf::from("/run/systemd/network"),
        PathBuf::from("/usr/lib/systemd/network"),
        PathBuf::from("/lib/systemd/network"),
    ];

    // Add package-relative paths for NixOS.
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        for ancestor in parent.ancestors().skip(1) {
            let candidate = ancestor.join("lib/systemd/network");
            if candidate.is_dir() && !search_dirs.contains(&candidate) {
                search_dirs.push(candidate);
                break;
            }
        }
    }

    load_netdev_configs_from(&search_dirs)
}

/// Load `.netdev` files from the given directories, deduplicating by
/// filename (first occurrence wins).
pub fn load_netdev_configs_from(dirs: &[PathBuf]) -> Vec<NetDevConfig> {
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    let mut configs = Vec::new();

    for dir in dirs {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "netdev"))
            .collect();

        files.sort_by_key(|e| e.file_name());

        for entry in files {
            let name = entry.file_name().to_string_lossy().to_string();
            if seen.contains_key(&name) {
                continue;
            }
            let path = entry.path();
            seen.insert(name, path.clone());

            match parse_netdev_file(&path) {
                Ok(cfg) => configs.push(cfg),
                Err(e) => {
                    log::warn!("Failed to parse {}: {}", path.display(), e);
                }
            }
        }
    }

    // Sort by filename for deterministic ordering.
    configs.sort_by(|a, b| {
        let a_name = a.path.file_name().unwrap_or_default();
        let b_name = b.path.file_name().unwrap_or_default();
        a_name.cmp(b_name)
    });

    configs
}

/// Parse a single `.netdev` file from disk.
pub fn parse_netdev_file(path: &Path) -> Result<NetDevConfig, String> {
    let content = fs::read_to_string(path).map_err(|e| format!("{}: {}", path.display(), e))?;
    parse_netdev_content(&content, path)
}

/// Parse `.netdev` file content (the text body, not the file path).
pub fn parse_netdev_content(content: &str, path: &Path) -> Result<NetDevConfig, String> {
    let mut match_section = NetDevMatchSection::default();
    let mut netdev_name: Option<String> = None;
    let mut netdev_kind: Option<NetDevKind> = None;
    let mut netdev_description: Option<String> = None;
    let mut netdev_mtu: Option<u64> = None;
    let mut netdev_mac: Option<String> = None;

    let mut kind_config = KindConfig::default();

    // Track which section we're in.
    #[derive(Clone, Copy, PartialEq)]
    enum Section {
        None,
        Match,
        NetDev,
        Bridge,
        Bond,
        Vlan,
        Vxlan,
        WireGuard,
        WireGuardPeer,
        Tunnel,
        MacVlan,
        MacVtap,
        IpVlan,
        TunTap,
        Peer,
        Fou,
        Geneve,
        L2tp,
        L2tpSession,
        MACsec,
        BatmanAdvanced,
        BareUdp,
        Xfrm,
        Vrf,
        Unknown,
    }

    let mut current_section = Section::None;

    // Lazy-initialize kind-specific sections as we encounter them.
    let mut bridge = BridgeSection::default();
    let mut bond = BondSection::default();
    let mut vlan = VlanSection::default();
    let mut vxlan = VxlanSection::default();
    let mut wireguard = WireGuardSection::default();
    let mut wireguard_peers: Vec<WireGuardPeerSection> = Vec::new();
    let mut current_wg_peer: Option<WireGuardPeerSection> = None;
    let mut tunnel = TunnelSection::default();
    let mut macvlan = MacVlanSection::default();
    let mut ipvlan = IpVlanSection::default();
    let mut tuntap = TunTapSection::default();
    let mut peer = PeerSection::default();
    let mut fou = FouSection::default();
    let mut geneve = GeneveSection::default();
    let mut l2tp = L2tpSection::default();
    let mut l2tp_sessions: Vec<L2tpSessionSection> = Vec::new();
    let mut current_l2tp_session: Option<L2tpSessionSection> = None;
    let mut macsec = MACsecSection::default();
    let mut batman = BatmanAdvancedSection::default();
    let mut bare_udp = BareUdpSection::default();
    let mut xfrm = XfrmSection::default();
    let mut vrf = VrfSection::default();

    // Track which sections were actually used (had a header).
    let mut saw_bridge = false;
    let mut saw_bond = false;
    let mut saw_vlan = false;
    let mut saw_vxlan = false;
    let mut saw_wireguard = false;
    let mut saw_tunnel = false;
    let mut saw_macvlan = false;
    let mut saw_macvtap = false;
    let mut saw_ipvlan = false;
    let mut saw_tuntap = false;
    let mut saw_peer = false;
    let mut saw_fou = false;
    let mut saw_geneve = false;
    let mut saw_l2tp = false;
    let mut saw_macsec = false;
    let mut saw_batman = false;
    let mut saw_bare_udp = false;
    let mut saw_xfrm = false;
    let mut saw_vrf = false;

    for raw_line in content.lines() {
        let line = raw_line.trim();

        // Skip blank lines and comments.
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header.
        if line.starts_with('[') && line.ends_with(']') {
            let section_name = &line[1..line.len() - 1];

            // Flush any pending WireGuardPeer section.
            if current_section == Section::WireGuardPeer
                && let Some(p) = current_wg_peer.take()
            {
                wireguard_peers.push(p);
            }

            // Flush any pending L2TPSession section.
            if current_section == Section::L2tpSession
                && let Some(s) = current_l2tp_session.take()
            {
                l2tp_sessions.push(s);
            }

            current_section = match section_name {
                "Match" => Section::Match,
                "NetDev" => Section::NetDev,
                "Bridge" => {
                    saw_bridge = true;
                    Section::Bridge
                }
                "Bond" => {
                    saw_bond = true;
                    Section::Bond
                }
                "VLAN" => {
                    saw_vlan = true;
                    Section::Vlan
                }
                "VXLAN" => {
                    saw_vxlan = true;
                    Section::Vxlan
                }
                "WireGuard" => {
                    saw_wireguard = true;
                    Section::WireGuard
                }
                "WireGuardPeer" => {
                    saw_wireguard = true;
                    current_wg_peer = Some(WireGuardPeerSection::default());
                    Section::WireGuardPeer
                }
                "Tunnel" => {
                    saw_tunnel = true;
                    Section::Tunnel
                }
                "MACVLAN" => {
                    saw_macvlan = true;
                    Section::MacVlan
                }
                "MACVTAP" => {
                    saw_macvtap = true;
                    Section::MacVtap
                }
                "IPVLAN" | "IPVTAP" => {
                    saw_ipvlan = true;
                    Section::IpVlan
                }
                "Tun" | "Tap" => {
                    saw_tuntap = true;
                    Section::TunTap
                }
                "Peer" => {
                    saw_peer = true;
                    Section::Peer
                }
                "FOU" => {
                    saw_fou = true;
                    Section::Fou
                }
                "Geneve" => {
                    saw_geneve = true;
                    Section::Geneve
                }
                "L2TP" => {
                    saw_l2tp = true;
                    Section::L2tp
                }
                "L2TPSession" => {
                    saw_l2tp = true;
                    current_l2tp_session = Some(L2tpSessionSection::default());
                    Section::L2tpSession
                }
                "MACsec" => {
                    saw_macsec = true;
                    Section::MACsec
                }
                "BatmanAdvanced" => {
                    saw_batman = true;
                    Section::BatmanAdvanced
                }
                "BareUDP" => {
                    saw_bare_udp = true;
                    Section::BareUdp
                }
                "XFRM" => {
                    saw_xfrm = true;
                    Section::Xfrm
                }
                "VRF" => {
                    saw_vrf = true;
                    Section::Vrf
                }
                s if s.starts_with("X-") => Section::Unknown,
                _ => {
                    log::debug!(
                        "{}: ignoring unknown section [{}]",
                        path.display(),
                        section_name
                    );
                    Section::Unknown
                }
            };

            continue;
        }

        // Key=value pair.
        if current_section == Section::Unknown || current_section == Section::None {
            continue;
        }

        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        match current_section {
            Section::Match => parse_match_entry(&mut match_section, key, value),
            Section::NetDev => {
                parse_netdev_entry(
                    key,
                    value,
                    &mut netdev_name,
                    &mut netdev_kind,
                    &mut netdev_description,
                    &mut netdev_mtu,
                    &mut netdev_mac,
                    path,
                );
            }
            Section::Bridge => parse_bridge_entry(&mut bridge, key, value),
            Section::Bond => parse_bond_entry(&mut bond, key, value),
            Section::Vlan => parse_vlan_entry(&mut vlan, key, value),
            Section::Vxlan => parse_vxlan_entry(&mut vxlan, key, value),
            Section::WireGuard => parse_wireguard_entry(&mut wireguard, key, value),
            Section::WireGuardPeer => {
                if let Some(ref mut p) = current_wg_peer {
                    parse_wireguard_peer_entry(p, key, value);
                }
            }
            Section::Tunnel => parse_tunnel_entry(&mut tunnel, key, value),
            Section::MacVlan | Section::MacVtap => parse_macvlan_entry(&mut macvlan, key, value),
            Section::IpVlan => parse_ipvlan_entry(&mut ipvlan, key, value),
            Section::TunTap => parse_tuntap_entry(&mut tuntap, key, value),
            Section::Peer => parse_peer_entry(&mut peer, key, value),
            Section::Fou => parse_fou_entry(&mut fou, key, value),
            Section::Geneve => parse_geneve_entry(&mut geneve, key, value),
            Section::L2tp => parse_l2tp_entry(&mut l2tp, key, value),
            Section::L2tpSession => {
                if let Some(ref mut s) = current_l2tp_session {
                    parse_l2tp_session_entry(s, key, value);
                }
            }
            Section::MACsec => parse_macsec_entry(&mut macsec, key, value),
            Section::BatmanAdvanced => parse_batman_entry(&mut batman, key, value),
            Section::BareUdp => parse_bare_udp_entry(&mut bare_udp, key, value),
            Section::Xfrm => parse_xfrm_entry(&mut xfrm, key, value),
            Section::Vrf => parse_vrf_entry(&mut vrf, key, value),
            Section::None | Section::Unknown => {}
        }
    }

    // Flush trailing WireGuardPeer / L2TPSession.
    if let Some(p) = current_wg_peer.take() {
        wireguard_peers.push(p);
    }
    if let Some(s) = current_l2tp_session.take() {
        l2tp_sessions.push(s);
    }

    // Validate required fields.
    let name =
        netdev_name.ok_or_else(|| format!("{}: missing Name= in [NetDev]", path.display()))?;
    let kind =
        netdev_kind.ok_or_else(|| format!("{}: missing Kind= in [NetDev]", path.display()))?;

    // Populate kind_config with sections that were actually present.
    if saw_bridge {
        kind_config.bridge = Some(bridge);
    }
    if saw_bond {
        kind_config.bond = Some(bond);
    }
    if saw_vlan {
        kind_config.vlan = Some(vlan);
    }
    if saw_vxlan {
        kind_config.vxlan = Some(vxlan);
    }
    if saw_wireguard {
        kind_config.wireguard = Some(wireguard);
        kind_config.wireguard_peers = wireguard_peers;
    }
    if saw_tunnel {
        kind_config.tunnel = Some(tunnel);
    }
    if saw_macvlan || saw_macvtap {
        kind_config.macvlan = Some(macvlan);
    }
    if saw_ipvlan {
        kind_config.ipvlan = Some(ipvlan);
    }
    if saw_tuntap {
        kind_config.tun = Some(tuntap);
    }
    if saw_peer {
        kind_config.peer = Some(peer);
    }
    if saw_fou {
        kind_config.fou = Some(fou);
    }
    if saw_geneve {
        kind_config.geneve = Some(geneve);
    }
    if saw_l2tp {
        kind_config.l2tp = Some(l2tp);
        kind_config.l2tp_sessions = l2tp_sessions;
    }
    if saw_macsec {
        kind_config.macsec = Some(macsec);
    }
    if saw_batman {
        kind_config.batman = Some(batman);
    }
    if saw_bare_udp {
        kind_config.bare_udp = Some(bare_udp);
    }
    if saw_xfrm {
        kind_config.xfrm = Some(xfrm);
    }
    if saw_vrf {
        kind_config.vrf = Some(vrf);
    }

    Ok(NetDevConfig {
        path: path.to_path_buf(),
        match_section,
        netdev_section: NetDevSection {
            name,
            kind,
            description: netdev_description,
            mtu_bytes: netdev_mtu,
            mac_address: netdev_mac,
        },
        kind_config,
    })
}

// ---------------------------------------------------------------------------
// Per-section entry parsers
// ---------------------------------------------------------------------------

fn parse_match_entry(section: &mut NetDevMatchSection, key: &str, value: &str) {
    match key {
        "Host" => section.host = Some(value.to_string()),
        "Virtualization" => section.virtualization = Some(value.to_string()),
        "KernelCommandLine" => section.kernel_command_line = Some(value.to_string()),
        "KernelVersion" => section.kernel_version = Some(value.to_string()),
        "Architecture" => section.architecture = Some(value.to_string()),
        "Credential" => section.credential = Some(value.to_string()),
        _ => {
            log::trace!("[Match] ignoring unknown key: {}={}", key, value);
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn parse_netdev_entry(
    key: &str,
    value: &str,
    name: &mut Option<String>,
    kind: &mut Option<NetDevKind>,
    description: &mut Option<String>,
    mtu: &mut Option<u64>,
    mac: &mut Option<String>,
    path: &Path,
) {
    match key {
        "Name" => *name = Some(value.to_string()),
        "Kind" => match NetDevKind::parse(value) {
            Some(k) => *kind = Some(k),
            None => log::warn!("{}: unknown Kind={}", path.display(), value),
        },
        "Description" => *description = Some(value.to_string()),
        "MTUBytes" => *mtu = parse_bytes_value(value),
        "MACAddress" => *mac = Some(value.to_string()),
        _ => {
            log::trace!("[NetDev] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_bridge_entry(section: &mut BridgeSection, key: &str, value: &str) {
    match key {
        "STP" => section.stp = parse_bool(value),
        "ForwardDelaySec" => section.forward_delay_sec = parse_seconds(value),
        "HelloTimeSec" => section.hello_time_sec = parse_seconds(value),
        "MaxAgeSec" => section.max_age_sec = parse_seconds(value),
        "AgeingTimeSec" => section.ageing_time_sec = parse_seconds(value),
        "Priority" => section.priority = value.parse().ok(),
        "GroupForwardMask" => section.group_forward_mask = parse_u16(value),
        "DefaultPVID" => section.default_pvid = parse_u16(value),
        "MulticastQuerier" => section.multicast_querier = parse_bool(value),
        "MulticastSnooping" => section.multicast_snooping = parse_bool(value),
        "VLANFiltering" => section.vlan_filtering = parse_bool(value),
        "VLANProtocol" => section.vlan_protocol = Some(value.to_string()),
        "MulticastIGMPVersion" => section.multicast_igmp_version = value.parse().ok(),
        _ => {
            log::trace!("[Bridge] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_bond_entry(section: &mut BondSection, key: &str, value: &str) {
    match key {
        "Mode" => section.mode = BondMode::parse(value),
        "TransmitHashPolicy" => section.transmit_hash_policy = TransmitHashPolicy::parse(value),
        "LACPTransmitRate" => section.lacp_transmit_rate = LacpTransmitRate::parse(value),
        "MIIMonitorSec" => section.mii_monitor_sec = Some(value.to_string()),
        "UpDelaySec" => section.up_delay_sec = Some(value.to_string()),
        "DownDelaySec" => section.down_delay_sec = Some(value.to_string()),
        "ArpIntervalSec" => section.arp_interval_sec = Some(value.to_string()),
        "ArpIpTargets" => {
            for target in value.split_whitespace() {
                for t in target.split(',') {
                    let t = t.trim();
                    if !t.is_empty() {
                        section.arp_ip_targets.push(t.to_string());
                    }
                }
            }
        }
        "ArpValidate" => section.arp_validate = ArpValidate::parse(value),
        "ArpAllTargets" => section.arp_all_targets = ArpAllTargets::parse(value),
        "PrimaryReselectPolicy" => {
            section.primary_reselect_policy = PrimaryReselectPolicy::parse(value)
        }
        "ResendIGMP" => section.resend_igmp = value.parse().ok(),
        "PacketsPerSlave" => section.packets_per_slave = value.parse().ok(),
        "GratuitousARP" | "GratuitousArp" => section.gratuitous_arp = value.parse().ok(),
        "AllSlavesActive" => section.all_slaves_active = parse_bool(value),
        "MinLinks" => section.min_links = value.parse().ok(),
        "AdSelect" => section.ad_select = AdSelect::parse(value),
        "FailOverMACPolicy" => section.fail_over_mac_policy = FailOverMacPolicy::parse(value),
        "Primary" => section.primary = Some(value.to_string()),
        "AdActorSystemPriority" => section.ad_actor_sys_prio = parse_u16(value),
        "AdUserPortKey" => section.ad_user_port_key = parse_u16(value),
        "AdActorSystem" => section.ad_actor_system = Some(value.to_string()),
        "PeerNotifyDelaySec" => section.peer_notify_delay_sec = Some(value.to_string()),
        "LearnPacketIntervalSec" | "LPInterval" => section.lp_interval = value.parse().ok(),
        _ => {
            log::trace!("[Bond] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_vlan_entry(section: &mut VlanSection, key: &str, value: &str) {
    match key {
        "Id" => section.id = parse_u16(value),
        "Protocol" => section.protocol = Some(value.to_string()),
        "GVRP" => section.gvrp = parse_bool(value),
        "MVRP" => section.mvrp = parse_bool(value),
        "LooseBinding" => section.loose_binding = parse_bool(value),
        "ReorderHeader" => section.reorder_header = parse_bool(value),
        "EgressQOSMaps" => {
            for mapping in value.split_whitespace() {
                let m = mapping.trim();
                if !m.is_empty() {
                    section.egress_qos_maps.push(m.to_string());
                }
            }
        }
        "IngressQOSMaps" => {
            for mapping in value.split_whitespace() {
                let m = mapping.trim();
                if !m.is_empty() {
                    section.ingress_qos_maps.push(m.to_string());
                }
            }
        }
        _ => {
            log::trace!("[VLAN] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_vxlan_entry(section: &mut VxlanSection, key: &str, value: &str) {
    match key {
        "VNI" | "Id" => section.vni = value.parse().ok(),
        "Remote" => section.remote = Some(value.to_string()),
        "Group" => section.group = Some(value.to_string()),
        "Local" => section.local = Some(value.to_string()),
        "TOS" => section.tos = parse_u8(value),
        "TTL" => section.ttl = parse_u8(value),
        "MacLearning" => section.mac_learning = parse_bool(value),
        "FDBAgeingSec" => section.fdb_ageing_sec = parse_seconds(value),
        "DestinationPort" => section.destination_port = parse_u16(value),
        "MaximumFDBEntries" => section.maximum_fdb_entries = value.parse().ok(),
        "UDP6ZeroChecksumRx" => section.udp6_zero_checksum_rx = parse_bool(value),
        "UDP6ZeroChecksumTx" => section.udp6_zero_checksum_tx = parse_bool(value),
        "PortRange" => section.port_range = Some(value.to_string()),
        "FlowLabel" => section.flow_label = value.parse().ok(),
        "L2MissNotification" => section.l2_miss = parse_bool(value),
        "L3MissNotification" => section.l3_miss = parse_bool(value),
        "ReduceARPProxy" => section.reduce_arp_proxy = parse_bool(value),
        "RouteShortCircuit" => section.route_short_circuit = parse_bool(value),
        "GroupPolicyExtension" => section.group_policy_extension = parse_bool(value),
        "GenericProtocolExtension" => section.generic_protocol_extension = parse_bool(value),
        "Independent" => section.independent = parse_bool(value),
        _ => {
            log::trace!("[VXLAN] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_wireguard_entry(section: &mut WireGuardSection, key: &str, value: &str) {
    match key {
        "PrivateKey" => section.private_key = Some(value.to_string()),
        "PrivateKeyFile" => section.private_key_file = Some(value.to_string()),
        "ListenPort" => section.listen_port = parse_u16(value),
        "FirewallMark" | "FwMark" => section.fw_mark = value.parse().ok(),
        "RouteTable" => section.route_table = Some(value.to_string()),
        "RouteMetric" => section.route_metric = value.parse().ok(),
        _ => {
            log::trace!("[WireGuard] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_wireguard_peer_entry(section: &mut WireGuardPeerSection, key: &str, value: &str) {
    match key {
        "PublicKey" => section.public_key = Some(value.to_string()),
        "PresharedKey" => section.preshared_key = Some(value.to_string()),
        "PresharedKeyFile" => section.preshared_key_file = Some(value.to_string()),
        "Endpoint" => section.endpoint = Some(value.to_string()),
        "AllowedIPs" => {
            for ip in value.split(',') {
                let ip = ip.trim();
                if !ip.is_empty() {
                    section.allowed_ips.push(ip.to_string());
                }
            }
        }
        "PersistentKeepalive" => section.persistent_keepalive = parse_u16(value),
        "RouteTable" => section.route_table = Some(value.to_string()),
        "RouteMetric" => section.route_metric = value.parse().ok(),
        _ => {
            log::trace!("[WireGuardPeer] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_tunnel_entry(section: &mut TunnelSection, key: &str, value: &str) {
    match key {
        "Local" => section.local = Some(value.to_string()),
        "Remote" => section.remote = Some(value.to_string()),
        "TOS" => section.tos = parse_u8(value),
        "TTL" => section.ttl = parse_u8(value),
        "DiscoverPathMTU" => section.discover_path_mtu = parse_bool(value),
        "IPv6FlowLabel" => section.ipv6_flow_label = Some(value.to_string()),
        "CopyDSCP" => section.copy_dscp = parse_bool(value),
        "Key" => section.key = Some(value.to_string()),
        "InputKey" => section.input_key = Some(value.to_string()),
        "OutputKey" => section.output_key = Some(value.to_string()),
        "Mode" => section.mode = Some(value.to_string()),
        "Independent" => section.independent = parse_bool(value),
        "AllowLocalRemote" => section.allow_local_remote = parse_bool(value),
        "FooOverUDP" => section.foo_over_udp = parse_bool(value),
        "FOUDestinationPort" => section.fou_destination_port = parse_u16(value),
        "FOUSourcePort" => section.fou_source_port = parse_u16(value),
        "Encapsulation" => section.encapsulation = TunnelEncapsulation::parse(value),
        "ERSPANIndex" => section.erspan_index = value.parse().ok(),
        "ERSPANVersion" => section.erspan_version = parse_u8(value),
        "SerializeTunneledPackets" => section.serialize_tunneled_packets = parse_bool(value),
        "AssignToLoopback" => section.assign_to_loopback = parse_bool(value),
        "IPv6RapidDeploymentPrefix" | "6rd-prefix" => {
            section.ipv6_rapid_deployment_prefix = Some(value.to_string())
        }
        "ISATAP" => section.isatap = parse_bool(value),
        _ => {
            log::trace!("[Tunnel] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_macvlan_entry(section: &mut MacVlanSection, key: &str, value: &str) {
    match key {
        "Mode" => section.mode = MacVlanMode::parse(value),
        "SourceMACAddress" => {
            for mac in value.split_whitespace() {
                let m = mac.trim();
                if !m.is_empty() {
                    section.source_mac_address.push(m.to_string());
                }
            }
        }
        "BroadcastMode" => section.broadcast_mode = Some(value.to_string()),
        _ => {
            log::trace!("[MACVLAN] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_ipvlan_entry(section: &mut IpVlanSection, key: &str, value: &str) {
    match key {
        "Mode" => section.mode = IpVlanMode::parse(value),
        "Flags" => section.flags = IpVlanFlags::parse(value),
        _ => {
            log::trace!("[IPVLAN] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_tuntap_entry(section: &mut TunTapSection, key: &str, value: &str) {
    match key {
        "MultiQueue" => section.multi_queue = parse_bool(value),
        "PacketInfo" => section.packet_info = parse_bool(value),
        "VNetHeader" => section.vnet_header = parse_bool(value),
        "User" => section.user = Some(value.to_string()),
        "Group" => section.group = Some(value.to_string()),
        "KeepCarrier" => section.keep_carrier = parse_bool(value),
        _ => {
            log::trace!("[Tun/Tap] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_peer_entry(section: &mut PeerSection, key: &str, value: &str) {
    match key {
        "Name" => section.name = Some(value.to_string()),
        "MACAddress" => section.mac_address = Some(value.to_string()),
        _ => {
            log::trace!("[Peer] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_fou_entry(section: &mut FouSection, key: &str, value: &str) {
    match key {
        "Encapsulation" => section.encapsulation = TunnelEncapsulation::parse(value),
        "Port" => section.port = parse_u16(value),
        "PeerPort" => section.peer_port = parse_u16(value),
        "Local" => section.local = Some(value.to_string()),
        "Peer" => section.peer = Some(value.to_string()),
        "Protocol" => section.protocol = parse_u8(value),
        _ => {
            log::trace!("[FOU] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_geneve_entry(section: &mut GeneveSection, key: &str, value: &str) {
    match key {
        "VNI" | "Id" => section.vni = value.parse().ok(),
        "Remote" => section.remote = Some(value.to_string()),
        "TOS" => section.tos = parse_u8(value),
        "TTL" => section.ttl = parse_u8(value),
        "DestinationPort" => section.destination_port = parse_u16(value),
        "FlowLabel" => section.flow_label = value.parse().ok(),
        "UDP6ZeroChecksumTx" => section.udp6_zero_checksum_tx = parse_bool(value),
        "UDP6ZeroChecksumRx" => section.udp6_zero_checksum_rx = parse_bool(value),
        _ => {
            log::trace!("[Geneve] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_l2tp_entry(section: &mut L2tpSection, key: &str, value: &str) {
    match key {
        "TunnelId" => section.tunnel_id = value.parse().ok(),
        "PeerTunnelId" => section.peer_tunnel_id = value.parse().ok(),
        "Remote" => section.remote = Some(value.to_string()),
        "Local" => section.local = Some(value.to_string()),
        "EncapsulationType" => section.encapsulation_type = Some(value.to_string()),
        "UDPSourcePort" => section.udp_source_port = parse_u16(value),
        "UDPDestinationPort" | "DestinationPort" => section.udp_destination_port = parse_u16(value),
        "UDPChecksum" => section.udp_checksum = parse_bool(value),
        "UDP6ZeroChecksumTx" => section.udp6_checksum_tx = parse_bool(value),
        "UDP6ZeroChecksumRx" => section.udp6_checksum_rx = parse_bool(value),
        _ => {
            log::trace!("[L2TP] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_l2tp_session_entry(section: &mut L2tpSessionSection, key: &str, value: &str) {
    match key {
        "SessionId" => section.session_id = value.parse().ok(),
        "PeerSessionId" => section.peer_session_id = value.parse().ok(),
        "Name" => section.name = Some(value.to_string()),
        "L2SpecificHeader" => section.l2_specific_header = Some(value.to_string()),
        _ => {
            log::trace!("[L2TPSession] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_macsec_entry(section: &mut MACsecSection, key: &str, value: &str) {
    match key {
        "Port" => section.port = parse_u16(value),
        "Encrypt" => section.encrypt = parse_bool(value),
        _ => {
            log::trace!("[MACsec] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_batman_entry(section: &mut BatmanAdvancedSection, key: &str, value: &str) {
    match key {
        "GatewayMode" => section.gateway_mode = BatmanGatewayMode::parse(value),
        "Aggregation" => section.aggregation = parse_bool(value),
        "BridgeLoopAvoidance" => section.bridge_loop_avoidance = parse_bool(value),
        "DistributedArpTable" => section.distributed_arp_table = parse_bool(value),
        "Fragmentation" => section.fragmentation = parse_bool(value),
        "HopPenalty" => section.hop_penalty = parse_u8(value),
        "OriginatorIntervalSec" => section.originator_interval_sec = Some(value.to_string()),
        "GatewayBandwidthDown" => section.gateway_bandwidth_down = Some(value.to_string()),
        "GatewayBandwidthUp" => section.gateway_bandwidth_up = Some(value.to_string()),
        "RoutingAlgorithm" => section.routing_algorithm = BatmanRoutingAlgorithm::parse(value),
        _ => {
            log::trace!("[BatmanAdvanced] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_bare_udp_entry(section: &mut BareUdpSection, key: &str, value: &str) {
    match key {
        "DestinationPort" => section.destination_port = parse_u16(value),
        "EtherType" => section.ether_type = Some(value.to_string()),
        _ => {
            log::trace!("[BareUDP] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_xfrm_entry(section: &mut XfrmSection, key: &str, value: &str) {
    match key {
        "InterfaceId" => section.interface_id = value.parse().ok(),
        "Independent" => section.independent = parse_bool(value),
        _ => {
            log::trace!("[XFRM] ignoring unknown key: {}={}", key, value);
        }
    }
}

fn parse_vrf_entry(section: &mut VrfSection, key: &str, value: &str) {
    match key {
        "Table" | "TableId" => section.table = value.parse().ok(),
        _ => {
            log::trace!("[VRF] ignoring unknown key: {}={}", key, value);
        }
    }
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Parse a boolean value (case-insensitive, matching systemd).
fn parse_bool(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "yes" | "true" | "on" | "1" => Some(true),
        "no" | "false" | "off" | "0" => Some(false),
        _ => None,
    }
}

/// Parse a u8 value, supporting decimal and `0x` hex prefix.
fn parse_u8(s: &str) -> Option<u8> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u8::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Parse a u16 value, supporting decimal and `0x` hex prefix.
fn parse_u16(s: &str) -> Option<u16> {
    let s = s.trim();
    if let Some(hex) = s.strip_prefix("0x").or_else(|| s.strip_prefix("0X")) {
        u16::from_str_radix(hex, 16).ok()
    } else {
        s.parse().ok()
    }
}

/// Parse a byte size value with optional K/M/G suffix (matching systemd).
fn parse_bytes_value(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Try stripping suffixes in order from longest to shortest.
    let suffixes: &[(&str, u64)] = &[("G", 1024 * 1024 * 1024), ("M", 1024 * 1024), ("K", 1024)];

    for &(suffix, multiplier) in suffixes {
        if let Some(num_str) = s.strip_suffix(suffix) {
            return num_str.trim().parse::<u64>().ok().map(|n| n * multiplier);
        }
        // Also accept lowercase.
        if let Some(num_str) = s.strip_suffix(&suffix.to_lowercase()) {
            return num_str.trim().parse::<u64>().ok().map(|n| n * multiplier);
        }
    }

    // No suffix — plain byte count.
    s.parse().ok()
}

/// Parse a value as seconds. Supports bare numbers (interpreted as seconds)
/// and the "sec"/"s" suffixes. Returns Some(seconds).
fn parse_seconds(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    // Strip known suffixes.
    let num_str = s
        .strip_suffix("sec")
        .or_else(|| s.strip_suffix("s"))
        .unwrap_or(s)
        .trim();

    num_str.parse().ok()
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn test_path() -> PathBuf {
        PathBuf::from("/test/25-bridge.netdev")
    }

    // ── NetDevKind ─────────────────────────────────────────────────────

    #[test]
    fn test_kind_parse_all() {
        assert_eq!(NetDevKind::parse("bridge"), Some(NetDevKind::Bridge));
        assert_eq!(NetDevKind::parse("bond"), Some(NetDevKind::Bond));
        assert_eq!(NetDevKind::parse("vlan"), Some(NetDevKind::Vlan));
        assert_eq!(NetDevKind::parse("vxlan"), Some(NetDevKind::Vxlan));
        assert_eq!(NetDevKind::parse("wireguard"), Some(NetDevKind::WireGuard));
        assert_eq!(NetDevKind::parse("macvlan"), Some(NetDevKind::MacVlan));
        assert_eq!(NetDevKind::parse("macvtap"), Some(NetDevKind::MacVtap));
        assert_eq!(NetDevKind::parse("ipvlan"), Some(NetDevKind::IpVlan));
        assert_eq!(NetDevKind::parse("ipvtap"), Some(NetDevKind::IpVtap));
        assert_eq!(NetDevKind::parse("veth"), Some(NetDevKind::Veth));
        assert_eq!(NetDevKind::parse("tun"), Some(NetDevKind::Tun));
        assert_eq!(NetDevKind::parse("tap"), Some(NetDevKind::Tap));
        assert_eq!(NetDevKind::parse("dummy"), Some(NetDevKind::Dummy));
        assert_eq!(NetDevKind::parse("ifb"), Some(NetDevKind::Ifb));
        assert_eq!(NetDevKind::parse("gre"), Some(NetDevKind::Gre));
        assert_eq!(NetDevKind::parse("gretap"), Some(NetDevKind::GreTap));
        assert_eq!(NetDevKind::parse("ip6gre"), Some(NetDevKind::Ip6Gre));
        assert_eq!(NetDevKind::parse("ip6gretap"), Some(NetDevKind::Ip6GreTap));
        assert_eq!(NetDevKind::parse("sit"), Some(NetDevKind::Sit));
        assert_eq!(NetDevKind::parse("vti"), Some(NetDevKind::Vti));
        assert_eq!(NetDevKind::parse("vti6"), Some(NetDevKind::Vti6));
        assert_eq!(NetDevKind::parse("ip6tnl"), Some(NetDevKind::Ip6Tnl));
        assert_eq!(NetDevKind::parse("erspan"), Some(NetDevKind::Erspan));
        assert_eq!(NetDevKind::parse("geneve"), Some(NetDevKind::Geneve));
        assert_eq!(NetDevKind::parse("bareudp"), Some(NetDevKind::BareUdp));
        assert_eq!(NetDevKind::parse("l2tp"), Some(NetDevKind::L2tp));
        assert_eq!(NetDevKind::parse("macsec"), Some(NetDevKind::MACsec));
        assert_eq!(NetDevKind::parse("xfrm"), Some(NetDevKind::Xfrm));
        assert_eq!(NetDevKind::parse("batadv"), Some(NetDevKind::BatAdv));
        assert_eq!(NetDevKind::parse("fou"), Some(NetDevKind::Fou));
        assert_eq!(NetDevKind::parse("wlan"), Some(NetDevKind::Wlan));
        assert_eq!(NetDevKind::parse("nlmon"), Some(NetDevKind::NLMon));
        assert_eq!(NetDevKind::parse("vrf"), Some(NetDevKind::Vrf));
        assert_eq!(NetDevKind::parse("vcan"), Some(NetDevKind::Vcan));
        assert_eq!(NetDevKind::parse("vxcan"), Some(NetDevKind::VxCan));
    }

    #[test]
    fn test_kind_parse_case_insensitive() {
        assert_eq!(NetDevKind::parse("Bridge"), Some(NetDevKind::Bridge));
        assert_eq!(NetDevKind::parse("BOND"), Some(NetDevKind::Bond));
        assert_eq!(NetDevKind::parse("WireGuard"), Some(NetDevKind::WireGuard));
        assert_eq!(NetDevKind::parse("MacVLAN"), Some(NetDevKind::MacVlan));
    }

    #[test]
    fn test_kind_parse_unknown() {
        assert_eq!(NetDevKind::parse(""), None);
        assert_eq!(NetDevKind::parse("something"), None);
        assert_eq!(NetDevKind::parse("ethernet"), None);
    }

    #[test]
    fn test_kind_as_str_roundtrip() {
        let kinds = [
            NetDevKind::Bridge,
            NetDevKind::Bond,
            NetDevKind::Vlan,
            NetDevKind::Vxlan,
            NetDevKind::WireGuard,
            NetDevKind::Dummy,
            NetDevKind::Veth,
            NetDevKind::Gre,
            NetDevKind::Sit,
            NetDevKind::Ip6Tnl,
        ];
        for kind in kinds {
            let s = kind.as_str();
            assert_eq!(NetDevKind::parse(s), Some(kind), "roundtrip failed for {s}");
        }
    }

    #[test]
    fn test_kind_display() {
        assert_eq!(format!("{}", NetDevKind::Bridge), "bridge");
        assert_eq!(format!("{}", NetDevKind::WireGuard), "wireguard");
        assert_eq!(format!("{}", NetDevKind::Ip6GreTap), "ip6gretap");
    }

    #[test]
    fn test_kind_is_tunnel() {
        assert!(NetDevKind::Gre.is_tunnel());
        assert!(NetDevKind::GreTap.is_tunnel());
        assert!(NetDevKind::Ip6Gre.is_tunnel());
        assert!(NetDevKind::Ip6GreTap.is_tunnel());
        assert!(NetDevKind::Sit.is_tunnel());
        assert!(NetDevKind::Vti.is_tunnel());
        assert!(NetDevKind::Vti6.is_tunnel());
        assert!(NetDevKind::Ip6Tnl.is_tunnel());
        assert!(NetDevKind::Erspan.is_tunnel());

        assert!(!NetDevKind::Bridge.is_tunnel());
        assert!(!NetDevKind::Bond.is_tunnel());
        assert!(!NetDevKind::Vlan.is_tunnel());
        assert!(!NetDevKind::WireGuard.is_tunnel());
        assert!(!NetDevKind::Dummy.is_tunnel());
        assert!(!NetDevKind::Veth.is_tunnel());
    }

    // ── BondMode ───────────────────────────────────────────────────────

    #[test]
    fn test_bond_mode_parse() {
        assert_eq!(BondMode::parse("balance-rr"), Some(BondMode::BalanceRr));
        assert_eq!(
            BondMode::parse("active-backup"),
            Some(BondMode::ActiveBackup)
        );
        assert_eq!(BondMode::parse("balance-xor"), Some(BondMode::BalanceXor));
        assert_eq!(BondMode::parse("broadcast"), Some(BondMode::Broadcast));
        assert_eq!(BondMode::parse("802.3ad"), Some(BondMode::Ieee802_3ad));
        assert_eq!(BondMode::parse("balance-tlb"), Some(BondMode::BalanceTlb));
        assert_eq!(BondMode::parse("balance-alb"), Some(BondMode::BalanceAlb));
    }

    #[test]
    fn test_bond_mode_parse_numeric() {
        assert_eq!(BondMode::parse("0"), Some(BondMode::BalanceRr));
        assert_eq!(BondMode::parse("1"), Some(BondMode::ActiveBackup));
        assert_eq!(BondMode::parse("2"), Some(BondMode::BalanceXor));
        assert_eq!(BondMode::parse("3"), Some(BondMode::Broadcast));
        assert_eq!(BondMode::parse("4"), Some(BondMode::Ieee802_3ad));
        assert_eq!(BondMode::parse("5"), Some(BondMode::BalanceTlb));
        assert_eq!(BondMode::parse("6"), Some(BondMode::BalanceAlb));
    }

    #[test]
    fn test_bond_mode_as_num() {
        assert_eq!(BondMode::BalanceRr.as_num(), 0);
        assert_eq!(BondMode::ActiveBackup.as_num(), 1);
        assert_eq!(BondMode::Ieee802_3ad.as_num(), 4);
    }

    #[test]
    fn test_bond_mode_display() {
        assert_eq!(format!("{}", BondMode::BalanceRr), "balance-rr");
        assert_eq!(format!("{}", BondMode::Ieee802_3ad), "802.3ad");
    }

    #[test]
    fn test_bond_mode_unknown() {
        assert_eq!(BondMode::parse(""), None);
        assert_eq!(BondMode::parse("invalid"), None);
        assert_eq!(BondMode::parse("7"), None);
    }

    // ── TransmitHashPolicy ─────────────────────────────────────────────

    #[test]
    fn test_transmit_hash_policy_parse() {
        assert_eq!(
            TransmitHashPolicy::parse("layer2"),
            Some(TransmitHashPolicy::Layer2)
        );
        assert_eq!(
            TransmitHashPolicy::parse("layer3+4"),
            Some(TransmitHashPolicy::Layer3Plus4)
        );
        assert_eq!(
            TransmitHashPolicy::parse("layer2+3"),
            Some(TransmitHashPolicy::Layer2Plus3)
        );
        assert_eq!(
            TransmitHashPolicy::parse("encap2+3"),
            Some(TransmitHashPolicy::Encap2Plus3)
        );
        assert_eq!(
            TransmitHashPolicy::parse("encap3+4"),
            Some(TransmitHashPolicy::Encap3Plus4)
        );
    }

    #[test]
    fn test_transmit_hash_policy_display() {
        assert_eq!(format!("{}", TransmitHashPolicy::Layer3Plus4), "layer3+4");
        assert_eq!(format!("{}", TransmitHashPolicy::VlanSrcMac), "vlan+srcmac");
    }

    // ── LacpTransmitRate ───────────────────────────────────────────────

    #[test]
    fn test_lacp_transmit_rate_parse() {
        assert_eq!(
            LacpTransmitRate::parse("slow"),
            Some(LacpTransmitRate::Slow)
        );
        assert_eq!(
            LacpTransmitRate::parse("fast"),
            Some(LacpTransmitRate::Fast)
        );
        assert_eq!(LacpTransmitRate::parse("0"), Some(LacpTransmitRate::Slow));
        assert_eq!(LacpTransmitRate::parse("1"), Some(LacpTransmitRate::Fast));
        assert_eq!(LacpTransmitRate::parse("invalid"), None);
    }

    // ── AdSelect / FailOverMacPolicy / ArpValidate / ArpAllTargets ──────

    #[test]
    fn test_ad_select_parse() {
        assert_eq!(AdSelect::parse("stable"), Some(AdSelect::Stable));
        assert_eq!(AdSelect::parse("bandwidth"), Some(AdSelect::Bandwidth));
        assert_eq!(AdSelect::parse("count"), Some(AdSelect::Count));
        assert_eq!(AdSelect::parse("0"), Some(AdSelect::Stable));
        assert_eq!(AdSelect::parse("1"), Some(AdSelect::Bandwidth));
        assert_eq!(AdSelect::parse("2"), Some(AdSelect::Count));
    }

    #[test]
    fn test_fail_over_mac_policy_parse() {
        assert_eq!(
            FailOverMacPolicy::parse("none"),
            Some(FailOverMacPolicy::None)
        );
        assert_eq!(
            FailOverMacPolicy::parse("active"),
            Some(FailOverMacPolicy::Active)
        );
        assert_eq!(
            FailOverMacPolicy::parse("follow"),
            Some(FailOverMacPolicy::Follow)
        );
    }

    #[test]
    fn test_arp_validate_parse() {
        assert_eq!(ArpValidate::parse("none"), Some(ArpValidate::None));
        assert_eq!(ArpValidate::parse("active"), Some(ArpValidate::Active));
        assert_eq!(ArpValidate::parse("backup"), Some(ArpValidate::Backup));
        assert_eq!(ArpValidate::parse("all"), Some(ArpValidate::All));
        assert_eq!(ArpValidate::parse("filter"), Some(ArpValidate::Filter));
        assert_eq!(
            ArpValidate::parse("filter_active"),
            Some(ArpValidate::FilterActive)
        );
        assert_eq!(
            ArpValidate::parse("filter-active"),
            Some(ArpValidate::FilterActive)
        );
    }

    #[test]
    fn test_arp_all_targets_parse() {
        assert_eq!(ArpAllTargets::parse("any"), Some(ArpAllTargets::Any));
        assert_eq!(ArpAllTargets::parse("all"), Some(ArpAllTargets::All));
        assert_eq!(ArpAllTargets::parse("0"), Some(ArpAllTargets::Any));
        assert_eq!(ArpAllTargets::parse("1"), Some(ArpAllTargets::All));
    }

    #[test]
    fn test_primary_reselect_policy_parse() {
        assert_eq!(
            PrimaryReselectPolicy::parse("always"),
            Some(PrimaryReselectPolicy::Always)
        );
        assert_eq!(
            PrimaryReselectPolicy::parse("better"),
            Some(PrimaryReselectPolicy::Better)
        );
        assert_eq!(
            PrimaryReselectPolicy::parse("failure"),
            Some(PrimaryReselectPolicy::Failure)
        );
    }

    // ── MacVlanMode ────────────────────────────────────────────────────

    #[test]
    fn test_macvlan_mode_parse() {
        assert_eq!(MacVlanMode::parse("private"), Some(MacVlanMode::Private));
        assert_eq!(MacVlanMode::parse("vepa"), Some(MacVlanMode::Vepa));
        assert_eq!(MacVlanMode::parse("bridge"), Some(MacVlanMode::Bridge));
        assert_eq!(MacVlanMode::parse("passthru"), Some(MacVlanMode::Passthru));
        assert_eq!(
            MacVlanMode::parse("passthrough"),
            Some(MacVlanMode::Passthru)
        );
        assert_eq!(MacVlanMode::parse("source"), Some(MacVlanMode::Source));
        assert_eq!(MacVlanMode::parse("unknown"), None);
    }

    #[test]
    fn test_macvlan_mode_display() {
        assert_eq!(format!("{}", MacVlanMode::Bridge), "bridge");
        assert_eq!(format!("{}", MacVlanMode::Passthru), "passthru");
    }

    // ── IpVlanMode / IpVlanFlags ───────────────────────────────────────

    #[test]
    fn test_ipvlan_mode_parse() {
        assert_eq!(IpVlanMode::parse("L2"), Some(IpVlanMode::L2));
        assert_eq!(IpVlanMode::parse("L3"), Some(IpVlanMode::L3));
        assert_eq!(IpVlanMode::parse("L3S"), Some(IpVlanMode::L3S));
        assert_eq!(IpVlanMode::parse("l2"), Some(IpVlanMode::L2));
        assert_eq!(IpVlanMode::parse("l3s"), Some(IpVlanMode::L3S));
    }

    #[test]
    fn test_ipvlan_flags_parse() {
        assert_eq!(IpVlanFlags::parse("bridge"), Some(IpVlanFlags::Bridge));
        assert_eq!(IpVlanFlags::parse("private"), Some(IpVlanFlags::Private));
        assert_eq!(IpVlanFlags::parse("vepa"), Some(IpVlanFlags::Vepa));
        assert_eq!(IpVlanFlags::parse("unknown"), None);
    }

    // ── TunnelEncapsulation ────────────────────────────────────────────

    #[test]
    fn test_tunnel_encapsulation_parse() {
        assert_eq!(
            TunnelEncapsulation::parse("FooOverUDP"),
            Some(TunnelEncapsulation::FooOverUdp)
        );
        assert_eq!(
            TunnelEncapsulation::parse("GenericUDPEncapsulation"),
            Some(TunnelEncapsulation::GenericUdpEncapsulation)
        );
        assert_eq!(
            TunnelEncapsulation::parse("fooudp"),
            Some(TunnelEncapsulation::FooOverUdp)
        );
        assert_eq!(
            TunnelEncapsulation::parse("gue"),
            Some(TunnelEncapsulation::GenericUdpEncapsulation)
        );
    }

    // ── BatmanGatewayMode / BatmanRoutingAlgorithm ─────────────────────

    #[test]
    fn test_batman_gateway_mode_parse() {
        assert_eq!(
            BatmanGatewayMode::parse("off"),
            Some(BatmanGatewayMode::Off)
        );
        assert_eq!(
            BatmanGatewayMode::parse("client"),
            Some(BatmanGatewayMode::Client)
        );
        assert_eq!(
            BatmanGatewayMode::parse("server"),
            Some(BatmanGatewayMode::Server)
        );
        assert_eq!(BatmanGatewayMode::parse("unknown"), None);
    }

    #[test]
    fn test_batman_routing_algorithm_parse() {
        assert_eq!(
            BatmanRoutingAlgorithm::parse("batman-v"),
            Some(BatmanRoutingAlgorithm::BatmanV)
        );
        assert_eq!(
            BatmanRoutingAlgorithm::parse("batman-iv"),
            Some(BatmanRoutingAlgorithm::BatmanIv)
        );
        assert_eq!(
            BatmanRoutingAlgorithm::parse("batman_v"),
            Some(BatmanRoutingAlgorithm::BatmanV)
        );
    }

    // ── Parsing helpers ────────────────────────────────────────────────

    #[test]
    fn test_parse_bool_values() {
        assert_eq!(parse_bool("yes"), Some(true));
        assert_eq!(parse_bool("true"), Some(true));
        assert_eq!(parse_bool("on"), Some(true));
        assert_eq!(parse_bool("1"), Some(true));
        assert_eq!(parse_bool("no"), Some(false));
        assert_eq!(parse_bool("false"), Some(false));
        assert_eq!(parse_bool("off"), Some(false));
        assert_eq!(parse_bool("0"), Some(false));
        assert_eq!(parse_bool("Yes"), Some(true));
        assert_eq!(parse_bool("TRUE"), Some(true));
        assert_eq!(parse_bool("invalid"), None);
    }

    #[test]
    fn test_parse_bytes_value() {
        assert_eq!(parse_bytes_value("1500"), Some(1500));
        assert_eq!(parse_bytes_value("1K"), Some(1024));
        assert_eq!(parse_bytes_value("1k"), Some(1024));
        assert_eq!(parse_bytes_value("1M"), Some(1024 * 1024));
        assert_eq!(parse_bytes_value("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_bytes_value(""), None);
        assert_eq!(parse_bytes_value("abc"), None);
    }

    #[test]
    fn test_parse_seconds() {
        assert_eq!(parse_seconds("5"), Some(5));
        assert_eq!(parse_seconds("5s"), Some(5));
        assert_eq!(parse_seconds("15sec"), Some(15));
        assert_eq!(parse_seconds(""), None);
        assert_eq!(parse_seconds("abc"), None);
    }

    #[test]
    fn test_parse_u8_decimal_and_hex() {
        assert_eq!(parse_u8("42"), Some(42));
        assert_eq!(parse_u8("0xFF"), Some(255));
        assert_eq!(parse_u8("0x00"), Some(0));
        assert_eq!(parse_u8(""), None);
    }

    #[test]
    fn test_parse_u16_decimal_and_hex() {
        assert_eq!(parse_u16("4789"), Some(4789));
        assert_eq!(parse_u16("0x1000"), Some(4096));
        assert_eq!(parse_u16("0"), Some(0));
        assert_eq!(parse_u16("65535"), Some(65535));
    }

    // ── Full file parsing: Bridge ──────────────────────────────────────

    #[test]
    fn test_parse_bridge_basic() {
        let content = "\
[NetDev]
Name=br0
Kind=bridge

[Bridge]
STP=yes
ForwardDelaySec=15
Priority=32768
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "br0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Bridge);
        let bridge = cfg.kind_config.bridge.unwrap();
        assert_eq!(bridge.stp, Some(true));
        assert_eq!(bridge.forward_delay_sec, Some(15));
        assert_eq!(bridge.priority, Some(32768));
    }

    #[test]
    fn test_parse_bridge_all_fields() {
        let content = "\
[NetDev]
Name=br0
Kind=bridge
Description=Test bridge
MTUBytes=9000
MACAddress=00:11:22:33:44:55

[Bridge]
STP=yes
ForwardDelaySec=15
HelloTimeSec=2
MaxAgeSec=20
AgeingTimeSec=300
Priority=100
GroupForwardMask=0
DefaultPVID=1
MulticastQuerier=no
MulticastSnooping=yes
VLANFiltering=true
VLANProtocol=802.1Q
MulticastIGMPVersion=3
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(
            cfg.netdev_section.description.as_deref(),
            Some("Test bridge")
        );
        assert_eq!(cfg.netdev_section.mtu_bytes, Some(9000));
        assert_eq!(
            cfg.netdev_section.mac_address.as_deref(),
            Some("00:11:22:33:44:55")
        );
        let b = cfg.kind_config.bridge.unwrap();
        assert_eq!(b.stp, Some(true));
        assert_eq!(b.forward_delay_sec, Some(15));
        assert_eq!(b.hello_time_sec, Some(2));
        assert_eq!(b.max_age_sec, Some(20));
        assert_eq!(b.ageing_time_sec, Some(300));
        assert_eq!(b.priority, Some(100));
        assert_eq!(b.group_forward_mask, Some(0));
        assert_eq!(b.default_pvid, Some(1));
        assert_eq!(b.multicast_querier, Some(false));
        assert_eq!(b.multicast_snooping, Some(true));
        assert_eq!(b.vlan_filtering, Some(true));
        assert_eq!(b.vlan_protocol.as_deref(), Some("802.1Q"));
        assert_eq!(b.multicast_igmp_version, Some(3));
    }

    // ── Full file parsing: Bond ────────────────────────────────────────

    #[test]
    fn test_parse_bond_basic() {
        let content = "\
[NetDev]
Name=bond0
Kind=bond

[Bond]
Mode=802.3ad
TransmitHashPolicy=layer3+4
LACPTransmitRate=fast
MIIMonitorSec=100
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "bond0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Bond);
        let bond = cfg.kind_config.bond.unwrap();
        assert_eq!(bond.mode, Some(BondMode::Ieee802_3ad));
        assert_eq!(
            bond.transmit_hash_policy,
            Some(TransmitHashPolicy::Layer3Plus4)
        );
        assert_eq!(bond.lacp_transmit_rate, Some(LacpTransmitRate::Fast));
        assert_eq!(bond.mii_monitor_sec.as_deref(), Some("100"));
    }

    #[test]
    fn test_parse_bond_all_fields() {
        let content = "\
[NetDev]
Name=bond0
Kind=bond

[Bond]
Mode=active-backup
TransmitHashPolicy=layer2
LACPTransmitRate=slow
MIIMonitorSec=100
UpDelaySec=200
DownDelaySec=200
ArpIntervalSec=500
ArpIpTargets=10.0.0.1 10.0.0.2
ArpValidate=all
ArpAllTargets=any
PrimaryReselectPolicy=always
ResendIGMP=1
PacketsPerSlave=1
GratuitousARP=1
AllSlavesActive=no
MinLinks=1
AdSelect=stable
FailOverMACPolicy=none
Primary=eth0
AdActorSystemPriority=65535
AdUserPortKey=0
AdActorSystem=00:de:ad:be:ef:01
PeerNotifyDelaySec=0
LPInterval=1
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let b = cfg.kind_config.bond.unwrap();
        assert_eq!(b.mode, Some(BondMode::ActiveBackup));
        assert_eq!(b.transmit_hash_policy, Some(TransmitHashPolicy::Layer2));
        assert_eq!(b.lacp_transmit_rate, Some(LacpTransmitRate::Slow));
        assert_eq!(b.mii_monitor_sec.as_deref(), Some("100"));
        assert_eq!(b.up_delay_sec.as_deref(), Some("200"));
        assert_eq!(b.down_delay_sec.as_deref(), Some("200"));
        assert_eq!(b.arp_interval_sec.as_deref(), Some("500"));
        assert_eq!(b.arp_ip_targets, vec!["10.0.0.1", "10.0.0.2"]);
        assert_eq!(b.arp_validate, Some(ArpValidate::All));
        assert_eq!(b.arp_all_targets, Some(ArpAllTargets::Any));
        assert_eq!(
            b.primary_reselect_policy,
            Some(PrimaryReselectPolicy::Always)
        );
        assert_eq!(b.resend_igmp, Some(1));
        assert_eq!(b.packets_per_slave, Some(1));
        assert_eq!(b.gratuitous_arp, Some(1));
        assert_eq!(b.all_slaves_active, Some(false));
        assert_eq!(b.min_links, Some(1));
        assert_eq!(b.ad_select, Some(AdSelect::Stable));
        assert_eq!(b.fail_over_mac_policy, Some(FailOverMacPolicy::None));
        assert_eq!(b.primary.as_deref(), Some("eth0"));
        assert_eq!(b.ad_actor_sys_prio, Some(65535));
        assert_eq!(b.ad_user_port_key, Some(0));
        assert_eq!(b.ad_actor_system.as_deref(), Some("00:de:ad:be:ef:01"));
        assert_eq!(b.peer_notify_delay_sec.as_deref(), Some("0"));
        assert_eq!(b.lp_interval, Some(1));
    }

    // ── Full file parsing: VLAN ────────────────────────────────────────

    #[test]
    fn test_parse_vlan_basic() {
        let content = "\
[NetDev]
Name=vlan100
Kind=vlan

[VLAN]
Id=100
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "vlan100");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Vlan);
        let vlan = cfg.kind_config.vlan.unwrap();
        assert_eq!(vlan.id, Some(100));
    }

    #[test]
    fn test_parse_vlan_all_fields() {
        let content = "\
[NetDev]
Name=vlan50
Kind=vlan

[VLAN]
Id=50
Protocol=802.1ad
GVRP=yes
MVRP=no
LooseBinding=true
ReorderHeader=false
EgressQOSMaps=0:1 2:3
IngressQOSMaps=4:5
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let v = cfg.kind_config.vlan.unwrap();
        assert_eq!(v.id, Some(50));
        assert_eq!(v.protocol.as_deref(), Some("802.1ad"));
        assert_eq!(v.gvrp, Some(true));
        assert_eq!(v.mvrp, Some(false));
        assert_eq!(v.loose_binding, Some(true));
        assert_eq!(v.reorder_header, Some(false));
        assert_eq!(v.egress_qos_maps, vec!["0:1", "2:3"]);
        assert_eq!(v.ingress_qos_maps, vec!["4:5"]);
    }

    // ── Full file parsing: VXLAN ───────────────────────────────────────

    #[test]
    fn test_parse_vxlan_basic() {
        let content = "\
[NetDev]
Name=vxlan10
Kind=vxlan

[VXLAN]
VNI=10
Remote=192.168.1.1
DestinationPort=4789
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "vxlan10");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Vxlan);
        let vx = cfg.kind_config.vxlan.unwrap();
        assert_eq!(vx.vni, Some(10));
        assert_eq!(vx.remote.as_deref(), Some("192.168.1.1"));
        assert_eq!(vx.destination_port, Some(4789));
    }

    #[test]
    fn test_parse_vxlan_all_fields() {
        let content = "\
[NetDev]
Name=vxlan42
Kind=vxlan

[VXLAN]
VNI=42
Remote=10.0.0.1
Group=239.0.0.1
Local=10.0.0.2
TOS=0
TTL=64
MacLearning=yes
FDBAgeingSec=300
DestinationPort=4789
MaximumFDBEntries=1000
UDP6ZeroChecksumRx=no
UDP6ZeroChecksumTx=no
PortRange=49152 65535
FlowLabel=0
L2MissNotification=no
L3MissNotification=no
ReduceARPProxy=no
RouteShortCircuit=no
GroupPolicyExtension=no
GenericProtocolExtension=no
Independent=false
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let vx = cfg.kind_config.vxlan.unwrap();
        assert_eq!(vx.vni, Some(42));
        assert_eq!(vx.remote.as_deref(), Some("10.0.0.1"));
        assert_eq!(vx.group.as_deref(), Some("239.0.0.1"));
        assert_eq!(vx.local.as_deref(), Some("10.0.0.2"));
        assert_eq!(vx.tos, Some(0));
        assert_eq!(vx.ttl, Some(64));
        assert_eq!(vx.mac_learning, Some(true));
        assert_eq!(vx.fdb_ageing_sec, Some(300));
        assert_eq!(vx.destination_port, Some(4789));
        assert_eq!(vx.maximum_fdb_entries, Some(1000));
        assert_eq!(vx.udp6_zero_checksum_rx, Some(false));
        assert_eq!(vx.udp6_zero_checksum_tx, Some(false));
        assert_eq!(vx.port_range.as_deref(), Some("49152 65535"));
        assert_eq!(vx.flow_label, Some(0));
        assert_eq!(vx.l2_miss, Some(false));
        assert_eq!(vx.l3_miss, Some(false));
        assert_eq!(vx.reduce_arp_proxy, Some(false));
        assert_eq!(vx.route_short_circuit, Some(false));
        assert_eq!(vx.group_policy_extension, Some(false));
        assert_eq!(vx.generic_protocol_extension, Some(false));
        assert_eq!(vx.independent, Some(false));
    }

    // ── Full file parsing: WireGuard ───────────────────────────────────

    #[test]
    fn test_parse_wireguard_basic() {
        let content = "\
[NetDev]
Name=wg0
Kind=wireguard

[WireGuard]
PrivateKey=testprivkey=
ListenPort=51820

[WireGuardPeer]
PublicKey=testpubkey=
AllowedIPs=10.192.122.3/32,10.192.124.0/24
Endpoint=209.202.254.14:51820
PersistentKeepalive=25
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "wg0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::WireGuard);
        let wg = cfg.kind_config.wireguard.unwrap();
        assert_eq!(wg.private_key.as_deref(), Some("testprivkey="));
        assert_eq!(wg.listen_port, Some(51820));
        assert_eq!(cfg.kind_config.wireguard_peers.len(), 1);
        let peer = &cfg.kind_config.wireguard_peers[0];
        assert_eq!(peer.public_key.as_deref(), Some("testpubkey="));
        assert_eq!(peer.allowed_ips, vec!["10.192.122.3/32", "10.192.124.0/24"]);
        assert_eq!(peer.endpoint.as_deref(), Some("209.202.254.14:51820"));
        assert_eq!(peer.persistent_keepalive, Some(25));
    }

    #[test]
    fn test_parse_wireguard_multiple_peers() {
        let content = "\
[NetDev]
Name=wg0
Kind=wireguard

[WireGuard]
PrivateKeyFile=/etc/wireguard/private.key
ListenPort=51820
FirewallMark=42

[WireGuardPeer]
PublicKey=peer1key=
Endpoint=1.2.3.4:51820
AllowedIPs=10.0.0.1/32

[WireGuardPeer]
PublicKey=peer2key=
Endpoint=5.6.7.8:51820
AllowedIPs=10.0.0.2/32
PresharedKeyFile=/etc/wireguard/psk.key
RouteTable=main
RouteMetric=100
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let wg = cfg.kind_config.wireguard.unwrap();
        assert_eq!(
            wg.private_key_file.as_deref(),
            Some("/etc/wireguard/private.key")
        );
        assert_eq!(wg.listen_port, Some(51820));
        assert_eq!(wg.fw_mark, Some(42));
        assert_eq!(cfg.kind_config.wireguard_peers.len(), 2);
        assert_eq!(
            cfg.kind_config.wireguard_peers[0].public_key.as_deref(),
            Some("peer1key=")
        );
        assert_eq!(
            cfg.kind_config.wireguard_peers[1].public_key.as_deref(),
            Some("peer2key=")
        );
        assert_eq!(
            cfg.kind_config.wireguard_peers[1]
                .preshared_key_file
                .as_deref(),
            Some("/etc/wireguard/psk.key")
        );
        assert_eq!(
            cfg.kind_config.wireguard_peers[1].route_table.as_deref(),
            Some("main")
        );
        assert_eq!(cfg.kind_config.wireguard_peers[1].route_metric, Some(100));
    }

    // ── Full file parsing: Tunnel ──────────────────────────────────────

    #[test]
    fn test_parse_gre_tunnel() {
        let content = "\
[NetDev]
Name=gre1
Kind=gre

[Tunnel]
Local=192.168.1.1
Remote=10.0.0.1
TTL=64
Key=1234
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "gre1");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Gre);
        assert!(cfg.netdev_section.kind.is_tunnel());
        let tunnel = cfg.kind_config.tunnel.unwrap();
        assert_eq!(tunnel.local.as_deref(), Some("192.168.1.1"));
        assert_eq!(tunnel.remote.as_deref(), Some("10.0.0.1"));
        assert_eq!(tunnel.ttl, Some(64));
        assert_eq!(tunnel.key.as_deref(), Some("1234"));
    }

    #[test]
    fn test_parse_sit_tunnel_all_fields() {
        let content = "\
[NetDev]
Name=sit1
Kind=sit

[Tunnel]
Local=192.168.0.1
Remote=10.0.0.1
TOS=16
TTL=128
DiscoverPathMTU=yes
Mode=ip6ip
Independent=false
AllowLocalRemote=no
FooOverUDP=yes
FOUDestinationPort=5555
FOUSourcePort=6666
Encapsulation=FooOverUDP
IPv6RapidDeploymentPrefix=2001:db8::/32
ISATAP=no
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let t = cfg.kind_config.tunnel.unwrap();
        assert_eq!(t.local.as_deref(), Some("192.168.0.1"));
        assert_eq!(t.remote.as_deref(), Some("10.0.0.1"));
        assert_eq!(t.tos, Some(16));
        assert_eq!(t.ttl, Some(128));
        assert_eq!(t.discover_path_mtu, Some(true));
        assert_eq!(t.mode.as_deref(), Some("ip6ip"));
        assert_eq!(t.independent, Some(false));
        assert_eq!(t.allow_local_remote, Some(false));
        assert_eq!(t.foo_over_udp, Some(true));
        assert_eq!(t.fou_destination_port, Some(5555));
        assert_eq!(t.fou_source_port, Some(6666));
        assert_eq!(t.encapsulation, Some(TunnelEncapsulation::FooOverUdp));
        assert_eq!(
            t.ipv6_rapid_deployment_prefix.as_deref(),
            Some("2001:db8::/32")
        );
        assert_eq!(t.isatap, Some(false));
    }

    #[test]
    fn test_parse_erspan_tunnel() {
        let content = "\
[NetDev]
Name=erspan0
Kind=erspan

[Tunnel]
Local=192.168.1.1
Remote=192.168.1.2
ERSPANVersion=2
ERSPANIndex=123
SerializeTunneledPackets=yes
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Erspan);
        let t = cfg.kind_config.tunnel.unwrap();
        assert_eq!(t.erspan_version, Some(2));
        assert_eq!(t.erspan_index, Some(123));
        assert_eq!(t.serialize_tunneled_packets, Some(true));
    }

    // ── Full file parsing: MACVLAN ─────────────────────────────────────

    #[test]
    fn test_parse_macvlan() {
        let content = "\
[NetDev]
Name=macvlan0
Kind=macvlan

[MACVLAN]
Mode=bridge
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::MacVlan);
        let mv = cfg.kind_config.macvlan.unwrap();
        assert_eq!(mv.mode, Some(MacVlanMode::Bridge));
    }

    #[test]
    fn test_parse_macvtap() {
        let content = "\
[NetDev]
Name=macvtap0
Kind=macvtap

[MACVTAP]
Mode=passthru
SourceMACAddress=00:11:22:33:44:55 aa:bb:cc:dd:ee:ff
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::MacVtap);
        let mv = cfg.kind_config.macvlan.unwrap();
        assert_eq!(mv.mode, Some(MacVlanMode::Passthru));
        assert_eq!(
            mv.source_mac_address,
            vec!["00:11:22:33:44:55", "aa:bb:cc:dd:ee:ff"]
        );
    }

    // ── Full file parsing: IPVLAN ──────────────────────────────────────

    #[test]
    fn test_parse_ipvlan() {
        let content = "\
[NetDev]
Name=ipvlan0
Kind=ipvlan

[IPVLAN]
Mode=L3
Flags=private
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::IpVlan);
        let iv = cfg.kind_config.ipvlan.unwrap();
        assert_eq!(iv.mode, Some(IpVlanMode::L3));
        assert_eq!(iv.flags, Some(IpVlanFlags::Private));
    }

    // ── Full file parsing: Tun/Tap ─────────────────────────────────────

    #[test]
    fn test_parse_tun() {
        let content = "\
[NetDev]
Name=tun0
Kind=tun

[Tun]
MultiQueue=yes
PacketInfo=no
VNetHeader=yes
User=nobody
Group=nogroup
KeepCarrier=false
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Tun);
        let t = cfg.kind_config.tun.unwrap();
        assert_eq!(t.multi_queue, Some(true));
        assert_eq!(t.packet_info, Some(false));
        assert_eq!(t.vnet_header, Some(true));
        assert_eq!(t.user.as_deref(), Some("nobody"));
        assert_eq!(t.group.as_deref(), Some("nogroup"));
        assert_eq!(t.keep_carrier, Some(false));
    }

    #[test]
    fn test_parse_tap() {
        let content = "\
[NetDev]
Name=tap0
Kind=tap

[Tap]
MultiQueue=yes
User=1000
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Tap);
        let t = cfg.kind_config.tun.unwrap();
        assert_eq!(t.multi_queue, Some(true));
        assert_eq!(t.user.as_deref(), Some("1000"));
    }

    // ── Full file parsing: Veth ────────────────────────────────────────

    #[test]
    fn test_parse_veth() {
        let content = "\
[NetDev]
Name=veth0
Kind=veth

[Peer]
Name=veth-peer
MACAddress=00:11:22:33:44:55
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Veth);
        let peer = cfg.kind_config.peer.unwrap();
        assert_eq!(peer.name.as_deref(), Some("veth-peer"));
        assert_eq!(peer.mac_address.as_deref(), Some("00:11:22:33:44:55"));
    }

    // ── Full file parsing: Dummy ───────────────────────────────────────

    #[test]
    fn test_parse_dummy() {
        let content = "\
[NetDev]
Name=dummy0
Kind=dummy
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "dummy0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Dummy);
        // No kind-specific section needed for dummy.
        assert!(cfg.kind_config.bridge.is_none());
        assert!(cfg.kind_config.bond.is_none());
    }

    // ── Full file parsing: FOU ─────────────────────────────────────────

    #[test]
    fn test_parse_fou() {
        let content = "\
[NetDev]
Name=fou0
Kind=fou

[FOU]
Encapsulation=FooOverUDP
Port=5555
PeerPort=6666
Local=192.168.1.1
Peer=192.168.1.2
Protocol=4
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Fou);
        let f = cfg.kind_config.fou.unwrap();
        assert_eq!(f.encapsulation, Some(TunnelEncapsulation::FooOverUdp));
        assert_eq!(f.port, Some(5555));
        assert_eq!(f.peer_port, Some(6666));
        assert_eq!(f.local.as_deref(), Some("192.168.1.1"));
        assert_eq!(f.peer.as_deref(), Some("192.168.1.2"));
        assert_eq!(f.protocol, Some(4));
    }

    // ── Full file parsing: Geneve ──────────────────────────────────────

    #[test]
    fn test_parse_geneve() {
        let content = "\
[NetDev]
Name=geneve0
Kind=geneve

[Geneve]
VNI=1000
Remote=10.0.0.1
TOS=0
TTL=64
DestinationPort=6081
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Geneve);
        let g = cfg.kind_config.geneve.unwrap();
        assert_eq!(g.vni, Some(1000));
        assert_eq!(g.remote.as_deref(), Some("10.0.0.1"));
        assert_eq!(g.tos, Some(0));
        assert_eq!(g.ttl, Some(64));
        assert_eq!(g.destination_port, Some(6081));
    }

    // ── Full file parsing: L2TP ────────────────────────────────────────

    #[test]
    fn test_parse_l2tp() {
        let content = "\
[NetDev]
Name=l2tp0
Kind=l2tp

[L2TP]
TunnelId=10
PeerTunnelId=20
Remote=10.0.0.1
Local=10.0.0.2
EncapsulationType=udp
UDPSourcePort=5000
UDPDestinationPort=5001

[L2TPSession]
SessionId=100
PeerSessionId=200
Name=l2tp-sess0

[L2TPSession]
SessionId=101
PeerSessionId=201
Name=l2tp-sess1
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::L2tp);
        let l = cfg.kind_config.l2tp.unwrap();
        assert_eq!(l.tunnel_id, Some(10));
        assert_eq!(l.peer_tunnel_id, Some(20));
        assert_eq!(l.remote.as_deref(), Some("10.0.0.1"));
        assert_eq!(l.local.as_deref(), Some("10.0.0.2"));
        assert_eq!(l.encapsulation_type.as_deref(), Some("udp"));
        assert_eq!(l.udp_source_port, Some(5000));
        assert_eq!(l.udp_destination_port, Some(5001));
        assert_eq!(cfg.kind_config.l2tp_sessions.len(), 2);
        assert_eq!(cfg.kind_config.l2tp_sessions[0].session_id, Some(100));
        assert_eq!(cfg.kind_config.l2tp_sessions[0].peer_session_id, Some(200));
        assert_eq!(
            cfg.kind_config.l2tp_sessions[0].name.as_deref(),
            Some("l2tp-sess0")
        );
        assert_eq!(cfg.kind_config.l2tp_sessions[1].session_id, Some(101));
    }

    // ── Full file parsing: MACsec ──────────────────────────────────────

    #[test]
    fn test_parse_macsec() {
        let content = "\
[NetDev]
Name=macsec0
Kind=macsec

[MACsec]
Port=1
Encrypt=yes
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::MACsec);
        let ms = cfg.kind_config.macsec.unwrap();
        assert_eq!(ms.port, Some(1));
        assert_eq!(ms.encrypt, Some(true));
    }

    // ── Full file parsing: BatmanAdvanced ──────────────────────────────

    #[test]
    fn test_parse_batman_advanced() {
        let content = "\
[NetDev]
Name=bat0
Kind=batadv

[BatmanAdvanced]
GatewayMode=client
Aggregation=yes
BridgeLoopAvoidance=yes
DistributedArpTable=yes
Fragmentation=yes
HopPenalty=15
OriginatorIntervalSec=1000
GatewayBandwidthDown=10M
GatewayBandwidthUp=5M
RoutingAlgorithm=batman-v
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::BatAdv);
        let bat = cfg.kind_config.batman.unwrap();
        assert_eq!(bat.gateway_mode, Some(BatmanGatewayMode::Client));
        assert_eq!(bat.aggregation, Some(true));
        assert_eq!(bat.bridge_loop_avoidance, Some(true));
        assert_eq!(bat.distributed_arp_table, Some(true));
        assert_eq!(bat.fragmentation, Some(true));
        assert_eq!(bat.hop_penalty, Some(15));
        assert_eq!(bat.originator_interval_sec.as_deref(), Some("1000"));
        assert_eq!(bat.gateway_bandwidth_down.as_deref(), Some("10M"));
        assert_eq!(bat.gateway_bandwidth_up.as_deref(), Some("5M"));
        assert_eq!(bat.routing_algorithm, Some(BatmanRoutingAlgorithm::BatmanV));
    }

    // ── Full file parsing: BareUDP ─────────────────────────────────────

    #[test]
    fn test_parse_bare_udp() {
        let content = "\
[NetDev]
Name=bareudp0
Kind=bareudp

[BareUDP]
DestinationPort=6635
EtherType=mpls_uc
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::BareUdp);
        let bu = cfg.kind_config.bare_udp.unwrap();
        assert_eq!(bu.destination_port, Some(6635));
        assert_eq!(bu.ether_type.as_deref(), Some("mpls_uc"));
    }

    // ── Full file parsing: XFRM ───────────────────────────────────────

    #[test]
    fn test_parse_xfrm() {
        let content = "\
[NetDev]
Name=xfrm0
Kind=xfrm

[XFRM]
InterfaceId=42
Independent=yes
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Xfrm);
        let x = cfg.kind_config.xfrm.unwrap();
        assert_eq!(x.interface_id, Some(42));
        assert_eq!(x.independent, Some(true));
    }

    // ── Full file parsing: VRF ─────────────────────────────────────────

    #[test]
    fn test_parse_vrf() {
        let content = "\
[NetDev]
Name=vrf0
Kind=vrf

[VRF]
Table=100
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Vrf);
        let v = cfg.kind_config.vrf.unwrap();
        assert_eq!(v.table, Some(100));
    }

    // ── Error handling ─────────────────────────────────────────────────

    #[test]
    fn test_parse_missing_name() {
        let content = "\
[NetDev]
Kind=bridge
";
        let result = parse_netdev_content(content, &test_path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing Name="));
    }

    #[test]
    fn test_parse_missing_kind() {
        let content = "\
[NetDev]
Name=br0
";
        let result = parse_netdev_content(content, &test_path());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("missing Kind="));
    }

    #[test]
    fn test_parse_empty_file() {
        let content = "";
        let result = parse_netdev_content(content, &test_path());
        assert!(result.is_err());
    }

    // ── Comments and blanks ────────────────────────────────────────────

    #[test]
    fn test_parse_comments_and_blanks() {
        let content = "\
# This is a comment
; Another comment

[NetDev]
# Comment in section
Name=br0
Kind=bridge

[Bridge]
# Bridge comment
STP=yes

";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "br0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Bridge);
        let bridge = cfg.kind_config.bridge.unwrap();
        assert_eq!(bridge.stp, Some(true));
    }

    // ── Unknown sections ───────────────────────────────────────────────

    #[test]
    fn test_parse_unknown_section_ignored() {
        let content = "\
[NetDev]
Name=br0
Kind=bridge

[SomeUnknownSection]
Foo=bar

[Bridge]
STP=yes
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "br0");
        let bridge = cfg.kind_config.bridge.unwrap();
        assert_eq!(bridge.stp, Some(true));
    }

    #[test]
    fn test_parse_vendor_extension_section_ignored() {
        let content = "\
[NetDev]
Name=br0
Kind=bridge

[X-CustomExtension]
Vendor=specific

[Bridge]
STP=no
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let bridge = cfg.kind_config.bridge.unwrap();
        assert_eq!(bridge.stp, Some(false));
    }

    // ── [Match] section ────────────────────────────────────────────────

    #[test]
    fn test_parse_match_section() {
        let content = "\
[Match]
Host=myhost
Virtualization=kvm
Architecture=x86-64

[NetDev]
Name=br0
Kind=bridge
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.match_section.host.as_deref(), Some("myhost"));
        assert_eq!(cfg.match_section.virtualization.as_deref(), Some("kvm"));
        assert_eq!(cfg.match_section.architecture.as_deref(), Some("x86-64"));
    }

    // ── Kind-specific section absent → None ────────────────────────────

    #[test]
    fn test_parse_bridge_without_section() {
        let content = "\
[NetDev]
Name=br0
Kind=bridge
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        // No [Bridge] section, so kind_config.bridge should be None.
        assert!(cfg.kind_config.bridge.is_none());
    }

    // ── Loading from directories ───────────────────────────────────────

    #[test]
    fn test_load_netdev_configs_from_empty() {
        let configs = load_netdev_configs_from(&[]);
        assert!(configs.is_empty());
    }

    #[test]
    fn test_load_netdev_configs_from_nonexistent() {
        let configs = load_netdev_configs_from(&[PathBuf::from("/nonexistent/path")]);
        assert!(configs.is_empty());
    }

    #[test]
    fn test_load_netdev_configs_from_with_files() {
        let dir = tempfile::tempdir().unwrap();
        let p1 = dir.path().join("10-br0.netdev");
        let p2 = dir.path().join("20-vlan100.netdev");
        let p3 = dir.path().join("README.txt"); // not a .netdev file

        fs::write(
            &p1,
            "[NetDev]\nName=br0\nKind=bridge\n\n[Bridge]\nSTP=yes\n",
        )
        .unwrap();
        fs::write(&p2, "[NetDev]\nName=vlan100\nKind=vlan\n\n[VLAN]\nId=100\n").unwrap();
        fs::write(&p3, "This is not a netdev file").unwrap();

        let configs = load_netdev_configs_from(&[dir.path().to_path_buf()]);
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].netdev_section.name, "br0");
        assert_eq!(configs[1].netdev_section.name, "vlan100");
    }

    #[test]
    fn test_load_netdev_configs_from_dedup() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();

        // Same filename in both dirs — first wins.
        fs::write(
            dir1.path().join("10-br0.netdev"),
            "[NetDev]\nName=br0\nKind=bridge\n\n[Bridge]\nSTP=yes\n",
        )
        .unwrap();
        fs::write(
            dir2.path().join("10-br0.netdev"),
            "[NetDev]\nName=br0-override\nKind=bridge\n\n[Bridge]\nSTP=no\n",
        )
        .unwrap();

        let configs =
            load_netdev_configs_from(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()]);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].netdev_section.name, "br0");
        let bridge = configs[0].kind_config.bridge.as_ref().unwrap();
        assert_eq!(bridge.stp, Some(true));
    }

    #[test]
    fn test_load_netdev_configs_from_sorted() {
        let dir = tempfile::tempdir().unwrap();

        fs::write(
            dir.path().join("30-vlan.netdev"),
            "[NetDev]\nName=vlan30\nKind=vlan\n\n[VLAN]\nId=30\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("10-br0.netdev"),
            "[NetDev]\nName=br0\nKind=bridge\n",
        )
        .unwrap();
        fs::write(
            dir.path().join("20-bond0.netdev"),
            "[NetDev]\nName=bond0\nKind=bond\n",
        )
        .unwrap();

        let configs = load_netdev_configs_from(&[dir.path().to_path_buf()]);
        assert_eq!(configs.len(), 3);
        // Sorted by filename lexicographically.
        assert_eq!(configs[0].netdev_section.name, "br0");
        assert_eq!(configs[1].netdev_section.name, "bond0");
        assert_eq!(configs[2].netdev_section.name, "vlan30");
    }

    #[test]
    fn test_load_netdev_configs_from_skips_non_netdev() {
        let dir = tempfile::tempdir().unwrap();
        fs::write(
            dir.path().join("10-br0.netdev"),
            "[NetDev]\nName=br0\nKind=bridge\n",
        )
        .unwrap();
        fs::write(dir.path().join("20-eth0.network"), "[Match]\nName=eth0\n").unwrap();
        fs::write(dir.path().join("99-default.link"), "[Match]\n").unwrap();

        let configs = load_netdev_configs_from(&[dir.path().to_path_buf()]);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].netdev_section.name, "br0");
    }

    // ── MTUBytes parsing ───────────────────────────────────────────────

    #[test]
    fn test_parse_mtu_bytes() {
        let content = "\
[NetDev]
Name=br0
Kind=bridge
MTUBytes=9000
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.mtu_bytes, Some(9000));
    }

    #[test]
    fn test_parse_mtu_bytes_with_suffix() {
        let content = "\
[NetDev]
Name=br0
Kind=bridge
MTUBytes=1K
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.mtu_bytes, Some(1024));
    }

    // ── Complex combined test ──────────────────────────────────────────

    #[test]
    fn test_parse_nixos_style_wireguard() {
        let content = "\
# Generated by NixOS
[NetDev]
Name=wg0
Kind=wireguard
Description=WireGuard VPN

[WireGuard]
PrivateKeyFile=/run/keys/wireguard-private
ListenPort=51820

[WireGuardPeer]
PublicKey=AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=
PresharedKeyFile=/run/keys/wireguard-psk
AllowedIPs=10.100.0.0/24,fd00::/64
Endpoint=[2001:db8::1]:51820
PersistentKeepalive=25

[WireGuardPeer]
PublicKey=BBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBBB=
AllowedIPs=10.100.1.0/24
Endpoint=203.0.113.1:51820
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "wg0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::WireGuard);
        assert_eq!(
            cfg.netdev_section.description.as_deref(),
            Some("WireGuard VPN")
        );
        let wg = cfg.kind_config.wireguard.unwrap();
        assert_eq!(
            wg.private_key_file.as_deref(),
            Some("/run/keys/wireguard-private")
        );
        assert_eq!(wg.listen_port, Some(51820));
        assert_eq!(cfg.kind_config.wireguard_peers.len(), 2);

        let p0 = &cfg.kind_config.wireguard_peers[0];
        assert_eq!(p0.allowed_ips, vec!["10.100.0.0/24", "fd00::/64"]);
        assert_eq!(p0.endpoint.as_deref(), Some("[2001:db8::1]:51820"));
        assert_eq!(p0.persistent_keepalive, Some(25));
        assert_eq!(
            p0.preshared_key_file.as_deref(),
            Some("/run/keys/wireguard-psk")
        );

        let p1 = &cfg.kind_config.wireguard_peers[1];
        assert_eq!(p1.allowed_ips, vec!["10.100.1.0/24"]);
        assert_eq!(p1.endpoint.as_deref(), Some("203.0.113.1:51820"));
        assert!(p1.persistent_keepalive.is_none());
    }

    #[test]
    fn test_parse_bond_with_arp_targets_comma_separated() {
        let content = "\
[NetDev]
Name=bond0
Kind=bond

[Bond]
Mode=active-backup
ArpIpTargets=10.0.0.1,10.0.0.2,10.0.0.3
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let b = cfg.kind_config.bond.unwrap();
        assert_eq!(b.arp_ip_targets, vec!["10.0.0.1", "10.0.0.2", "10.0.0.3"]);
    }

    #[test]
    fn test_no_kind_specific_sections_for_dummy() {
        let content = "\
[NetDev]
Name=dummy0
Kind=dummy
MTUBytes=1500
MACAddress=de:ad:be:ef:00:01
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "dummy0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Dummy);
        assert_eq!(cfg.netdev_section.mtu_bytes, Some(1500));
        assert_eq!(
            cfg.netdev_section.mac_address.as_deref(),
            Some("de:ad:be:ef:00:01")
        );
        assert!(cfg.kind_config.bridge.is_none());
        assert!(cfg.kind_config.bond.is_none());
        assert!(cfg.kind_config.vlan.is_none());
        assert!(cfg.kind_config.vxlan.is_none());
        assert!(cfg.kind_config.wireguard.is_none());
        assert!(cfg.kind_config.tunnel.is_none());
        assert!(cfg.kind_config.macvlan.is_none());
        assert!(cfg.kind_config.ipvlan.is_none());
        assert!(cfg.kind_config.tun.is_none());
        assert!(cfg.kind_config.peer.is_none());
        assert!(cfg.kind_config.fou.is_none());
        assert!(cfg.kind_config.geneve.is_none());
        assert!(cfg.kind_config.l2tp.is_none());
        assert!(cfg.kind_config.macsec.is_none());
        assert!(cfg.kind_config.batman.is_none());
        assert!(cfg.kind_config.bare_udp.is_none());
        assert!(cfg.kind_config.xfrm.is_none());
        assert!(cfg.kind_config.vrf.is_none());
    }

    #[test]
    fn test_parse_vlan_id_accumulates_qos_maps() {
        let content = "\
[NetDev]
Name=vlan10
Kind=vlan

[VLAN]
Id=10
EgressQOSMaps=0:0 1:1
EgressQOSMaps=2:2
IngressQOSMaps=0:0
IngressQOSMaps=1:1 2:2
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let v = cfg.kind_config.vlan.unwrap();
        assert_eq!(v.id, Some(10));
        assert_eq!(v.egress_qos_maps, vec!["0:0", "1:1", "2:2"]);
        assert_eq!(v.ingress_qos_maps, vec!["0:0", "1:1", "2:2"]);
    }

    #[test]
    fn test_parse_wireguard_allowed_ips_accumulate() {
        let content = "\
[NetDev]
Name=wg0
Kind=wireguard

[WireGuard]
PrivateKey=test=

[WireGuardPeer]
PublicKey=peer=
AllowedIPs=10.0.0.0/24
AllowedIPs=10.0.1.0/24,10.0.2.0/24
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let peer = &cfg.kind_config.wireguard_peers[0];
        assert_eq!(
            peer.allowed_ips,
            vec!["10.0.0.0/24", "10.0.1.0/24", "10.0.2.0/24"]
        );
    }

    #[test]
    fn test_parse_macvlan_source_mac_accumulate() {
        let content = "\
[NetDev]
Name=mv0
Kind=macvlan

[MACVLAN]
Mode=source
SourceMACAddress=00:11:22:33:44:55
SourceMACAddress=aa:bb:cc:dd:ee:ff 11:22:33:44:55:66
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        let mv = cfg.kind_config.macvlan.unwrap();
        assert_eq!(mv.mode, Some(MacVlanMode::Source));
        assert_eq!(
            mv.source_mac_address,
            vec![
                "00:11:22:33:44:55",
                "aa:bb:cc:dd:ee:ff",
                "11:22:33:44:55:66"
            ]
        );
    }

    // ── Whitespace handling ────────────────────────────────────────────

    #[test]
    fn test_parse_whitespace_around_values() {
        let content = "\
[NetDev]
Name =  br0
Kind  = bridge

[Bridge]
STP =  yes
Priority=  100
";
        let cfg = parse_netdev_content(content, &test_path()).unwrap();
        assert_eq!(cfg.netdev_section.name, "br0");
        assert_eq!(cfg.netdev_section.kind, NetDevKind::Bridge);
        let b = cfg.kind_config.bridge.unwrap();
        assert_eq!(b.stp, Some(true));
        assert_eq!(b.priority, Some(100));
    }
}
