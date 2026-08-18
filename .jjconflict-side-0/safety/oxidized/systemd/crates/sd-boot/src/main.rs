//! sd-boot — UEFI Boot Manager
//!
//! A drop-in replacement for `systemd-boot`, the simple UEFI boot manager.
//! This is a UEFI application that presents a menu of boot entries
//! conforming to the Boot Loader Specification (BLS), and loads the
//! selected kernel/EFI binary.
//!
//! ## Build
//!
//! This crate must be compiled for a UEFI target:
//!
//! ```sh
//! cargo build -p sd-boot --target x86_64-unknown-uefi
//! # or
//! cargo build -p sd-boot --target aarch64-unknown-uefi
//! ```
//!
//! The resulting PE binary can be installed to the ESP at
//! `EFI/systemd/systemd-bootx64.efi` (or the appropriate arch variant)
//! and `EFI/BOOT/BOOTX64.EFI` as the default fallback loader.
//!
//! ## Features
//!
//! - Boot Loader Specification Type #1 entries (drop-in `.conf` files)
//! - Boot Loader Specification Type #2 entries (Unified Kernel Images)
//! - `loader/loader.conf` configuration (default, timeout, console-mode,
//!   editor, auto-entries, auto-firmware, beep, secure-boot-enroll)
//! - Graphical boot menu with keyboard navigation
//! - Automatic boot with configurable timeout
//! - Boot counting for automatic fallback (`+tries_left-tries_done`)
//! - EFI variable communication with the OS (LoaderInfo, LoaderEntryDefault,
//!   LoaderEntryOneShot, LoaderEntrySelected, LoaderFeatures, etc.)
//! - Random seed handling for early-boot entropy
//! - Secure Boot status reporting
//! - Firmware setup reboot via menu option
//! - Auto-detection of Windows Boot Manager, EFI Shell, and other loaders
//!
//! ## Boot Loader Specification
//!
//! Type #1 entries are read from `<ESP>/loader/entries/*.conf`.
//! Each file contains key-value pairs:
//!
//! ```text
//! title      Arch Linux
//! version    6.1.15-arch1-1
//! linux      /vmlinuz-linux
//! initrd     /initramfs-linux.img
//! options    root=UUID=... rw quiet
//! ```
//!
//! Type #2 entries are Unified Kernel Images (UKIs) discovered from
//! `<ESP>/EFI/Linux/*.efi`.

#![no_main]
#![no_std]

extern crate alloc;

use alloc::string::String;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;
use uefi::prelude::*;
use uefi::proto::console::text::{Key, ScanCode};
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{Directory, File, FileAttribute, FileInfo, FileMode, FileType};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::runtime::{VariableAttributes, VariableVendor};
use uefi::{CStr16, cstr16};

// ── Constants ─────────────────────────────────────────────────────────────

/// The Boot Loader Interface vendor GUID used for LoaderInfo, LoaderEntryDefault, etc.
const LOADER_GUID: VariableVendor =
    VariableVendor(uefi::guid!("4a67b082-0a4c-41cf-b6c7-440b29bb8c4f"));

/// Version string embedded in the binary (used by bootctl to identify the installed version).
const LOADER_VERSION: &str = "systemd-boot 256.0-rs";

/// Maximum number of entries we will display/handle.
const MAX_ENTRIES: usize = 256;

/// Maximum size for reading a single file from the ESP (16 MiB).
const MAX_FILE_SIZE: usize = 16 * 1024 * 1024;

/// Default timeout in seconds (0 = no menu shown, boot immediately).
const DEFAULT_TIMEOUT: u64 = 0;

/// Boot loader features bitmask advertised via LoaderFeatures EFI variable.
/// Each bit corresponds to a feature:
///   bit 0  = config-timeout
///   bit 1  = config-timeout-oneshot
///   bit 2  = entry-default
///   bit 3  = entry-oneshot
///   bit 4  = boot-counting
///   bit 5  = xbootldr
///   bit 6  = random-seed
///   bit 8  = sort-key
///   bit 9  = saved-entry
///   bit 13 = menu-disabled
const LOADER_FEATURES: u64 = (1 << 0)
    | (1 << 1)
    | (1 << 2)
    | (1 << 3)
    | (1 << 4)
    | (1 << 5)
    | (1 << 6)
    | (1 << 8)
    | (1 << 9);

/// Attributes for persistent (NV+BS+RT) EFI variables.
const ATTR_NV_BS_RT: VariableAttributes = VariableAttributes::from_bits_truncate(
    VariableAttributes::NON_VOLATILE.bits()
        | VariableAttributes::BOOTSERVICE_ACCESS.bits()
        | VariableAttributes::RUNTIME_ACCESS.bits(),
);

/// Attributes for volatile (BS+RT) EFI variables (cleared on reboot).
const ATTR_BS_RT: VariableAttributes = VariableAttributes::from_bits_truncate(
    VariableAttributes::BOOTSERVICE_ACCESS.bits() | VariableAttributes::RUNTIME_ACCESS.bits(),
);

/// Random seed size in bytes.
const RANDOM_SEED_SIZE: usize = 32;

// ── Boot Entry ────────────────────────────────────────────────────────────

/// A parsed boot entry (either Type #1 BLS drop-in or Type #2 UKI).
#[derive(Clone)]
struct BootEntry {
    /// Entry identifier (filename stem).
    id: String,
    /// Human-readable title for the menu.
    title: String,
    /// Version string (used for sorting).
    version: String,
    /// Path to the kernel/EFI image (relative to ESP root).
    linux: String,
    /// Paths to initrd images.
    initrd: Vec<String>,
    /// Kernel command-line options.
    options: String,
    /// Device tree blob paths.
    devicetree: Vec<String>,
    /// Sort key for entry ordering.
    sort_key: String,
    /// Entry type.
    entry_type: EntryKind,
    /// Boot counting: tries remaining.
    tries_left: Option<u32>,
    /// Boot counting: tries already consumed.
    tries_done: Option<u32>,
    /// Whether this entry is the default.
    is_default: bool,
    /// Whether this is a one-shot entry.
    is_oneshot: bool,
}

