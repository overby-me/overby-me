//! Shared `.link` file parsing for `systemd.link(5)` configuration.
//!
//! This module provides types and functions for loading, parsing, and matching
//! `.link` configuration files. These files control link-level settings such as
//! interface naming, MAC address policy, MTU, and offload settings.
//!
//! Used by both `systemd-networkd` (for link configuration) and `systemd-udevd`
//! (via the `net_setup_link` builtin for interface naming during device setup).
//!
//! # Search paths
//!
//! `.link` files are loaded from (in priority order):
//! 1. `/etc/systemd/network/`
//! 2. `/run/systemd/network/`
//! 3. `/usr/lib/systemd/network/`
//! 4. `/lib/systemd/network/`
//! 5. NixOS package-relative paths (auto-detected)
//!
//! Files are deduplicated by filename (first occurrence wins) and sorted
//! lexicographically. The first matching `.link` file is used.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A parsed `.link` file.
#[derive(Debug, Clone)]
pub struct LinkFileConfig {
    /// Original file path (for diagnostics).
    pub path: PathBuf,

    /// Any `.link.d/*.conf` drop-in files merged on top of the base file,
    /// in application order. Populated by `load_link_configs_from`.
    pub dropin_paths: Vec<PathBuf>,

    /// `[Match]` section — determines which links this config applies to.
    pub match_section: LinkMatchSection,

    /// `[Link]` section — link-level settings to apply.
    pub link_section: LinkSettingsSection,
}

/// `[Match]` section of a `.link` file.
///
/// Determines which network devices this `.link` file applies to.
/// All specified conditions must match for the file to apply (AND logic).
#[derive(Debug, Clone, Default)]
pub struct LinkMatchSection {
    /// `MACAddress=` — match by permanent hardware address.
    pub mac_addresses: Vec<String>,

    /// `OriginalName=` — match by the udev-assigned original interface name
    /// (before any renaming). Supports glob patterns.
    pub original_names: Vec<String>,

    /// `Path=` — match by sysfs device path (e.g. `pci-0000:02:00.0-*`).
    pub paths: Vec<String>,

    /// `Driver=` — match by kernel driver name.
    pub drivers: Vec<String>,

    /// `Type=` — match by device type (e.g. `ether`, `wlan`).
    pub types: Vec<String>,

    /// `Property=` — match by udev property (KEY=VALUE).
    pub properties: Vec<String>,

    /// `Host=` — match by hostname.
    pub host: Option<String>,

    /// `Virtualization=` — match by virtualization type.
    pub virtualization: Option<String>,

    /// `KernelCommandLine=` — match by kernel command line content.
    pub kernel_command_line: Option<String>,

    /// `KernelVersion=` — match by kernel version string.
    pub kernel_version: Option<String>,

    /// `Architecture=` — match by CPU architecture.
    pub architecture: Option<String>,

    /// `Credential=` — match by system credential.
    pub credential: Option<String>,
}

