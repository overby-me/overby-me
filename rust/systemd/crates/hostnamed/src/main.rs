//! systemd-hostnamed — hostname management daemon
//!
//! Manages three types of hostname:
//! - **Static hostname**: stored in `/etc/hostname`, persists across reboots
//! - **Pretty hostname**: human-readable, stored in `/etc/machine-info` as `PRETTY_HOSTNAME=`
//! - **Transient hostname**: the live kernel hostname, set via `sethostname(2)`
//!
//! Also manages machine metadata from `/etc/machine-info`:
//! - `CHASSIS=` — form factor (desktop, laptop, server, tablet, etc.)
//! - `DEPLOYMENT=` — deployment environment (development, staging, production, etc.)
//! - `LOCATION=` — physical location description
//! - `ICON_NAME=` — icon name for the machine
//! - `HARDWARE_VENDOR=` — hardware vendor name
//! - `HARDWARE_MODEL=` — hardware model name
//!
//! The daemon listens for SIGTERM/SIGINT to shut down and SIGHUP to reload
//! configuration. It sends sd_notify READY/STATUS/STOPPING messages.

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::net::Shutdown;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use zbus::blocking::Connection;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const HOSTNAME_PATH: &str = "/etc/hostname";
const MACHINE_INFO_PATH: &str = "/etc/machine-info";
const OS_RELEASE_PATH: &str = "/etc/os-release";
const OS_RELEASE_USR_PATH: &str = "/usr/lib/os-release";
const DMI_CHASSIS_TYPE_PATH: &str = "/sys/class/dmi/id/chassis_type";
const DMI_VENDOR_PATH: &str = "/sys/class/dmi/id/sys_vendor";
const DMI_MODEL_PATH: &str = "/sys/class/dmi/id/product_name";
const CONTROL_SOCKET_PATH: &str = "/run/systemd/hostnamed.sock";

/// Return the effective hostname file path, honoring `SYSTEMD_ETC_HOSTNAME`.
fn hostname_path() -> String {
    env::var("SYSTEMD_ETC_HOSTNAME").unwrap_or_else(|_| HOSTNAME_PATH.to_string())
}

/// Return the effective machine-info file path, honoring `SYSTEMD_ETC_MACHINE_INFO`.
fn machine_info_path() -> String {
    env::var("SYSTEMD_ETC_MACHINE_INFO").unwrap_or_else(|_| MACHINE_INFO_PATH.to_string())
}

const DBUS_NAME: &str = "org.freedesktop.hostname1";
const DBUS_PATH: &str = "/org/freedesktop/hostname1";

/// Known chassis type strings accepted by systemd-hostnamed.
const VALID_CHASSIS: &[&str] = &[
    "desktop",
    "laptop",
    "convertible",
    "server",
    "tablet",
    "handset",
    "watch",
    "embedded",
    "vm",
    "container",
];

// ---------------------------------------------------------------------------
// Data model
// ---------------------------------------------------------------------------

/// All hostname and machine-info state held by the daemon.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct HostnameState {
    /// Static hostname (from /etc/hostname).
    pub static_hostname: String,
    /// Pretty (human-readable) hostname (from PRETTY_HOSTNAME in /etc/machine-info).
    pub pretty_hostname: String,
    /// Transient hostname (live kernel hostname).
    pub transient_hostname: String,
    /// Chassis type.
    pub chassis: String,
    /// Deployment environment.
    pub deployment: String,
    /// Physical location.
    pub location: String,
    /// Icon name.
    pub icon_name: String,
    /// Hardware vendor (from DMI or machine-info).
    pub hardware_vendor: String,
    /// Hardware model (from DMI or machine-info).
    pub hardware_model: String,
    /// Operating system pretty name.
    pub os_pretty_name: String,
    /// Operating system CPE name.
    pub os_cpe_name: String,
    /// Operating system home URL.
    pub os_home_url: String,
    /// Kernel name (uname -s).
    pub kernel_name: String,
    /// Kernel release (uname -r).
    pub kernel_release: String,
}

// ---------------------------------------------------------------------------
// Reading state
// ---------------------------------------------------------------------------

impl HostnameState {
    /// Load all hostname state from the filesystem.
    pub fn load() -> Self {
        Self::load_from(
            &hostname_path(),
            &machine_info_path(),
            OS_RELEASE_PATH,
            OS_RELEASE_USR_PATH,
            DMI_CHASSIS_TYPE_PATH,
            DMI_VENDOR_PATH,
            DMI_MODEL_PATH,
        )
    }

    /// Load state from custom paths (for testing).
    pub fn load_from(
        hostname_path: &str,
        machine_info_path: &str,
        os_release_path: &str,
        os_release_usr_path: &str,
        dmi_chassis_path: &str,
        dmi_vendor_path: &str,
        dmi_model_path: &str,
    ) -> Self {
        let mut state = HostnameState {
            static_hostname: read_trimmed(hostname_path).unwrap_or_default(),
            transient_hostname: get_kernel_hostname(),
            ..Default::default()
        };

        // Machine-info
        let mi = parse_env_file(machine_info_path);
        state.pretty_hostname = mi.get("PRETTY_HOSTNAME").cloned().unwrap_or_default();
        state.chassis = mi
            .get("CHASSIS")
            .cloned()
            .unwrap_or_default()
            .to_lowercase();
        state.deployment = mi.get("DEPLOYMENT").cloned().unwrap_or_default();
        state.location = mi.get("LOCATION").cloned().unwrap_or_default();
        state.icon_name = mi.get("ICON_NAME").cloned().unwrap_or_default();

        // Hardware info: prefer machine-info, fall back to DMI
        state.hardware_vendor = mi
            .get("HARDWARE_VENDOR")
            .cloned()
            .or_else(|| read_trimmed(dmi_vendor_path))
            .unwrap_or_default();
        state.hardware_model = mi
            .get("HARDWARE_MODEL")
            .cloned()
            .or_else(|| read_trimmed(dmi_model_path))
            .unwrap_or_default();

        // If no chassis in machine-info, try to auto-detect from DMI
        if state.chassis.is_empty() {
            state.chassis = detect_chassis_from_dmi(dmi_chassis_path);
        }

        // OS release info
        let osr = if Path::new(os_release_path).exists() {
            parse_env_file(os_release_path)
        } else {
            parse_env_file(os_release_usr_path)
        };
        state.os_pretty_name = osr.get("PRETTY_NAME").cloned().unwrap_or_default();
        state.os_cpe_name = osr.get("CPE_NAME").cloned().unwrap_or_default();
        state.os_home_url = osr.get("HOME_URL").cloned().unwrap_or_default();

        // Kernel info
        let uname = get_uname();
        state.kernel_name = uname.0;
        state.kernel_release = uname.1;

        state
    }

    /// Return the "effective" hostname: transient if non-empty, else static,
    /// else "localhost".
    pub fn hostname(&self) -> &str {
        if !self.transient_hostname.is_empty() {
            &self.transient_hostname
        } else if !self.static_hostname.is_empty() {
            &self.static_hostname
        } else {
            "localhost"
        }
    }

