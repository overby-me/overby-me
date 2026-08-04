#![allow(dead_code)]

//! systemd-network-generator — generate networkd config from kernel command line.
//!
//! Reads `/proc/cmdline` and translates network-related kernel parameters into
//! `.network`, `.netdev`, and `.link` files under `/run/systemd/network/`.
//!
//! Supported kernel command line parameters:
//! - `ip=` — network configuration (DHCP, static, auto6, etc.)
//! - `rd.route=` — static route definitions
//! - `nameserver=` — DNS server addresses
//! - `rd.peerdns=` — whether to use DNS from DHCP
//! - `vlan=` — VLAN device definitions
//! - `bond=` — bond device definitions
//! - `bridge=` — bridge device definitions
//! - `ifname=` — interface renaming by MAC address
//! - `net.ifnames=` — predictable network interface names
//!
//! Usage:
//!   systemd-network-generator [OUTPUT_DIR]
//!
//! If OUTPUT_DIR is not specified, files are written to `/run/systemd/network/`.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as FmtWrite;
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::process;

// ── Constants ──────────────────────────────────────────────────────────────

const DEFAULT_OUTPUT_DIR: &str = "/run/systemd/network";
const PROC_CMDLINE: &str = "/proc/cmdline";

// ── Data model ─────────────────────────────────────────────────────────────

/// Parsed kernel `ip=` parameter.
#[derive(Debug, Clone, Default)]
struct IpConfig {
    /// Client IP address (empty for DHCP).
    client_ip: String,
    /// Server IP (PXE).
    server_ip: String,
    /// Default gateway.
    gateway: String,
    /// Netmask or prefix length.
    netmask: String,
    /// Hostname to set.
    hostname: String,
    /// Interface name.
    device: String,
    /// Autoconf method: dhcp, dhcp6, auto6, on, any, none, off, ibft, static.
    autoconf: String,
    /// Primary DNS server.
    dns0: String,
    /// Secondary DNS server.
    dns1: String,
    /// NTP server.
    ntp0: String,
    /// Link MTU from the `ip=` short form's `[:<mtu>]` field.
    mtu: String,
    /// Link MAC from the `ip=` short form's `[:<macaddr>]` field.
    mac: String,
}

/// Parsed `rd.route=` parameter: `<net>/<mask>:<gateway>[:<interface>]`.
#[derive(Debug, Clone)]
struct RouteConfig {
    destination: String,
    gateway: String,
    device: String,
}

/// Parsed `vlan=` parameter: `<vlanname>:<phys_dev>`.
#[derive(Debug, Clone)]
struct VlanConfig {
    name: String,
    parent: String,
    id: u16,
}

/// Parsed `bond=` parameter: `<bondname>:<slaves>[:<options>[:<mtu>]]`.
#[derive(Debug, Clone)]
struct BondConfig {
    name: String,
    slaves: Vec<String>,
    options: String,
    mtu: String,
}

/// Parsed `bridge=` parameter: `<bridgename>:<members>`.
#[derive(Debug, Clone)]
struct BridgeConfig {
    name: String,
    members: Vec<String>,
}

/// Per-interface `[Network]` settings accumulated from `vlan=`/`bond=`/`bridge=`.
/// C keys its Networks by interface name and merges these with any `ip=` config
/// into a single `70-<ifname>.network`.
#[derive(Debug, Default)]
struct NetworkExtra {
    vlans: Vec<String>,
    bridge: Option<String>,
    bond: Option<String>,
}

/// A `.netdev` device (`vlan`/`bond`/`bridge`), emitted as `70-<name>.netdev`.
#[derive(Debug)]
struct NetDev {
    kind: String,
    name: String,
    mtu: String,
    vlan_id: Option<u16>,
}

/// Parsed `ifname=` parameter: `<interface>:<mac>`.
#[derive(Debug, Clone)]
struct IfnameConfig {
    name: String,
    mac: String,
}

/// Parsed `net.ifname_policy=` parameter: a list of naming policies and an
/// optional trailing MAC address that scopes them to one interface.
#[derive(Debug, Clone)]
struct IfnamePolicyConfig {
    /// Policies in cmdline order (become `NamePolicy=`).
    policies: Vec<String>,
    /// The subset of `policies` that are also valid alternative-names policies
    /// (become `AlternativeNamesPolicy=`).
    alt_policies: Vec<String>,
    /// Optional MAC (colon form, lowercase); when set the link matches on it.
    mac: Option<String>,
}

/// All parsed kernel command line network parameters.
#[derive(Debug, Default)]
struct CmdlineConfig {
    ip_configs: Vec<IpConfig>,
    routes: Vec<RouteConfig>,
    nameservers: Vec<String>,
    peer_dns: Option<bool>,
    vlans: Vec<VlanConfig>,
    bonds: Vec<BondConfig>,
    bridges: Vec<BridgeConfig>,
    ifnames: Vec<IfnameConfig>,
    ifname_policies: Vec<IfnamePolicyConfig>,
    /// `net.ifnames=0` disables predictable interface names.
    net_ifnames: Option<bool>,
}

/// Collects generated file output keyed by filename.
#[derive(Debug, Default)]
struct GeneratedFiles {
    files: BTreeMap<String, String>,
}

impl GeneratedFiles {
    fn new() -> Self {
        Self::default()
    }

    fn add(&mut self, name: String, content: String) {
        self.files.insert(name, content);
    }

    fn write_to(&self, dir: &Path) -> io::Result<usize> {
        fs::create_dir_all(dir)?;
        let mut count = 0;
        for (name, content) in &self.files {
            let path = dir.join(name);
            fs::write(&path, content)?;
            count += 1;
        }
        Ok(count)
    }
}

// ── Kernel command line parsing ────────────────────────────────────────────

/// Read and parse `/proc/cmdline` (or a custom path for testing).
fn read_cmdline(path: &str) -> io::Result<String> {
    fs::read_to_string(path)
}

/// Parse a kernel command line string into tokens, respecting quoting.
fn tokenize_cmdline(cmdline: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut in_quote = false;
    let mut quote_char = ' ';

    for ch in cmdline.chars() {
        match ch {
            '"' | '\'' if !in_quote => {
                in_quote = true;
                quote_char = ch;
            }
            c if in_quote && c == quote_char => {
                in_quote = false;
            }
            ' ' | '\t' | '\n' if !in_quote => {
                if !current.is_empty() {
                    tokens.push(std::mem::take(&mut current));
                }
            }
            c => {
                current.push(c);
            }
        }
    }
    if !current.is_empty() {
        tokens.push(current);
    }
    tokens
}

/// Parse all network-related parameters from a kernel command line string.
fn parse_cmdline(cmdline: &str) -> CmdlineConfig {
    let tokens = tokenize_cmdline(cmdline);
    let mut config = CmdlineConfig::default();

    for token in &tokens {
        if let Some(val) = strip_param(token, "ip=") {
            if let Some(ip) = parse_ip_param(val) {
                config.ip_configs.push(ip);
            }
        } else if let Some(val) = strip_param(token, "rd.route=") {
            if let Some(route) = parse_route_param(val) {
                config.routes.push(route);
            }
        } else if let Some(val) = strip_param(token, "nameserver=") {
            if !val.is_empty() {
                config.nameservers.push(val.to_string());
            }
        } else if let Some(val) = strip_param(token, "rd.peerdns=") {
            config.peer_dns = parse_bool_param(val);
        } else if let Some(val) = strip_param(token, "vlan=") {
            if let Some(vlan) = parse_vlan_param(val) {
                config.vlans.push(vlan);
            }
        } else if let Some(val) = strip_param(token, "bond=") {
            if let Some(bond) = parse_bond_param(val) {
                config.bonds.push(bond);
            }
        } else if let Some(val) = strip_param(token, "bridge=") {
            if let Some(bridge) = parse_bridge_param(val) {
                config.bridges.push(bridge);
            }
        } else if let Some(val) = strip_param(token, "ifname=") {
            if let Some(ifn) = parse_ifname_param(val) {
                config.ifnames.push(ifn);
            }
        } else if let Some(val) = strip_param(token, "net.ifname_policy=") {
            if let Some(pol) = parse_ifname_policy_param(val) {
                config.ifname_policies.push(pol);
            }
        } else if let Some(val) = strip_param(token, "net.ifnames=") {
            config.net_ifnames = parse_bool_param(val);
        }
    }

    config
}

fn strip_param<'a>(token: &'a str, prefix: &str) -> Option<&'a str> {
    token.strip_prefix(prefix)
}

/// Whether `s` is a recognized `ip=` autoconf method. C keys the short form
/// `ip=<device>:<method>[:<mtu>[:<mac>]]` off the second colon-field being one
/// of these, so it is what distinguishes the short form from the full
/// `ip=<client>:<peer>:...` form.
fn is_autoconf_method(s: &str) -> bool {
    matches!(
        s.to_ascii_lowercase().as_str(),
        "dhcp"
            | "dhcp6"
            | "auto6"
            | "on"
            | "any"
            | "off"
            | "none"
            | "ibft"
            | "link6"
            | "link-local"
            | "either6"
    )
}

/// Split an `ip=` value on ':' while keeping a bracketed IPv6 literal
/// (`[2001:db8::1]`) as a single field with the brackets stripped. C's
/// `extract_ip_address` accepts `[...]`-wrapped IPv6 addresses in the
/// colon-delimited `ip=` form; a naive split on ':' shreds them (the address's
/// own colons become field separators).
fn split_ip_fields(val: &str) -> Vec<String> {
    let mut fields = Vec::new();
    let mut cur = String::new();
    let mut in_bracket = false;
    for c in val.chars() {
        match c {
            '[' => in_bracket = true,
            ']' => in_bracket = false,
            ':' if !in_bracket => fields.push(std::mem::take(&mut cur)),
            _ => cur.push(c),
        }
    }
    fields.push(cur);
    fields
}