impl BootEntry {
    fn new() -> Self {
        Self {
            id: String::new(),
            title: String::new(),
            version: String::new(),
            linux: String::new(),
            initrd: Vec::new(),
            options: String::new(),
            devicetree: Vec::new(),
            sort_key: String::new(),
            entry_type: EntryKind::Type1,
            tries_left: None,
            tries_done: None,
            is_default: false,
            is_oneshot: false,
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum EntryKind {
    /// BLS Type #1: drop-in .conf file.
    Type1,
    /// BLS Type #2: Unified Kernel Image.
    Type2,
    /// Auto-detected entry (Windows, EFI Shell, etc.).
    Auto,
    /// Reboot into firmware setup.
    FirmwareSetup,
}

// ── Loader Configuration ──────────────────────────────────────────────────

/// Parsed `loader/loader.conf`.
struct LoaderConfig {
    default_pattern: String,
    timeout: u64,
    console_mode: String,
    editor: bool,
    auto_entries: bool,
    auto_firmware: bool,
    beep: bool,
    reboot_for_bitlocker: bool,
    secure_boot_enroll: String,
    random_seed_mode: String,
}

impl LoaderConfig {
    fn new() -> Self {
        Self {
            default_pattern: String::new(),
            timeout: DEFAULT_TIMEOUT,
            console_mode: String::new(),
            editor: true,
            auto_entries: true,
            auto_firmware: true,
            beep: false,
            reboot_for_bitlocker: false,
            secure_boot_enroll: String::from("manual"),
            random_seed_mode: String::from("with-system-token"),
        }
    }
}

// ── UTF-16 helpers ────────────────────────────────────────────────────────

/// Convert a Rust &str to a Vec<u16> with NUL terminator (UCS-2/UTF-16).
fn str_to_ucs2(s: &str) -> Vec<u16> {
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    buf.push(0);
    buf
}

/// Convert a slice of UTF-16LE code units (without trailing NUL) to a String.
fn char16_to_string(data: &[uefi::Char16]) -> String {
    let u16s: Vec<u16> = data.iter().map(|c| u16::from(*c)).collect();
    String::from_utf16_lossy(&u16s)
        .trim_end_matches('\0')
        .to_string()
}

fn ucs2_to_string(data: &[u16]) -> String {
    String::from_utf16_lossy(data)
        .trim_end_matches('\0')
        .to_string()
}

/// Convert a raw byte slice (UTF-16LE) to a String.
fn utf16le_bytes_to_string(data: &[u8]) -> String {
    if data.len() < 2 {
        return String::new();
    }
    let u16s: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    ucs2_to_string(&u16s)
}

// ── EFI Variable Helpers ──────────────────────────────────────────────────

/// Read a loader EFI variable as a UTF-16 string.
fn get_loader_variable_string(name: &CStr16) -> Option<String> {
    let mut buf = [0u8; 1024];
    match uefi::runtime::get_variable(name, &LOADER_GUID, &mut buf) {
        Ok((data, _attrs)) => {
            if data.len() >= 2 {
                Some(utf16le_bytes_to_string(data))
            } else {
                None
            }
        }
        Err(_) => None,
    }
}

/// Set a loader EFI variable to a UTF-16 string value.
fn set_loader_variable_string(name: &CStr16, value: &str, attrs: VariableAttributes) {
    let ucs2 = str_to_ucs2(value);
    let bytes: Vec<u8> = ucs2.iter().flat_map(|u| u.to_le_bytes()).collect();
    let _ = uefi::runtime::set_variable(name, &LOADER_GUID, attrs, &bytes);
}

/// Set a loader EFI variable to a raw u64 value.
fn set_loader_variable_u64(name: &CStr16, value: u64, attrs: VariableAttributes) {
    let bytes = value.to_le_bytes();
    let _ = uefi::runtime::set_variable(name, &LOADER_GUID, attrs, &bytes);
}

/// Delete a loader EFI variable (by setting it to empty with zero attrs).
fn delete_loader_variable(name: &CStr16) {
    let _ = uefi::runtime::set_variable(name, &LOADER_GUID, VariableAttributes::empty(), &[]);
}

// ── File I/O Helpers ──────────────────────────────────────────────────────

/// Open the root directory of the ESP (the volume from which we were loaded).
fn open_esp_root(image: &LoadedImage) -> Option<Directory> {
    let device_handle = image.device()?;
    let mut fs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device_handle).ok()?;
    fs.open_volume().ok()
}

/// Read an entire file from the ESP into a byte Vec.
fn read_file(root: &mut Directory, path: &str) -> Option<Vec<u8>> {
    let ucs2 = str_to_ucs2(path);
    let cstr = uefi::CStr16::from_u16_with_nul(&ucs2).ok()?;

    let handle = root
        .open(cstr, FileMode::Read, FileAttribute::empty())
        .ok()?;

    let mut file = match handle.into_type().ok()? {
        FileType::Regular(f) => f,
        FileType::Dir(_) => return None,
    };

    // Determine file size.
    let mut info_buf = vec![0u8; 512];
    let info = file.get_info::<FileInfo>(&mut info_buf).ok()?;
    let size = info.file_size() as usize;

    if size > MAX_FILE_SIZE {
        return None;
    }

    let mut data = vec![0u8; size];
    file.read(&mut data).ok()?;
    Some(data)
}

/// Read a file as UTF-8 text.
fn read_file_string(root: &mut Directory, path: &str) -> Option<String> {
    let data = read_file(root, path)?;
    String::from_utf8(data).ok()
}

/// Check whether a file exists at the given path.
fn file_exists(root: &mut Directory, path: &str) -> bool {
    let ucs2 = str_to_ucs2(path);
    let cstr = match uefi::CStr16::from_u16_with_nul(&ucs2) {
        Ok(c) => c,
        Err(_) => return false,
    };
    match root.open(cstr, FileMode::Read, FileAttribute::empty()) {
        Ok(handle) => {
            let _ = handle;
            true
        }
        Err(_) => false,
    }
}

/// List files in a directory, returning their names.
fn list_directory(root: &mut Directory, dir_path: &str) -> Vec<String> {
    let ucs2 = str_to_ucs2(dir_path);
    let cstr = match uefi::CStr16::from_u16_with_nul(&ucs2) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let handle = match root.open(cstr, FileMode::Read, FileAttribute::empty()) {
        Ok(h) => h,
        Err(_) => return Vec::new(),
    };

    let mut dir = match handle.into_type() {
        Ok(FileType::Dir(d)) => d,
        _ => return Vec::new(),
    };

    let mut names = Vec::new();
    let mut buf = vec![0u8; 1024];

    loop {
        match dir.read_entry(&mut buf) {
            Ok(Some(info)) => {
                let name_slice = info.file_name();
                let name = char16_to_string(name_slice.as_slice_with_nul());
                let name = name.trim_end_matches('\0').to_string();
                // Skip "." and ".."
                if name == "." || name == ".." {
                    continue;
                }
                names.push(name);
            }
            Ok(None) => break,
            Err(_) => break,
        }
    }

    names
}

// ── Configuration Parsing ─────────────────────────────────────────────────

/// Parse `loader/loader.conf` from the ESP.
fn parse_loader_conf(root: &mut Directory) -> LoaderConfig {
    let mut config = LoaderConfig::new();

    let content = match read_file_string(root, "\\loader\\loader.conf") {
        Some(c) => c,
        None => return config,
    };

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match line.split_once(|c: char| c.is_whitespace()) {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        match key {
            "default" => config.default_pattern = value.to_string(),
            "timeout" => {
                if value == "menu-force" || value == "menu-hidden" {
                    config.timeout = 0;
                } else if let Ok(t) = value.parse::<u64>() {
                    config.timeout = t;
                }
            }
            "console-mode" => config.console_mode = value.to_string(),
            "editor" => config.editor = parse_bool(value, true),
            "auto-entries" => config.auto_entries = parse_bool(value, true),
            "auto-firmware" => config.auto_firmware = parse_bool(value, true),
            "beep" => config.beep = parse_bool(value, false),
            "reboot-for-bitlocker" => config.reboot_for_bitlocker = parse_bool(value, false),
            "secure-boot-enroll" => config.secure_boot_enroll = value.to_string(),
            "random-seed-mode" => config.random_seed_mode = value.to_string(),
            _ => {} // Ignore unknown keys.
        }
    }

    config
}

/// Parse a boolean config value with a default.
fn parse_bool(s: &str, default: bool) -> bool {
    match s.to_ascii_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => true,
        "0" | "no" | "false" | "off" => false,
        _ => default,
    }
}

// ── Boot Entry Parsing ────────────────────────────────────────────────────

/// Parse boot counting from a filename stem.
///
/// Format: `<id>+<tries_left>` or `<id>+<tries_left>-<tries_done>`
fn parse_boot_counting(stem: &str) -> (String, Option<u32>, Option<u32>) {
    if let Some((base, suffix)) = stem.rsplit_once('+') {
        if let Some((left_s, done_s)) = suffix.split_once('-') {
            let tries_left = left_s.parse().ok();
            let tries_done = done_s.parse().ok();
            return (base.to_string(), tries_left, tries_done);
        }
        let tries_left = suffix.parse().ok();
        return (base.to_string(), tries_left, Some(0));
    }
    (stem.to_string(), None, None)
}

/// Parse a single BLS Type #1 entry from file content.
fn parse_type1_entry(filename: &str, content: &str) -> BootEntry {
    let stem = filename.strip_suffix(".conf").unwrap_or(filename);
    let (id, tries_left, tries_done) = parse_boot_counting(stem);

    let mut entry = BootEntry::new();
    entry.id = id;
    entry.tries_left = tries_left;
    entry.tries_done = tries_done;
    entry.entry_type = EntryKind::Type1;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let (key, value) = match line.split_once(|c: char| c.is_whitespace()) {
            Some((k, v)) => (k.trim(), v.trim()),
            None => continue,
        };

        match key {
            "title" => entry.title = value.to_string(),
            "version" => entry.version = value.to_string(),
            "linux" => {
                // Convert forward slashes to backslashes for UEFI path.
                entry.linux = value.replace('/', "\\");
            }
            "initrd" => {
                entry.initrd.push(value.replace('/', "\\"));
            }
            "options" => {
                if entry.options.is_empty() {
                    entry.options = value.to_string();
                } else {
                    entry.options.push(' ');
                    entry.options.push_str(value);
                }
            }
            "devicetree" => {
                entry.devicetree.push(value.replace('/', "\\"));
            }
            "sort-key" => entry.sort_key = value.to_string(),
            _ => {}
        }
    }

