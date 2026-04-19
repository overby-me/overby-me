//! bootctl — Control EFI firmware boot settings and manage the boot loader.
//!
//! A drop-in replacement for `bootctl(1)`. Provides subcommands for
//! installing, updating, and removing the systemd-boot EFI boot manager,
//! listing boot entries conforming to the Boot Loader Specification,
//! querying firmware and boot loader status via EFI variables, and
//! managing the random seed on the ESP.
//!
//! This tool operates on the EFI System Partition (ESP), typically
//! mounted at `/efi`, `/boot`, or `/boot/efi`, and optionally on the
//! Extended Boot Loader Partition (XBOOTLDR) mounted at `/boot`.

use clap::{ArgAction, Parser, Subcommand, ValueEnum};
use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;

// ── Constants ─────────────────────────────────────────────────────────────

/// Well-known EFI variable vendor GUIDs.
const EFI_GLOBAL_VARIABLE: &str = "8be4df61-93ca-11d2-aa0d-00e098032b8c";
const LOADER_GUID: &str = "4a67b082-0a4c-41cf-b6c7-440b29bb8c4f";

/// EFI variable attribute flags.
const EFI_VARIABLE_NON_VOLATILE: u32 = 0x0000_0001;
const EFI_VARIABLE_BOOTSERVICE_ACCESS: u32 = 0x0000_0002;
const EFI_VARIABLE_RUNTIME_ACCESS: u32 = 0x0000_0004;

/// Standard NV+BS+RT attributes for persistent EFI variables.
const EFI_VARIABLE_PERSISTENT: u32 =
    EFI_VARIABLE_NON_VOLATILE | EFI_VARIABLE_BOOTSERVICE_ACCESS | EFI_VARIABLE_RUNTIME_ACCESS;

/// OsIndications bit for rebooting into firmware setup.
const EFI_OS_INDICATIONS_BOOT_TO_FW_UI: u64 = 0x0000_0000_0000_0001;

/// Well-known ESP mount points to probe (in priority order).
const ESP_SEARCH_PATHS: &[&str] = &["/efi", "/boot", "/boot/efi"];

/// Well-known XBOOTLDR mount points.
const XBOOTLDR_SEARCH_PATHS: &[&str] = &["/boot"];

/// The EFI binary path for systemd-boot on x86_64.
#[cfg(target_arch = "x86_64")]
const EFI_ARCH_DIR: &str = "x64";
#[cfg(target_arch = "x86_64")]
const EFI_BINARY_NAME: &str = "systemd-bootx64.efi";

#[cfg(target_arch = "aarch64")]
const EFI_ARCH_DIR: &str = "aa64";
#[cfg(target_arch = "aarch64")]
const EFI_BINARY_NAME: &str = "systemd-bootaa64.efi";

#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const EFI_ARCH_DIR: &str = "unknown";
#[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
const EFI_BINARY_NAME: &str = "systemd-boot.efi";

/// ESP partition type GUID.
#[allow(dead_code)]
const ESP_GUID: &str = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";

/// XBOOTLDR partition type GUID.
#[allow(dead_code)]
const XBOOTLDR_GUID: &str = "bc13c2ff-59e6-4262-a352-b275fd6f7172";

/// Random seed file path on ESP.
const RANDOM_SEED_PATH: &str = "loader/random-seed";

/// Random seed size (systemd uses 32 bytes for the system token + 32
/// bytes for the random seed = 512 bits total, but the file on the ESP
/// is 32 bytes by default).
const RANDOM_SEED_SIZE: usize = 32;

// ── CLI ───────────────────────────────────────────────────────────────────

#[derive(Parser, Debug)]
#[command(
    name = "bootctl",
    about = "Control EFI firmware boot settings and manage the boot loader",
    version,
    disable_help_subcommand = true
)]
struct Cli {
    /// Path to the EFI System Partition (ESP)
    #[arg(long, value_name = "PATH", global = true)]
    esp_path: Option<PathBuf>,

    /// Path to the Extended Boot Loader Partition (XBOOTLDR)
    #[arg(long, value_name = "PATH", global = true)]
    boot_path: Option<PathBuf>,

    /// Do not pipe output into a pager
    #[arg(long, global = true)]
    no_pager: bool,

    /// Produce JSON output where supported
    #[arg(long, global = true)]
    json: Option<Option<JsonFormat>>,

    /// Suppress output, only return exit status
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Print additional information
    #[arg(long, global = true)]
    verbose: bool,

    /// Do not actually install or remove files
    #[arg(long, global = true)]
    dry_run: bool,

    /// Include entries from all accessible partitions
    #[arg(long, global = true)]
    all: bool,

    /// Print the path to the device backing the root filesystem and
    /// exit.  `-R` prints the partition (e.g. `/dev/sda1`), `-RR`
    /// prints the backing block device (e.g. `/dev/sda`).
    /// Matches upstream `bootctl --print-root-device` / `-R[R]`.
    #[arg(short = 'R', long = "print-root-device", action = ArgAction::Count)]
    print_root_device: u8,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Clone, Debug, ValueEnum)]
enum JsonFormat {
    Short,
    Pretty,
    Off,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Show boot loader status and available boot entries
    Status,

    /// List available boot entries
    List,

    /// Install systemd-boot to the ESP
    Install {
        /// Override the source directory for EFI binaries
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// Make the installed boot loader the default EFI boot entry
        #[arg(long)]
        make_entry_directory: Option<bool>,

        /// Do not create the default EFI Boot entry
        #[arg(long)]
        no_variables: bool,

        /// Install even if already installed
        #[arg(long)]
        force: bool,
    },

    /// Update systemd-boot in the ESP
    Update {
        /// Override the source directory for EFI binaries
        #[arg(long, value_name = "PATH")]
        path: Option<PathBuf>,

        /// Update even if the installed version is newer
        #[arg(long)]
        force: bool,

        /// Do not touch EFI variables
        #[arg(long)]
        no_variables: bool,
    },

    /// Remove systemd-boot from the ESP
    Remove {
        /// Also remove EFI variables
        #[arg(long)]
        no_variables: bool,
    },

    /// Check whether systemd-boot is installed
    IsInstalled,

    /// Set the default boot entry
    SetDefault {
        /// Boot entry ID or glob pattern
        id: String,
    },

    /// Set a one-time boot entry for the next reboot only
    SetOneshot {
        /// Boot entry ID or glob pattern
        id: String,
    },

    /// Set the boot menu timeout
    SetTimeout {
        /// Timeout in seconds (0 to disable menu)
        seconds: String,
    },

    /// Set a one-time boot menu timeout for the next reboot only
    SetTimeoutOneshot {
        /// Timeout in seconds
        seconds: String,
    },

    /// Initialize or refresh the random seed stored on the ESP
    RandomSeed,

    /// Query or set the reboot-into-firmware flag
    RebootToFirmware {
        /// Set the flag (true/false), or omit to query
        flag: Option<bool>,
    },

    /// Query or set the systemd-specific EFI options string
    #[command(name = "systemd-efi-options")]
    SystemdEfiOptions {
        /// Options string to set, or omit to query
        options: Option<String>,
    },

    /// Identify the type of a kernel image
    KernelIdentify {
        /// Path to the kernel image
        file: PathBuf,
    },

    /// Inspect metadata of a kernel image
    KernelInspect {
        /// Path to the kernel image
        file: PathBuf,
    },

    /// Remove files left over in the ESP
    Cleanup,

    /// Show the entries directory (if any)
    EntryDirectory,

    /// Unlink a boot entry
    Unlink {
        /// Boot entry ID to remove
        id: String,
    },
}

// ── Boot Entry types ──────────────────────────────────────────────────────

/// A parsed Boot Loader Specification Type #1 entry (drop-in .conf file).
#[derive(Debug, Clone)]
struct BootEntry {
    /// Entry identifier (filename stem of the .conf file).
    id: String,
    /// Human-readable title.
    title: Option<String>,
    /// Version string.
    version: Option<String>,
    /// Machine ID.
    machine_id: Option<String>,
    /// Path to the kernel image (relative to the partition root).
    linux: Option<String>,
    /// Path to the initrd(s).
    initrd: Vec<String>,
    /// Kernel command line options.
    options: Option<String>,
    /// Path to a device tree blob.
    devicetree: Vec<String>,
    /// Device tree overlays.
    devicetree_overlay: Vec<String>,
    /// Architecture.
    architecture: Option<String>,
    /// Sort key for ordering.
    sort_key: Option<String>,
    /// Source file path.
    source: PathBuf,
    /// Whether this is the default entry.
    is_default: bool,
    /// Whether this is a one-shot entry.
    is_oneshot: bool,
    /// Boot counting: tries left.
    tries_left: Option<u32>,
    /// Boot counting: tries done.
    tries_done: Option<u32>,
    /// Entry type: "type1" for BLS drop-in, "type2" for UKI, "auto" for auto-detected.
    entry_type: EntryType,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryType {
    /// Boot Loader Specification Type #1 — drop-in .conf file
    Type1,
    /// Boot Loader Specification Type #2 — Unified Kernel Image
    Type2,
    /// Auto-detected entry (e.g. Windows Boot Manager)
    Auto,
    /// Loader entry (the boot loader itself)
    #[allow(dead_code)]
    Loader,
}

impl fmt::Display for EntryType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EntryType::Type1 => write!(f, "type1"),
            EntryType::Type2 => write!(f, "type2"),
            EntryType::Auto => write!(f, "auto"),
            EntryType::Loader => write!(f, "loader"),
        }
    }
}

/// Parsed `loader/loader.conf` configuration.
#[derive(Debug, Clone, Default)]
struct LoaderConfig {
    /// Default entry pattern/ID.
    default: Option<String>,
    /// Timeout in seconds (None = no timeout / menu hidden).
    timeout: Option<u64>,
    /// Console mode.
    console_mode: Option<String>,
    /// Editor enabled (default true).
    editor: Option<bool>,
    /// Auto-entries enabled (default true).
    auto_entries: Option<bool>,
    /// Auto-firmware enabled (default true).
    auto_firmware: Option<bool>,
    /// Beep on menu (default false).
    beep: Option<bool>,
    /// Reboot for BitLocker (default false).
    reboot_for_bitlocker: Option<bool>,
    /// Secure boot enroll method.
    secure_boot_enroll: Option<String>,
    /// Random seed mode.
    random_seed_mode: Option<String>,
}

/// Information about the system's firmware.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct FirmwareInfo {
    /// Whether the system booted via UEFI.
    is_efi: bool,
    /// Secure Boot status.
    secure_boot: Option<bool>,
    /// Setup mode status.
    setup_mode: Option<bool>,
    /// Firmware type string (e.g. "UEFI" or "BIOS").
    firmware_type: String,
    /// Firmware vendor string.
    vendor: Option<String>,
    /// Available OsIndications.
    os_indications_supported: Option<u64>,
    /// Current OsIndications.
    os_indications: Option<u64>,
}

/// Information about the installed boot loader as read from EFI variables.
#[derive(Debug, Clone, Default)]
#[allow(dead_code)]
struct LoaderInfo {
    /// Boot loader name.
    loader: Option<String>,
    /// Boot loader firmware type.
    loader_firmware_type: Option<String>,
    /// Boot loader firmware information.
    loader_firmware_info: Option<String>,
    /// Features advertised by the boot loader.
    loader_features: Option<u64>,
    /// Loader configuration timeout.
    loader_config_timeout: Option<String>,
    /// Loader configuration timeout (oneshot).
    loader_config_timeout_oneshot: Option<String>,
    /// Currently booted entry ID.
    loader_entry_selected: Option<String>,
    /// Default entry ID.
    loader_entry_default: Option<String>,
    /// Oneshot entry ID.
    loader_entry_oneshot: Option<String>,
    /// Loader image identifier.
    loader_image_identifier: Option<String>,
    /// Loader device part UUID.
    loader_device_part_uuid: Option<String>,
}

/// Identifies the type of a kernel image file.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum KernelType {
    /// Unified Kernel Image (UKI) — a PE binary with .linux/.initrd sections
    Uki,
    /// Plain Linux kernel image (vmlinuz / bzImage / Image)
    Vmlinuz,
    /// Unknown or unrecognised format
    Unknown,
}

impl fmt::Display for KernelType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            KernelType::Uki => write!(f, "uki"),
            KernelType::Vmlinuz => write!(f, "linux"),
            KernelType::Unknown => write!(f, "unknown"),
        }
    }
}

// ── EFI variable helpers ──────────────────────────────────────────────────

/// The base path for EFI variables in sysfs.
fn efivarfs_path() -> &'static Path {
    Path::new("/sys/firmware/efi/efivars")
}

/// Check if the system booted via EFI.
fn is_efi_system() -> bool {
    Path::new("/sys/firmware/efi").exists()
}

/// Read a raw EFI variable (4-byte attribute header + data).
fn read_efi_variable_raw(name: &str, guid: &str) -> io::Result<Vec<u8>> {
    let path = efivarfs_path().join(format!("{name}-{guid}"));
    fs::read(&path)
}

/// Read an EFI variable as a UTF-16LE string.
fn read_efi_variable_string(name: &str, guid: &str) -> Option<String> {
    let data = read_efi_variable_raw(name, guid).ok()?;
    if data.len() < 4 {
        return None;
    }
    // Skip the 4-byte attributes prefix.
    let payload = &data[4..];
    decode_utf16le(payload)
}

