//! hostnamectl — query and change the system hostname and related settings
//!
//! This is a Rust implementation of systemd's `hostnamectl` command. It reads
//! and writes hostname state directly from/to the filesystem:
//! - `/etc/hostname` for the static hostname
//! - `/etc/machine-info` for pretty hostname, chassis, deployment, location, icon
//! - `/etc/os-release` (or `/usr/lib/os-release`) for OS information
//! - `/sys/class/dmi/id/` for hardware information
//! - Kernel hostname via `gethostname(2)` / `sethostname(2)`

use std::collections::BTreeMap;
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process;

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
const DMI_BIOS_DATE_PATH: &str = "/sys/class/dmi/id/bios_date";
const DMI_PRODUCT_SERIAL_PATH: &str = "/sys/class/dmi/id/product_serial";
const DMI_BOARD_SERIAL_PATH: &str = "/sys/class/dmi/id/board_serial";

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

#[derive(Debug, Clone, Default)]
struct HostnameState {
    static_hostname: String,
    pretty_hostname: String,
    transient_hostname: String,
    chassis: String,
    deployment: String,
    location: String,
    icon_name: String,
    hardware_vendor: String,
    hardware_model: String,
    hardware_serial: String,
    firmware_date: String,
    os_pretty_name: String,
    os_cpe_name: String,
    os_home_url: String,
    kernel_name: String,
    kernel_release: String,
    machine_id: String,
    boot_id: String,
}

impl HostnameState {
    fn load() -> Self {
        let mut state = Self {
            static_hostname: read_trimmed(HOSTNAME_PATH).unwrap_or_default(),
            transient_hostname: get_kernel_hostname(),
            ..Default::default()
        };

        // Machine-info
        let mi = parse_env_file(MACHINE_INFO_PATH);
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
            .or_else(|| read_trimmed(DMI_VENDOR_PATH))
            .unwrap_or_default();
        state.hardware_model = mi
            .get("HARDWARE_MODEL")
            .cloned()
            .or_else(|| read_trimmed(DMI_MODEL_PATH))
            .unwrap_or_default();

        // Hardware serial: prefer product_serial over board_serial
        state.hardware_serial = read_trimmed(DMI_PRODUCT_SERIAL_PATH)
            .or_else(|| read_trimmed(DMI_BOARD_SERIAL_PATH))
            .unwrap_or_default();

        // Firmware date from BIOS date (MM/DD/YYYY -> YYYY-MM-DD)
        state.firmware_date = read_trimmed(DMI_BIOS_DATE_PATH)
            .and_then(|s| parse_bios_date(&s))
            .unwrap_or_default();

        // If no chassis in machine-info, try to auto-detect.
        // Check virtualization first (like real systemd does), then fall
        // back to DMI chassis type.
        if state.chassis.is_empty() {
            state.chassis = detect_chassis();
        }

        // OS release info
        let osr = if Path::new(OS_RELEASE_PATH).exists() {
            parse_env_file(OS_RELEASE_PATH)
        } else {
            parse_env_file(OS_RELEASE_USR_PATH)
        };
        state.os_pretty_name = osr.get("PRETTY_NAME").cloned().unwrap_or_default();
        state.os_cpe_name = osr.get("CPE_NAME").cloned().unwrap_or_default();
        state.os_home_url = osr.get("HOME_URL").cloned().unwrap_or_default();

        // Kernel info
        let uname = get_uname();
        state.kernel_name = uname.0;
        state.kernel_release = uname.1;

        // Machine ID
        state.machine_id = read_trimmed("/etc/machine-id").unwrap_or_default();

        // Boot ID
        state.boot_id = read_trimmed("/proc/sys/kernel/random/boot_id").unwrap_or_default();

        state
    }

    fn hostname(&self) -> &str {
        if !self.transient_hostname.is_empty() {
            &self.transient_hostname
        } else if !self.static_hostname.is_empty() {
            &self.static_hostname
        } else {
            "localhost"
        }
    }