    /// Return the effective icon name. If none is set, derive one from the
    /// chassis type.
    pub fn effective_icon_name(&self) -> String {
        if !self.icon_name.is_empty() {
            return self.icon_name.clone();
        }
        match self.chassis.as_str() {
            "laptop" | "convertible" => "computer-laptop".to_string(),
            "tablet" => "computer-tablet".to_string(),
            "handset" => "phone".to_string(),
            "watch" => "watch".to_string(),
            "server" => "computer-server".to_string(),
            "desktop" => "computer".to_string(),
            "vm" => "computer-vm".to_string(),
            "container" => "container".to_string(),
            _ => "computer".to_string(),
        }
    }

    /// Format the state as a key=value status block (one per line).
    pub fn format_status(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "   Static hostname: {}\n",
            if self.static_hostname.is_empty() {
                "(unset)"
            } else {
                &self.static_hostname
            }
        ));
        if !self.pretty_hostname.is_empty() {
            out.push_str(&format!("   Pretty hostname: {}\n", self.pretty_hostname));
        }
        if !self.transient_hostname.is_empty() && self.transient_hostname != self.static_hostname {
            out.push_str(&format!(
                "Transient hostname: {}\n",
                self.transient_hostname
            ));
        }
        out.push_str(&format!(
            "         Icon name: {}\n",
            self.effective_icon_name()
        ));
        if !self.chassis.is_empty() {
            out.push_str(&format!("           Chassis: {}\n", self.chassis));
        }
        if !self.deployment.is_empty() {
            out.push_str(&format!("        Deployment: {}\n", self.deployment));
        }
        if !self.location.is_empty() {
            out.push_str(&format!("          Location: {}\n", self.location));
        }
        if !self.os_pretty_name.is_empty() {
            out.push_str(&format!("  Operating System: {}\n", self.os_pretty_name));
        }
        if !self.os_cpe_name.is_empty() {
            out.push_str(&format!("CPE OS Name: {}\n", self.os_cpe_name));
        }
        if !self.os_home_url.is_empty() {
            out.push_str(&format!("  Home URL: {}\n", self.os_home_url));
        }
        if !self.kernel_name.is_empty() || !self.kernel_release.is_empty() {
            out.push_str(&format!(
                "            Kernel: {} {}\n",
                self.kernel_name, self.kernel_release
            ));
        }
        if !self.hardware_vendor.is_empty() {
            out.push_str(&format!("  Hardware Vendor: {}\n", self.hardware_vendor));
        }
        if !self.hardware_model.is_empty() {
            out.push_str(&format!("  Hardware Model: {}\n", self.hardware_model));
        }
        out
    }

    /// Format state as machine-readable `key=value` pairs (for `hostnamectl show`).
    pub fn format_show(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!("Hostname={}\n", self.hostname()));
        out.push_str(&format!("StaticHostname={}\n", self.static_hostname));
        out.push_str(&format!("PrettyHostname={}\n", self.pretty_hostname));
        out.push_str(&format!("TransientHostname={}\n", self.transient_hostname));
        out.push_str(&format!("IconName={}\n", self.effective_icon_name()));
        out.push_str(&format!("Chassis={}\n", self.chassis));
        out.push_str(&format!("Deployment={}\n", self.deployment));
        out.push_str(&format!("Location={}\n", self.location));
        out.push_str(&format!(
            "OperatingSystemPrettyName={}\n",
            self.os_pretty_name
        ));
        out.push_str(&format!("OperatingSystemCPEName={}\n", self.os_cpe_name));
        out.push_str(&format!("OperatingSystemHomeURL={}\n", self.os_home_url));
        out.push_str(&format!("KernelName={}\n", self.kernel_name));
        out.push_str(&format!("KernelRelease={}\n", self.kernel_release));
        out.push_str(&format!("HardwareVendor={}\n", self.hardware_vendor));
        out.push_str(&format!("HardwareModel={}\n", self.hardware_model));
        out
    }
}

// ---------------------------------------------------------------------------
// Writing state
// ---------------------------------------------------------------------------

/// Set the static hostname: write to /etc/hostname and optionally set the
/// kernel hostname too.
pub fn set_static_hostname(hostname: &str) -> io::Result<()> {
    set_static_hostname_at(hostname, &hostname_path())
}

pub fn set_static_hostname_at(hostname: &str, path: &str) -> io::Result<()> {
    let clean = hostname.trim();
    if clean.is_empty() {
        // Remove the file if the hostname is being unset
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    } else {
        let mut f = fs::File::create(path)?;
        writeln!(f, "{}", clean)?;
    }
    Ok(())
}

/// Set the transient (kernel) hostname via `sethostname(2)`.
pub fn set_transient_hostname(hostname: &str) -> io::Result<()> {
    nix::unistd::sethostname(hostname).map_err(|e| io::Error::other(e.to_string()))
}

/// Update a key in /etc/machine-info. If value is empty, the key is removed.
pub fn set_machine_info_key(key: &str, value: &str) -> io::Result<()> {
    set_machine_info_key_at(key, value, &machine_info_path())
}

pub fn set_machine_info_key_at(key: &str, value: &str, path: &str) -> io::Result<()> {
    let mut entries = parse_env_file(path);

    if value.is_empty() {
        entries.remove(key);
    } else {
        entries.insert(key.to_string(), value.to_string());
    }

    write_env_file(path, &entries)
}

/// Write all machine-info entries to a file in `KEY="value"` format.
fn write_env_file(path: &str, entries: &BTreeMap<String, String>) -> io::Result<()> {
    if entries.is_empty() {
        // Remove the file if there are no entries
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        return Ok(());
    }

    let mut f = fs::File::create(path)?;
    for (k, v) in entries {
        // Quote values that contain spaces or special characters
        if v.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\' || c == '$') {
            let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
            writeln!(f, "{}=\"{}\"", k, escaped)?;
        } else {
            writeln!(f, "{}={}", k, v)?;
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Read a file and return its trimmed contents, or None on error.
fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

/// Parse an environment-style file (`KEY=VALUE` or `KEY="VALUE"` per line).
fn parse_env_file(path: &str) -> BTreeMap<String, String> {
    parse_env_file_content(&fs::read_to_string(path).unwrap_or_default())
}

/// Parse environment file content.
fn parse_env_file_content(content: &str) -> BTreeMap<String, String> {
    let mut map = BTreeMap::new();
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let mut value = value.trim().to_string();
            // Strip surrounding quotes
            if (value.starts_with('"') && value.ends_with('"'))
                || (value.starts_with('\'') && value.ends_with('\''))
            {
                value = value[1..value.len() - 1].to_string();
            }
            // Unescape common escape sequences
            value = value.replace("\\\"", "\"").replace("\\\\", "\\");
            if !key.is_empty() {
                map.insert(key.to_string(), value);
            }
        }
    }
    map
}

/// Get the kernel hostname via `gethostname(2)`.
fn get_kernel_hostname() -> String {
    nix::unistd::gethostname()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_default()
}

/// Get uname sysname and release.
fn get_uname() -> (String, String) {
    let info = nix::sys::utsname::uname();
    match info {
        Ok(u) => (
            u.sysname().to_string_lossy().to_string(),
            u.release().to_string_lossy().to_string(),
        ),
        Err(_) => ("Linux".to_string(), String::new()),
    }
}

