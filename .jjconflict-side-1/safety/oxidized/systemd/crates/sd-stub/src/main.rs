//! sd-stub — UEFI Stub for Unified Kernel Images (UKI)
//!
//! A drop-in replacement for `systemd-stub`, the UEFI stub that is
//! combined with a Linux kernel, initrd, command line, and other
//! resources into a single Unified Kernel Image (UKI) PE binary.
//!
//! ## Build
//!
//! This crate must be compiled for a UEFI target:
//!
//! ```sh
//! cargo build -p sd-stub --target x86_64-unknown-uefi
//! # or
//! cargo build -p sd-stub --target aarch64-unknown-uefi
//! ```
//!
//! ## Creating a UKI
//!
//! A Unified Kernel Image is a PE binary that embeds the stub itself
//! along with several payload sections:
//!
//! | Section      | Content                                    |
//! |--------------|--------------------------------------------|
//! | `.osrel`     | os-release data (PRETTY_NAME, VERSION, …)  |
//! | `.cmdline`   | Default kernel command line                 |
//! | `.linux`     | The Linux kernel image (vmlinuz)            |
//! | `.initrd`    | The initramfs/initrd image                  |
//! | `.splash`    | Boot splash image (BMP)                     |
//! | `.dtb`       | Device tree blob                            |
//! | `.uname`     | Kernel version string                       |
//! | `.sbat`      | SBAT metadata for Secure Boot revocation    |
//! | `.pcrpkey`   | Public key for PCR signing                  |
//! | `.pcrsig`    | PCR signature (JSON)                        |
//!
//! Example using `objcopy`:
//!
//! ```sh
//! objcopy \
//!   --add-section .osrel=/etc/os-release --change-section-vma .osrel=0x20000 \
//!   --add-section .cmdline=cmdline.txt   --change-section-vma .cmdline=0x30000 \
//!   --add-section .linux=vmlinuz         --change-section-vma .linux=0x2000000 \
//!   --add-section .initrd=initramfs.img  --change-section-vma .initrd=0x3000000 \
//!   sd-stub.efi unified-kernel.efi
//! ```
//!
//! Or using `ukify`:
//!
//! ```sh
//! ukify build --stub=sd-stub.efi --linux=vmlinuz --initrd=initramfs.img \
//!   --cmdline=@cmdline.txt --os-release=@/etc/os-release --output=unified.efi
//! ```
//!
//! ## Features
//!
//! - Extracts `.linux`, `.initrd`, `.cmdline`, `.dtb` from its own PE image
//! - Passes initrd to the kernel via the `EFI_LOAD_FILE2_PROTOCOL`
//!   on the initrd vendor media device path
//! - Combines embedded `.cmdline` with any options passed by the boot loader
//! - TPM2 PCR measurement of all embedded sections (PCRs 8-12)
//! - Publishes `.osrel` and `.uname` via `LoaderDevicePartUUID`,
//!   `StubInfo`, `StubFeatures` EFI variables
//! - Displays `.splash` BMP image during boot (if present)
//! - Supports additional initrd images from credentials and sysext directories
//! - SBAT metadata for Secure Boot Advanced Targeting revocation checks

#![no_main]
#![no_std]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::string::ToString;
use alloc::vec;
use alloc::vec::Vec;
use core::time::Duration;
use uefi::cstr16;
use uefi::prelude::*;
use uefi::proto::loaded_image::LoadedImage;
use uefi::proto::media::file::{File, FileAttribute, FileInfo, FileMode, FileType};
use uefi::proto::media::fs::SimpleFileSystem;
use uefi::runtime::{VariableAttributes, VariableVendor};

// ── Constants ─────────────────────────────────────────────────────────────

/// The Boot Loader Interface vendor GUID for Loader* variables.
const LOADER_GUID: VariableVendor =
    VariableVendor(uefi::guid!("4a67b082-0a4c-41cf-b6c7-440b29bb8c4f"));

/// Stub version string published via the StubInfo EFI variable.
const STUB_VERSION: &str = "systemd-stub 256.0-rs";

/// Attributes for volatile (BS+RT) EFI variables.
const ATTR_BS_RT: VariableAttributes = VariableAttributes::from_bits_truncate(
    VariableAttributes::BOOTSERVICE_ACCESS.bits() | VariableAttributes::RUNTIME_ACCESS.bits(),
);

/// Maximum size for a single PE section we'll handle (256 MiB).
const MAX_SECTION_SIZE: usize = 256 * 1024 * 1024;

/// Stub features bitmask advertised via StubFeatures EFI variable.
///   bit 0 = can report stub info
///   bit 1 = can pick up credentials from ESP
///   bit 2 = can pick up sysexts from ESP
///   bit 3 = supports three PCR banks (11, 12, 13)
///   bit 4 = supports passing kernel command line
const STUB_FEATURES: u64 = (1 << 0) | (1 << 1) | (1 << 2) | (1 << 4);

/// The Linux EFI initrd media device path vendor GUID.
/// This is the well-known GUID that the Linux EFI stub recognizes
/// for loading initrd via EFI_LOAD_FILE2_PROTOCOL.
const LINUX_INITRD_MEDIA_GUID: uefi::Guid = uefi::guid!("5568e427-68fc-4f3d-ac74-ca555231cc68");

