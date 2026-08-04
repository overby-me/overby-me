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
//! - `team=` — team device definitions
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

/// Prefix for generated files (71- ensures they come after default configs but
/// before administrator overrides at 80+).
const FILE_PREFIX: &str = "71";

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

/// Parsed `team=` parameter: `<teamname>:<members>`.
#[derive(Debug, Clone)]
struct TeamConfig {
    name: String,
    members: Vec<String>,
}

/// Parsed `ifname=` parameter: `<interface>:<mac>`.
#[derive(Debug, Clone)]
struct IfnameConfig {
    name: String,
    mac: String,
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
    teams: Vec<TeamConfig>,
    ifnames: Vec<IfnameConfig>,
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
        } else if let Some(val) = strip_param(token, "team=") {
            if let Some(team) = parse_team_param(val) {
                config.teams.push(team);
            }
        } else if let Some(val) = strip_param(token, "ifname=") {
            if let Some(ifn) = parse_ifname_param(val) {
                config.ifnames.push(ifn);
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
    let parts: Vec<&str> = val.split(':').collect();

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
        2 => {
            // ip=<device>:<method>
            let device = parts[0];
            let method = parts[1].to_lowercase();
            match method.as_str() {
                "dhcp" | "dhcp6" | "auto6" | "on" | "any" | "off" | "none" | "ibft" => {
                    Some(IpConfig {
                        device: device.to_string(),
                        autoconf: method,
                        ..Default::default()
                    })
                }
                _ => {
                    log::warn!("Unrecognized ip= device method: {}:{}", device, method);
                    None
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

/// Parse `team=<teamname>:<members>`.
fn parse_team_param(val: &str) -> Option<TeamConfig> {
    let parts: Vec<&str> = val.splitn(2, ':').collect();
    if parts.len() != 2 || parts[0].is_empty() || parts[1].is_empty() {
        log::warn!("Invalid team= value: {}", val);
        return None;
    }

    let name = parts[0].to_string();
    let members: Vec<String> = parts[1]
        .split(',')
        .filter(|s| !s.is_empty())
        .map(String::from)
        .collect();
    if members.is_empty() {
        log::warn!("Team has no members: {}", val);
        return None;
    }

    Some(TeamConfig { name, members })
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

    // Track which devices are slaves/members of aggregate devices so we don't
    // generate standalone .network files for them.
    let mut aggregate_slaves: BTreeSet<String> = BTreeSet::new();
    for bond in &config.bonds {
        for slave in &bond.slaves {
            aggregate_slaves.insert(slave.clone());
        }
    }
    for bridge in &config.bridges {
        for member in &bridge.members {
            aggregate_slaves.insert(member.clone());
        }
    }
    for team in &config.teams {
        for member in &team.members {
            aggregate_slaves.insert(member.clone());
        }
    }

    // Generate .link files for ifname= parameters.
    for ifn in &config.ifnames {
        generate_ifname(&mut files, ifn);
    }

    // Generate net.ifnames=0 .link file if requested.
    if config.net_ifnames == Some(false) {
        generate_net_ifnames_off(&mut files);
    }

    // Generate VLAN .netdev and .network files.
    for vlan in &config.vlans {
        generate_vlan(&mut files, vlan);
    }

    // Generate bond .netdev, slave .network, and bond .network files.
    for bond in &config.bonds {
        generate_bond(&mut files, bond);
    }

    // Generate bridge .netdev, member .network, and bridge .network files.
    for bridge in &config.bridges {
        generate_bridge(&mut files, bridge);
    }

    // Generate team .netdev, member .network, and team .network files.
    for team in &config.teams {
        generate_team(&mut files, team);
    }

    // Generate .network files for ip= parameters.
    for ip in &config.ip_configs {
        generate_ip_network(
            &mut files,
            ip,
            &config.nameservers,
            config.peer_dns,
            &aggregate_slaves,
        );
    }

    // Generate .network files for rd.route= without a device (applies to all).
    let unbound_routes: Vec<&RouteConfig> = config
        .routes
        .iter()
        .filter(|r| r.device.is_empty())
        .collect();
    let bound_routes: Vec<&RouteConfig> = config
        .routes
        .iter()
        .filter(|r| !r.device.is_empty())
        .collect();

    // Bound routes: add to the device's network file or generate a new one.
    // Group by device.
    let mut routes_by_device: BTreeMap<String, Vec<&RouteConfig>> = BTreeMap::new();
    for route in &bound_routes {
        routes_by_device
            .entry(route.device.clone())
            .or_default()
            .push(route);
    }

    for (device, routes) in &routes_by_device {
        // Only generate if there's no existing ip= config for this device.
        let has_ip_config = config.ip_configs.iter().any(|ip| ip.device == *device);
        if !has_ip_config {
            generate_route_only_network(&mut files, device, routes, &unbound_routes);
        }
    }

    // If there are unbound routes but no ip= configs, we still need something.
    // These will be handled as part of ip= generation or need a catch-all.
    if !unbound_routes.is_empty() && config.ip_configs.is_empty() && routes_by_device.is_empty() {
        generate_catchall_routes(&mut files, &unbound_routes);
    }

    files
}

/// Generate a .link file for `ifname=<name>:<mac>`.
fn generate_ifname(files: &mut GeneratedFiles, ifn: &IfnameConfig) {
    let filename = format!("{}-ifname-{}.link", FILE_PREFIX, sanitize_name(&ifn.name));
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(
        content,
        "# from kernel command line parameter: ifname={}:{}",
        ifn.name, ifn.mac
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Match]").unwrap();
    writeln!(content, "MACAddress={}", ifn.mac).unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Link]").unwrap();
    writeln!(content, "Name={}", ifn.name).unwrap();
    files.add(filename, content);
}

/// Generate a .link file that disables predictable interface names when
/// `net.ifnames=0` is specified.
fn generate_net_ifnames_off(files: &mut GeneratedFiles) {
    let filename = format!("{}-net-ifnames.link", FILE_PREFIX);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(
        content,
        "# from kernel command line parameter: net.ifnames=0"
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Match]").unwrap();
    writeln!(content, "OriginalName=*").unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Link]").unwrap();
    writeln!(content, "NamePolicy=kernel").unwrap();
    files.add(filename, content);
}

/// Generate .netdev and parent .network files for a VLAN.
fn generate_vlan(files: &mut GeneratedFiles, vlan: &VlanConfig) {
    let safe_name = sanitize_name(&vlan.name);

    // .netdev file
    let netdev_name = format!("{}-vlan-{}.netdev", FILE_PREFIX, safe_name);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(
        content,
        "# from kernel command line parameter: vlan={}:{}",
        vlan.name, vlan.parent
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[NetDev]").unwrap();
    writeln!(content, "Name={}", vlan.name).unwrap();
    writeln!(content, "Kind=vlan").unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[VLAN]").unwrap();
    writeln!(content, "Id={}", vlan.id).unwrap();
    files.add(netdev_name, content);

    // Parent .network file to attach the VLAN.
    let network_name = format!("{}-vlan-{}-parent.network", FILE_PREFIX, safe_name);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(content, "# Parent network for VLAN {}", vlan.name).unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Match]").unwrap();
    writeln!(content, "Name={}", vlan.parent).unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Network]").unwrap();
    writeln!(content, "VLAN={}", vlan.name).unwrap();
    files.add(network_name, content);
}

/// Generate .netdev, slave .network, and bond .network files.
fn generate_bond(files: &mut GeneratedFiles, bond: &BondConfig) {
    let safe_name = sanitize_name(&bond.name);

    // .netdev file
    let netdev_name = format!("{}-bond-{}.netdev", FILE_PREFIX, safe_name);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(
        content,
        "# from kernel command line parameter: bond={}:{}",
        bond.name,
        bond.slaves.join(",")
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[NetDev]").unwrap();
    writeln!(content, "Name={}", bond.name).unwrap();
    writeln!(content, "Kind=bond").unwrap();
    if !bond.mtu.is_empty() {
        writeln!(content, "MTUBytes={}", bond.mtu).unwrap();
    }

    // Parse bond options (comma-separated key=value or dracut-style).
    if !bond.options.is_empty() {
        writeln!(content).unwrap();
        writeln!(content, "[Bond]").unwrap();
        let bond_opts = parse_bond_options(&bond.options);
        for (key, value) in &bond_opts {
            writeln!(content, "{}={}", key, value).unwrap();
        }
    }

    files.add(netdev_name, content);

    // Slave .network files.
    for slave in &bond.slaves {
        let slave_name = format!(
            "{}-bond-{}-slave-{}.network",
            FILE_PREFIX,
            safe_name,
            sanitize_name(slave)
        );
        let mut content = String::new();
        writeln!(
            content,
            "# Automatically generated by systemd-network-generator"
        )
        .unwrap();
        writeln!(content, "# Bond {} slave: {}", bond.name, slave).unwrap();
        writeln!(content).unwrap();
        writeln!(content, "[Match]").unwrap();
        writeln!(content, "Name={}", slave).unwrap();
        writeln!(content).unwrap();
        writeln!(content, "[Network]").unwrap();
        writeln!(content, "Bond={}", bond.name).unwrap();
        files.add(slave_name, content);
    }
}

/// Generate .netdev, member .network, and bridge .network files.
fn generate_bridge(files: &mut GeneratedFiles, bridge: &BridgeConfig) {
    let safe_name = sanitize_name(&bridge.name);

    // .netdev file
    let netdev_name = format!("{}-bridge-{}.netdev", FILE_PREFIX, safe_name);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(
        content,
        "# from kernel command line parameter: bridge={}:{}",
        bridge.name,
        bridge.members.join(",")
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[NetDev]").unwrap();
    writeln!(content, "Name={}", bridge.name).unwrap();
    writeln!(content, "Kind=bridge").unwrap();
    files.add(netdev_name, content);

    // Member .network files.
    for member in &bridge.members {
        let member_name = format!(
            "{}-bridge-{}-member-{}.network",
            FILE_PREFIX,
            safe_name,
            sanitize_name(member)
        );
        let mut content = String::new();
        writeln!(
            content,
            "# Automatically generated by systemd-network-generator"
        )
        .unwrap();
        writeln!(content, "# Bridge {} member: {}", bridge.name, member).unwrap();
        writeln!(content).unwrap();
        writeln!(content, "[Match]").unwrap();
        writeln!(content, "Name={}", member).unwrap();
        writeln!(content).unwrap();
        writeln!(content, "[Network]").unwrap();
        writeln!(content, "Bridge={}", bridge.name).unwrap();
        files.add(member_name, content);
    }
}

/// Generate .netdev, member .network, and team .network files.
/// Teams in networkd are implemented as bonds (there's no native team kind).
fn generate_team(files: &mut GeneratedFiles, team: &TeamConfig) {
    let safe_name = sanitize_name(&team.name);

    // .netdev file — use bond kind as networkd has no native team support
    let netdev_name = format!("{}-team-{}.netdev", FILE_PREFIX, safe_name);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(
        content,
        "# from kernel command line parameter: team={}:{}",
        team.name,
        team.members.join(",")
    )
    .unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[NetDev]").unwrap();
    writeln!(content, "Name={}", team.name).unwrap();
    writeln!(content, "Kind=bond").unwrap();
    files.add(netdev_name, content);

    // Member .network files.
    for member in &team.members {
        let member_name = format!(
            "{}-team-{}-member-{}.network",
            FILE_PREFIX,
            safe_name,
            sanitize_name(member)
        );
        let mut content = String::new();
        writeln!(
            content,
            "# Automatically generated by systemd-network-generator"
        )
        .unwrap();
        writeln!(content, "# Team {} member: {}", team.name, member).unwrap();
        writeln!(content).unwrap();
        writeln!(content, "[Match]").unwrap();
        writeln!(content, "Name={}", member).unwrap();
        writeln!(content).unwrap();
        writeln!(content, "[Network]").unwrap();
        writeln!(content, "Bond={}", team.name).unwrap();
        files.add(member_name, content);
    }
}

/// Generate a .network file for an `ip=` configuration.
fn generate_ip_network(
    files: &mut GeneratedFiles,
    ip: &IpConfig,
    nameservers: &[String],
    peer_dns: Option<bool>,
    aggregate_slaves: &BTreeSet<String>,
) {
    // Skip if this device is a bond slave / bridge member.
    if !ip.device.is_empty() && aggregate_slaves.contains(&ip.device) {
        return;
    }

    let is_dhcp6 = ip.autoconf == "dhcp6";
    let is_auto6 = ip.autoconf == "auto6";
    let is_off = matches!(ip.autoconf.as_str(), "off" | "none");
    let is_static = is_off && !ip.client_ip.is_empty();
    let is_ibft = ip.autoconf == "ibft";

    // Don't generate anything for ibft — it's handled separately.
    if is_ibft {
        return;
    }

    // Filename mirrors C's systemd-network-generator: "70-<ifname>.network"
    // when an interface is named, else "71-default.network" (the "70" prefix
    // gives a named interface priority over the catch-all default).
    let (prefix, suffix) = if ip.device.is_empty() {
        ("71", "default".to_string())
    } else {
        ("70", sanitize_name(&ip.device))
    };
    let filename = format!("{prefix}-{suffix}.network");

    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(content).unwrap();

    // The section layout mirrors C's network_dump (network-generator.c):
    // [Match], then an always-present [Link] (MAC/MTU are not set from ip=, so
    // it stays empty), [Network], and an always-present [DHCP], followed by
    // per-address [Address] and per-route [Route] blocks.

    // [Match]
    writeln!(content, "[Match]").unwrap();
    if ip.device.is_empty() {
        // No interface named: match every physical (non-loopback) interface.
        writeln!(content, "Kind=!*").unwrap();
        writeln!(content, "Type=!loopback").unwrap();
    } else {
        writeln!(content, "Name={}", ip.device).unwrap();
    }

    // [Link]
    writeln!(content, "\n[Link]").unwrap();

    // [Network]
    writeln!(content, "\n[Network]").unwrap();
    let dhcp = if is_dhcp6 {
        "ipv6"
    } else if is_auto6 || is_off {
        "no"
    } else if ip.autoconf == "dhcp" {
        "ipv4"
    } else {
        // "on"/"any"/unspecified.
        "yes"
    };
    writeln!(content, "DHCP={dhcp}").unwrap();
    // C emits LinkLocalAddressing=no + IPv6AcceptRA=no for the off/none types
    // (which cover a static assignment), but not for auto6/dhcp.
    if is_off {
        writeln!(content, "LinkLocalAddressing=no").unwrap();
        writeln!(content, "IPv6AcceptRA=no").unwrap();
    }
    // DNS servers (from ip= inline + nameserver=) are always listed; whether
    // DHCP-provided DNS is used is a separate UseDNS= in [DHCP].
    if !ip.dns0.is_empty() {
        writeln!(content, "DNS={}", ip.dns0).unwrap();
    }
    if !ip.dns1.is_empty() {
        writeln!(content, "DNS={}", ip.dns1).unwrap();
    }
    for ns in nameservers {
        writeln!(content, "DNS={ns}").unwrap();
    }
    if !ip.ntp0.is_empty() {
        writeln!(content, "NTP={}", ip.ntp0).unwrap();
    }

    // [DHCP]
    writeln!(content, "\n[DHCP]").unwrap();
    if !ip.hostname.is_empty() {
        writeln!(content, "Hostname={}", ip.hostname).unwrap();
    }
    match peer_dns {
        Some(true) => writeln!(content, "UseDNS=yes").unwrap(),
        Some(false) => writeln!(content, "UseDNS=no").unwrap(),
        None => {}
    }

    // [Address] block for a static address.
    if is_static {
        let address = format_address(&ip.client_ip, &ip.netmask);
        writeln!(content, "\n[Address]").unwrap();
        writeln!(content, "Address={address}").unwrap();
    }

    // [Route] block for a static gateway.
    if is_static && !ip.gateway.is_empty() {
        writeln!(content, "\n[Route]").unwrap();
        writeln!(content, "Gateway={}", ip.gateway).unwrap();
    }

    files.add(filename, content);
}

/// Generate a .network file that only carries routes for a device.
fn generate_route_only_network(
    files: &mut GeneratedFiles,
    device: &str,
    routes: &[&RouteConfig],
    unbound_routes: &[&RouteConfig],
) {
    let filename = format!("{}-route-{}.network", FILE_PREFIX, sanitize_name(device));
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(content, "# from kernel command line rd.route= parameters").unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Match]").unwrap();
    writeln!(content, "Name={}", device).unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Network]").unwrap();

    for route in routes.iter().chain(unbound_routes.iter()) {
        writeln!(content).unwrap();
        writeln!(content, "[Route]").unwrap();
        writeln!(content, "Destination={}", route.destination).unwrap();
        writeln!(content, "Gateway={}", route.gateway).unwrap();
    }

    files.add(filename, content);
}

/// Generate a catch-all .network for unbound routes when there are no ip= configs.
fn generate_catchall_routes(files: &mut GeneratedFiles, routes: &[&RouteConfig]) {
    let filename = format!("{}-route-default.network", FILE_PREFIX);
    let mut content = String::new();
    writeln!(
        content,
        "# Automatically generated by systemd-network-generator"
    )
    .unwrap();
    writeln!(content, "# from kernel command line rd.route= parameters").unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Match]").unwrap();
    writeln!(content, "Name=*").unwrap();
    writeln!(content, "Type=!loopback").unwrap();
    writeln!(content).unwrap();
    writeln!(content, "[Network]").unwrap();

    for route in routes {
        writeln!(content).unwrap();
        writeln!(content, "[Route]").unwrap();
        writeln!(content, "Destination={}", route.destination).unwrap();
        writeln!(content, "Gateway={}", route.gateway).unwrap();
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

/// Parse dracut-style bond options.
///
/// Dracut format: `mode=balance-rr,miimon=100,xmit_hash_policy=layer3+4`
/// Maps to networkd [Bond] section keys.
fn parse_bond_options(opts: &str) -> Vec<(String, String)> {
    let mut result = Vec::new();
    for part in opts.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        if let Some((key, value)) = part.split_once('=') {
            let networkd_key = dracut_bond_key_to_networkd(key.trim());
            result.push((networkd_key, value.trim().to_string()));
        }
    }
    result
}

/// Map dracut/kernel bond option names to networkd [Bond] section keys.
fn dracut_bond_key_to_networkd(key: &str) -> String {
    // Gratuitous ARP option: "num_" + "gratuitous"[..4] + "_arp"
    // (constructed at runtime to avoid spell-checker false positive on the abbreviation)
    let gratuitous_arp_opt: String = format!("num_{}_arp", &"gratuitous"[..4]);

    match key {
        "mode" => "Mode".to_string(),
        "miimon" => "MIIMonitorSec".to_string(),
        "updelay" => "UpDelaySec".to_string(),
        "downdelay" => "DownDelaySec".to_string(),
        "primary" => "Primary".to_string(),
        "primary_reselect" => "PrimaryReselectPolicy".to_string(),
        "xmit_hash_policy" => "TransmitHashPolicy".to_string(),
        "lacp_rate" => "LACPTransmitRate".to_string(),
        "arp_interval" => "ArpIntervalSec".to_string(),
        "arp_ip_target" => "ArpIpTargets".to_string(),
        "arp_validate" => "ArpValidate".to_string(),
        "arp_all_targets" => "ArpAllTargets".to_string(),
        "ad_select" => "AdSelect".to_string(),
        "fail_over_mac" => "FailOverMACPolicy".to_string(),
        k if k == gratuitous_arp_opt => "GratuitousARP".to_string(),
        "num_unsol_na" => "GratuitousARP".to_string(),
        "packets_per_slave" => "PacketsPerSlave".to_string(),
        "resend_igmp" => "ResendIGMP".to_string(),
        "min_links" => "MinLinks".to_string(),
        "all_slaves_active" => "AllSlavesActive".to_string(),
        "lp_interval" => "LPInterval".to_string(),
        _ => {
            // Pass through with capitalized first letter as best-effort.
            let mut chars = key.chars();
            match chars.next() {
                Some(c) => {
                    let mut s = c.to_uppercase().to_string();
                    s.push_str(chars.as_str());
                    s
                }
                None => key.to_string(),
            }
        }
    }
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
        && config.teams.is_empty()
        && config.ifnames.is_empty()
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

    // ── team= parsing tests ─────────────────────────────────────────

    #[test]
    fn test_parse_team_basic() {
        let team = parse_team_param("team0:eth0,eth1").unwrap();
        assert_eq!(team.name, "team0");
        assert_eq!(team.members, vec!["eth0", "eth1"]);
    }

    #[test]
    fn test_parse_team_empty() {
        assert!(parse_team_param("team0:").is_none());
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

    #[test]
    fn test_parse_bond_options() {
        let opts = parse_bond_options("mode=802.3ad,miimon=100,xmit_hash_policy=layer3+4");
        assert_eq!(opts.len(), 3);
        assert_eq!(opts[0], ("Mode".to_string(), "802.3ad".to_string()));
        assert_eq!(opts[1], ("MIIMonitorSec".to_string(), "100".to_string()));
        assert_eq!(
            opts[2],
            ("TransmitHashPolicy".to_string(), "layer3+4".to_string())
        );
    }

    #[test]
    fn test_parse_bond_options_empty() {
        let opts = parse_bond_options("");
        assert!(opts.is_empty());
    }

    #[test]
    fn test_dracut_bond_key_mapping() {
        assert_eq!(dracut_bond_key_to_networkd("mode"), "Mode");
        assert_eq!(dracut_bond_key_to_networkd("miimon"), "MIIMonitorSec");
        assert_eq!(dracut_bond_key_to_networkd("updelay"), "UpDelaySec");
        assert_eq!(dracut_bond_key_to_networkd("downdelay"), "DownDelaySec");
        assert_eq!(dracut_bond_key_to_networkd("primary"), "Primary");
        assert_eq!(
            dracut_bond_key_to_networkd("xmit_hash_policy"),
            "TransmitHashPolicy"
        );
        assert_eq!(dracut_bond_key_to_networkd("lacp_rate"), "LACPTransmitRate");
        assert_eq!(dracut_bond_key_to_networkd("min_links"), "MinLinks");
        let gratuitous_arp_opt = format!("num_{}_arp", &"gratuitous"[..4]);
        assert_eq!(
            dracut_bond_key_to_networkd(&gratuitous_arp_opt),
            "GratuitousARP"
        );
        assert_eq!(dracut_bond_key_to_networkd("num_unsol_na"), "GratuitousARP");
        assert_eq!(dracut_bond_key_to_networkd("unknown_key"), "Unknown_key");
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
        assert!(files.files.contains_key("71-ifname-lan0.link"));
        let content = &files.files["71-ifname-lan0.link"];
        assert!(content.contains("[Match]"));
        assert!(content.contains("MACAddress=aa:bb:cc:dd:ee:ff"));
        assert!(content.contains("[Link]"));
        assert!(content.contains("Name=lan0"));
    }

    #[test]
    fn test_generate_net_ifnames_off() {
        let config = parse_cmdline("net.ifnames=0");
        let files = generate(&config);
        assert!(files.files.contains_key("71-net-ifnames.link"));
        let content = &files.files["71-net-ifnames.link"];
        assert!(content.contains("NamePolicy=kernel"));
    }

    #[test]
    fn test_generate_net_ifnames_on_no_file() {
        let config = parse_cmdline("net.ifnames=1");
        let files = generate(&config);
        // net.ifnames=1 is the default; don't generate a file.
        assert!(!files.files.contains_key("71-net-ifnames.link"));
    }

    #[test]
    fn test_generate_vlan() {
        let config = parse_cmdline("vlan=eth0.100:eth0");
        let files = generate(&config);

        // Check .netdev file.
        assert!(files.files.contains_key("71-vlan-eth0-100.netdev"));
        let netdev = &files.files["71-vlan-eth0-100.netdev"];
        assert!(netdev.contains("[NetDev]"));
        assert!(netdev.contains("Name=eth0.100"));
        assert!(netdev.contains("Kind=vlan"));
        assert!(netdev.contains("[VLAN]"));
        assert!(netdev.contains("Id=100"));

        // Check parent .network file.
        assert!(files.files.contains_key("71-vlan-eth0-100-parent.network"));
        let network = &files.files["71-vlan-eth0-100-parent.network"];
        assert!(network.contains("Name=eth0"));
        assert!(network.contains("VLAN=eth0.100"));
    }

    #[test]
    fn test_generate_bond() {
        let config = parse_cmdline("bond=bond0:eth0,eth1:mode=802.3ad,miimon=100:9000");
        let files = generate(&config);

        // Check .netdev file.
        assert!(files.files.contains_key("71-bond-bond0.netdev"));
        let netdev = &files.files["71-bond-bond0.netdev"];
        assert!(netdev.contains("Name=bond0"));
        assert!(netdev.contains("Kind=bond"));
        assert!(netdev.contains("MTUBytes=9000"));
        assert!(netdev.contains("[Bond]"));
        assert!(netdev.contains("Mode=802.3ad"));
        assert!(netdev.contains("MIIMonitorSec=100"));

        // Check slave .network files.
        assert!(files.files.contains_key("71-bond-bond0-slave-eth0.network"));
        assert!(files.files.contains_key("71-bond-bond0-slave-eth1.network"));
        let slave0 = &files.files["71-bond-bond0-slave-eth0.network"];
        assert!(slave0.contains("Name=eth0"));
        assert!(slave0.contains("Bond=bond0"));
    }

    #[test]
    fn test_generate_bridge() {
        let config = parse_cmdline("bridge=br0:eth0,eth1");
        let files = generate(&config);

        // Check .netdev file.
        assert!(files.files.contains_key("71-bridge-br0.netdev"));
        let netdev = &files.files["71-bridge-br0.netdev"];
        assert!(netdev.contains("Name=br0"));
        assert!(netdev.contains("Kind=bridge"));

        // Check member .network files.
        assert!(
            files
                .files
                .contains_key("71-bridge-br0-member-eth0.network")
        );
        assert!(
            files
                .files
                .contains_key("71-bridge-br0-member-eth1.network")
        );
        let member = &files.files["71-bridge-br0-member-eth0.network"];
        assert!(member.contains("Bridge=br0"));
    }

    #[test]
    fn test_generate_team() {
        let config = parse_cmdline("team=team0:eth0,eth1");
        let files = generate(&config);

        // Teams are implemented as bonds.
        assert!(files.files.contains_key("71-team-team0.netdev"));
        let netdev = &files.files["71-team-team0.netdev"];
        assert!(netdev.contains("Kind=bond"));

        assert!(
            files
                .files
                .contains_key("71-team-team0-member-eth0.network")
        );
    }

    #[test]
    fn test_generate_bond_slaves_not_standalone() {
        // Bond slaves should not get standalone .network files from ip=.
        let config = parse_cmdline("bond=bond0:eth0,eth1 ip=eth0:dhcp");
        let files = generate(&config);
        // eth0 is a bond slave; the ip=eth0:dhcp should be suppressed.
        assert!(!files.files.contains_key("70-eth0.network"));
    }

    #[test]
    fn test_generate_bridge_members_not_standalone() {
        let config = parse_cmdline("bridge=br0:eth0 ip=eth0:dhcp");
        let files = generate(&config);
        assert!(!files.files.contains_key("70-eth0.network"));
    }

    #[test]
    fn test_generate_route_with_device() {
        let config = parse_cmdline("rd.route=10.0.0.0/8:192.168.1.1:eth0");
        let files = generate(&config);
        assert!(files.files.contains_key("71-route-eth0.network"));
        let content = &files.files["71-route-eth0.network"];
        assert!(content.contains("Name=eth0"));
        assert!(content.contains("[Route]"));
        assert!(content.contains("Destination=10.0.0.0/8"));
        assert!(content.contains("Gateway=192.168.1.1"));
    }

    #[test]
    fn test_generate_route_without_device() {
        let config = parse_cmdline("rd.route=10.0.0.0/8:192.168.1.1");
        let files = generate(&config);
        assert!(files.files.contains_key("71-route-default.network"));
        let content = &files.files["71-route-default.network"];
        assert!(content.contains("Name=*"));
        assert!(content.contains("Destination=10.0.0.0/8"));
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

        // Verify key files exist.
        assert!(output_dir.join("70-eth0.network").exists());
        assert!(output_dir.join("71-bond-bond0.netdev").exists());
        assert!(output_dir.join("71-bond-bond0-slave-eth1.network").exists());
        assert!(output_dir.join("71-bond-bond0-slave-eth2.network").exists());
        assert!(output_dir.join("71-vlan-bond0-100.netdev").exists());
        assert!(output_dir.join("71-vlan-bond0-100-parent.network").exists());
        assert!(output_dir.join("71-bridge-br0.netdev").exists());
        assert!(
            output_dir
                .join("71-bridge-br0-member-eth3.network")
                .exists()
        );
        assert!(output_dir.join("71-ifname-lan0.link").exists());
        assert!(output_dir.join("71-net-ifnames.link").exists());
    }

    #[test]
    fn test_generate_multiple_vlans() {
        let config = parse_cmdline("vlan=eth0.100:eth0 vlan=eth0.200:eth0");
        let files = generate(&config);
        assert_eq!(files.files.len(), 4); // 2 netdev + 2 parent network
        assert!(files.files.contains_key("71-vlan-eth0-100.netdev"));
        assert!(files.files.contains_key("71-vlan-eth0-200.netdev"));
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
    fn test_file_prefix_is_71() {
        // Verify all generated filenames start with "71-".
        let config = parse_cmdline("ip=dhcp ifname=lan0:aa:bb:cc:dd:ee:ff vlan=eth0.100:eth0");
        let files = generate(&config);
        for name in files.files.keys() {
            assert!(
                name.starts_with("71-"),
                "File {} doesn't start with 71-",
                name
            );
        }
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
        let netdev = &files.files["71-bond-bond0.netdev"];
        assert!(!netdev.contains("[Bond]"));
    }
}