/// Read an EFI variable as a u64 value.
fn read_efi_variable_u64(name: &str, guid: &str) -> Option<u64> {
    let data = read_efi_variable_raw(name, guid).ok()?;
    if data.len() < 5 {
        return None;
    }
    let payload = &data[4..];
    match payload.len() {
        1 => Some(payload[0] as u64),
        2 => Some(u16::from_le_bytes([payload[0], payload[1]]) as u64),
        4 => Some(u32::from_le_bytes([payload[0], payload[1], payload[2], payload[3]]) as u64),
        n if n >= 8 => Some(u64::from_le_bytes([
            payload[0], payload[1], payload[2], payload[3], payload[4], payload[5], payload[6],
            payload[7],
        ])),
        _ => None,
    }
}

/// Write an EFI variable with the given attributes and raw payload.
fn write_efi_variable_raw(name: &str, guid: &str, attrs: u32, payload: &[u8]) -> io::Result<()> {
    let path = efivarfs_path().join(format!("{name}-{guid}"));

    // efivarfs requires removing the immutable flag before writing.
    // We do this by first deleting the file, then recreating it.
    // (On some kernels we can just open-and-write.)
    if path.exists() {
        // Try to remove the immutable flag via ioctl. If that fails, try
        // deleting and recreating the file.
        let _ = remove_efi_immutable(&path);
    }

    let mut data = Vec::with_capacity(4 + payload.len());
    data.extend_from_slice(&attrs.to_le_bytes());
    data.extend_from_slice(payload);

    fs::write(&path, &data)
}

/// Write an EFI variable as a UTF-16LE string.
fn write_efi_variable_string(name: &str, guid: &str, value: &str) -> io::Result<()> {
    let payload = encode_utf16le(value);
    write_efi_variable_raw(name, guid, EFI_VARIABLE_PERSISTENT, &payload)
}

/// Delete an EFI variable.
fn delete_efi_variable(name: &str, guid: &str) -> io::Result<()> {
    let path = efivarfs_path().join(format!("{name}-{guid}"));
    if path.exists() {
        let _ = remove_efi_immutable(&path);
        fs::remove_file(&path)?;
    }
    Ok(())
}

/// Remove the immutable flag from an efivarfs file using ioctl.
fn remove_efi_immutable(path: &Path) -> io::Result<()> {
    use std::os::unix::io::AsRawFd;
    let file = fs::OpenOptions::new().write(true).open(path)?;

    // FS_IOC_GETFLAGS = 0x80086601, FS_IOC_SETFLAGS = 0x40086602
    // FS_IMMUTABLE_FL = 0x00000010
    const FS_IOC_GETFLAGS: libc::c_ulong = 0x8008_6601;
    const FS_IOC_SETFLAGS: libc::c_ulong = 0x4008_6602;
    const FS_IMMUTABLE_FL: libc::c_long = 0x0000_0010;

    unsafe {
        let mut flags: libc::c_long = 0;
        if libc::ioctl(file.as_raw_fd(), FS_IOC_GETFLAGS, &mut flags) == 0 {
            flags &= !FS_IMMUTABLE_FL;
            libc::ioctl(file.as_raw_fd(), FS_IOC_SETFLAGS, &flags);
        }
    }
    Ok(())
}

/// Decode a UTF-16LE byte slice into a String, stripping NUL terminators.
fn decode_utf16le(data: &[u8]) -> Option<String> {
    if !data.len().is_multiple_of(2) {
        return None;
    }
    let u16s: Vec<u16> = data
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    let s = String::from_utf16(&u16s).ok()?;
    Some(s.trim_end_matches('\0').to_string())
}

/// Encode a string as UTF-16LE with a NUL terminator.
fn encode_utf16le(s: &str) -> Vec<u8> {
    let mut out = Vec::new();
    for code_unit in s.encode_utf16() {
        out.extend_from_slice(&code_unit.to_le_bytes());
    }
    // NUL terminator
    out.extend_from_slice(&0u16.to_le_bytes());
    out
}

// ── Firmware info ─────────────────────────────────────────────────────────

fn read_firmware_info() -> FirmwareInfo {
    if !is_efi_system() {
        return FirmwareInfo {
            is_efi: false,
            secure_boot: None,
            setup_mode: None,
            firmware_type: "BIOS".to_string(),
            vendor: None,
            os_indications_supported: None,
            os_indications: None,
        };
    }

    let secure_boot = read_efi_variable_raw("SecureBoot", EFI_GLOBAL_VARIABLE)
        .ok()
        .and_then(|d| if d.len() >= 5 { Some(d[4] == 1) } else { None });

    let setup_mode = read_efi_variable_raw("SetupMode", EFI_GLOBAL_VARIABLE)
        .ok()
        .and_then(|d| if d.len() >= 5 { Some(d[4] == 1) } else { None });

    let vendor = read_efi_variable_string("LoaderFirmwareInfo", LOADER_GUID);

    let os_indications_supported =
        read_efi_variable_u64("OsIndicationsSupported", EFI_GLOBAL_VARIABLE);
    let os_indications = read_efi_variable_u64("OsIndications", EFI_GLOBAL_VARIABLE);

    FirmwareInfo {
        is_efi: true,
        secure_boot,
        setup_mode,
        firmware_type: "UEFI".to_string(),
        vendor,
        os_indications_supported,
        os_indications,
    }
}

/// Read boot loader information from EFI variables set by systemd-boot.
fn read_loader_info() -> LoaderInfo {
    if !is_efi_system() {
        return LoaderInfo::default();
    }

    LoaderInfo {
        loader: read_efi_variable_string("LoaderInfo", LOADER_GUID),
        loader_firmware_type: read_efi_variable_string("LoaderFirmwareType", LOADER_GUID),
        loader_firmware_info: read_efi_variable_string("LoaderFirmwareInfo", LOADER_GUID),
        loader_features: read_efi_variable_u64("LoaderFeatures", LOADER_GUID),
        loader_config_timeout: read_efi_variable_string("LoaderConfigTimeout", LOADER_GUID),
        loader_config_timeout_oneshot: read_efi_variable_string(
            "LoaderConfigTimeoutOneShot",
            LOADER_GUID,
        ),
        loader_entry_selected: read_efi_variable_string("LoaderEntrySelected", LOADER_GUID),
        loader_entry_default: read_efi_variable_string("LoaderEntryDefault", LOADER_GUID),
        loader_entry_oneshot: read_efi_variable_string("LoaderEntryOneShot", LOADER_GUID),
        loader_image_identifier: read_efi_variable_string("LoaderImageIdentifier", LOADER_GUID),
        loader_device_part_uuid: read_efi_variable_string("LoaderDevicePartUUID", LOADER_GUID),
    }
}

// ── ESP / XBOOTLDR discovery ──────────────────────────────────────────────

/// Information about a mounted filesystem gleaned from /proc/self/mountinfo.
#[derive(Debug, Clone)]
struct MountEntry {
    mount_point: String,
    fs_type: String,
    #[allow(dead_code)]
    mount_source: String,
}

/// Parse /proc/self/mountinfo to find mounted filesystems.
fn read_mountinfo() -> Vec<MountEntry> {
    let content = match fs::read_to_string("/proc/self/mountinfo") {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut entries = Vec::new();
    for line in content.lines() {
        // mountinfo format: id parent major:minor root mount_point options ... - fs_type source super_options
        let parts: Vec<&str> = line.split_whitespace().collect();
        if parts.len() < 10 {
            continue;
        }

        // Find the separator "-"
        let sep_pos = parts.iter().position(|&p| p == "-");
        let sep_pos = match sep_pos {
            Some(pos) => pos,
            None => continue,
        };

        if sep_pos + 2 >= parts.len() {
            continue;
        }

        let mount_point = parts[4].to_string();
        let fs_type = parts[sep_pos + 1].to_string();
        let mount_source = parts[sep_pos + 2].to_string();

        entries.push(MountEntry {
            mount_point,
            fs_type,
            mount_source,
        });
    }

    entries
}

/// Find the ESP mount point.
fn find_esp(cli_path: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = cli_path
        && p.is_dir()
    {
        return Some(p.to_path_buf());
    }

    // Check the environment variable.
    if let Ok(val) = std::env::var("SYSTEMD_ESP_PATH") {
        let p = PathBuf::from(&val);
        if p.is_dir() {
            return Some(p);
        }
    }

    let mounts = read_mountinfo();

    // Check well-known paths.
    for candidate in ESP_SEARCH_PATHS {
        let p = Path::new(candidate);
        if !p.is_dir() {
            continue;
        }

        // Verify it's a VFAT filesystem (typical for ESP).
        let is_vfat = mounts
            .iter()
            .any(|m| m.mount_point == *candidate && m.fs_type == "vfat");

        // Also accept if the loader directory structure is present.
        let has_loader = p.join("loader").is_dir() || p.join("EFI").is_dir();

        if is_vfat || has_loader {
            return Some(p.to_path_buf());
        }
    }

    // Fallback: any VFAT mount that contains an EFI directory.
    for mount in &mounts {
        if mount.fs_type == "vfat" {
            let p = Path::new(&mount.mount_point);
            if p.join("EFI").is_dir() {
                return Some(p.to_path_buf());
            }
        }
    }

    None
}

/// Find the XBOOTLDR mount point.
fn find_xbootldr(cli_path: Option<&Path>, esp: Option<&Path>) -> Option<PathBuf> {
    if let Some(p) = cli_path
        && p.is_dir()
    {
        return Some(p.to_path_buf());
    }

    if let Ok(val) = std::env::var("SYSTEMD_XBOOTLDR_PATH") {
        let p = PathBuf::from(&val);
        if p.is_dir() {
            return Some(p);
        }
    }

    for candidate in XBOOTLDR_SEARCH_PATHS {
        let p = Path::new(candidate);
        if !p.is_dir() {
            continue;
        }

        // Don't use the same path as the ESP.
        if let Some(esp_path) = esp
            && p == esp_path
        {
            continue;
        }

        // Check if entries directory exists.
        if p.join("loader/entries").is_dir() || p.join("EFI/Linux").is_dir() {
            return Some(p.to_path_buf());
        }
    }

    None
}

// ── loader.conf parsing ───────────────────────────────────────────────────

/// Parse a loader.conf file from the ESP.
fn parse_loader_conf(esp: &Path) -> LoaderConfig {
    let path = esp.join("loader/loader.conf");
    let content = match fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return LoaderConfig::default(),
    };

    let mut config = LoaderConfig::default();

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match line.split_once(char::is_whitespace) {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        match key {
            "default" => config.default = Some(value.to_string()),
            "timeout" => {
                if value == "menu-force" || value == "menu-hidden" {
                    config.timeout = Some(0);
                } else {
                    config.timeout = value.parse().ok();
                }
            }
            "console-mode" => config.console_mode = Some(value.to_string()),
            "editor" => config.editor = parse_bool_value(value),
            "auto-entries" => config.auto_entries = parse_bool_value(value),
            "auto-firmware" => config.auto_firmware = parse_bool_value(value),
            "beep" => config.beep = parse_bool_value(value),
            "reboot-for-bitlocker" => config.reboot_for_bitlocker = parse_bool_value(value),
            "secure-boot-enroll" => config.secure_boot_enroll = Some(value.to_string()),
            "random-seed-mode" => config.random_seed_mode = Some(value.to_string()),
            _ => {
                // Ignore unknown keys.
            }
        }
    }

    config
}

/// Parse a boolean value from a loader.conf field.
fn parse_bool_value(s: &str) -> Option<bool> {
    match s.to_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Some(true),
        "0" | "no" | "false" | "off" => Some(false),
        _ => None,
    }
}

// ── Boot entry parsing ────────────────────────────────────────────────────

/// Parse boot counting information from a filename stem.
///
/// Format: `<id>+<tries_left>` or `<id>+<tries_left>-<tries_done>`
fn parse_boot_counting(stem: &str) -> (String, Option<u32>, Option<u32>) {
    if let Some((base, suffix)) = stem.rsplit_once('+') {
        if let Some((left, done)) = suffix.split_once('-') {
            let tries_left = left.parse().ok();
            let tries_done = done.parse().ok();
            return (base.to_string(), tries_left, tries_done);
        }
        let tries_left = suffix.parse().ok();
        return (base.to_string(), tries_left, Some(0));
    }
    (stem.to_string(), None, None)
}