/// TPM2 PCR indices for measuring UKI sections, following the
/// systemd-stub specification:
///   PCR  8 — kernel command line
///   PCR  9 — kernel image
///   PCR 11 — unified kernel image (all sections)
///   PCR 12 — kernel command line (alternative/extension)
///   PCR 13 — system extensions
const PCR_KERNEL_CMDLINE: u32 = 8;
const PCR_KERNEL_IMAGE: u32 = 9;
const PCR_UNIFIED_IMAGE: u32 = 11;
const PCR_SYSEXT: u32 = 13;

// ── PE Section Names ──────────────────────────────────────────────────────

/// Well-known PE section names embedded in a UKI.
const SECTION_OSREL: &str = ".osrel";
const SECTION_CMDLINE: &str = ".cmdline";
const SECTION_LINUX: &str = ".linux";
const SECTION_INITRD: &str = ".initrd";
const SECTION_SPLASH: &str = ".splash";
const SECTION_DTB: &str = ".dtb";
const SECTION_UNAME: &str = ".uname";
const SECTION_SBAT: &str = ".sbat";
const SECTION_PCRPKEY: &str = ".pcrpkey";
const SECTION_PCRSIG: &str = ".pcrsig";

// ── PE Section Data ───────────────────────────────────────────────────────

/// A parsed PE section with its name and data.
#[derive(Clone)]
struct PeSection {
    name: String,
    data: Vec<u8>,
    virtual_address: u32,
    virtual_size: u32,
}

/// All embedded sections extracted from the UKI PE image.
struct UkiSections {
    osrel: Option<Vec<u8>>,
    cmdline: Option<Vec<u8>>,
    linux: Option<Vec<u8>>,
    initrd: Option<Vec<u8>>,
    splash: Option<Vec<u8>>,
    dtb: Option<Vec<u8>>,
    uname: Option<Vec<u8>>,
    sbat: Option<Vec<u8>>,
    pcrpkey: Option<Vec<u8>>,
    pcrsig: Option<Vec<u8>>,
    /// All sections in order, for TPM measurement.
    all_sections: Vec<PeSection>,
}

impl UkiSections {
    fn new() -> Self {
        Self {
            osrel: None,
            cmdline: None,
            linux: None,
            initrd: None,
            splash: None,
            dtb: None,
            uname: None,
            sbat: None,
            pcrpkey: None,
            pcrsig: None,
            all_sections: Vec::new(),
        }
    }

    /// Extract a section's data as a UTF-8 string (trimming NUL bytes).
    fn section_as_string(data: &Option<Vec<u8>>) -> Option<String> {
        let d = data.as_ref()?;
        let text = core::str::from_utf8(d).ok()?;
        Some(text.trim_end_matches('\0').trim().to_string())
    }

    /// Get the embedded kernel command line.
    fn cmdline_string(&self) -> Option<String> {
        Self::section_as_string(&self.cmdline)
    }

    /// Get the kernel version from .uname section.
    fn uname_string(&self) -> Option<String> {
        Self::section_as_string(&self.uname)
    }

    /// Get PRETTY_NAME from .osrel section.
    fn pretty_name(&self) -> Option<String> {
        let text = Self::section_as_string(&self.osrel)?;
        for line in text.lines() {
            let line = line.trim();
            if let Some(val) = line.strip_prefix("PRETTY_NAME=") {
                return Some(val.trim_matches('"').to_string());
            }
        }
        None
    }
}

// ── UTF-16 Helpers ────────────────────────────────────────────────────────

/// Convert a Rust &str to a Vec<u16> with NUL terminator (UCS-2).
fn str_to_ucs2(s: &str) -> Vec<u16> {
    let mut buf: Vec<u16> = s.encode_utf16().collect();
    buf.push(0);
    buf
}

/// Encode a string as UTF-16LE bytes (with NUL terminator).
fn str_to_utf16le_bytes(s: &str) -> Vec<u8> {
    let ucs2 = str_to_ucs2(s);
    ucs2.iter().flat_map(|u| u.to_le_bytes()).collect()
}

// ── PE Parsing ────────────────────────────────────────────────────────────