    fn effective_icon_name(&self) -> String {
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
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn read_trimmed(path: &str) -> Option<String> {
    fs::read_to_string(path).ok().map(|s| s.trim().to_string())
}

fn parse_env_file(path: &str) -> BTreeMap<String, String> {
    parse_env_file_content(&fs::read_to_string(path).unwrap_or_default())
}

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
            // Unescape
            value = value.replace("\\\"", "\"").replace("\\\\", "\\");
            if !key.is_empty() {
                map.insert(key.to_string(), value);
            }
        }
    }
    map
}

fn write_env_file(path: &str, entries: &BTreeMap<String, String>) -> io::Result<()> {
    if entries.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
        return Ok(());
    }

    let mut f = fs::File::create(path)?;
    for (k, v) in entries {
        if v.contains(|c: char| c.is_whitespace() || c == '"' || c == '\\' || c == '$') {
            let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
            writeln!(f, "{}=\"{}\"", k, escaped)?;
        } else {
            writeln!(f, "{}={}", k, v)?;
        }
    }
    Ok(())
}

fn get_kernel_hostname() -> String {
    nix::unistd::gethostname()
        .ok()
        .and_then(|s| s.into_string().ok())
        .unwrap_or_default()
}

fn get_uname() -> (String, String) {
    match nix::sys::utsname::uname() {
        Ok(u) => (
            u.sysname().to_string_lossy().to_string(),
            u.release().to_string_lossy().to_string(),
        ),
        Err(_) => ("Linux".to_string(), String::new()),
    }
}

/// Detect chassis type by checking virtualization first, then DMI.
///
/// Real systemd uses systemd-detect-virt to determine if running in a VM
/// or container, and returns "vm" or "container" accordingly. Only if
/// not virtualized does it fall back to DMI chassis type.
fn detect_chassis() -> String {
    // Check if running in a container
    if std::process::Command::new("systemd-detect-virt")
        .arg("-q")
        .arg("-c")
        .status()
        .is_ok_and(|s| s.success())
    {
        return "container".to_string();
    }

    // Check if running in a VM
    if std::process::Command::new("systemd-detect-virt")
        .arg("-q")
        .arg("-v")
        .status()
        .is_ok_and(|s| s.success())
    {
        return "vm".to_string();
    }

    // Fall back to DMI chassis type
    detect_chassis_from_dmi(DMI_CHASSIS_TYPE_PATH)
}

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
        1 | 2 => "desktop".to_string(),
        3 | 4 | 5 | 6 | 7 | 13 | 15 | 16 | 24 => "desktop".to_string(),
        8 | 9 | 10 | 12 | 14 => "laptop".to_string(),
        11 => "handset".to_string(),
        17 | 23 | 25 | 28 => "server".to_string(),
        30 => "tablet".to_string(),
        31 => "convertible".to_string(),
        32 => "laptop".to_string(),
        _ => String::new(),
    }
}

fn is_valid_hostname(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }
    // systemd allows hostnames with wildcards (?, *) for pattern-based hostnames
    // Only reject truly invalid characters
    name.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '?' || c == '*')
}

fn is_valid_chassis(chassis: &str) -> bool {
    chassis.is_empty() || VALID_CHASSIS.contains(&chassis)
}

fn set_static_hostname(hostname: &str) -> io::Result<()> {
    let clean = hostname.trim();
    if clean.is_empty() {
        match fs::remove_file(HOSTNAME_PATH) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(e),
        }
    } else {
        let mut f = fs::File::create(HOSTNAME_PATH)?;
        writeln!(f, "{}", clean)?;
    }
    Ok(())
}

fn set_transient_hostname(hostname: &str) -> io::Result<()> {
    nix::unistd::sethostname(hostname).map_err(|e| io::Error::other(e.to_string()))
}

fn set_machine_info_key(key: &str, value: &str) -> io::Result<()> {
    let mut entries = parse_env_file(MACHINE_INFO_PATH);
    if value.is_empty() {
        entries.remove(key);
    } else {
        entries.insert(key.to_string(), value.to_string());
    }
    write_env_file(MACHINE_INFO_PATH, &entries)
}

// ---------------------------------------------------------------------------
// Commands
// ---------------------------------------------------------------------------