/// Detect chassis type from DMI chassis_type value.
/// See SMBIOS spec for chassis type numbers.
fn detect_chassis_from_dmi(dmi_chassis_path: &str) -> String {
    let raw = match read_trimmed(dmi_chassis_path) {
        Some(s) => s,
        None => return String::new(),
    };
    let chassis_type: u32 = match raw.parse() {
        Ok(v) => v,
        Err(_) => return String::new(),
    };
    match chassis_type {
        1 | 2 => "desktop".to_string(), // Other, Unknown -> default to desktop
        3 | 4 | 5 | 6 | 7 | 24 => "desktop".to_string(), // Desktop, LP Desktop, Pizza Box, Mini Tower, Tower, Sealed-Case PC
        8..=10 => "laptop".to_string(),                  // Portable, Laptop, Notebook
        11 => "handset".to_string(),                     // Hand Held
        12 => "laptop".to_string(), // Docking Station -> associated with laptops
        13 => "desktop".to_string(), // All In One
        14 => "laptop".to_string(), // Sub Notebook
        15 | 16 => "desktop".to_string(), // Space-saving, Lunch Box
        17 => "server".to_string(), // Main Server Chassis
        23 => "server".to_string(), // Rack Mount Chassis
        25 => "server".to_string(), // Blade
        28 => "server".to_string(), // Blade Enclosure
        30 => "tablet".to_string(), // Tablet
        31 => "convertible".to_string(), // Convertible
        32 => "laptop".to_string(), // Detachable
        _ => String::new(),
    }
}

/// Validate a chassis string.
fn is_valid_chassis(chassis: &str) -> bool {
    chassis.is_empty() || VALID_CHASSIS.contains(&chassis)
}

/// Validate a hostname string. Must be non-empty, <= 64 chars, contain only
/// alphanumerics, hyphens, and dots, and not start/end with a hyphen or dot.
fn is_valid_hostname(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    if name.starts_with('.') || name.starts_with('-') {
        return false;
    }
    if name.ends_with('.') || name.ends_with('-') {
        return false;
    }
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.')
}

// ---------------------------------------------------------------------------
// Shared state for D-Bus
// ---------------------------------------------------------------------------

type SharedState = Arc<Mutex<HostnameState>>;

// ---------------------------------------------------------------------------
// D-Bus interface: org.freedesktop.hostname1
// ---------------------------------------------------------------------------

/// D-Bus interface struct for org.freedesktop.hostname1.
///
/// Properties (all read-only, matching upstream systemd):
///   Hostname, StaticHostname, PrettyHostname, IconName, Chassis,
///   Deployment, Location, KernelName, KernelRelease,
///   OperatingSystemPrettyName, OperatingSystemCPEName,
///   OperatingSystemHomeURL, HardwareVendor, HardwareModel,
///   HostnameSource
///
/// Methods:
///   SetHostname(s hostname, b interactive) → sets transient hostname
///   SetStaticHostname(s hostname, b interactive) → sets static hostname
///   SetPrettyHostname(s hostname, b interactive)
///   SetIconName(s icon, b interactive)
///   SetChassis(s chassis, b interactive)
///   SetDeployment(s deployment, b interactive)
///   SetLocation(s location, b interactive)
///   GetProductUUID(b interactive) → ay
///   Describe() → s (JSON description)
struct Hostname1Manager {
    state: SharedState,
}

#[zbus::interface(name = "org.freedesktop.hostname1")]
impl Hostname1Manager {
    // --- Properties (read-only) ---

    #[zbus(property, name = "Hostname")]
    fn hostname(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.hostname().to_string()
    }

    #[zbus(property, name = "StaticHostname")]
    fn static_hostname(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.static_hostname.clone()
    }

    #[zbus(property, name = "PrettyHostname")]
    fn pretty_hostname(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.pretty_hostname.clone()
    }

    #[zbus(property, name = "IconName")]
    fn icon_name(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.effective_icon_name()
    }

    #[zbus(property, name = "Chassis")]
    fn chassis(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.chassis.clone()
    }

    #[zbus(property, name = "Deployment")]
    fn deployment(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.deployment.clone()
    }

    #[zbus(property, name = "Location")]
    fn location(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.location.clone()
    }

    #[zbus(property, name = "KernelName")]
    fn kernel_name(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.kernel_name.clone()
    }

    #[zbus(property, name = "KernelRelease")]
    fn kernel_release(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.kernel_release.clone()
    }

    #[zbus(property, name = "OperatingSystemPrettyName")]
    fn operating_system_pretty_name(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.os_pretty_name.clone()
    }

    #[zbus(property, name = "OperatingSystemCPEName")]
    fn operating_system_cpe_name(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.os_cpe_name.clone()
    }

    #[zbus(property, name = "OperatingSystemHomeURL")]
    fn operating_system_home_url(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.os_home_url.clone()
    }

    #[zbus(property, name = "HardwareVendor")]
    fn hardware_vendor(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.hardware_vendor.clone()
    }

    #[zbus(property, name = "HardwareModel")]
    fn hardware_model(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.hardware_model.clone()
    }

    #[zbus(property, name = "HostnameSource")]
    fn hostname_source(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let source = if !s.static_hostname.is_empty() {
            "static"
        } else if !s.transient_hostname.is_empty() {
            "transient"
        } else {
            "default"
        };
        source.to_string()
    }

    // --- Methods ---