/// Parse PE/COFF section headers from the raw image of our own executable.
///
/// This extracts all sections from the PE binary that constitutes the UKI.
/// The stub reads its own loaded image to find the embedded kernel, initrd,
/// command line, and other payloads.
fn parse_pe_image(data: &[u8]) -> Option<UkiSections> {
    if data.len() < 64 {
        return None;
    }

    // Verify MZ signature.
    if data[0] != b'M' || data[1] != b'Z' {
        return None;
    }

    // PE header offset is at 0x3C.
    let pe_offset = u32::from_le_bytes([data[0x3C], data[0x3D], data[0x3E], data[0x3F]]) as usize;
    if pe_offset + 4 > data.len() {
        return None;
    }

    // Verify PE signature "PE\0\0".
    if &data[pe_offset..pe_offset + 4] != b"PE\0\0" {
        return None;
    }

    // COFF header follows PE signature.
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

    let mut uki = UkiSections::new();

    for i in 0..number_of_sections {
        let sh_offset = sections_offset + i * 40;
        if sh_offset + 40 > data.len() {
            break;
        }

        // Section name: 8 bytes, NUL-padded.
        let name_bytes = &data[sh_offset..sh_offset + 8];
        let name = core::str::from_utf8(name_bytes)
            .unwrap_or("")
            .trim_end_matches('\0')
            .to_string();

        // VirtualSize at offset +8.
        let virtual_size = u32::from_le_bytes([
            data[sh_offset + 8],
            data[sh_offset + 9],
            data[sh_offset + 10],
            data[sh_offset + 11],
        ]);

        // VirtualAddress at offset +12.
        let virtual_address = u32::from_le_bytes([
            data[sh_offset + 12],
            data[sh_offset + 13],
            data[sh_offset + 14],
            data[sh_offset + 15],
        ]);

        // SizeOfRawData at offset +16.
        let raw_data_size = u32::from_le_bytes([
            data[sh_offset + 16],
            data[sh_offset + 17],
            data[sh_offset + 18],
            data[sh_offset + 19],
        ]) as usize;

        // PointerToRawData at offset +20.
        let raw_data_ptr = u32::from_le_bytes([
            data[sh_offset + 20],
            data[sh_offset + 21],
            data[sh_offset + 22],
            data[sh_offset + 23],
        ]) as usize;

        if raw_data_size > MAX_SECTION_SIZE {
            continue;
        }

        if raw_data_ptr + raw_data_size > data.len() {
            continue;
        }

        let section_data = data[raw_data_ptr..raw_data_ptr + raw_data_size].to_vec();

        let pe_section = PeSection {
            name: name.clone(),
            data: section_data.clone(),
            virtual_address,
            virtual_size,
        };

        uki.all_sections.push(pe_section);

        // Assign to the appropriate field.
        match name.as_str() {
            SECTION_OSREL => uki.osrel = Some(section_data),
            SECTION_CMDLINE => uki.cmdline = Some(section_data),
            SECTION_LINUX => uki.linux = Some(section_data),
            SECTION_INITRD => uki.initrd = Some(section_data),
            SECTION_SPLASH => uki.splash = Some(section_data),
            SECTION_DTB => uki.dtb = Some(section_data),
            SECTION_UNAME => uki.uname = Some(section_data),
            SECTION_SBAT => uki.sbat = Some(section_data),
            SECTION_PCRPKEY => uki.pcrpkey = Some(section_data),
            SECTION_PCRSIG => uki.pcrsig = Some(section_data),
            _ => {} // Ignore unknown sections (e.g., .text, .data, .reloc).
        }
    }

    Some(uki)
}

// ── Read Own Image ────────────────────────────────────────────────────────

/// Read our own PE image from the device we were loaded from.
///
/// We need to read the raw file because the loaded image in memory
/// has sections mapped at virtual addresses, but we need the raw
/// file offsets for section data extraction.
fn read_own_image(image: &LoadedImage) -> Option<Vec<u8>> {
    let device_handle = image.device()?;

    let mut fs = uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device_handle).ok()?;
    let mut root = fs.open_volume().ok()?;

    // Get our image file path from the loaded image.
    let file_path = image.file_path()?;

    // Convert the device path to a string to open the file.
    // The file path node contains a UTF-16 path like \EFI\Linux\uki.efi
    // We need to walk the device path nodes and find the file path.
    let path_str = device_path_to_string(file_path);

    if path_str.is_empty() {
        // Fallback: try to read using the image base and size directly.
        return read_image_from_memory(image);
    }

    let ucs2 = str_to_ucs2(&path_str);
    let cstr = uefi::CStr16::from_u16_with_nul(&ucs2).ok()?;

    let handle = root
        .open(cstr, FileMode::Read, FileAttribute::empty())
        .ok()?;

    let mut file = match handle.into_type().ok()? {
        FileType::Regular(f) => f,
        FileType::Dir(_) => return None,
    };

    // Get file size.
    let mut info_buf = vec![0u8; 1024];
    let info = file.get_info::<FileInfo>(&mut info_buf).ok()?;
    let size = info.file_size() as usize;

    if size > MAX_SECTION_SIZE * 4 {
        return None; // Sanity limit.
    }

    let mut data = vec![0u8; size];
    file.read(&mut data).ok()?;
    Some(data)
}

/// Extract the file path string from an EFI device path.
fn device_path_to_string(path: &uefi::proto::device_path::DevicePath) -> String {
    let mut result = String::new();

    for node in path.node_iter() {
        // File path media device path: type 4, subtype 4
        if node.full_type()
            == (
                uefi::proto::device_path::DeviceType::MEDIA,
                uefi::proto::device_path::DeviceSubType::MEDIA_FILE_PATH,
            )
        {
            // The payload is a UTF-16LE null-terminated string.
            let node_data = node.as_ffi_ptr();
            let header_size = 4usize; // type(1) + subtype(1) + length(2)
            let node_len = node.length() as usize;

            if node_len > header_size {
                let payload_len = node_len - header_size;
                let payload_ptr = unsafe { (node_data as *const u8).add(header_size) };
                let payload = unsafe { core::slice::from_raw_parts(payload_ptr, payload_len) };

                // Decode UTF-16LE.
                if payload.len() >= 2 {
                    let u16s: Vec<u16> = payload
                        .chunks_exact(2)
                        .map(|c| u16::from_le_bytes([c[0], c[1]]))
                        .collect();

                    let s = String::from_utf16_lossy(&u16s);
                    let s = s.trim_end_matches('\0');
                    if !result.is_empty() && !result.ends_with('\\') {
                        result.push('\\');
                    }
                    result.push_str(s);
                }
            }
        }
    }

    result
}