fn cmd_status() {
    let state = HostnameState::load();

    // Match systemd's hostnamectl output format
    let label_width = 21;

    if state.static_hostname.is_empty() {
        println!("{:>label_width$}: (unset)", "Static hostname");
    } else {
        println!(
            "{:>label_width$}: {}",
            "Static hostname", state.static_hostname
        );
    }

    if !state.pretty_hostname.is_empty() {
        println!(
            "{:>label_width$}: {}",
            "Pretty hostname", state.pretty_hostname
        );
    }

    if !state.transient_hostname.is_empty() && state.transient_hostname != state.static_hostname {
        println!(
            "{:>label_width$}: {}",
            "Transient hostname", state.transient_hostname
        );
    }

    println!(
        "{:>label_width$}: {}",
        "Icon name",
        state.effective_icon_name()
    );

    if !state.chassis.is_empty() {
        println!("{:>label_width$}: {}", "Chassis", state.chassis);
    }

    if !state.deployment.is_empty() {
        println!("{:>label_width$}: {}", "Deployment", state.deployment);
    }

    if !state.location.is_empty() {
        println!("{:>label_width$}: {}", "Location", state.location);
    }

    if !state.machine_id.is_empty() {
        println!("{:>label_width$}: {}", "Machine ID", state.machine_id);
    }

    if !state.boot_id.is_empty() {
        println!("{:>label_width$}: {}", "Boot ID", state.boot_id);
    }

    if !state.os_pretty_name.is_empty() {
        println!(
            "{:>label_width$}: {}",
            "Operating System", state.os_pretty_name
        );
    }

    if !state.os_cpe_name.is_empty() {
        println!("{:>label_width$}: {}", "CPE OS Name", state.os_cpe_name);
    }

    if !state.os_home_url.is_empty() {
        println!("{:>label_width$}: {}", "Home URL", state.os_home_url);
    }

    if !state.kernel_name.is_empty() || !state.kernel_release.is_empty() {
        println!(
            "{:>label_width$}: {} {}",
            "Kernel", state.kernel_name, state.kernel_release
        );
    }

    let arch = get_architecture();
    if !arch.is_empty() {
        println!("{:>label_width$}: {}", "Architecture", arch);
    }

    if !state.hardware_vendor.is_empty() {
        println!(
            "{:>label_width$}: {}",
            "Hardware Vendor", state.hardware_vendor
        );
    }

    if !state.hardware_model.is_empty() {
        println!(
            "{:>label_width$}: {}",
            "Hardware Model", state.hardware_model
        );
    }

    if !state.firmware_date.is_empty() {
        println!("{:>label_width$}: {}", "Firmware Date", state.firmware_date);
    }
}

fn cmd_status_json() {
    let state = HostnameState::load();

    // Build a JSON object matching systemd's hostnamectl --json=short output
    let mut obj = std::collections::BTreeMap::new();
    obj.insert("Hostname", state.hostname().to_string());
    obj.insert("StaticHostname", state.static_hostname.clone());
    obj.insert("PrettyHostname", state.pretty_hostname.clone());
    obj.insert("DefaultHostname", "localhost".to_string());
    obj.insert(
        "HostnameSource",
        if !state.static_hostname.is_empty() {
            "static"
        } else {
            "default"
        }
        .to_string(),
    );
    obj.insert("IconName", state.effective_icon_name());
    obj.insert("Chassis", state.chassis.clone());
    obj.insert("Deployment", state.deployment.clone());
    obj.insert("Location", state.location.clone());
    obj.insert("KernelName", state.kernel_name.clone());
    obj.insert("KernelRelease", state.kernel_release.clone());
    obj.insert("OperatingSystemPrettyName", state.os_pretty_name.clone());
    obj.insert("OperatingSystemCPEName", state.os_cpe_name.clone());
    obj.insert("OperatingSystemHomeURL", state.os_home_url.clone());
    obj.insert("HardwareVendor", state.hardware_vendor.clone());
    obj.insert("HardwareModel", state.hardware_model.clone());
    obj.insert("HardwareSerial", state.hardware_serial.clone());
    obj.insert("FirmwareDate", state.firmware_date.clone());
    obj.insert("MachineID", state.machine_id.clone());
    obj.insert("BootID", state.boot_id.clone());

    // Manual JSON serialization to avoid adding serde dependency
    print!("{{");
    let mut first = true;
    for (k, v) in &obj {
        if !first {
            print!(",");
        }
        first = false;
        // Escape JSON string values
        let escaped = v.replace('\\', "\\\\").replace('"', "\\\"");
        print!("\"{}\":\"{}\"", k, escaped);
    }
    println!("}}");
}