    fn set_hostname(&self, hostname: String, _interactive: bool) -> zbus::fdo::Result<()> {
        if !hostname.is_empty() && !is_valid_hostname(&hostname) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Invalid hostname '{}'",
                hostname
            )));
        }
        if let Err(e) = set_transient_hostname(&hostname) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set transient hostname: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.transient_hostname = if hostname.is_empty() {
            get_kernel_hostname()
        } else {
            hostname
        };
        Ok(())
    }

    fn set_static_hostname(&self, hostname: String, _interactive: bool) -> zbus::fdo::Result<()> {
        if !hostname.is_empty() && !is_valid_hostname(&hostname) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Invalid hostname '{}'",
                hostname
            )));
        }
        if let Err(e) = set_static_hostname(&hostname) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set static hostname: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.static_hostname = hostname;
        Ok(())
    }

    fn set_pretty_hostname(&self, hostname: String, _interactive: bool) -> zbus::fdo::Result<()> {
        if let Err(e) = set_machine_info_key("PRETTY_HOSTNAME", &hostname) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set pretty hostname: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.pretty_hostname = hostname;
        Ok(())
    }

    fn set_icon_name(&self, icon: String, _interactive: bool) -> zbus::fdo::Result<()> {
        if let Err(e) = set_machine_info_key("ICON_NAME", &icon) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set icon name: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.icon_name = icon;
        Ok(())
    }

    fn set_chassis(&self, chassis: String, _interactive: bool) -> zbus::fdo::Result<()> {
        let chassis = chassis.to_lowercase();
        if !is_valid_chassis(&chassis) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Invalid chassis '{}'. Valid values: {}",
                chassis,
                VALID_CHASSIS.join(", ")
            )));
        }
        if let Err(e) = set_machine_info_key("CHASSIS", &chassis) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set chassis: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.chassis = chassis;
        Ok(())
    }

    fn set_deployment(&self, deployment: String, _interactive: bool) -> zbus::fdo::Result<()> {
        if let Err(e) = set_machine_info_key("DEPLOYMENT", &deployment) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set deployment: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.deployment = deployment;
        Ok(())
    }

    fn set_location(&self, location: String, _interactive: bool) -> zbus::fdo::Result<()> {
        if let Err(e) = set_machine_info_key("LOCATION", &location) {
            return Err(zbus::fdo::Error::Failed(format!(
                "Failed to set location: {}",
                e
            )));
        }
        let mut s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        s.location = location;
        Ok(())
    }

    fn get_product_uuid(&self, _interactive: bool) -> Vec<u8> {
        match fs::read_to_string("/sys/class/dmi/id/product_uuid") {
            Ok(s) => {
                let hex: String = s.trim().replace('-', "");
                let mut bytes = Vec::with_capacity(16);
                let mut i = 0;
                while i + 1 < hex.len() && bytes.len() < 16 {
                    if let Ok(b) = u8::from_str_radix(&hex[i..i + 2], 16) {
                        bytes.push(b);
                    }
                    i += 2;
                }
                bytes
            }
            Err(_) => vec![0u8; 16],
        }
    }

    fn describe(&self) -> String {
        let s = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let hostname_source = if !s.static_hostname.is_empty() {
            "static"
        } else if !s.transient_hostname.is_empty() {
            "transient"
        } else {
            "default"
        };
        format!(
            concat!(
                "{{",
                "\"Hostname\":\"{}\",",
                "\"StaticHostname\":\"{}\",",
                "\"PrettyHostname\":\"{}\",",
                "\"IconName\":\"{}\",",
                "\"Chassis\":\"{}\",",
                "\"Deployment\":\"{}\",",
                "\"Location\":\"{}\",",
                "\"KernelName\":\"{}\",",
                "\"KernelRelease\":\"{}\",",
                "\"OperatingSystemPrettyName\":\"{}\",",
                "\"OperatingSystemCPEName\":\"{}\",",
                "\"OperatingSystemHomeURL\":\"{}\",",
                "\"HardwareVendor\":\"{}\",",
                "\"HardwareModel\":\"{}\",",
                "\"HostnameSource\":\"{}\"",
                "}}"
            ),
            json_escape(s.hostname()),
            json_escape(&s.static_hostname),
            json_escape(&s.pretty_hostname),
            json_escape(&s.effective_icon_name()),
            json_escape(&s.chassis),
            json_escape(&s.deployment),
            json_escape(&s.location),
            json_escape(&s.kernel_name),
            json_escape(&s.kernel_release),
            json_escape(&s.os_pretty_name),
            json_escape(&s.os_cpe_name),
            json_escape(&s.os_home_url),
            json_escape(&s.hardware_vendor),
            json_escape(&s.hardware_model),
            hostname_source,
        )
    }
}

/// Escape a string for embedding in a JSON value.
fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c < '\x20' => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

/// Set up the D-Bus connection and register the hostname1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedState) -> Result<Connection, String> {
    let iface = Hostname1Manager { state: shared };
    let conn = zbus::blocking::connection::Builder::system()
        .map_err(|e| format!("D-Bus builder failed: {}", e))?
        .name(DBUS_NAME)
        .map_err(|e| format!("D-Bus name request failed: {}", e))?
        .serve_at(DBUS_PATH, iface)
        .map_err(|e| format!("D-Bus serve_at failed: {}", e))?
        .build()
        .map_err(|e| format!("D-Bus connection failed: {}", e))?;
    Ok(conn)
}

// ---------------------------------------------------------------------------
// Control socket protocol
// ---------------------------------------------------------------------------