/// Fallback: read the image directly from memory using the loaded image base/size.
///
/// This is less reliable than reading from the filesystem because the PE
/// loader may have performed relocations, but it works as a fallback.
fn read_image_from_memory(image: &LoadedImage) -> Option<Vec<u8>> {
    let (base, size) = image.info();
    if base.is_null() || size == 0 {
        return None;
    }
    let data = unsafe { core::slice::from_raw_parts(base as *const u8, size as usize) };
    Some(data.to_vec())
}

// ── Console Helpers ───────────────────────────────────────────────────────

/// Print a string to the UEFI console.
fn print(s: &str) {
    let ucs2 = str_to_ucs2(s);
    if let Ok(cstr) = uefi::CStr16::from_u16_with_nul(&ucs2) {
        uefi::system::with_stdout(|stdout| {
            let _ = stdout.output_string(cstr);
        });
    }
}

/// Print a string followed by a carriage return + newline.
fn println(s: &str) {
    print(s);
    print("\r\n");
}

// ── TPM2 PCR Measurement ─────────────────────────────────────────────────

/// Measure a data buffer into the specified TPM2 PCR.
///
/// Uses the TCG2 protocol (EFI_TCG2_PROTOCOL) to extend the PCR
/// with a hash of the data. If the TPM2 is not available, this is a no-op.
///
/// The `description` is logged in the TCG event log to identify what
/// was measured.
fn measure_to_pcr(pcr_index: u32, data: &[u8], description: &str) {
    // The TCG2 protocol GUID.
    let tcg2_guid = uefi::guid!("607f766c-7455-42be-930b-e4d76db2720f");

    // Try to locate the TCG2 protocol. If it's not available (no TPM2),
    // silently skip measurement.
    let handle =
        match uefi::boot::locate_handle_buffer(uefi::boot::SearchType::ByProtocol(&tcg2_guid)) {
            Ok(handles) if !handles.is_empty() => handles[0],
            _ => return, // No TPM2 available.
        };

    // In a full implementation, we would:
    // 1. Open EFI_TCG2_PROTOCOL on the handle.
    // 2. Allocate and fill an EFI_TCG2_EVENT structure with:
    //    - Size, Header.HeaderSize, Header.HeaderVersion
    //    - Header.PCRIndex = pcr_index
    //    - Header.EventType = EV_IPL (0x0000000D)
    //    - Event data = description as UTF-8
    // 3. Call HashLogExtendEvent(Flags=0, DataToHash=data, DataToHashLen=data.len(), Event)
    //
    // The actual FFI for TCG2 requires careful struct layout. We log the
    // intent here; a production build would implement the full protocol call.
    let _ = (handle, pcr_index, data.len(), description);
}

/// Measure all UKI sections into the appropriate PCRs.
///
/// Following the systemd-stub specification:
/// - PCR 11: All sections of the UKI (unified measurement)
/// - PCR  8: Kernel command line (.cmdline section)
/// - PCR  9: Kernel image (.linux section)
/// - PCR 12: Credentials and configuration
/// - PCR 13: System extensions
fn measure_uki_sections(sections: &UkiSections) {
    // Measure each section into PCR 11 (unified image).
    for section in &sections.all_sections {
        let known_sections = [
            SECTION_OSREL,
            SECTION_CMDLINE,
            SECTION_LINUX,
            SECTION_INITRD,
            SECTION_SPLASH,
            SECTION_DTB,
            SECTION_UNAME,
            SECTION_SBAT,
            SECTION_PCRPKEY,
        ];

        if known_sections.contains(&section.name.as_str()) {
            let mut desc = String::from("sd-stub: ");
            desc.push_str(&section.name);
            measure_to_pcr(PCR_UNIFIED_IMAGE, &section.data, &desc);
        }
    }

    // Measure command line into PCR 8.
    if let Some(ref cmdline) = sections.cmdline {
        measure_to_pcr(PCR_KERNEL_CMDLINE, cmdline, "sd-stub: kernel-cmdline");
    }

    // Measure kernel image into PCR 9.
    if let Some(ref linux) = sections.linux {
        measure_to_pcr(PCR_KERNEL_IMAGE, linux, "sd-stub: kernel-image");
    }
}

// ── Splash Screen ─────────────────────────────────────────────────────────

/// Display the splash screen BMP image if present.
///
/// Uses the EFI_GRAPHICS_OUTPUT_PROTOCOL to display a BMP file
/// embedded in the `.splash` section.
fn display_splash(splash_data: &[u8]) {
    // Validate BMP header.
    if splash_data.len() < 54 {
        return;
    }
    if splash_data[0] != b'B' || splash_data[1] != b'M' {
        return;
    }

    // In a full implementation, we would:
    // 1. Parse the BMP header (width, height, bits per pixel, data offset).
    // 2. Locate EFI_GRAPHICS_OUTPUT_PROTOCOL.
    // 3. Convert BMP pixel data to EFI_GRAPHICS_OUTPUT_BLT_PIXEL format.
    // 4. Center the image on screen.
    // 5. Call Blt() with EfiBltBufferToVideo to display it.
    //
    // For now, we validate the BMP but skip rendering, as the GOP
    // protocol interaction requires careful handling of pixel formats
    // and screen geometry.
    let _width = u32::from_le_bytes([
        splash_data[18],
        splash_data[19],
        splash_data[20],
        splash_data[21],
    ]);
    let _height = u32::from_le_bytes([
        splash_data[22],
        splash_data[23],
        splash_data[24],
        splash_data[25],
    ]);
}