impl LinkMatchSection {
    /// Check whether this match section applies to a given interface.
    ///
    /// If no match criteria are specified, the section matches everything.
    /// Otherwise, all specified criteria must match (AND logic).
    pub fn matches_interface(
        &self,
        original_name: &str,
        mac: Option<&str>,
        driver: Option<&str>,
        dev_type: Option<&str>,
        path: Option<&str>,
    ) -> bool {
        // If OriginalName is specified, at least one pattern must match.
        if !self.original_names.is_empty()
            && !self
                .original_names
                .iter()
                .any(|pat| glob_match(pat, original_name))
        {
            return false;
        }

        // MAC address match.
        if !self.mac_addresses.is_empty() {
            match mac {
                Some(m) => {
                    if !self.mac_addresses.iter().any(|a| a.eq_ignore_ascii_case(m)) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        // Driver match.
        if !self.drivers.is_empty() {
            match driver {
                Some(d) => {
                    if !self.drivers.iter().any(|pat| glob_match(pat, d)) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        // Type match.
        if !self.types.is_empty() {
            match dev_type {
                Some(t) => {
                    if !self.types.iter().any(|pat| glob_match(pat, t)) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        // Path match.
        if !self.paths.is_empty() {
            match path {
                Some(p) => {
                    if !self.paths.iter().any(|pat| glob_match(pat, p)) {
                        return false;
                    }
                }
                None => return false,
            }
        }

        true
    }
}

/// Naming policy for network interfaces.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NamePolicy {
    /// Use the kernel's default name.
    Kernel,
    /// Use the database (udev) name.
    Database,
    /// Use onboard device index.
    Onboard,
    /// Use slot (hot-plug) based name.
    Slot,
    /// Use PCI path-based name.
    Path,
    /// Use MAC address-based name.
    Mac,
    /// Keep the name already set.
    Keep,
}

impl NamePolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "kernel" => Some(Self::Kernel),
            "database" => Some(Self::Database),
            "onboard" => Some(Self::Onboard),
            "slot" => Some(Self::Slot),
            "path" => Some(Self::Path),
            "mac" => Some(Self::Mac),
            "keep" => Some(Self::Keep),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Kernel => "kernel",
            Self::Database => "database",
            Self::Onboard => "onboard",
            Self::Slot => "slot",
            Self::Path => "path",
            Self::Mac => "mac",
            Self::Keep => "keep",
        }
    }
}

/// MAC address assignment policy.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MACAddressPolicy {
    /// Use the permanent hardware address.
    Persistent,
    /// Generate a random MAC address.
    Random,
    /// Do not set a MAC address policy.
    None,
}

impl MACAddressPolicy {
    pub fn parse(s: &str) -> Option<Self> {
        match s.trim().to_lowercase().as_str() {
            "persistent" => Some(Self::Persistent),
            "random" => Some(Self::Random),
            "none" => Some(Self::None),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Persistent => "persistent",
            Self::Random => "random",
            Self::None => "none",
        }
    }
}

/// `[Link]` section of a `.link` file — settings to apply to matched links.
#[derive(Debug, Clone, Default)]
pub struct LinkSettingsSection {
    /// `Description=` — human-readable description.
    pub description: Option<String>,

    /// `Name=` — explicit interface name.
    pub name: Option<String>,

    /// `NamePolicy=` — ordered list of naming policies.
    pub name_policy: Vec<NamePolicy>,

    /// `AlternativeNamesPolicy=` — alternative name policies.
    pub alternative_names_policy: Vec<NamePolicy>,

    /// `AlternativeName=` — explicit alternative name(s).
    pub alternative_names: Vec<String>,

    /// `MACAddressPolicy=` — MAC address assignment policy.
    pub mac_address_policy: Option<MACAddressPolicy>,

    /// `MACAddress=` — explicit MAC address to set.
    pub mac_address: Option<String>,

    /// `MTUBytes=` — maximum transmission unit (bytes).
    pub mtu: Option<u32>,

    /// `BitsPerSecond=` — link speed.
    pub bits_per_second: Option<u64>,

    /// `Duplex=` — link duplex mode (`full` or `half`).
    pub duplex: Option<String>,

    /// `AutoNegotiation=` — enable/disable auto-negotiation.
    pub auto_negotiation: Option<bool>,

    /// `WakeOnLan=` — wake-on-LAN modes.
    pub wake_on_lan: Option<String>,

    /// `Port=` — port type (`tp`, `aui`, `bnc`, `mii`, `fibre`).
    pub port: Option<String>,

    /// `GenericSegmentOffloadMaxBytes=`
    pub gso_max_bytes: Option<u32>,

    /// `GenericSegmentOffloadMaxSegments=`
    pub gso_max_segments: Option<u32>,

    /// `ReceiveChecksumOffload=`
    pub rx_checksum_offload: Option<bool>,

    /// `TransmitChecksumOffload=`
    pub tx_checksum_offload: Option<bool>,

    /// `TCPSegmentationOffload=`
    pub tcp_segmentation_offload: Option<bool>,

    /// `TCP6SegmentationOffload=`
    pub tcp6_segmentation_offload: Option<bool>,

    /// `GenericReceiveOffload=`
    pub generic_receive_offload: Option<bool>,

    /// `GenericSegmentationOffload=`
    pub generic_segmentation_offload: Option<bool>,

    /// `LargeReceiveOffload=`
    pub large_receive_offload: Option<bool>,

    /// `RxChannels=`
    pub rx_channels: Option<u32>,

    /// `TxChannels=`
    pub tx_channels: Option<u32>,

    /// `OtherChannels=`
    pub other_channels: Option<u32>,

    /// `CombinedChannels=`
    pub combined_channels: Option<u32>,

    /// `Unmanaged=` — if true, networkd ignores this link.
    pub unmanaged: Option<bool>,

    /// `RequiredForOnline=` — whether this link is required for online.
    pub required_for_online: Option<bool>,

    /// `RequiredFamilyForOnline=` — address family required for online.
    pub required_family_for_online: Option<String>,

    /// `ActivationPolicy=` — when to bring the link up.
    pub activation_policy: Option<String>,

    /// `Property=KEY=VALUE ...` — set udev properties on the matched link.
    /// Each entry is the raw `KEY=VALUE` string, parsed later by the caller.
    pub properties: Vec<String>,

    /// `UnsetProperty=KEY ...` — remove udev properties previously set
    /// by `Property=` or other rules.
    pub unset_properties: Vec<String>,

    /// `ImportProperty=KEY ...` — import a matching property from the
    /// current udev event environment. For rust-systemd this is currently
    /// treated the same as a no-op (the property is already in the env).
    pub import_properties: Vec<String>,
}

// ---------------------------------------------------------------------------
// Loading
// ---------------------------------------------------------------------------

/// Standard search directories for `.link` files.
pub const LINK_CONFIG_SEARCH_DIRS: &[&str] = &[
    "/etc/systemd/network",
    "/run/systemd/network",
    "/usr/lib/systemd/network",
    "/lib/systemd/network",
];

/// Load all `.link` configuration files from the standard search directories.
///
/// Also probes NixOS package-relative paths (relative to the current executable).
pub fn load_link_configs() -> Vec<LinkFileConfig> {
    let mut search_dirs: Vec<PathBuf> = LINK_CONFIG_SEARCH_DIRS.iter().map(PathBuf::from).collect();

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

    load_link_configs_from(&search_dirs)
}

/// Load `.link` files from the given directories, deduplicating by filename.
///
/// Earlier directories take priority. Within a directory, files are sorted
/// lexicographically by name.
pub fn load_link_configs_from(dirs: &[PathBuf]) -> Vec<LinkFileConfig> {
    let mut seen: HashMap<String, PathBuf> = HashMap::new();
    let mut configs = Vec::new();

    for dir in dirs {
        let entries = match fs::read_dir(dir) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut files: Vec<_> = entries
            .filter_map(|e| e.ok())
            .filter(|e| e.path().extension().is_some_and(|ext| ext == "link"))
            .collect();

        files.sort_by_key(|e| e.file_name());

        for entry in files {
            let name = entry.file_name().to_string_lossy().to_string();
            if seen.contains_key(&name) {
                continue;
            }
            let path = entry.path();
            seen.insert(name.clone(), path.clone());

            match parse_link_file(&path) {
                Ok(mut cfg) => {
                    // Apply drop-ins from <name>.link.d/*.conf across all
                    // search dirs (later dirs with the same drop-in name
                    // win). Merges settings on top of the base config.
                    let dropin_dir_name = format!("{name}.d");
                    let mut dropin_files: Vec<PathBuf> = Vec::new();
                    let mut dropin_seen: std::collections::HashSet<String> =
                        std::collections::HashSet::new();
                    for d in dirs {
                        let dropin_dir = d.join(&dropin_dir_name);
                        if let Ok(d_entries) = fs::read_dir(&dropin_dir) {
                            for de in d_entries.flatten() {
                                let dname = de.file_name().to_string_lossy().to_string();
                                if !dname.ends_with(".conf") {
                                    continue;
                                }
                                if dropin_seen.insert(dname.clone()) {
                                    dropin_files.push(de.path());
                                }
                            }
                        }
                    }
                    dropin_files
                        .sort_by(|a, b| a.file_name().unwrap_or_default().cmp(b.file_name().unwrap_or_default()));
                    for dp in &dropin_files {
                        if let Ok(content) = fs::read_to_string(dp) {
                            apply_dropin_to_link_config(&content, &mut cfg);
                            cfg.dropin_paths.push(dp.clone());
                        }
                    }
                    configs.push(cfg);
                }
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

// ---------------------------------------------------------------------------
// Parsing
// ---------------------------------------------------------------------------

/// Parse a single `.link` file from disk.
pub fn parse_link_file(path: &Path) -> Result<LinkFileConfig, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("read {}: {}", path.display(), e))?;
    parse_link_file_content(&content, path)
}

/// Parse `.link` file content into a [`LinkFileConfig`].
pub fn parse_link_file_content(content: &str, path: &Path) -> Result<LinkFileConfig, String> {
    let mut match_section = LinkMatchSection::default();
    let mut link_section = LinkSettingsSection::default();
    let mut current_section = String::new();

    for line in content.lines() {
        let line = line.trim();

        // Skip comments and empty lines.
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Section header.
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_lowercase();
            continue;
        }

        // Key=Value
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        match current_section.as_str() {
            "match" => parse_link_match_entry(key, value, &mut match_section),
            "link" => parse_link_settings_entry(key, value, &mut link_section),
            _ => {} // Ignore unknown sections.
        }
    }

    Ok(LinkFileConfig {
        path: path.to_path_buf(),
        dropin_paths: Vec::new(),
        match_section,
        link_section,
    })
}

/// Apply a `.link.d/*.conf` drop-in's content to a base `LinkFileConfig`.
/// Only [Link]-section settings are merged — [Match] is ignored since
/// drop-ins do not alter which links a file applies to.
pub fn apply_dropin_to_link_config(content: &str, cfg: &mut LinkFileConfig) {
    let mut current_section = String::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') && line.ends_with(']') {
            current_section = line[1..line.len() - 1].to_lowercase();
            continue;
        }
        let (key, value) = match line.split_once('=') {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };
        if current_section == "link" {
            parse_link_settings_entry(key, value, &mut cfg.link_section);
        }
    }
}

fn parse_link_match_entry(key: &str, value: &str, section: &mut LinkMatchSection) {
    match key {
        "MACAddress" => {
            section.mac_addresses.extend(split_whitespace_values(value));
        }
        "OriginalName" => {
            section
                .original_names
                .extend(split_whitespace_values(value));
        }
        "Path" => {
            section.paths.extend(split_whitespace_values(value));
        }
        "Driver" => {
            section.drivers.extend(split_whitespace_values(value));
        }
        "Type" => {
            section.types.extend(split_whitespace_values(value));
        }
        "Property" => {
            section.properties.push(value.to_string());
        }
        "Host" => {
            section.host = Some(value.to_string());
        }
        "Virtualization" => {
            section.virtualization = Some(value.to_string());
        }
        "KernelCommandLine" => {
            section.kernel_command_line = Some(value.to_string());
        }
        "KernelVersion" => {
            section.kernel_version = Some(value.to_string());
        }
        "Architecture" => {
            section.architecture = Some(value.to_string());
        }
        "Credential" => {
            section.credential = Some(value.to_string());
        }
        _ => {}
    }
}

fn parse_link_settings_entry(key: &str, value: &str, section: &mut LinkSettingsSection) {
    match key {
        "Description" => {
            section.description = Some(value.to_string());
        }
        "Name" => {
            section.name = Some(value.to_string());
        }
        "NamePolicy" => {
            section.name_policy = value
                .split_whitespace()
                .filter_map(NamePolicy::parse)
                .collect();
        }
        "AlternativeNamesPolicy" => {
            section.alternative_names_policy = value
                .split_whitespace()
                .filter_map(NamePolicy::parse)
                .collect();
        }
        "AlternativeName" => {
            section.alternative_names.push(value.to_string());
        }
        "MACAddressPolicy" => {
            section.mac_address_policy = MACAddressPolicy::parse(value);
        }
        "MACAddress" => {
            section.mac_address = Some(value.to_string());
        }
        "MTUBytes" => {
            section.mtu = parse_bytes_value(value);
        }
        "BitsPerSecond" => {
            section.bits_per_second = parse_bytes_value(value).map(|v| v as u64);
        }
        "Duplex" => {
            section.duplex = Some(value.to_string());
        }
        "AutoNegotiation" => {
            section.auto_negotiation = Some(parse_bool(value));
        }
        "WakeOnLan" => {
            section.wake_on_lan = Some(value.to_string());
        }
        "Port" => {
            section.port = Some(value.to_string());
        }
        "GenericSegmentOffloadMaxBytes" => {
            section.gso_max_bytes = parse_bytes_value(value);
        }
        "GenericSegmentOffloadMaxSegments" => {
            section.gso_max_segments = value.parse().ok();
        }
        "ReceiveChecksumOffload" => {
            section.rx_checksum_offload = Some(parse_bool(value));
        }
        "TransmitChecksumOffload" => {
            section.tx_checksum_offload = Some(parse_bool(value));
        }
        "TCPSegmentationOffload" => {
            section.tcp_segmentation_offload = Some(parse_bool(value));
        }
        "TCP6SegmentationOffload" => {
            section.tcp6_segmentation_offload = Some(parse_bool(value));
        }
        "GenericReceiveOffload" => {
            section.generic_receive_offload = Some(parse_bool(value));
        }
        "GenericSegmentationOffload" => {
            section.generic_segmentation_offload = Some(parse_bool(value));
        }
        "LargeReceiveOffload" => {
            section.large_receive_offload = Some(parse_bool(value));
        }
        "RxChannels" => {
            section.rx_channels = value.parse().ok();
        }
        "TxChannels" => {
            section.tx_channels = value.parse().ok();
        }
        "OtherChannels" => {
            section.other_channels = value.parse().ok();
        }
        "CombinedChannels" => {
            section.combined_channels = value.parse().ok();
        }
        "Unmanaged" => {
            section.unmanaged = Some(parse_bool(value));
        }
        "RequiredForOnline" => {
            section.required_for_online = Some(parse_bool(value));
        }
        "RequiredFamilyForOnline" => {
            section.required_family_for_online = Some(value.to_string());
        }
        "ActivationPolicy" => {
            section.activation_policy = Some(value.to_string());
        }
        "Property" => {
            // Empty `Property=` resets the list — matches upstream's
            // behavior where a later drop-in can clear earlier entries
            // before appending its own.
            if value.is_empty() {
                section.properties.clear();
            } else {
                for entry in split_whitespace_values(value) {
                    section.properties.push(entry);
                }
            }
        }
        "UnsetProperty" => {
            if value.is_empty() {
                section.unset_properties.clear();
            } else {
                for entry in split_whitespace_values(value) {
                    section.unset_properties.push(entry);
                }
            }
        }
        "ImportProperty" => {
            if value.is_empty() {
                section.import_properties.clear();
            } else {
                for entry in split_whitespace_values(value) {
                    section.import_properties.push(entry);
                }
            }
        }
        _ => {}
    }
}

// ---------------------------------------------------------------------------
// Matching
// ---------------------------------------------------------------------------

/// Find the first matching `.link` configuration for a given interface.
///
/// Returns `None` if no `.link` file matches the provided interface properties.
pub fn find_matching_link_config<'a>(
    configs: &'a [LinkFileConfig],
    original_name: &str,
    mac: Option<&str>,
    driver: Option<&str>,
    dev_type: Option<&str>,
    path: Option<&str>,
) -> Option<&'a LinkFileConfig> {
    configs.iter().find(|cfg| {
        cfg.match_section
            .matches_interface(original_name, mac, driver, dev_type, path)
    })
}