fn cmd_show(properties: &[String]) {
    let state = HostnameState::load();

    let all_props: Vec<(&str, String)> = vec![
        ("Hostname", state.hostname().to_string()),
        ("StaticHostname", state.static_hostname.clone()),
        ("PrettyHostname", state.pretty_hostname.clone()),
        ("TransientHostname", state.transient_hostname.clone()),
        ("IconName", state.effective_icon_name()),
        ("Chassis", state.chassis.clone()),
        ("Deployment", state.deployment.clone()),
        ("Location", state.location.clone()),
        ("OperatingSystemPrettyName", state.os_pretty_name.clone()),
        ("OperatingSystemCPEName", state.os_cpe_name.clone()),
        ("OperatingSystemHomeURL", state.os_home_url.clone()),
        ("KernelName", state.kernel_name.clone()),
        ("KernelRelease", state.kernel_release.clone()),
        ("HardwareVendor", state.hardware_vendor.clone()),
        ("HardwareModel", state.hardware_model.clone()),
    ];

    if properties.is_empty() {
        for (key, value) in &all_props {
            println!("{}={}", key, value);
        }
    } else {
        for prop in properties {
            if let Some((_key, value)) = all_props.iter().find(|(k, _)| k == prop) {
                println!("{}={}", prop, value);
            } else {
                // Unknown property — print empty value (matches systemd behavior)
                println!("{}=", prop);
            }
        }
    }
}

fn cmd_set_hostname(hostname: &str, transient_only: bool, static_only: bool) {
    if hostname.is_empty() {
        // Unset: clear static hostname, set transient to localhost
        if !transient_only && let Err(e) = set_static_hostname("") {
            eprintln!("Failed to clear static hostname: {}", e);
            process::exit(1);
        }
        if !static_only && let Err(e) = set_transient_hostname("localhost") {
            eprintln!("Failed to set transient hostname: {}", e);
            process::exit(1);
        }
    } else {
        if !is_valid_hostname(hostname) {
            eprintln!("Invalid hostname: {}", hostname);
            process::exit(1);
        }
        if !transient_only && let Err(e) = set_static_hostname(hostname) {
            eprintln!("Failed to set static hostname: {}", e);
            process::exit(1);
        }
        if !static_only && let Err(e) = set_transient_hostname(hostname) {
            eprintln!("Failed to set transient hostname: {}", e);
            process::exit(1);
        }
    }
}

fn cmd_set_pretty_hostname(pretty: &str) {
    if let Err(e) = set_machine_info_key("PRETTY_HOSTNAME", pretty) {
        eprintln!("Failed to set pretty hostname: {}", e);
        process::exit(1);
    }

    // If setting a pretty hostname and no static hostname is set, derive one
    if !pretty.is_empty() {
        let static_hn = read_trimmed(HOSTNAME_PATH).unwrap_or_default();
        if static_hn.is_empty() {
            let derived = derive_hostname_from_pretty(pretty);
            if !derived.is_empty() && is_valid_hostname(&derived) {
                let _ = set_static_hostname(&derived);
                let _ = set_transient_hostname(&derived);
            }
        }
    }
}

fn cmd_set_chassis(chassis: &str) {
    let lower = chassis.to_lowercase();
    if !lower.is_empty() && !is_valid_chassis(&lower) {
        eprintln!("Invalid chassis type: {}", chassis);
        eprintln!("Valid chassis types: {}", VALID_CHASSIS.join(", "));
        process::exit(1);
    }
    if let Err(e) = set_machine_info_key("CHASSIS", &lower) {
        eprintln!("Failed to set chassis: {}", e);
        process::exit(1);
    }
}