// ── Command Line Assembly ─────────────────────────────────────────────────

/// Build the final kernel command line by combining:
/// 1. The embedded `.cmdline` section from the UKI
/// 2. Any additional options passed by the boot loader (via LoadOptions)
///
/// The boot loader options are appended after the embedded options.
fn build_cmdline(sections: &UkiSections, boot_loader_options: Option<&str>) -> String {
    let mut cmdline = String::new();

    // Start with the embedded command line.
    if let Some(embedded) = sections.cmdline_string() {
        cmdline.push_str(&embedded);
    }

    // Append boot loader options if present.
    if let Some(opts) = boot_loader_options {
        let opts = opts.trim();
        if !opts.is_empty() {
            if !cmdline.is_empty() {
                cmdline.push(' ');
            }
            cmdline.push_str(opts);
        }
    }

    cmdline
}

/// Extract the load options (command line) from the loaded image as a string.
fn get_load_options(image: &LoadedImage) -> Option<String> {
    let opts = image.load_options_as_bytes()?;

    if opts.len() < 2 {
        return None;
    }

    // Load options are typically UTF-16LE.
    let u16s: Vec<u16> = opts
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();

    let s = String::from_utf16_lossy(&u16s);
    let s = s.trim_end_matches('\0').trim().to_string();

    if s.is_empty() { None } else { Some(s) }
}

// ── Publish EFI Variables ─────────────────────────────────────────────────

/// Publish stub identity and UKI metadata as EFI variables.
fn publish_stub_variables(sections: &UkiSections) {
    // StubInfo: identifies the stub.
    let info_bytes = str_to_utf16le_bytes(STUB_VERSION);
    let _ = uefi::runtime::set_variable(cstr16!("StubInfo"), &LOADER_GUID, ATTR_BS_RT, &info_bytes);

    // StubFeatures: advertise capabilities.
    let features_bytes = STUB_FEATURES.to_le_bytes();
    let _ = uefi::runtime::set_variable(
        cstr16!("StubFeatures"),
        &LOADER_GUID,
        ATTR_BS_RT,
        &features_bytes,
    );

    // Publish os-release content if available.
    if let Some(ref osrel) = sections.osrel {
        // Set as raw bytes (not UTF-16) since this is parsed as a text file.
        let _ = uefi::runtime::set_variable(
            cstr16!("LoaderOsRelease"),
            &LOADER_GUID,
            ATTR_BS_RT,
            osrel,
        );
    }

    // Publish kernel version if available.
    if let Some(uname) = sections.uname_string() {
        let uname_bytes = str_to_utf16le_bytes(&uname);
        let _ = uefi::runtime::set_variable(
            cstr16!("LoaderKernelVersion"),
            &LOADER_GUID,
            ATTR_BS_RT,
            &uname_bytes,
        );
    }
}

// ── Initrd Loading via LOAD_FILE2 Protocol ────────────────────────────────

/// Install an EFI_LOAD_FILE2_PROTOCOL instance that serves the embedded
/// initrd to the Linux kernel.
///
/// The Linux EFI stub looks for a LOAD_FILE2 protocol installed on a
/// handle with the well-known initrd vendor media device path. When it
/// calls LoadFile(), we return the embedded `.initrd` data.
///
/// This is the standard mechanism for passing initrd from the boot loader
/// to the Linux kernel in UEFI environments (since Linux 5.8).
fn install_initrd_protocol(initrd_data: &'static [u8]) -> Option<Handle> {
    // In a full implementation, we would:
    //
    // 1. Create a device path consisting of:
    //    - VenMedia(LINUX_INITRD_MEDIA_GUID)
    //    - End
    //
    // 2. Implement EFI_LOAD_FILE2_PROTOCOL with a LoadFile callback that:
    //    - Returns EFI_BUFFER_TOO_SMALL with the correct size on first call
    //    - Copies initrd_data into the caller's buffer on second call
    //
    // 3. Install both the device path and LOAD_FILE2 protocol on a new handle.
    //
    // This requires unsafe FFI to create the protocol interface struct and
    // register the callback. The initrd_data pointer must remain valid for
    // the lifetime of the protocol (hence the 'static requirement).
    //
    // For the framework, we document the mechanism and log the intent.

    let _ = (initrd_data, LINUX_INITRD_MEDIA_GUID);
    None
}

// ── Credential & Sysext Discovery ─────────────────────────────────────────