/// Resolve the interface name to use based on a matched `.link` file and
/// available naming information from udev environment variables.
///
/// This implements the `NamePolicy=` logic: iterate through each policy in
/// order and return the first name that is available in the environment.
/// If no policy matches, fall back to the explicit `Name=` setting.
///
/// # Arguments
///
/// * `link` - The matched `.link` file configuration.
/// * `get_env` - A closure that looks up a udev environment variable by name.
///   For example, `|key| event.env.get(key).cloned()`.
///
/// # Returns
///
/// The resolved interface name, or `None` if no name could be determined.
pub fn resolve_name_from_policy<F>(link: &LinkFileConfig, get_env: F) -> Option<String>
where
    F: Fn(&str) -> Option<String>,
{
    // Try each NamePolicy entry in order.
    for policy in &link.link_section.name_policy {
        let env_key = match policy {
            NamePolicy::Kernel => continue, // kernel name = keep as-is
            NamePolicy::Database => "ID_NET_NAME_FROM_DATABASE",
            NamePolicy::Onboard => "ID_NET_NAME_ONBOARD",
            NamePolicy::Slot => "ID_NET_NAME_SLOT",
            NamePolicy::Path => "ID_NET_NAME_PATH",
            NamePolicy::Mac => "ID_NET_NAME_MAC",
            NamePolicy::Keep => return None, // keep = don't rename
        };

        if let Some(name) = get_env(env_key)
            && !name.is_empty()
        {
            return Some(name);
        }
    }

    // Fall back to explicit Name=.
    link.link_section.name.clone()
}

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