fn cmd_set_deployment(deployment: &str) {
    if let Err(e) = set_machine_info_key("DEPLOYMENT", deployment) {
        eprintln!("Failed to set deployment: {}", e);
        process::exit(1);
    }
}

fn cmd_set_location(location: &str) {
    if let Err(e) = set_machine_info_key("LOCATION", location) {
        eprintln!("Failed to set location: {}", e);
        process::exit(1);
    }
}

fn cmd_set_icon_name(icon: &str) {
    if let Err(e) = set_machine_info_key("ICON_NAME", icon) {
        eprintln!("Failed to set icon name: {}", e);
        process::exit(1);
    }
}

fn derive_hostname_from_pretty(pretty: &str) -> String {
    pretty
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '.' {
                c.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect::<String>()
        .trim_matches('-')
        .to_string()
}

/// Parse BIOS date from MM/DD/YYYY format to YYYY-MM-DD
fn parse_bios_date(s: &str) -> Option<String> {
    let parts: Vec<&str> = s.split('/').collect();
    if parts.len() != 3 {
        return None;
    }
    let month: u32 = parts[0].parse().ok()?;
    let day: u32 = parts[1].parse().ok()?;
    let year: u32 = parts[2].parse().ok()?;
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }
    Some(format!("{:04}-{:02}-{:02}", year, month, day))
}

fn get_architecture() -> String {
    match nix::sys::utsname::uname() {
        Ok(u) => u.machine().to_string_lossy().to_string(),
        Err(_) => String::new(),
    }
}