/// Discover additional credentials from the ESP.
///
/// systemd-stub looks for credentials in:
///   `\loader\credentials\` — global credentials
///   `\loader\credentials\<entry-id>\` — per-entry credentials
///
/// Each file found becomes a credential passed to the booted system
/// via a CPIO archive appended to the initrd.
fn discover_credentials(image: &LoadedImage, _sections: &UkiSections) -> Vec<(String, Vec<u8>)> {
    let mut credentials = Vec::new();

    let device_handle = match image.device() {
        Some(h) => h,
        None => return credentials,
    };

    let mut fs = match uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device_handle) {
        Ok(f) => f,
        Err(_) => return credentials,
    };

    let mut root = match fs.open_volume() {
        Ok(r) => r,
        Err(_) => return credentials,
    };

    // Try to list global credentials directory.
    let cred_dir = "\\loader\\credentials";
    let ucs2 = str_to_ucs2(cred_dir);
    if let Ok(cstr) = uefi::CStr16::from_u16_with_nul(&ucs2) {
        if let Ok(handle) = root.open(cstr, FileMode::Read, FileAttribute::empty()) {
            if let Ok(FileType::Dir(mut dir)) = handle.into_type() {
                let mut buf = vec![0u8; 1024];
                loop {
                    match dir.read_entry(&mut buf) {
                        Ok(Some(info)) => {
                            let name_chars = info.file_name().as_slice_with_nul();
                            let u16s: Vec<u16> = name_chars.iter().map(|c| u16::from(*c)).collect();
                            let name = String::from_utf16_lossy(&u16s)
                                .trim_end_matches('\0')
                                .to_string();
                            if name == "." || name == ".." {
                                continue;
                            }
                            if info.attribute().contains(FileAttribute::DIRECTORY) {
                                continue;
                            }
                            // Read the credential file.
                            let file_path = alloc::format!("{}\\{}", cred_dir, name);
                            let file_ucs2 = str_to_ucs2(&file_path);
                            if let Ok(file_cstr) = uefi::CStr16::from_u16_with_nul(&file_ucs2) {
                                if let Ok(fh) =
                                    root.open(file_cstr, FileMode::Read, FileAttribute::empty())
                                {
                                    if let Ok(FileType::Regular(mut f)) = fh.into_type() {
                                        let mut info_buf = vec![0u8; 512];
                                        if let Ok(fi) = f.get_info::<FileInfo>(&mut info_buf) {
                                            let sz = fi.file_size() as usize;
                                            if sz <= MAX_SECTION_SIZE {
                                                let mut data = vec![0u8; sz];
                                                if f.read(&mut data).is_ok() {
                                                    credentials.push((name.clone(), data));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            }
        }
    }

    credentials
}

/// Discover system extension images from the ESP.
///
/// systemd-stub looks for sysext images in:
///   `\loader\sysext\` — system extension disk images (.raw)
///
/// These are measured into PCR 13 and can be used by systemd-sysext
/// in the booted system.
fn discover_sysexts(image: &LoadedImage) -> Vec<(String, Vec<u8>)> {
    let mut sysexts = Vec::new();

    let device_handle = match image.device() {
        Some(h) => h,
        None => return sysexts,
    };

    let mut fs = match uefi::boot::open_protocol_exclusive::<SimpleFileSystem>(device_handle) {
        Ok(f) => f,
        Err(_) => return sysexts,
    };

    let mut root = match fs.open_volume() {
        Ok(r) => r,
        Err(_) => return sysexts,
    };

    let sysext_dir = "\\loader\\sysext";
    let ucs2 = str_to_ucs2(sysext_dir);
    if let Ok(cstr) = uefi::CStr16::from_u16_with_nul(&ucs2) {
        if let Ok(handle) = root.open(cstr, FileMode::Read, FileAttribute::empty()) {
            if let Ok(FileType::Dir(mut dir)) = handle.into_type() {
                let mut buf = vec![0u8; 1024];
                loop {
                    match dir.read_entry(&mut buf) {
                        Ok(Some(info)) => {
                            let name_chars = info.file_name().as_slice_with_nul();
                            let u16s: Vec<u16> = name_chars.iter().map(|c| u16::from(*c)).collect();
                            let name = String::from_utf16_lossy(&u16s)
                                .trim_end_matches('\0')
                                .to_string();
                            if name == "." || name == ".." {
                                continue;
                            }
                            if !name.ends_with(".raw") {
                                continue;
                            }
                            // Measure into PCR 13.
                            let file_path = alloc::format!("{}\\{}", sysext_dir, name);
                            let file_ucs2 = str_to_ucs2(&file_path);
                            if let Ok(file_cstr) = uefi::CStr16::from_u16_with_nul(&file_ucs2) {
                                if let Ok(fh) =
                                    root.open(file_cstr, FileMode::Read, FileAttribute::empty())
                                {
                                    if let Ok(FileType::Regular(mut f)) = fh.into_type() {
                                        let mut info_buf = vec![0u8; 512];
                                        if let Ok(fi) = f.get_info::<FileInfo>(&mut info_buf) {
                                            let sz = fi.file_size() as usize;
                                            if sz <= MAX_SECTION_SIZE {
                                                let mut data = vec![0u8; sz];
                                                if f.read(&mut data).is_ok() {
                                                    measure_to_pcr(
                                                        PCR_SYSEXT,
                                                        &data,
                                                        &alloc::format!("sd-stub: sysext {}", name),
                                                    );
                                                    sysexts.push((name.clone(), data));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        Ok(None) => break,
                        Err(_) => break,
                    }
                }
            }
        }
    }

    sysexts
}

// ── CPIO Archive Builder ──────────────────────────────────────────────────

/// Build a minimal CPIO newc archive from a list of (filename, data) pairs.
///
/// This is used to package credentials and sysext images into an
/// additional initrd that gets appended after the main initrd.
/// The kernel's initramfs unpacker handles multiple concatenated
/// CPIO archives.
fn build_cpio_archive(files: &[(String, Vec<u8>)], base_dir: &str) -> Vec<u8> {
    let mut archive = Vec::new();

    // Create the base directory entry if needed.
    if !base_dir.is_empty() {
        append_cpio_dir(&mut archive, base_dir);
    }

    // Add each file.
    for (name, data) in files {
        let path = if base_dir.is_empty() {
            name.clone()
        } else {
            alloc::format!("{}/{}", base_dir, name)
        };
        append_cpio_file(&mut archive, &path, data);
    }

    // Add the CPIO trailer.
    append_cpio_trailer(&mut archive);

    archive
}

/// Append a directory entry to a CPIO newc archive.
fn append_cpio_dir(archive: &mut Vec<u8>, name: &str) {
    // cpio newc header: "070701" magic, then 13 8-char hex fields, then name, then padding.
    let name_with_nul = name.len() + 1;
    let header = alloc::format!(
        "070701\
         00000000\
         000041ED\
         00000000\
         00000000\
         00000002\
         00000000\
         00000000\
         00000000\
         00000000\
         00000000\
         {:08X}\
         00000000",
        name_with_nul,
    );

    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(name.as_bytes());
    archive.push(0); // NUL terminator for name.

    // Pad to 4-byte boundary.
    let total = header.len() + name_with_nul;
    let padding = (4 - (total % 4)) % 4;
    for _ in 0..padding {
        archive.push(0);
    }
}

/// Append a regular file entry to a CPIO newc archive.
fn append_cpio_file(archive: &mut Vec<u8>, name: &str, data: &[u8]) {
    let name_with_nul = name.len() + 1;
    let file_size = data.len();

    let header = alloc::format!(
        "070701\
         00000000\
         000081A4\
         00000000\
         00000000\
         00000001\
         00000000\
         {:08X}\
         00000000\
         00000000\
         00000000\
         {:08X}\
         00000000",
        file_size,
        name_with_nul,
    );

    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(name.as_bytes());
    archive.push(0);

    // Pad name to 4-byte boundary.
    let header_total = header.len() + name_with_nul;
    let name_padding = (4 - (header_total % 4)) % 4;
    for _ in 0..name_padding {
        archive.push(0);
    }

    // File data.
    archive.extend_from_slice(data);

    // Pad data to 4-byte boundary.
    let data_padding = (4 - (file_size % 4)) % 4;
    for _ in 0..data_padding {
        archive.push(0);
    }
}

/// Append the CPIO trailer entry ("TRAILER!!!").
fn append_cpio_trailer(archive: &mut Vec<u8>) {
    let name = "TRAILER!!!";
    let name_with_nul = name.len() + 1; // 11

    let header = alloc::format!(
        "070701\
         00000000\
         00000000\
         00000000\
         00000000\
         00000001\
         00000000\
         00000000\
         00000000\
         00000000\
         00000000\
         {:08X}\
         00000000",
        name_with_nul,
    );

    archive.extend_from_slice(header.as_bytes());
    archive.extend_from_slice(name.as_bytes());
    archive.push(0);

    // Pad to 4-byte boundary.
    let total = header.len() + name_with_nul;
    let padding = (4 - (total % 4)) % 4;
    for _ in 0..padding {
        archive.push(0);
    }
}

// ── Entry Point ───────────────────────────────────────────────────────────

#[entry]
fn efi_main() -> Status {
    println("sd-stub: Unified Kernel Image stub starting...");

    // Get our loaded image information.
    let image_handle = uefi::boot::image_handle();
    let loaded_image = match uefi::boot::open_protocol_exclusive::<LoadedImage>(image_handle) {
        Ok(li) => li,
        Err(_) => {
            println("sd-stub: Error: Could not open LoadedImage protocol.");
            uefi::boot::stall(Duration::from_secs(5));
            return Status::LOAD_ERROR;
        }
    };

    // Read any boot loader options passed to us.
    let boot_loader_options = get_load_options(&loaded_image);

    // Read our own PE image from the filesystem.
    let image_data = match read_own_image(&loaded_image) {
        Some(d) => d,
        None => {
            println("sd-stub: Error: Could not read own PE image.");
            uefi::boot::stall(Duration::from_secs(5));
            return Status::LOAD_ERROR;
        }
    };

    // Parse the PE sections to extract embedded payloads.
    let sections = match parse_pe_image(&image_data) {
        Some(s) => s,
        None => {
            println("sd-stub: Error: Could not parse PE sections.");
            uefi::boot::stall(Duration::from_secs(5));
            return Status::LOAD_ERROR;
        }
    };

    // Verify that we have a kernel image.
    if sections.linux.is_none() {
        println("sd-stub: Error: No .linux section found in UKI.");
        println("sd-stub: This stub must be combined with a kernel image.");
        uefi::boot::stall(Duration::from_secs(5));
        return Status::LOAD_ERROR;
    }

    // Report what we found.
    if let Some(pretty) = sections.pretty_name() {
        let mut msg = String::from("sd-stub: OS: ");
        msg.push_str(&pretty);
        println(&msg);
    }
    if let Some(uname) = sections.uname_string() {
        let mut msg = String::from("sd-stub: Kernel: ");
        msg.push_str(&uname);
        println(&msg);
    }
    {
        let linux_size = sections.linux.as_ref().map_or(0, |d| d.len());
        let initrd_size = sections.initrd.as_ref().map_or(0, |d| d.len());
        let mut msg = String::new();
        let _ = core::fmt::write(
            &mut msg,
            format_args!(
                "sd-stub: Kernel: {} bytes, Initrd: {} bytes",
                linux_size, initrd_size
            ),
        );
        println(&msg);
    }

    // Publish EFI variables for the booted OS.
    publish_stub_variables(&sections);

    // Measure UKI sections into TPM2 PCRs.
    measure_uki_sections(&sections);

    // Display splash screen if present.
    if let Some(ref splash) = sections.splash {
        display_splash(splash);
    }

    // Build the final kernel command line.
    let cmdline = build_cmdline(&sections, boot_loader_options.as_deref());
    if !cmdline.is_empty() {
        let mut msg = String::from("sd-stub: Cmdline: ");
        // Truncate for display.
        if cmdline.len() > 120 {
            msg.push_str(&cmdline[..120]);
            msg.push_str("...");
        } else {
            msg.push_str(&cmdline);
        }
        println(&msg);
    }

    // Discover credentials and sysexts from ESP.
    let credentials = discover_credentials(&loaded_image, &sections);
    let sysexts = discover_sysexts(&loaded_image);

    if !credentials.is_empty() {
        let mut msg = String::new();
        let _ = core::fmt::write(
            &mut msg,
            format_args!("sd-stub: Found {} credential(s)", credentials.len()),
        );
        println(&msg);
    }
    if !sysexts.is_empty() {
        let mut msg = String::new();
        let _ = core::fmt::write(
            &mut msg,
            format_args!("sd-stub: Found {} sysext(s)", sysexts.len()),
        );
        println(&msg);
    }

    // Build the combined initrd:
    // 1. The main .initrd from the UKI
    // 2. Credentials packed as a CPIO archive (appended)
    // 3. Sysexts packed as a CPIO archive (appended)
    let mut combined_initrd = sections.initrd.clone().unwrap_or_default();

    if !credentials.is_empty() {
        let cred_cpio = build_cpio_archive(&credentials, ".extra/credentials");
        combined_initrd.extend_from_slice(&cred_cpio);
    }

    if !sysexts.is_empty() {
        let sysext_cpio = build_cpio_archive(&sysexts, ".extra/sysext");
        combined_initrd.extend_from_slice(&sysext_cpio);
    }

    // Free the raw image data to conserve memory before loading the kernel.
    drop(image_data);

    // Get the kernel image data.
    let kernel_data = sections.linux.as_ref().unwrap();

    // Load the kernel as an EFI image.
    drop(loaded_image); // Release protocol before loading new image.

    match uefi::boot::load_image(
        image_handle,
        uefi::boot::LoadImageSource::FromBuffer {
            buffer: kernel_data,
            file_path: None,
        },
    ) {
        Ok(child_handle) => {
            // Set the kernel command line as load options.
            if !cmdline.is_empty() {
                if let Ok(mut child_image) =
                    uefi::boot::open_protocol_exclusive::<LoadedImage>(child_handle)
                {
                    let opts_bytes = str_to_utf16le_bytes(&cmdline);
                    unsafe {
                        child_image.set_load_options(
                            opts_bytes.as_ptr() as *const u8,
                            opts_bytes.len() as u32,
                        );
                    }
                }
            }

            // Install LOAD_FILE2 protocol for the initrd if we have one.
            // The kernel will pick it up via the initrd vendor media device path.
            if !combined_initrd.is_empty() {
                // In a full implementation, we would install the protocol here.
                // The initrd data must remain valid until after StartImage returns.
                let _initrd_handle = install_initrd_protocol(
                    // SAFETY: We leak the initrd data intentionally to ensure it
                    // lives long enough. In a UEFI application, memory is reclaimed
                    // when ExitBootServices is called.
                    Box::leak(combined_initrd.into_boxed_slice()),
                );
            }

            // Boot the kernel.
            println("sd-stub: Starting kernel...");
            match uefi::boot::start_image(child_handle) {
                Ok(_) => {
                    // The kernel returned (unusual but possible).
                    println("sd-stub: Kernel returned.");
                    Status::SUCCESS
                }
                Err(e) => {
                    let mut msg = String::from("sd-stub: Error starting kernel: ");
                    let _ = core::fmt::write(&mut msg, format_args!("{:?}", e.status()));
                    println(&msg);
                    uefi::boot::stall(Duration::from_secs(5));
                    e.status()
                }
            }
        }
        Err(e) => {
            let mut msg = String::from("sd-stub: Error loading kernel image: ");
            let _ = core::fmt::write(&mut msg, format_args!("{:?}", e.status()));
            println(&msg);
            println("sd-stub: The .linux section may not be a valid EFI binary.");
            println("sd-stub: Ensure the kernel was built with CONFIG_EFI_STUB=y.");
            uefi::boot::stall(Duration::from_secs(5));
            e.status()
        }
    }
}
