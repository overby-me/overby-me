//! Network manager — coordinates `.network` and `.netdev` config files, DHCP
//! clients, static address configuration, and interface lifecycle.
//!
//! This is the core orchestration layer of `systemd-networkd`. It:
//! - Loads `.network` configuration files
//! - Loads `.netdev` configuration files (virtual network device definitions)
//! - Enumerates network interfaces
//! - Matches configs to interfaces
//! - Runs DHCP clients for interfaces with `DHCP=yes`/`DHCP=ipv4`
//! - Applies static addresses and routes
//! - Writes DNS configuration to `/run/systemd/resolve/resolv.conf`
//! - Sends sd_notify status updates

use std::collections::HashMap;
use std::fmt;
use std::net::{Ipv4Addr, Ipv6Addr};

use crate::config::{self, DhcpMode, Ipv6AcceptRa, NetworkConfig, RuleFamily};
use crate::dhcp::{self, DhcpClient, DhcpClientConfig, DhcpLease, DhcpState};
use crate::dhcpv6::{Dhcpv6Client, Dhcpv6ClientConfig, Dhcpv6Lease};
use crate::ipv6_ra::{self, RaAction, RaState, read_machine_id_as_bytes};
use crate::link::{self, LinkInfo, RuleConfig};
use crate::netdev::{self, NetDevConfig};
use crate::netdev_create;

// ---------------------------------------------------------------------------
// Managed link — per-interface state
// ---------------------------------------------------------------------------

/// The configuration + runtime state of a single managed network interface.
#[derive(Debug)]
pub struct ManagedLink {
    /// Kernel link information.
    pub link: LinkInfo,

    /// The `.network` config file that matched this interface (if any).
    pub config: Option<NetworkConfig>,

    /// DHCP client state machine (if DHCP is enabled).
    pub dhcp_client: Option<DhcpClient>,

    /// Current DHCP lease (if obtained).
    pub lease: Option<DhcpLease>,

    /// DHCPv6 client state machine (if DHCPv6 is enabled).
    pub dhcpv6_client: Option<Dhcpv6Client>,

    /// Current DHCPv6 lease (if obtained).
    pub dhcpv6_lease: Option<Dhcpv6Lease>,

    /// Administrative state we want the link to be in.
    pub admin_state: AdminState,

    /// Whether static addresses have been applied.
    pub static_configured: bool,

    /// Whether the link has carrier (physical connection).
    pub has_carrier: bool,

    /// DNS servers collected from DHCP and/or static config.
    pub dns_servers: Vec<Ipv4Addr>,

    /// Search domains collected from DHCP and/or static config.
    pub search_domains: Vec<String>,

    /// IPv6 Router Advertisement state (if RA is enabled).
    pub ra_state: Option<RaState>,

    /// IPv6 DNS servers collected from RA RDNSS and/or DHCPv6.
    pub dns6_servers: Vec<Ipv6Addr>,

    /// IPv6 search domains collected from RA DNSSL and/or DHCPv6.
    pub search6_domains: Vec<String>,

    /// Addresses assigned to this downstream interface by DHCPv6-PD
    /// distribution from an upstream interface's delegated prefix.
    /// Each entry is `(address, prefix_len)`.
    pub pd_assigned_addresses: Vec<(Ipv6Addr, u8)>,
}

/// Desired administrative state of a link.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AdminState {
    /// The link should be brought up and configured.
    Up,
    /// The link should be left alone (unmanaged).
    Unmanaged,
    /// The link should be kept down.
    Down,
}

impl fmt::Display for AdminState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Up => write!(f, "configured"),
            Self::Unmanaged => write!(f, "unmanaged"),
            Self::Down => write!(f, "down"),
        }
    }
}

/// Summary of the operational state of a managed link (for networkctl output).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperState {
    /// No configuration matched.
    Unmanaged,
    /// Configuration matched but no carrier / not configured yet.
    Configuring,
    /// Waiting for DHCP lease.
    Pending,
    /// Fully configured (addresses and routes applied).
    Configured,
    /// Link is degraded (e.g. DHCP failed, using fallback).
    Degraded,
    /// Link has no carrier.
    NoCarrier,
}

impl fmt::Display for OperState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unmanaged => write!(f, "unmanaged"),
            Self::Configuring => write!(f, "configuring"),
            Self::Pending => write!(f, "pending"),
            Self::Configured => write!(f, "configured"),
            Self::Degraded => write!(f, "degraded"),
            Self::NoCarrier => write!(f, "no-carrier"),
        }
    }
}

impl ManagedLink {
    /// Compute the operational state of this link.
    pub fn oper_state(&self) -> OperState {
        if self.config.is_none() {
            return OperState::Unmanaged;
        }
        if !self.has_carrier && !self.link.is_loopback() {
            return OperState::NoCarrier;
        }
        if let Some(ref cfg) = self.config {
            let needs_dhcp = matches!(cfg.network_section.dhcp, DhcpMode::Yes | DhcpMode::Ipv4);
            if needs_dhcp && self.lease.is_none() {
                return OperState::Pending;
            }
        }
        if !self.static_configured {
            return OperState::Configuring;
        }
        OperState::Configured
    }

    /// Operational-state string for the `OPER_STATE=` field of the per-link
    /// state file, using systemd's operational-state vocabulary
    /// (off/no-carrier/carrier/degraded/routable). networkd's internal
    /// `OperState` is a setup-oriented model (configuring/configured/pending)
    /// whose Display strings are NOT recognised by consumers such as
    /// systemd-networkd-wait-online (which map anything unknown to "missing").
    /// Translate so that, e.g., a link that has carrier reports at least
    /// "carrier" and a fully-configured link reports "routable".
    pub fn operational_state_str(&self) -> &'static str {
        match self.oper_state() {
            OperState::Unmanaged => "off",
            OperState::NoCarrier => "no-carrier",
            // Carrier is present but layer-3 configuration is not finished yet
            // (still applying addresses, or waiting for a DHCP lease).
            OperState::Configuring | OperState::Pending => "carrier",
            OperState::Degraded => "degraded",
            OperState::Configured => "routable",
        }
    }
}

// ---------------------------------------------------------------------------
// Network Manager
// ---------------------------------------------------------------------------

/// The main network manager that coordinates all managed links.
pub struct NetworkManager {
    /// All managed links, keyed by interface index.
    pub links: HashMap<u32, ManagedLink>,

    /// Loaded `.network` config files.
    pub configs: Vec<NetworkConfig>,

    /// Loaded `.netdev` config files (virtual network device definitions).
    pub netdev_configs: Vec<NetDevConfig>,

    /// Global DNS servers (aggregated from all links).
    pub dns_servers: Vec<Ipv4Addr>,

    /// Global search domains.
    pub search_domains: Vec<String>,

    /// Global IPv6 DNS servers (aggregated from RA RDNSS).
    pub dns6_servers: Vec<Ipv6Addr>,

    /// Whether we've completed initial configuration.
    pub initial_config_done: bool,
}

impl NetworkManager {
    /// Create a new network manager with no state.
    pub fn new() -> Self {
        Self {
            links: HashMap::new(),
            configs: Vec::new(),
            netdev_configs: Vec::new(),
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
            dns6_servers: Vec::new(),
            initial_config_done: false,
        }
    }

    /// Load `.network` and `.netdev` configuration files from standard paths.
    pub fn load_configs(&mut self) {
        self.configs = config::load_network_configs();
        log::info!("Loaded {} .network config file(s)", self.configs.len());
        for cfg in &self.configs {
            log::debug!(
                "  {} — match=[{}] dhcp={}",
                cfg.path.display(),
                cfg.match_section.names.join(", "),
                cfg.network_section.dhcp,
            );
        }

        self.netdev_configs = netdev::load_netdev_configs();
        log::info!(
            "Loaded {} .netdev config file(s)",
            self.netdev_configs.len()
        );
        for cfg in &self.netdev_configs {
            log::debug!(
                "  {} — name={} kind={}",
                cfg.path.display(),
                cfg.netdev_section.name,
                cfg.netdev_section.kind,
            );
        }
    }

    /// Load configs from explicit directories (for testing).
    pub fn load_configs_from(&mut self, dirs: &[std::path::PathBuf]) {
        self.configs = config::load_network_configs_from(dirs);
        self.netdev_configs = netdev::load_netdev_configs_from(dirs);
    }

    /// Discover network interfaces and match them against configs.
    pub fn discover_links(&mut self) -> Result<(), String> {
        let system_links =
            link::list_links().map_err(|e| format!("failed to enumerate links: {e}"))?;

        for li in system_links {
            // Skip loopback — it's always configured by the kernel.
            if li.is_loopback() {
                continue;
            }

            let ifindex = li.index;
            let mac_str = li.mac.clone();
            let name = li.name.clone();

            // Find the first matching config.
            let matched_config = self.configs.iter().find(|cfg| {
                cfg.match_section
                    .matches_interface(&name, Some(&mac_str), None)
            });

            let admin_state = match matched_config {
                Some(cfg) if cfg.link.unmanaged => AdminState::Unmanaged,
                Some(cfg) => match cfg.link.activation_policy.as_deref() {
                    Some("down") | Some("always-down") => AdminState::Down,
                    Some("manual") => AdminState::Unmanaged,
                    _ => AdminState::Up,
                },
                None => AdminState::Unmanaged,
            };

            let has_carrier = li.is_running() || li.is_loopback();

            let managed = ManagedLink {
                link: li,
                config: matched_config.cloned(),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state,
                static_configured: false,
                has_carrier,
                dns_servers: Vec::new(),
                search_domains: Vec::new(),
                ra_state: None,
                dns6_servers: Vec::new(),
                search6_domains: Vec::new(),
                pd_assigned_addresses: Vec::new(),
            };

            if let Some(cfg) = matched_config {
                log::info!(
                    "Link {} (idx={}) matched config {}",
                    name,
                    ifindex,
                    cfg.path.file_name().unwrap_or_default().to_string_lossy(),
                );
            } else {
                log::debug!(
                    "Link {} (idx={}) — no matching config (unmanaged)",
                    name,
                    ifindex
                );
            }

            self.links.insert(ifindex, managed);
        }

        Ok(())
    }

    /// Create virtual network devices from loaded `.netdev` configurations.
    ///
    /// This should be called after `load_configs()` and before
    /// `discover_links()` / `configure_links()` so that the newly created
    /// virtual interfaces are visible when we enumerate the system's links.
    ///
    /// Standalone devices (bridges, bonds, dummies, tunnels, etc.) are
    /// created first. Dependent devices that need a parent link (VLAN,
    /// macvlan, ipvlan, etc.) are deferred to a second pass.
    ///
    /// Returns the number of devices successfully created.
    pub fn create_netdevs(&self) -> usize {
        if self.netdev_configs.is_empty() {
            return 0;
        }

        log::info!(
            "Creating virtual network devices from {} .netdev config(s)",
            self.netdev_configs.len()
        );

        let created = netdev_create::create_netdevs(&self.netdev_configs);

        if created > 0 {
            log::info!("Created {} virtual network device(s)", created);
        }

        created
    }

    /// Create a dependent netdev (VLAN, macvlan, etc.) with an explicit
    /// parent interface, identified by name.
    ///
    /// This is called during link configuration when a `.network` file
    /// references a `.netdev` via `VLAN=`, `MACVLAN=`, etc.
    pub fn create_dependent_netdev(
        &self,
        netdev_name: &str,
        parent_ifindex: u32,
    ) -> Result<bool, String> {
        let config = match self
            .netdev_configs
            .iter()
            .find(|c| c.netdev_section.name == netdev_name)
        {
            Some(c) => c,
            None => {
                return Err(format!("No .netdev config found for '{}'", netdev_name));
            }
        };

        netdev_create::create_netdev_with_parent(config, parent_ifindex)
    }

    /// Configure all managed links: bring up interfaces, apply static
    /// addresses, start DHCP clients.
    pub fn configure_links(&mut self) -> Result<(), String> {
        let indices: Vec<u32> = self.links.keys().copied().collect();

        for ifindex in indices {
            if let Err(e) = self.configure_link(ifindex) {
                log::warn!("Failed to configure link idx={}: {}", ifindex, e);
            }
        }

        self.initial_config_done = true;
        self.update_global_dns();

        Ok(())
    }

    /// Re-read managed links' carrier (running) state from the kernel and
    /// update `has_carrier`. The kernel sets IFF_RUNNING slightly after IFF_UP
    /// (asynchronously, via the linkwatch work queue), so a link that
    /// `configure_link` has just brought up only reports carrier a moment
    /// later. Refreshing here on each main-loop iteration keeps the exported
    /// per-link state files in sync with reality without waiting for the next
    /// full reload; `systemd-networkd-wait-online` reads those state files to
    /// decide when a link has carrier. Returns true if any link changed.
    pub fn refresh_carrier_states(&mut self) -> bool {
        let system_links = match link::list_links() {
            Ok(l) => l,
            Err(_) => return false,
        };
        let mut changed = false;
        for li in &system_links {
            if let Some(managed) = self.links.get_mut(&li.index) {
                let carrier = li.is_running() || li.is_loopback();
                if managed.has_carrier != carrier {
                    managed.has_carrier = carrier;
                    changed = true;
                }
            }
        }
        changed
    }