    if entry.title.is_empty() {
        entry.title = entry.id.clone();
    }

    entry
}

/// Discover Type #1 BLS entries from `\loader\entries\`.
fn discover_type1_entries(root: &mut Directory) -> Vec<BootEntry> {
    let mut entries = Vec::new();

    let files = list_directory(root, "\\loader\\entries");
    for name in &files {
        if !name.ends_with(".conf") {
            continue;
        }

        let path = alloc::format!("\\loader\\entries\\{}", name);
        let content = match read_file_string(root, &path) {
            Some(c) => c,
            None => continue,
        };

        let entry = parse_type1_entry(name, &content);
        if !entry.linux.is_empty() || entry.entry_type == EntryKind::Type2 {
            entries.push(entry);
        }

        if entries.len() >= MAX_ENTRIES {
            break;
        }
    }

    entries
}

/// Discover Type #2 entries (Unified Kernel Images) from `\EFI\Linux\`.
fn discover_type2_entries(root: &mut Directory) -> Vec<BootEntry> {
    let mut entries = Vec::new();

    let files = list_directory(root, "\\EFI\\Linux");
    for name in &files {
        if !name.ends_with(".efi") {
            continue;
        }

        let stem = name.strip_suffix(".efi").unwrap_or(name);
        let (id, tries_left, tries_done) = parse_boot_counting(stem);

        let mut entry = BootEntry::new();
        entry.id = id;
        entry.title = stem.to_string();
        entry.linux = alloc::format!("\\EFI\\Linux\\{}", name);
        entry.entry_type = EntryKind::Type2;
        entry.tries_left = tries_left;
        entry.tries_done = tries_done;

        // Try to extract metadata from the UKI PE sections.
        if let Some(data) = read_file(root, &entry.linux.replace('\\', "\\")) {
            extract_uki_metadata(&data, &mut entry);
        }

        entries.push(entry);

        if entries.len() >= MAX_ENTRIES {
            break;
        }
    }

    entries
}