// ---------------------------------------------------------------------------
// Usage / help
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!("hostnamectl [OPTIONS] COMMAND ...");
    eprintln!();
    eprintln!("Query or change system hostname.");
    eprintln!();
    eprintln!("Commands:");
    eprintln!("  status                 Show current hostname settings (default)");
    eprintln!("  hostname [NAME]        Get/set system hostname");
    eprintln!("  icon-name [NAME]       Get/set icon name for host");
    eprintln!("  chassis [TYPE]         Get/set chassis type for host");
    eprintln!("  deployment [ENV]       Get/set deployment environment for host");
    eprintln!("  location [LOC]         Get/set location for host");
    eprintln!();
    eprintln!("Compatibility commands:");
    eprintln!("  set-hostname NAME      Set system hostname");
    eprintln!("  set-icon-name NAME     Set icon name");
    eprintln!("  set-chassis TYPE       Set chassis type");
    eprintln!("  set-deployment ENV     Set deployment environment");
    eprintln!("  set-location LOC       Set location string");
    eprintln!();
    eprintln!("Options:");
    eprintln!("  --transient            Only set transient hostname");
    eprintln!("  --static               Only set static hostname");
    eprintln!("  --pretty               Set pretty hostname");
    eprintln!("  -p, --property=PROP    Show only specified property (with show)");
    eprintln!("  -H, --host=HOST        Operate on remote host (not supported)");
    eprintln!("  --no-ask-password      Do not ask for system passwords");
    eprintln!("  -h, --help             Show this help");
    eprintln!();
    eprintln!("See the hostnamectl(1) man page for details.");
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn main() {
    let args: Vec<String> = env::args().collect();

    // Parse flags
    let mut transient_only = false;
    let mut static_only = false;
    let mut pretty_mode = false;
    let mut json_mode: Option<String> = None;
    let mut properties: Vec<String> = Vec::new();
    let mut positional: Vec<String> = Vec::new();
    let mut skip_next = false;

    for i in 1..args.len() {
        if skip_next {
            skip_next = false;
            continue;
        }

        let arg = &args[i];
        match arg.as_str() {
            "--transient" => transient_only = true,
            "--static" => static_only = true,
            "--pretty" => pretty_mode = true,
            "--no-ask-password" | "--no-pager" => {} // silently accept
            "--json" => {
                if i + 1 < args.len() {
                    json_mode = Some(args[i + 1].clone());
                    skip_next = true;
                }
            }
            "-h" | "--help" | "help" => {
                print_usage();
                return;
            }
            "-H" | "--host" => {
                eprintln!("Remote host operation is not supported.");
                process::exit(1);
            }
            "-p" | "--property" => {
                if i + 1 < args.len() {
                    properties.push(args[i + 1].clone());
                    skip_next = true;
                } else {
                    eprintln!("--property requires a value");
                    process::exit(1);
                }
            }
            other if other.starts_with("--property=") => {
                if let Some(val) = other.strip_prefix("--property=") {
                    properties.push(val.to_string());
                }
            }
            other if other.starts_with("--json=") => {
                json_mode = other.strip_prefix("--json=").map(|s| s.to_string());
            }
            other if other.starts_with("--host=") => {
                eprintln!("Remote host operation is not supported.");
                process::exit(1);
            }
            other if other.starts_with('-') && !other.starts_with("--") && other.len() > 1 => {
                // Handle combined short flags like -pH
                for ch in other[1..].chars() {
                    match ch {
                        'h' => {
                            print_usage();
                            return;
                        }
                        'p' => {
                            if i + 1 < args.len() {
                                properties.push(args[i + 1].clone());
                                skip_next = true;
                            }
                        }
                        _ => {} // ignore unknown short flags
                    }
                }
            }
            _ => positional.push(arg.clone()),
        }
    }

    if positional.is_empty() {
        if json_mode.is_some() {
            cmd_status_json();
        } else {
            cmd_status();
        }
        return;
    }

    let command = positional[0].as_str();
    let rest = &positional[1..];

    match command {
        "status" => {
            if json_mode.is_some() {
                cmd_status_json();
            } else {
                cmd_status();
            }
        }
        "show" => {
            cmd_show(&properties);
        }
        "hostname" => {
            if rest.is_empty() {
                // Query mode
                let state = HostnameState::load();
                if pretty_mode {
                    println!("{}", state.pretty_hostname);
                } else {
                    println!("{}", state.hostname());
                }
            } else if pretty_mode {
                cmd_set_pretty_hostname(&rest.join(" "));
            } else {
                cmd_set_hostname(&rest[0], transient_only, static_only);
            }
        }
        "set-hostname" => {
            if rest.is_empty() {
                // Unset hostname
                if pretty_mode {
                    cmd_set_pretty_hostname("");
                } else {
                    cmd_set_hostname("", transient_only, static_only);
                }
            } else if pretty_mode {
                cmd_set_pretty_hostname(&rest.join(" "));
            } else {
                cmd_set_hostname(&rest[0], transient_only, static_only);
            }
        }
        "icon-name" => {
            if rest.is_empty() {
                let state = HostnameState::load();
                println!("{}", state.effective_icon_name());
            } else {
                cmd_set_icon_name(&rest[0]);
            }
        }
        "set-icon-name" => {
            let value = if rest.is_empty() { "" } else { &rest[0] };
            cmd_set_icon_name(value);
        }
        "chassis" => {
            if rest.is_empty() {
                let state = HostnameState::load();
                println!("{}", state.chassis);
            } else {
                cmd_set_chassis(&rest[0]);
            }
        }
        "set-chassis" => {
            let value = if rest.is_empty() { "" } else { &rest[0] };
            cmd_set_chassis(value);
        }
        "deployment" => {
            if rest.is_empty() {
                let state = HostnameState::load();
                println!("{}", state.deployment);
            } else {
                cmd_set_deployment(&rest[0]);
            }
        }
        "set-deployment" => {
            let value = if rest.is_empty() { "" } else { &rest[0] };
            cmd_set_deployment(value);
        }
        "location" => {
            if rest.is_empty() {
                let state = HostnameState::load();
                println!("{}", state.location);
            } else {
                cmd_set_location(&rest.join(" "));
            }
        }
        "set-location" => {
            let value = if rest.is_empty() { "" } else { &rest.join(" ") };
            cmd_set_location(value);
        }
        other => {
            eprintln!("Unknown command: {}", other);
            eprintln!();
            print_usage();
            process::exit(1);
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_env_file_content --

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
    fn test_hostname_with_wildcards() {
        assert!(is_valid_hostname("foo-??-??.????bar"));
        assert!(is_valid_hostname("host-*"));
    }

    #[test]
    fn test_invalid_hostname_special_chars() {
        assert!(!is_valid_hostname("host name"));
        assert!(!is_valid_hostname("host_name"));
        assert!(!is_valid_hostname("host@name"));
    }

    #[test]
    fn test_parse_bios_date() {
        assert_eq!(
            parse_bios_date("09/08/2000"),
            Some("2000-09-08".to_string())
        );
        assert_eq!(parse_bios_date("2022"), None);
        assert_eq!(parse_bios_date("garbage"), None);
        assert_eq!(parse_bios_date("13/01/2020"), None); // month > 12
    }

    #[test]
    fn test_valid_hostname_max_length() {
        let name = "a".repeat(64);
        assert!(is_valid_hostname(&name));
    }

    #[test]
    fn test_hostname_with_dots() {
        assert!(is_valid_hostname("host.example.com"));
        // Trailing dots are valid in our relaxed validation
        assert!(is_valid_hostname("host.example.com."));
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
        assert!(!is_valid_chassis("DESKTOP"));
    }

    // -- chassis detection --

    #[test]
    fn test_detect_chassis_nonexistent() {
        assert_eq!(detect_chassis_from_dmi("/nonexistent/chassis_type"), "");
    }

    // -- derive_hostname_from_pretty --

    #[test]
    fn test_derive_hostname_simple() {
        assert_eq!(derive_hostname_from_pretty("My Server"), "my-server");
    }

    #[test]
    fn test_derive_hostname_special_chars() {
        assert_eq!(
            derive_hostname_from_pretty("Server #1 (Production)"),
            "server--1--production"
        );
    }

    #[test]
    fn test_derive_hostname_leading_trailing() {
        assert_eq!(derive_hostname_from_pretty(" - server - "), "server");
    }

    #[test]
    fn test_derive_hostname_empty() {
        assert_eq!(derive_hostname_from_pretty(""), "");
    }

    // -- effective icon name --

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

    // -- hostname resolution --

    #[test]
    fn test_hostname_priority() {
        let mut state = HostnameState::default();
        assert_eq!(state.hostname(), "localhost");

        state.static_hostname = "static-host".to_string();
        assert_eq!(state.hostname(), "static-host");

        state.transient_hostname = "transient-host".to_string();
        assert_eq!(state.hostname(), "transient-host");
    }

    // -- get_architecture --

    #[test]
    fn test_get_architecture_not_empty() {
        let arch = get_architecture();
        assert!(!arch.is_empty());
    }

    // -- get_kernel_hostname --

    #[test]
    fn test_get_kernel_hostname_not_empty() {
        let h = get_kernel_hostname();
        assert!(!h.is_empty());
    }

    // -- get_uname --

    #[test]
    fn test_get_uname_sysname() {
        let (sysname, _release) = get_uname();
        assert_eq!(sysname, "Linux");
    }

    // -- write_env_file --

    #[test]
    fn test_write_env_file_roundtrip() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("env");
        let path_str = path.to_str().unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("KEY1".to_string(), "value1".to_string());
        entries.insert("KEY2".to_string(), "value with spaces".to_string());

        write_env_file(path_str, &entries).unwrap();

        let parsed = parse_env_file(path_str);
        assert_eq!(parsed.get("KEY1").unwrap(), "value1");
        assert_eq!(parsed.get("KEY2").unwrap(), "value with spaces");
    }

    #[test]
    fn test_write_env_file_empty_removes() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("env");
        fs::write(&path, "KEY=val\n").unwrap();
        assert!(path.exists());

        let entries = BTreeMap::new();
        write_env_file(path.to_str().unwrap(), &entries).unwrap();
        assert!(!path.exists());
    }

    #[test]
    fn test_write_env_file_sorted() {
        let dir = tempfile::TempDir::new().unwrap();
        let path = dir.path().join("env");
        let path_str = path.to_str().unwrap();

        let mut entries = BTreeMap::new();
        entries.insert("ZZZ".to_string(), "last".to_string());
        entries.insert("AAA".to_string(), "first".to_string());

        write_env_file(path_str, &entries).unwrap();
        let content = fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().collect();
        assert_eq!(lines[0], "AAA=first");
        assert_eq!(lines[1], "ZZZ=last");
    }
}