fn parse_bool(value: &str) -> bool {
    matches!(value.to_lowercase().as_str(), "yes" | "true" | "1" | "on")
}

fn parse_bytes_value(value: &str) -> Option<u32> {
    let value = value.trim();
    // Support suffixes: K, M, G (case-insensitive, with optional 'B').
    let (num_str, multiplier) =
        if let Some(s) = value.strip_suffix('G').or_else(|| value.strip_suffix("GB")) {
            (s.trim(), 1024 * 1024 * 1024)
        } else if let Some(s) = value.strip_suffix('M').or_else(|| value.strip_suffix("MB")) {
            (s.trim(), 1024 * 1024)
        } else if let Some(s) = value.strip_suffix('K').or_else(|| value.strip_suffix("KB")) {
            (s.trim(), 1024)
        } else {
            (value, 1)
        };
    num_str.parse::<u32>().ok().map(|n| n * multiplier)
}

fn split_whitespace_values(value: &str) -> Vec<String> {
    value.split_whitespace().map(|s| s.to_string()).collect()
}

/// Minimalist glob matching supporting `*` (any chars) and `?` (single char).
pub fn glob_match(pattern: &str, text: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let txt: Vec<char> = text.chars().collect();
    glob_match_inner(&pat, &txt)
}

fn glob_match_inner(pattern: &[char], text: &[char]) -> bool {
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star_pi, mut star_ti) = (usize::MAX, 0usize);

    while ti < text.len() {
        if pi < pattern.len() && (pattern[pi] == '?' || pattern[pi] == text[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < pattern.len() && pattern[pi] == '*' {
            star_pi = pi;
            star_ti = ti;
            pi += 1;
        } else if star_pi != usize::MAX {
            pi = star_pi + 1;
            star_ti += 1;
            ti = star_ti;
        } else {
            return false;
        }
    }

    while pi < pattern.len() && pattern[pi] == '*' {
        pi += 1;
    }

    pi == pattern.len()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    // ── Parsing tests ──────────────────────────────────────────────────────

    #[test]
    fn test_parse_link_file_basic() {
        let content = "\
[Match]
OriginalName=en*

[Link]
Name=eth0
";
        let cfg = parse_link_file_content(content, Path::new("10-eth.link")).unwrap();
        assert_eq!(cfg.match_section.original_names, vec!["en*"]);
        assert_eq!(cfg.link_section.name.as_deref(), Some("eth0"));
    }

    #[test]
    fn test_parse_link_file_name_policy() {
        let content = "\
[Match]
OriginalName=*

[Link]
NamePolicy=kernel database onboard slot path
AlternativeNamesPolicy=database onboard slot path mac
";
        let cfg = parse_link_file_content(content, Path::new("99-default.link")).unwrap();
        assert_eq!(
            cfg.link_section.name_policy,
            vec![
                NamePolicy::Kernel,
                NamePolicy::Database,
                NamePolicy::Onboard,
                NamePolicy::Slot,
                NamePolicy::Path,
            ]
        );
        assert_eq!(
            cfg.link_section.alternative_names_policy,
            vec![
                NamePolicy::Database,
                NamePolicy::Onboard,
                NamePolicy::Slot,
                NamePolicy::Path,
                NamePolicy::Mac,
            ]
        );
    }

    #[test]
    fn test_parse_link_file_mac_address_policy_persistent() {
        let content = "\
[Link]
MACAddressPolicy=persistent
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.mac_address_policy,
            Some(MACAddressPolicy::Persistent)
        );
    }

    #[test]
    fn test_parse_link_file_mac_address_policy_random() {
        let content = "\
[Link]
MACAddressPolicy=random
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.mac_address_policy,
            Some(MACAddressPolicy::Random)
        );
    }

    #[test]
    fn test_parse_link_file_mac_address_policy_none() {
        let content = "\
[Link]
MACAddressPolicy=none
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.mac_address_policy,
            Some(MACAddressPolicy::None)
        );
    }

    #[test]
    fn test_parse_link_file_match_mac() {
        let content = "\
[Match]
MACAddress=00:11:22:33:44:55 aa:bb:cc:dd:ee:ff

[Link]
Name=eth0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.mac_addresses.len(), 2);
        assert_eq!(cfg.match_section.mac_addresses[0], "00:11:22:33:44:55");
        assert_eq!(cfg.match_section.mac_addresses[1], "aa:bb:cc:dd:ee:ff");
    }

    #[test]
    fn test_parse_link_file_match_driver() {
        let content = "\
[Match]
Driver=virtio_net

[Link]
Name=virt0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.drivers, vec!["virtio_net"]);
    }

    #[test]
    fn test_parse_link_file_match_type() {
        let content = "\
[Match]
Type=ether

[Link]
Name=eth0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.types, vec!["ether"]);
    }

    #[test]
    fn test_parse_link_file_match_path() {
        let content = "\
[Match]
Path=pci-0000:02:00.0-*

[Link]
Name=lan0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.paths, vec!["pci-0000:02:00.0-*"]);
    }

    #[test]
    fn test_parse_link_file_match_host_virt_arch() {
        let content = "\
[Match]
Host=myserver
Virtualization=kvm
Architecture=x86-64

[Link]
Name=eth0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.host.as_deref(), Some("myserver"));
        assert_eq!(cfg.match_section.virtualization.as_deref(), Some("kvm"));
        assert_eq!(cfg.match_section.architecture.as_deref(), Some("x86-64"));
    }

    #[test]
    fn test_parse_link_file_match_property() {
        let content = "\
[Match]
Property=ID_VENDOR=ACME

[Link]
Name=acme0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.properties, vec!["ID_VENDOR=ACME"]);
    }

    #[test]
    fn test_parse_link_file_offload_settings() {
        let content = "\
[Link]
ReceiveChecksumOffload=yes
TransmitChecksumOffload=no
TCPSegmentationOffload=true
TCP6SegmentationOffload=false
GenericReceiveOffload=1
GenericSegmentationOffload=0
LargeReceiveOffload=on
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.link_section.rx_checksum_offload, Some(true));
        assert_eq!(cfg.link_section.tx_checksum_offload, Some(false));
        assert_eq!(cfg.link_section.tcp_segmentation_offload, Some(true));
        assert_eq!(cfg.link_section.tcp6_segmentation_offload, Some(false));
        assert_eq!(cfg.link_section.generic_receive_offload, Some(true));
        assert_eq!(cfg.link_section.generic_segmentation_offload, Some(false));
        assert_eq!(cfg.link_section.large_receive_offload, Some(true));
    }

    #[test]
    fn test_parse_link_file_channel_settings() {
        let content = "\
[Link]
RxChannels=4
TxChannels=4
OtherChannels=2
CombinedChannels=8
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.link_section.rx_channels, Some(4));
        assert_eq!(cfg.link_section.tx_channels, Some(4));
        assert_eq!(cfg.link_section.other_channels, Some(2));
        assert_eq!(cfg.link_section.combined_channels, Some(8));
    }

    #[test]
    fn test_parse_link_file_misc_settings() {
        let content = "\
[Link]
MTUBytes=9000
BitsPerSecond=1G
Duplex=full
AutoNegotiation=yes
WakeOnLan=magic
Port=tp
Unmanaged=no
RequiredForOnline=yes
ActivationPolicy=up
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.link_section.mtu, Some(9000));
        assert_eq!(cfg.link_section.bits_per_second, Some(1024 * 1024 * 1024));
        assert_eq!(cfg.link_section.duplex.as_deref(), Some("full"));
        assert_eq!(cfg.link_section.auto_negotiation, Some(true));
        assert_eq!(cfg.link_section.wake_on_lan.as_deref(), Some("magic"));
        assert_eq!(cfg.link_section.port.as_deref(), Some("tp"));
        assert_eq!(cfg.link_section.unmanaged, Some(false));
        assert_eq!(cfg.link_section.required_for_online, Some(true));
        assert_eq!(cfg.link_section.activation_policy.as_deref(), Some("up"));
    }

    #[test]
    fn test_parse_link_file_explicit_mac() {
        let content = "\
[Link]
MACAddress=00:11:22:33:44:55
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.mac_address.as_deref(),
            Some("00:11:22:33:44:55")
        );
    }

    #[test]
    fn test_parse_link_file_alternative_names() {
        let content = "\
[Link]
AlternativeName=altname0
AlternativeName=altname1
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.alternative_names,
            vec!["altname0", "altname1"]
        );
    }

    #[test]
    fn test_parse_link_file_comments_and_blanks() {
        let content = "\
# This is a comment
; This is also a comment

[Match]
OriginalName=en*

# Another comment
[Link]
Name=eth0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.original_names, vec!["en*"]);
        assert_eq!(cfg.link_section.name.as_deref(), Some("eth0"));
    }

    #[test]
    fn test_parse_link_file_unknown_sections_ignored() {
        let content = "\
[Match]
OriginalName=en*

[SomeUnknownSection]
Foo=bar

[Link]
Name=eth0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.original_names, vec!["en*"]);
        assert_eq!(cfg.link_section.name.as_deref(), Some("eth0"));
    }

    #[test]
    fn test_parse_link_file_empty() {
        let cfg = parse_link_file_content("", Path::new("empty.link")).unwrap();
        assert!(cfg.match_section.original_names.is_empty());
        assert!(cfg.link_section.name.is_none());
    }

    #[test]
    fn test_parse_link_file_mtu_with_suffix() {
        let content = "\
[Link]
MTUBytes=9K
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.link_section.mtu, Some(9 * 1024));
    }

    #[test]
    fn test_parse_link_file_gso_max() {
        let content = "\
[Link]
GenericSegmentOffloadMaxBytes=65536
GenericSegmentOffloadMaxSegments=64
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.link_section.gso_max_bytes, Some(65536));
        assert_eq!(cfg.link_section.gso_max_segments, Some(64));
    }

    #[test]
    fn test_parse_link_file_required_family_for_online() {
        let content = "\
[Link]
RequiredFamilyForOnline=ipv4
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.required_family_for_online.as_deref(),
            Some("ipv4")
        );
    }

    #[test]
    fn test_parse_link_file_description() {
        let content = "\
[Link]
Description=My fancy link
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.description.as_deref(),
            Some("My fancy link")
        );
    }

    #[test]
    fn test_parse_link_file_kernel_version_credential() {
        let content = "\
[Match]
KernelVersion=6.1.*
Credential=my.cred

[Link]
Name=special0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(cfg.match_section.kernel_version.as_deref(), Some("6.1.*"));
        assert_eq!(cfg.match_section.credential.as_deref(), Some("my.cred"));
    }

    #[test]
    fn test_parse_link_file_kernel_command_line() {
        let content = "\
[Match]
KernelCommandLine=systemd.debug

[Link]
Name=debug0
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.match_section.kernel_command_line.as_deref(),
            Some("systemd.debug")
        );
    }

    #[test]
    fn test_parse_link_file_name_policy_unknown_filtered() {
        let content = "\
[Link]
NamePolicy=kernel bogus_policy path
";
        let cfg = parse_link_file_content(content, Path::new("test.link")).unwrap();
        assert_eq!(
            cfg.link_section.name_policy,
            vec![NamePolicy::Kernel, NamePolicy::Path]
        );
    }

    // ── Matching tests ─────────────────────────────────────────────────────

    #[test]
    fn test_link_match_empty_matches_all() {
        let section = LinkMatchSection::default();
        assert!(section.matches_interface("anything", None, None, None, None));
    }

    #[test]
    fn test_link_match_original_name_glob() {
        let section = LinkMatchSection {
            original_names: vec!["en*".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("enp3s0", None, None, None, None));
        assert!(!section.matches_interface("wlp2s0", None, None, None, None));
    }

    #[test]
    fn test_link_match_original_name_wildcard_all() {
        let section = LinkMatchSection {
            original_names: vec!["*".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("anything", None, None, None, None));
    }

    #[test]
    fn test_link_match_original_name_exact() {
        let section = LinkMatchSection {
            original_names: vec!["eth0".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("eth0", None, None, None, None));
        assert!(!section.matches_interface("eth1", None, None, None, None));
    }

    #[test]
    fn test_link_match_mac_address() {
        let section = LinkMatchSection {
            mac_addresses: vec!["00:11:22:33:44:55".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("eth0", Some("00:11:22:33:44:55"), None, None, None));
        assert!(section.matches_interface("eth0", Some("00:11:22:33:44:55"), None, None, None));
        assert!(!section.matches_interface("eth0", Some("aa:bb:cc:dd:ee:ff"), None, None, None));
        assert!(!section.matches_interface("eth0", None, None, None, None));
    }

    #[test]
    fn test_link_match_mac_address_case_insensitive() {
        let section = LinkMatchSection {
            mac_addresses: vec!["00:11:22:33:44:55".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("eth0", Some("00:11:22:33:44:55"), None, None, None));
        assert!(section.matches_interface("eth0", Some("00:11:22:33:44:55"), None, None, None));
    }

    #[test]
    fn test_link_match_driver() {
        let section = LinkMatchSection {
            drivers: vec!["virtio*".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("eth0", None, Some("virtio_net"), None, None));
        assert!(!section.matches_interface("eth0", None, Some("e1000"), None, None));
        assert!(!section.matches_interface("eth0", None, None, None, None));
    }

    #[test]
    fn test_link_match_type() {
        let section = LinkMatchSection {
            types: vec!["ether".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("eth0", None, None, Some("ether"), None));
        assert!(!section.matches_interface("eth0", None, None, Some("wlan"), None));
        assert!(!section.matches_interface("eth0", None, None, None, None));
    }

    #[test]
    fn test_link_match_path() {
        let section = LinkMatchSection {
            paths: vec!["pci-0000:02:00.0-*".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface(
            "eth0",
            None,
            None,
            None,
            Some("pci-0000:02:00.0-port0")
        ));
        assert!(!section.matches_interface("eth0", None, None, None, Some("usb-0:1")));
        assert!(!section.matches_interface("eth0", None, None, None, None));
    }

    #[test]
    fn test_link_match_multiple_criteria_all_must_match() {
        let section = LinkMatchSection {
            original_names: vec!["en*".to_string()],
            drivers: vec!["virtio*".to_string()],
            ..Default::default()
        };

        // Both match — OK.
        assert!(section.matches_interface("enp0s3", None, Some("virtio_net"), None, None));
        // Name matches, driver doesn't.
        assert!(!section.matches_interface("enp0s3", None, Some("e1000"), None, None));
        // Driver matches, name doesn't.
        assert!(!section.matches_interface("wlp2s0", None, Some("virtio_net"), None, None));
    }

    #[test]
    fn test_link_match_multiple_original_names() {
        let section = LinkMatchSection {
            original_names: vec!["en*".to_string(), "eth*".to_string()],
            ..Default::default()
        };
        assert!(section.matches_interface("enp3s0", None, None, None, None));
        assert!(section.matches_interface("eth0", None, None, None, None));
        assert!(!section.matches_interface("wlan0", None, None, None, None));
    }

    #[test]
    fn test_link_match_multiple_mac_addresses() {
        let section = LinkMatchSection {
            mac_addresses: vec![
                "00:11:22:33:44:55".to_string(),
                "aa:bb:cc:dd:ee:ff".to_string(),
            ],
            ..Default::default()
        };
        assert!(section.matches_interface("eth0", Some("00:11:22:33:44:55"), None, None, None));
        assert!(section.matches_interface("eth0", Some("aa:bb:cc:dd:ee:ff"), None, None, None));
        assert!(!section.matches_interface("eth0", Some("11:22:33:44:55:66"), None, None, None));
    }

    // ── NamePolicy tests ───────────────────────────────────────────────────

    #[test]
    fn test_name_policy_parse() {
        assert_eq!(NamePolicy::parse("kernel"), Some(NamePolicy::Kernel));
        assert_eq!(NamePolicy::parse("database"), Some(NamePolicy::Database));
        assert_eq!(NamePolicy::parse("onboard"), Some(NamePolicy::Onboard));
        assert_eq!(NamePolicy::parse("slot"), Some(NamePolicy::Slot));
        assert_eq!(NamePolicy::parse("path"), Some(NamePolicy::Path));
        assert_eq!(NamePolicy::parse("mac"), Some(NamePolicy::Mac));
        assert_eq!(NamePolicy::parse("keep"), Some(NamePolicy::Keep));
        assert_eq!(NamePolicy::parse("bogus"), None);
    }

    #[test]
    fn test_name_policy_parse_case_insensitive() {
        assert_eq!(NamePolicy::parse("Kernel"), Some(NamePolicy::Kernel));
        assert_eq!(NamePolicy::parse("DATABASE"), Some(NamePolicy::Database));
        assert_eq!(NamePolicy::parse("Path"), Some(NamePolicy::Path));
    }

    #[test]
    fn test_name_policy_as_str() {
        assert_eq!(NamePolicy::Kernel.as_str(), "kernel");
        assert_eq!(NamePolicy::Database.as_str(), "database");
        assert_eq!(NamePolicy::Onboard.as_str(), "onboard");
        assert_eq!(NamePolicy::Slot.as_str(), "slot");
        assert_eq!(NamePolicy::Path.as_str(), "path");
        assert_eq!(NamePolicy::Mac.as_str(), "mac");
        assert_eq!(NamePolicy::Keep.as_str(), "keep");
    }

    // ── MACAddressPolicy tests ─────────────────────────────────────────────

    #[test]
    fn test_mac_address_policy_parse() {
        assert_eq!(
            MACAddressPolicy::parse("persistent"),
            Some(MACAddressPolicy::Persistent)
        );
        assert_eq!(
            MACAddressPolicy::parse("random"),
            Some(MACAddressPolicy::Random)
        );
        assert_eq!(
            MACAddressPolicy::parse("none"),
            Some(MACAddressPolicy::None)
        );
        assert_eq!(MACAddressPolicy::parse("bogus"), None);
    }

    #[test]
    fn test_mac_address_policy_parse_case_insensitive() {
        assert_eq!(
            MACAddressPolicy::parse("PERSISTENT"),
            Some(MACAddressPolicy::Persistent)
        );
        assert_eq!(
            MACAddressPolicy::parse("Random"),
            Some(MACAddressPolicy::Random)
        );
        assert_eq!(
            MACAddressPolicy::parse("None"),
            Some(MACAddressPolicy::None)
        );
    }

    #[test]
    fn test_mac_address_policy_as_str() {
        assert_eq!(MACAddressPolicy::Persistent.as_str(), "persistent");
        assert_eq!(MACAddressPolicy::Random.as_str(), "random");
        assert_eq!(MACAddressPolicy::None.as_str(), "none");
    }

    // ── load_link_configs_from tests ───────────────────────────────────────

    #[test]
    fn test_load_link_configs_from_empty() {
        let dir = tempfile::tempdir().unwrap();
        let configs = load_link_configs_from(&[dir.path().to_path_buf()]);
        assert!(configs.is_empty());
    }

    #[test]
    fn test_load_link_configs_from_with_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("10-eth.link"),
            "[Match]\nOriginalName=en*\n\n[Link]\nName=eth0\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("20-wlan.link"),
            "[Match]\nOriginalName=wl*\n\n[Link]\nName=wlan0\n",
        )
        .unwrap();
        let configs = load_link_configs_from(&[dir.path().to_path_buf()]);
        assert_eq!(configs.len(), 2);
        assert_eq!(configs[0].link_section.name.as_deref(), Some("eth0"));
        assert_eq!(configs[1].link_section.name.as_deref(), Some("wlan0"));
    }

    #[test]
    fn test_load_link_configs_from_skips_non_link_files() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(
            dir.path().join("10-eth.link"),
            "[Match]\nOriginalName=en*\n\n[Link]\nName=eth0\n",
        )
        .unwrap();
        std::fs::write(
            dir.path().join("10-eth.network"),
            "[Match]\nName=eth0\n\n[Network]\nDHCP=yes\n",
        )
        .unwrap();
        std::fs::write(dir.path().join("README"), "not a config\n").unwrap();
        let configs = load_link_configs_from(&[dir.path().to_path_buf()]);
        assert_eq!(configs.len(), 1);
    }

    #[test]
    fn test_load_link_configs_from_dedup_across_dirs() {
        let dir1 = tempfile::tempdir().unwrap();
        let dir2 = tempfile::tempdir().unwrap();
        std::fs::write(dir1.path().join("10-test.link"), "[Link]\nName=first\n").unwrap();
        std::fs::write(dir2.path().join("10-test.link"), "[Link]\nName=second\n").unwrap();
        let configs =
            load_link_configs_from(&[dir1.path().to_path_buf(), dir2.path().to_path_buf()]);
        assert_eq!(configs.len(), 1);
        assert_eq!(configs[0].link_section.name.as_deref(), Some("first"));
    }

    #[test]
    fn test_load_link_configs_from_sorted_by_filename() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("90-last.link"), "[Link]\nName=last\n").unwrap();
        std::fs::write(dir.path().join("10-first.link"), "[Link]\nName=first\n").unwrap();
        std::fs::write(dir.path().join("50-middle.link"), "[Link]\nName=middle\n").unwrap();
        let configs = load_link_configs_from(&[dir.path().to_path_buf()]);
        assert_eq!(configs.len(), 3);
        assert_eq!(configs[0].link_section.name.as_deref(), Some("first"));
        assert_eq!(configs[1].link_section.name.as_deref(), Some("middle"));
        assert_eq!(configs[2].link_section.name.as_deref(), Some("last"));
    }

    #[test]
    fn test_load_link_configs_from_nonexistent_dir() {
        let configs = load_link_configs_from(&[PathBuf::from("/nonexistent/dir/for/test")]);
        assert!(configs.is_empty());
    }

    // ── find_matching_link_config tests ────────────────────────────────────

    #[test]
    fn test_find_matching_link_config_by_name() {
        let configs = vec![
            parse_link_file_content(
                "[Match]\nOriginalName=en*\n\n[Link]\nName=eth0\n",
                Path::new("10-eth.link"),
            )
            .unwrap(),
            parse_link_file_content(
                "[Match]\nOriginalName=wl*\n\n[Link]\nName=wlan0\n",
                Path::new("20-wlan.link"),
            )
            .unwrap(),
        ];

        let result = find_matching_link_config(&configs, "enp3s0", None, None, None, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().link_section.name.as_deref(), Some("eth0"));

        let result = find_matching_link_config(&configs, "wlp2s0", None, None, None, None);
        assert!(result.is_some());
        assert_eq!(result.unwrap().link_section.name.as_deref(), Some("wlan0"));

        let result = find_matching_link_config(&configs, "lo", None, None, None, None);
        assert!(result.is_none());
    }

    #[test]
    fn test_find_matching_link_config_first_match_wins() {
        let configs = vec![
            parse_link_file_content(
                "[Match]\nOriginalName=en*\n\n[Link]\nName=first\n",
                Path::new("10-first.link"),
            )
            .unwrap(),
            parse_link_file_content(
                "[Match]\nOriginalName=en*\n\n[Link]\nName=second\n",
                Path::new("20-second.link"),
            )
            .unwrap(),
        ];

        let result = find_matching_link_config(&configs, "enp3s0", None, None, None, None);
        assert_eq!(result.unwrap().link_section.name.as_deref(), Some("first"));
    }

    #[test]
    fn test_find_matching_link_config_with_mac() {
        let configs = vec![
            parse_link_file_content(
                "[Match]\nMACAddress=00:11:22:33:44:55\n\n[Link]\nName=specific\n",
                Path::new("10-mac.link"),
            )
            .unwrap(),
            parse_link_file_content(
                "[Match]\nOriginalName=*\n\n[Link]\nName=fallback\n",
                Path::new("99-default.link"),
            )
            .unwrap(),
        ];

        let result = find_matching_link_config(
            &configs,
            "enp3s0",
            Some("00:11:22:33:44:55"),
            None,
            None,
            None,
        );
        assert_eq!(
            result.unwrap().link_section.name.as_deref(),
            Some("specific")
        );

        let result = find_matching_link_config(
            &configs,
            "enp3s0",
            Some("aa:bb:cc:dd:ee:ff"),
            None,
            None,
            None,
        );
        assert_eq!(
            result.unwrap().link_section.name.as_deref(),
            Some("fallback")
        );
    }

    #[test]
    fn test_find_matching_link_config_no_match_section_matches_all() {
        let configs = vec![
            parse_link_file_content("[Link]\nName=catch-all\n", Path::new("99-default.link"))
                .unwrap(),
        ];

        let result = find_matching_link_config(&configs, "anything", None, None, None, None);
        assert_eq!(
            result.unwrap().link_section.name.as_deref(),
            Some("catch-all")
        );
    }

    #[test]
    fn test_find_matching_link_config_empty_list() {
        let configs: Vec<LinkFileConfig> = vec![];
        let result = find_matching_link_config(&configs, "eth0", None, None, None, None);
        assert!(result.is_none());
    }

    // ── resolve_name_from_policy tests ─────────────────────────────────────

    #[test]
    fn test_resolve_name_from_policy_path() {
        let cfg = parse_link_file_content(
            "[Link]\nNamePolicy=kernel database onboard slot path\n",
            Path::new("99-default.link"),
        )
        .unwrap();

        let name = resolve_name_from_policy(&cfg, |key| {
            if key == "ID_NET_NAME_PATH" {
                Some("enp3s0".to_string())
            } else {
                None
            }
        });
        assert_eq!(name.as_deref(), Some("enp3s0"));
    }

    #[test]
    fn test_resolve_name_from_policy_prefers_earlier_policy() {
        let cfg = parse_link_file_content(
            "[Link]\nNamePolicy=onboard slot path\n",
            Path::new("99-default.link"),
        )
        .unwrap();

        let name = resolve_name_from_policy(&cfg, |key| match key {
            "ID_NET_NAME_ONBOARD" => Some("eno1".to_string()),
            "ID_NET_NAME_SLOT" => Some("ens3".to_string()),
            "ID_NET_NAME_PATH" => Some("enp3s0".to_string()),
            _ => None,
        });
        assert_eq!(name.as_deref(), Some("eno1"));
    }

    #[test]
    fn test_resolve_name_from_policy_falls_back_to_name() {
        let cfg = parse_link_file_content(
            "[Link]\nNamePolicy=database\nName=fallback0\n",
            Path::new("10-fallback.link"),
        )
        .unwrap();

        let name = resolve_name_from_policy(&cfg, |_| None);
        assert_eq!(name.as_deref(), Some("fallback0"));
    }

    #[test]
    fn test_resolve_name_from_policy_kernel_skips() {
        // kernel policy means "keep the kernel name" — should skip and try next.
        let cfg = parse_link_file_content(
            "[Link]\nNamePolicy=kernel path\n",
            Path::new("99-default.link"),
        )
        .unwrap();

        let name = resolve_name_from_policy(&cfg, |key| {
            if key == "ID_NET_NAME_PATH" {
                Some("enp3s0".to_string())
            } else {
                None
            }
        });
        assert_eq!(name.as_deref(), Some("enp3s0"));
    }

    #[test]
    fn test_resolve_name_from_policy_keep_returns_none() {
        let cfg =
            parse_link_file_content("[Link]\nNamePolicy=keep\n", Path::new("99-default.link"))
                .unwrap();

        let name = resolve_name_from_policy(&cfg, |_| None);
        assert!(name.is_none());
    }

    #[test]
    fn test_resolve_name_from_policy_empty_env_value_skipped() {
        let cfg = parse_link_file_content(
            "[Link]\nNamePolicy=onboard path\nName=fallback\n",
            Path::new("test.link"),
        )
        .unwrap();

        let name = resolve_name_from_policy(&cfg, |key| match key {
            "ID_NET_NAME_ONBOARD" => Some(String::new()), // empty — skip
            "ID_NET_NAME_PATH" => Some("enp0s3".to_string()),
            _ => None,
        });
        assert_eq!(name.as_deref(), Some("enp0s3"));
    }

    #[test]
    fn test_resolve_name_from_policy_no_policy_no_name() {
        let cfg =
            parse_link_file_content("[Link]\nMTUBytes=1500\n", Path::new("10-mtu.link")).unwrap();

        let name = resolve_name_from_policy(&cfg, |_| None);
        assert!(name.is_none());
    }

    #[test]
    fn test_resolve_name_from_policy_no_policy_with_name() {
        let cfg =
            parse_link_file_content("[Link]\nName=myeth0\n", Path::new("10-custom.link")).unwrap();

        let name = resolve_name_from_policy(&cfg, |_| None);
        assert_eq!(name.as_deref(), Some("myeth0"));
    }

    // ── glob_match tests ───────────────────────────────────────────────────

    #[test]
    fn test_glob_match_exact() {
        assert!(glob_match("hello", "hello"));
        assert!(!glob_match("hello", "world"));
    }

    #[test]
    fn test_glob_match_star() {
        assert!(glob_match("en*", "enp3s0"));
        assert!(glob_match("en*", "en"));
        assert!(!glob_match("en*", "wlan0"));
        assert!(glob_match("*", "anything"));
        assert!(glob_match("*", ""));
    }

    #[test]
    fn test_glob_match_question() {
        assert!(glob_match("eth?", "eth0"));
        assert!(glob_match("eth?", "eth1"));
        assert!(!glob_match("eth?", "eth10"));
        assert!(!glob_match("eth?", "eth"));
    }

    #[test]
    fn test_glob_match_combined() {
        assert!(glob_match("e?p*", "enp3s0"));
        assert!(glob_match("*s0", "enp3s0"));
        assert!(!glob_match("*s1", "enp3s0"));
    }

    #[test]
    fn test_glob_match_empty() {
        assert!(glob_match("", ""));
        assert!(!glob_match("", "notempty"));
        assert!(glob_match("*", ""));
    }

    // ── parse_bool tests ───────────────────────────────────────────────────

    #[test]
    fn test_parse_bool_true_variants() {
        assert!(parse_bool("yes"));
        assert!(parse_bool("true"));
        assert!(parse_bool("1"));
        assert!(parse_bool("on"));
        assert!(parse_bool("Yes"));
        assert!(parse_bool("TRUE"));
        assert!(parse_bool("ON"));
    }

    #[test]
    fn test_parse_bool_false_variants() {
        assert!(!parse_bool("no"));
        assert!(!parse_bool("false"));
        assert!(!parse_bool("0"));
        assert!(!parse_bool("off"));
        assert!(!parse_bool("anything_else"));
    }

    // ── parse_bytes_value tests ────────────────────────────────────────────

    #[test]
    fn test_parse_bytes_value_plain() {
        assert_eq!(parse_bytes_value("1500"), Some(1500));
    }

    #[test]
    fn test_parse_bytes_value_k() {
        assert_eq!(parse_bytes_value("9K"), Some(9 * 1024));
        assert_eq!(parse_bytes_value("9KB"), Some(9 * 1024));
    }

    #[test]
    fn test_parse_bytes_value_m() {
        assert_eq!(parse_bytes_value("1M"), Some(1024 * 1024));
        assert_eq!(parse_bytes_value("1MB"), Some(1024 * 1024));
    }

    #[test]
    fn test_parse_bytes_value_g() {
        assert_eq!(parse_bytes_value("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_bytes_value("1GB"), Some(1024 * 1024 * 1024));
    }

    #[test]
    fn test_parse_bytes_value_invalid() {
        assert_eq!(parse_bytes_value("abc"), None);
        assert_eq!(parse_bytes_value(""), None);
    }

    // ── NixOS-style .link file ─────────────────────────────────────────────

    #[test]
    fn test_parse_nixos_style_link_file() {
        let content = "\
# NixOS default link configuration
[Match]
OriginalName=*

[Link]
NamePolicy=kernel database onboard slot path
AlternativeNamesPolicy=database onboard slot path mac
MACAddressPolicy=persistent
";
        let cfg = parse_link_file_content(content, Path::new("99-default.link")).unwrap();
        assert_eq!(cfg.match_section.original_names, vec!["*"]);
        assert_eq!(
            cfg.link_section.name_policy,
            vec![
                NamePolicy::Kernel,
                NamePolicy::Database,
                NamePolicy::Onboard,
                NamePolicy::Slot,
                NamePolicy::Path,
            ]
        );
        assert_eq!(
            cfg.link_section.mac_address_policy,
            Some(MACAddressPolicy::Persistent)
        );
    }
}