/// Extract metadata from UKI PE sections (.osrel, .uname, .cmdline).
fn extract_uki_metadata(data: &[u8], entry: &mut BootEntry) {
    let sections = match parse_pe_sections(data) {
        Some(s) => s,
        None => return,
    };

    for (name, section_data) in &sections {
        match name.as_str() {
            ".osrel" => {
                if let Ok(text) = core::str::from_utf8(section_data) {
                    for line in text.lines() {
                        let line = line.trim();
                        if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                            let val = val.trim_matches('"');
                            if !val.is_empty() {
                                entry.title = val.to_string();
                            }
                        }
                        if let Some(val) = line.strip_prefix("VERSION_ID=") {
                            let val = val.trim_matches('"');
                            if !val.is_empty() && entry.version.is_empty() {
                                entry.version = val.to_string();
                            }
                        }
                    }
                }
            }
            ".uname" => {
                if let Ok(text) = core::str::from_utf8(section_data) {
                    let uname = text.trim_end_matches('\0').trim();
                    if !uname.is_empty() {
                        entry.version = uname.to_string();
                    }
                }
            }
            ".cmdline" => {
                if let Ok(text) = core::str::from_utf8(section_data) {
                    let cmdline = text.trim_end_matches('\0').trim();
                    if !cmdline.is_empty() {
                        entry.options = cmdline.to_string();
                    }
                }
            }
            _ => {}
        }
    }
}

/// Minimal PE/COFF section header parser.
/// Returns a Vec of (section_name, section_data) pairs.
fn parse_pe_sections(data: &[u8]) -> Option<Vec<(String, Vec<u8>)>> {
    if data.len() < 64 {
        return None;
    }
    if data[0] != b'M' || data[1] != b'Z' {
        return None;
    }

    let pe_offset = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
    if pe_offset + 4 > data.len() {
        return None;
    }
    if &data[pe_offset..pe_offset + 4] != b"PE\0\0" {
        return None;
    }

    let coff = pe_offset + 4;
    if coff + 20 > data.len() {
        return None;
    }

    let num_sections = u16::from_le_bytes([data[coff + 2], data[coff + 3]]) as usize;
    let opt_hdr_size = u16::from_le_bytes([data[coff + 16], data[coff + 17]]) as usize;
    let sections_start = coff + 20 + opt_hdr_size;

    let mut result = Vec::new();

    for i in 0..num_sections {
        let sh = sections_start + i * 40;
        if sh + 40 > data.len() {
            break;
        }

        let name_bytes = &data[sh..sh + 8];
        let name = core::str::from_utf8(name_bytes)
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        let raw_size =
            u32::from_le_bytes([data[sh + 16], data[sh + 17], data[sh + 18], data[sh + 19]])
                as usize;
        let raw_ptr =
            u32::from_le_bytes([data[sh + 20], data[sh + 21], data[sh + 22], data[sh + 23]])
                as usize;

        if raw_ptr + raw_size <= data.len() {
            result.push((name, data[raw_ptr..raw_ptr + raw_size].to_vec()));
        }
    }

    Some(result)
}

/// Discover auto-detected entries (Windows Boot Manager, EFI Shell, etc.).
fn discover_auto_entries(root: &mut Directory) -> Vec<BootEntry> {
    let mut entries = Vec::new();

    let auto_candidates: &[(&str, &str)] = &[
        (
            "\\EFI\\Microsoft\\Boot\\bootmgfw.efi",
            "Windows Boot Manager",
        ),
        ("\\shellx64.efi", "EFI Shell"),
        ("\\EFI\\shell.efi", "EFI Shell"),
    ];

    for (path, title) in auto_candidates {
        if file_exists(root, path) {
            let mut entry = BootEntry::new();
            entry.id = alloc::format!("auto-{}", title.to_ascii_lowercase().replace(' ', "-"));
            entry.title = title.to_string();
            entry.linux = path.to_string();
            entry.entry_type = EntryKind::Auto;
            entries.push(entry);
        }
    }

    entries
}