/// Handle a single command received on the control socket.
/// Commands are simple line-based: `COMMAND [ARGS...]`
fn handle_control_command(line: &str) -> String {
    let parts: Vec<&str> = line.trim().splitn(3, ' ').collect();
    let cmd = parts.first().copied().unwrap_or("");

    match cmd {
        "STATUS" | "status" => {
            let state = HostnameState::load();
            state.format_status()
        }
        "SHOW" | "show" => {
            let state = HostnameState::load();
            state.format_show()
        }
        "SET-HOSTNAME" | "set-hostname" => {
            let hostname = parts.get(1).unwrap_or(&"");
            if hostname.is_empty() {
                // Unset static hostname
                if let Err(e) = set_static_hostname("") {
                    return format!("ERROR: {}\n", e);
                }
            } else if !is_valid_hostname(hostname) {
                return format!("ERROR: Invalid hostname '{}'\n", hostname);
            } else {
                if let Err(e) = set_static_hostname(hostname) {
                    return format!("ERROR: {}\n", e);
                }
                if let Err(e) = set_transient_hostname(hostname) {
                    return format!("ERROR: {}\n", e);
                }
            }
            "OK\n".to_string()
        }
        "SET-STATIC-HOSTNAME" | "set-static-hostname" => {
            let hostname = parts.get(1).unwrap_or(&"");
            if !hostname.is_empty() && !is_valid_hostname(hostname) {
                return format!("ERROR: Invalid hostname '{}'\n", hostname);
            }
            match set_static_hostname(hostname) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-TRANSIENT-HOSTNAME" | "set-transient-hostname" => {
            let hostname = parts.get(1).unwrap_or(&"");
            if !hostname.is_empty() && !is_valid_hostname(hostname) {
                return format!("ERROR: Invalid hostname '{}'\n", hostname);
            }
            match set_transient_hostname(hostname) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-PRETTY-HOSTNAME" | "set-pretty-hostname" => {
            // Pretty hostname can contain spaces, so use everything after the command
            let pretty = if parts.len() >= 2 {
                line.trim().split_once(' ').map(|x| x.1).unwrap_or("")
            } else {
                ""
            };
            match set_machine_info_key("PRETTY_HOSTNAME", pretty) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-CHASSIS" | "set-chassis" => {
            let chassis = parts.get(1).unwrap_or(&"").to_lowercase();
            if !is_valid_chassis(&chassis) {
                return format!(
                    "ERROR: Invalid chassis '{}'. Valid values: {}\n",
                    chassis,
                    VALID_CHASSIS.join(", ")
                );
            }
            match set_machine_info_key("CHASSIS", &chassis) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-DEPLOYMENT" | "set-deployment" => {
            let deployment = parts.get(1).unwrap_or(&"");
            match set_machine_info_key("DEPLOYMENT", deployment) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-LOCATION" | "set-location" => {
            let location = if parts.len() >= 2 {
                line.trim().split_once(' ').map(|x| x.1).unwrap_or("")
            } else {
                ""
            };
            match set_machine_info_key("LOCATION", location) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-ICON-NAME" | "set-icon-name" => {
            let icon = parts.get(1).unwrap_or(&"");
            match set_machine_info_key("ICON_NAME", icon) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        "SET-MACHINE-INFO" | "set-machine-info" => {
            // SET-MACHINE-INFO KEY VALUE
            let key = parts.get(1).unwrap_or(&"");
            let value = if parts.len() >= 3 {
                // Everything after "SET-MACHINE-INFO KEY " is the value
                let prefix = format!("{} {} ", cmd, key);
                let trimmed = line.trim();
                if trimmed.len() > prefix.len() {
                    &trimmed[prefix.len()..]
                } else {
                    ""
                }
            } else {
                ""
            };
            if key.is_empty() {
                return "ERROR: Key argument required\n".to_string();
            }
            match set_machine_info_key(key, value) {
                Ok(()) => "OK\n".to_string(),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }
        _ => format!("ERROR: Unknown command '{}'\n", cmd),
    }
}

/// Handle a client connection on the control socket.
fn handle_client(stream: &mut UnixStream) {
    use std::io::{BufRead, BufReader};

    let reader = BufReader::new(stream.try_clone().unwrap_or_else(|_| {
        // Can't clone; just return — client will get EOF
        stream.try_clone().expect("stream clone failed twice")
    }));

    for line in reader.lines() {
        match line {
            Ok(l) if l.trim().is_empty() => continue,
            Ok(l) => {
                let response = handle_control_command(&l);
                let _ = stream.write_all(response.as_bytes());
                let _ = stream.flush();
            }
            Err(_) => break,
        }
    }
}

// ---------------------------------------------------------------------------
// sd_notify helper
// ---------------------------------------------------------------------------

fn sd_notify(msg: &str) {
    let sock_path = match env::var("NOTIFY_SOCKET") {
        Ok(p) => p,
        Err(_) => return,
    };

    let path = if let Some(stripped) = sock_path.strip_prefix('@') {
        // Abstract socket — use nul byte prefix
        format!("\0{}", stripped)
    } else {
        sock_path
    };

    let sock = match std::os::unix::net::UnixDatagram::unbound() {
        Ok(s) => s,
        Err(_) => return,
    };

    let _ = sock.send_to(msg.as_bytes(), &path);
}

// ---------------------------------------------------------------------------
// Signal handling
// ---------------------------------------------------------------------------

static SHUTDOWN: AtomicBool = AtomicBool::new(false);
static RELOAD: AtomicBool = AtomicBool::new(false);

extern "C" fn handle_sigterm(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sigint(_: libc::c_int) {
    SHUTDOWN.store(true, Ordering::SeqCst);
}

extern "C" fn handle_sighup(_: libc::c_int) {
    RELOAD.store(true, Ordering::SeqCst);
}

fn setup_signal_handlers() {
    unsafe {
        libc::signal(libc::SIGTERM, handle_sigterm as libc::sighandler_t);
        libc::signal(libc::SIGINT, handle_sigint as libc::sighandler_t);
        libc::signal(libc::SIGHUP, handle_sighup as libc::sighandler_t);
        libc::signal(libc::SIGPIPE, libc::SIG_IGN);
    }
}

// ---------------------------------------------------------------------------
// Logging
// ---------------------------------------------------------------------------

fn init_logging() {
    struct StderrLogger;

    impl log::Log for StderrLogger {
        fn enabled(&self, _metadata: &log::Metadata) -> bool {
            true
        }

        fn log(&self, record: &log::Record) {
            if self.enabled(record.metadata()) {
                let now = chrono_lite_timestamp();
                eprintln!(
                    "[{}][systemd-hostnamed][{}] {}",
                    now,
                    record.level(),
                    record.args()
                );
            }
        }

        fn flush(&self) {}
    }

    static LOGGER: StderrLogger = StderrLogger;
    let _ = log::set_logger(&LOGGER);
    log::set_max_level(log::LevelFilter::Info);
}

/// Minimal timestamp without pulling in chrono.
fn chrono_lite_timestamp() -> String {
    // Use libc to get wall-clock time
    let mut tv: libc::timeval = unsafe { std::mem::zeroed() };
    unsafe {
        libc::gettimeofday(&mut tv, std::ptr::null_mut());
    }

    let mut tm: libc::tm = unsafe { std::mem::zeroed() };
    unsafe {
        libc::localtime_r(&tv.tv_sec, &mut tm);
    }

    format!("{:02}:{:02}:{:02}", tm.tm_hour, tm.tm_min, tm.tm_sec)
}

// ---------------------------------------------------------------------------
// main
// ---------------------------------------------------------------------------

/// Parse `WATCHDOG_USEC` from the environment and return the interval at which
/// we should send `WATCHDOG=1` (half the watchdog timeout). Returns `None` if
/// the variable is absent or unparsable.
fn watchdog_interval() -> Option<Duration> {
    let usec: u64 = env::var("WATCHDOG_USEC").ok()?.parse().ok()?;
    if usec == 0 {
        return None;
    }
    // Notify at half the watchdog period so we have margin
    Some(Duration::from_micros(usec / 2))
}

fn main() {
    init_logging();
    setup_signal_handlers();

    log::info!("systemd-hostnamed starting");

    // Load initial state into shared state for D-Bus and control socket
    let initial_state = HostnameState::load();
    log::info!("Hostname: {}", initial_state.hostname());
    let shared_state: SharedState = Arc::new(Mutex::new(initial_state.clone()));

    // Watchdog support — send WATCHDOG=1 at half the configured interval
    let wd_interval = watchdog_interval();
    if let Some(ref iv) = wd_interval {
        log::info!("Watchdog enabled, interval {:?}", iv);
    }
    let mut last_watchdog = Instant::now();

    // D-Bus connection is deferred to after READY=1 so we don't block
    // early boot waiting for dbus-daemon.  zbus dispatches messages
    // automatically in a background thread — we just keep the connection alive.
    let mut _dbus_conn: Option<Connection> = None;
    let mut dbus_attempted = false;

    // Ensure /run/systemd exists
    let _ = fs::create_dir_all(Path::new(CONTROL_SOCKET_PATH).parent().unwrap());

    // Remove stale socket
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);

    // Bind control socket
    let listener = match UnixListener::bind(CONTROL_SOCKET_PATH) {
        Ok(l) => {
            log::info!("Listening on {}", CONTROL_SOCKET_PATH);
            Some(l)
        }
        Err(e) => {
            log::error!(
                "Failed to bind control socket {}: {}",
                CONTROL_SOCKET_PATH,
                e
            );
            None
        }
    };

    // Set socket to non-blocking so we can check SHUTDOWN flag periodically
    if let Some(ref l) = listener {
        l.set_nonblocking(true).expect("Failed to set non-blocking");
    }

    let hostname_display = initial_state.hostname().to_string();
    sd_notify(&format!("READY=1\nSTATUS=Hostname: {}", hostname_display));

    log::info!("systemd-hostnamed ready");

    // Main loop
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            log::info!("Received shutdown signal");
            break;
        }

        if RELOAD.load(Ordering::SeqCst) {
            RELOAD.store(false, Ordering::SeqCst);
            let new_state = HostnameState::load();
            log::info!("Reloaded configuration, hostname: {}", new_state.hostname());
            {
                let mut s = shared_state.lock().unwrap_or_else(|e| e.into_inner());
                *s = new_state.clone();
            }
            sd_notify(&format!("STATUS=Hostname: {}", new_state.hostname()));
        }

        // Send watchdog keepalive
        if let Some(ref iv) = wd_interval
            && last_watchdog.elapsed() >= *iv
        {
            sd_notify("WATCHDOG=1");
            last_watchdog = Instant::now();
        }

        // Attempt D-Bus registration once (deferred from startup so we don't
        // block early boot before dbus-daemon is running).
        // zbus handles message dispatch in a background thread automatically.
        if !dbus_attempted {
            dbus_attempted = true;
            match setup_dbus(shared_state.clone()) {
                Ok(conn) => {
                    log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                    _dbus_conn = Some(conn);
                    sd_notify(&format!(
                        "STATUS=Hostname: {} (D-Bus active)",
                        hostname_display
                    ));
                }
                Err(e) => {
                    log::warn!(
                        "Failed to register D-Bus interface ({}); control socket only",
                        e
                    );
                }
            }
        }

        // Accept control socket connections
        if let Some(ref listener) = listener {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    handle_client(&mut stream);
                    let _ = stream.shutdown(Shutdown::Both);
                }
                Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => {
                    // No connection waiting
                }
                Err(e) => {
                    log::warn!("Accept error: {}", e);
                }
            }
        }

        // Brief sleep to avoid busy-looping when there's no work
        thread::sleep(Duration::from_millis(50));
    }

    // Cleanup
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);
    sd_notify("STOPPING=1");
    log::info!("systemd-hostnamed stopped");
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn write_file(dir: &TempDir, name: &str, content: &str) -> PathBuf {
        let path = dir.path().join(name);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(content.as_bytes()).unwrap();
        path
    }

    // -- parse_env_file_content tests --

    #[test]
    fn test_parse_env_file_empty() {
        let result = parse_env_file_content("");
        assert!(result.is_empty());
    }

    #[test]
    fn test_parse_env_file_basic() {
        let content = "KEY1=value1\nKEY2=value2\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("KEY1").unwrap(), "value1");
        assert_eq!(result.get("KEY2").unwrap(), "value2");
    }

    #[test]
    fn test_parse_env_file_quoted() {
        let content = "PRETTY_HOSTNAME=\"My Pretty Host\"\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("PRETTY_HOSTNAME").unwrap(), "My Pretty Host");
    }

    #[test]
    fn test_parse_env_file_single_quoted() {
        let content = "CHASSIS='laptop'\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("CHASSIS").unwrap(), "laptop");
    }

    #[test]
    fn test_parse_env_file_comments_and_blanks() {
        let content = "# Comment\n\nKEY=val\n  # another comment\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.len(), 1);
        assert_eq!(result.get("KEY").unwrap(), "val");
    }

    #[test]
    fn test_parse_env_file_escaped_quote() {
        let content = "NAME=\"value with \\\"quotes\\\"\"\n";
        let result = parse_env_file_content(content);
        assert_eq!(result.get("NAME").unwrap(), "value with \"quotes\"");
    }

    // -- hostname validation --

    #[test]
    fn test_valid_hostname() {
        assert!(is_valid_hostname("myhost"));
        assert!(is_valid_hostname("my-host"));
        assert!(is_valid_hostname("my.host.example"));
        assert!(is_valid_hostname("HOST123"));
        assert!(is_valid_hostname("a"));
    }

    #[test]
    fn test_invalid_hostname_empty() {
        assert!(!is_valid_hostname(""));
    }

    #[test]
    fn test_invalid_hostname_too_long() {
        let long = "a".repeat(65);
        assert!(!is_valid_hostname(&long));
    }

    #[test]
    fn test_invalid_hostname_starts_with_hyphen() {
        assert!(!is_valid_hostname("-host"));
    }

    #[test]
    fn test_invalid_hostname_ends_with_hyphen() {
        assert!(!is_valid_hostname("host-"));
    }

    #[test]
    fn test_invalid_hostname_starts_with_dot() {
        assert!(!is_valid_hostname(".host"));
    }

    #[test]
    fn test_invalid_hostname_special_chars() {
        assert!(!is_valid_hostname("host name"));
        assert!(!is_valid_hostname("host_name"));
        assert!(!is_valid_hostname("host@name"));
    }

    // -- chassis validation --

    #[test]
    fn test_valid_chassis() {
        for ch in VALID_CHASSIS {
            assert!(is_valid_chassis(ch));
        }
        assert!(is_valid_chassis("")); // empty is valid (unset)
    }

    #[test]
    fn test_invalid_chassis() {
        assert!(!is_valid_chassis("spaceship"));
        assert!(!is_valid_chassis("DESKTOP")); // case-sensitive
    }

    // -- chassis detection --

    #[test]
    fn test_detect_chassis_desktop() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "chassis_type", "3\n");
        assert_eq!(detect_chassis_from_dmi(path.to_str().unwrap()), "desktop");
    }

    #[test]
    fn test_detect_chassis_laptop() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "chassis_type", "9\n");
        assert_eq!(detect_chassis_from_dmi(path.to_str().unwrap()), "laptop");
    }

    #[test]
    fn test_detect_chassis_server() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "chassis_type", "17\n");
        assert_eq!(detect_chassis_from_dmi(path.to_str().unwrap()), "server");
    }

    #[test]
    fn test_detect_chassis_tablet() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "chassis_type", "30\n");
        assert_eq!(detect_chassis_from_dmi(path.to_str().unwrap()), "tablet");
    }

    #[test]
    fn test_detect_chassis_convertible() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "chassis_type", "31\n");
        assert_eq!(
            detect_chassis_from_dmi(path.to_str().unwrap()),
            "convertible"
        );
    }

    #[test]
    fn test_detect_chassis_unknown_type() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "chassis_type", "99\n");
        assert_eq!(detect_chassis_from_dmi(path.to_str().unwrap()), "");
    }

    #[test]
    fn test_detect_chassis_nonexistent() {
        assert_eq!(detect_chassis_from_dmi("/nonexistent/chassis_type"), "");
    }

    #[test]
    fn test_detect_chassis_non_numeric() {
        let dir = TempDir::new().unwrap();
        let path = write_file(&dir, "chassis_type", "invalid\n");
        assert_eq!(detect_chassis_from_dmi(path.to_str().unwrap()), "");
    }

    // -- HostnameState tests --

    #[test]
    fn test_load_state_from_files() {
        let dir = TempDir::new().unwrap();
        let hostname_path = write_file(&dir, "hostname", "myhost\n");
        let machine_info_path = write_file(
            &dir,
            "machine-info",
            "PRETTY_HOSTNAME=\"My Pretty Host\"\nCHASSIS=laptop\nDEPLOYMENT=production\nLOCATION=\"Rack A3\"\nICON_NAME=computer-laptop\n",
        );
        let os_release_path = write_file(
            &dir,
            "os-release",
            "PRETTY_NAME=\"NixOS 25.11\"\nHOME_URL=https://nixos.org\nCPE_NAME=cpe:/o:nixos:nixos:25.11\n",
        );
        let dmi_chassis = write_file(&dir, "chassis_type", "9\n");
        let dmi_vendor = write_file(&dir, "sys_vendor", "QEMU\n");
        let dmi_model = write_file(&dir, "product_name", "Standard PC\n");

        let state = HostnameState::load_from(
            hostname_path.to_str().unwrap(),
            machine_info_path.to_str().unwrap(),
            os_release_path.to_str().unwrap(),
            "/nonexistent/usr-os-release",
            dmi_chassis.to_str().unwrap(),
            dmi_vendor.to_str().unwrap(),
            dmi_model.to_str().unwrap(),
        );

        assert_eq!(state.static_hostname, "myhost");
        assert_eq!(state.pretty_hostname, "My Pretty Host");
        assert_eq!(state.chassis, "laptop");
        assert_eq!(state.deployment, "production");
        assert_eq!(state.location, "Rack A3");
        assert_eq!(state.icon_name, "computer-laptop");
        assert_eq!(state.os_pretty_name, "NixOS 25.11");
        assert_eq!(state.os_cpe_name, "cpe:/o:nixos:nixos:25.11");
        assert_eq!(state.os_home_url, "https://nixos.org");
    }

    #[test]
    fn test_load_state_missing_files() {
        let state = HostnameState::load_from(
            "/nonexistent/hostname",
            "/nonexistent/machine-info",
            "/nonexistent/os-release",
            "/nonexistent/usr-os-release",
            "/nonexistent/chassis_type",
            "/nonexistent/sys_vendor",
            "/nonexistent/product_name",
        );
        assert_eq!(state.static_hostname, "");
        assert_eq!(state.pretty_hostname, "");
        assert_eq!(state.chassis, "");
    }

    #[test]
    fn test_hostname_effective() {
        let mut state = HostnameState::default();
        assert_eq!(state.hostname(), "localhost");

        state.static_hostname = "static-host".to_string();
        assert_eq!(state.hostname(), "static-host");

        state.transient_hostname = "transient-host".to_string();
        assert_eq!(state.hostname(), "transient-host");
    }

    #[test]
    fn test_effective_icon_name_explicit() {
        let state = HostnameState {
            icon_name: "my-icon".to_string(),
            ..Default::default()
        };
        assert_eq!(state.effective_icon_name(), "my-icon");
    }

    #[test]
    fn test_effective_icon_name_from_chassis() {
        let mut state = HostnameState {
            chassis: "laptop".to_string(),
            ..Default::default()
        };
        assert_eq!(state.effective_icon_name(), "computer-laptop");

        state.chassis = "server".to_string();
        assert_eq!(state.effective_icon_name(), "computer-server");

        state.chassis = "tablet".to_string();
        assert_eq!(state.effective_icon_name(), "computer-tablet");

        state.chassis = "vm".to_string();
        assert_eq!(state.effective_icon_name(), "computer-vm");

        state.chassis = "container".to_string();
        assert_eq!(state.effective_icon_name(), "container");
    }

    #[test]
    fn test_effective_icon_name_default() {
        let state = HostnameState::default();
        assert_eq!(state.effective_icon_name(), "computer");
    }

    // -- set_static_hostname tests --

    #[test]
    fn test_set_static_hostname() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hostname");

        set_static_hostname_at("myhost", path.to_str().unwrap()).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert_eq!(content.trim(), "myhost");
    }

    #[test]
    fn test_set_static_hostname_empty_removes_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hostname");

        // Create the file first
        fs::write(&path, "oldhost\n").unwrap();
        assert!(path.exists());

        // Setting empty should remove it
        set_static_hostname_at("", path.to_str().unwrap()).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_set_static_hostname_empty_nonexistent_ok() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("hostname");
        // Should not error even if file doesn't exist
        set_static_hostname_at("", path.to_str().unwrap()).unwrap();
    }

    // -- set_machine_info_key tests --

    #[test]
    fn test_set_machine_info_key_create() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("machine-info");

        set_machine_info_key_at("CHASSIS", "laptop", path.to_str().unwrap()).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("CHASSIS=laptop"));
    }

    #[test]
    fn test_set_machine_info_key_update() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("machine-info");
        fs::write(&path, "CHASSIS=desktop\nLOCATION=here\n").unwrap();

        set_machine_info_key_at("CHASSIS", "laptop", path.to_str().unwrap()).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("CHASSIS=laptop"));
        assert!(content.contains("LOCATION=here"));
        assert!(!content.contains("desktop"));
    }

    #[test]
    fn test_set_machine_info_key_remove() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("machine-info");
        fs::write(&path, "CHASSIS=desktop\nLOCATION=here\n").unwrap();

        set_machine_info_key_at("CHASSIS", "", path.to_str().unwrap()).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(!content.contains("CHASSIS"));
        assert!(content.contains("LOCATION=here"));
    }

    #[test]
    fn test_set_machine_info_key_quoted_value() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("machine-info");

        set_machine_info_key_at("PRETTY_HOSTNAME", "My Pretty Host", path.to_str().unwrap())
            .unwrap();
        let content = fs::read_to_string(&path).unwrap();
        assert!(content.contains("PRETTY_HOSTNAME=\"My Pretty Host\""));
    }

    // -- format tests --

    #[test]
    fn test_format_status() {
        let state = HostnameState {
            static_hostname: "testhost".to_string(),
            chassis: "laptop".to_string(),
            os_pretty_name: "NixOS 25.11".to_string(),
            kernel_name: "Linux".to_string(),
            kernel_release: "6.12.0".to_string(),
            ..Default::default()
        };

        let output = state.format_status();
        assert!(output.contains("testhost"));
        assert!(output.contains("laptop"));
        assert!(output.contains("NixOS 25.11"));
        assert!(output.contains("Linux 6.12.0"));
    }

    #[test]
    fn test_format_status_unset_hostname() {
        let state = HostnameState::default();
        let output = state.format_status();
        assert!(output.contains("(unset)"));
    }

    #[test]
    fn test_format_show() {
        let state = HostnameState {
            static_hostname: "testhost".to_string(),
            chassis: "server".to_string(),
            ..Default::default()
        };

        let output = state.format_show();
        assert!(output.contains("Hostname="));
        assert!(output.contains("StaticHostname=testhost"));
        assert!(output.contains("Chassis=server"));
    }

    // -- control command tests --

    #[test]
    fn test_handle_control_status() {
        let response = handle_control_command("STATUS");
        // Should contain hostname info; not an error
        assert!(!response.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_control_show() {
        let response = handle_control_command("SHOW");
        assert!(response.contains("Hostname="));
        assert!(response.contains("StaticHostname="));
    }

    #[test]
    fn test_handle_control_unknown() {
        let response = handle_control_command("NONSENSE");
        assert!(response.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_control_set_hostname_invalid() {
        let response = handle_control_command("SET-HOSTNAME -invalid");
        assert!(response.starts_with("ERROR"));
    }

    #[test]
    fn test_handle_control_set_chassis_invalid() {
        let response = handle_control_command("SET-CHASSIS spaceship");
        assert!(response.starts_with("ERROR"));
    }

    // -- write_env_file tests --

    #[test]
    fn test_write_env_file_empty_removes() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("env");
        fs::write(&path, "KEY=val\n").unwrap();

        let entries = BTreeMap::new();
        write_env_file(path.to_str().unwrap(), &entries).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_write_env_file_sorted() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("env");

        let mut entries = BTreeMap::new();
        entries.insert("ZZZ".to_string(), "last".to_string());
        entries.insert("AAA".to_string(), "first".to_string());

        write_env_file(path.to_str().unwrap(), &entries).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "AAA=first");
        assert_eq!(lines[1], "ZZZ=last");
    }

    // -- HostnameState with fallback USR os-release --

    #[test]
    fn test_load_state_usr_os_release_fallback() {
        let dir = TempDir::new().unwrap();
        let hostname_path = write_file(&dir, "hostname", "testhost\n");
        let machine_info_path = dir.path().join("machine-info"); // doesn't exist
        let usr_os_release = write_file(&dir, "usr-os-release", "PRETTY_NAME=\"Fallback OS\"\n");

        let state = HostnameState::load_from(
            hostname_path.to_str().unwrap(),
            machine_info_path.to_str().unwrap(),
            "/nonexistent/os-release",
            usr_os_release.to_str().unwrap(),
            "/nonexistent/chassis_type",
            "/nonexistent/sys_vendor",
            "/nonexistent/product_name",
        );

        assert_eq!(state.os_pretty_name, "Fallback OS");
    }

    // -- DMI vendor/model fallback tests --

    #[test]
    fn test_load_hardware_from_dmi() {
        let dir = TempDir::new().unwrap();
        let hostname_path = write_file(&dir, "hostname", "h\n");
        let machine_info_path = dir.path().join("machine-info");
        let dmi_vendor = write_file(&dir, "sys_vendor", "QEMU\n");
        let dmi_model = write_file(&dir, "product_name", "Virtual Machine\n");

        let state = HostnameState::load_from(
            hostname_path.to_str().unwrap(),
            machine_info_path.to_str().unwrap(),
            "/nonexistent/os-release",
            "/nonexistent/usr-os-release",
            "/nonexistent/chassis_type",
            dmi_vendor.to_str().unwrap(),
            dmi_model.to_str().unwrap(),
        );

        assert_eq!(state.hardware_vendor, "QEMU");
        assert_eq!(state.hardware_model, "Virtual Machine");
    }

    #[test]
    fn test_load_hardware_from_machine_info_overrides_dmi() {
        let dir = TempDir::new().unwrap();
        let hostname_path = write_file(&dir, "hostname", "h\n");
        let machine_info_path = write_file(
            &dir,
            "machine-info",
            "HARDWARE_VENDOR=Custom\nHARDWARE_MODEL=MyModel\n",
        );
        let dmi_vendor = write_file(&dir, "sys_vendor", "QEMU\n");
        let dmi_model = write_file(&dir, "product_name", "Virtual Machine\n");

        let state = HostnameState::load_from(
            hostname_path.to_str().unwrap(),
            machine_info_path.to_str().unwrap(),
            "/nonexistent/os-release",
            "/nonexistent/usr-os-release",
            "/nonexistent/chassis_type",
            dmi_vendor.to_str().unwrap(),
            dmi_model.to_str().unwrap(),
        );

        assert_eq!(state.hardware_vendor, "Custom");
        assert_eq!(state.hardware_model, "MyModel");
    }

    // -- uname / kernel info --

    #[test]
    fn test_get_uname_returns_something() {
        let (sysname, _release) = get_uname();
        assert!(!sysname.is_empty());
        // release might be empty in some environments but sysname should not
    }

    #[test]
    fn test_get_kernel_hostname_returns_something() {
        let h = get_kernel_hostname();
        // In test environments this should be set
        assert!(!h.is_empty());
    }

    // -- max-length hostname --

    #[test]
    fn test_valid_hostname_max_length() {
        let name = "a".repeat(64);
        assert!(is_valid_hostname(&name));
    }

    #[test]
    fn test_hostname_with_dots() {
        assert!(is_valid_hostname("host.example.com"));
    }

    #[test]
    fn test_hostname_ends_with_dot_invalid() {
        assert!(!is_valid_hostname("host.example.com."));
    }

    // -- control socket command handler edge cases --

    #[test]
    fn test_handle_empty_command() {
        let resp = handle_control_command("");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_handle_case_insensitive_commands() {
        let _s = handle_control_command("STATUS");
        let _s2 = handle_control_command("status");
        // Both should succeed (not return ERROR)
        assert!(!_s.starts_with("ERROR"));
        assert!(!_s2.starts_with("ERROR"));
    }

    // --- D-Bus interface tests ---

    #[test]
    fn test_json_escape_plain() {
        assert_eq!(json_escape("hello"), "hello");
    }

    #[test]
    fn test_json_escape_quotes() {
        assert_eq!(json_escape(r#"say "hi""#), r#"say \"hi\""#);
    }

    #[test]
    fn test_json_escape_backslash() {
        assert_eq!(json_escape(r"a\b"), r"a\\b");
    }

    #[test]
    fn test_json_escape_newline() {
        assert_eq!(json_escape("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_json_escape_control_char() {
        assert_eq!(json_escape("\x01"), "\\u0001");
    }

    #[test]
    fn test_json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    #[test]
    fn test_dbus_hostname1_manager_struct() {
        // Verify the interface struct can be created without panic
        let shared: SharedState = Arc::new(Mutex::new(HostnameState::default()));
        let _mgr = Hostname1Manager { state: shared };
    }

    #[test]
    fn test_shared_state_reload() {
        let state = HostnameState {
            static_hostname: "host1".to_string(),
            transient_hostname: "host1".to_string(),
            ..Default::default()
        };
        let shared: SharedState = Arc::new(Mutex::new(state));

        // Simulate a reload
        {
            let mut s = shared.lock().unwrap();
            s.static_hostname = "host2".to_string();
            s.transient_hostname = "host2".to_string();
        }

        let s = shared.lock().unwrap();
        assert_eq!(s.hostname(), "host2");
    }

    #[test]
    fn test_hostname_source_logic() {
        // Static hostname set → source is "static"
        let state = HostnameState {
            static_hostname: "myhost".to_string(),
            transient_hostname: "myhost".to_string(),
            ..Default::default()
        };
        let source = if !state.static_hostname.is_empty() {
            "static"
        } else if !state.transient_hostname.is_empty() {
            "transient"
        } else {
            "default"
        };
        assert_eq!(source, "static");

        // Only transient → source is "transient"
        let state2 = HostnameState {
            static_hostname: String::new(),
            transient_hostname: "tmphost".to_string(),
            ..Default::default()
        };
        let source2 = if !state2.static_hostname.is_empty() {
            "static"
        } else if !state2.transient_hostname.is_empty() {
            "transient"
        } else {
            "default"
        };
        assert_eq!(source2, "transient");

        // Neither → source is "default"
        let state3 = HostnameState::default();
        let source3 = if !state3.static_hostname.is_empty() {
            "static"
        } else if !state3.transient_hostname.is_empty() {
            "transient"
        } else {
            "default"
        };
        assert_eq!(source3, "default");
    }
}