/// Parse `ip=` value.
///
/// Supported forms:
/// - `ip=dhcp` / `ip=dhcp6` / `ip=auto6` / `ip=on` / `ip=any` — global autoconf
/// - `ip=<device>:dhcp` / `ip=<device>:dhcp6` / etc. — per-device autoconf
/// - `ip=<client-ip>:<server-ip>:<gw-ip>:<netmask>:<hostname>:<device>:<autoconf>[:<dns0>[:<dns1>[:<ntp0>]]]`
/// - `ip=<client-ip>::<gw-ip>:<netmask>::<device>:none`
fn parse_ip_param(val: &str) -> Option<IpConfig> {
    if val.is_empty() {
        return None;
    }

    // Check for simple keyword form (no colons, or single colon device:method).
    // Split bracket-aware so `[...]`-wrapped IPv6 addresses stay one field.
    let fields = split_ip_fields(val);
    let parts: Vec<&str> = fields.iter().map(String::as_str).collect();

    // Short form: ip=<device>:<method>[:<mtu>[:<macaddr>]]. C keys off the
    // second field being an autoconf method (regardless of whether the first
    // field looks like an IP), so this must be checked before the full form.
    // The remainder after the method is [<mtu>][:<macaddr>]: the mtu is the
    // first colon-delimited word, and everything after it (which may itself
    // contain the MAC's colons) is the MAC address.
    if parts.len() >= 2 && is_autoconf_method(parts[1]) {
        let mut ip = IpConfig {
            device: parts[0].to_string(),
            autoconf: parts[1].to_lowercase(),
            ..Default::default()
        };
        if parts.len() > 2 {
            let rest = parts[2..].join(":");
            let (mtu, mac) = rest.split_once(':').unwrap_or((rest.as_str(), ""));
            ip.mtu = mtu.to_string();
            ip.mac = mac.to_string();
        }
        return Some(ip);
    }

    match parts.len() {
        1 => {
            // ip=dhcp, ip=dhcp6, ip=auto6, ip=on, ip=any, ip=off, ip=none
            let method = parts[0].to_lowercase();
            match method.as_str() {
                "dhcp" | "dhcp6" | "auto6" | "on" | "any" | "off" | "none" | "ibft" => {
                    Some(IpConfig {
                        autoconf: method,
                        ..Default::default()
                    })
                }
                _ => {
                    // Could be a bare IP address — treat as client IP with no method
                    if looks_like_ip(parts[0]) {
                        Some(IpConfig {
                            client_ip: parts[0].to_string(),
                            autoconf: "none".to_string(),
                            ..Default::default()
                        })
                    } else {
                        log::warn!("Unrecognized ip= value: {}", val);
                        None
                    }
                }
            }
        }
        7..=10 => {
            // Full form:
            // ip=<client-ip>:<server-ip>:<gw-ip>:<netmask>:<hostname>:<device>:<autoconf>[:<dns0>[:<dns1>[:<ntp0>]]]
            let mut ip = IpConfig {
                client_ip: parts[0].to_string(),
                server_ip: parts[1].to_string(),
                gateway: parts[2].to_string(),
                netmask: parts[3].to_string(),
                hostname: parts[4].to_string(),
                device: parts[5].to_string(),
                autoconf: parts[6].to_lowercase(),
                ..Default::default()
            };
            if parts.len() > 7 {
                ip.dns0 = parts[7].to_string();
            }
            if parts.len() > 8 {
                ip.dns1 = parts[8].to_string();
            }
            if parts.len() > 9 {
                ip.ntp0 = parts[9].to_string();
            }
            // Default autoconf to "none" if client IP is set and autoconf is empty
            if !ip.client_ip.is_empty() && ip.autoconf.is_empty() {
                ip.autoconf = "none".to_string();
            }
            Some(ip)
        }
        _ => {
            log::warn!(
                "Unrecognized ip= format with {} fields: {}",
                parts.len(),
                val
            );
            None
        }
    }
}

/// Parse `rd.route=<net>/<mask>:<gateway>[:<interface>]`.
///
/// The destination always contains a `/` for CIDR notation, so we find the
/// first `:` after the `/` to split destination from gateway. This correctly
/// handles IPv6 addresses which contain colons (e.g.
/// `2001:db8::/32:fe80::1:eth0`).
fn parse_route_param(val: &str) -> Option<RouteConfig> {
    // Find the CIDR '/' — everything up to the first ':' after it is the destination.
    let slash_pos = match val.find('/') {
        Some(p) => p,
        None => {
            log::warn!(
                "Invalid rd.route= value (no CIDR / in destination): {}",
                val
            );
            return None;
        }
    };

    // Find the first ':' after the slash — that separates destination from gateway.
    let sep = match val[slash_pos..].find(':') {
        Some(offset) => slash_pos + offset,
        None => {
            log::warn!("Invalid rd.route= value (need at least dest:gw): {}", val);
            return None;
        }
    };

    let destination = val[..sep].to_string();
    let rest = &val[sep + 1..];

    // The rest is <gateway>[:<interface>]. The interface name never contains
    // colons, so split from the right: the last ':'-separated token that looks
    // like a plain interface name (no '.' or ':' inside) is the device.
    let (gateway, device) = split_gateway_device(rest);

    if destination.is_empty() || gateway.is_empty() {
        log::warn!("Invalid rd.route= value (empty dest or gw): {}", val);
        return None;
    }

    Some(RouteConfig {
        destination,
        gateway,
        device,
    })
}

/// Split `<gateway>[:<interface>]` where the gateway may be an IPv6 address.
///
/// Interface names are simple alphanumeric identifiers (e.g. `eth0`, `enp1s0`).
/// If the last colon-separated token looks like an interface name (all
/// alphanumeric, dots, dashes, underscores — and doesn't look like a hex IPv6
/// group), treat it as the device; otherwise the whole string is the gateway.
fn split_gateway_device(s: &str) -> (String, String) {
    if let Some(last_colon) = s.rfind(':') {
        let candidate = &s[last_colon + 1..];
        // An interface name is non-empty and contains at least one letter.
        let looks_like_iface = !candidate.is_empty()
            && candidate.chars().any(|c| c.is_ascii_alphabetic())
            && candidate
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.');
        if looks_like_iface {
            return (s[..last_colon].to_string(), candidate.to_string());
        }
    }
    (s.to_string(), String::new())
}

/// Parse `vlan=<vlanname>:<phys_dev>`.
///
/// The VLAN ID is extracted from the VLAN name: if the name contains a dot,
/// the part after the last dot is the ID (e.g. `eth0.100`); otherwise we try
/// to parse trailing digits from the name (e.g. `vlan100`).
fn parse_vlan_param(val: &str) -> Option<VlanConfig> {
    let parts: Vec<&str> = val.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        log::warn!("Invalid vlan= value: {}", val);
        return None;
    }

    let name = parts[0].to_string();
    let parent = parts[1].to_string();

    // Extract VLAN ID from name.
    let id = extract_vlan_id(&name)?;

    Some(VlanConfig { name, parent, id })
}

/// Extract VLAN ID from a VLAN interface name.
///
/// Tries `name.NNN` format first, then `vlanNNN` / trailing digits.
fn extract_vlan_id(name: &str) -> Option<u16> {
    // Try <parent>.<id> format
    if let Some(dot_pos) = name.rfind('.')
        && let Ok(id) = name[dot_pos + 1..].parse::<u16>()
    {
        return Some(id);
    }

    // Try trailing digits (e.g. vlan100)
    let digits: String = name
        .chars()
        .rev()
        .take_while(|c| c.is_ascii_digit())
        .collect::<String>()
        .chars()
        .rev()
        .collect();
    if !digits.is_empty()
        && let Ok(id) = digits.parse::<u16>()
    {
        return Some(id);
    }

    log::warn!("Cannot extract VLAN ID from name: {}", name);
    None
}

/// Parse `bond=<bondname>:<slaves>[:<options>[:<mtu>]]`.
fn parse_bond_param(val: &str) -> Option<BondConfig> {
    let parts: Vec<&str> = val.splitn(4, ':').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        log::warn!("Invalid bond= value: {}", val);
        return None;
    }

    let name = parts[0].to_string();
    let slaves: Vec<String> = parts[1]
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if slaves.is_empty() {
        log::warn!("Bond has no slaves: {}", val);
        return None;
    }
    let options = if parts.len() > 2 {
        parts[2].to_string()
    } else {
        String::new()
    };
    let mtu = if parts.len() > 3 {
        parts[3].to_string()
    } else {
        String::new()
    };

    Some(BondConfig {
        name,
        slaves,
        options,
        mtu,
    })
}

/// Parse `bridge=<bridgename>:<members>`.
fn parse_bridge_param(val: &str) -> Option<BridgeConfig> {
    let parts: Vec<&str> = val.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        log::warn!("Invalid bridge= value: {}", val);
        return None;
    }

    let name = parts[0].to_string();
    let members: Vec<String> = parts[1]
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if members.is_empty() {
        log::warn!("Bridge has no members: {}", val);
        return None;
    }

    Some(BridgeConfig { name, members })
}

/// Parse `ifname=<interface>:<mac>`.
fn parse_ifname_param(val: &str) -> Option<IfnameConfig> {
    let parts: Vec<&str> = val.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        log::warn!("Invalid ifname= value: {}", val);
        return None;
    }

    Some(IfnameConfig {
        name: parts[0].to_string(),
        mac: parts[1].to_string(),
    })
}

/// Membership in C's NamePolicy enum (name_policy_from_string).
fn is_name_policy(word: &str) -> bool {
    matches!(
        word,
        "kernel" | "keep" | "database" | "onboard" | "slot" | "path" | "mac"
    )
}

/// Membership in C's AlternativeNamesPolicy enum
/// (alternative_names_policy_from_string) — a subset of the name policies.
fn is_alt_names_policy(word: &str) -> bool {
    matches!(word, "database" | "onboard" | "slot" | "path" | "mac")
}

/// Normalize a 6-octet colon-separated MAC to lowercase, or None if malformed.
fn normalize_mac(word: &str) -> Option<String> {
    let octets: Vec<&str> = word.split(':').collect();
    if octets.len() != 6 {
        return None;
    }
    if octets
        .iter()
        .any(|o| o.len() != 2 || !o.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return None;
    }
    Some(word.to_lowercase())
}

/// Parse `net.ifname_policy=policy1[,policy2,...][,<MAC>]`.
///
/// Mirrors C's parse_cmdline_ifname_policy (network-generator.c): comma-separated
/// words are naming policies; the first word that is not a known policy is taken
/// as a trailing MAC address and must be last. Returns None (dropping the whole
/// item, as C errors out) if no policy is present or the MAC is malformed or not
/// last.
fn parse_ifname_policy_param(val: &str) -> Option<IfnamePolicyConfig> {
    let words: Vec<&str> = val.split(',').filter(|w| !w.is_empty()).collect();
    let mut policies = Vec::new();
    let mut alt_policies = Vec::new();
    let mut mac = None;
    for (i, word) in words.iter().enumerate() {
        if is_name_policy(word) {
            if is_alt_names_policy(word) {
                alt_policies.push((*word).to_string());
            }
            policies.push((*word).to_string());
        } else {
            // Not a policy: must be the trailing MAC address.
            if i != words.len() - 1 {
                log::warn!("Unexpected trailing string in ifname policy: {}", val);
                return None;
            }
            match normalize_mac(word) {
                Some(m) => mac = Some(m),
                None => {
                    log::warn!("Invalid MAC address in ifname policy: {}", word);
                    return None;
                }
            }
        }
    }
    if policies.is_empty() {
        log::warn!("No ifname policy specified: {}", val);
        return None;
    }
    Some(IfnamePolicyConfig {
        policies,
        alt_policies,
        mac,
    })
}

fn parse_bool_param(val: &str) -> Option<bool> {
    match val.to_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Some(true),
        "0" | "no" | "false" | "off" => Some(false),
        _ => {
            log::warn!("Invalid boolean value: {}", val);
            None
        }
    }
}

fn looks_like_ip(s: &str) -> bool {
    s.contains('.') || s.contains(':')
}