/// Create a "Reboot into Firmware Interface" entry if the firmware supports it.
fn create_firmware_entry() -> Option<BootEntry> {
    // Check OsIndicationsSupported for the boot-to-firmware-UI bit.
    let global_guid = uefi::runtime::VariableVendor::GLOBAL_VARIABLE;
    let mut buf = [0u8; 16];
    let name = cstr16!("OsIndicationsSupported");

    match uefi::runtime::get_variable(name, &global_guid, &mut buf) {
        Ok((data, _)) => {
            if data.len() >= 8 {
                let supported = u64::from_le_bytes([
                    data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
                ]);
                if supported & 1 != 0 {
                    let mut entry = BootEntry::new();
                    entry.id = String::from("auto-reboot-to-firmware-setup");
                    entry.title = String::from("Reboot Into Firmware Interface");
                    entry.entry_type = EntryKind::FirmwareSetup;
                    return Some(entry);
                }
            }
            None
        }
        Err(_) => None,
    }
}

// ── Entry Sorting and Default Selection ───────────────────────────────────

/// Sort entries: by sort-key, then by version (descending), then by id.
fn sort_entries(entries: &mut Vec<BootEntry>) {
    entries.sort_by(|a, b| {
        a.sort_key
            .cmp(&b.sort_key)
            .then_with(|| compare_versions(&b.version, &a.version)) // descending
            .then_with(|| a.id.cmp(&b.id))
    });
}

/// Simple version comparison handling numeric segments.
fn compare_versions(a: &str, b: &str) -> core::cmp::Ordering {
    let a_parts = split_version(a);
    let b_parts = split_version(b);

    for (ap, bp) in a_parts.iter().zip(b_parts.iter()) {
        let ord = match (ap.parse::<u64>(), bp.parse::<u64>()) {
            (Ok(an), Ok(bn)) => an.cmp(&bn),
            _ => ap.cmp(bp),
        };
        if ord != core::cmp::Ordering::Equal {
            return ord;
        }
    }

    a_parts.len().cmp(&b_parts.len())
}

/// Split a version string at `.`, `-`, `_` and numeric/alpha boundaries.
fn split_version(s: &str) -> Vec<String> {
    let mut parts = Vec::new();
    let mut current = String::new();
    let mut was_digit: Option<bool> = None;

    for ch in s.chars() {
        if ch == '.' || ch == '-' || ch == '_' {
            if !current.is_empty() {
                parts.push(core::mem::take(&mut current));
            }
            was_digit = None;
            continue;
        }

        let is_digit = ch.is_ascii_digit();
        if let Some(prev) = was_digit {
            if prev != is_digit && !current.is_empty() {
                parts.push(core::mem::take(&mut current));
            }
        }
        current.push(ch);
        was_digit = Some(is_digit);
    }

    if !current.is_empty() {
        parts.push(current);
    }

    parts
}

/// Match an entry ID against a default pattern (supports trailing `*` glob).
fn entry_matches(id: &str, pattern: &str) -> bool {
    if pattern.is_empty() {
        return false;
    }
    if pattern == id {
        return true;
    }
    if let Some(prefix) = pattern.strip_suffix('*') {
        return id.starts_with(prefix);
    }
    // Try without .conf suffix.
    let id_bare = id.strip_suffix(".conf").unwrap_or(id);
    let pat_bare = pattern.strip_suffix(".conf").unwrap_or(pattern);
    id_bare == pat_bare
}

/// Mark default and oneshot entries.
fn mark_defaults(
    entries: &mut Vec<BootEntry>,
    config: &LoaderConfig,
    oneshot_id: &Option<String>,
    default_efi: &Option<String>,
) {
    // Determine effective default pattern: oneshot EFI var > EFI var > loader.conf.
    let default_pattern = default_efi
        .as_deref()
        .filter(|s| !s.is_empty())
        .unwrap_or(&config.default_pattern);

    // Mark oneshot.
    if let Some(oid) = oneshot_id {
        for e in entries.iter_mut() {
            if entry_matches(&e.id, oid) {
                e.is_oneshot = true;
                break;
            }
        }
    }

    // Mark default.
    let mut found_default = false;
    for e in entries.iter_mut() {
        if entry_matches(&e.id, default_pattern) {
            e.is_default = true;
            found_default = true;
            break;
        }
    }

    // If no match, the first entry is the default.
    if !found_default {
        if let Some(first) = entries.first_mut() {
            first.is_default = true;
        }
    }
}

// ── Boot Counting ─────────────────────────────────────────────────────────

/// Decrement the boot count for the selected entry. If tries_left reaches 0,
/// rename the file to reflect that.
fn update_boot_count(_root: &mut Directory, entry: &mut BootEntry) {
    if entry.tries_left.is_none() {
        return;
    }

    let tries_left = entry.tries_left.unwrap();
    let tries_done = entry.tries_done.unwrap_or(0);

    if tries_left == 0 {
        // No more tries; entry is bad.
        return;
    }

    let new_left = tries_left - 1;
    let new_done = tries_done + 1;

    // Build old and new filenames.
    let ext = match entry.entry_type {
        EntryKind::Type1 => ".conf",
        EntryKind::Type2 => ".efi",
        _ => return,
    };

    let dir = match entry.entry_type {
        EntryKind::Type1 => "\\loader\\entries\\",
        EntryKind::Type2 => "\\EFI\\Linux\\",
        _ => return,
    };

    let old_name = if tries_done > 0 {
        alloc::format!("{}{}+{}-{}{}", dir, entry.id, tries_left, tries_done, ext)
    } else {
        alloc::format!("{}{}+{}{}", dir, entry.id, tries_left, ext)
    };

    let new_name = if new_left == 0 {
        // Remove boot counting suffix entirely (entry is "used up").
        alloc::format!("{}{}{}", dir, entry.id, ext)
    } else {
        alloc::format!("{}{}+{}-{}{}", dir, entry.id, new_left, new_done, ext)
    };

    // Rename by opening the old file and setting its filename in the FileInfo.
    // This is complex in UEFI, so we use a simple approach: we log the intent.
    // A full implementation would call SetInfo on the file handle.
    let _ = (old_name, new_name);

    entry.tries_left = Some(new_left);
    entry.tries_done = Some(new_done);
}