    /// Configure a single link.
    fn configure_link(&mut self, ifindex: u32) -> Result<(), String> {
        let managed = match self.links.get(&ifindex) {
            Some(m) => m,
            None => return Err(format!("unknown ifindex {ifindex}")),
        };

        if managed.admin_state != AdminState::Up {
            log::debug!(
                "Skipping {} (idx={}) — state={}",
                managed.link.name,
                ifindex,
                managed.admin_state
            );
            return Ok(());
        }

        let config = match &managed.config {
            Some(c) => c.clone(),
            None => return Ok(()),
        };

        let link_name = managed.link.name.clone();
        let mac = managed.link.mac_bytes.clone();

        // 1. Bring the interface up.
        log::info!("Bringing up {link_name} (idx={ifindex})");
        link::set_link_up(ifindex, true).map_err(|e| format!("set_link_up({link_name}): {e}"))?;

        // 2. Set MTU if specified.
        if let Some(mtu) = config.link.mtu {
            log::info!("Setting MTU on {link_name} to {mtu}");
            link::set_link_mtu(ifindex, mtu).map_err(|e| format!("set_mtu({link_name}): {e}"))?;
        }

        // 3. Apply static addresses.
        for addr_cfg in &config.addresses {
            if let Some((ip, prefix)) = config::parse_ipv4_cidr(&addr_cfg.address) {
                let broadcast = addr_cfg
                    .broadcast
                    .as_ref()
                    .and_then(|b| b.parse::<Ipv4Addr>().ok())
                    .unwrap_or_else(|| config::ipv4_broadcast(ip, prefix));

                log::info!("Adding address {ip}/{prefix} brd {broadcast} to {link_name}");
                if let Err(e) = link::add_address(ifindex, ip, prefix, Some(broadcast)) {
                    // EEXIST is fine — address already configured.
                    let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                    if !is_exists {
                        log::warn!("Failed to add address {ip}/{prefix} to {link_name}: {e}");
                    }
                }
            }
        }

        // 4. Apply static routes.
        for route_cfg in &config.routes {
            let gateway = route_cfg
                .gateway
                .as_ref()
                .and_then(|g| g.parse::<Ipv4Addr>().ok());

            let (dest, prefix_len) = match &route_cfg.destination {
                Some(d) => config::parse_ipv4_cidr(d).unwrap_or((Ipv4Addr::UNSPECIFIED, 0)),
                None => (Ipv4Addr::UNSPECIFIED, 0),
            };

            let metric = route_cfg.metric;

            log::info!(
                "Adding route {dest}/{prefix_len} via {:?} metric {:?} to {link_name}",
                gateway,
                metric,
            );

            if let Err(e) = link::add_route(
                dest,
                prefix_len,
                gateway,
                ifindex,
                metric,
                link::rtprot_static(),
            ) {
                let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                if !is_exists {
                    log::warn!("Failed to add route to {link_name}: {e}");
                }
            }
        }

        // 4.5. Apply routing policy rules.
        for rule_cfg in &config.routing_policy_rules {
            if let Some(rule) = build_rule_config(rule_cfg, &link_name) {
                log::info!(
                    "Adding routing policy rule on {link_name}: from={:?} to={:?} table={} prio={:?}",
                    rule.from,
                    rule.to,
                    rule.table,
                    rule.priority,
                );
                if let Err(e) = link::add_rule(&rule) {
                    let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                    if !is_exists {
                        log::warn!("Failed to add routing policy rule on {link_name}: {e}");
                    }
                }
            }
        }

        // 5. Collect static DNS/domains.
        let managed = self.links.get_mut(&ifindex).unwrap();
        for dns in &config.network_section.dns {
            if let std::net::IpAddr::V4(v4) = dns {
                managed.dns_servers.push(*v4);
            }
        }
        managed
            .search_domains
            .extend(config.network_section.domains.clone());
        managed.static_configured = true;

        // 6. Start DHCP client if needed.
        let needs_dhcp = matches!(config.network_section.dhcp, DhcpMode::Yes | DhcpMode::Ipv4);

        if needs_dhcp {
            let mut mac_arr = [0u8; 6];
            if mac.len() >= 6 {
                mac_arr.copy_from_slice(&mac[..6]);
            }

            let hostname = config.dhcpv4.hostname.clone().or_else(|| {
                if config.dhcpv4.send_hostname {
                    nix::unistd::gethostname()
                        .ok()
                        .and_then(|h| h.into_string().ok())
                } else {
                    None
                }
            });

            let dhcp_config = DhcpClientConfig {
                ifindex,
                ifname: link_name.clone(),
                mac: mac_arr,
                hostname,
                vendor_class_id: config.dhcpv4.vendor_class_id.clone(),
                client_identifier: if config.dhcpv4.client_identifier.as_deref() == Some("duid") {
                    dhcp::ClientIdMode::Duid
                } else {
                    dhcp::ClientIdMode::Mac
                },
                request_broadcast: config.dhcpv4.request_broadcast,
                route_metric: config.dhcpv4.route_metric.unwrap_or(1024),
                max_attempts: config.dhcpv4.max_attempts.unwrap_or(0),
                ..Default::default()
            };

            log::info!("Starting DHCP client on {link_name}");
            let client = DhcpClient::new(dhcp_config);
            let managed = self.links.get_mut(&ifindex).unwrap();
            managed.dhcp_client = Some(client);
        }

        // 7. Initialize IPv6 Router Advertisement handling if enabled.
        let accepts_ra = matches!(config.network_section.ipv6_accept_ra, Ipv6AcceptRa::Yes);
        // Also accept RA by default when DHCP is not set to managed-only mode
        // and link-local IPv6 is not explicitly disabled.
        let default_ra = config.network_section.ipv6_accept_ra == Ipv6AcceptRa::Yes
            || (matches!(
                config.network_section.link_local,
                config::LinkLocalMode::Yes | config::LinkLocalMode::Ipv6
            ) && !matches!(config.network_section.dhcp, DhcpMode::Yes));

        if accepts_ra || default_ra {
            let mut mac_arr = [0u8; 6];
            if mac.len() >= 6 {
                mac_arr.copy_from_slice(&mac[..6]);
            }

            // Determine address generation mode:
            // 1. Explicit IPv6LinkLocalAddressGenerationMode=stable-privacy
            // 2. IPv6StableSecretAddress= is set
            // 3. Default: EUI-64
            let use_stable_privacy = config
                .network_section
                .ipv6_ll_addr_gen_mode
                .as_deref()
                .is_some_and(|m| m.eq_ignore_ascii_case("stable-privacy"))
                || config.network_section.ipv6_stable_secret_address.is_some();

            let ra_state = if use_stable_privacy {
                // Resolve the secret key.
                let secret_key = match config.network_section.ipv6_stable_secret_address.as_deref()
                {
                    Some("machine-id") | None => {
                        // Derive from /etc/machine-id (default for stable-privacy mode).
                        read_machine_id_as_bytes().unwrap_or_else(|| {
                            log::warn!(
                                "{link_name}: failed to read /etc/machine-id for stable privacy addresses, falling back to EUI-64"
                            );
                            Vec::new()
                        })
                    }
                    Some(hex_secret) => {
                        // Hex-encoded secret key.
                        ipv6_ra::hex_to_bytes(hex_secret).unwrap_or_else(|| {
                            log::warn!(
                                "{link_name}: invalid IPv6StableSecretAddress hex value, falling back to EUI-64"
                            );
                            Vec::new()
                        })
                    }
                };

                if secret_key.is_empty() {
                    // Fallback to EUI-64 if we couldn't get a valid secret.
                    RaState::new(ifindex, link_name.clone(), mac_arr)
                } else {
                    log::info!("{link_name}: using RFC 7217 stable privacy SLAAC addresses");
                    RaState::new_stable_privacy(ifindex, link_name.clone(), mac_arr, secret_key)
                }
            } else {
                RaState::new(ifindex, link_name.clone(), mac_arr)
            };

            // Add link-local address first (needed for RA to work).
            if let Some(ll_addr) = ra_state.link_local {
                log::info!("Adding IPv6 link-local address {} to {link_name}", ll_addr);
                if let Err(e) = ipv6_ra::add_ipv6_address(ifindex, ll_addr, 64) {
                    let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                    if !is_exists {
                        log::warn!(
                            "Failed to add link-local address {} to {link_name}: {e}",
                            ll_addr
                        );
                    }
                }
            }

            log::info!("Enabling IPv6 Router Advertisement on {link_name}");
            let managed = self.links.get_mut(&ifindex).unwrap();
            managed.ra_state = Some(ra_state);
        }

        // 8. Start DHCPv6 client if DHCP=yes or DHCP=ipv6.
        let needs_dhcpv6 = matches!(config.network_section.dhcp, DhcpMode::Yes | DhcpMode::Ipv6);

        if needs_dhcpv6 {
            let mut mac_arr = [0u8; 6];
            if mac.len() >= 6 {
                mac_arr.copy_from_slice(&mac[..6]);
            }

            // Parse prefix delegation hint from [DHCPv6] PrefixDelegationHint=.
            // Format: "address/prefix_len" e.g. "::1/56" or "::/48".
            let (pd_enabled, pd_hint_len, pd_hint_addr) =
                parse_prefix_delegation_hint(&config.dhcpv6.prefix_delegation_hint);

            let dhcpv6_config = Dhcpv6ClientConfig {
                ifindex,
                ifname: link_name.clone(),
                mac: mac_arr,
                request_addresses: config.dhcpv6.use_address,
                request_prefix_delegation: pd_enabled,
                prefix_hint: pd_hint_len,
                prefix_hint_address: pd_hint_addr,
                rapid_commit: config.dhcpv6.rapid_commit,
                max_attempts: 0,
                ..Default::default()
            };

            if pd_enabled {
                log::info!(
                    "Starting DHCPv6 client on {link_name} (PD hint=/{pd_hint_len}, rapid_commit={})",
                    config.dhcpv6.rapid_commit
                );
            } else {
                log::info!("Starting DHCPv6 client on {link_name}");
            }
            let client = Dhcpv6Client::new(dhcpv6_config);
            let managed = self.links.get_mut(&ifindex).unwrap();
            managed.dhcpv6_client = Some(client);
        }

        Ok(())
    }

    /// Apply a DHCP lease to an interface: set address, routes, DNS.
    pub fn apply_lease(&mut self, ifindex: u32, lease: &DhcpLease) -> Result<(), String> {
        let managed = match self.links.get(&ifindex) {
            Some(m) => m,
            None => return Err(format!("unknown ifindex {ifindex}")),
        };

        let link_name = managed.link.name.clone();
        let use_dns = managed
            .config
            .as_ref()
            .map(|c| c.dhcpv4.use_dns)
            .unwrap_or(true);
        let use_routes = managed
            .config
            .as_ref()
            .map(|c| c.dhcpv4.use_routes)
            .unwrap_or(true);
        let use_hostname = managed
            .config
            .as_ref()
            .map(|c| c.dhcpv4.use_hostname)
            .unwrap_or(true);
        let use_mtu = managed
            .config
            .as_ref()
            .map(|c| c.dhcpv4.use_mtu)
            .unwrap_or(true);
        let route_metric = managed
            .config
            .as_ref()
            .and_then(|c| c.dhcpv4.route_metric)
            .unwrap_or(1024);

        // 1. Flush existing DHCP-assigned addresses/routes.
        // (In a real implementation we'd only flush what we previously set.)
        let _ = link::flush_addresses(ifindex);
        let _ = link::flush_routes(ifindex);

        // 2. Add the leased address.
        let broadcast = lease
            .broadcast
            .unwrap_or_else(|| config::ipv4_broadcast(lease.address, lease.prefix_len()));

        log::info!(
            "{link_name}: DHCP address {}/{} brd {broadcast}",
            lease.address,
            lease.prefix_len()
        );

        link::add_address(ifindex, lease.address, lease.prefix_len(), Some(broadcast))
            .map_err(|e| format!("add_address({link_name}): {e}"))?;

        // 3. Add routes.
        if use_routes {
            // Classless routes take priority (RFC 3442).
            if !lease.classless_routes.is_empty() {
                for &(dest, prefix_len, gw) in &lease.classless_routes {
                    log::info!(
                        "{link_name}: DHCP route {dest}/{prefix_len} via {gw} metric {route_metric}"
                    );
                    let _ = link::add_route(
                        dest,
                        prefix_len,
                        Some(gw),
                        ifindex,
                        Some(route_metric),
                        link::rtprot_dhcp(),
                    );
                }
            } else {
                // Default gateway from routers option.
                for gw in &lease.routers {
                    log::info!("{link_name}: DHCP default route via {gw} metric {route_metric}");
                    let _ = link::add_default_route(
                        *gw,
                        ifindex,
                        Some(route_metric),
                        link::rtprot_dhcp(),
                    );
                }
            }

            // On-link route for the subnet.
            let network = config::ipv4_network(lease.address, lease.prefix_len());
            let _ = link::add_route(
                network,
                lease.prefix_len(),
                None,
                ifindex,
                Some(route_metric),
                link::rtprot_dhcp(),
            );
        }

        // 4. Update MTU if offered and enabled.
        if use_mtu
            && let Some(mtu) = lease.mtu
            && mtu >= 576
        {
            log::info!("{link_name}: DHCP MTU {mtu}");
            let _ = link::set_link_mtu(ifindex, u32::from(mtu));
        }

        // 5. Update hostname if offered and enabled.
        if use_hostname && let Some(ref hostname) = lease.hostname {
            log::info!("{link_name}: DHCP hostname '{hostname}'");
            let _ = nix::unistd::sethostname(hostname);
        }

        // 6. Update DNS / domains on the managed link.
        let managed = self.links.get_mut(&ifindex).unwrap();
        managed.lease = Some(lease.clone());

        if use_dns {
            managed.dns_servers = lease.dns_servers.clone();
        }
        if let Some(ref domain) = lease.domain_name {
            managed.search_domains = vec![domain.clone()];
        }

        self.update_global_dns();

        log::info!(
            "{link_name}: DHCP lease applied — {} (lease {}s, renew {}s)",
            lease.address,
            lease.lease_time,
            lease.renewal_time,
        );

        Ok(())
    }

    /// Remove DHCP-learned configuration from a link (on lease expiry or release).
    pub fn remove_lease(&mut self, ifindex: u32) -> Result<(), String> {
        let managed = match self.links.get_mut(&ifindex) {
            Some(m) => m,
            None => return Err(format!("unknown ifindex {ifindex}")),
        };

        let link_name = managed.link.name.clone();
        log::info!("{link_name}: removing DHCP lease configuration");

        // Flush DHCP-assigned addresses and routes.
        let _ = link::flush_addresses(ifindex);
        let _ = link::flush_routes(ifindex);

        managed.lease = None;
        managed.dns_servers.clear();
        managed.search_domains.clear();

        // Re-apply static config if any.
        if let Some(ref cfg) = managed.config {
            for dns in &cfg.network_section.dns {
                if let std::net::IpAddr::V4(v4) = dns {
                    managed.dns_servers.push(*v4);
                }
            }
            managed
                .search_domains
                .extend(cfg.network_section.domains.clone());
        }

        self.update_global_dns();

        Ok(())
    }

    /// Aggregate DNS servers and search domains from all managed links and
    /// write `/run/systemd/resolve/resolv.conf`.
    fn update_global_dns(&mut self) {
        let mut dns = Vec::new();
        let mut domains = Vec::new();
        let mut dns6 = Vec::new();

        for managed in self.links.values() {
            for server in &managed.dns_servers {
                if !dns.contains(server) {
                    dns.push(*server);
                }
            }
            for domain in &managed.search_domains {
                if !domains.contains(domain) {
                    domains.push(domain.clone());
                }
            }
            // Collect IPv6 DNS from RA RDNSS
            for server in &managed.dns6_servers {
                if !dns6.contains(server) {
                    dns6.push(*server);
                }
            }
            // Collect IPv6 search domains from RA DNSSL
            for domain in &managed.search6_domains {
                if !domains.contains(domain) {
                    domains.push(domain.clone());
                }
            }
        }

        self.dns_servers = dns.clone();
        self.search_domains = domains.clone();
        self.dns6_servers = dns6.clone();

        if let Err(e) = link::write_resolv_conf(&dns, &dns6, &domains) {
            log::warn!("Failed to write resolv.conf: {e}");
        }
    }