// ── File generation ────────────────────────────────────────────────────────

/// Generate all configuration files from parsed kernel command line.
fn generate(config: &CmdlineConfig) -> GeneratedFiles {
    let mut files = GeneratedFiles::new();

    // Generate .link files for ifname= parameters.
    for ifn in &config.ifnames {
        generate_ifname(&mut files, ifn);
    }

    // Generate .link files for net.ifname_policy= parameters.
    for pol in &config.ifname_policies {
        generate_ifname_policy(&mut files, pol);
    }

    // net.ifnames= is consumed by udev, not this generator; C's
    // systemd-network-generator writes no file for it, so neither do we.

    // Build netdevs and per-interface [Network] membership from vlan=/bond=/
    // bridge=. Each creates a 70-<name>.netdev; the parent (vlan) or each member
    // (bond/bridge) gets a VLAN=/Bond=/Bridge= entry merged into its .network.
    // team= is NOT a C kernel-command-line option, so C emits nothing for it.
    let mut netdevs: Vec<NetDev> = Vec::new();
    let mut extra: BTreeMap<String, NetworkExtra> = BTreeMap::new();
    for vlan in &config.vlans {
        netdevs.push(NetDev {
            kind: "vlan".to_string(),
            name: vlan.name.clone(),
            mtu: String::new(),
            vlan_id: Some(vlan.id),
        });
        extra
            .entry(vlan.parent.clone())
            .or_default()
            .vlans
            .push(vlan.name.clone());
    }
    for bond in &config.bonds {
        netdevs.push(NetDev {
            kind: "bond".to_string(),
            name: bond.name.clone(),
            mtu: bond.mtu.clone(),
            vlan_id: None,
        });
        for slave in &bond.slaves {
            extra.entry(slave.clone()).or_default().bond = Some(bond.name.clone());
        }
    }
    for bridge in &config.bridges {
        netdevs.push(NetDev {
            kind: "bridge".to_string(),
            name: bridge.name.clone(),
            mtu: String::new(),
            vlan_id: None,
        });
        for member in &bridge.members {
            extra.entry(member.clone()).or_default().bridge = Some(bridge.name.clone());
        }
    }
    for nd in &netdevs {
        emit_netdev(&mut files, nd);
    }

    // Group rd.route= by device ("" = unbound / deviceless default network).
    let mut routes_by_device: BTreeMap<String, Vec<&RouteConfig>> = BTreeMap::new();
    for route in &config.routes {
        routes_by_device
            .entry(route.device.clone())
            .or_default()
            .push(route);
    }

    // Every interface that needs a .network: any with an ip= config, a
    // vlan/bond/bridge membership, or a route. C keys its Networks by interface
    // and merges all of these into one 70-<ifname>.network (or 71-default for
    // the deviceless set), because systemd applies only one .network per link.
    let mut ifnames: BTreeSet<String> = BTreeSet::new();
    for ip in &config.ip_configs {
        ifnames.insert(ip.device.clone());
    }
    for k in extra.keys() {
        ifnames.insert(k.clone());
    }
    for k in routes_by_device.keys() {
        ifnames.insert(k.clone());
    }

    // C's context_merge_networks: the deviceless "" bucket holds nameserver= DNS,
    // unbound rd.route= routes, and rd.peerdns (dhcp_use_dns). When any device
    // network exists, that bucket's DNS/routes/peerdns are merged into EACH
    // device network and the bucket is dropped; otherwise it is emitted alone as
    // 71-default.
    let default_extra = NetworkExtra::default();
    let unbound_routes: Vec<&RouteConfig> =
        routes_by_device.get("").cloned().unwrap_or_default();
    let deviceless_ip = config.ip_configs.iter().find(|ip| ip.device.is_empty());
    let device_ifnames: Vec<&String> = ifnames.iter().filter(|n| !n.is_empty()).collect();

    if device_ifnames.is_empty() {
        // No device network: emit the default bucket if it carries anything.
        let has_default = deviceless_ip.is_some()
            || !config.nameservers.is_empty()
            || !unbound_routes.is_empty()
            || config.peer_dns.is_some();
        if has_default {
            emit_network(
                &mut files,
                "",
                deviceless_ip,
                &default_extra,
                &config.nameservers,
                config.peer_dns,
                &unbound_routes,
            );
        }
    } else {
        for ifname in device_ifnames {
            let ip = config.ip_configs.iter().find(|ip| &ip.device == ifname);
            let ex = extra.get(ifname).unwrap_or(&default_extra);
            // The device's own routes, followed by the merged unbound routes.
            let mut rts: Vec<&RouteConfig> =
                routes_by_device.get(ifname).cloned().unwrap_or_default();
            rts.extend(unbound_routes.iter().copied());
            emit_network(
                &mut files,
                ifname,
                ip,
                ex,
                &config.nameservers,
                config.peer_dns,
                &rts,
            );
        }
    }

    files
}

/// Write a `.link` file matching C's link_dump + link_save (network-generator.c):
/// filename prefix is `70` when an interface name is given, `71` for a MAC-only
/// match, `72` otherwise; the base is the ifname, the MAC without colons, or
/// `default`. The body matches on `Name=`/`MACAddress=`/`OriginalName=*` and
/// carries the `NamePolicy=`/`AlternativeNamesPolicy=` lists when present.
fn add_link_file(
    files: &mut GeneratedFiles,
    ifname: &str,
    mac: Option<&str>,
    policies: &[String],
    alt_policies: &[String],
) {
    let (prefix, base) = if !ifname.is_empty() {
        ("70", sanitize_name(ifname))
    } else if let Some(m) = mac {
        ("71", m.replace(':', ""))
    } else {
        ("72", "default".to_string())
    };
    let filename = format!("{prefix}-{base}.link");

    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Match]").unwrap();
    match mac {
        Some(m) => writeln!(content, "MACAddress={m}").unwrap(),
        None => writeln!(content, "OriginalName=*").unwrap(),
    }
    writeln!(content).unwrap();
    writeln!(content, "[Link]").unwrap();
    if !ifname.is_empty() {
        writeln!(content, "Name={ifname}").unwrap();
    }
    if !policies.is_empty() {
        writeln!(content, "NamePolicy={}", policies.join(" ")).unwrap();
    }
    if !alt_policies.is_empty() {
        writeln!(content, "AlternativeNamesPolicy={}", alt_policies.join(" ")).unwrap();
    }
    files.add(filename, content);
}

/// Generate a .link file for `ifname=<name>:<mac>` (70-<ifname>.link).
fn generate_ifname(files: &mut GeneratedFiles, ifn: &IfnameConfig) {
    add_link_file(files, &ifn.name, Some(&ifn.mac), &[], &[]);
}

/// Generate a .link file for `net.ifname_policy=<policies>[,<mac>]`
/// (72-default.link, or 71-<mac>.link when scoped to a MAC).
fn generate_ifname_policy(files: &mut GeneratedFiles, pol: &IfnamePolicyConfig) {
    add_link_file(
        files,
        "",
        pol.mac.as_deref(),
        &pol.policies,
        &pol.alt_policies,
    );
}

/// Emit one merged `70-<ifname>.network` (or `71-default.network` when
/// deviceless), matching C's network_dump. The `[Network]` section carries the
/// `ip=` DHCP/DNS/NTP settings, the `VLAN=`/`Bridge=`/`Bond=` membership from
/// vlan=/bond=/bridge=, and trailing `[Route]` blocks from rd.route=. C keys its
/// Networks by interface, so all of these merge into a single file per link.
fn emit_network(
    files: &mut GeneratedFiles,
    ifname: &str,
    ip: Option<&IpConfig>,
    extra: &NetworkExtra,
    nameservers: &[String],
    peer_dns: Option<bool>,
    routes: &[&RouteConfig],
) {
    // ibft interfaces are brought up by the initrd; C emits no .network.
    if ip.is_some_and(|ip| ip.autoconf == "ibft") {
        return;
    }

    // "70-<ifname>.network" for a named interface, else "71-default.network"
    // (the "70" prefix gives a named interface priority over the catch-all).
    // C uses the raw interface name in the filename (e.g. "70-eth0.100.network").
    let (prefix, suffix) = if ifname.is_empty() {
        ("71", "default")
    } else {
        ("70", ifname)
    };
    let filename = format!("{prefix}-{suffix}.network");

    let is_off = ip.is_some_and(|ip| matches!(ip.autoconf.as_str(), "off" | "none"));
    let is_static = is_off && ip.is_some_and(|ip| !ip.client_ip.is_empty());

    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(content).unwrap();

    // [Match]
    writeln!(content, "[Match]").unwrap();
    if ifname.is_empty() {
        writeln!(content, "Kind=!*").unwrap();
        writeln!(content, "Type=!loopback").unwrap();
    } else {
        writeln!(content, "Name={ifname}").unwrap();
    }

    // [Link]: MAC then MTU from an ip= short form (that order matches C's
    // link_dump); empty otherwise.
    writeln!(content, "\n[Link]").unwrap();
    if let Some(ip) = ip {
        if !ip.mac.is_empty() {
            writeln!(content, "MACAddress={}", ip.mac).unwrap();
        }
        if !ip.mtu.is_empty() {
            writeln!(content, "MTUBytes={}", ip.mtu).unwrap();
        }
    }

    // [Network] — entry order matches C's network_dump: DHCP, LinkLocal, RA,
    // DNS, VLAN, Bridge, Bond, NTP.
    writeln!(content, "\n[Network]").unwrap();
    if let Some(ip) = ip {
        let dhcp = if ip.autoconf == "dhcp6" {
            "ipv6"
        } else if ip.autoconf == "auto6" || is_off {
            "no"
        } else if ip.autoconf == "dhcp" {
            "ipv4"
        } else {
            "yes"
        };
        writeln!(content, "DHCP={dhcp}").unwrap();
        if is_off {
            writeln!(content, "LinkLocalAddressing=no").unwrap();
            writeln!(content, "IPv6AcceptRA=no").unwrap();
        }
        if !ip.dns0.is_empty() {
            writeln!(content, "DNS={}", ip.dns0).unwrap();
        }
        if !ip.dns1.is_empty() {
            writeln!(content, "DNS={}", ip.dns1).unwrap();
        }
    }
    // nameserver= entries apply to every network C emits (including bond/bridge
    // members and vlan parents that carry no ip= config), so they are listed
    // outside the ip= block, after any inline ip= DNS and before VLAN/Bond.
    for ns in nameservers {
        writeln!(content, "DNS={ns}").unwrap();
    }
    for v in &extra.vlans {
        writeln!(content, "VLAN={v}").unwrap();
    }
    if let Some(bridge) = &extra.bridge {
        writeln!(content, "Bridge={bridge}").unwrap();
    }
    if let Some(bond) = &extra.bond {
        writeln!(content, "Bond={bond}").unwrap();
    }
    if let Some(ip) = ip
        && !ip.ntp0.is_empty()
    {
        writeln!(content, "NTP={}", ip.ntp0).unwrap();
    }

    // [DHCP]
    writeln!(content, "\n[DHCP]").unwrap();
    if let Some(ip) = ip
        && !ip.hostname.is_empty()
    {
        writeln!(content, "Hostname={}", ip.hostname).unwrap();
    }
    // rd.peerdns= (UseDNS) is a global that C merges into every network.
    match peer_dns {
        Some(true) => writeln!(content, "UseDNS=yes").unwrap(),
        Some(false) => writeln!(content, "UseDNS=no").unwrap(),
        None => {}
    }

    // [Address]/[Route] for a static ip=.
    if let Some(ip) = ip
        && is_static
    {
        let address = format_address(&ip.client_ip, &ip.netmask);
        writeln!(content, "\n[Address]").unwrap();
        writeln!(content, "Address={address}").unwrap();
        if !ip.gateway.is_empty() {
            writeln!(content, "\n[Route]").unwrap();
            writeln!(content, "Gateway={}", ip.gateway).unwrap();
        }
    }

    // [Route] blocks for rd.route= entries. C keeps its route list head-first
    // (LIST_PREPEND), so it emits them in reverse of the command-line order.
    for route in routes.iter().rev() {
        writeln!(content, "\n[Route]").unwrap();
        writeln!(content, "Destination={}", route.destination).unwrap();
        writeln!(content, "Gateway={}", route.gateway).unwrap();
    }

    files.add(filename, content);
}