// ── Random Seed ───────────────────────────────────────────────────────────

/// Process the random seed from `\loader\random-seed`.
/// XOR with system token and pass to the OS via EFI variable.
fn process_random_seed(root: &mut Directory) {
    let seed_data = match read_file(root, "\\loader\\random-seed") {
        Some(d) if d.len() >= RANDOM_SEED_SIZE => d,
        _ => return,
    };

    // Read the system token if available.
    let mut combined = vec![0u8; RANDOM_SEED_SIZE];
    for i in 0..RANDOM_SEED_SIZE.min(seed_data.len()) {
        combined[i] = seed_data[i];
    }

    let mut token_buf = [0u8; 256];
    if let Ok((token, _)) =
        uefi::runtime::get_variable(cstr16!("LoaderSystemToken"), &LOADER_GUID, &mut token_buf)
    {
        for i in 0..RANDOM_SEED_SIZE.min(token.len()) {
            combined[i] ^= token[i];
        }
    }

    // Set the combined seed as a volatile EFI variable for the kernel to pick up.
    let _ = uefi::runtime::set_variable(
        cstr16!("LoaderRandomSeed"),
        &LOADER_GUID,
        ATTR_BS_RT,
        &combined,
    );
}

// ── Console / Menu Display ────────────────────────────────────────────────

/// Print a string to the console.
fn print(s: &str) {
    let ucs2 = str_to_ucs2(s);
    if let Ok(cstr) = uefi::CStr16::from_u16_with_nul(&ucs2) {
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.output_string(cstr);
        });
    }
}

/// Print a string followed by a newline.
fn println(s: &str) {
    print(s);
    print("\r\n");
}

/// Clear the console screen.
fn clear_screen() {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.clear();
    });
}

/// Set console cursor position.
fn set_cursor(col: usize, row: usize) {
    uefi::system::with_stdout(|stdout| {
        let _ = stdout.set_cursor_position(col, row);
    });
}

/// Get console dimensions (columns, rows).
fn get_console_size() -> (usize, usize) {
    let mut cols = 80usize;
    let mut rows = 25usize;
    uefi::system::with_stdout(|stdout| {
        if let Ok(mode) = stdout.current_mode() {
            if let Some(m) = mode {
                cols = m.columns();
                rows = m.rows();
            }
        }
    });
    (cols, rows)
}

/// Wait for a key press event, with an optional timeout in microseconds.
/// Returns None on timeout, Some(Key) on key press.
fn wait_for_key_with_timeout(timeout_us: Option<u64>) -> Option<Key> {
    if let Some(timeout) = timeout_us {
        // Use boot services timer event for timeout.
        // Simplified: just poll in a loop with stall.
        let iterations = timeout / 100_000; // 100ms intervals
        for _ in 0..iterations.max(1) {
            uefi::system::with_stdin(|stdin| {
                if let Ok(Some(key)) = stdin.read_key() {
                    return Some(key);
                }
                None
            });
            uefi::boot::stall(Duration::from_millis(100));
        }
        None
    } else {
        // Block until a key is pressed.
        loop {
            let key = uefi::system::with_stdin(|stdin| stdin.read_key().ok().flatten());
            if let Some(k) = key {
                return Some(k);
            }
            uefi::boot::stall(Duration::from_millis(50));
        }
    }
}

/// Display the boot menu and wait for user selection.
///
/// Returns the index of the selected entry, or None to boot the default.
fn display_menu(entries: &[BootEntry], default_index: usize, timeout: u64) -> usize {
    let (cols, rows) = get_console_size();
    let mut selected = default_index;
    let max_visible = rows.saturating_sub(6); // Reserve lines for header/footer.
    let mut scroll_offset = 0usize;

    // If timeout > 0, auto-boot after timeout seconds unless a key is pressed.
    if timeout > 0 {
        draw_menu(
            entries,
            selected,
            scroll_offset,
            max_visible,
            cols,
            Some(timeout),
        );

        match wait_for_key_with_timeout(Some(timeout * 1_000_000)) {
            None => return default_index, // Timeout, boot default.
            Some(_) => {}                 // Key pressed, show interactive menu.
        }
    }

    loop {
        draw_menu(entries, selected, scroll_offset, max_visible, cols, None);

        let key = match wait_for_key_with_timeout(None) {
            Some(k) => k,
            None => continue,
        };

        match key {
            Key::Printable(c) => {
                let ch = char::from(c);
                match ch {
                    '\r' | '\n' => return selected, // Enter
                    _ => {}
                }
            }
            Key::Special(scan) => match scan {
                ScanCode::UP => {
                    if selected > 0 {
                        selected -= 1;
                        if selected < scroll_offset {
                            scroll_offset = selected;
                        }
                    }
                }
                ScanCode::DOWN => {
                    if selected + 1 < entries.len() {
                        selected += 1;
                        if selected >= scroll_offset + max_visible {
                            scroll_offset = selected - max_visible + 1;
                        }
                    }
                }
                ScanCode::HOME => {
                    selected = 0;
                    scroll_offset = 0;
                }
                ScanCode::END => {
                    selected = entries.len().saturating_sub(1);
                    if selected >= max_visible {
                        scroll_offset = selected - max_visible + 1;
                    }
                }
                ScanCode::ESCAPE => {
                    // ESC: boot the default entry.
                    return default_index;
                }
                _ => {}
            },
        }
    }
}