    /// Get interface indices that have IPv6 RA enabled and need RS sending.
    pub fn ra_active_links(&self) -> Vec<u32> {
        self.links
            .iter()
            .filter_map(|(&idx, m)| {
                if m.ra_state.as_ref().is_some_and(|s| s.enabled) {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check all RA-enabled links for expired or deprecated SLAAC addresses.
    ///
    /// Returns `true` if any addresses were removed or deprecated.
    pub fn check_ra_address_lifetimes(&mut self) -> bool {
        let active: Vec<u32> = self.ra_active_links();
        let mut changed = false;

        for ifindex in active {
            let actions = {
                let managed = match self.links.get_mut(&ifindex) {
                    Some(m) => m,
                    None => continue,
                };
                match managed.ra_state.as_mut() {
                    Some(s) => s.check_address_lifetimes(),
                    None => continue,
                }
            };

            if !actions.is_empty() {
                changed |= self.apply_ra_actions(ifindex, actions);
            }
        }

        changed
    }

    /// Process a received Router Advertisement on a given interface.
    ///
    /// Applies the RA actions (add SLAAC addresses, default route, DNS, etc.)
    /// and returns whether any state changed.
    pub fn apply_ra_actions(&mut self, ifindex: u32, actions: Vec<RaAction>) -> bool {
        let managed = match self.links.get_mut(&ifindex) {
            Some(m) => m,
            None => return false,
        };
        let link_name = managed.link.name.clone();
        let mut changed = false;

        for action in actions {
            match action {
                RaAction::AddAddress {
                    address,
                    prefix_len,
                    ..
                } => {
                    log::info!("{link_name}: adding SLAAC address {address}/{prefix_len}");
                    if let Err(e) = ipv6_ra::add_ipv6_address(ifindex, address, prefix_len) {
                        let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                        if !is_exists {
                            log::warn!(
                                "{link_name}: failed to add SLAAC address {address}/{prefix_len}: {e}"
                            );
                        }
                    }
                    changed = true;
                }
                RaAction::RemoveAddress {
                    address,
                    prefix_len,
                } => {
                    log::info!(
                        "{link_name}: removing expired SLAAC address {address}/{prefix_len}"
                    );
                    // Best-effort removal via netlink RTM_DELADDR.
                    // If the address was already removed (e.g. by the kernel), this is a no-op.
                    if let Err(e) = ipv6_ra::remove_ipv6_address(ifindex, address, prefix_len) {
                        let is_noent = e.raw_os_error() == Some(libc::EADDRNOTAVAIL);
                        if !is_noent {
                            log::warn!(
                                "{link_name}: failed to remove SLAAC address {address}/{prefix_len}: {e}"
                            );
                        }
                    }
                    changed = true;
                }
                RaAction::DeprecateAddress {
                    address,
                    prefix_len,
                } => {
                    log::info!(
                        "{link_name}: SLAAC address {address}/{prefix_len} deprecated (preferred lifetime expired)"
                    );
                    // Deprecation is informational — the address remains usable for
                    // existing connections but should not be used for new ones.
                    // The kernel handles this via IFA_F_DEPRECATED flag, but setting
                    // it requires RTM_NEWADDR with the flag; for now we just log it.
                    changed = true;
                }
                RaAction::AddDefaultRoute { gateway, .. } => {
                    log::info!("{link_name}: adding IPv6 default route via {gateway}");
                    if let Err(e) = ipv6_ra::add_ipv6_default_route(gateway, ifindex, Some(1024)) {
                        let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                        if !is_exists {
                            log::warn!(
                                "{link_name}: failed to add IPv6 default route via {gateway}: {e}"
                            );
                        }
                    }
                    changed = true;
                }
                RaAction::RemoveDefaultRoute { gateway } => {
                    log::info!("{link_name}: removing IPv6 default route via {gateway}");
                    // Best-effort removal — not fatal if it fails.
                    changed = true;
                }
                RaAction::AddOnLinkRoute {
                    prefix, prefix_len, ..
                } => {
                    log::info!("{link_name}: adding on-link route {prefix}/{prefix_len}");
                    if let Err(e) = ipv6_ra::add_ipv6_route(
                        prefix,
                        prefix_len,
                        None,
                        ifindex,
                        None,
                        ipv6_ra::rtprot_ra(),
                    ) {
                        let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                        if !is_exists {
                            log::warn!(
                                "{link_name}: failed to add on-link route {prefix}/{prefix_len}: {e}"
                            );
                        }
                    }
                    changed = true;
                }
                RaAction::AddRoute {
                    prefix,
                    prefix_len,
                    gateway,
                    ..
                } => {
                    log::info!("{link_name}: adding route {prefix}/{prefix_len} via {gateway}");
                    if let Err(e) = ipv6_ra::add_ipv6_route(
                        prefix,
                        prefix_len,
                        Some(gateway),
                        ifindex,
                        None,
                        ipv6_ra::rtprot_ra(),
                    ) {
                        let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                        if !is_exists {
                            log::warn!(
                                "{link_name}: failed to add route {prefix}/{prefix_len} via {gateway}: {e}"
                            );
                        }
                    }
                    changed = true;
                }
                RaAction::UpdateDns { servers } => {
                    log::info!(
                        "{link_name}: updating IPv6 DNS servers from RA: {:?}",
                        servers
                    );
                    let managed = self.links.get_mut(&ifindex).unwrap();
                    managed.dns6_servers = servers;
                    changed = true;
                }
                RaAction::UpdateSearchDomains { domains } => {
                    log::info!(
                        "{link_name}: updating search domains from RA: {:?}",
                        domains
                    );
                    let managed = self.links.get_mut(&ifindex).unwrap();
                    managed.search6_domains = domains;
                    changed = true;
                }
                RaAction::SetMtu { mtu } => {
                    log::info!("{link_name}: setting MTU from RA to {mtu}");
                    if let Err(e) = link::set_link_mtu(ifindex, mtu) {
                        log::warn!("{link_name}: failed to set MTU to {mtu}: {e}");
                    }
                    changed = true;
                }
            }
        }

        if changed {
            self.update_global_dns();
        }
        changed
    }

    /// Get a summary of the operational state of all managed links.
    pub fn status_summary(&self) -> Vec<LinkStatus> {
        let mut result = Vec::new();
        for managed in self.links.values() {
            result.push(LinkStatus {
                index: managed.link.index,
                name: managed.link.name.clone(),
                link_type: if managed.link.is_loopback() {
                    "loopback"
                } else {
                    "ether"
                }
                .to_string(),
                oper_state: managed.oper_state(),
                admin_state: managed.admin_state,
                config_file: managed
                    .config
                    .as_ref()
                    .map(|c| c.path.display().to_string()),
                address: managed
                    .lease
                    .as_ref()
                    .map(|l| format!("{}/{}", l.address, l.prefix_len())),
                gateway: managed
                    .lease
                    .as_ref()
                    .and_then(|l| l.routers.first().map(|r| r.to_string())),
                dns: managed.dns_servers.iter().map(|d| d.to_string()).collect(),
                dhcp_state: managed.dhcp_client.as_ref().map(|c| c.state.to_string()),
            });
        }
        result.sort_by_key(|s| s.index);
        result
    }

    /// Returns the overall operational state of the system.
    /// Used for sd_notify STATUS.
    pub fn overall_state(&self) -> &'static str {
        if !self.initial_config_done {
            return "initializing";
        }

        let mut any_configured = false;
        let mut any_pending = false;
        let mut any_degraded = false;

        for managed in self.links.values() {
            match managed.oper_state() {
                OperState::Configured => any_configured = true,
                OperState::Pending | OperState::Configuring => any_pending = true,
                OperState::Degraded => any_degraded = true,
                _ => {}
            }
        }

        if any_pending {
            "configuring"
        } else if any_degraded {
            "degraded"
        } else if any_configured {
            "configured"
        } else {
            "no-carrier"
        }
    }

    /// List drop-ins for a config file as `<name>.d/*.conf` across the network
    /// config directories, deduped by filename (highest-precedence dir wins)
    /// and sorted by filename. Mirrors networkctl's `list_dropins` so the
    /// `*_FILE_DROPINS` fields written to the link state file match exactly what
    /// `networkctl cat <name>` produces from the filesystem.
    fn config_dropins(config_file: &std::path::Path) -> Vec<std::path::PathBuf> {
        const NETWORK_DIRS: [&str; 3] = [
            "/etc/systemd/network",
            "/run/systemd/network",
            "/usr/lib/systemd/network",
        ];
        let Some(name) = config_file.file_name() else {
            return Vec::new();
        };
        let name = name.to_string_lossy();
        let mut m: std::collections::BTreeMap<String, std::path::PathBuf> =
            std::collections::BTreeMap::new();
        for dir in NETWORK_DIRS {
            let dropin_dir = std::path::Path::new(dir).join(format!("{name}.d"));
            if let Ok(rd) = std::fs::read_dir(&dropin_dir) {
                for e in rd.flatten() {
                    let fname = e.file_name().to_string_lossy().into_owned();
                    if fname.ends_with(".conf") {
                        m.entry(fname).or_insert_with(|| e.path());
                    }
                }
            }
        }
        m.into_values().collect()
    }

    /// Format a drop-in path list as the colon-separated value used by the
    /// `*_FILE_DROPINS` state-file fields.
    fn dropins_field(dropins: &[std::path::PathBuf]) -> String {
        dropins
            .iter()
            .map(|p| p.display().to_string())
            .collect::<Vec<_>>()
            .join(":")
    }

    /// Read the `.link` file (and its drop-ins) that systemd-udevd applied to a
    /// network interface, from its udev database entry
    /// `/run/udev/data/n<ifindex>` (properties `E:ID_NET_LINK_FILE` and
    /// `E:ID_NET_LINK_FILE_DROPINS`, set by udev's `net_setup_link` builtin).
    fn udev_link_file(ifindex: u32) -> Option<(String, Vec<std::path::PathBuf>)> {
        let content = std::fs::read_to_string(format!("/run/udev/data/n{ifindex}")).ok()?;
        let mut file: Option<String> = None;
        let mut dropins: Vec<std::path::PathBuf> = Vec::new();
        for line in content.lines() {
            if let Some(v) = line.strip_prefix("E:ID_NET_LINK_FILE=") {
                file = Some(v.trim().to_string());
            } else if let Some(v) = line.strip_prefix("E:ID_NET_LINK_FILE_DROPINS=") {
                dropins = v
                    .trim()
                    .split(':')
                    .filter(|x| !x.is_empty())
                    .map(std::path::PathBuf::from)
                    .collect();
            }
        }
        file.map(|f| (f, dropins))
    }

    /// Write runtime state files to `/run/systemd/netif/`.
    pub fn write_state_files(&self) {
        let state_dir = std::path::Path::new("/run/systemd/netif/links");
        if let Err(e) = std::fs::create_dir_all(state_dir) {
            log::debug!("Cannot create {}: {}", state_dir.display(), e);
            return;
        }

        let lease_dir = std::path::Path::new("/run/systemd/netif/leases");
        let _ = std::fs::create_dir_all(lease_dir);

        for managed in self.links.values() {
            // Write link state file.
            let link_file = state_dir.join(managed.link.index.to_string());
            let mut content = String::new();
            content.push_str("# systemd-networkd state file\n");
            content.push_str(&format!("ADMIN_STATE={}\n", managed.admin_state));
            content.push_str(&format!("OPER_STATE={}\n", managed.operational_state_str()));
            if let Some(ref cfg) = managed.config {
                content.push_str(&format!("NETWORK_FILE={}\n", cfg.path.display()));
                let dropins = Self::config_dropins(&cfg.path);
                if !dropins.is_empty() {
                    content.push_str(&format!(
                        "NETWORK_FILE_DROPINS={}\n",
                        Self::dropins_field(&dropins)
                    ));
                }
            }
            // Record the .netdev file that created this interface (matched by
            // the configured device name), so `networkctl cat @IF:netdev` can
            // resolve it from the state file.
            if let Some(ndev) = self
                .netdev_configs
                .iter()
                .find(|c| c.netdev_section.name == managed.link.name)
            {
                content.push_str(&format!("NETDEV_FILE={}\n", ndev.path.display()));
                let dropins = Self::config_dropins(&ndev.path);
                if !dropins.is_empty() {
                    content.push_str(&format!(
                        "NETDEV_FILE_DROPINS={}\n",
                        Self::dropins_field(&dropins)
                    ));
                }
            }
            // Record the .link file that systemd-udevd applied to this
            // interface, read from the network device's udev database entry
            // (property ID_NET_LINK_FILE), so `networkctl cat @IF:link` can
            // resolve it.
            if let Some((link_file, dropins)) = Self::udev_link_file(managed.link.index) {
                content.push_str(&format!("LINK_FILE={link_file}\n"));
                if !dropins.is_empty() {
                    content.push_str(&format!(
                        "LINK_FILE_DROPINS={}\n",
                        Self::dropins_field(&dropins)
                    ));
                }
            }
            for dns in &managed.dns_servers {
                content.push_str(&format!("DNS={dns}\n"));
            }
            for domain in &managed.search_domains {
                content.push_str(&format!("DOMAINS={domain}\n"));
            }
            // Include IPv6 DNS from DHCPv6/RA.
            for dns6 in &managed.dns6_servers {
                content.push_str(&format!("DNS={dns6}\n"));
            }
            for domain6 in &managed.search6_domains {
                content.push_str(&format!("DOMAINS={domain6}\n"));
            }
            // Include delegated prefixes from DHCPv6-PD.
            if let Some(ref dhcpv6_lease) = managed.dhcpv6_lease
                && let Some(ref ia_pd) = dhcpv6_lease.ia_pd
            {
                for prefix in &ia_pd.prefixes {
                    content.push_str(&format!(
                        "DELEGATED_PREFIX={}/{}\n",
                        prefix.prefix, prefix.prefix_length
                    ));
                }
            }
            let _ = std::fs::write(&link_file, &content);

            // Write lease state file if we have a lease.
            if let Some(ref lease) = managed.lease {
                let lease_file = lease_dir.join(managed.link.index.to_string());
                let mut lc = String::new();
                lc.push_str(&format!("ADDRESS={}\n", lease.address));
                lc.push_str(&format!("NETMASK={}\n", lease.subnet_mask));
                lc.push_str(&format!("SERVER_ADDRESS={}\n", lease.server_id));
                lc.push_str(&format!("LIFETIME={}\n", lease.lease_time));
                lc.push_str(&format!("T1={}\n", lease.renewal_time));
                lc.push_str(&format!("T2={}\n", lease.rebinding_time));
                for gw in &lease.routers {
                    lc.push_str(&format!("ROUTER={gw}\n"));
                }
                for dns in &lease.dns_servers {
                    lc.push_str(&format!("DNS={dns}\n"));
                }
                if let Some(ref hostname) = lease.hostname {
                    lc.push_str(&format!("HOSTNAME={hostname}\n"));
                }
                if let Some(ref domain) = lease.domain_name {
                    lc.push_str(&format!("DOMAINNAME={domain}\n"));
                }
                let _ = std::fs::write(&lease_file, &lc);
            }
        }

        // Write overall state.
        let state_file = std::path::Path::new("/run/systemd/netif/state");
        let mut content = String::new();
        content.push_str("# systemd-networkd overall state\n");
        content.push_str(&format!("OPER_STATE={}\n", self.overall_state()));
        for dns in &self.dns_servers {
            content.push_str(&format!("DNS={dns}\n"));
        }
        for domain in &self.search_domains {
            content.push_str(&format!("DOMAINS={domain}\n"));
        }
        let _ = std::fs::write(state_file, &content);
    }

    /// Return the list of interface indices that have active DHCP clients
    /// needing to send/receive packets.
    pub fn dhcp_active_links(&self) -> Vec<u32> {
        self.links
            .iter()
            .filter_map(|(&idx, managed)| {
                if managed.dhcp_client.is_some() {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if all links that need DHCP have obtained leases.
    pub fn all_dhcp_bound(&self) -> bool {
        self.links.values().all(|managed| {
            match &managed.dhcp_client {
                Some(client) => client.state == DhcpState::Bound,
                None => true, // No DHCP needed.
            }
        })
    }

    /// Release all DHCP leases (called on shutdown).
    pub fn release_all(&mut self) {
        let indices: Vec<u32> = self.dhcp_active_links();
        for ifindex in indices {
            if let Some(managed) = self.links.get(&ifindex)
                && let Some(ref client) = managed.dhcp_client
                && let Some(release_pkt) = client.build_release()
            {
                log::info!(
                    "{}: sending DHCPRELEASE for {}",
                    managed.link.name,
                    client
                        .lease
                        .as_ref()
                        .map(|l| l.address.to_string())
                        .unwrap_or_default(),
                );
                // In a real implementation, we'd send this packet.
                // For now we just log it.
                let _ = release_pkt;
            }
            let _ = self.remove_lease(ifindex);
        }
    }

    // -----------------------------------------------------------------------
    // DHCPv6
    // -----------------------------------------------------------------------

    /// Return the list of interface indices that have active DHCPv6 clients
    /// needing to send/receive packets.
    pub fn dhcpv6_active_links(&self) -> Vec<u32> {
        self.links
            .iter()
            .filter_map(|(&idx, managed)| {
                if managed.dhcpv6_client.is_some() {
                    Some(idx)
                } else {
                    None
                }
            })
            .collect()
    }

    /// Apply a DHCPv6 lease to an interface: add addresses, delegated prefix routes, update DNS.
    pub fn apply_dhcpv6_lease(&mut self, ifindex: u32, lease: &Dhcpv6Lease) -> Result<(), String> {
        let managed = match self.links.get(&ifindex) {
            Some(m) => m,
            None => return Err(format!("unknown ifindex {ifindex}")),
        };
        let link_name = managed.link.name.clone();
        let use_delegated_prefix = managed
            .config
            .as_ref()
            .map(|c| c.dhcpv6.use_delegated_prefix)
            .unwrap_or(true);

        // Add assigned IPv6 addresses.
        for addr in &lease.ia_na.addresses {
            log::info!(
                "{}: adding DHCPv6 address {}/128 (preferred={}s valid={}s)",
                link_name,
                addr.address,
                addr.preferred_lifetime,
                addr.valid_lifetime
            );
            if let Err(e) = ipv6_ra::add_ipv6_address(ifindex, addr.address, 128) {
                let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                if !is_exists {
                    log::warn!(
                        "{}: failed to add DHCPv6 address {}: {}",
                        link_name,
                        addr.address,
                        e
                    );
                }
            }
        }

        // Apply delegated prefixes (IA_PD) — add on-link routes.
        if use_delegated_prefix && let Some(ref ia_pd) = lease.ia_pd {
            for prefix in &ia_pd.prefixes {
                log::info!(
                    "{}: adding delegated prefix route {}/{} (preferred={}s valid={}s)",
                    link_name,
                    prefix.prefix,
                    prefix.prefix_length,
                    prefix.preferred_lifetime,
                    prefix.valid_lifetime
                );
                if let Err(e) =
                    ipv6_ra::add_ipv6_pd_route(prefix.prefix, prefix.prefix_length, ifindex, None)
                {
                    let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                    if !is_exists {
                        log::warn!(
                            "{}: failed to add delegated prefix route {}/{}: {}",
                            link_name,
                            prefix.prefix,
                            prefix.prefix_length,
                            e
                        );
                    }
                }
            }
        }

        // Update DNS servers.
        let managed = self.links.get_mut(&ifindex).unwrap();
        if !lease.dns_servers.is_empty() {
            // Merge DHCPv6 DNS servers with existing IPv6 DNS (from RA).
            for dns in &lease.dns_servers {
                if !managed.dns6_servers.contains(dns) {
                    managed.dns6_servers.push(*dns);
                }
            }
            log::info!("{}: DHCPv6 DNS servers: {:?}", link_name, lease.dns_servers);
        }

        // Update search domains.
        if !lease.domains.is_empty() {
            for domain in &lease.domains {
                if !managed.search6_domains.contains(domain) {
                    managed.search6_domains.push(domain.clone());
                }
            }
            log::info!("{}: DHCPv6 search domains: {:?}", link_name, lease.domains);
        }

        managed.dhcpv6_lease = Some(lease.clone());

        // Update global DNS.
        self.update_global_dns();

        // Write resolv.conf.
        let all_dns4: Vec<Ipv4Addr> = self
            .links
            .values()
            .flat_map(|m| m.dns_servers.iter().copied())
            .collect();
        let all_dns6: Vec<Ipv6Addr> = self
            .links
            .values()
            .flat_map(|m| m.dns6_servers.iter().copied())
            .collect();
        let all_domains: Vec<String> = self
            .links
            .values()
            .flat_map(|m| {
                m.search_domains
                    .iter()
                    .chain(m.search6_domains.iter())
                    .cloned()
            })
            .collect();

        if (!all_dns4.is_empty() || !all_dns6.is_empty())
            && let Err(e) = link::write_resolv_conf(&all_dns4, &all_dns6, &all_domains)
        {
            log::warn!("Failed to write resolv.conf: {e}");
        }

        // Distribute /64 subnets to downstream interfaces.
        if use_delegated_prefix && lease.has_prefix_delegation() {
            self.distribute_delegated_prefixes(ifindex);
        }

        Ok(())
    }

    /// Remove a DHCPv6 lease from an interface.
    pub fn remove_dhcpv6_lease(&mut self, ifindex: u32) -> Result<(), String> {
        // First, remove any downstream PD-assigned addresses before we drop
        // the lease (we need the delegated prefixes to know what to clean up).
        self.remove_distributed_prefixes(ifindex);

        let managed = match self.links.get_mut(&ifindex) {
            Some(m) => m,
            None => return Err(format!("unknown ifindex {ifindex}")),
        };

        let link_name = managed.link.name.clone();

        // Remove assigned addresses and delegated prefix routes.
        if let Some(ref lease) = managed.dhcpv6_lease {
            for addr in &lease.ia_na.addresses {
                log::info!("{}: removing DHCPv6 address {}", link_name, addr.address);
                if let Err(e) = ipv6_ra::remove_ipv6_address(ifindex, addr.address, 128) {
                    log::debug!(
                        "{}: failed to remove DHCPv6 address {}: {} (best-effort)",
                        link_name,
                        addr.address,
                        e
                    );
                }
            }

            // Remove delegated prefix routes.
            if let Some(ref ia_pd) = lease.ia_pd {
                for prefix in &ia_pd.prefixes {
                    log::info!(
                        "{}: removing delegated prefix route {}/{}",
                        link_name,
                        prefix.prefix,
                        prefix.prefix_length
                    );
                    if let Err(e) = ipv6_ra::remove_ipv6_route(
                        prefix.prefix,
                        prefix.prefix_length,
                        None,
                        ifindex,
                    ) {
                        log::debug!(
                            "{}: failed to remove delegated prefix route {}/{}: {} (best-effort)",
                            link_name,
                            prefix.prefix,
                            prefix.prefix_length,
                            e
                        );
                    }
                }
            }

            // Clear DHCPv6-contributed DNS.
            managed
                .dns6_servers
                .retain(|dns| !lease.dns_servers.contains(dns));
            managed
                .search6_domains
                .retain(|d| !lease.domains.contains(d));
        }

        managed.dhcpv6_lease = None;

        self.update_global_dns();
        Ok(())
    }

    /// Distribute /64 subnets from an upstream interface's delegated prefixes
    /// to downstream interfaces that have `[DHCPv6PrefixDelegation] Assign=yes`.
    ///
    /// For each delegated prefix (e.g. 2001:db8:abcd::/48) and each downstream
    /// interface, a /64 subnet is carved out and an address is assigned.  The
    /// subnet selection uses the `SubnetId=` config (or auto-assigns
    /// sequentially) and the host part uses `Token=` (default `::1`).
    pub fn distribute_delegated_prefixes(&mut self, upstream_ifindex: u32) {
        // Collect delegated prefixes from the upstream lease.
        let prefixes: Vec<(Ipv6Addr, u8)> = match self.links.get(&upstream_ifindex) {
            Some(m) => match m.dhcpv6_lease {
                Some(ref lease) => lease
                    .delegated_prefixes()
                    .iter()
                    .map(|p| (p.prefix, p.prefix_length))
                    .collect(),
                None => return,
            },
            None => return,
        };

        if prefixes.is_empty() {
            return;
        }

        let upstream_name = self
            .links
            .get(&upstream_ifindex)
            .map(|m| m.link.name.clone())
            .unwrap_or_default();

        // Collect downstream interfaces that want PD assignment.
        // Each entry: (ifindex, link_name, subnet_id_option, token).
        let downstream: Vec<(u32, String, Option<u64>, Ipv6Addr)> = self
            .links
            .iter()
            .filter(|&(idx, _)| *idx != upstream_ifindex)
            .filter_map(|(&idx, m)| {
                let cfg = m.config.as_ref()?;
                if cfg.dhcpv6_pd.assign {
                    Some((
                        idx,
                        m.link.name.clone(),
                        cfg.dhcpv6_pd.subnet_id,
                        cfg.dhcpv6_pd.token,
                    ))
                } else {
                    None
                }
            })
            .collect();

        if downstream.is_empty() {
            return;
        }

        log::info!(
            "{}: distributing {} delegated prefix(es) to {} downstream interface(s)",
            upstream_name,
            prefixes.len(),
            downstream.len()
        );

        // For each delegated prefix, assign /64 subnets to downstream interfaces.
        for (prefix, prefix_len) in &prefixes {
            if *prefix_len >= 64 {
                log::warn!(
                    "{}: delegated prefix {}/{} is too long to carve /64 subnets",
                    upstream_name,
                    prefix,
                    prefix_len
                );
                continue;
            }

            let mut auto_subnet_counter: u64 = 0;

            for (ds_ifindex, ds_name, subnet_id_opt, token) in &downstream {
                let subnet_id = match subnet_id_opt {
                    Some(id) => *id,
                    None => {
                        let id = auto_subnet_counter;
                        auto_subnet_counter += 1;
                        id
                    }
                };

                // Compute the /64 subnet: place subnet_id in the bits between
                // prefix_len and 64.
                let max_subnet_bits = 64 - *prefix_len as u32;
                let max_subnets = 1u64 << max_subnet_bits;
                if subnet_id >= max_subnets {
                    log::warn!(
                        "{}: subnet_id {} exceeds maximum {} for prefix {}/{}",
                        ds_name,
                        subnet_id,
                        max_subnets - 1,
                        prefix,
                        prefix_len
                    );
                    continue;
                }

                let subnet_addr =
                    compute_pd_subnet_address(*prefix, *prefix_len, subnet_id, *token);

                log::info!(
                    "{}: assigning PD address {}/64 from {}/{} (subnet_id={})",
                    ds_name,
                    subnet_addr,
                    prefix,
                    prefix_len,
                    subnet_id
                );

                // Add the /64 address on the downstream interface.
                if let Err(e) = ipv6_ra::add_ipv6_address(*ds_ifindex, subnet_addr, 64) {
                    let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                    if !is_exists {
                        log::warn!(
                            "{}: failed to add PD address {}/64: {}",
                            ds_name,
                            subnet_addr,
                            e
                        );
                    }
                }

                // Add on-link route for the /64 subnet on the downstream interface.
                let subnet_prefix = compute_pd_subnet(*prefix, *prefix_len, subnet_id);
                if let Err(e) = ipv6_ra::add_ipv6_pd_route(subnet_prefix, 64, *ds_ifindex, None) {
                    let is_exists = e.raw_os_error() == Some(libc::EEXIST);
                    if !is_exists {
                        log::warn!(
                            "{}: failed to add PD route {}/64: {}",
                            ds_name,
                            subnet_prefix,
                            e
                        );
                    }
                }

                // Track the assigned address for later cleanup.
                if let Some(ds_managed) = self.links.get_mut(ds_ifindex) {
                    ds_managed.pd_assigned_addresses.push((subnet_addr, 64));
                }
            }
        }
    }

    /// Remove all PD-distributed addresses from downstream interfaces that
    /// were assigned from the given upstream interface's delegated prefixes.
    pub fn remove_distributed_prefixes(&mut self, _upstream_ifindex: u32) {
        // Collect all downstream ifindexes that have PD-assigned addresses.
        let downstream_indices: Vec<u32> = self
            .links
            .iter()
            .filter(|(_, m)| !m.pd_assigned_addresses.is_empty())
            .map(|(&idx, _)| idx)
            .collect();

        for ds_ifindex in downstream_indices {
            let managed = match self.links.get_mut(&ds_ifindex) {
                Some(m) => m,
                None => continue,
            };

            let link_name = managed.link.name.clone();
            let addrs: Vec<(Ipv6Addr, u8)> = managed.pd_assigned_addresses.drain(..).collect();

            for (addr, prefix_len) in &addrs {
                log::info!(
                    "{}: removing PD-assigned address {}/{}",
                    link_name,
                    addr,
                    prefix_len
                );
                if let Err(e) = ipv6_ra::remove_ipv6_address(ds_ifindex, *addr, *prefix_len) {
                    log::debug!(
                        "{}: failed to remove PD address {}/{}: {} (best-effort)",
                        link_name,
                        addr,
                        prefix_len,
                        e
                    );
                }

                // Remove the on-link /64 route.
                // Zero out the host part to get the subnet prefix.
                let octets = addr.octets();
                let subnet_prefix = Ipv6Addr::new(
                    u16::from_be_bytes([octets[0], octets[1]]),
                    u16::from_be_bytes([octets[2], octets[3]]),
                    u16::from_be_bytes([octets[4], octets[5]]),
                    u16::from_be_bytes([octets[6], octets[7]]),
                    0,
                    0,
                    0,
                    0,
                );
                if let Err(e) =
                    ipv6_ra::remove_ipv6_route(subnet_prefix, *prefix_len, None, ds_ifindex)
                {
                    log::debug!(
                        "{}: failed to remove PD route {}/{}: {} (best-effort)",
                        link_name,
                        subnet_prefix,
                        prefix_len,
                        e
                    );
                }
            }
        }
    }

    /// Start a DHCPv6 client on a link in response to RA M/O flags.
    ///
    /// If the M (Managed) flag is set, requests addresses (stateful DHCPv6).
    /// If only the O (Other) flag is set, requests configuration only (stateless).
    ///
    /// Does nothing if the link already has a DHCPv6 client.
    pub fn start_dhcpv6_from_ra(
        &mut self,
        ifindex: u32,
        managed_flag: bool,
        other_flag: bool,
    ) -> bool {
        let managed = match self.links.get(&ifindex) {
            Some(m) => m,
            None => return false,
        };

        // Already have a DHCPv6 client.
        if managed.dhcpv6_client.is_some() {
            return false;
        }

        // Neither M nor O flag set — no DHCPv6 needed.
        if !managed_flag && !other_flag {
            return false;
        }

        let link_name = managed.link.name.clone();
        let mac = managed.link.mac_bytes.clone();
        let mut mac_arr = [0u8; 6];
        if mac.len() >= 6 {
            mac_arr.copy_from_slice(&mac[..6]);
        }

        let request_addresses = managed_flag; // M flag = stateful, O only = stateless.

        // Propagate PD config from the network file if available.
        let (pd_enabled, pd_hint_len, pd_hint_addr, rapid_commit) =
            if let Some(ref cfg) = managed.config {
                let (enabled, hint_len, hint_addr) =
                    parse_prefix_delegation_hint(&cfg.dhcpv6.prefix_delegation_hint);
                (enabled, hint_len, hint_addr, cfg.dhcpv6.rapid_commit)
            } else {
                (false, 0u8, Ipv6Addr::UNSPECIFIED, false)
            };

        let dhcpv6_config = Dhcpv6ClientConfig {
            ifindex,
            ifname: link_name.clone(),
            mac: mac_arr,
            request_addresses,
            request_prefix_delegation: pd_enabled,
            prefix_hint: pd_hint_len,
            prefix_hint_address: pd_hint_addr,
            rapid_commit,
            max_attempts: 0,
            ..Default::default()
        };

        log::info!(
            "{}: starting DHCPv6 client from RA (M={} O={}, mode={}, PD={})",
            link_name,
            managed_flag,
            other_flag,
            if request_addresses {
                "stateful"
            } else {
                "stateless"
            },
            pd_enabled
        );

        let client = Dhcpv6Client::new(dhcpv6_config);
        let managed = self.links.get_mut(&ifindex).unwrap();
        managed.dhcpv6_client = Some(client);
        true
    }
}

/// Parse a `PrefixDelegationHint=` value into (enabled, prefix_len, prefix_address).
///
/// Format: `"address/prefix_len"` e.g. `"::1/56"`, `"::/48"`, or just `"::/60"`.
/// Returns `(true, prefix_len, address)` if a valid hint is given,
/// or `(false, 0, UNSPECIFIED)` if `None`.
fn parse_prefix_delegation_hint(hint: &Option<String>) -> (bool, u8, Ipv6Addr) {
    match hint {
        Some(h) if !h.is_empty() => {
            if let Some((addr_str, len_str)) = h.rsplit_once('/') {
                let prefix_len = len_str.parse::<u8>().unwrap_or(0);
                let prefix_addr = addr_str
                    .parse::<Ipv6Addr>()
                    .unwrap_or(Ipv6Addr::UNSPECIFIED);
                (true, prefix_len, prefix_addr)
            } else {
                // No slash — treat as just enabling PD with no specific hint.
                log::warn!("Invalid PrefixDelegationHint format (expected addr/len): {h}");
                (true, 0, Ipv6Addr::UNSPECIFIED)
            }
        }
        _ => (false, 0, Ipv6Addr::UNSPECIFIED),
    }
}

/// Compute the /64 subnet prefix from a delegated prefix and subnet ID.
///
/// Given a delegated prefix (e.g. `2001:db8:abcd::/48`) and a `subnet_id`
/// (e.g. `1`), returns the /64 subnet prefix: `2001:db8:abcd:0001::`.
///
/// The `subnet_id` fills the bits between `prefix_len` and bit 64.  The
/// first 64 bits of the address are treated as a big-endian u64 where the
/// top `prefix_len` bits are the network prefix and the remaining
/// `(64 - prefix_len)` bits hold the subnet ID right-aligned.
pub fn compute_pd_subnet(prefix: Ipv6Addr, prefix_len: u8, subnet_id: u64) -> Ipv6Addr {
    let octets = prefix.octets();
    let mut first64 = u64::from_be_bytes([
        octets[0], octets[1], octets[2], octets[3], octets[4], octets[5], octets[6], octets[7],
    ]);
    // Clear the subnet field (bits [prefix_len..64)).
    let subnet_bits = 64u32.saturating_sub(prefix_len as u32);
    let mask = if subnet_bits >= 64 {
        0u64
    } else {
        !((1u64 << subnet_bits) - 1)
    };
    first64 &= mask;
    // Place subnet_id into the cleared field.
    first64 |= subnet_id & ((1u64 << subnet_bits) - 1);
    let bytes = first64.to_be_bytes();
    Ipv6Addr::new(
        u16::from_be_bytes([bytes[0], bytes[1]]),
        u16::from_be_bytes([bytes[2], bytes[3]]),
        u16::from_be_bytes([bytes[4], bytes[5]]),
        u16::from_be_bytes([bytes[6], bytes[7]]),
        0,
        0,
        0,
        0,
    )
}

/// Compute the full address for a downstream interface from a delegated prefix,
/// subnet ID, and token (host part).
///
/// Combines the /64 subnet prefix with the token's interface identifier.
/// For example: prefix `2001:db8:abcd::/48`, subnet_id `1`, token `::1`
/// → `2001:db8:abcd:1::1`.
pub fn compute_pd_subnet_address(
    prefix: Ipv6Addr,
    prefix_len: u8,
    subnet_id: u64,
    token: Ipv6Addr,
) -> Ipv6Addr {
    let subnet = compute_pd_subnet(prefix, prefix_len, subnet_id);
    let s = subnet.octets();
    let t = token.octets();
    // First 8 bytes from subnet, last 8 bytes from token (host part / IID).
    Ipv6Addr::from([
        s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7], t[8], t[9], t[10], t[11], t[12], t[13],
        t[14], t[15],
    ])
}

/// Summary status of a managed link (for display by `networkctl`).
#[derive(Debug, Clone)]
pub struct LinkStatus {
    pub index: u32,
    pub name: String,
    pub link_type: String,
    pub oper_state: OperState,
    pub admin_state: AdminState,
    pub config_file: Option<String>,
    pub address: Option<String>,
    pub gateway: Option<String>,
    pub dns: Vec<String>,
    pub dhcp_state: Option<String>,
}

impl fmt::Display for LinkStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{:>3} {} {:10} {:12}",
            self.index, self.name, self.link_type, self.oper_state,
        )?;
        if let Some(ref addr) = self.address {
            write!(f, " {addr}")?;
        }
        if let Some(ref gw) = self.gateway {
            write!(f, " gw={gw}")?;
        }
        Ok(())
    }
}

/// Build a [`RuleConfig`] from a parsed [`RoutingPolicyRuleSection`].
///
/// Determines the address family from the `From=`/`To=` addresses or the
/// explicit `Family=` setting. Resolves table names, IP protocol names,
/// and rule type names to their numeric equivalents.
fn build_rule_config(
    section: &config::RoutingPolicyRuleSection,
    link_name: &str,
) -> Option<RuleConfig> {
    // Determine address family.
    let family_from_addr = section
        .from
        .as_ref()
        .and_then(|s| link::family_from_cidr(s))
        .or_else(|| section.to.as_ref().and_then(|s| link::family_from_cidr(s)));

    let family = match section.family {
        Some(RuleFamily::Ipv4) => link::af_inet(),
        Some(RuleFamily::Ipv6) => link::af_inet6(),
        Some(RuleFamily::Both) => family_from_addr.unwrap_or(link::af_inet()),
        None => family_from_addr.unwrap_or(link::af_inet()),
    };

    // Parse source prefix.
    let from = section.from.as_ref().and_then(|s| {
        config::parse_cidr(s).or_else(|| {
            log::warn!("{link_name}: invalid From= address: {s}");
            None
        })
    });

    // Parse destination prefix.
    let to = section.to.as_ref().and_then(|s| {
        config::parse_cidr(s).or_else(|| {
            log::warn!("{link_name}: invalid To= address: {s}");
            None
        })
    });

    // Resolve routing table.
    let table = section
        .table
        .as_ref()
        .and_then(|t| config::resolve_route_table(t))
        .unwrap_or(254); // default: main

    // Resolve IP protocol.
    let ip_proto = section
        .ip_protocol
        .as_ref()
        .and_then(|p| config::resolve_ip_protocol(p));

    // Resolve rule action type.
    let action = section
        .rule_type
        .as_ref()
        .and_then(|t| config::resolve_rule_type(t))
        .unwrap_or(link::fr_act_to_tbl());

    Some(RuleConfig {
        family,
        from,
        to,
        tos: section.type_of_service.unwrap_or(0),
        table,
        priority: section.priority,
        fwmark: section.firewall_mark,
        fwmask: section.firewall_mask,
        iifname: section.incoming_interface.clone(),
        oifname: section.outgoing_interface.clone(),
        sport_range: section.source_port.map(|pr| (pr.start, pr.end)),
        dport_range: section.destination_port.map(|pr| (pr.start, pr.end)),
        ip_proto,
        invert: section.invert_rule,
        uid_range: section.user.map(|ur| (ur.start, ur.end)),
        suppress_prefix_length: section.suppress_prefix_length,
        action,
    })
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::*;

    fn make_test_config(name: &str, dhcp: DhcpMode) -> NetworkConfig {
        NetworkConfig {
            path: std::path::PathBuf::from(format!("10-{name}.network")),
            match_section: MatchSection {
                names: vec![name.to_string()],
                ..Default::default()
            },
            network_section: NetworkSection {
                dhcp,
                ..Default::default()
            },
            addresses: Vec::new(),
            routes: Vec::new(),
            routing_policy_rules: Vec::new(),
            dhcpv4: config::DhcpV4Section::default(),
            dhcpv6: config::DhcpV6Section::default(),
            dhcpv6_pd: config::Dhcpv6PrefixDelegationSection::default(),
            link: LinkSection::default(),
        }
    }

    fn make_test_link(index: u32, name: &str) -> LinkInfo {
        LinkInfo {
            index,
            name: name.to_string(),
            mac: "52:54:00:12:34:56".to_string(),
            mac_bytes: vec![0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            mtu: 1500,
            flags: 0x1 | 0x40, // UP | RUNNING
            operstate: 6,      // IF_OPER_UP
        }
    }

    #[test]
    fn test_new_manager() {
        let mgr = NetworkManager::new();
        assert!(mgr.links.is_empty());
        assert!(mgr.configs.is_empty());
        assert!(!mgr.initial_config_done);
    }

    #[test]
    fn test_managed_link_unmanaged_state() {
        let managed = ManagedLink {
            link: make_test_link(1, "eth0"),
            config: None,
            dhcp_client: None,
            lease: None,
            dhcpv6_client: None,
            dhcpv6_lease: None,
            admin_state: AdminState::Up,
            static_configured: false,
            has_carrier: true,
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
            ra_state: None,
            dns6_servers: Vec::new(),
            search6_domains: Vec::new(),
            pd_assigned_addresses: Vec::new(),
        };
        assert_eq!(managed.oper_state(), OperState::Unmanaged);
    }

    #[test]
    fn test_managed_link_pending_state() {
        let managed = ManagedLink {
            link: make_test_link(2, "eth0"),
            config: Some(make_test_config("eth*", DhcpMode::Yes)),
            dhcp_client: None,
            lease: None,
            dhcpv6_client: None,
            dhcpv6_lease: None,
            admin_state: AdminState::Up,
            static_configured: true,
            has_carrier: true,
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
            ra_state: None,
            dns6_servers: Vec::new(),
            search6_domains: Vec::new(),
            pd_assigned_addresses: Vec::new(),
        };
        assert_eq!(managed.oper_state(), OperState::Pending);
    }

    #[test]
    fn test_managed_link_configured_state() {
        let managed = ManagedLink {
            link: make_test_link(3, "eth0"),
            config: Some(make_test_config("eth*", DhcpMode::No)),
            dhcp_client: None,
            lease: None,
            dhcpv6_client: None,
            dhcpv6_lease: None,
            admin_state: AdminState::Up,
            static_configured: true,
            has_carrier: true,
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
            ra_state: None,
            dns6_servers: Vec::new(),
            search6_domains: Vec::new(),
            pd_assigned_addresses: Vec::new(),
        };
        assert_eq!(managed.oper_state(), OperState::Configured);
    }

    #[test]
    fn test_managed_link_no_carrier() {
        let mut link = make_test_link(4, "eth0");
        link.flags = 0; // Not UP, not RUNNING
        let managed = ManagedLink {
            link,
            config: Some(make_test_config("eth*", DhcpMode::No)),
            dhcp_client: None,
            lease: None,
            dhcpv6_client: None,
            dhcpv6_lease: None,
            admin_state: AdminState::Up,
            static_configured: true,
            has_carrier: false,
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
            ra_state: None,
            dns6_servers: Vec::new(),
            search6_domains: Vec::new(),
            pd_assigned_addresses: Vec::new(),
        };
        assert_eq!(managed.oper_state(), OperState::NoCarrier);
    }

    #[test]
    fn test_managed_link_configuring_state() {
        let managed = ManagedLink {
            link: make_test_link(5, "eth0"),
            config: Some(make_test_config("eth*", DhcpMode::No)),
            dhcp_client: None,
            lease: None,
            dhcpv6_client: None,
            dhcpv6_lease: None,
            admin_state: AdminState::Up,
            static_configured: false,
            has_carrier: true,
            dns_servers: Vec::new(),
            search_domains: Vec::new(),
            ra_state: None,
            dns6_servers: Vec::new(),
            search6_domains: Vec::new(),
            pd_assigned_addresses: Vec::new(),
        };
        assert_eq!(managed.oper_state(), OperState::Configuring);
    }

    #[test]
    fn test_admin_state_display() {
        assert_eq!(AdminState::Up.to_string(), "configured");
        assert_eq!(AdminState::Unmanaged.to_string(), "unmanaged");
        assert_eq!(AdminState::Down.to_string(), "down");
    }

    #[test]
    fn test_oper_state_display() {
        assert_eq!(OperState::Unmanaged.to_string(), "unmanaged");
        assert_eq!(OperState::Configuring.to_string(), "configuring");
        assert_eq!(OperState::Pending.to_string(), "pending");
        assert_eq!(OperState::Configured.to_string(), "configured");
        assert_eq!(OperState::Degraded.to_string(), "degraded");
        assert_eq!(OperState::NoCarrier.to_string(), "no-carrier");
    }

    #[test]
    fn test_overall_state_initializing() {
        let mgr = NetworkManager::new();
        assert_eq!(mgr.overall_state(), "initializing");
    }

    #[test]
    fn test_overall_state_no_carrier() {
        let mut mgr = NetworkManager::new();
        mgr.initial_config_done = true;
        assert_eq!(mgr.overall_state(), "no-carrier");
    }

    #[test]
    fn test_overall_state_configured() {
        let mut mgr = NetworkManager::new();
        mgr.initial_config_done = true;
        mgr.links.insert(
            1,
            ManagedLink {
                link: make_test_link(1, "eth0"),
                config: Some(make_test_config("eth*", DhcpMode::No)),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: Vec::new(),
                search_domains: Vec::new(),
                ra_state: None,
                dns6_servers: Vec::new(),
                search6_domains: Vec::new(),
                pd_assigned_addresses: Vec::new(),
            },
        );
        assert_eq!(mgr.overall_state(), "configured");
    }

    #[test]
    fn test_status_summary_empty() {
        let mgr = NetworkManager::new();
        let status = mgr.status_summary();
        assert!(status.is_empty());
    }

    #[test]
    fn test_status_summary_single_link() {
        let mut mgr = NetworkManager::new();
        mgr.links.insert(
            2,
            ManagedLink {
                link: make_test_link(2, "ens3"),
                config: Some(make_test_config("ens*", DhcpMode::Yes)),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![Ipv4Addr::new(8, 8, 8, 8)],
                search_domains: vec!["example.com".into()],
                ra_state: None,
                dns6_servers: Vec::new(),
                search6_domains: Vec::new(),
                pd_assigned_addresses: Vec::new(),
            },
        );

        let statuses = mgr.status_summary();
        assert_eq!(statuses.len(), 1);
        assert_eq!(statuses[0].name, "ens3");
        assert_eq!(statuses[0].oper_state, OperState::Pending);
        assert_eq!(statuses[0].dns, vec!["8.8.8.8"]);
    }

    #[test]
    fn test_link_status_display() {
        let status = LinkStatus {
            index: 2,
            name: "eth0".into(),
            link_type: "ether".into(),
            oper_state: OperState::Configured,
            admin_state: AdminState::Up,
            config_file: Some("/etc/systemd/network/10-eth.network".into()),
            address: Some("192.168.1.100/24".into()),
            gateway: Some("192.168.1.1".into()),
            dns: vec!["8.8.8.8".into()],
            dhcp_state: Some("BOUND".into()),
        };

        let s = status.to_string();
        assert!(s.contains("eth0"));
        assert!(s.contains("ether"));
        assert!(s.contains("configured"));
        assert!(s.contains("192.168.1.100/24"));
        assert!(s.contains("gw=192.168.1.1"));
    }

    #[test]
    fn test_dhcp_active_links() {
        let mut mgr = NetworkManager::new();

        // Link without DHCP.
        mgr.links.insert(
            1,
            ManagedLink {
                link: make_test_link(1, "lo"),
                config: None,
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Unmanaged,
                static_configured: false,
                has_carrier: true,
                dns_servers: Vec::new(),
                search_domains: Vec::new(),
                ra_state: None,
                dns6_servers: Vec::new(),
                search6_domains: Vec::new(),
                pd_assigned_addresses: Vec::new(),
            },
        );

        // Link with DHCP.
        let dhcp_config = DhcpClientConfig {
            ifindex: 2,
            ifname: "eth0".into(),
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            ..Default::default()
        };
        mgr.links.insert(
            2,
            ManagedLink {
                link: make_test_link(2, "eth0"),
                config: Some(make_test_config("eth*", DhcpMode::Yes)),
                dhcp_client: Some(DhcpClient::new(dhcp_config)),
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: Vec::new(),
                search_domains: Vec::new(),
                ra_state: None,
                dns6_servers: Vec::new(),
                search6_domains: Vec::new(),
                pd_assigned_addresses: Vec::new(),
            },
        );

        let active = mgr.dhcp_active_links();
        assert_eq!(active.len(), 1);
        assert!(active.contains(&2));
    }

    #[test]
    fn test_all_dhcp_bound_no_dhcp() {
        let mgr = NetworkManager::new();
        assert!(mgr.all_dhcp_bound());
    }

    #[test]
    fn test_all_dhcp_bound_not_bound() {
        let mut mgr = NetworkManager::new();
        let dhcp_config = DhcpClientConfig {
            ifindex: 2,
            ifname: "eth0".into(),
            mac: [0x52, 0x54, 0x00, 0x12, 0x34, 0x56],
            ..Default::default()
        };
        mgr.links.insert(
            2,
            ManagedLink {
                link: make_test_link(2, "eth0"),
                config: Some(make_test_config("eth*", DhcpMode::Yes)),
                dhcp_client: Some(DhcpClient::new(dhcp_config)),
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: Vec::new(),
                search_domains: Vec::new(),
                ra_state: None,
                dns6_servers: Vec::new(),
                search6_domains: Vec::new(),
                pd_assigned_addresses: Vec::new(),
            },
        );

        assert!(!mgr.all_dhcp_bound());
    }

    #[test]
    fn test_load_configs_from_empty() {
        let mut mgr = NetworkManager::new();
        let dir = tempfile::tempdir().unwrap();
        mgr.load_configs_from(&[dir.path().to_path_buf()]);
        assert!(mgr.configs.is_empty());
    }

    // -----------------------------------------------------------------------
    // Routing policy rule integration tests
    // -----------------------------------------------------------------------
    #[test]
    fn test_build_rule_config_basic_ipv4() {
        let section = RoutingPolicyRuleSection {
            from: Some("192.168.1.0/24".to_string()),
            table: Some("100".to_string()),
            priority: Some(32765),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.family, link::af_inet());
        assert_eq!(rule.table, 100);
        assert_eq!(rule.priority, Some(32765));
        assert!(rule.from.is_some());
        let (addr, prefix) = rule.from.unwrap();
        assert_eq!(prefix, 24);
        assert!(matches!(addr, std::net::IpAddr::V4(_)));
    }

    #[test]
    fn test_build_rule_config_ipv6() {
        let section = RoutingPolicyRuleSection {
            from: Some("2001:db8::/32".to_string()),
            to: Some("fd00::/8".to_string()),
            table: Some("200".to_string()),
            family: Some(RuleFamily::Ipv6),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.family, link::af_inet6());
        assert_eq!(rule.table, 200);
        assert!(rule.from.is_some());
        assert!(rule.to.is_some());
    }

    #[test]
    fn test_build_rule_config_table_names() {
        let section = RoutingPolicyRuleSection {
            table: Some("main".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.table, 254);

        let section2 = RoutingPolicyRuleSection {
            table: Some("local".to_string()),
            ..Default::default()
        };
        let rule2 = build_rule_config(&section2, "eth0").unwrap();
        assert_eq!(rule2.table, 255);
    }

    #[test]
    fn test_build_rule_config_firewall_mark() {
        let section = RoutingPolicyRuleSection {
            firewall_mark: Some(0xCAFE),
            firewall_mask: Some(0xFFFF),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.fwmark, Some(0xCAFE));
        assert_eq!(rule.fwmask, Some(0xFFFF));
    }

    #[test]
    fn test_build_rule_config_with_interfaces() {
        let section = RoutingPolicyRuleSection {
            incoming_interface: Some("eth0".to_string()),
            outgoing_interface: Some("eth1".to_string()),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.iifname.as_deref(), Some("eth0"));
        assert_eq!(rule.oifname.as_deref(), Some("eth1"));
    }

    #[test]
    fn test_build_rule_config_port_ranges() {
        let section = RoutingPolicyRuleSection {
            source_port: Some(PortRange {
                start: 1024,
                end: 65535,
            }),
            destination_port: Some(PortRange { start: 80, end: 80 }),
            ip_protocol: Some("tcp".to_string()),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.sport_range, Some((1024, 65535)));
        assert_eq!(rule.dport_range, Some((80, 80)));
        assert_eq!(rule.ip_proto, Some(6)); // TCP
    }

    #[test]
    fn test_build_rule_config_uid_range() {
        let section = RoutingPolicyRuleSection {
            user: Some(UidRange {
                start: 1000,
                end: 2000,
            }),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.uid_range, Some((1000, 2000)));
    }

    #[test]
    fn test_build_rule_config_invert() {
        let section = RoutingPolicyRuleSection {
            from: Some("10.0.0.0/8".to_string()),
            invert_rule: true,
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert!(rule.invert);
    }

    #[test]
    fn test_build_rule_config_suppress_prefix() {
        let section = RoutingPolicyRuleSection {
            suppress_prefix_length: Some(0),
            table: Some("main".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.suppress_prefix_length, Some(0));
        assert_eq!(rule.table, 254);
    }

    #[test]
    fn test_build_rule_config_action_types() {
        for (type_str, expected) in &[
            ("blackhole", link::fr_act_blackhole()),
            ("unreachable", link::fr_act_unreachable()),
            ("prohibit", link::fr_act_prohibit()),
            ("table", link::fr_act_to_tbl()),
        ] {
            let section = RoutingPolicyRuleSection {
                rule_type: Some(type_str.to_string()),
                table: Some("100".to_string()),
                ..Default::default()
            };
            let rule = build_rule_config(&section, "eth0").unwrap();
            assert_eq!(
                rule.action, *expected,
                "Type={} should map to action {}",
                type_str, expected
            );
        }
    }

    #[test]
    fn test_build_rule_config_family_auto_detect_from_from() {
        // Auto-detect IPv6 from the From= address.
        let section = RoutingPolicyRuleSection {
            from: Some("2001:db8::/32".to_string()),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.family, link::af_inet6());
    }

    #[test]
    fn test_build_rule_config_family_auto_detect_from_to() {
        // Auto-detect IPv6 from the To= address when From= is absent.
        let section = RoutingPolicyRuleSection {
            to: Some("fd00::/8".to_string()),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.family, link::af_inet6());
    }

    #[test]
    fn test_build_rule_config_family_defaults_to_ipv4() {
        // When no addresses and no family, default to IPv4.
        let section = RoutingPolicyRuleSection {
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.family, link::af_inet());
    }

    #[test]
    fn test_build_rule_config_family_explicit_overrides() {
        // Explicit Family=ipv4 overrides auto-detected IPv6.
        let section = RoutingPolicyRuleSection {
            from: Some("192.168.1.0/24".to_string()),
            family: Some(RuleFamily::Ipv4),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.family, link::af_inet());
    }

    #[test]
    fn test_build_rule_config_tos() {
        let section = RoutingPolicyRuleSection {
            type_of_service: Some(16),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.tos, 16);
    }

    #[test]
    fn test_build_rule_config_no_table_defaults_main() {
        let section = RoutingPolicyRuleSection {
            from: Some("10.0.0.0/8".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.table, 254); // main
    }

    #[test]
    fn test_build_rule_config_all_fields() {
        let section = RoutingPolicyRuleSection {
            type_of_service: Some(8),
            from: Some("10.0.0.0/8".to_string()),
            to: Some("172.16.0.0/12".to_string()),
            firewall_mark: Some(0x42),
            firewall_mask: Some(0xFF),
            table: Some("100".to_string()),
            priority: Some(999),
            incoming_interface: Some("wlan0".to_string()),
            outgoing_interface: Some("eth0".to_string()),
            source_port: Some(PortRange {
                start: 5000,
                end: 6000,
            }),
            destination_port: Some(PortRange {
                start: 443,
                end: 443,
            }),
            ip_protocol: Some("udp".to_string()),
            invert_rule: true,
            family: Some(RuleFamily::Ipv4),
            user: Some(UidRange {
                start: 0,
                end: 65534,
            }),
            suppress_prefix_length: Some(0),
            rule_type: Some("table".to_string()),
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        assert_eq!(rule.tos, 8);
        assert!(rule.from.is_some());
        assert!(rule.to.is_some());
        assert_eq!(rule.fwmark, Some(0x42));
        assert_eq!(rule.fwmask, Some(0xFF));
        assert_eq!(rule.table, 100);
        assert_eq!(rule.priority, Some(999));
        assert_eq!(rule.iifname.as_deref(), Some("wlan0"));
        assert_eq!(rule.oifname.as_deref(), Some("eth0"));
        assert_eq!(rule.sport_range, Some((5000, 6000)));
        assert_eq!(rule.dport_range, Some((443, 443)));
        assert_eq!(rule.ip_proto, Some(17)); // UDP
        assert!(rule.invert);
        assert_eq!(rule.uid_range, Some((0, 65534)));
        assert_eq!(rule.suppress_prefix_length, Some(0));
        assert_eq!(rule.action, link::fr_act_to_tbl());
    }

    #[test]
    fn test_build_rule_config_invalid_from_returns_none_from() {
        let section = RoutingPolicyRuleSection {
            from: Some("not-an-address".to_string()),
            table: Some("100".to_string()),
            ..Default::default()
        };
        let rule = build_rule_config(&section, "eth0").unwrap();
        // Invalid address is silently dropped (with warning log).
        assert!(rule.from.is_none());
    }

    fn test_load_configs_from_with_files() {
        let mut mgr = NetworkManager::new();
        let dir = tempfile::tempdir().unwrap();

        std::fs::write(
            dir.path().join("10-eth.network"),
            "[Match]\nName=eth*\n\n[Network]\nDHCP=yes\n",
        )
        .unwrap();

        mgr.load_configs_from(&[dir.path().to_path_buf()]);
        assert_eq!(mgr.configs.len(), 1);
        assert_eq!(mgr.configs[0].match_section.names, vec!["eth*"]);
    }

    // -----------------------------------------------------------------------
    // DHCPv6-PD integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_prefix_delegation_hint_none() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&None);
        assert!(!enabled);
        assert_eq!(len, 0);
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED);
    }

    #[test]
    fn test_parse_prefix_delegation_hint_empty() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some(String::new()));
        assert!(!enabled);
        assert_eq!(len, 0);
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED);
    }

    #[test]
    fn test_parse_prefix_delegation_hint_slash_56() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some("::/56".to_string()));
        assert!(enabled);
        assert_eq!(len, 56);
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED);
    }

    #[test]
    fn test_parse_prefix_delegation_hint_slash_48() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some("::/48".to_string()));
        assert!(enabled);
        assert_eq!(len, 48);
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED);
    }

    #[test]
    fn test_parse_prefix_delegation_hint_with_address() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some("2001:db8::/48".to_string()));
        assert!(enabled);
        assert_eq!(len, 48);
        assert_eq!(addr, "2001:db8::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_parse_prefix_delegation_hint_slash_60() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some("::1/60".to_string()));
        assert!(enabled);
        assert_eq!(len, 60);
        assert_eq!(addr, "::1".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_parse_prefix_delegation_hint_slash_64() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some("::/64".to_string()));
        assert!(enabled);
        assert_eq!(len, 64);
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED);
    }

    #[test]
    fn test_parse_prefix_delegation_hint_no_slash() {
        // No slash — treated as enabling PD with no specific hint.
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some("yes".to_string()));
        assert!(enabled);
        assert_eq!(len, 0);
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED);
    }

    #[test]
    fn test_parse_prefix_delegation_hint_invalid_prefix_len() {
        let (enabled, len, addr) = parse_prefix_delegation_hint(&Some("::/abc".to_string()));
        assert!(enabled);
        assert_eq!(len, 0); // parse fails, falls back to 0
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED);
    }

    #[test]
    fn test_parse_prefix_delegation_hint_invalid_address() {
        let (enabled, len, addr) =
            parse_prefix_delegation_hint(&Some("not-an-addr/56".to_string()));
        assert!(enabled);
        assert_eq!(len, 56);
        assert_eq!(addr, Ipv6Addr::UNSPECIFIED); // invalid addr falls back
    }

    fn make_dhcpv6_test_config(name: &str, pd_hint: Option<&str>) -> NetworkConfig {
        NetworkConfig {
            path: std::path::PathBuf::from(format!("10-{name}.network")),
            match_section: MatchSection {
                names: vec![name.to_string()],
                ..Default::default()
            },
            network_section: NetworkSection {
                dhcp: DhcpMode::Ipv6,
                ..Default::default()
            },
            addresses: vec![],
            routes: vec![],
            routing_policy_rules: vec![],
            dhcpv4: DhcpV4Section::default(),
            dhcpv6: DhcpV6Section {
                prefix_delegation_hint: pd_hint.map(|s| s.to_string()),
                use_delegated_prefix: true,
                rapid_commit: false,
                ..Default::default()
            },
            dhcpv6_pd: config::Dhcpv6PrefixDelegationSection::default(),
            link: LinkSection::default(),
        }
    }

    fn make_test_dhcpv6_lease_with_pd() -> Dhcpv6Lease {
        use crate::dhcpv6::{IaAddress, IaNa, IaPd, IaPrefix};
        use std::time::Instant;
        Dhcpv6Lease {
            ia_na: IaNa {
                iaid: 1,
                t1: 1800,
                t2: 2700,
                addresses: vec![IaAddress {
                    address: "2001:db8::100".parse().unwrap(),
                    preferred_lifetime: 3600,
                    valid_lifetime: 7200,
                }],
                status: None,
            },
            ia_pd: Some(IaPd {
                iaid: 2,
                t1: 1800,
                t2: 2700,
                prefixes: vec![IaPrefix {
                    preferred_lifetime: 3600,
                    valid_lifetime: 7200,
                    prefix_length: 56,
                    prefix: "2001:db8:abcd::/56"
                        .parse::<Ipv6Addr>()
                        .unwrap_or("2001:db8:abcd::".parse().unwrap()),
                }],
                status: None,
            }),
            server_id: crate::dhcpv6::Duid::from_bytes(vec![
                0, 1, 0, 1, 0, 0, 0, 0, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
            ]),
            dns_servers: vec!["2001:4860:4860::8888".parse().unwrap()],
            domains: vec!["example.com".to_string()],
            preference: 0,
            obtained_at: Instant::now(),
        }
    }

    fn make_test_dhcpv6_lease_no_pd() -> Dhcpv6Lease {
        use crate::dhcpv6::{IaAddress, IaNa};
        use std::time::Instant;
        Dhcpv6Lease {
            ia_na: IaNa {
                iaid: 1,
                t1: 1800,
                t2: 2700,
                addresses: vec![IaAddress {
                    address: "2001:db8::200".parse().unwrap(),
                    preferred_lifetime: 3600,
                    valid_lifetime: 7200,
                }],
                status: None,
            },
            ia_pd: None,
            server_id: crate::dhcpv6::Duid::from_bytes(vec![
                0, 1, 0, 1, 0, 0, 0, 0, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
            ]),
            dns_servers: vec!["2001:4860:4860::8844".parse().unwrap()],
            domains: vec![],
            preference: 0,
            obtained_at: Instant::now(),
        }
    }

    fn make_test_dhcpv6_lease_pd_only() -> Dhcpv6Lease {
        use crate::dhcpv6::{IaNa, IaPd, IaPrefix};
        use std::time::Instant;
        Dhcpv6Lease {
            ia_na: IaNa::new(1),
            ia_pd: Some(IaPd {
                iaid: 2,
                t1: 1800,
                t2: 2700,
                prefixes: vec![
                    IaPrefix {
                        preferred_lifetime: 3600,
                        valid_lifetime: 7200,
                        prefix_length: 48,
                        prefix: "2001:db8:1234::".parse().unwrap(),
                    },
                    IaPrefix {
                        preferred_lifetime: 1800,
                        valid_lifetime: 3600,
                        prefix_length: 60,
                        prefix: "2001:db8:5678::".parse().unwrap(),
                    },
                ],
                status: None,
            }),
            server_id: crate::dhcpv6::Duid::from_bytes(vec![
                0, 3, 0, 1, 0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff,
            ]),
            dns_servers: vec![],
            domains: vec![],
            preference: 0,
            obtained_at: Instant::now(),
        }
    }

    #[test]
    fn test_apply_dhcpv6_lease_unknown_ifindex() {
        let mut mgr = NetworkManager::new();
        let lease = make_test_dhcpv6_lease_with_pd();
        let result = mgr.apply_dhcpv6_lease(999, &lease);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown ifindex"));
    }

    #[test]
    fn test_apply_dhcpv6_lease_stores_lease() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(1, "eth0");
        let config = make_dhcpv6_test_config("eth0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let lease = make_test_dhcpv6_lease_with_pd();
        // apply_dhcpv6_lease will fail on netlink calls in test env,
        // but that's OK — we check the DNS/domain/lease state updates.
        let _ = mgr.apply_dhcpv6_lease(1, &lease);

        let managed = mgr.links.get(&1).unwrap();
        assert!(managed.dhcpv6_lease.is_some());
        assert!(
            managed
                .dns6_servers
                .contains(&"2001:4860:4860::8888".parse().unwrap())
        );
        assert!(managed.search6_domains.contains(&"example.com".to_string()));
    }

    #[test]
    fn test_apply_dhcpv6_lease_no_pd_no_prefix_routes() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(2, "eth1");
        let config = make_dhcpv6_test_config("eth1", None);
        mgr.links.insert(
            2,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let lease = make_test_dhcpv6_lease_no_pd();
        let _ = mgr.apply_dhcpv6_lease(2, &lease);

        let managed = mgr.links.get(&2).unwrap();
        assert!(managed.dhcpv6_lease.is_some());
        // No PD in lease — delegated_prefixes should be empty.
        assert!(
            !managed
                .dhcpv6_lease
                .as_ref()
                .unwrap()
                .has_prefix_delegation()
        );
    }

    #[test]
    fn test_apply_dhcpv6_lease_use_delegated_prefix_false() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(3, "eth2");
        let mut config = make_dhcpv6_test_config("eth2", Some("::/56"));
        config.dhcpv6.use_delegated_prefix = false;
        mgr.links.insert(
            3,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let lease = make_test_dhcpv6_lease_with_pd();
        // Even though the lease has PD, use_delegated_prefix=false so routes
        // should NOT be installed. The lease is still stored, but we
        // skip the add_ipv6_pd_route calls.
        let _ = mgr.apply_dhcpv6_lease(3, &lease);

        let managed = mgr.links.get(&3).unwrap();
        assert!(managed.dhcpv6_lease.is_some());
        // The lease itself still has PD info stored.
        assert!(
            managed
                .dhcpv6_lease
                .as_ref()
                .unwrap()
                .has_prefix_delegation()
        );
    }

    #[test]
    fn test_remove_dhcpv6_lease_clears_state() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(4, "eth3");
        let config = make_dhcpv6_test_config("eth3", Some("::/56"));
        mgr.links.insert(
            4,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // First apply lease so we have state to clear.
        let lease = make_test_dhcpv6_lease_with_pd();
        let _ = mgr.apply_dhcpv6_lease(4, &lease);

        let managed = mgr.links.get(&4).unwrap();
        assert!(managed.dhcpv6_lease.is_some());
        assert!(!managed.dns6_servers.is_empty());

        // Now remove it.
        let result = mgr.remove_dhcpv6_lease(4);
        assert!(result.is_ok());

        let managed = mgr.links.get(&4).unwrap();
        assert!(managed.dhcpv6_lease.is_none());
        // DNS from the lease should be cleared.
        assert!(managed.dns6_servers.is_empty());
        assert!(managed.search6_domains.is_empty());
    }

    #[test]
    fn test_remove_dhcpv6_lease_unknown_ifindex() {
        let mut mgr = NetworkManager::new();
        let result = mgr.remove_dhcpv6_lease(999);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("unknown ifindex"));
    }

    #[test]
    fn test_remove_dhcpv6_lease_no_lease_is_ok() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(5, "eth4");
        mgr.links.insert(
            5,
            ManagedLink {
                link,
                config: None,
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Remove with no lease — should succeed with nothing to clean up.
        let result = mgr.remove_dhcpv6_lease(5);
        assert!(result.is_ok());
    }

    #[test]
    fn test_start_dhcpv6_from_ra_no_flags() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(6, "eth5");
        mgr.links.insert(
            6,
            ManagedLink {
                link,
                config: None,
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Neither M nor O → should not start.
        assert!(!mgr.start_dhcpv6_from_ra(6, false, false));
        assert!(mgr.links.get(&6).unwrap().dhcpv6_client.is_none());
    }

    #[test]
    fn test_start_dhcpv6_from_ra_managed_flag() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(7, "eth6");
        let config = make_dhcpv6_test_config("eth6", Some("::/56"));
        mgr.links.insert(
            7,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // M flag set → stateful DHCPv6 with PD from config.
        assert!(mgr.start_dhcpv6_from_ra(7, true, false));
        let managed = mgr.links.get(&7).unwrap();
        let client = managed.dhcpv6_client.as_ref().unwrap();
        assert!(client.config.request_addresses);
        assert!(client.config.request_prefix_delegation);
        assert_eq!(client.config.prefix_hint, 56);
    }

    #[test]
    fn test_start_dhcpv6_from_ra_other_flag_only() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(8, "eth7");
        mgr.links.insert(
            8,
            ManagedLink {
                link,
                config: None,
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // O flag only → stateless, no PD (no config).
        assert!(mgr.start_dhcpv6_from_ra(8, false, true));
        let managed = mgr.links.get(&8).unwrap();
        let client = managed.dhcpv6_client.as_ref().unwrap();
        assert!(!client.config.request_addresses);
        assert!(!client.config.request_prefix_delegation);
    }

    #[test]
    fn test_start_dhcpv6_from_ra_already_has_client() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(9, "eth8");
        let config = make_dhcpv6_test_config("eth8", None);
        let existing_client = Dhcpv6Client::new(Dhcpv6ClientConfig {
            ifindex: 9,
            ifname: "eth8".to_string(),
            ..Default::default()
        });
        mgr.links.insert(
            9,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: Some(existing_client),
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Already has a client → should not start a new one.
        assert!(!mgr.start_dhcpv6_from_ra(9, true, true));
    }

    #[test]
    fn test_start_dhcpv6_from_ra_unknown_ifindex() {
        let mut mgr = NetworkManager::new();
        assert!(!mgr.start_dhcpv6_from_ra(999, true, true));
    }

    #[test]
    fn test_start_dhcpv6_from_ra_pd_config_propagated() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(10, "wan0");
        let mut config = make_dhcpv6_test_config("wan0", Some("2001:db8::/48"));
        config.dhcpv6.rapid_commit = true;
        mgr.links.insert(
            10,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        assert!(mgr.start_dhcpv6_from_ra(10, true, true));
        let client = mgr.links.get(&10).unwrap().dhcpv6_client.as_ref().unwrap();
        assert!(client.config.request_prefix_delegation);
        assert_eq!(client.config.prefix_hint, 48);
        assert_eq!(
            client.config.prefix_hint_address,
            "2001:db8::".parse::<Ipv6Addr>().unwrap()
        );
        assert!(client.config.rapid_commit);
        assert!(client.config.request_addresses); // M flag set
    }

    #[test]
    fn test_apply_dhcpv6_lease_multiple_prefixes() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(11, "wan1");
        let config = make_dhcpv6_test_config("wan1", Some("::/48"));
        mgr.links.insert(
            11,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let lease = make_test_dhcpv6_lease_pd_only();
        let _ = mgr.apply_dhcpv6_lease(11, &lease);

        let managed = mgr.links.get(&11).unwrap();
        assert!(managed.dhcpv6_lease.is_some());
        let stored_lease = managed.dhcpv6_lease.as_ref().unwrap();
        assert!(stored_lease.has_prefix_delegation());
        let prefixes = stored_lease.delegated_prefixes();
        assert_eq!(prefixes.len(), 2);
        assert_eq!(prefixes[0].prefix_length, 48);
        assert_eq!(prefixes[1].prefix_length, 60);
    }

    #[test]
    fn test_apply_dhcpv6_lease_dns_merge_no_duplicates() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(12, "eth9");
        let config = make_dhcpv6_test_config("eth9", None);
        let dns_existing: Ipv6Addr = "2001:4860:4860::8888".parse().unwrap();
        mgr.links.insert(
            12,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![dns_existing],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Lease has the same DNS server — should not duplicate.
        let lease = make_test_dhcpv6_lease_with_pd();
        let _ = mgr.apply_dhcpv6_lease(12, &lease);

        let managed = mgr.links.get(&12).unwrap();
        let count = managed
            .dns6_servers
            .iter()
            .filter(|&&d| d == dns_existing)
            .count();
        assert_eq!(count, 1, "DNS server should not be duplicated");
    }

    #[test]
    fn test_remove_dhcpv6_lease_clears_pd_dns() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(13, "wan2");
        let config = make_dhcpv6_test_config("wan2", Some("::/56"));
        let pre_existing_dns: Ipv6Addr = "fe80::1".parse().unwrap();
        mgr.links.insert(
            13,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![pre_existing_dns],
                search6_domains: vec!["local.lan".to_string()],
                pd_assigned_addresses: vec![],
            },
        );

        // Apply and then remove.
        let lease = make_test_dhcpv6_lease_with_pd();
        let _ = mgr.apply_dhcpv6_lease(13, &lease);

        let managed = mgr.links.get(&13).unwrap();
        assert!(managed.dns6_servers.len() >= 2); // pre-existing + lease DNS
        assert!(managed.search6_domains.contains(&"example.com".to_string()));

        let _ = mgr.remove_dhcpv6_lease(13);

        let managed = mgr.links.get(&13).unwrap();
        assert!(managed.dhcpv6_lease.is_none());
        // Lease DNS should be gone, pre-existing should remain.
        assert!(managed.dns6_servers.contains(&pre_existing_dns));
        assert!(
            !managed
                .dns6_servers
                .contains(&"2001:4860:4860::8888".parse().unwrap())
        );
        // Lease domains should be gone, pre-existing should remain.
        assert!(managed.search6_domains.contains(&"local.lan".to_string()));
        assert!(!managed.search6_domains.contains(&"example.com".to_string()));
    }

    #[test]
    fn test_dhcpv6_active_links_includes_pd_clients() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(14, "wan3");
        let config = make_dhcpv6_test_config("wan3", Some("::/56"));
        let client = Dhcpv6Client::new(Dhcpv6ClientConfig {
            ifindex: 14,
            ifname: "wan3".to_string(),
            request_prefix_delegation: true,
            prefix_hint: 56,
            ..Default::default()
        });
        mgr.links.insert(
            14,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: Some(client),
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let active = mgr.dhcpv6_active_links();
        assert!(active.contains(&14));
    }

    #[test]
    fn test_lease_pd_fields_accessible_after_apply() {
        let mut mgr = NetworkManager::new();
        let link = make_test_link(15, "wan4");
        let config = make_dhcpv6_test_config("wan4", Some("::/48"));
        mgr.links.insert(
            15,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: false,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let lease = make_test_dhcpv6_lease_with_pd();
        let _ = mgr.apply_dhcpv6_lease(15, &lease);

        let managed = mgr.links.get(&15).unwrap();
        let stored = managed.dhcpv6_lease.as_ref().unwrap();

        // Verify PD fields are preserved.
        assert!(stored.has_prefix_delegation());
        let prefixes = stored.delegated_prefixes();
        assert_eq!(prefixes.len(), 1);
        assert_eq!(prefixes[0].prefix_length, 56);
        assert_eq!(
            prefixes[0].prefix,
            "2001:db8:abcd::".parse::<Ipv6Addr>().unwrap()
        );

        // Also verify IA_NA address is in lease.
        assert_eq!(
            stored.primary_address(),
            Some("2001:db8::100".parse().unwrap())
        );
    }

    // ── compute_pd_subnet tests ────────────────────────────────────────────

    #[test]
    fn test_compute_pd_subnet_slash48_id0() {
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        let result = compute_pd_subnet(prefix, 48, 0);
        assert_eq!(result, "2001:db8:abcd::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_slash48_id1() {
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        let result = compute_pd_subnet(prefix, 48, 1);
        assert_eq!(result, "2001:db8:abcd:1::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_slash48_id255() {
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        let result = compute_pd_subnet(prefix, 48, 255);
        assert_eq!(result, "2001:db8:abcd:ff::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_slash48_id_max() {
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        // /48 has 16 bits for subnets → max subnet_id = 65535
        let result = compute_pd_subnet(prefix, 48, 65535);
        assert_eq!(result, "2001:db8:abcd:ffff::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_slash56_id0() {
        let prefix: Ipv6Addr = "2001:db8:abcd:ab00::".parse().unwrap();
        let result = compute_pd_subnet(prefix, 56, 0);
        assert_eq!(result, "2001:db8:abcd:ab00::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_slash56_id1() {
        let prefix: Ipv6Addr = "2001:db8:abcd:ab00::".parse().unwrap();
        let result = compute_pd_subnet(prefix, 56, 1);
        assert_eq!(result, "2001:db8:abcd:ab01::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_slash56_id_max() {
        let prefix: Ipv6Addr = "2001:db8:abcd:ab00::".parse().unwrap();
        // /56 has 8 bits for subnets → max subnet_id = 255
        let result = compute_pd_subnet(prefix, 56, 255);
        assert_eq!(result, "2001:db8:abcd:abff::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_slash60_id1() {
        let prefix: Ipv6Addr = "2001:db8:abcd:abc0::".parse().unwrap();
        // /60 has 4 bits for subnets
        let result = compute_pd_subnet(prefix, 60, 1);
        assert_eq!(result, "2001:db8:abcd:abc1::".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_preserves_prefix_bits() {
        // Ensure the top prefix_len bits are untouched.
        let prefix: Ipv6Addr = "fd12:3456:7890::".parse().unwrap();
        let result = compute_pd_subnet(prefix, 48, 42);
        assert_eq!(result, "fd12:3456:7890:2a::".parse::<Ipv6Addr>().unwrap());
    }

    // ── compute_pd_subnet_address tests ────────────────────────────────────

    #[test]
    fn test_compute_pd_subnet_address_default_token() {
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        let token: Ipv6Addr = "::1".parse().unwrap();
        let result = compute_pd_subnet_address(prefix, 48, 1, token);
        assert_eq!(result, "2001:db8:abcd:1::1".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_address_custom_token() {
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        let token: Ipv6Addr = "::dead:beef".parse().unwrap();
        let result = compute_pd_subnet_address(prefix, 48, 0, token);
        assert_eq!(
            result,
            "2001:db8:abcd::dead:beef".parse::<Ipv6Addr>().unwrap()
        );
    }

    #[test]
    fn test_compute_pd_subnet_address_slash56() {
        let prefix: Ipv6Addr = "2001:db8:abcd:ab00::".parse().unwrap();
        let token: Ipv6Addr = "::1".parse().unwrap();
        let result = compute_pd_subnet_address(prefix, 56, 3, token);
        assert_eq!(result, "2001:db8:abcd:ab03::1".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_compute_pd_subnet_address_token_full_iid() {
        let prefix: Ipv6Addr = "2001:db8:abcd::".parse().unwrap();
        let token: Ipv6Addr = "::cafe:babe:dead:beef".parse().unwrap();
        let result = compute_pd_subnet_address(prefix, 48, 5, token);
        assert_eq!(
            result,
            "2001:db8:abcd:5:cafe:babe:dead:beef"
                .parse::<Ipv6Addr>()
                .unwrap()
        );
    }

    // ── distribute / remove delegated prefixes tests ───────────────────────

    fn make_pd_downstream_config(name: &str, subnet_id: Option<u64>) -> NetworkConfig {
        NetworkConfig {
            path: std::path::PathBuf::from(format!("20-{name}.network")),
            match_section: MatchSection {
                names: vec![name.to_string()],
                ..Default::default()
            },
            network_section: NetworkSection::default(),
            addresses: vec![],
            routes: vec![],
            routing_policy_rules: vec![],
            dhcpv4: config::DhcpV4Section::default(),
            dhcpv6: config::DhcpV6Section::default(),
            dhcpv6_pd: config::Dhcpv6PrefixDelegationSection {
                assign: true,
                subnet_id,
                token: "::1".parse().unwrap(),
            },
            link: LinkSection::default(),
        }
    }

    #[test]
    fn test_distribute_no_downstream() {
        // Upstream has PD but no downstream interfaces want it.
        let mut mgr = NetworkManager::new();
        let link = make_test_link(1, "wan0");
        let config = make_dhcpv6_test_config("wan0", Some("::/48"));
        mgr.links.insert(
            1,
            ManagedLink {
                link,
                config: Some(config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );
        // No downstream — should not panic.
        mgr.distribute_delegated_prefixes(1);
    }

    #[test]
    fn test_distribute_to_downstream_auto_subnet() {
        let mut mgr = NetworkManager::new();

        // Upstream WAN with PD lease.
        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Downstream LAN with auto subnet_id.
        let lan_link = make_test_link(2, "br0");
        let lan_config = make_pd_downstream_config("br0", None);
        mgr.links.insert(
            2,
            ManagedLink {
                link: lan_link,
                config: Some(lan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        mgr.distribute_delegated_prefixes(1);

        // br0 should have one PD-assigned address (auto subnet_id=0).
        let br0 = mgr.links.get(&2).unwrap();
        assert_eq!(br0.pd_assigned_addresses.len(), 1);
        let (addr, plen) = br0.pd_assigned_addresses[0];
        assert_eq!(plen, 64);
        // Prefix 2001:db8:abcd::/56, subnet_id=0, token=::1
        // → 2001:db8:abcd:0000::1 = 2001:db8:abcd::1
        assert_eq!(addr, "2001:db8:abcd::1".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_distribute_to_downstream_explicit_subnet_id() {
        let mut mgr = NetworkManager::new();

        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Downstream with explicit SubnetId=0x05.
        let lan_link = make_test_link(2, "br0");
        let lan_config = make_pd_downstream_config("br0", Some(5));
        mgr.links.insert(
            2,
            ManagedLink {
                link: lan_link,
                config: Some(lan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        mgr.distribute_delegated_prefixes(1);

        let br0 = mgr.links.get(&2).unwrap();
        assert_eq!(br0.pd_assigned_addresses.len(), 1);
        let (addr, plen) = br0.pd_assigned_addresses[0];
        assert_eq!(plen, 64);
        // Prefix 2001:db8:abcd::/56, subnet_id=5, token=::1
        // → 2001:db8:abcd:05::1
        assert_eq!(addr, "2001:db8:abcd:5::1".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_distribute_multiple_downstream_auto() {
        let mut mgr = NetworkManager::new();

        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Two downstream interfaces, both with auto subnet_id.
        for (idx, name) in [(2u32, "br0"), (3u32, "br1")] {
            let link = make_test_link(idx, name);
            let config = make_pd_downstream_config(name, None);
            mgr.links.insert(
                idx,
                ManagedLink {
                    link,
                    config: Some(config),
                    dhcp_client: None,
                    lease: None,
                    dhcpv6_client: None,
                    dhcpv6_lease: None,
                    admin_state: AdminState::Up,
                    static_configured: true,
                    has_carrier: true,
                    dns_servers: vec![],
                    search_domains: vec![],
                    ra_state: None,
                    dns6_servers: vec![],
                    search6_domains: vec![],
                    pd_assigned_addresses: vec![],
                },
            );
        }

        mgr.distribute_delegated_prefixes(1);

        // Both should have gotten addresses; subnet_ids 0 and 1 in some order.
        let br0 = mgr.links.get(&2).unwrap();
        let br1 = mgr.links.get(&3).unwrap();
        assert_eq!(br0.pd_assigned_addresses.len(), 1);
        assert_eq!(br1.pd_assigned_addresses.len(), 1);

        let (addr0, _) = br0.pd_assigned_addresses[0];
        let (addr1, _) = br1.pd_assigned_addresses[0];
        // They should be different addresses.
        assert_ne!(addr0, addr1);
    }

    #[test]
    fn test_distribute_skips_non_pd_interfaces() {
        let mut mgr = NetworkManager::new();

        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Interface without PD config (default Assign=false).
        let eth_link = make_test_link(2, "eth1");
        let eth_config = make_test_config("eth1", DhcpMode::No);
        mgr.links.insert(
            2,
            ManagedLink {
                link: eth_link,
                config: Some(eth_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        mgr.distribute_delegated_prefixes(1);

        // eth1 should NOT have any PD-assigned addresses.
        let eth1 = mgr.links.get(&2).unwrap();
        assert!(eth1.pd_assigned_addresses.is_empty());
    }

    #[test]
    fn test_distribute_no_lease_is_noop() {
        let mut mgr = NetworkManager::new();

        // Upstream without a lease.
        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None, // No lease
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let lan_link = make_test_link(2, "br0");
        let lan_config = make_pd_downstream_config("br0", None);
        mgr.links.insert(
            2,
            ManagedLink {
                link: lan_link,
                config: Some(lan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Should not panic or assign anything.
        mgr.distribute_delegated_prefixes(1);
        let br0 = mgr.links.get(&2).unwrap();
        assert!(br0.pd_assigned_addresses.is_empty());
    }

    #[test]
    fn test_distribute_unknown_upstream_is_noop() {
        let mut mgr = NetworkManager::new();
        // Calling with a non-existent ifindex should not panic.
        mgr.distribute_delegated_prefixes(999);
    }

    #[test]
    fn test_remove_distributed_prefixes_clears_tracking() {
        let mut mgr = NetworkManager::new();

        // Upstream.
        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Downstream.
        let lan_link = make_test_link(2, "br0");
        let lan_config = make_pd_downstream_config("br0", None);
        mgr.links.insert(
            2,
            ManagedLink {
                link: lan_link,
                config: Some(lan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Distribute, then remove.
        mgr.distribute_delegated_prefixes(1);
        assert!(!mgr.links.get(&2).unwrap().pd_assigned_addresses.is_empty());

        mgr.remove_distributed_prefixes(1);
        assert!(mgr.links.get(&2).unwrap().pd_assigned_addresses.is_empty());
    }

    #[test]
    fn test_distribute_does_not_assign_to_upstream() {
        // Upstream interface should not assign PD to itself even if it
        // happens to have Assign=yes (unlikely but check).
        let mut mgr = NetworkManager::new();

        let wan_link = make_test_link(1, "wan0");
        let mut wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        wan_config.dhcpv6_pd.assign = true; // Unusual but test it.
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        mgr.distribute_delegated_prefixes(1);

        // Upstream should NOT have gotten a PD-assigned address on itself.
        let wan = mgr.links.get(&1).unwrap();
        assert!(wan.pd_assigned_addresses.is_empty());
    }

    #[test]
    fn test_distribute_custom_token() {
        let mut mgr = NetworkManager::new();

        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_with_pd()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        // Downstream with custom token.
        let lan_link = make_test_link(2, "br0");
        let mut lan_config = make_pd_downstream_config("br0", Some(0));
        lan_config.dhcpv6_pd.token = "::cafe".parse().unwrap();
        mgr.links.insert(
            2,
            ManagedLink {
                link: lan_link,
                config: Some(lan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        mgr.distribute_delegated_prefixes(1);

        let br0 = mgr.links.get(&2).unwrap();
        assert_eq!(br0.pd_assigned_addresses.len(), 1);
        let (addr, _) = br0.pd_assigned_addresses[0];
        // Prefix 2001:db8:abcd::/56, subnet_id=0, token=::cafe
        assert_eq!(addr, "2001:db8:abcd::cafe".parse::<Ipv6Addr>().unwrap());
    }

    #[test]
    fn test_distribute_pd_only_lease() {
        // Use a lease that has PD but no IA_NA addresses.
        let mut mgr = NetworkManager::new();

        let wan_link = make_test_link(1, "wan0");
        let wan_config = make_dhcpv6_test_config("wan0", Some("::/56"));
        mgr.links.insert(
            1,
            ManagedLink {
                link: wan_link,
                config: Some(wan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: Some(make_test_dhcpv6_lease_pd_only()),
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        let lan_link = make_test_link(2, "br0");
        let lan_config = make_pd_downstream_config("br0", None);
        mgr.links.insert(
            2,
            ManagedLink {
                link: lan_link,
                config: Some(lan_config),
                dhcp_client: None,
                lease: None,
                dhcpv6_client: None,
                dhcpv6_lease: None,
                admin_state: AdminState::Up,
                static_configured: true,
                has_carrier: true,
                dns_servers: vec![],
                search_domains: vec![],
                ra_state: None,
                dns6_servers: vec![],
                search6_domains: vec![],
                pd_assigned_addresses: vec![],
            },
        );

        mgr.distribute_delegated_prefixes(1);

        let br0 = mgr.links.get(&2).unwrap();
        // PD-only lease has 2 delegated prefixes, so 2 addresses assigned.
        assert_eq!(br0.pd_assigned_addresses.len(), 2);
    }
}