/// Emit a `70-<name>.netdev` matching C's netdev_dump: Kind, Name, optional
/// MTUBytes, and a `[VLAN] Id` section for vlan kinds. C emits no `[Bond]`
/// section, so bond options are dropped.
fn emit_netdev(files: &mut GeneratedFiles, nd: &NetDev) {
    // C uses the raw device name (e.g. "70-eth0.100.netdev").
    let filename = format!("70-{}.netdev", nd.name);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[NetDev]").unwrap();
    writeln!(content, "Kind={}", nd.kind).unwrap();
    writeln!(content, "Name={}", nd.name).unwrap();
    if !nd.mtu.is_empty() {
        writeln!(content, "MTUBytes={}", nd.mtu).unwrap();
    }
    if let Some(id) = nd.vlan_id {
        writeln!(content, "\n[VLAN]").unwrap();
        writeln!(content, "Id={id}").unwrap();
    }
    files.add(filename, content);
}

// ── Helpers ────────────────────────────────────────────────────────────────

/// Format an IP address with netmask as CIDR notation for networkd.
fn format_address(ip: &str, netmask: &str) -> String {
    if netmask.is_empty() {
        // If no mask, guess from address family.
        if ip.contains(':') {
            format!("{}/128", ip)
        } else {
            format!("{}/32", ip)
        }
    } else if netmask.contains('.') {
        // Dotted netmask → convert to prefix length.
        let prefix_len = netmask_to_prefix(netmask);
        format!("{}/{}", ip, prefix_len)
    } else if let Ok(_n) = netmask.parse::<u8>() {
        // Already a prefix length.
        format!("{}/{}", ip, netmask)
    } else {
        // Unknown format, just append.
        format!("{}/{}", ip, netmask)
    }
}

/// Convert a dotted netmask (e.g., "255.255.255.0") to prefix length (e.g., 24).
fn netmask_to_prefix(mask: &str) -> u32 {
    let parts: Vec<u8> = mask.split('.').filter_map(|p| p.parse().ok()).collect();
    if parts.len() != 4 {
        return 32;
    }
    let bits: u32 = ((parts[0] as u32) << 24)
        | ((parts[1] as u32) << 16)
        | ((parts[2] as u32) << 8)
        | (parts[3] as u32);
    bits.count_ones()
}

/// Sanitize a device/interface name for use in filenames.
fn sanitize_name(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

// ── Main ───────────────────────────────────────────────────────────────────

fn print_help() {
    eprintln!("Usage: systemd-network-generator [OUTPUT_DIR]");
    eprintln!();
    eprintln!("Generate networkd .network/.netdev/.link files from kernel command line.");
    eprintln!();
    eprintln!(
        "  OUTPUT_DIR   Directory to write generated files (default: {})",
        DEFAULT_OUTPUT_DIR
    );
    eprintln!("  --help       Show this help");
    eprintln!("  --version    Show version");
}

fn print_version() {
    eprintln!("systemd-network-generator (rust-systemd)");
}

/// Copy credential-provided network config into place. For each file in
/// `$CREDENTIALS_DIRECTORY` whose name starts with one of the table prefixes,
/// write it to `<target_dir>/<rest><suffix>` (mode 0644). Mirrors upstream
/// systemd-network-generator's `pick_up_credentials()` (src/shared/creds-util.c),
/// which runs unconditionally alongside the kernel-command-line handling.
fn pick_up_credentials() -> io::Result<()> {
    // (credential name prefix, target directory, target filename suffix)
    const TABLE: &[(&str, &str, &str)] = &[
        ("network.conf.", "/run/systemd/networkd.conf.d/", ".conf"),
        ("network.link.", "/run/systemd/network/", ".link"),
        ("network.netdev.", "/run/systemd/network/", ".netdev"),
        ("network.network.", "/run/systemd/network/", ".network"),
    ];

    let cred_dir = match std::env::var_os("CREDENTIALS_DIRECTORY") {
        Some(d) => PathBuf::from(d),
        None => {
            log::debug!("No credentials directory set, skipping credential pick-up.");
            return Ok(());
        }
    };

    let entries = match fs::read_dir(&cred_dir) {
        Ok(e) => e,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(e),
    };

    for entry in entries {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let name = entry.file_name();
        let name = name.to_string_lossy();
        for (prefix, target_dir, suffix) in TABLE {
            let Some(rest) = name.strip_prefix(prefix) else {
                continue;
            };
            let filename = format!("{rest}{suffix}");
            // Reject anything that wouldn't resolve to a plain filename.
            if rest.is_empty() || filename.contains('/') || filename == "." || filename == ".." {
                log::warn!("Credential '{name}' yields invalid filename '{filename}', ignoring.");
                break;
            }
            fs::create_dir_all(target_dir)?;
            let target = Path::new(target_dir).join(&filename);
            fs::copy(entry.path(), &target)?;
            use std::os::unix::fs::PermissionsExt;
            let _ = fs::set_permissions(&target, fs::Permissions::from_mode(0o644));
            log::info!("Installed {} from credential.", target.display());
            break; // matched this credential; move on to the next one
        }
    }

    Ok(())
}

fn run(cmdline_path: &str, output_dir: &Path) -> i32 {
    // Kernel command line → .network files. Not having /proc/cmdline is fine
    // (e.g. in containers); credentials are still picked up below.
    let cmdline = match read_cmdline(cmdline_path) {
        Ok(c) => c,
        Err(e) => {
            log::info!("Could not read {}: {}", cmdline_path, e);
            String::new()
        }
    };
    run_cmdline_str(&cmdline, output_dir)
}

/// Process a kernel-command-line string (from `/proc/cmdline` or, when C's
/// `systemd-network-generator` is invoked with positional arguments, from those
/// arguments) into `.network`/`.netdev`/`.link` files under `output_dir`, then
/// pick up any credentials. Shared by `run` and the argument-driven `main` path.
fn run_cmdline_str(cmdline: &str, output_dir: &Path) -> i32 {
    let mut ret = 0;

    let config = parse_cmdline(cmdline);
    // If there are no network parameters, there is nothing to generate.
    if config.ip_configs.is_empty()
        && config.routes.is_empty()
        && config.nameservers.is_empty()
        && config.vlans.is_empty()
        && config.bonds.is_empty()
        && config.bridges.is_empty()
        && config.ifnames.is_empty()
        && config.ifname_policies.is_empty()
        && config.net_ifnames.is_none()
    {
        log::info!("No network parameters on kernel command line.");
    } else {
        let files = generate(&config);
        if files.files.is_empty() {
            log::info!("No configuration files to generate.");
        } else if let Err(e) = files.write_to(output_dir) {
            log::error!(
                "Failed to write configuration files to {}: {}",
                output_dir.display(),
                e
            );
            ret = 1;
        } else {
            log::info!("Generated configuration file(s) in {}", output_dir.display());
        }
    }

    // Credentials → networkd.conf.d/.network/.netdev/.link files. This runs
    // regardless of the kernel command line, matching upstream.
    if let Err(e) = pick_up_credentials() {
        log::warn!("Failed to pick up credentials: {e}");
        if ret == 0 {
            ret = 1;
        }
    }

    ret
}

fn setup_logging() {
    // Simple stderr logger.
    let level = std::env::var("SYSTEMD_LOG_LEVEL")
        .ok()
        .and_then(|l| match l.to_lowercase().as_str() {
            "debug" | "7" => Some(log::LevelFilter::Debug),
            "info" | "6" => Some(log::LevelFilter::Info),
            "warn" | "warning" | "4" => Some(log::LevelFilter::Warn),
            "err" | "error" | "3" => Some(log::LevelFilter::Error),
            _ => None,
        })
        .unwrap_or(log::LevelFilter::Info);

    log::set_max_level(level);

    // Use a simple logger that writes to stderr.
    struct SimpleLogger;
    impl log::Log for SimpleLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                eprintln!(
                    "systemd-network-generator[{}]: {}: {}",
                    std::process::id(),
                    record.level(),
                    record.args()
                );
            }
        }
        fn flush(&self) {}
    }

    static LOGGER: SimpleLogger = SimpleLogger;
    let _ = log::set_logger(&LOGGER);
}

fn main() {
    setup_logging();

    let args: Vec<String> = std::env::args().collect();

    // Argument handling mirrors C's systemd-network-generator: `--root=PATH`
    // selects an alternate filesystem root, and any positional arguments are
    // parsed as kernel-command-line items (used instead of /proc/cmdline when
    // present). With no positional item the real /proc/cmdline is read.
    let mut root: Option<PathBuf> = None;
    let mut items: Vec<String> = Vec::new();
    let mut i = 1;
    while i < args.len() {
        let arg = &args[i];
        match arg.as_str() {
            "--help" | "-h" => {
                print_help();
                process::exit(0);
            }
            "--version" => {
                print_version();
                process::exit(0);
            }
            "--root" => {
                i += 1;
                if i < args.len() {
                    root = Some(PathBuf::from(&args[i]));
                }
            }
            s if s.starts_with("--root=") => {
                root = Some(PathBuf::from(&s["--root=".len()..]));
            }
            other => items.push(other.to_string()),
        }
        i += 1;
    }

    // C writes under <root>/run/systemd/network/; without a root it is the
    // default absolute directory.
    let output_dir = match &root {
        Some(r) => r.join("run/systemd/network"),
        None => PathBuf::from(DEFAULT_OUTPUT_DIR),
    };

    let code = if items.is_empty() {
        run(PROC_CMDLINE, &output_dir)
    } else {
        run_cmdline_str(&items.join(" "), &output_dir)
    };
    process::exit(code);
}

// ── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── Tokenizer tests ────────────────────────────────────────────────

    #[test]
    fn test_tokenize_simple() {
        let tokens = tokenize_cmdline("foo bar baz");
        assert_eq!(tokens, vec!["foo", "bar", "baz"]);
    }

    #[test]
    fn test_tokenize_with_quotes() {
        let tokens = tokenize_cmdline("foo \"bar baz\" qux");
        assert_eq!(tokens, vec!["foo", "bar baz", "qux"]);
    }

    #[test]
    fn test_tokenize_with_single_quotes() {
        let tokens = tokenize_cmdline("foo 'bar baz' qux");
        assert_eq!(tokens, vec!["foo", "bar baz", "qux"]);
    }

    #[test]
    fn test_tokenize_empty() {
        let tokens = tokenize_cmdline("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn test_tokenize_extra_whitespace() {
        let tokens = tokenize_cmdline("  foo   bar  ");
        assert_eq!(tokens, vec!["foo", "bar"]);
    }

    #[test]
    fn test_tokenize_tabs_and_newlines() {
        let tokens = tokenize_cmdline("foo\tbar\nbaz");
        assert_eq!(tokens, vec!["foo", "bar", "baz"]);
    }

    // ── ip= parsing tests ─────────────────────────────────────────────

    #[test]
    fn test_parse_ip_dhcp() {
        let ip = parse_ip_param("dhcp").unwrap();
        assert_eq!(ip.autoconf, "dhcp");
        assert!(ip.device.is_empty());
    }

    #[test]
    fn test_parse_ip_dhcp6() {
        let ip = parse_ip_param("dhcp6").unwrap();
        assert_eq!(ip.autoconf, "dhcp6");
    }

    #[test]
    fn test_parse_ip_auto6() {
        let ip = parse_ip_param("auto6").unwrap();
        assert_eq!(ip.autoconf, "auto6");
    }

    #[test]
    fn test_parse_ip_on() {
        let ip = parse_ip_param("on").unwrap();
        assert_eq!(ip.autoconf, "on");
    }

    #[test]
    fn test_parse_ip_any() {
        let ip = parse_ip_param("any").unwrap();
        assert_eq!(ip.autoconf, "any");
    }

    #[test]
    fn test_parse_ip_off() {
        let ip = parse_ip_param("off").unwrap();
        assert_eq!(ip.autoconf, "off");
    }

    #[test]
    fn test_parse_ip_none() {
        let ip = parse_ip_param("none").unwrap();
        assert_eq!(ip.autoconf, "none");
    }

    #[test]
    fn test_parse_ip_ibft() {
        let ip = parse_ip_param("ibft").unwrap();
        assert_eq!(ip.autoconf, "ibft");
    }

    #[test]
    fn test_parse_ip_device_dhcp() {
        let ip = parse_ip_param("eth0:dhcp").unwrap();
        assert_eq!(ip.device, "eth0");
        assert_eq!(ip.autoconf, "dhcp");
    }

    #[test]
    fn test_parse_ip_device_dhcp6() {
        let ip = parse_ip_param("eth0:dhcp6").unwrap();
        assert_eq!(ip.device, "eth0");
        assert_eq!(ip.autoconf, "dhcp6");
    }

    #[test]
    fn test_parse_ip_device_auto6() {
        let ip = parse_ip_param("enp1s0:auto6").unwrap();
        assert_eq!(ip.device, "enp1s0");
        assert_eq!(ip.autoconf, "auto6");
    }

    #[test]
    fn test_parse_ip_device_off() {
        let ip = parse_ip_param("eth0:off").unwrap();
        assert_eq!(ip.device, "eth0");
        assert_eq!(ip.autoconf, "off");
    }

    #[test]
    fn test_parse_ip_static_full() {
        let ip =
            parse_ip_param("192.168.1.100::192.168.1.1:255.255.255.0:myhost:eth0:none").unwrap();
        assert_eq!(ip.client_ip, "192.168.1.100");
        assert_eq!(ip.server_ip, "");
        assert_eq!(ip.gateway, "192.168.1.1");
        assert_eq!(ip.netmask, "255.255.255.0");
        assert_eq!(ip.hostname, "myhost");
        assert_eq!(ip.device, "eth0");
        assert_eq!(ip.autoconf, "none");
    }

    #[test]
    fn test_parse_ip_static_with_dns() {
        let ip =
            parse_ip_param("10.0.0.2::10.0.0.1:255.255.255.0::eth0:none:8.8.8.8:8.8.4.4").unwrap();
        assert_eq!(ip.client_ip, "10.0.0.2");
        assert_eq!(ip.gateway, "10.0.0.1");
        assert_eq!(ip.dns0, "8.8.8.8");
        assert_eq!(ip.dns1, "8.8.4.4");
    }

    #[test]
    fn test_parse_ip_static_with_ntp() {
        let ip = parse_ip_param("10.0.0.2::10.0.0.1:24::eth0:none:8.8.8.8::pool.ntp.org").unwrap();
        assert_eq!(ip.ntp0, "pool.ntp.org");
    }

    #[test]
    fn test_parse_ip_static_with_server() {
        let ip =
            parse_ip_param("10.0.0.2:10.0.0.254:10.0.0.1:255.255.255.0:pxehost:eth0:none").unwrap();
        assert_eq!(ip.server_ip, "10.0.0.254");
    }

    #[test]
    fn test_parse_ip_empty() {
        assert!(parse_ip_param("").is_none());
    }

    #[test]
    fn test_parse_ip_invalid_method() {
        assert!(parse_ip_param("eth0:invalid_method").is_none());
    }

    #[test]
    fn test_parse_ip_bare_ip_address() {
        let ip = parse_ip_param("192.168.1.5").unwrap();
        assert_eq!(ip.client_ip, "192.168.1.5");
        assert_eq!(ip.autoconf, "none");
    }

    #[test]
    fn test_parse_ip_invalid_single_word() {
        assert!(parse_ip_param("foobar").is_none());
    }

    // ── rd.route= parsing tests ───────────────────────────────────────

    #[test]
    fn test_parse_route_basic() {
        let route = parse_route_param("10.0.0.0/8:192.168.1.1").unwrap();
        assert_eq!(route.destination, "10.0.0.0/8");
        assert_eq!(route.gateway, "192.168.1.1");
        assert!(route.device.is_empty());
    }

    #[test]
    fn test_parse_route_with_device() {
        let route = parse_route_param("10.0.0.0/8:192.168.1.1:eth0").unwrap();
        assert_eq!(route.destination, "10.0.0.0/8");
        assert_eq!(route.gateway, "192.168.1.1");
        assert_eq!(route.device, "eth0");
    }

    #[test]
    fn test_parse_route_ipv6() {
        let route = parse_route_param("2001:db8::/32:fe80::1:eth0").unwrap();
        assert_eq!(route.destination, "2001:db8::/32");
        assert_eq!(route.gateway, "fe80::1");
        assert_eq!(route.device, "eth0");
    }

    #[test]
    fn test_parse_route_default() {
        let route = parse_route_param("0.0.0.0/0:10.0.0.1").unwrap();
        assert_eq!(route.destination, "0.0.0.0/0");
        assert_eq!(route.gateway, "10.0.0.1");
    }

    #[test]
    fn test_parse_route_no_colon() {
        assert!(parse_route_param("10.0.0.0/8").is_none());
    }

    #[test]
    fn test_parse_route_empty_dest() {
        assert!(parse_route_param(":192.168.1.1").is_none());
    }

    #[test]
    fn test_parse_route_empty_gw() {
        assert!(parse_route_param("10.0.0.0/8:").is_none());
    }

    // ── vlan= parsing tests ──────────────────────────────────────────

    #[test]
    fn test_parse_vlan_dot_format() {
        let vlan = parse_vlan_param("eth0.100:eth0").unwrap();
        assert_eq!(vlan.name, "eth0.100");
        assert_eq!(vlan.parent, "eth0");
        assert_eq!(vlan.id, 100);
    }

    #[test]
    fn test_parse_vlan_name_format() {
        let vlan = parse_vlan_param("vlan50:enp1s0").unwrap();
        assert_eq!(vlan.name, "vlan50");
        assert_eq!(vlan.parent, "enp1s0");
        assert_eq!(vlan.id, 50);
    }

    #[test]
    fn test_parse_vlan_missing_parent() {
        assert!(parse_vlan_param("vlan50").is_none());
    }

    #[test]
    fn test_parse_vlan_empty_name() {
        assert!(parse_vlan_param(":eth0").is_none());
    }

    #[test]
    fn test_parse_vlan_no_id() {
        assert!(parse_vlan_param("myvlan:eth0").is_none());
    }

    // ── bond= parsing tests ──────────────────────────────────────────

    #[test]
    fn test_parse_bond_basic() {
        let bond = parse_bond_param("bond0:eth0,eth1").unwrap();
        assert_eq!(bond.name, "bond0");
        assert_eq!(bond.slaves, vec!["eth0", "eth1"]);
        assert!(bond.options.is_empty());
        assert!(bond.mtu.is_empty());
    }

    #[test]
    fn test_parse_bond_with_options() {
        let bond = parse_bond_param("bond0:eth0,eth1:mode=802.3ad,miimon=100").unwrap();
        assert_eq!(bond.name, "bond0");
        assert_eq!(bond.options, "mode=802.3ad,miimon=100");
    }

    #[test]
    fn test_parse_bond_with_mtu() {
        let bond = parse_bond_param("bond0:eth0,eth1:mode=balance-rr:9000").unwrap();
        assert_eq!(bond.mtu, "9000");
    }

    #[test]
    fn test_parse_bond_single_slave() {
        let bond = parse_bond_param("bond0:eth0").unwrap();
        assert_eq!(bond.slaves, vec!["eth0"]);
    }

    #[test]
    fn test_parse_bond_empty_name() {
        assert!(parse_bond_param(":eth0,eth1").is_none());
    }

    #[test]
    fn test_parse_bond_no_slaves() {
        assert!(parse_bond_param("bond0:").is_none());
    }

    // ── bridge= parsing tests ────────────────────────────────────────

    #[test]
    fn test_parse_bridge_basic() {
        let bridge = parse_bridge_param("br0:eth0,eth1").unwrap();
        assert_eq!(bridge.name, "br0");
        assert_eq!(bridge.members, vec!["eth0", "eth1"]);
    }

    #[test]
    fn test_parse_bridge_single_member() {
        let bridge = parse_bridge_param("br0:eth0").unwrap();
        assert_eq!(bridge.members, vec!["eth0"]);
    }

    #[test]
    fn test_parse_bridge_empty() {
        assert!(parse_bridge_param("br0:").is_none());
    }

    // ── ifname= parsing tests ───────────────────────────────────────

    #[test]
    fn test_parse_ifname_basic() {
        let ifn = parse_ifname_param("eth0:aa:bb:cc:dd:ee:ff").unwrap();
        assert_eq!(ifn.name, "eth0");
        assert_eq!(ifn.mac, "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn test_parse_ifname_missing_mac() {
        // splitn(2, ':') on "eth0" gives ["eth0"] which has len 1.
        assert!(parse_ifname_param("eth0").is_none());
    }

    #[test]
    fn test_parse_ifname_empty_name() {
        assert!(parse_ifname_param(":aa:bb:cc:dd:ee:ff").is_none());
    }

    // ── bool parsing tests ──────────────────────────────────────────

    #[test]
    fn test_parse_bool_true_values() {
        assert_eq!(parse_bool_param("1"), Some(true));
        assert_eq!(parse_bool_param("yes"), Some(true));
        assert_eq!(parse_bool_param("true"), Some(true));
        assert_eq!(parse_bool_param("on"), Some(true));
        assert_eq!(parse_bool_param("YES"), Some(true));
    }

    #[test]
    fn test_parse_bool_false_values() {
        assert_eq!(parse_bool_param("0"), Some(false));
        assert_eq!(parse_bool_param("no"), Some(false));
        assert_eq!(parse_bool_param("false"), Some(false));
        assert_eq!(parse_bool_param("off"), Some(false));
    }

    #[test]
    fn test_parse_bool_invalid() {
        assert_eq!(parse_bool_param("maybe"), None);
    }

    // ── net.ifnames= tests ──────────────────────────────────────────

    #[test]
    fn test_parse_net_ifnames_off() {
        let config = parse_cmdline("net.ifnames=0");
        assert_eq!(config.net_ifnames, Some(false));
    }

    #[test]
    fn test_parse_net_ifnames_on() {
        let config = parse_cmdline("net.ifnames=1");
        assert_eq!(config.net_ifnames, Some(true));
    }

    // ── Full cmdline parsing tests ──────────────────────────────────

    #[test]
    fn test_parse_cmdline_dhcp() {
        let config = parse_cmdline("ro root=UUID=xxx ip=dhcp");
        assert_eq!(config.ip_configs.len(), 1);
        assert_eq!(config.ip_configs[0].autoconf, "dhcp");
    }

    #[test]
    fn test_parse_cmdline_static_ip() {
        let config = parse_cmdline(
            "ip=192.168.1.100::192.168.1.1:255.255.255.0::eth0:none nameserver=8.8.8.8",
        );
        assert_eq!(config.ip_configs.len(), 1);
        assert_eq!(config.ip_configs[0].client_ip, "192.168.1.100");
        assert_eq!(config.nameservers, vec!["8.8.8.8"]);
    }

    #[test]
    fn test_parse_cmdline_multiple_nameservers() {
        let config = parse_cmdline("nameserver=8.8.8.8 nameserver=8.8.4.4 nameserver=1.1.1.1");
        assert_eq!(config.nameservers, vec!["8.8.8.8", "8.8.4.4", "1.1.1.1"]);
    }

    #[test]
    fn test_parse_cmdline_multiple_ip() {
        let config = parse_cmdline("ip=eth0:dhcp ip=eth1:dhcp6");
        assert_eq!(config.ip_configs.len(), 2);
        assert_eq!(config.ip_configs[0].device, "eth0");
        assert_eq!(config.ip_configs[0].autoconf, "dhcp");
        assert_eq!(config.ip_configs[1].device, "eth1");
        assert_eq!(config.ip_configs[1].autoconf, "dhcp6");
    }

    #[test]
    fn test_parse_cmdline_vlan_and_bond() {
        let config = parse_cmdline("vlan=eth0.100:eth0 bond=bond0:eth1,eth2:mode=802.3ad");
        assert_eq!(config.vlans.len(), 1);
        assert_eq!(config.bonds.len(), 1);
    }

    #[test]
    fn test_parse_cmdline_bridge() {
        let config = parse_cmdline("bridge=br0:eth0,eth1");
        assert_eq!(config.bridges.len(), 1);
        assert_eq!(config.bridges[0].name, "br0");
    }

    #[test]
    fn test_parse_cmdline_ifname() {
        let config = parse_cmdline("ifname=lan0:aa:bb:cc:dd:ee:ff");
        assert_eq!(config.ifnames.len(), 1);
        assert_eq!(config.ifnames[0].name, "lan0");
    }

    #[test]
    fn test_parse_cmdline_rd_route() {
        let config = parse_cmdline("rd.route=10.0.0.0/8:192.168.1.1:eth0");
        assert_eq!(config.routes.len(), 1);
        assert_eq!(config.routes[0].destination, "10.0.0.0/8");
        assert_eq!(config.routes[0].device, "eth0");
    }

    #[test]
    fn test_parse_cmdline_peerdns() {
        let config = parse_cmdline("rd.peerdns=0");
        assert_eq!(config.peer_dns, Some(false));
    }

    #[test]
    fn test_parse_cmdline_empty() {
        let config = parse_cmdline("");
        assert!(config.ip_configs.is_empty());
        assert!(config.routes.is_empty());
        assert!(config.nameservers.is_empty());
        assert!(config.vlans.is_empty());
        assert!(config.bonds.is_empty());
        assert!(config.bridges.is_empty());
        assert!(config.ifnames.is_empty());
    }

    #[test]
    fn test_parse_cmdline_non_network_params_ignored() {
        let config = parse_cmdline("ro root=UUID=xxx quiet splash");
        assert!(config.ip_configs.is_empty());
        assert!(config.routes.is_empty());
    }

    // ── Helper function tests ───────────────────────────────────────

    #[test]
    fn test_netmask_to_prefix() {
        assert_eq!(netmask_to_prefix("255.255.255.0"), 24);
        assert_eq!(netmask_to_prefix("255.255.0.0"), 16);
        assert_eq!(netmask_to_prefix("255.0.0.0"), 8);
        assert_eq!(netmask_to_prefix("255.255.255.255"), 32);
        assert_eq!(netmask_to_prefix("0.0.0.0"), 0);
        assert_eq!(netmask_to_prefix("255.255.255.128"), 25);
        assert_eq!(netmask_to_prefix("255.255.255.192"), 26);
    }

    #[test]
    fn test_netmask_to_prefix_invalid() {
        // Invalid format returns 32.
        assert_eq!(netmask_to_prefix("bad"), 32);
    }

    #[test]
    fn test_format_address_with_dotted_mask() {
        assert_eq!(
            format_address("192.168.1.100", "255.255.255.0"),
            "192.168.1.100/24"
        );
    }

    #[test]
    fn test_format_address_with_prefix_len() {
        assert_eq!(format_address("192.168.1.100", "24"), "192.168.1.100/24");
    }

    #[test]
    fn test_format_address_with_empty_mask_ipv4() {
        assert_eq!(format_address("192.168.1.100", ""), "192.168.1.100/32");
    }

    #[test]
    fn test_format_address_with_empty_mask_ipv6() {
        assert_eq!(format_address("2001:db8::1", ""), "2001:db8::1/128");
    }

    #[test]
    fn test_sanitize_name() {
        assert_eq!(sanitize_name("eth0"), "eth0");
        assert_eq!(sanitize_name("eth0.100"), "eth0-100");
        assert_eq!(sanitize_name("enp1s0f0"), "enp1s0f0");
        assert_eq!(sanitize_name("my/device"), "my-device");
    }

    #[test]
    fn test_extract_vlan_id_dot_format() {
        assert_eq!(extract_vlan_id("eth0.100"), Some(100));
        assert_eq!(extract_vlan_id("enp1s0.42"), Some(42));
    }

    #[test]
    fn test_extract_vlan_id_name_format() {
        assert_eq!(extract_vlan_id("vlan100"), Some(100));
        assert_eq!(extract_vlan_id("vlan42"), Some(42));
    }

    #[test]
    fn test_extract_vlan_id_no_digits() {
        assert_eq!(extract_vlan_id("myvlan"), None);
    }

    // ── File generation tests ───────────────────────────────────────

    #[test]
    fn test_generate_empty_cmdline() {
        let config = parse_cmdline("");
        let files = generate(&config);
        assert!(files.files.is_empty());
    }

    #[test]
    fn test_generate_dhcp_all() {
        let config = parse_cmdline("ip=dhcp");
        let files = generate(&config);
        assert!(files.files.contains_key("71-default.network"));
        let content = &files.files["71-default.network"];
        assert!(content.contains("[Match]"));
        // A device-less ip= matches every physical interface via Kind=!*.
        assert!(content.contains("Kind=!*"));
        // "dhcp" maps to IPv4 DHCP specifically (C: DHCP=ipv4, not "yes").
        assert!(content.contains("DHCP=ipv4"));
    }

    #[test]
    fn test_generate_dhcp_device() {
        let config = parse_cmdline("ip=eth0:dhcp");
        let files = generate(&config);
        assert!(files.files.contains_key("70-eth0.network"));
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("Name=eth0"));
        assert!(content.contains("DHCP=ipv4"));
    }

    #[test]
    fn test_generate_dhcp6() {
        let config = parse_cmdline("ip=eth0:dhcp6");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("DHCP=ipv6"));
    }

    #[test]
    fn test_generate_auto6() {
        let config = parse_cmdline("ip=eth0:auto6");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("DHCP=no"));
        // auto6 leaves RA at the networkd default (C emits no IPv6AcceptRA line).
        assert!(!content.contains("IPv6AcceptRA"));
    }

    #[test]
    fn test_generate_static_ip() {
        let config = parse_cmdline("ip=192.168.1.100::192.168.1.1:255.255.255.0::eth0:none");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("[Address]"));
        assert!(content.contains("Address=192.168.1.100/24"));
        assert!(content.contains("[Route]"));
        assert!(content.contains("Gateway=192.168.1.1"));
    }

    #[test]
    fn test_generate_static_no_gateway() {
        let config = parse_cmdline("ip=192.168.1.100:::255.255.255.0::eth0:none");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("[Address]"));
        assert!(!content.contains("[Route]"));
    }

    #[test]
    fn test_generate_with_nameservers() {
        let config = parse_cmdline("ip=eth0:dhcp nameserver=8.8.8.8 nameserver=1.1.1.1");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("DNS=8.8.8.8"));
        assert!(content.contains("DNS=1.1.1.1"));
    }

    #[test]
    fn test_generate_with_inline_dns() {
        let config = parse_cmdline("ip=10.0.0.2::10.0.0.1:24::eth0:none:8.8.8.8:8.8.4.4");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("DNS=8.8.8.8"));
        assert!(content.contains("DNS=8.8.4.4"));
    }

    #[test]
    fn test_generate_peerdns_off_disables_dhcp_dns() {
        // rd.peerdns=0 does not drop an explicit nameserver= (which is always a
        // static DNS= line); it only turns off DHCP-provided DNS via UseDNS=no,
        // matching C's systemd-network-generator.
        let config = parse_cmdline("ip=eth0:dhcp nameserver=8.8.8.8 rd.peerdns=0");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("DNS=8.8.8.8"));
        assert!(content.contains("UseDNS=no"));
    }

    #[test]
    fn test_generate_with_ntp() {
        let config = parse_cmdline("ip=10.0.0.2::10.0.0.1:24::eth0:none:::pool.ntp.org");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("NTP=pool.ntp.org"));
    }

    #[test]
    fn test_generate_off_no_address() {
        let config = parse_cmdline("ip=eth0:off");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("DHCP=no"));
        assert!(content.contains("LinkLocalAddressing=no"));
    }

    #[test]
    fn test_generate_ibft_no_output() {
        let config = parse_cmdline("ip=ibft");
        let files = generate(&config);
        // ibft is handled externally; we should not generate files for it.
        assert!(files.files.is_empty());
    }

    #[test]
    fn test_generate_ifname() {
        let config = parse_cmdline("ifname=lan0:aa:bb:cc:dd:ee:ff");
        let files = generate(&config);
        // C names the link file 70-<ifname>.link.
        assert!(files.files.contains_key("70-lan0.link"));
        let content = &files.files["70-lan0.link"];
        assert!(content.contains("[Match]"));
        assert!(content.contains("MACAddress=aa:bb:cc:dd:ee:ff"));
        assert!(content.contains("[Link]"));
        assert!(content.contains("Name=lan0"));
    }

    #[test]
    fn test_generate_net_ifnames_off_no_file() {
        // net.ifnames= is consumed by udev, not this generator; C's
        // systemd-network-generator writes no file for it (parsed but ignored).
        let config = parse_cmdline("net.ifnames=0");
        assert_eq!(config.net_ifnames, Some(false));
        let files = generate(&config);
        assert!(files.files.is_empty());
    }

    #[test]
    fn test_generate_net_ifnames_on_no_file() {
        let config = parse_cmdline("net.ifnames=1");
        let files = generate(&config);
        // net.ifnames=1 is the default; don't generate a file.
        assert!(!files.files.contains_key("71-net-ifnames.link"));
    }

    // ── net.ifname_policy= tests ────────────────────────────────────────────

    #[test]
    fn test_parse_ifname_policy_basic() {
        let config = parse_cmdline("net.ifname_policy=keep,kernel,path");
        assert_eq!(config.ifname_policies.len(), 1);
        let p = &config.ifname_policies[0];
        assert_eq!(p.policies, vec!["keep", "kernel", "path"]);
        // Only alternative-names policies (path) go to AlternativeNamesPolicy.
        assert_eq!(p.alt_policies, vec!["path"]);
        assert_eq!(p.mac, None);
    }

    #[test]
    fn test_parse_ifname_policy_with_mac() {
        let config = parse_cmdline("net.ifname_policy=path,mac,AA:BB:CC:DD:EE:FF");
        let p = &config.ifname_policies[0];
        assert_eq!(p.policies, vec!["path", "mac"]);
        assert_eq!(p.alt_policies, vec!["path", "mac"]);
        // MAC is normalized to lowercase.
        assert_eq!(p.mac.as_deref(), Some("aa:bb:cc:dd:ee:ff"));
    }

    #[test]
    fn test_parse_ifname_policy_invalid() {
        // No policy at all (bare MAC) -> dropped.
        assert!(parse_cmdline("net.ifname_policy=aa:bb:cc:dd:ee:ff")
            .ifname_policies
            .is_empty());
        // MAC not last -> dropped.
        assert!(parse_cmdline("net.ifname_policy=aa:bb:cc:dd:ee:ff,path")
            .ifname_policies
            .is_empty());
        // Malformed MAC (and not a policy) -> dropped.
        assert!(parse_cmdline("net.ifname_policy=path,bogus")
            .ifname_policies
            .is_empty());
    }

    #[test]
    fn test_generate_ifname_policy_default() {
        let config = parse_cmdline("net.ifname_policy=keep,kernel,path");
        let files = generate(&config);
        let content = files
            .files
            .get("72-default.link")
            .expect("72-default.link");
        assert_eq!(
            content,
            "# Automatically generated by systemd-network-generator\n\
             \n\
             [Match]\n\
             OriginalName=*\n\
             \n\
             [Link]\n\
             NamePolicy=keep kernel path\n\
             AlternativeNamesPolicy=path\n"
        );
    }

    #[test]
    fn test_generate_ifname_policy_no_alt() {
        // A pure NamePolicy (keep) yields no AlternativeNamesPolicy line.
        let config = parse_cmdline("net.ifname_policy=keep");
        let content = &generate(&config).files["72-default.link"];
        assert!(content.contains("NamePolicy=keep\n"));
        assert!(!content.contains("AlternativeNamesPolicy"));
    }

    #[test]
    fn test_generate_ifname_policy_with_mac() {
        let config = parse_cmdline("net.ifname_policy=path,mac,aa:bb:cc:dd:ee:ff");
        let files = generate(&config);
        let content = files
            .files
            .get("71-aabbccddeeff.link")
            .expect("71-aabbccddeeff.link");
        assert_eq!(
            content,
            "# Automatically generated by systemd-network-generator\n\
             \n\
             [Match]\n\
             MACAddress=aa:bb:cc:dd:ee:ff\n\
             \n\
             [Link]\n\
             NamePolicy=path mac\n\
             AlternativeNamesPolicy=path mac\n"
        );
    }

    #[test]
    fn test_generate_vlan() {
        // C: a 70-<vlan>.netdev plus the parent's 70-<parent>.network with VLAN=.
        // The raw interface name (with its dot) is used in the filename.
        let config = parse_cmdline("vlan=eth0.100:eth0");
        let files = generate(&config);

        let netdev = &files.files["70-eth0.100.netdev"];
        assert!(netdev.contains("[NetDev]\nKind=vlan\nName=eth0.100\n"), "{netdev}");
        assert!(netdev.contains("[VLAN]\nId=100\n"), "{netdev}");

        let network = &files.files["70-eth0.network"];
        assert!(network.contains("Name=eth0"), "{network}");
        assert!(network.contains("VLAN=eth0.100"), "{network}");
        // The vlan device itself gets no .network (only the netdev).
        assert!(!files.files.contains_key("70-eth0.100.network"));
    }

    #[test]
    fn test_generate_bond() {
        let config = parse_cmdline("bond=bond0:eth0,eth1:mode=802.3ad,miimon=100:9000");
        let files = generate(&config);

        let netdev = &files.files["70-bond0.netdev"];
        assert!(netdev.contains("Kind=bond"), "{netdev}");
        assert!(netdev.contains("Name=bond0"), "{netdev}");
        assert!(netdev.contains("MTUBytes=9000"), "{netdev}");
        // C's netdev_dump emits no [Bond] section, so bond options are dropped.
        assert!(!netdev.contains("[Bond]"), "{netdev}");

        // Each member gets its own 70-<member>.network with Bond=.
        let eth0 = &files.files["70-eth0.network"];
        assert!(eth0.contains("Name=eth0"), "{eth0}");
        assert!(eth0.contains("Bond=bond0"), "{eth0}");
        assert!(files.files["70-eth1.network"].contains("Bond=bond0"));
    }

    #[test]
    fn test_generate_bridge() {
        let config = parse_cmdline("bridge=br0:eth0,eth1");
        let files = generate(&config);

        let netdev = &files.files["70-br0.netdev"];
        assert!(netdev.contains("Kind=bridge"), "{netdev}");
        assert!(netdev.contains("Name=br0"), "{netdev}");

        assert!(files.files["70-eth0.network"].contains("Bridge=br0"));
        assert!(files.files["70-eth1.network"].contains("Bridge=br0"));
    }

    #[test]
    fn test_generate_team() {
        // team= is not a C kernel-command-line option, so C emits nothing.
        let config = parse_cmdline("team=team0:eth0,eth1");
        let files = generate(&config);
        assert!(
            files.files.is_empty(),
            "{:?}",
            files.files.keys().collect::<Vec<_>>()
        );
    }

    #[test]
    fn test_bond_member_merges_ip() {
        // A bond member that also has ip= gets ONE merged 70-<member>.network
        // with both DHCP and Bond= (C keys Networks by interface).
        let config = parse_cmdline("bond=bond0:eth0,eth1 ip=eth0:dhcp");
        let files = generate(&config);
        let eth0 = &files.files["70-eth0.network"];
        assert!(eth0.contains("DHCP=ipv4"), "{eth0}");
        assert!(eth0.contains("Bond=bond0"), "{eth0}");
    }

    #[test]
    fn test_bridge_member_merges_ip() {
        let config = parse_cmdline("bridge=br0:eth0 ip=eth0:dhcp");
        let files = generate(&config);
        let eth0 = &files.files["70-eth0.network"];
        assert!(eth0.contains("DHCP=ipv4"), "{eth0}");
        assert!(eth0.contains("Bridge=br0"), "{eth0}");
    }

    #[test]
    fn test_generate_route_with_device() {
        // A route-only device gets its own 70-<dev>.network with the [Route].
        let config = parse_cmdline("rd.route=10.0.0.0/8:192.168.1.1:eth0");
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("Name=eth0"), "{content}");
        assert!(content.contains("[Route]"), "{content}");
        assert!(content.contains("Destination=10.0.0.0/8"), "{content}");
        assert!(content.contains("Gateway=192.168.1.1"), "{content}");
    }

    #[test]
    fn test_generate_route_without_device() {
        // An unbound route goes into the deviceless 71-default.network.
        let config = parse_cmdline("rd.route=10.0.0.0/8:192.168.1.1");
        let files = generate(&config);
        let content = &files.files["71-default.network"];
        assert!(content.contains("Kind=!*"), "{content}");
        assert!(content.contains("Destination=10.0.0.0/8"), "{content}");
    }

    #[test]
    fn test_generate_route_not_duplicated_with_ip() {
        // If there's an ip= config for the same device, don't generate a separate route file.
        let config = parse_cmdline("ip=eth0:dhcp rd.route=10.0.0.0/8:192.168.1.1:eth0");
        let files = generate(&config);
        // Should have the ip= network file, not a separate route file.
        assert!(files.files.contains_key("70-eth0.network"));
        assert!(!files.files.contains_key("71-route-eth0.network"));
    }

    #[test]
    fn test_generate_dhcp_hostname() {
        // Test the 7-field form with hostname and device.
        let _config = parse_cmdline("ip=::::::dhcp");
        let ip = parse_ip_param("::::myhost:eth0:dhcp").unwrap();
        assert_eq!(ip.hostname, "myhost");
        assert_eq!(ip.device, "eth0");
        assert_eq!(ip.autoconf, "dhcp");
    }

    // ── File write tests ────────────────────────────────────────────

    #[test]
    fn test_generated_files_write() {
        let dir = tempfile::tempdir().unwrap();
        let mut files = GeneratedFiles::new();
        files.add(
            "test.network".to_string(),
            "[Match]\nName=eth0\n".to_string(),
        );
        files.add(
            "test.netdev".to_string(),
            "[NetDev]\nName=br0\n".to_string(),
        );

        let count = files.write_to(dir.path()).unwrap();
        assert_eq!(count, 2);
        assert!(dir.path().join("test.network").exists());
        assert!(dir.path().join("test.netdev").exists());

        let content = fs::read_to_string(dir.path().join("test.network")).unwrap();
        assert!(content.contains("Name=eth0"));
    }

    #[test]
    fn test_generated_files_write_creates_dir() {
        let dir = tempfile::tempdir().unwrap();
        let subdir = dir.path().join("sub").join("dir");
        let mut files = GeneratedFiles::new();
        files.add("test.network".to_string(), "content".to_string());

        let count = files.write_to(&subdir).unwrap();
        assert_eq!(count, 1);
        assert!(subdir.join("test.network").exists());
    }

    #[test]
    fn test_generated_files_empty_write() {
        let dir = tempfile::tempdir().unwrap();
        let files = GeneratedFiles::new();
        let count = files.write_to(dir.path()).unwrap();
        assert_eq!(count, 0);
    }

    // ── Integration-style tests ─────────────────────────────────────

    #[test]
    fn test_run_with_cmdline_file() {
        let dir = tempfile::tempdir().unwrap();
        let cmdline_file = dir.path().join("cmdline");
        fs::write(&cmdline_file, "ip=eth0:dhcp nameserver=8.8.8.8").unwrap();

        let output_dir = dir.path().join("output");
        let code = run(cmdline_file.to_str().unwrap(), &output_dir);
        assert_eq!(code, 0);
        assert!(output_dir.join("70-eth0.network").exists());
    }

    #[test]
    fn test_run_with_no_network_params() {
        let dir = tempfile::tempdir().unwrap();
        let cmdline_file = dir.path().join("cmdline");
        fs::write(&cmdline_file, "ro root=UUID=xxx quiet splash").unwrap();

        let output_dir = dir.path().join("output");
        let code = run(cmdline_file.to_str().unwrap(), &output_dir);
        assert_eq!(code, 0);
        // No files should be generated.
        assert!(!output_dir.exists());
    }

    #[test]
    fn test_run_with_missing_cmdline() {
        let dir = tempfile::tempdir().unwrap();
        let output_dir = dir.path().join("output");
        let code = run("/nonexistent/cmdline", &output_dir);
        assert_eq!(code, 0);
    }

    #[test]
    fn test_run_complex_cmdline() {
        let dir = tempfile::tempdir().unwrap();
        let cmdline_file = dir.path().join("cmdline");
        fs::write(
            &cmdline_file,
            "ro ip=eth0:dhcp bond=bond0:eth1,eth2:mode=802.3ad vlan=bond0.100:bond0 \
             bridge=br0:eth3 nameserver=1.1.1.1 ifname=lan0:aa:bb:cc:dd:ee:ff \
             rd.route=10.0.0.0/8:172.16.0.1:eth0 net.ifnames=0",
        )
        .unwrap();

        let output_dir = dir.path().join("output");
        let code = run(cmdline_file.to_str().unwrap(), &output_dir);
        assert_eq!(code, 0);

        // Verify key files exist, in C's merged-per-interface naming.
        assert!(output_dir.join("70-eth0.network").exists()); // ip=dhcp + route
        assert!(output_dir.join("70-bond0.netdev").exists());
        assert!(output_dir.join("70-eth1.network").exists()); // bond member
        assert!(output_dir.join("70-eth2.network").exists()); // bond member
        assert!(output_dir.join("70-bond0.100.netdev").exists()); // vlan on bond0
        assert!(output_dir.join("70-br0.netdev").exists());
        assert!(output_dir.join("70-eth3.network").exists()); // bridge member
        assert!(output_dir.join("70-lan0.link").exists());
        // net.ifnames= is a udev concern; the generator writes no file for it.
        assert!(!output_dir.join("71-net-ifnames.link").exists());
    }

    #[test]
    fn test_generate_multiple_vlans() {
        let config = parse_cmdline("vlan=eth0.100:eth0 vlan=eth0.200:eth0");
        let files = generate(&config);
        // Two netdevs plus ONE merged parent network carrying both VLANs.
        assert_eq!(
            files.files.len(),
            3,
            "{:?}",
            files.files.keys().collect::<Vec<_>>()
        );
        assert!(files.files.contains_key("70-eth0.100.netdev"));
        assert!(files.files.contains_key("70-eth0.200.netdev"));
        let parent = &files.files["70-eth0.network"];
        assert!(parent.contains("VLAN=eth0.100"), "{parent}");
        assert!(parent.contains("VLAN=eth0.200"), "{parent}");
    }

    #[test]
    fn test_generate_static_ipv6() {
        let _config = parse_cmdline("ip=2001:db8::1::fe80::1:64::eth0:none");
        // IPv6 addresses contain colons so the simple ':' split doesn't work for ip=.
        // In practice, kernel ip= with IPv6 is unusual; test via direct struct construction.
        let ip = IpConfig {
            client_ip: "2001:db8::1".to_string(),
            netmask: "64".to_string(),
            gateway: "fe80::1".to_string(),
            device: "eth0".to_string(),
            autoconf: "none".to_string(),
            ..Default::default()
        };
        let config = CmdlineConfig {
            ip_configs: vec![ip],
            ..Default::default()
        };
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("Address=2001:db8::1/64"));
        assert!(content.contains("Gateway=fe80::1"));
    }

    #[test]
    fn test_generated_files_are_valid_ini() {
        // Basic validation: every generated file should have [Match] or [NetDev] section.
        let config = parse_cmdline(
            "ip=eth0:dhcp bond=bond0:eth1,eth2 bridge=br0:eth3 vlan=eth0.100:eth0 ifname=lan0:aa:bb:cc:dd:ee:ff",
        );
        let files = generate(&config);
        for (name, content) in &files.files {
            assert!(
                content.contains('['),
                "File {} has no section headers: {}",
                name,
                content,
            );
        }
    }

    #[test]
    fn test_generate_dhcp_with_dhcpv4_hostname() {
        let ip = IpConfig {
            device: "eth0".to_string(),
            autoconf: "dhcp".to_string(),
            hostname: "myhost".to_string(),
            ..Default::default()
        };
        let config = CmdlineConfig {
            ip_configs: vec![ip],
            ..Default::default()
        };
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        // C writes the hostname as a bare Hostname= in the [DHCP] section.
        assert!(content.contains("[DHCP]"));
        assert!(content.contains("Hostname=myhost"));
    }

    #[test]
    fn test_generate_dhcp_peerdns_off() {
        let ip = IpConfig {
            device: "eth0".to_string(),
            autoconf: "dhcp".to_string(),
            ..Default::default()
        };
        let config = CmdlineConfig {
            ip_configs: vec![ip],
            peer_dns: Some(false),
            ..Default::default()
        };
        let files = generate(&config);
        let content = &files.files["70-eth0.network"];
        assert!(content.contains("[DHCP]"));
        assert!(content.contains("UseDNS=no"));
    }

    #[test]
    fn test_looks_like_ip() {
        assert!(looks_like_ip("192.168.1.1"));
        assert!(looks_like_ip("2001:db8::1"));
        assert!(!looks_like_ip("dhcp"));
        assert!(!looks_like_ip("eth0"));
    }

    #[test]
    fn test_ip_case_insensitive() {
        let ip = parse_ip_param("DHCP").unwrap();
        assert_eq!(ip.autoconf, "dhcp");

        let ip = parse_ip_param("eth0:DHCP6").unwrap();
        assert_eq!(ip.autoconf, "dhcp6");
    }

    #[test]
    fn test_bond_no_options_no_bond_section() {
        let config = parse_cmdline("bond=bond0:eth0,eth1");
        let files = generate(&config);
        let netdev = &files.files["70-bond0.netdev"];
        assert!(!netdev.contains("[Bond]"));
    }

    #[test]
    fn test_route_merges_into_ip_network() {
        // A device-bound rd.route= must merge into the interface's own .network
        // (C behavior): systemd applies only one matching .network per link, so
        // a route emitted as a separate file would be silently dropped.
        let config = parse_cmdline("ip=eth0:dhcp rd.route=10.1.0.0/16:192.168.1.1:eth0");
        let files = generate(&config);
        assert!(
            !files.files.keys().any(|k| k.contains("route")),
            "route must not be a separate file: {:?}",
            files.files.keys().collect::<Vec<_>>()
        );
        let net = files.files.get("70-eth0.network").expect("70-eth0.network");
        assert!(net.contains("DHCP=ipv4"), "{net}");
        assert!(
            net.contains("[Route]\nDestination=10.1.0.0/16\nGateway=192.168.1.1"),
            "{net}"
        );
    }

    /// Robustness fuzz (task #22): the kernel-command-line parser and the
    /// generator consume fully untrusted `ip=`/`vlan=`/`bond=`/`bridge=`/
    /// `rd.route=`/`ifname=`/`net.ifname_policy=`/... strings. Neither
    /// `parse_cmdline` nor `generate` may panic (out-of-bounds index, integer
    /// parse/overflow, bad slice) on any input. Feed deterministic random token
    /// soup and assert no panic.
    #[test]
    fn fuzz_cmdline_parser_and_generator_never_panic() {
        const TOKENS: &[&str] = &[
            "ip=",
            "ip=dhcp",
            "ip=dhcp6",
            "ip=auto6",
            "ip=on",
            "ip=off",
            "ip=:::::",
            "ip=eth0:dhcp",
            "ip=1.2.3.4::5.6.7.8:24:h:eth0:none:8.8.8.8:1.1.1.1:0.pool",
            "vlan=",
            "vlan=v.10:eth0",
            "vlan=eth0.4095:eth0",
            "vlan=x:99999999999",
            "bond=",
            "bond=b0:e1,e2:mode=1:1500",
            "bridge=",
            "bridge=br0:e1,e2",
            "team=t0:e1,e2",
            "ifname=",
            "ifname=lan0:00:11:22:33:44:55",
            "net.ifnames=0",
            "net.ifname_policy=",
            "net.ifname_policy=keep,mac,X",
            "nameserver=8.8.8.8",
            "rd.peerdns=0",
            "rd.route=",
            "rd.route=1.2.3.0/24:9.9.9.9:eth0",
            ":", ",", ".", "/", "=", "-", " ", "eth0", "99999999999999999999",
            "", "\t", "%", "::", "0/0", "4095", "65536", "gw",
        ];
        let mut state: u64 = 0x1234_5678_9abc_def0;
        let mut next = || {
            state = state
                .wrapping_mul(6_364_136_223_846_793_005)
                .wrapping_add(1_442_695_040_888_963_407);
            (state >> 33) as u32
        };
        for _ in 0..100_000u32 {
            let ntok = (next() % 8) as usize;
            let mut input = String::new();
            for _ in 0..ntok {
                if !input.is_empty() {
                    input.push(' ');
                }
                if next() % 7 == 0 {
                    input.push(char::from_u32(next() % 0x100).unwrap_or('?'));
                } else {
                    input.push_str(TOKENS[(next() as usize) % TOKENS.len()]);
                }
            }
            let buf = input.clone();
            let res = std::panic::catch_unwind(move || {
                let config = parse_cmdline(&input);
                let _ = generate(&config);
            });
            assert!(res.is_ok(), "cmdline parser/generator panicked on: {buf:?}");
        }
    }
}