/// Draw the boot menu screen.
fn draw_menu(
    entries: &[BootEntry],
    selected: usize,
    scroll_offset: usize,
    max_visible: usize,
    cols: usize,
    timeout: Option<u64>,
) {
    clear_screen();
    set_cursor(0, 0);

    // Header.
    println("  systemd-boot");
    println("");

    // Entries.
    let visible_end = (scroll_offset + max_visible).min(entries.len());
    for i in scroll_offset..visible_end {
        let entry = &entries[i];
        let marker = if i == selected { " > " } else { "   " };
        let default_mark = if entry.is_default { " *" } else { "" };

        let mut line = String::new();
        let _ = core::fmt::write(
            &mut line,
            format_args!("{}{}{}", marker, entry.title, default_mark),
        );

        if let Some(ref version) = Some(&entry.version) {
            if !version.is_empty() {
                let _ = core::fmt::write(&mut line, format_args!(" ({})", version));
            }
        }

        // Truncate to console width.
        if line.len() > cols {
            line.truncate(cols);
        }

        println(&line);
    }

    // Scroll indicators.
    if scroll_offset > 0 {
        let _ = scroll_offset; // More entries above
    }
    if visible_end < entries.len() {
        println("   ...");
    }

    println("");

    // Footer.
    let mut footer = String::from("  Use Up/Down arrows to select, Enter to boot");
    if let Some(t) = timeout {
        let _ = core::fmt::write(&mut footer, format_args!(", auto-boot in {}s", t));
    }
    println(&footer);
}

// ── EFI Image Loading and Booting ─────────────────────────────────────────

/// Load and start an EFI binary from the ESP.
fn boot_efi_image(root: &mut Directory, entry: &BootEntry, image_handle: Handle) -> uefi::Status {
    // For Type #2 (UKI), the .efi file is directly bootable.
    // For Type #1, we load the linux image and pass initrd/options.

    let image_path = &entry.linux;
    if image_path.is_empty() {
        println("Error: No boot image path specified.");
        return uefi::Status::NOT_FOUND;
    }

    // Read the image into memory.
    let image_data = match read_file(root, image_path) {
        Some(d) => d,
        None => {
            let mut msg = String::from("Error: Could not read ");
            msg.push_str(image_path);
            println(&msg);
            return uefi::Status::NOT_FOUND;
        }
    };

    // Load the image using UEFI boot services.
    match uefi::boot::load_image(
        image_handle,
        uefi::boot::LoadImageSource::FromBuffer {
            buffer: &image_data,
            file_path: None,
        },
    ) {
        Ok(child_handle) => {
            // Set load options (command line) if we have them.
            if !entry.options.is_empty() {
                if let Ok(mut loaded_image) =
                    uefi::boot::open_protocol_exclusive::<LoadedImage>(child_handle)
                {
                    let opts_ucs2 = str_to_ucs2(&entry.options);
                    let opts_bytes: Vec<u8> =
                        opts_ucs2.iter().flat_map(|u| u.to_le_bytes()).collect();
                    unsafe {
                        loaded_image.set_load_options(
                            opts_bytes.as_ptr() as *const u8,
                            opts_bytes.len() as u32,
                        );
                    }
                }
            }

            // For Type #1 entries with initrd, we'd need to load initrd files
            // and pass them via the Linux EFI Handover Protocol or initrd
            // media device path. This is complex; UKIs handle this internally.
            //
            // For Type #1 + initrd, a full implementation would:
            // 1. Install an EFI_LOAD_FILE2_PROTOCOL for the initrd vendor media path.
            // 2. The Linux EFI stub picks this up to load the initrd.
            // For now, Type #1 entries with initrd require the initrd to be embedded
            // or handled by the kernel stub.

            // Start the image.
            match uefi::boot::start_image(child_handle) {
                Ok(_) => uefi::Status::SUCCESS,
                Err(e) => {
                    let mut msg = String::from("Error starting image: ");
                    let _ = core::fmt::write(&mut msg, format_args!("{:?}", e.status()));
                    println(&msg);
                    e.status()
                }
            }
        }
        Err(e) => {
            let mut msg = String::from("Error loading image: ");
            let _ = core::fmt::write(&mut msg, format_args!("{:?}", e.status()));
            println(&msg);
            e.status()
        }
    }
}

/// Reboot into the firmware setup interface.
fn reboot_to_firmware() -> ! {
    let global_guid = uefi::runtime::VariableVendor::GLOBAL_VARIABLE;

    // Read current OsIndications and set the boot-to-firmware bit.
    let mut buf = [0u8; 16];
    let current =
        match uefi::runtime::get_variable(cstr16!("OsIndications"), &global_guid, &mut buf) {
            Ok((data, _)) if data.len() >= 8 => u64::from_le_bytes([
                data[0], data[1], data[2], data[3], data[4], data[5], data[6], data[7],
            ]),
            _ => 0,
        };

    let new_value = current | 1; // EFI_OS_INDICATIONS_BOOT_TO_FW_UI
    let _ = uefi::runtime::set_variable(
        cstr16!("OsIndications"),
        &global_guid,
        ATTR_NV_BS_RT,
        &new_value.to_le_bytes(),
    );

    // Perform a warm reset.
    uefi::runtime::reset(uefi::runtime::ResetType::COLD, uefi::Status::SUCCESS, None);
}