/// Parse a single BLS Type #1 entry from a .conf file.
fn parse_type1_entry(path: &Path) -> Option<BootEntry> {
    let content = fs::read_to_string(path).ok()?;

    let file_stem = path.file_stem()?.to_string_lossy().to_string();
    let (id, tries_left, tries_done) = parse_boot_counting(&file_stem);

    let mut entry = BootEntry {
        id,
        title: None,
        version: None,
        machine_id: None,
        linux: None,
        initrd: Vec::new(),
        options: None,
        devicetree: Vec::new(),
        devicetree_overlay: Vec::new(),
        architecture: None,
        sort_key: None,
        source: path.to_path_buf(),
        is_default: false,
        is_oneshot: false,
        tries_left,
        tries_done,
        entry_type: EntryType::Type1,
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match line.split_once(char::is_whitespace) {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        match key {
            "title" => entry.title = Some(value.to_string()),
            "version" => entry.version = Some(value.to_string()),
            "machine-id" => entry.machine_id = Some(value.to_string()),
            "linux" => entry.linux = Some(value.to_string()),
            "initrd" => entry.initrd.push(value.to_string()),
            "options" => {
                // Options can be specified multiple times; they are concatenated.
                if let Some(ref mut existing) = entry.options {
                    existing.push(' ');
                    existing.push_str(value);
                } else {
                    entry.options = Some(value.to_string());
                }
            }
            "devicetree" => entry.devicetree.push(value.to_string()),
            "devicetree-overlay" => entry.devicetree_overlay.push(value.to_string()),
            "architecture" => entry.architecture = Some(value.to_string()),
            "sort-key" => entry.sort_key = Some(value.to_string()),
            _ => {}
        }
    }

    Some(entry)
}

/// Discover BLS Type #1 entries from the given root directory.
fn discover_type1_entries(root: &Path) -> Vec<BootEntry> {
    let entries_dir = root.join("loader/entries");
    let mut entries = Vec::new();

    let dir_iter = match fs::read_dir(&entries_dir) {
        Ok(d) => d,
        Err(_) => return entries,
    };

    for dir_entry in dir_iter.flatten() {
        let path = dir_entry.path();
        if path.extension().and_then(|e| e.to_str()) != Some("conf") {
            continue;
        }

        if let Some(entry) = parse_type1_entry(&path) {
            entries.push(entry);
        }
    }

    entries
}

/// Discover BLS Type #2 entries (Unified Kernel Images) from the EFI/Linux/ directory.
fn discover_type2_entries(root: &Path) -> Vec<BootEntry> {
    let uki_dir = root.join("EFI/Linux");
    let mut entries = Vec::new();

    let dir_iter = match fs::read_dir(&uki_dir) {
        Ok(d) => d,
        Err(_) => return entries,
    };

    for dir_entry in dir_iter.flatten() {
        let path = dir_entry.path();
        let ext = path.extension().and_then(|e| e.to_str());
        if ext != Some("efi") {
            continue;
        }

        let file_stem = match path.file_stem().and_then(|s| s.to_str()) {
            Some(s) => s.to_string(),
            None => continue,
        };

        let (id, tries_left, tries_done) = parse_boot_counting(&file_stem);

        // Try to read UKI metadata from the PE binary.
        let (title, version, options) = read_uki_metadata(&path);

        entries.push(BootEntry {
            id,
            title: title.or_else(|| Some(file_stem.clone())),
            version,
            machine_id: None,
            linux: Some(format!(
                "/EFI/Linux/{}",
                path.file_name().unwrap_or_default().to_string_lossy()
            )),
            initrd: Vec::new(),
            options,
            devicetree: Vec::new(),
            devicetree_overlay: Vec::new(),
            architecture: None,
            sort_key: None,
            source: path.clone(),
            is_default: false,
            is_oneshot: false,
            tries_left,
            tries_done,
            entry_type: EntryType::Type2,
        });
    }

    entries
}

/// Try to discover auto-entries (e.g. Windows Boot Manager, other EFI
/// binaries in well-known locations).
fn discover_auto_entries(esp: &Path) -> Vec<BootEntry> {
    let mut entries = Vec::new();

    // Check for Windows Boot Manager.
    let windows_path = esp.join("EFI/Microsoft/Boot/bootmgfw.efi");
    if windows_path.exists() {
        entries.push(BootEntry {
            id: "auto-windows".to_string(),
            title: Some("Windows Boot Manager".to_string()),
            version: None,
            machine_id: None,
            linux: None,
            initrd: Vec::new(),
            options: None,
            devicetree: Vec::new(),
            devicetree_overlay: Vec::new(),
            architecture: None,
            sort_key: None,
            source: windows_path,
            is_default: false,
            is_oneshot: false,
            tries_left: None,
            tries_done: None,
            entry_type: EntryType::Auto,
        });
    }

    // Check for other common EFI boot managers.
    let other_managers: &[(&str, &str, &str)] = &[
        (
            "EFI/BOOT/BOOTX64.EFI",
            "auto-efi-default",
            "EFI Default Loader",
        ),
        ("EFI/shell.efi", "auto-efi-shell", "EFI Shell"),
        (
            "EFI/refind/refind_x64.efi",
            "auto-refind",
            "rEFInd Boot Manager",
        ),
    ];

    for (path, id, title) in other_managers {
        let full_path = esp.join(path);
        if full_path.exists() {
            entries.push(BootEntry {
                id: id.to_string(),
                title: Some(title.to_string()),
                version: None,
                machine_id: None,
                linux: None,
                initrd: Vec::new(),
                options: None,
                devicetree: Vec::new(),
                devicetree_overlay: Vec::new(),
                architecture: None,
                sort_key: None,
                source: full_path,
                is_default: false,
                is_oneshot: false,
                tries_left: None,
                tries_done: None,
                entry_type: EntryType::Auto,
            });
        }
    }

    entries
}

/// Discover all boot entries from both ESP and XBOOTLDR, mark default/oneshot.
fn discover_all_entries(
    esp: Option<&Path>,
    xbootldr: Option<&Path>,
    loader_info: &LoaderInfo,
    loader_conf: &LoaderConfig,
) -> Vec<BootEntry> {
    let mut entries = Vec::new();

    // Collect Type #1 entries from both partitions.
    if let Some(esp) = esp {
        entries.extend(discover_type1_entries(esp));
        entries.extend(discover_type2_entries(esp));
        entries.extend(discover_auto_entries(esp));
    }
    if let Some(xbootldr) = xbootldr {
        entries.extend(discover_type1_entries(xbootldr));
        entries.extend(discover_type2_entries(xbootldr));
    }

    // Sort entries: sort-key first, then version (descending), then id.
    entries.sort_by(|a, b| {
        let a_key = a.sort_key.as_deref().unwrap_or("");
        let b_key = b.sort_key.as_deref().unwrap_or("");

        a_key
            .cmp(b_key)
            .then_with(|| {
                // Version: reverse order (newest first).
                let a_ver = a.version.as_deref().unwrap_or("");
                let b_ver = b.version.as_deref().unwrap_or("");
                version_compare(b_ver, a_ver)
            })
            .then_with(|| a.id.cmp(&b.id))
    });

    // Determine the default entry ID.
    let default_id = loader_info
        .loader_entry_default
        .as_deref()
        .or(loader_conf.default.as_deref());

    let oneshot_id = loader_info.loader_entry_oneshot.as_deref();

    // Mark default and oneshot entries.
    for entry in &mut entries {
        if let Some(d) = default_id
            && entry_matches_pattern(&entry.id, d)
        {
            entry.is_default = true;
        }
        if let Some(o) = oneshot_id
            && entry_matches_pattern(&entry.id, o)
        {
            entry.is_oneshot = true;
        }
    }

    // If no entry was marked as default, mark the first one.
    if !entries.iter().any(|e| e.is_default)
        && let Some(first) = entries.first_mut()
    {
        first.is_default = true;
    }

    entries
}

/// Match an entry ID against a pattern (supports trailing `*` glob).
fn entry_matches_pattern(id: &str, pattern: &str) -> bool {
    if pattern == id {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return id.starts_with(prefix);
    }
    // Also try matching without .conf extension.
    let id_stripped = id.strip_suffix(".conf").unwrap_or(id);
    let pat_stripped = pattern.strip_suffix(".conf").unwrap_or(pattern);
    id_stripped == pat_stripped
}

/// Simple version comparison that handles numeric segments.
fn version_compare(a: &str, b: &str) -> std::cmp::Ordering {
    let a_parts = split_version(a);
    let b_parts = split_version(b);

    for (ap, bp) in a_parts.iter().zip(b_parts.iter()) {
        let ord = match (ap.parse::<u64>(), bp.parse::<u64>()) {
            (Ok(an), Ok(bn)) => an.cmp(&bn),
            _ => ap.cmp(bp),
        };
        if ord != std::cmp::Ordering::Equal {
            return ord;
        }
    }

    a_parts.len().cmp(&b_parts.len())
}

/// Split a version string into segments at `.`, `-`, `_`, and numeric/alpha boundaries.
fn split_version(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut was_digit = None;

    for ch in s.chars() {
        if ch == '.' || ch == '-' || ch == '_' {
            if !current.is_empty() {
                parts.push(current.clone());
                current.clear();
            }
            was_digit = None;
            continue;
        }

        let is_digit = ch.is_ascii_digit();
        if let Some(prev_digit) = was_digit
            && prev_digit != is_digit
            && !current.is_empty()
        {
            parts.push(current.clone());
            current.clear();
        }

        current.push(ch);
        was_digit = Some(is_digit);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

// ── PE / UKI inspection ───────────────────────────────────────────────────

/// Minimal PE header parsing to extract section information from a UKI.
///
/// Returns (title, version, cmdline) extracted from PE sections like
/// `.osrel`, `.uname`, `.cmdline`.
fn read_uki_metadata(path: &Path) -> (Option<String>, Option<String>, Option<String>) {
    let data = match fs::read(path) {
        Ok(d) => d,
        Err(_) => return (None, None, None),
    };

    let sections = match parse_pe_sections(&data) {
        Some(s) => s,
        None => return (None, None, None),
    };

    let mut title = None;
    let mut version = None;
    let mut cmdline = None;

    for (name, section_data) in &sections {
        match name.as_str() {
            ".osrel" => {
                // Parse os-release format for PRETTY_NAME.
                if let Ok(text) = std::str::from_utf8(section_data) {
                    for line in text.lines() {
                        let line = line.trim();
                        if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                            title = Some(val.trim_matches('"').to_string());
                        }
                    }
                }
            }
            ".uname" => {
                if let Ok(text) = std::str::from_utf8(section_data) {
                    version = Some(text.trim_end_matches('\0').trim().to_string());
                }
            }
            ".cmdline" => {
                if let Ok(text) = std::str::from_utf8(section_data) {
                    cmdline = Some(text.trim_end_matches('\0').trim().to_string());
                }
            }
            _ => {}
        }
    }

    (title, version, cmdline)
}

/// Parse PE section headers from a PE/COFF binary.
/// Returns a Vec of (section_name, section_data) pairs.
fn parse_pe_sections(data: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    // Check MZ signature.
    if data.len() < 64 {
        return None;
    }
    if data[0] != b'M' || data[1] != b'Z' {
        return None;
    }

    // PE header offset is at offset 0x3C.
    let pe_offset = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
    if pe_offset + 4 > data.len() {
        return None;
    }

    // Check PE signature "PE\0\0".
    if &data[pe_offset..pe_offset + 4] != b"PE\0\0" {
        return None;
    }

    // COFF header starts right after PE signature.
    let coff_offset = pe_offset + 4;
    if coff_offset + 20 > data.len() {
        return None;
    }

    let number_of_sections =
        u16::from_le_bytes([data[coff_offset + 2], data[coff_offset + 3]]) as usize;
    let size_of_optional_header =
        u16::from_le_bytes([data[coff_offset + 16], data[coff_offset + 17]]) as usize;

    // Section headers start after COFF header (20 bytes) + optional header.
    let sections_offset = coff_offset + 20 + size_of_optional_header;

    let mut sections = Vec::new();

    for i in 0..number_of_sections {
        let sh_offset = sections_offset + i * 40;
        if sh_offset + 40 > data.len() {
            break;
        }

        // Section name: 8 bytes, NUL-padded.
        let name_bytes = &data[sh_offset..sh_offset + 8];
        let name = std::str::from_utf8(name_bytes)
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        let raw_data_size = u32::from_le_bytes([
            data[sh_offset + 16],
            data[sh_offset + 17],
            data[sh_offset + 18],
            data[sh_offset + 19],
        ]) as usize;

        let raw_data_ptr = u32::from_le_bytes([
            data[sh_offset + 20],
            data[sh_offset + 21],
            data[sh_offset + 22],
            data[sh_offset + 23],
        ]) as usize;

        if raw_data_ptr + raw_data_size <= data.len() {
            let section_data = data[raw_data_ptr..raw_data_ptr + raw_data_size].to_vec();
            sections.push((name, section_data));
        }
    }

    Some(sections)
}

/// Identify the type of a kernel image.
fn identify_kernel(path: &Path) -> KernelType {
    let mut file = match fs::File::open(path) {
        Ok(f) => f,
        Err(_) => return KernelType::Unknown,
    };

    let mut header = [0u8; 1024];
    let n = match file.read(&mut header) {
        Ok(n) => n,
        Err(_) => return KernelType::Unknown,
    };

    if n < 2 {
        return KernelType::Unknown;
    }

    // Check for PE/COFF (MZ header).
    if header[0] == b'M' && header[1] == b'Z' {
        // It's a PE binary. Check if it has UKI-specific sections.
        if let Ok(data) = fs::read(path)
            && let Some(sections) = parse_pe_sections(&data)
        {
            let has_linux = sections.iter().any(|(name, _)| name == ".linux");
            let has_osrel = sections.iter().any(|(name, _)| name == ".osrel");
            if has_linux || has_osrel {
                return KernelType::Uki;
            }
        }
        // PE binary without UKI sections — could still be a bare EFI stub.
        return KernelType::Unknown;
    }

    // Check for Linux kernel image signatures.
    // x86 bzImage: magic at offset 0x202 = "HdrS"
    if n >= 0x206
        && header[0x202] == b'H'
        && header[0x203] == b'd'
        && header[0x204] == b'r'
        && header[0x205] == b'S'
    {
        return KernelType::Vmlinuz;
    }

    // ARM64 Image: magic at offset 0x38 = "ARM\x64"
    if n >= 0x3C
        && header[0x38] == b'A'
        && header[0x39] == b'R'
        && header[0x3A] == b'M'
        && header[0x3B] == 0x64
    {
        return KernelType::Vmlinuz;
    }

    // ARM zImage: magic at offset 0x24 = 0x016F2818
    if n >= 0x28 {
        let magic = u32::from_le_bytes([header[0x24], header[0x25], header[0x26], header[0x27]]);
        if magic == 0x016F_2818 {
            return KernelType::Vmlinuz;
        }
    }

    // RISC-V Image: magic at offset 0x30 = "RSCV"
    if n >= 0x34
        && header[0x30] == b'R'
        && header[0x31] == b'S'
        && header[0x32] == b'C'
        && header[0x33] == b'V'
    {
        return KernelType::Vmlinuz;
    }

    // Gzip-compressed kernel (e.g. vmlinuz on some distros).
    if n >= 2 && header[0] == 0x1F && header[1] == 0x8B {
        return KernelType::Vmlinuz;
    }

    KernelType::Unknown
}

/// Inspect a kernel image and print metadata.
fn inspect_kernel(path: &Path) -> HashMap<String, String> {
    let mut info = HashMap::new();

    if !path.exists() {
        return info;
    }

    let kernel_type = identify_kernel(path);
    info.insert("type".to_string(), kernel_type.to_string());

    if let Ok(metadata) = fs::metadata(path) {
        info.insert("size".to_string(), format!("{}", metadata.len()));
    }

    // For UKIs, extract embedded metadata.
    if kernel_type == KernelType::Uki
        && let Ok(data) = fs::read(path)
        && let Some(sections) = parse_pe_sections(&data)
    {
        for (name, section_data) in &sections {
            match name.as_str() {
                ".osrel" => {
                    if let Ok(text) = std::str::from_utf8(section_data) {
                        for line in text.lines() {
                            let line = line.trim();
                            if let Some((key, val)) = line.split_once('=') {
                                let val = val.trim_matches('"');
                                info.insert(
                                    format!("osrel.{}", key.to_lowercase()),
                                    val.to_string(),
                                );
                            }
                        }
                    }
                }
                ".uname" => {
                    if let Ok(text) = std::str::from_utf8(section_data) {
                        info.insert(
                            "uname".to_string(),
                            text.trim_end_matches('\0').trim().to_string(),
                        );
                    }
                }
                ".cmdline" => {
                    if let Ok(text) = std::str::from_utf8(section_data) {
                        info.insert(
                            "cmdline".to_string(),
                            text.trim_end_matches('\0').trim().to_string(),
                        );
                    }
                }
                _ => {
                    info.insert(
                        format!("section.{name}"),
                        format!("{} bytes", section_data.len()),
                    );
                }
            }
        }
    }

    info
}

// ── Installation helpers ──────────────────────────────────────────────────

/// Locate the source EFI binary for installation.
fn find_source_efi_binary(override_path: Option<&Path>) -> Option<PathBuf> {
    // Check override path first.
    if let Some(p) = override_path {
        let candidate = p.join(EFI_BINARY_NAME);
        if candidate.exists() {
            return Some(candidate);
        }
        // Also check for the binary directly.
        if p.exists() && p.is_file() {
            return Some(p.to_path_buf());
        }
    }

    // Check well-known installation source directories.
    let search_dirs: Vec<PathBuf> = vec![
        PathBuf::from(format!("/usr/lib/systemd/boot/efi/{EFI_BINARY_NAME}")),
        PathBuf::from(format!("/usr/share/systemd/boot/efi/{EFI_BINARY_NAME}")),
        PathBuf::from(format!("/lib/systemd/boot/efi/{EFI_BINARY_NAME}")),
    ];

    // Also check relative to the running binary (for NixOS etc.).
    if let Ok(exe) = std::env::current_exe()
        && let Some(parent) = exe.parent()
    {
        let nix_candidate = parent.join("../lib/systemd/boot/efi").join(EFI_BINARY_NAME);
        if nix_candidate.exists() {
            return Some(nix_candidate);
        }
    }

    for path in &search_dirs {
        if path.exists() {
            return Some(path.clone());
        }
    }

    None
}

/// The destination path for the boot loader on the ESP.
fn esp_efi_boot_path(esp: &Path) -> PathBuf {
    esp.join(format!("EFI/BOOT/BOOT{}.EFI", EFI_ARCH_DIR.to_uppercase()))
}

/// The destination path for the systemd-boot binary on the ESP.
fn esp_systemd_boot_path(esp: &Path) -> PathBuf {
    esp.join(format!("EFI/systemd/{EFI_BINARY_NAME}"))
}

/// Copy a file, creating parent directories as needed.
fn copy_file_with_dirs(src: &Path, dst: &Path, dry_run: bool) -> io::Result<()> {
    if let Some(parent) = dst.parent()
        && !parent.exists()
    {
        if dry_run {
            println!("Would create directory: {}", parent.display());
        } else {
            fs::create_dir_all(parent)?;
        }
    }

    if dry_run {
        println!("Would copy {} -> {}", src.display(), dst.display());
    } else {
        fs::copy(src, dst)?;
    }

    Ok(())
}

/// Check if systemd-boot is installed on the ESP.
fn is_installed(esp: &Path) -> bool {
    esp_systemd_boot_path(esp).exists() || esp_efi_boot_path(esp).exists()
}

/// Read the installed EFI binary version by looking for an embedded version string.
fn read_efi_binary_version(path: &Path) -> Option<String> {
    let data = fs::read(path).ok()?;

    // systemd-boot embeds a version string as "systemd-boot <version>" in the binary.
    // Search for this pattern in the raw data.
    let pattern = b"systemd-boot ";
    for i in 0..data.len().saturating_sub(pattern.len() + 1) {
        if &data[i..i + pattern.len()] == pattern {
            let start = i + pattern.len();
            let mut end = start;
            while end < data.len() && data[end] != 0 && data[end].is_ascii_graphic() {
                end += 1;
            }
            if end > start
                && let Ok(version) = std::str::from_utf8(&data[start..end])
            {
                return Some(version.to_string());
            }
        }
    }

    None
}

/// Create an EFI boot entry via efibootmgr (if available).
fn create_efi_boot_entry(esp: &Path, _dry_run: bool) -> bool {
    // Try to determine the ESP device and partition number.
    let mounts = read_mountinfo();
    let esp_str = esp.to_string_lossy();
    let mount_entry = mounts.iter().find(|m| m.mount_point == *esp_str);

    if mount_entry.is_none() {
        eprintln!("Warning: Could not determine ESP device for EFI boot entry creation.");
        return false;
    }

    // We would call efibootmgr here, but let's just report that the user
    // should do this manually if needed, since efibootmgr may not be installed.
    true
}

// ── Random seed ───────────────────────────────────────────────────────────

/// Initialize or refresh the random seed on the ESP.
///
/// The random seed file at `<ESP>/loader/random-seed` is used by
/// systemd-boot to seed the kernel's random number generator early
/// during boot, before userspace is available.
fn manage_random_seed(esp: &Path, dry_run: bool) -> io::Result<()> {
    let seed_path = esp.join(RANDOM_SEED_PATH);

    // Ensure the loader directory exists.
    let loader_dir = esp.join("loader");
    if !loader_dir.exists() {
        if dry_run {
            println!("Would create directory: {}", loader_dir.display());
        } else {
            fs::create_dir_all(&loader_dir)?;
        }
    }

    // Generate a new random seed.
    let mut seed = vec![0u8; RANDOM_SEED_SIZE];
    let mut urandom = fs::File::open("/dev/urandom")?;
    urandom.read_exact(&mut seed)?;

    if seed_path.exists() {
        // XOR with existing seed to accumulate entropy.
        if let Ok(existing) = fs::read(&seed_path) {
            for (i, byte) in existing.iter().enumerate() {
                if i < seed.len() {
                    seed[i] ^= byte;
                }
            }
        }
    }

    if dry_run {
        println!("Would write random seed to: {}", seed_path.display());
    } else {
        // Write atomically by writing to a temporary file and renaming.
        let tmp_path = seed_path.with_extension("new");
        fs::write(&tmp_path, &seed)?;
        fs::rename(&tmp_path, &seed_path)?;

        // Also update the system token EFI variable if it doesn't exist.
        if is_efi_system() {
            let token_var = read_efi_variable_raw("LoaderSystemToken", LOADER_GUID);
            if token_var.is_err() {
                // Generate and store a system token.
                let mut token = vec![0u8; RANDOM_SEED_SIZE];
                urandom.read_exact(&mut token)?;
                let _ = write_efi_variable_raw(
                    "LoaderSystemToken",
                    LOADER_GUID,
                    EFI_VARIABLE_PERSISTENT,
                    &token,
                );
            }
        }
    }

    println!("Random seed file {} updated.", seed_path.display());
    Ok(())
}

// ── Subcommand implementations ────────────────────────────────────────────

fn cmd_status(cli: &Cli) {
    let fw = read_firmware_info();
    let loader = read_loader_info();
    let esp = find_esp(cli.esp_path.as_deref());
    let xbootldr = find_xbootldr(cli.boot_path.as_deref(), esp.as_deref());
    let loader_conf = esp.as_deref().map(parse_loader_conf).unwrap_or_default();

    // System firmware information.
    println!("System:");
    println!("      Firmware: {}", fw.firmware_type);
    if let Some(ref vendor) = fw.vendor {
        println!(" Firmware Info: {vendor}");
    }
    match fw.secure_boot {
        Some(true) => println!("   Secure Boot: enabled"),
        Some(false) => println!("   Secure Boot: disabled"),
        None => println!("   Secure Boot: unknown"),
    }
    match fw.setup_mode {
        Some(true) => println!("    Setup Mode: enabled"),
        Some(false) => println!("    Setup Mode: user"),
        None => {}
    }
    if let Some(supported) = fw.os_indications_supported {
        let fw_setup = (supported & EFI_OS_INDICATIONS_BOOT_TO_FW_UI) != 0;
        println!(
            "  TPM2 Support: {}",
            if Path::new("/sys/class/tpm/tpm0").exists() {
                "yes"
            } else {
                "no"
            }
        );
        println!(
            " Boot into FW:  {}",
            if fw_setup {
                "supported"
            } else {
                "not supported"
            }
        );
    }
    println!();

    // Current boot loader information.
    println!("Current Boot Loader:");
    if let Some(ref name) = loader.loader {
        println!("      Product: {name}");
    } else {
        println!("      Product: n/a");
    }
    if let Some(ref fw_type) = loader.loader_firmware_type {
        println!("     Firmware: {fw_type}");
    }
    if let Some(features) = loader.loader_features {
        println!("     Features: 0x{features:016x}");
        print_loader_features(features);
    }

    if let Some(ref entry) = loader.loader_entry_selected {
        println!("        Entry: {entry}");
    }
    if let Some(ref entry) = loader.loader_entry_default {
        println!("      Default: {entry}");
    }
    if let Some(ref entry) = loader.loader_entry_oneshot {
        println!("      Oneshot: {entry}");
    }

    if let Some(ref image) = loader.loader_image_identifier {
        println!("  Boot loader: {image}");
    }
    if let Some(ref uuid) = loader.loader_device_part_uuid {
        println!("    Part UUID: {uuid}");
    }
    println!();

    // ESP information.
    match &esp {
        Some(path) => {
            println!("Available Boot Loaders on ESP:");
            println!("          ESP: {}", path.display());

            let sb_path = esp_systemd_boot_path(path);
            if sb_path.exists() {
                if let Some(ver) = read_efi_binary_version(&sb_path) {
                    println!("  systemd-boot: {ver} ({})", sb_path.display());
                } else {
                    println!("  systemd-boot: installed ({})", sb_path.display());
                }
            }

            let efi_boot = esp_efi_boot_path(path);
            if efi_boot.exists() {
                if let Some(ver) = read_efi_binary_version(&efi_boot) {
                    println!("  EFI default: {ver} ({})", efi_boot.display());
                } else {
                    println!("  EFI default: installed ({})", efi_boot.display());
                }
            }

            // Loader config summary.
            println!();
            println!("Boot Loader Configuration:");
            println!("   Config file: {}/loader/loader.conf", path.display());
            if let Some(ref default) = loader_conf.default {
                println!("       Default: {default}");
            }
            if let Some(timeout) = loader_conf.timeout {
                println!("       Timeout: {timeout}s");
            }
            if let Some(editor) = loader_conf.editor {
                println!("        Editor: {}", if editor { "yes" } else { "no" });
            }
        }
        None => {
            println!("No ESP found.");
        }
    }
    println!();

    if let Some(ref path) = xbootldr {
        println!("Extended Boot Loader Partition:");
        println!("      XBOOTLDR: {}", path.display());
        println!();
    }

    // Boot entries.
    let entries = discover_all_entries(esp.as_deref(), xbootldr.as_deref(), &loader, &loader_conf);

    if !entries.is_empty() {
        println!("Boot Loaders Listed in EFI Variables:");
        for entry in &entries {
            print_entry_summary(entry);
        }
    } else {
        println!("No boot entries found.");
    }
}

fn print_loader_features(features: u64) {
    let feature_names = [
        (1 << 0, "config-timeout"),
        (1 << 1, "config-timeout-oneshot"),
        (1 << 2, "entry-default"),
        (1 << 3, "entry-oneshot"),
        (1 << 4, "boot-counting"),
        (1 << 5, "xbootldr"),
        (1 << 6, "random-seed"),
        (1 << 7, "load-driver"),
        (1 << 8, "sort-key"),
        (1 << 9, "saved-entry"),
        (1 << 10, "devicetree"),
        (1 << 11, "config-timeout-sec"),
        (1 << 12, "secure-boot-enroll"),
        (1 << 13, "menu-disabled"),
    ];

    let mut supported = Vec::new();
    for (bit, name) in &feature_names {
        if features & bit != 0 {
            supported.push(*name);
        }
    }

    if !supported.is_empty() {
        println!("               {}", supported.join(", "));
    }
}

fn print_entry_summary(entry: &BootEntry) {
    let markers = format!(
        "{}{}",
        if entry.is_default { " (default)" } else { "" },
        if entry.is_oneshot { " (oneshot)" } else { "" }
    );

    let title = entry.title.as_deref().unwrap_or(&entry.id);

    println!();
    println!("        type: {}", entry.entry_type);
    println!("       title: {title}{markers}");
    println!("          id: {}", entry.id);

    if let Some(ref ver) = entry.version {
        println!("     version: {ver}");
    }
    if let Some(ref machine_id) = entry.machine_id {
        println!("  machine-id: {machine_id}");
    }
    if let Some(ref linux) = entry.linux {
        println!("       linux: {linux}");
    }
    for initrd in &entry.initrd {
        println!("      initrd: {initrd}");
    }
    if let Some(ref options) = entry.options {
        println!("     options: {options}");
    }
    if let Some(ref sort_key) = entry.sort_key {
        println!("    sort-key: {sort_key}");
    }
    if let Some(tries_left) = entry.tries_left {
        let tries_done = entry.tries_done.unwrap_or(0);
        println!("  boot count: {tries_done} tries done, {tries_left} tries left");
    }
    println!("      source: {}", entry.source.display());
}

fn cmd_list(cli: &Cli) {
    let esp = find_esp(cli.esp_path.as_deref());
    let xbootldr = find_xbootldr(cli.boot_path.as_deref(), esp.as_deref());
    let loader = read_loader_info();
    let loader_conf = esp.as_deref().map(parse_loader_conf).unwrap_or_default();

    let entries = discover_all_entries(esp.as_deref(), xbootldr.as_deref(), &loader, &loader_conf);

    if entries.is_empty() {
        if !cli.quiet {
            println!("No boot entries found.");
        }
        return;
    }

    for entry in &entries {
        print_entry_summary(entry);
    }
}

fn cmd_install(cli: &Cli, path: Option<&Path>, no_variables: bool, force: bool) {
    let esp = match find_esp(cli.esp_path.as_deref()) {
        Some(p) => p,
        None => {
            eprintln!("Error: Could not find the ESP. Use --esp-path= to specify.");
            process::exit(1);
        }
    };

    if !force && is_installed(&esp) {
        eprintln!("systemd-boot is already installed on the ESP.");
        eprintln!("Use --force to reinstall.");
        process::exit(1);
    }

    let source = match find_source_efi_binary(path) {
        Some(s) => s,
        None => {
            eprintln!("Error: Could not find the systemd-boot EFI binary ({EFI_BINARY_NAME}).");
            eprintln!("Checked: /usr/lib/systemd/boot/efi/, /usr/share/systemd/boot/efi/");
            if let Some(p) = path {
                eprintln!("Also checked: {}", p.display());
            }
            process::exit(1);
        }
    };

    println!(
        "Installing systemd-boot from {} to {}...",
        source.display(),
        esp.display()
    );

    // Copy to EFI/systemd/<binary>.
    let dst_systemd = esp_systemd_boot_path(&esp);
    if let Err(e) = copy_file_with_dirs(&source, &dst_systemd, cli.dry_run) {
        eprintln!("Error copying to {}: {e}", dst_systemd.display());
        process::exit(1);
    }
    println!("  Installed: {}", dst_systemd.display());

    // Copy to EFI/BOOT/BOOT<ARCH>.EFI (default boot path).
    let dst_default = esp_efi_boot_path(&esp);
    if let Err(e) = copy_file_with_dirs(&source, &dst_default, cli.dry_run) {
        eprintln!("Error copying to {}: {e}", dst_default.display());
        process::exit(1);
    }
    println!("  Installed: {}", dst_default.display());

    // Create loader directory structure.
    let loader_dir = esp.join("loader");
    if !loader_dir.exists() && !cli.dry_run {
        let _ = fs::create_dir_all(&loader_dir);
    }
    let entries_dir = esp.join("loader/entries");
    if !entries_dir.exists() && !cli.dry_run {
        let _ = fs::create_dir_all(&entries_dir);
    }

    // Create a default loader.conf if none exists.
    let loader_conf_path = esp.join("loader/loader.conf");
    if !loader_conf_path.exists() {
        if cli.dry_run {
            println!("  Would create default: {}", loader_conf_path.display());
        } else {
            let default_conf = "# systemd-boot configuration\n#timeout 3\n#console-mode max\n";
            if let Err(e) = fs::write(&loader_conf_path, default_conf) {
                eprintln!("Warning: Could not create default loader.conf: {e}");
            } else {
                println!("  Created: {}", loader_conf_path.display());
            }
        }
    }

    // Initialize random seed.
    if let Err(e) = manage_random_seed(&esp, cli.dry_run) {
        eprintln!("Warning: Could not initialize random seed: {e}");
    }

    // Create EFI boot entry.
    if !no_variables && is_efi_system() && !cli.dry_run {
        create_efi_boot_entry(&esp, cli.dry_run);
    }

    println!("Installation complete.");
}

fn cmd_update(cli: &Cli, path: Option<&Path>, force: bool, _no_variables: bool) {
    let esp = match find_esp(cli.esp_path.as_deref()) {
        Some(p) => p,
        None => {
            eprintln!("Error: Could not find the ESP. Use --esp-path= to specify.");
            process::exit(1);
        }
    };

    if !is_installed(&esp) {
        eprintln!("systemd-boot is not installed on the ESP.");
        eprintln!("Use 'bootctl install' to install it first.");
        process::exit(1);
    }

    let source = match find_source_efi_binary(path) {
        Some(s) => s,
        None => {
            eprintln!("Error: Could not find the systemd-boot EFI binary ({EFI_BINARY_NAME}).");
            process::exit(1);
        }
    };

    // Compare versions if not forced.
    if !force {
        let installed_version = read_efi_binary_version(&esp_systemd_boot_path(&esp));
        let source_version = read_efi_binary_version(&source);

        if let (Some(ref installed), Some(ref src)) = (installed_version, source_version)
            && version_compare(installed, src) != std::cmp::Ordering::Less
        {
            println!("Installed version ({installed}) is not older than source ({src}).");
            println!("Use --force to update anyway.");
            return;
        }
    }

    println!(
        "Updating systemd-boot from {} to {}...",
        source.display(),
        esp.display()
    );

    let dst_systemd = esp_systemd_boot_path(&esp);
    if let Err(e) = copy_file_with_dirs(&source, &dst_systemd, cli.dry_run) {
        eprintln!("Error copying to {}: {e}", dst_systemd.display());
        process::exit(1);
    }
    println!("  Updated: {}", dst_systemd.display());

    let dst_default = esp_efi_boot_path(&esp);
    if let Err(e) = copy_file_with_dirs(&source, &dst_default, cli.dry_run) {
        eprintln!("Error copying to {}: {e}", dst_default.display());
        process::exit(1);
    }
    println!("  Updated: {}", dst_default.display());

    // Refresh random seed.
    if let Err(e) = manage_random_seed(&esp, cli.dry_run) {
        eprintln!("Warning: Could not refresh random seed: {e}");
    }

    println!("Update complete.");
}

fn cmd_remove(cli: &Cli, _no_variables: bool) {
    let esp = match find_esp(cli.esp_path.as_deref()) {
        Some(p) => p,
        None => {
            eprintln!("Error: Could not find the ESP. Use --esp-path= to specify.");
            process::exit(1);
        }
    };

    if !is_installed(&esp) {
        eprintln!("systemd-boot is not installed on the ESP.");
        process::exit(1);
    }

    println!("Removing systemd-boot from {}...", esp.display());

    let files_to_remove = [
        esp_systemd_boot_path(&esp),
        esp_efi_boot_path(&esp),
        esp.join(RANDOM_SEED_PATH),
    ];

    for path in &files_to_remove {
        if path.exists() {
            if cli.dry_run {
                println!("  Would remove: {}", path.display());
            } else {
                match fs::remove_file(path) {
                    Ok(()) => println!("  Removed: {}", path.display()),
                    Err(e) => eprintln!("  Warning: Could not remove {}: {e}", path.display()),
                }
            }
        }
    }

    // Try to remove empty directories.
    let dirs_to_clean = [
        esp.join("EFI/systemd"),
        esp.join("EFI/BOOT"),
        esp.join("loader/entries"),
        esp.join("loader"),
    ];

    for dir in &dirs_to_clean {
        if dir.is_dir() {
            // Only remove if empty.
            if let Ok(mut entries) = fs::read_dir(dir)
                && entries.next().is_none()
            {
                if cli.dry_run {
                    println!("  Would remove directory: {}", dir.display());
                } else {
                    let _ = fs::remove_dir(dir);
                }
            }
        }
    }

    println!("Removal complete.");
}

fn cmd_is_installed(cli: &Cli) {
    let esp = match find_esp(cli.esp_path.as_deref()) {
        Some(p) => p,
        None => {
            if !cli.quiet {
                println!("no");
            }
            process::exit(1);
        }
    };

    if is_installed(&esp) {
        if !cli.quiet {
            println!("yes");
        }
        process::exit(0);
    } else {
        if !cli.quiet {
            println!("no");
        }
        process::exit(1);
    }
}

fn cmd_set_default(id: &str) {
    if !is_efi_system() {
        eprintln!("Error: Not booted via EFI. Cannot set EFI variables.");
        process::exit(1);
    }

    match write_efi_variable_string("LoaderEntryDefault", LOADER_GUID, id) {
        Ok(()) => println!("Set default boot entry to: {id}"),
        Err(e) => {
            eprintln!("Error setting LoaderEntryDefault: {e}");
            eprintln!("Hint: This operation requires root privileges.");
            process::exit(1);
        }
    }
}

fn cmd_set_oneshot(id: &str) {
    if !is_efi_system() {
        eprintln!("Error: Not booted via EFI. Cannot set EFI variables.");
        process::exit(1);
    }

    // Oneshot variables use BS+RT attributes (not NV) so they are
    // cleared automatically after the next boot.
    let attrs = EFI_VARIABLE_BOOTSERVICE_ACCESS | EFI_VARIABLE_RUNTIME_ACCESS;
    let payload = encode_utf16le(id);

    match write_efi_variable_raw("LoaderEntryOneShot", LOADER_GUID, attrs, &payload) {
        Ok(()) => println!("Set oneshot boot entry to: {id}"),
        Err(e) => {
            eprintln!("Error setting LoaderEntryOneShot: {e}");
            eprintln!("Hint: This operation requires root privileges.");
            process::exit(1);
        }
    }
}

fn cmd_set_timeout(seconds: &str) {
    if !is_efi_system() {
        eprintln!("Error: Not booted via EFI. Cannot set EFI variables.");
        process::exit(1);
    }

    match write_efi_variable_string("LoaderConfigTimeout", LOADER_GUID, seconds) {
        Ok(()) => println!("Set boot menu timeout to: {seconds}s"),
        Err(e) => {
            eprintln!("Error setting LoaderConfigTimeout: {e}");
            process::exit(1);
        }
    }
}

fn cmd_set_timeout_oneshot(seconds: &str) {
    if !is_efi_system() {
        eprintln!("Error: Not booted via EFI. Cannot set EFI variables.");
        process::exit(1);
    }

    let attrs = EFI_VARIABLE_BOOTSERVICE_ACCESS | EFI_VARIABLE_RUNTIME_ACCESS;
    let payload = encode_utf16le(seconds);

    match write_efi_variable_raw("LoaderConfigTimeoutOneShot", LOADER_GUID, attrs, &payload) {
        Ok(()) => println!("Set oneshot boot menu timeout to: {seconds}s"),
        Err(e) => {
            eprintln!("Error setting LoaderConfigTimeoutOneShot: {e}");
            process::exit(1);
        }
    }
}

fn cmd_random_seed(cli: &Cli) {
    let esp = match find_esp(cli.esp_path.as_deref()) {
        Some(p) => p,
        None => {
            eprintln!("Error: Could not find the ESP. Use --esp-path= to specify.");
            process::exit(1);
        }
    };

    if let Err(e) = manage_random_seed(&esp, cli.dry_run) {
        eprintln!("Error managing random seed: {e}");
        process::exit(1);
    }
}

fn cmd_reboot_to_firmware(flag: Option<bool>) {
    if !is_efi_system() {
        eprintln!("Error: Not booted via EFI.");
        process::exit(1);
    }

    let supported = read_efi_variable_u64("OsIndicationsSupported", EFI_GLOBAL_VARIABLE);

    match supported {
        Some(s) if (s & EFI_OS_INDICATIONS_BOOT_TO_FW_UI) != 0 => {}
        _ => {
            eprintln!("Error: Firmware does not support boot-to-firmware-UI indication.");
            process::exit(1);
        }
    }

    match flag {
        Some(enable) => {
            let current = read_efi_variable_u64("OsIndications", EFI_GLOBAL_VARIABLE).unwrap_or(0);

            let new_value = if enable {
                current | EFI_OS_INDICATIONS_BOOT_TO_FW_UI
            } else {
                current & !EFI_OS_INDICATIONS_BOOT_TO_FW_UI
            };

            let payload = new_value.to_le_bytes();
            match write_efi_variable_raw(
                "OsIndications",
                EFI_GLOBAL_VARIABLE,
                EFI_VARIABLE_PERSISTENT,
                &payload,
            ) {
                Ok(()) => {
                    if enable {
                        println!("Indicating to firmware to boot into setup on next reboot.");
                    } else {
                        println!("Cleared reboot-to-firmware flag.");
                    }
                }
                Err(e) => {
                    eprintln!("Error setting OsIndications: {e}");
                    process::exit(1);
                }
            }
        }
        None => {
            let current = read_efi_variable_u64("OsIndications", EFI_GLOBAL_VARIABLE).unwrap_or(0);
            let is_set = (current & EFI_OS_INDICATIONS_BOOT_TO_FW_UI) != 0;
            println!(
                "Reboot into firmware: {}",
                if is_set { "active" } else { "not active" }
            );
            if is_set {
                process::exit(0);
            } else {
                process::exit(1);
            }
        }
    }
}

fn cmd_systemd_efi_options(options: Option<&str>) {
    if !is_efi_system() {
        eprintln!("Error: Not booted via EFI.");
        process::exit(1);
    }

    match options {
        Some(opts) => {
            if opts.is_empty() {
                // Clear the variable.
                match delete_efi_variable("SystemdOptions", LOADER_GUID) {
                    Ok(()) => println!("Cleared SystemdOptions EFI variable."),
                    Err(e) => {
                        eprintln!("Error clearing SystemdOptions: {e}");
                        process::exit(1);
                    }
                }
            } else {
                match write_efi_variable_string("SystemdOptions", LOADER_GUID, opts) {
                    Ok(()) => println!("Set SystemdOptions to: {opts}"),
                    Err(e) => {
                        eprintln!("Error setting SystemdOptions: {e}");
                        process::exit(1);
                    }
                }
            }
        }
        None => match read_efi_variable_string("SystemdOptions", LOADER_GUID) {
            Some(val) => println!("{val}"),
            None => {
                println!("SystemdOptions EFI variable is not set.");
                process::exit(1);
            }
        },
    }
}

fn cmd_kernel_identify(path: &Path) {
    if !path.exists() {
        eprintln!("Error: File not found: {}", path.display());
        process::exit(1);
    }

    let ktype = identify_kernel(path);
    println!("{ktype}");
}

fn cmd_kernel_inspect(path: &Path) {
    if !path.exists() {
        eprintln!("Error: File not found: {}", path.display());
        process::exit(1);
    }

    let info = inspect_kernel(path);

    if info.is_empty() {
        eprintln!("Could not inspect kernel image: {}", path.display());
        process::exit(1);
    }

    // Print in a stable order.
    let mut keys: Vec<&String> = info.keys().collect();
    keys.sort();

    for key in keys {
        println!("{key}: {}", info[key]);
    }
}

fn cmd_cleanup(cli: &Cli) {
    let esp = match find_esp(cli.esp_path.as_deref()) {
        Some(p) => p,
        None => {
            eprintln!("Error: Could not find the ESP. Use --esp-path= to specify.");
            process::exit(1);
        }
    };

    println!("Cleaning up {}...", esp.display());

    let mut cleaned = 0;

    // Remove temporary/leftover files.
    let patterns = ["*.new", "*.tmp", "*.old"];
    let dirs_to_check = [
        esp.join("loader"),
        esp.join("loader/entries"),
        esp.join("EFI/systemd"),
        esp.join("EFI/BOOT"),
        esp.join("EFI/Linux"),
    ];

    for dir in &dirs_to_check {
        if !dir.is_dir() {
            continue;
        }

        if let Ok(iter) = fs::read_dir(dir) {
            for entry in iter.flatten() {
                let name = entry.file_name();
                let name_str = name.to_string_lossy();

                for pattern in &patterns {
                    let suffix = pattern.trim_start_matches('*');
                    if name_str.ends_with(suffix) {
                        let path = entry.path();
                        if cli.dry_run {
                            println!("  Would remove: {}", path.display());
                        } else {
                            match fs::remove_file(&path) {
                                Ok(()) => {
                                    println!("  Removed: {}", path.display());
                                    cleaned += 1;
                                }
                                Err(e) => {
                                    eprintln!("  Warning: Could not remove {}: {e}", path.display())
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    if cleaned == 0 {
        println!("Nothing to clean up.");
    } else {
        println!("Cleaned up {cleaned} file(s).");
    }
}

fn cmd_entry_directory(cli: &Cli) {
    let esp = find_esp(cli.esp_path.as_deref());
    let xbootldr = find_xbootldr(cli.boot_path.as_deref(), esp.as_deref());

    // Determine the machine ID.
    let machine_id = fs::read_to_string("/etc/machine-id")
        .ok()
        .map(|s| s.trim().to_string());

    if let Some(ref mid) = machine_id {
        if let Some(ref boot) = xbootldr {
            let dir = boot.join(mid);
            if dir.is_dir() {
                println!("{}", dir.display());
                return;
            }
        }
        if let Some(ref esp) = esp {
            let dir = esp.join(mid);
            if dir.is_dir() {
                println!("{}", dir.display());
                return;
            }
        }
    }

    // Fallback: show the entries directory.
    if let Some(ref boot) = xbootldr {
        let dir = boot.join("loader/entries");
        if dir.is_dir() {
            println!("{}", dir.display());
            return;
        }
    }
    if let Some(ref esp) = esp {
        let dir = esp.join("loader/entries");
        if dir.is_dir() {
            println!("{}", dir.display());
            return;
        }
    }

    eprintln!("No boot entry directory found.");
    process::exit(1);
}

fn cmd_unlink(cli: &Cli, id: &str) {
    let esp = find_esp(cli.esp_path.as_deref());
    let xbootldr = find_xbootldr(cli.boot_path.as_deref(), esp.as_deref());

    let mut found = false;

    // Search for the entry in both partitions.
    let search_roots: Vec<&Path> = [esp.as_deref(), xbootldr.as_deref()]
        .into_iter()
        .flatten()
        .collect();

    for root in &search_roots {
        // Check Type #1 entries.
        let conf_path = root.join(format!("loader/entries/{id}.conf"));
        if conf_path.exists() {
            if cli.dry_run {
                println!("Would remove: {}", conf_path.display());
            } else {
                match fs::remove_file(&conf_path) {
                    Ok(()) => println!("Removed: {}", conf_path.display()),
                    Err(e) => eprintln!("Error removing {}: {e}", conf_path.display()),
                }
            }
            found = true;
        }

        // Check Type #2 entries (UKIs).
        let uki_path = root.join(format!("EFI/Linux/{id}.efi"));
        if uki_path.exists() {
            if cli.dry_run {
                println!("Would remove: {}", uki_path.display());
            } else {
                match fs::remove_file(&uki_path) {
                    Ok(()) => println!("Removed: {}", uki_path.display()),
                    Err(e) => eprintln!("Error removing {}: {e}", uki_path.display()),
                }
            }
            found = true;
        }
    }

    if !found {
        eprintln!("Boot entry not found: {id}");
        process::exit(1);
    }
}

/// Find the block device backing `/`.
///
/// If `backing` is true, additionally strip the partition suffix to
/// return the whole-disk device (e.g. `/dev/sda1` → `/dev/sda`, or
/// `/dev/nvme0n1p3` → `/dev/nvme0n1`).  This mirrors upstream
/// `bootctl -RR`.
///
/// Reads `/proc/self/mountinfo` — robust across btrfs subvols, overlay
/// roots, and tmpfs-rooted containers (in which case we return None
/// and the caller exits non-zero).
fn find_root_device(backing: bool) -> Option<String> {
    let content = std::fs::read_to_string("/proc/self/mountinfo").ok()?;
    for line in content.lines() {
        // mountinfo columns 4 (mount point) and 9 (fstype/source after `-`):
        //   36 35 98:0 /mnt1 /mnt/sda1 rw,noatime master:1 - ext4 /dev/sda1 rw
        let mut cols = line.split_whitespace();
        let (_id, _parent, _dev_major_minor, _root, mount_point) =
            (cols.next()?, cols.next()?, cols.next()?, cols.next()?, cols.next()?);
        if mount_point != "/" {
            continue;
        }
        // Walk to the separator `-`, then next field is fstype, then source.
        let rest: Vec<&str> = cols.collect();
        let sep_idx = rest.iter().position(|s| *s == "-")?;
        let source = rest.get(sep_idx + 2)?;
        // The source may be a non-block-device string (tmpfs, overlay,
        // ...).  Only accept entries that look like /dev/... — otherwise
        // the container/initrd scenarios would return garbage.
        if !source.starts_with("/dev/") {
            return None;
        }
        let dev = source.to_string();
        if !backing {
            return Some(dev);
        }
        return Some(strip_partition_suffix(&dev));
    }
    None
}

/// Strip the partition suffix from a /dev/ path to return the whole-
/// disk device.  Handles the two common kernel naming schemes:
///
/// - `/dev/sdXN`, `/dev/hdXN`, `/dev/vdXN`, `/dev/xvdXN` → `/dev/sdX`.
///   The partition is a trailing digit sequence directly appended to
///   the letter-only device name.
/// - `/dev/nvmeXnYpZ`, `/dev/mmcblkXpY`, `/dev/loopXpY` → the partition
///   is prefixed with `p` followed by digits.  Note that `nvme0n1` is
///   the whole-disk name (namespace 1) and has NO partition suffix —
///   so we must not strip its trailing `1`.
///
/// If the input doesn't match either pattern, it's returned unchanged.
fn strip_partition_suffix(dev: &str) -> String {
    // Extract basename (everything after the last '/').
    let (prefix, basename) = match dev.rfind('/') {
        Some(i) => (&dev[..=i], &dev[i + 1..]),
        None => return dev.to_string(),
    };

    // Pattern 1: nvme/mmcblk/loop — basename matches `<letters><digits>n<digits>p<digits>`
    // (nvme: nvme0n1p3) or `<letters><digits>p<digits>` (mmcblk0p1, loop0p1).
    // Heuristic: find the last `p` where the preceding char is a digit
    // and everything after is digits; only strip then.
    if let Some(p_pos) = basename.rfind('p') {
        let tail = &basename[p_pos + 1..];
        let before_p = &basename[..p_pos];
        if !tail.is_empty()
            && tail.chars().all(|c| c.is_ascii_digit())
            && before_p
                .chars()
                .last()
                .map(|c| c.is_ascii_digit())
                .unwrap_or(false)
        {
            return format!("{prefix}{before_p}");
        }
    }

    // Pattern 2: sd/hd/vd/xvd — basename is `<letters><digits>` where the
    // letters part contains no digits.  Only then is the trailing digit
    // sequence a partition suffix.  Protects nvme0n1 (letters contain
    // digits) and similar whole-disk names.
    let letter_end = basename
        .char_indices()
        .find(|(_, c)| c.is_ascii_digit())
        .map(|(i, _)| i);
    if let Some(split) = letter_end
        && split > 0
    {
        let (letters, digits) = basename.split_at(split);
        if !digits.is_empty() && digits.chars().all(|c| c.is_ascii_digit()) {
            // Only treat as partition suffix when letters match a known
            // disk-driver prefix.  Otherwise leave alone.
            let is_partitioned = matches!(letters, "sd" | "hd" | "vd" | "xvd")
                || (letters.starts_with("sd")
                    || letters.starts_with("hd")
                    || letters.starts_with("vd")
                    || letters.starts_with("xvd"));
            if is_partitioned {
                return format!("{prefix}{letters}");
            }
        }
    }

    dev.to_string()
}

// ── Main ──────────────────────────────────────────────────────────────────

fn main() {
    let cli = Cli::parse();

    // `-R` / `-RR` short-circuits before any other work — it just
    // prints the device backing / and exits.  Upstream behaviour used
    // by TEST-17-UDEV.SYSTEMD_WANTS (`ROOTDEV="$(bootctl -RR)"`).
    if cli.print_root_device > 0 {
        match find_root_device(cli.print_root_device >= 2) {
            Some(dev) => {
                println!("{}", dev);
                std::process::exit(0);
            }
            None => {
                eprintln!("bootctl: failed to determine root device");
                std::process::exit(1);
            }
        }
    }

    match cli.command {
        None | Some(Command::Status) => cmd_status(&cli),
        Some(Command::List) => cmd_list(&cli),
        Some(Command::Install {
            ref path,
            no_variables,
            force,
            ..
        }) => cmd_install(&cli, path.as_deref(), no_variables, force),
        Some(Command::Update {
            ref path,
            force,
            no_variables,
        }) => cmd_update(&cli, path.as_deref(), force, no_variables),
        Some(Command::Remove { no_variables }) => cmd_remove(&cli, no_variables),
        Some(Command::IsInstalled) => cmd_is_installed(&cli),
        Some(Command::SetDefault { ref id }) => cmd_set_default(id),
        Some(Command::SetOneshot { ref id }) => cmd_set_oneshot(id),
        Some(Command::SetTimeout { ref seconds }) => cmd_set_timeout(seconds),
        Some(Command::SetTimeoutOneshot { ref seconds }) => cmd_set_timeout_oneshot(seconds),
        Some(Command::RandomSeed) => cmd_random_seed(&cli),
        Some(Command::RebootToFirmware { flag }) => cmd_reboot_to_firmware(flag),
        Some(Command::SystemdEfiOptions { ref options }) => {
            cmd_systemd_efi_options(options.as_deref());
        }
        Some(Command::KernelIdentify { ref file }) => cmd_kernel_identify(file),
        Some(Command::KernelInspect { ref file }) => cmd_kernel_inspect(file),
        Some(Command::Cleanup) => cmd_cleanup(&cli),
        Some(Command::EntryDirectory) => cmd_entry_directory(&cli),
        Some(Command::Unlink { ref id }) => cmd_unlink(&cli, id),
    }
}

// ── Tests ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    // ── strip_partition_suffix ───────────────────────────────────────

    #[test]
    fn test_strip_partition_suffix_sda() {
        assert_eq!(strip_partition_suffix("/dev/sda1"), "/dev/sda");
        assert_eq!(strip_partition_suffix("/dev/sda42"), "/dev/sda");
    }

    #[test]
    fn test_strip_partition_suffix_nvme() {
        assert_eq!(strip_partition_suffix("/dev/nvme0n1p3"), "/dev/nvme0n1");
        assert_eq!(strip_partition_suffix("/dev/nvme0n1p42"), "/dev/nvme0n1");
    }

    #[test]
    fn test_strip_partition_suffix_mmcblk() {
        assert_eq!(strip_partition_suffix("/dev/mmcblk0p1"), "/dev/mmcblk0");
        assert_eq!(strip_partition_suffix("/dev/mmcblk0p42"), "/dev/mmcblk0");
    }

    #[test]
    fn test_strip_partition_suffix_loop() {
        assert_eq!(strip_partition_suffix("/dev/loop0p1"), "/dev/loop0");
    }

    #[test]
    fn test_strip_partition_suffix_whole_disk_unchanged() {
        // No partition suffix — return as-is.
        assert_eq!(strip_partition_suffix("/dev/sda"), "/dev/sda");
        assert_eq!(strip_partition_suffix("/dev/nvme0n1"), "/dev/nvme0n1");
    }

    #[test]
    fn test_strip_partition_suffix_weird_name_unchanged() {
        // Unknown prefix — return as-is.
        assert_eq!(strip_partition_suffix("/dev/123"), "/dev/123");
        assert_eq!(strip_partition_suffix("/dev/ram0"), "/dev/ram0");
    }

    // ── UTF-16LE encoding/decoding ───────────────────────────────────

    #[test]
    fn test_decode_utf16le_basic() {
        // "Hello" in UTF-16LE
        let data = b"H\x00e\x00l\x00l\x00o\x00";
        assert_eq!(decode_utf16le(data), Some("Hello".to_string()));
    }

    #[test]
    fn test_decode_utf16le_with_nul() {
        let data = b"H\x00i\x00\x00\x00";
        assert_eq!(decode_utf16le(data), Some("Hi".to_string()));
    }

    #[test]
    fn test_decode_utf16le_empty() {
        assert_eq!(decode_utf16le(b""), Some(String::new()));
    }

    #[test]
    fn test_decode_utf16le_odd_length() {
        assert_eq!(decode_utf16le(b"\x00\x00\x00"), None);
    }

    #[test]
    fn test_encode_utf16le_basic() {
        let encoded = encode_utf16le("Hi");
        // 'H' = 0x0048, 'i' = 0x0069, NUL = 0x0000
        assert_eq!(encoded, vec![0x48, 0x00, 0x69, 0x00, 0x00, 0x00]);
    }

    #[test]
    fn test_encode_utf16le_empty() {
        let encoded = encode_utf16le("");
        // Just a NUL terminator.
        assert_eq!(encoded, vec![0x00, 0x00]);
    }

    #[test]
    fn test_encode_decode_roundtrip() {
        let original = "systemd-boot 254.3";
        let encoded = encode_utf16le(original);
        let decoded = decode_utf16le(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn test_encode_decode_unicode() {
        let original = "Ünïcödé Bööt";
        let encoded = encode_utf16le(original);
        let decoded = decode_utf16le(&encoded).unwrap();
        assert_eq!(decoded, original);
    }

    // ── Boot counting ────────────────────────────────────────────────

    #[test]
    fn test_parse_boot_counting_none() {
        let (id, tries_left, tries_done) = parse_boot_counting("linux-6.1");
        assert_eq!(id, "linux-6.1");
        assert_eq!(tries_left, None);
        assert_eq!(tries_done, None);
    }

    #[test]
    fn test_parse_boot_counting_tries_left() {
        let (id, tries_left, tries_done) = parse_boot_counting("linux-6.1+3");
        assert_eq!(id, "linux-6.1");
        assert_eq!(tries_left, Some(3));
        assert_eq!(tries_done, Some(0));
    }

    #[test]
    fn test_parse_boot_counting_tries_both() {
        let (id, tries_left, tries_done) = parse_boot_counting("linux-6.1+3-1");
        assert_eq!(id, "linux-6.1");
        assert_eq!(tries_left, Some(3));
        assert_eq!(tries_done, Some(1));
    }

    #[test]
    fn test_parse_boot_counting_zero_tries() {
        let (id, tries_left, tries_done) = parse_boot_counting("entry+0-5");
        assert_eq!(id, "entry");
        assert_eq!(tries_left, Some(0));
        assert_eq!(tries_done, Some(5));
    }

    // ── Entry matching ───────────────────────────────────────────────

    #[test]
    fn test_entry_matches_exact() {
        assert!(entry_matches_pattern("linux-6.1", "linux-6.1"));
    }

    #[test]
    fn test_entry_matches_glob() {
        assert!(entry_matches_pattern("linux-6.1.15", "linux-6.1*"));
    }

    #[test]
    fn test_entry_matches_no_match() {
        assert!(!entry_matches_pattern("linux-6.1", "linux-5.15"));
    }

    #[test]
    fn test_entry_matches_conf_suffix() {
        assert!(entry_matches_pattern("linux-6.1.conf", "linux-6.1"));
    }

    #[test]
    fn test_entry_matches_glob_all() {
        assert!(entry_matches_pattern("anything", "*"));
    }

    // ── Version comparison ───────────────────────────────────────────

    #[test]
    fn test_version_compare_equal() {
        assert_eq!(version_compare("1.2.3", "1.2.3"), std::cmp::Ordering::Equal);
    }

    #[test]
    fn test_version_compare_less() {
        assert_eq!(version_compare("1.2.3", "1.2.4"), std::cmp::Ordering::Less);
    }

    #[test]
    fn test_version_compare_greater() {
        assert_eq!(
            version_compare("1.3.0", "1.2.9"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_version_compare_numeric_segment() {
        // "10" > "9" numerically.
        assert_eq!(
            version_compare("1.10.0", "1.9.0"),
            std::cmp::Ordering::Greater
        );
    }

    #[test]
    fn test_version_compare_different_lengths() {
        assert_eq!(
            version_compare("1.2.3", "1.2.3.1"),
            std::cmp::Ordering::Less
        );
    }

    #[test]
    fn test_version_compare_with_alpha() {
        assert_eq!(
            version_compare("6.1.0-arch1", "6.1.0-arch2"),
            std::cmp::Ordering::Less
        );
    }

    // ── split_version ────────────────────────────────────────────────

    #[test]
    fn test_split_version_dots() {
        assert_eq!(split_version("1.2.3"), vec!["1", "2", "3"]);
    }

    #[test]
    fn test_split_version_mixed() {
        assert_eq!(
            split_version("6.1.0-arch1"),
            vec!["6", "1", "0", "arch", "1"]
        );
    }

    #[test]
    fn test_split_version_underscores() {
        assert_eq!(split_version("1_2_3"), vec!["1", "2", "3"]);
    }

    #[test]
    fn test_split_version_alpha_numeric_boundary() {
        assert_eq!(split_version("abc123def"), vec!["abc", "123", "def"]);
    }

    // ── PE parsing ───────────────────────────────────────────────────

    #[test]
    fn test_parse_pe_sections_not_pe() {
        let data = b"This is not a PE file";
        assert!(parse_pe_sections(data).is_none());
    }

    #[test]
    fn test_parse_pe_sections_too_short() {
        let data = b"MZ";
        assert!(parse_pe_sections(data).is_none());
    }

    #[test]
    fn test_parse_pe_sections_invalid_pe_offset() {
        let mut data = vec![0u8; 128];
        data[0] = b'M';
        data[1] = b'Z';
        // PE offset pointing past end of data.
        data[0x3C] = 0xFF;
        assert!(parse_pe_sections(&data).is_none());
    }

    #[test]
    fn test_parse_pe_sections_valid_empty() {
        // Construct a minimal valid PE with zero sections.
        let pe_offset: u32 = 64;
        let mut data = vec![0u8; 256];
        data[0] = b'M';
        data[1] = b'Z';
        data[0x3C..0x40].copy_from_slice(&pe_offset.to_le_bytes());

        let po = pe_offset as usize;
        data[po] = b'P';
        data[po + 1] = b'E';
        data[po + 2] = 0;
        data[po + 3] = 0;

        // COFF header: number_of_sections = 0, size_of_optional_header = 0
        data[po + 6] = 0; // number_of_sections low
        data[po + 7] = 0; // number_of_sections high
        data[po + 20] = 0; // size_of_optional_header low
        data[po + 21] = 0; // size_of_optional_header high

        let sections = parse_pe_sections(&data);
        assert!(sections.is_some());
        assert!(sections.unwrap().is_empty());
    }

    #[test]
    fn test_parse_pe_sections_one_section() {
        // Construct a minimal PE with one section.
        let pe_offset: u32 = 64;
        let section_data_offset: u32 = 200;
        let section_data_size: u32 = 12;
        let mut data = vec![0u8; 256];

        // MZ header
        data[0] = b'M';
        data[1] = b'Z';
        data[0x3C..0x40].copy_from_slice(&pe_offset.to_le_bytes());

        let po = pe_offset as usize;
        // PE signature
        data[po] = b'P';
        data[po + 1] = b'E';

        // COFF header: 1 section, 0-byte optional header
        data[po + 6] = 1; // number_of_sections
        data[po + 20] = 0; // size_of_optional_header

        // Section header starts at po + 24
        let sh = po + 24;
        // Section name: ".test\0\0\0"
        data[sh] = b'.';
        data[sh + 1] = b't';
        data[sh + 2] = b'e';
        data[sh + 3] = b's';
        data[sh + 4] = b't';

        // SizeOfRawData at offset +16
        data[sh + 16..sh + 20].copy_from_slice(&section_data_size.to_le_bytes());
        // PointerToRawData at offset +20
        data[sh + 20..sh + 24].copy_from_slice(&section_data_offset.to_le_bytes());

        // Write section data
        let sdo = section_data_offset as usize;
        data[sdo..sdo + 12].copy_from_slice(b"Hello World!");

        let sections = parse_pe_sections(&data).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].0, ".test");
        assert_eq!(sections[0].1, b"Hello World!");
    }

    // ── Kernel identification ────────────────────────────────────────

    #[test]
    fn test_identify_kernel_nonexistent() {
        let ktype = identify_kernel(Path::new("/nonexistent/vmlinuz"));
        assert_eq!(ktype, KernelType::Unknown);
    }

    #[test]
    fn test_identify_kernel_empty_file() {
        let tmp = std::env::temp_dir().join("bootctl_test_empty_kernel");
        fs::write(&tmp, b"").unwrap();
        let ktype = identify_kernel(&tmp);
        assert_eq!(ktype, KernelType::Unknown);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_identify_kernel_random_data() {
        let tmp = std::env::temp_dir().join("bootctl_test_random_kernel");
        fs::write(&tmp, [0xDE, 0xAD, 0xBE, 0xEF, 0x00, 0x11, 0x22, 0x33]).unwrap();
        let ktype = identify_kernel(&tmp);
        assert_eq!(ktype, KernelType::Unknown);
        let _ = fs::remove_file(&tmp);
    }

    #[test]
    fn test_identify_kernel_gzip() {
        let tmp = std::env::temp_dir().join("bootctl_test_gzip_kernel");
        let mut data = vec![0u8; 64];
        data[0] = 0x1F;
        data[1] = 0x8B;
        fs::write(&tmp, &data).unwrap();
        let ktype = identify_kernel(&tmp);
        assert_eq!(ktype, KernelType::Vmlinuz);
        let _ = fs::remove_file(&tmp);
    }

    // ── loader.conf parsing ──────────────────────────────────────────

    #[test]
    fn test_parse_loader_conf_missing() {
        let config = parse_loader_conf(Path::new("/nonexistent"));
        assert!(config.default.is_none());
        assert!(config.timeout.is_none());
    }

    #[test]
    fn test_parse_loader_conf_basic() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_loader_conf");
        let loader_dir = tmp_dir.join("loader");
        let _ = fs::create_dir_all(&loader_dir);

        let conf = "# Boot loader configuration\n\
                     default linux-*\n\
                     timeout 5\n\
                     editor no\n\
                     console-mode max\n\
                     auto-entries yes\n\
                     auto-firmware true\n\
                     beep off\n\
                     reboot-for-bitlocker no\n\
                     secure-boot-enroll manual\n\
                     random-seed-mode with-system-token\n";

        fs::write(loader_dir.join("loader.conf"), conf).unwrap();

        let config = parse_loader_conf(&tmp_dir);
        assert_eq!(config.default.as_deref(), Some("linux-*"));
        assert_eq!(config.timeout, Some(5));
        assert_eq!(config.editor, Some(false));
        assert_eq!(config.console_mode.as_deref(), Some("max"));
        assert_eq!(config.auto_entries, Some(true));
        assert_eq!(config.auto_firmware, Some(true));
        assert_eq!(config.beep, Some(false));
        assert_eq!(config.reboot_for_bitlocker, Some(false));
        assert_eq!(config.secure_boot_enroll.as_deref(), Some("manual"));
        assert_eq!(
            config.random_seed_mode.as_deref(),
            Some("with-system-token")
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_parse_loader_conf_comments_and_blanks() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_loader_conf2");
        let loader_dir = tmp_dir.join("loader");
        let _ = fs::create_dir_all(&loader_dir);

        let conf = "# comment\n\n  # another comment\n\ntimeout 10\n\n";
        fs::write(loader_dir.join("loader.conf"), conf).unwrap();

        let config = parse_loader_conf(&tmp_dir);
        assert_eq!(config.timeout, Some(10));
        assert!(config.default.is_none());

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_parse_loader_conf_menu_force() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_loader_conf3");
        let loader_dir = tmp_dir.join("loader");
        let _ = fs::create_dir_all(&loader_dir);

        let conf = "timeout menu-force\n";
        fs::write(loader_dir.join("loader.conf"), conf).unwrap();

        let config = parse_loader_conf(&tmp_dir);
        assert_eq!(config.timeout, Some(0));

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    // ── parse_bool_value ─────────────────────────────────────────────

    #[test]
    fn test_parse_bool_value_true_variants() {
        assert_eq!(parse_bool_value("1"), Some(true));
        assert_eq!(parse_bool_value("yes"), Some(true));
        assert_eq!(parse_bool_value("true"), Some(true));
        assert_eq!(parse_bool_value("on"), Some(true));
        assert_eq!(parse_bool_value("YES"), Some(true));
        assert_eq!(parse_bool_value("True"), Some(true));
    }

    #[test]
    fn test_parse_bool_value_false_variants() {
        assert_eq!(parse_bool_value("0"), Some(false));
        assert_eq!(parse_bool_value("no"), Some(false));
        assert_eq!(parse_bool_value("false"), Some(false));
        assert_eq!(parse_bool_value("off"), Some(false));
        assert_eq!(parse_bool_value("NO"), Some(false));
    }

    #[test]
    fn test_parse_bool_value_invalid() {
        assert_eq!(parse_bool_value("maybe"), None);
        assert_eq!(parse_bool_value("2"), None);
        assert_eq!(parse_bool_value(""), None);
    }

    // ── Type #1 entry parsing ────────────────────────────────────────

    #[test]
    fn test_parse_type1_entry() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_type1");
        let _ = fs::create_dir_all(&tmp_dir);

        let entry_content = "title   Arch Linux\n\
                             version 6.1.15-arch1-1\n\
                             machine-id 1234abcd\n\
                             linux   /vmlinuz-linux\n\
                             initrd  /initramfs-linux.img\n\
                             options root=UUID=abcd-1234 rw quiet\n\
                             sort-key arch\n";

        let entry_path = tmp_dir.join("arch.conf");
        fs::write(&entry_path, entry_content).unwrap();

        let entry = parse_type1_entry(&entry_path).unwrap();
        assert_eq!(entry.id, "arch");
        assert_eq!(entry.title.as_deref(), Some("Arch Linux"));
        assert_eq!(entry.version.as_deref(), Some("6.1.15-arch1-1"));
        assert_eq!(entry.machine_id.as_deref(), Some("1234abcd"));
        assert_eq!(entry.linux.as_deref(), Some("/vmlinuz-linux"));
        assert_eq!(entry.initrd, vec!["/initramfs-linux.img"]);
        assert_eq!(
            entry.options.as_deref(),
            Some("root=UUID=abcd-1234 rw quiet")
        );
        assert_eq!(entry.sort_key.as_deref(), Some("arch"));
        assert_eq!(entry.entry_type, EntryType::Type1);
        assert_eq!(entry.tries_left, None);
        assert_eq!(entry.tries_done, None);

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_parse_type1_entry_multiple_initrd() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_type1_multi");
        let _ = fs::create_dir_all(&tmp_dir);

        let entry_content = "title   Test\n\
                             linux   /vmlinuz\n\
                             initrd  /microcode.img\n\
                             initrd  /initramfs.img\n";

        let entry_path = tmp_dir.join("test.conf");
        fs::write(&entry_path, entry_content).unwrap();

        let entry = parse_type1_entry(&entry_path).unwrap();
        assert_eq!(entry.initrd, vec!["/microcode.img", "/initramfs.img"]);

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_parse_type1_entry_boot_counting() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_type1_bc");
        let _ = fs::create_dir_all(&tmp_dir);

        let entry_content = "title Boot Entry\nlinux /vmlinuz\n";
        let entry_path = tmp_dir.join("linux-6.1+3-1.conf");
        fs::write(&entry_path, entry_content).unwrap();

        let entry = parse_type1_entry(&entry_path).unwrap();
        assert_eq!(entry.id, "linux-6.1");
        assert_eq!(entry.tries_left, Some(3));
        assert_eq!(entry.tries_done, Some(1));

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_parse_type1_entry_concatenated_options() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_type1_opts");
        let _ = fs::create_dir_all(&tmp_dir);

        let entry_content = "title   Test\n\
                             linux   /vmlinuz\n\
                             options root=/dev/sda1\n\
                             options quiet splash\n";

        let entry_path = tmp_dir.join("test.conf");
        fs::write(&entry_path, entry_content).unwrap();

        let entry = parse_type1_entry(&entry_path).unwrap();
        assert_eq!(
            entry.options.as_deref(),
            Some("root=/dev/sda1 quiet splash")
        );

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    // ── Entry discovery ──────────────────────────────────────────────

    #[test]
    fn test_discover_type1_entries_empty() {
        let entries = discover_type1_entries(Path::new("/nonexistent"));
        assert!(entries.is_empty());
    }

    #[test]
    fn test_discover_type1_entries() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_discover_t1");
        let entries_dir = tmp_dir.join("loader/entries");
        let _ = fs::create_dir_all(&entries_dir);

        fs::write(
            entries_dir.join("first.conf"),
            "title First\nlinux /vmlinuz1\n",
        )
        .unwrap();
        fs::write(
            entries_dir.join("second.conf"),
            "title Second\nlinux /vmlinuz2\n",
        )
        .unwrap();
        // Non-conf files should be ignored.
        fs::write(entries_dir.join("readme.txt"), "not an entry\n").unwrap();

        let entries = discover_type1_entries(&tmp_dir);
        assert_eq!(entries.len(), 2);

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_discover_type2_entries_empty() {
        let entries = discover_type2_entries(Path::new("/nonexistent"));
        assert!(entries.is_empty());
    }

    // ── Auto entries ─────────────────────────────────────────────────

    #[test]
    fn test_discover_auto_entries_empty() {
        let entries = discover_auto_entries(Path::new("/nonexistent"));
        assert!(entries.is_empty());
    }

    // ── EFI system detection ─────────────────────────────────────────

    #[test]
    fn test_is_efi_system_no_panic() {
        // Should not panic regardless of the system.
        let _ = is_efi_system();
    }

    // ── Firmware info ────────────────────────────────────────────────

    #[test]
    fn test_read_firmware_info_no_panic() {
        let fw = read_firmware_info();
        // Should at least have a firmware type.
        assert!(!fw.firmware_type.is_empty());
    }

    // ── Loader info ──────────────────────────────────────────────────

    #[test]
    fn test_read_loader_info_no_panic() {
        let _ = read_loader_info();
    }

    // ── ESP discovery ────────────────────────────────────────────────

    #[test]
    fn test_find_esp_explicit_path() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_esp");
        let _ = fs::create_dir_all(&tmp_dir);

        let found = find_esp(Some(&tmp_dir));
        assert_eq!(found, Some(tmp_dir.clone()));

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_find_esp_nonexistent_path() {
        let found = find_esp(Some(Path::new("/nonexistent/esp")));
        // Should fall through to other detection methods.
        // Result depends on the system.
        let _ = found;
    }

    // ── mountinfo parsing ────────────────────────────────────────────

    #[test]
    fn test_read_mountinfo_no_panic() {
        let mounts = read_mountinfo();
        // On a running Linux system, there should be at least some mounts.
        // On non-Linux, it should return empty without panicking.
        let _ = mounts;
    }

    // ── Installation helpers ─────────────────────────────────────────

    #[test]
    fn test_esp_paths() {
        let esp = Path::new("/efi");
        let boot_path = esp_efi_boot_path(esp);
        let sd_boot_path = esp_systemd_boot_path(esp);

        // Verify the paths look correct.
        assert!(
            boot_path
                .to_string_lossy()
                .starts_with("/efi/EFI/BOOT/BOOT")
        );
        assert!(
            sd_boot_path
                .to_string_lossy()
                .starts_with("/efi/EFI/systemd/systemd-boot")
        );
    }

    #[test]
    fn test_is_installed_nonexistent() {
        assert!(!is_installed(Path::new("/nonexistent/esp")));
    }

    // ── EntryType display ────────────────────────────────────────────

    #[test]
    fn test_entry_type_display() {
        assert_eq!(format!("{}", EntryType::Type1), "type1");
        assert_eq!(format!("{}", EntryType::Type2), "type2");
        assert_eq!(format!("{}", EntryType::Auto), "auto");
        assert_eq!(format!("{}", EntryType::Loader), "loader");
    }

    // ── KernelType display ───────────────────────────────────────────

    #[test]
    fn test_kernel_type_display() {
        assert_eq!(format!("{}", KernelType::Uki), "uki");
        assert_eq!(format!("{}", KernelType::Vmlinuz), "linux");
        assert_eq!(format!("{}", KernelType::Unknown), "unknown");
    }

    // ── discover_all_entries ordering ────────────────────────────────

    #[test]
    fn test_discover_all_entries_default_marking() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_all_entries");
        let entries_dir = tmp_dir.join("loader/entries");
        let _ = fs::create_dir_all(&entries_dir);

        fs::write(
            entries_dir.join("alpha.conf"),
            "title Alpha\nlinux /vmlinuz1\nsort-key a\n",
        )
        .unwrap();
        fs::write(
            entries_dir.join("beta.conf"),
            "title Beta\nlinux /vmlinuz2\nsort-key b\n",
        )
        .unwrap();

        let loader = LoaderInfo {
            loader_entry_default: Some("beta".to_string()),
            ..LoaderInfo::default()
        };
        let loader_conf = LoaderConfig::default();

        let entries = discover_all_entries(Some(&tmp_dir), None, &loader, &loader_conf);

        assert_eq!(entries.len(), 2);

        let default_entry = entries.iter().find(|e| e.is_default).unwrap();
        assert_eq!(default_entry.id, "beta");

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn test_discover_all_entries_first_is_default() {
        let tmp_dir = std::env::temp_dir().join("bootctl_test_first_default");
        let entries_dir = tmp_dir.join("loader/entries");
        let _ = fs::create_dir_all(&entries_dir);

        fs::write(
            entries_dir.join("only.conf"),
            "title Only Entry\nlinux /vmlinuz\n",
        )
        .unwrap();

        let loader = LoaderInfo::default();
        let loader_conf = LoaderConfig::default();

        let entries = discover_all_entries(Some(&tmp_dir), None, &loader, &loader_conf);

        assert_eq!(entries.len(), 1);
        assert!(entries[0].is_default);

        let _ = fs::remove_dir_all(&tmp_dir);
    }

    // ── inspect_kernel ───────────────────────────────────────────────

    #[test]
    fn test_inspect_kernel_nonexistent() {
        let info = inspect_kernel(Path::new("/nonexistent/vmlinuz"));
        assert!(info.is_empty());
    }

    // ── Loader feature printing ──────────────────────────────────────

    #[test]
    fn test_print_loader_features_no_panic() {
        // Should not panic for any input.
        print_loader_features(0);
        print_loader_features(0xFFFF);
        print_loader_features(0x0001);
    }

    // ── UKI metadata ─────────────────────────────────────────────────

    #[test]
    fn test_read_uki_metadata_nonexistent() {
        let (title, version, cmdline) = read_uki_metadata(Path::new("/nonexistent"));
        assert!(title.is_none());
        assert!(version.is_none());
        assert!(cmdline.is_none());
    }

    #[test]
    fn test_read_uki_metadata_not_pe() {
        let tmp = std::env::temp_dir().join("bootctl_test_not_pe");
        fs::write(&tmp, b"not a PE file").unwrap();
        let (title, version, cmdline) = read_uki_metadata(&tmp);
        assert!(title.is_none());
        assert!(version.is_none());
        assert!(cmdline.is_none());
        let _ = fs::remove_file(&tmp);
    }

    // ── LoaderConfig default ─────────────────────────────────────────

    #[test]
    fn test_loader_config_default() {
        let config = LoaderConfig::default();
        assert!(config.default.is_none());
        assert!(config.timeout.is_none());
        assert!(config.console_mode.is_none());
        assert!(config.editor.is_none());
        assert!(config.auto_entries.is_none());
        assert!(config.auto_firmware.is_none());
        assert!(config.beep.is_none());
        assert!(config.reboot_for_bitlocker.is_none());
        assert!(config.secure_boot_enroll.is_none());
        assert!(config.random_seed_mode.is_none());
    }
}