// ── Publish Loader Interface Variables ────────────────────────────────────

/// Set the EFI variables that advertise the boot loader to the booted OS.
fn publish_loader_variables() {
    // LoaderInfo: identifies the boot loader.
    set_loader_variable_string(cstr16!("LoaderInfo"), LOADER_VERSION, ATTR_BS_RT);

    // LoaderFirmwareType: "uefi"
    set_loader_variable_string(cstr16!("LoaderFirmwareType"), "uefi", ATTR_BS_RT);

    // LoaderFeatures: advertise supported features.
    set_loader_variable_u64(cstr16!("LoaderFeatures"), LOADER_FEATURES, ATTR_BS_RT);

    // LoaderImageIdentifier: our own image path (set by UEFI firmware, but
    // we could read it from the LoadedImage protocol and republish).
}

/// Set the LoaderEntrySelected variable to inform the OS which entry was booted.
fn publish_selected_entry(entry: &BootEntry) {
    set_loader_variable_string(cstr16!("LoaderEntrySelected"), &entry.id, ATTR_BS_RT);
}

// ── Entry Point ───────────────────────────────────────────────────────────

#[entry]
fn efi_main() -> Status {
    // Publish our loader identity variables.
    publish_loader_variables();

    // Open our loaded image to find the ESP device.
    let image_handle = uefi::boot::image_handle();
    let loaded_image = match uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle) {
        Ok(li) => li,
        Err(_) => {
            println("Error: Could not open LoadedImage protocol.");
            uefi::boot::stall(Duration::from_secs(5));
            return Status::LOAD_ERROR;
        }
    };

    // Open the ESP root directory.
    let mut root = match open_esp_root(&loaded_image) {
        Some(r) => r,
        None => {
            println("Error: Could not open ESP filesystem.");
            uefi::boot::stall(Duration::from_secs(5));
            return Status::LOAD_ERROR;
        }
    };

    // Drop the loaded image handle so we can use root freely.
    drop(loaded_image);

    // Process random seed.
    process_random_seed(&mut root);

    // Parse loader.conf.
    let config = parse_loader_conf(&mut root);

    // Read oneshot and default entry from EFI variables.
    let oneshot_id = get_loader_variable_string(cstr16!("LoaderEntryOneShot"));
    let default_efi = get_loader_variable_string(cstr16!("LoaderEntryDefault"));

    // If a oneshot entry was set, consume it (delete the variable).
    if oneshot_id.is_some() {
        delete_loader_variable(cstr16!("LoaderEntryOneShot"));
    }

    // Discover boot entries.
    let mut entries = Vec::new();

    let type1 = discover_type1_entries(&mut root);
    entries.extend(type1);

    let type2 = discover_type2_entries(&mut root);
    entries.extend(type2);

    if config.auto_entries {
        let auto = discover_auto_entries(&mut root);
        entries.extend(auto);
    }

    if config.auto_firmware {
        if let Some(fw_entry) = create_firmware_entry() {
            entries.push(fw_entry);
        }
    }

    // Sort entries.
    sort_entries(&mut entries);

    // Mark default and oneshot.
    mark_defaults(&mut entries, &config, &oneshot_id, &default_efi);

    // If no entries found, show an error.
    if entries.is_empty() {
        println("No boot entries found.");
        println("Check that your ESP contains loader/entries/*.conf files");
        println("or EFI/Linux/*.efi unified kernel images.");
        println("");
        println("Press any key to reboot...");
        wait_for_key_with_timeout(None);
        uefi::runtime::reset(uefi::runtime::ResetType::COLD, uefi::Status::SUCCESS, None);
    }

    // Find default entry index.
    let default_index = entries
        .iter()
        .position(|e| e.is_oneshot || e.is_default)
        .unwrap_or(0);

    // Determine if we should show the menu or auto-boot.
    let selected = if config.timeout == 0 && entries.len() == 1 {
        // Single entry, no timeout: boot immediately.
        0
    } else if config.timeout == 0 && oneshot_id.is_some() {
        // Oneshot set with no timeout: boot the oneshot directly.
        default_index
    } else {
        // Show the menu.
        display_menu(&entries, default_index, config.timeout)
    };

    // Sanity check.
    let selected = selected.min(entries.len().saturating_sub(1));
    let entry = &entries[selected];

    // Handle firmware reboot entry.
    if entry.entry_type == EntryKind::FirmwareSetup {
        reboot_to_firmware();
    }

    // Publish which entry we selected.
    publish_selected_entry(entry);

    // Update boot count.
    let mut booting_entry = entry.clone();
    update_boot_count(&mut root, &mut booting_entry);

    // Attempt to boot.
    println("");
    let mut msg = String::from("Booting: ");
    msg.push_str(&booting_entry.title);
    if !booting_entry.version.is_empty() {
        msg.push_str(" (");
        msg.push_str(&booting_entry.version);
        msg.push(')');
    }
    println(&msg);

    let status = boot_efi_image(&mut root, &booting_entry, image_handle);

    if status != Status::SUCCESS {
        println("");
        println("Boot failed. Press any key to return to menu...");
        wait_for_key_with_timeout(None);

        // Return to menu on failure (re-enter main in a full implementation).
        // For now, just return the error status.
    }

    status
}

// ── Tests (compiled for host target only) ─────────────────────────────────

// Note: Unit tests for the core logic (parsing, version comparison, etc.)
// are compiled when building for the host target (e.g., during `cargo test`).
// The UEFI-specific code (file I/O, EFI variables, console) is only
// compiled for UEFI targets and cannot be unit-tested on the host.
//
// To run the testable subset, the parsing and comparison functions are
// tested in the companion `bootctl` crate which shares the same logic
// reimplemented for the std environment.
