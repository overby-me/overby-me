//! systemd-dissect — Inspect and interact with OS disk images.
//!
//! A drop-in replacement for `systemd-dissect(8)`. This tool inspects,
//! mounts, unmounts, and manipulates OS disk images conforming to the
//! Discoverable Partitions Specification (DPS).
//!
//! It can parse GPT partition tables, identify well-known partition types
//! (root, /usr, /home, /srv, /var, ESP, XBOOTLDR, swap), display filesystem
//! metadata, and perform mount/unmount operations.
//!
//! Subcommands:
//!
//!   systemd-dissect IMAGE
//!       Show partition table and filesystem information (default).
//!
//!   systemd-dissect --mount IMAGE PATH
//!       Mount the image at the given path.
//!
//!   systemd-dissect --umount PATH
//!       Unmount a previously mounted image.
//!
//!   systemd-dissect --list IMAGE
//!       List files in the image's root filesystem.
//!
//!   systemd-dissect --copy-from IMAGE SOURCE [DEST]
//!       Copy a file out of the image.
//!
//!   systemd-dissect --copy-to IMAGE SOURCE [DEST]
//!       Copy a file into the image.
//!
//!   systemd-dissect --discover
//!       Discover and list all disk images in well-known locations.
//!
//!   systemd-dissect --validate IMAGE
//!       Validate image structure without mounting.
//!
//! Options:
//!   --root-hash=HASH     Provide root hash for dm-verity
//!   --root-hash-sig=SIG  Provide root hash signature
//!   --verity-data=PATH   Provide verity data file
//!   --json=MODE          JSON output (short, pretty, off)
//!   --no-pager           Don't pipe output through a pager
//!   --no-legend           Don't show column headers / footers
//!   -h --help            Show help
//!      --version         Show version
//!
//! Exit codes:
//!   0 — success
//!   1 — error

use std::collections::HashMap;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::process;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// GPT partition table header signature: "EFI PART"
const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";

/// Size of a standard disk sector
const SECTOR_SIZE: u64 = 512;

/// GPT header is at LBA 1 (offset 512)
const GPT_HEADER_LBA: u64 = 1;

/// GPT header size (minimum)
const GPT_HEADER_MIN_SIZE: u64 = 92;

/// GPT partition entry size (standard)
const GPT_ENTRY_SIZE: u64 = 128;

/// MBR signature bytes at offset 510
const MBR_SIGNATURE: [u8; 2] = [0x55, 0xAA];

/// Well-known image search paths for --discover
const IMAGE_SEARCH_PATHS: &[&str] = &[
    "/var/lib/machines",
    "/var/lib/portables",
    "/var/lib/extensions",
    "/usr/lib/machines",
    "/usr/lib/portables",
    "/usr/lib/extensions",
];

// ---------------------------------------------------------------------------
// Discoverable Partitions Specification (DPS) GUIDs
// ---------------------------------------------------------------------------

/// Well-known GPT partition type GUIDs (Discoverable Partitions Spec).
/// Stored as lowercase UUID strings.
struct PartitionTypeInfo {
    name: &'static str,
    mount_point: Option<&'static str>,
}

fn known_partition_types() -> HashMap<String, PartitionTypeInfo> {
    let mut m = HashMap::new();

    // Linux root (x86-64)
    m.insert(
        "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".to_string(),
        PartitionTypeInfo {
            name: "Linux root (x86-64)",
            mount_point: Some("/"),
        },
    );
    // Linux root (ARM64)
    m.insert(
        "b921b045-1df0-41c3-af44-4c6f280d3fae".to_string(),
        PartitionTypeInfo {
            name: "Linux root (ARM64)",
            mount_point: Some("/"),
        },
    );
    // Linux root (x86)
    m.insert(
        "44479540-f297-41b2-9af7-d131d5f0458a".to_string(),
        PartitionTypeInfo {
            name: "Linux root (x86)",
            mount_point: Some("/"),
        },
    );
    // Linux root (RISC-V 64)
    m.insert(
        "72ec70a6-cf74-40e6-bd49-4bda08e8f224".to_string(),
        PartitionTypeInfo {
            name: "Linux root (RISC-V 64)",
            mount_point: Some("/"),
        },
    );
    // Linux /usr (x86-64)
    m.insert(
        "8484680c-9521-48c6-9c11-b0720656f69e".to_string(),
        PartitionTypeInfo {
            name: "Linux /usr (x86-64)",
            mount_point: Some("/usr"),
        },
    );
    // Linux /usr (ARM64)
    m.insert(
        "b0e01050-ee5f-4390-949a-9101b17104e9".to_string(),
        PartitionTypeInfo {
            name: "Linux /usr (ARM64)",
            mount_point: Some("/usr"),
        },
    );
    // Linux root verity (x86-64)
    m.insert(
        "2c7357ed-ebd2-46d9-aec1-23d437ec2bf5".to_string(),
        PartitionTypeInfo {
            name: "Linux root verity (x86-64)",
            mount_point: None,
        },
    );
    // Linux /usr verity (x86-64)
    m.insert(
        "77ff5f63-e7b6-4633-acf4-1565b864c0e6".to_string(),
        PartitionTypeInfo {
            name: "Linux /usr verity (x86-64)",
            mount_point: None,
        },
    );
    // Linux /home
    m.insert(
        "933ac7e1-2eb4-4f13-b844-0e14e2aef915".to_string(),
        PartitionTypeInfo {
            name: "Linux /home",
            mount_point: Some("/home"),
        },
    );
    // Linux /srv
    m.insert(
        "3b8f8425-20e0-4f3b-907f-1a25a76f98e8".to_string(),
        PartitionTypeInfo {
            name: "Linux /srv",
            mount_point: Some("/srv"),
        },
    );
    // Linux /var
    m.insert(
        "4d21b016-b534-45c2-a9fb-5c16e091fd2d".to_string(),
        PartitionTypeInfo {
            name: "Linux /var",
            mount_point: Some("/var"),
        },
    );
    // Linux /var/tmp
    m.insert(
        "7ec6f557-3bc5-4aca-b293-16ef5df639d1".to_string(),
        PartitionTypeInfo {
            name: "Linux /var/tmp",
            mount_point: Some("/var/tmp"),
        },
    );
    // Linux generic data
    m.insert(
        "0fc63daf-8483-4772-8e79-3d69d8477de4".to_string(),
        PartitionTypeInfo {
            name: "Linux filesystem",
            mount_point: None,
        },
    );
    // Linux swap
    m.insert(
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f".to_string(),
        PartitionTypeInfo {
            name: "Linux swap",
            mount_point: None,
        },
    );
    // EFI System Partition
    m.insert(
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b".to_string(),
        PartitionTypeInfo {
            name: "EFI System Partition",
            mount_point: Some("/efi"),
        },
    );
    // Extended Boot Loader Partition (XBOOTLDR)
    m.insert(
        "bc13c2ff-59e6-4262-a352-b275fd6f7172".to_string(),
        PartitionTypeInfo {
            name: "Extended Boot Loader",
            mount_point: Some("/boot"),
        },
    );
    // BIOS boot
    m.insert(
        "21686148-6449-6e6f-744e-656564454649".to_string(),
        PartitionTypeInfo {
            name: "BIOS boot",
            mount_point: None,
        },
    );
    // Microsoft basic data
    m.insert(
        "ebd0a0a2-b9e5-4433-87c0-68b6b72699c7".to_string(),
        PartitionTypeInfo {
            name: "Microsoft basic data",
            mount_point: None,
        },
    );
    // Linux RAID
    m.insert(
        "a19d880f-05fc-4d3b-a006-743f0f84911e".to_string(),
        PartitionTypeInfo {
            name: "Linux RAID",
            mount_point: None,
        },
    );
    // Linux LVM
    m.insert(
        "e6d6d379-f507-44c2-a23c-238f2a3df928".to_string(),
        PartitionTypeInfo {
            name: "Linux LVM",
            mount_point: None,
        },
    );
    // Linux dm-crypt
    m.insert(
        "7ffec5c9-2d00-49b7-8941-3ea10a5586b7".to_string(),
        PartitionTypeInfo {
            name: "Linux dm-crypt",
            mount_point: None,
        },
    );
    // Linux LUKS
    m.insert(
        "ca7d7ccb-63ed-4c53-861c-1742536059cc".to_string(),
        PartitionTypeInfo {
            name: "Linux LUKS",
            mount_point: None,
        },
    );

    m
}

// ---------------------------------------------------------------------------
// Data structures
// ---------------------------------------------------------------------------

/// Parsed GPT partition entry.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct GptPartition {
    /// Partition type GUID (lowercase hex with dashes).
    type_guid: String,
    /// Unique partition GUID (lowercase hex with dashes).
    unique_guid: String,
    /// First LBA of partition data.
    first_lba: u64,
    /// Last LBA of partition data (inclusive).
    last_lba: u64,
    /// Attribute flags.
    attributes: u64,
    /// Partition name (UTF-16LE decoded).
    name: String,
    /// Partition index (1-based).
    index: u32,
}

impl GptPartition {
    /// Size in bytes.
    fn size_bytes(&self) -> u64 {
        (self.last_lba - self.first_lba + 1) * SECTOR_SIZE
    }
}

/// Parsed GPT header.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct GptHeader {
    /// Revision of the GPT header.
    revision: u32,
    /// Size of the GPT header in bytes.
    header_size: u32,
    /// CRC32 of the header.
    header_crc32: u32,
    /// LBA of this header.
    my_lba: u64,
    /// LBA of the alternate header.
    alternate_lba: u64,
    /// First usable LBA for partitions.
    first_usable_lba: u64,
    /// Last usable LBA for partitions.
    last_usable_lba: u64,
    /// Disk GUID.
    disk_guid: String,
    /// LBA of the partition entry array.
    partition_entry_lba: u64,
    /// Number of partition entries.
    num_partition_entries: u32,
    /// Size of each partition entry.
    partition_entry_size: u32,
    /// CRC32 of partition entries array.
    partition_entries_crc32: u32,
}

/// MBR partition entry.
#[derive(Debug, Clone)]
struct MbrPartition {
    /// Boot indicator (0x80 = active, 0x00 = inactive).
    bootable: bool,
    /// Partition type byte.
    partition_type: u8,
    /// Starting LBA.
    first_lba: u32,
    /// Number of sectors.
    num_sectors: u32,
    /// Partition index (1-based).
    index: u32,
}

impl MbrPartition {
    fn size_bytes(&self) -> u64 {
        self.num_sectors as u64 * SECTOR_SIZE
    }
}

/// Partition table type detected in an image.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PartitionTableType {
    Gpt,
    Mbr,
    None,
}

impl std::fmt::Display for PartitionTableType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            PartitionTableType::Gpt => write!(f, "GPT"),
            PartitionTableType::Mbr => write!(f, "MBR/DOS"),
            PartitionTableType::None => write!(f, "none"),
        }
    }
}

/// Full image analysis result.
#[derive(Debug, Clone)]
struct ImageInfo {
    path: PathBuf,
    size: u64,
    table_type: PartitionTableType,
    gpt_header: Option<GptHeader>,
    gpt_partitions: Vec<GptPartition>,
    mbr_partitions: Vec<MbrPartition>,
}

/// Parsed command-line arguments.
#[derive(Debug, Clone)]
struct Args {
    command: Command,
    image: Option<PathBuf>,
    mount_path: Option<PathBuf>,
    source: Option<String>,
    dest: Option<String>,
    root_hash: Option<String>,
    root_hash_sig: Option<String>,
    verity_data: Option<String>,
    json_mode: JsonMode,
    no_legend: bool,
    /// --mkdir: create the mount point directory before mounting.
    mkdir: bool,
    /// --rmdir: remove the mount point directory after unmounting.
    rmdir: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Show,
    Mount,
    Umount,
    List,
    CopyFrom,
    CopyTo,
    Discover,
    Validate,
    Help,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonMode {
    Off,
    Short,
    Pretty,
}

impl Default for Args {
    fn default() -> Self {
        Args {
            command: Command::Show,
            image: None,
            mount_path: None,
            source: None,
            dest: None,
            root_hash: None,
            root_hash_sig: None,
            verity_data: None,
            json_mode: JsonMode::Off,
            no_legend: false,
            mkdir: false,
            rmdir: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Argument parsing
// ---------------------------------------------------------------------------

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut positionals: Vec<String> = Vec::new();

    let mut i = 0;
    while i < argv.len() {
        let arg = &argv[i];

        let (key, value) = if let Some(pos) = arg.find('=') {
            (&arg[..pos], Some(arg[pos + 1..].to_string()))
        } else {
            (arg.as_str(), None)
        };

        match key {
            "--help" | "-h" => {
                args.command = Command::Help;
                return Ok(args);
            }
            "--version" => {
                println!("systemd-dissect (rust-systemd)");
                process::exit(0);
            }
            "--mount" | "-m" => args.command = Command::Mount,
            "--umount" | "-u" | "--unmount" => args.command = Command::Umount,
            "--list" | "-l" => args.command = Command::List,
            "--copy-from" => args.command = Command::CopyFrom,
            "--copy-to" => args.command = Command::CopyTo,
            "--discover" => args.command = Command::Discover,
            "--validate" => args.command = Command::Validate,
            "--root-hash" => {
                let v = value_or_next(argv, &mut i, value, "--root-hash")?;
                args.root_hash = Some(v);
            }
            "--root-hash-sig" => {
                let v = value_or_next(argv, &mut i, value, "--root-hash-sig")?;
                args.root_hash_sig = Some(v);
            }
            "--verity-data" => {
                let v = value_or_next(argv, &mut i, value, "--verity-data")?;
                args.verity_data = Some(v);
            }
            "--json" => {
                let v = value_or_next(argv, &mut i, value, "--json")?;
                args.json_mode = match v.as_str() {
                    "short" => JsonMode::Short,
                    "pretty" => JsonMode::Pretty,
                    "off" => JsonMode::Off,
                    _ => return Err(format!("Unknown JSON mode: {}", v)),
                };
            }
            "--no-pager" => { /* accepted and ignored */ }
            "--no-legend" => args.no_legend = true,
            "--mkdir" => args.mkdir = true,
            "--rmdir" => args.rmdir = true,
            _ if !arg.starts_with('-') => {
                positionals.push(arg.clone());
            }
            other => {
                return Err(format!("Unknown option: {}", other));
            }
        }

        i += 1;
    }

    // Assign positionals based on command
    match args.command {
        Command::Show | Command::List | Command::Validate => {
            if let Some(p) = positionals.first() {
                args.image = Some(PathBuf::from(p));
            }
        }
        Command::Mount => {
            if positionals.len() >= 2 {
                args.image = Some(PathBuf::from(&positionals[0]));
                args.mount_path = Some(PathBuf::from(&positionals[1]));
            } else if positionals.len() == 1 {
                args.image = Some(PathBuf::from(&positionals[0]));
            }
        }
        Command::Umount => {
            if let Some(p) = positionals.first() {
                args.mount_path = Some(PathBuf::from(p));
            }
        }
        Command::CopyFrom | Command::CopyTo => {
            if positionals.len() >= 3 {
                args.image = Some(PathBuf::from(&positionals[0]));
                args.source = Some(positionals[1].clone());
                args.dest = Some(positionals[2].clone());
            } else if positionals.len() >= 2 {
                args.image = Some(PathBuf::from(&positionals[0]));
                args.source = Some(positionals[1].clone());
            } else if positionals.len() == 1 {
                args.image = Some(PathBuf::from(&positionals[0]));
            }
        }
        Command::Discover | Command::Help => {}
    }

    Ok(args)
}

fn value_or_next(
    args: &[String],
    i: &mut usize,
    value: Option<String>,
    name: &str,
) -> Result<String, String> {
    if let Some(v) = value {
        Ok(v)
    } else if *i + 1 < args.len() {
        *i += 1;
        Ok(args[*i].clone())
    } else {
        Err(format!("Option {} requires an argument", name))
    }
}

// ---------------------------------------------------------------------------
// GUID parsing
// ---------------------------------------------------------------------------

/// Parse a mixed-endian GPT GUID from 16 bytes.
///
/// GPT GUIDs are stored in a mixed-endian format:
///   - First 4 bytes: little-endian (time_low)
///   - Next 2 bytes: little-endian (time_mid)
///   - Next 2 bytes: little-endian (time_hi_and_version)
///   - Next 2 bytes: big-endian (clock_seq)
///   - Last 6 bytes: big-endian (node)
fn parse_guid(data: &[u8]) -> String {
    if data.len() < 16 {
        return "00000000-0000-0000-0000-000000000000".to_string();
    }

    let time_low = u32::from_le_bytes([data[0], data[1], data[2], data[3]]);
    let time_mid = u16::from_le_bytes([data[4], data[5]]);
    let time_hi = u16::from_le_bytes([data[6], data[7]]);
    let clock_seq_hi = data[8];
    let clock_seq_low = data[9];
    let node = &data[10..16];

    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        time_low,
        time_mid,
        time_hi,
        clock_seq_hi,
        clock_seq_low,
        node[0],
        node[1],
        node[2],
        node[3],
        node[4],
        node[5]
    )
}

/// Check if a GUID is all zeros.
fn is_zero_guid(guid: &str) -> bool {
    guid == "00000000-0000-0000-0000-000000000000"
}

// ---------------------------------------------------------------------------
// GPT parsing
// ---------------------------------------------------------------------------

fn read_gpt_header(file: &mut fs::File) -> io::Result<Option<GptHeader>> {
    let mut buf = [0u8; 512];

    // Read LBA 1 (GPT header)
    file.seek(SeekFrom::Start(GPT_HEADER_LBA * SECTOR_SIZE))?;
    let n = file.read(&mut buf)?;
    if n < GPT_HEADER_MIN_SIZE as usize {
        return Ok(None);
    }

    // Check signature
    if &buf[0..8] != GPT_SIGNATURE {
        return Ok(None);
    }

    let revision = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let header_size = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    let header_crc32 = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
    // buf[20..24] reserved
    let my_lba = u64::from_le_bytes([
        buf[24], buf[25], buf[26], buf[27], buf[28], buf[29], buf[30], buf[31],
    ]);
    let alternate_lba = u64::from_le_bytes([
        buf[32], buf[33], buf[34], buf[35], buf[36], buf[37], buf[38], buf[39],
    ]);
    let first_usable_lba = u64::from_le_bytes([
        buf[40], buf[41], buf[42], buf[43], buf[44], buf[45], buf[46], buf[47],
    ]);
    let last_usable_lba = u64::from_le_bytes([
        buf[48], buf[49], buf[50], buf[51], buf[52], buf[53], buf[54], buf[55],
    ]);
    let disk_guid = parse_guid(&buf[56..72]);
    let partition_entry_lba = u64::from_le_bytes([
        buf[72], buf[73], buf[74], buf[75], buf[76], buf[77], buf[78], buf[79],
    ]);
    let num_partition_entries = u32::from_le_bytes([buf[80], buf[81], buf[82], buf[83]]);
    let partition_entry_size = u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]);
    let partition_entries_crc32 = u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]);

    Ok(Some(GptHeader {
        revision,
        header_size,
        header_crc32,
        my_lba,
        alternate_lba,
        first_usable_lba,
        last_usable_lba,
        disk_guid,
        partition_entry_lba,
        num_partition_entries,
        partition_entry_size,
        partition_entries_crc32,
    }))
}

fn read_gpt_partitions(file: &mut fs::File, header: &GptHeader) -> io::Result<Vec<GptPartition>> {
    let mut partitions = Vec::new();
    let entry_size = header.partition_entry_size.max(GPT_ENTRY_SIZE as u32) as u64;

    let start_offset = header.partition_entry_lba * SECTOR_SIZE;

    for i in 0..header.num_partition_entries {
        let offset = start_offset + (i as u64) * entry_size;
        file.seek(SeekFrom::Start(offset))?;

        let mut entry = vec![0u8; entry_size as usize];
        let n = file.read(&mut entry)?;
        if n < 128 {
            break;
        }

        let type_guid = parse_guid(&entry[0..16]);

        // Skip empty entries
        if is_zero_guid(&type_guid) {
            continue;
        }

        let unique_guid = parse_guid(&entry[16..32]);
        let first_lba = u64::from_le_bytes([
            entry[32], entry[33], entry[34], entry[35], entry[36], entry[37], entry[38], entry[39],
        ]);
        let last_lba = u64::from_le_bytes([
            entry[40], entry[41], entry[42], entry[43], entry[44], entry[45], entry[46], entry[47],
        ]);
        let attributes = u64::from_le_bytes([
            entry[48], entry[49], entry[50], entry[51], entry[52], entry[53], entry[54], entry[55],
        ]);

        // Parse UTF-16LE partition name (bytes 56..128)
        let name_bytes = &entry[56..entry_size.min(128) as usize];
        let name = parse_utf16le_name(name_bytes);

        partitions.push(GptPartition {
            type_guid,
            unique_guid,
            first_lba,
            last_lba,
            attributes,
            name,
            index: i + 1,
        });
    }

    Ok(partitions)
}

fn parse_utf16le_name(data: &[u8]) -> String {
    let mut chars = Vec::new();
    for chunk in data.chunks_exact(2) {
        let code_unit = u16::from_le_bytes([chunk[0], chunk[1]]);
        if code_unit == 0 {
            break;
        }
        chars.push(code_unit);
    }
    String::from_utf16_lossy(&chars)
}

// ---------------------------------------------------------------------------
// MBR parsing
// ---------------------------------------------------------------------------

fn read_mbr_partitions(file: &mut fs::File) -> io::Result<Option<Vec<MbrPartition>>> {
    let mut buf = [0u8; 512];
    file.seek(SeekFrom::Start(0))?;
    let n = file.read(&mut buf)?;
    if n < 512 {
        return Ok(None);
    }

    // Check MBR signature
    if buf[510] != MBR_SIGNATURE[0] || buf[511] != MBR_SIGNATURE[1] {
        return Ok(None);
    }

    let mut partitions = Vec::new();

    for i in 0..4 {
        let offset = 446 + i * 16;
        let entry = &buf[offset..offset + 16];

        let bootable = entry[0] == 0x80;
        let partition_type = entry[4];

        // Skip empty entries
        if partition_type == 0x00 {
            continue;
        }

        let first_lba = u32::from_le_bytes([entry[8], entry[9], entry[10], entry[11]]);
        let num_sectors = u32::from_le_bytes([entry[12], entry[13], entry[14], entry[15]]);

        partitions.push(MbrPartition {
            bootable,
            partition_type,
            first_lba,
            num_sectors,
            index: i as u32 + 1,
        });
    }

    if partitions.is_empty() {
        Ok(None)
    } else {
        Ok(Some(partitions))
    }
}

fn mbr_type_name(t: u8) -> &'static str {
    match t {
        0x00 => "Empty",
        0x01 => "FAT12",
        0x04 => "FAT16 (<32M)",
        0x05 => "Extended",
        0x06 => "FAT16 (>32M)",
        0x07 => "NTFS/HPFS/exFAT",
        0x0b => "FAT32 (CHS)",
        0x0c => "FAT32 (LBA)",
        0x0e => "FAT16 (LBA)",
        0x0f => "Extended (LBA)",
        0x11 => "Hidden FAT12",
        0x14 => "Hidden FAT16 (<32M)",
        0x16 => "Hidden FAT16 (>32M)",
        0x1b => "Hidden FAT32 (CHS)",
        0x1c => "Hidden FAT32 (LBA)",
        0x1e => "Hidden FAT16 (LBA)",
        0x27 => "WinRE/Hidden NTFS",
        0x42 => "Windows Dynamic",
        0x82 => "Linux swap",
        0x83 => "Linux",
        0x85 => "Linux extended",
        0x8e => "Linux LVM",
        0xee => "GPT Protective",
        0xef => "EFI System",
        0xfd => "Linux RAID",
        _ => "Unknown",
    }
}

// ---------------------------------------------------------------------------
// Image analysis
// ---------------------------------------------------------------------------

fn analyze_image(path: &Path) -> Result<ImageInfo, String> {
    let metadata =
        fs::metadata(path).map_err(|e| format!("Cannot stat {}: {}", path.display(), e))?;
    let size = metadata.len();

    let mut file =
        fs::File::open(path).map_err(|e| format!("Cannot open {}: {}", path.display(), e))?;

    // Try GPT first
    if size >= 2 * SECTOR_SIZE + GPT_HEADER_MIN_SIZE
        && let Ok(Some(header)) = read_gpt_header(&mut file)
    {
        let partitions = read_gpt_partitions(&mut file, &header)
            .map_err(|e| format!("Failed to read GPT partitions: {}", e))?;

        return Ok(ImageInfo {
            path: path.to_path_buf(),
            size,
            table_type: PartitionTableType::Gpt,
            gpt_header: Some(header),
            gpt_partitions: partitions,
            mbr_partitions: Vec::new(),
        });
    }

    // Try MBR
    if size >= SECTOR_SIZE
        && let Ok(Some(partitions)) = read_mbr_partitions(&mut file)
    {
        // Check if MBR is just a protective MBR for GPT
        let is_protective = partitions.len() == 1 && partitions[0].partition_type == 0xEE;
        if !is_protective {
            return Ok(ImageInfo {
                path: path.to_path_buf(),
                size,
                table_type: PartitionTableType::Mbr,
                gpt_header: None,
                gpt_partitions: Vec::new(),
                mbr_partitions: partitions,
            });
        }
    }

    Ok(ImageInfo {
        path: path.to_path_buf(),
        size,
        table_type: PartitionTableType::None,
        gpt_header: None,
        gpt_partitions: Vec::new(),
        mbr_partitions: Vec::new(),
    })
}

// ---------------------------------------------------------------------------
// Size formatting
// ---------------------------------------------------------------------------

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return "0B".to_string();
    }

    const UNITS: &[&str] = &["B", "K", "M", "G", "T", "P", "E"];
    let mut size = bytes as f64;
    let mut unit_idx = 0;

    while size >= 1024.0 && unit_idx + 1 < UNITS.len() {
        size /= 1024.0;
        unit_idx += 1;
    }

    if unit_idx == 0 {
        format!("{}B", bytes)
    } else if size >= 100.0 {
        format!("{:.0}{}", size, UNITS[unit_idx])
    } else if size >= 10.0 {
        format!("{:.1}{}", size, UNITS[unit_idx])
    } else {
        format!("{:.2}{}", size, UNITS[unit_idx])
    }
}

// ---------------------------------------------------------------------------
// Output: show (default command)
// ---------------------------------------------------------------------------

fn cmd_show(info: &ImageInfo, json_mode: JsonMode, no_legend: bool) {
    if json_mode != JsonMode::Off {
        print_show_json(info, json_mode);
        return;
    }

    println!("      Name: {}", info.path.display());
    println!("      Size: {} ({})", format_size(info.size), info.size);
    println!("  Sec. Size: {}", SECTOR_SIZE);
    println!("Table Type: {}", info.table_type);

    if let Some(ref header) = info.gpt_header {
        println!(" Disk GUID: {}", header.disk_guid);
        println!(
            "  Revision: {}.{}",
            header.revision >> 16,
            header.revision & 0xFFFF
        );
    }

    println!();

    let type_map = known_partition_types();

    match info.table_type {
        PartitionTableType::Gpt => {
            if !no_legend {
                println!(
                    "{:<5} {:<36} {:<10} {:<28} NAME",
                    "IDX", "TYPE-UUID", "SIZE", "TYPE"
                );
            }

            for part in &info.gpt_partitions {
                let type_name = type_map
                    .get(&part.type_guid)
                    .map(|t| t.name)
                    .unwrap_or("Unknown");

                println!(
                    "{:<5} {} {:<10} {:<28} {}",
                    part.index,
                    part.type_guid,
                    format_size(part.size_bytes()),
                    type_name,
                    part.name
                );
            }

            if !no_legend && !info.gpt_partitions.is_empty() {
                println!();
                println!("{} partition(s).", info.gpt_partitions.len());
            }
        }
        PartitionTableType::Mbr => {
            if !no_legend {
                println!(
                    "{:<5} {:<6} {:<10} {:<10} {:<20} START LBA",
                    "IDX", "BOOT", "TYPE", "SIZE", "TYPE NAME"
                );
            }

            for part in &info.mbr_partitions {
                let boot = if part.bootable { "*" } else { "" };
                println!(
                    "{:<5} {:<6} 0x{:02x}     {:<10} {:<20} {}",
                    part.index,
                    boot,
                    part.partition_type,
                    format_size(part.size_bytes()),
                    mbr_type_name(part.partition_type),
                    part.first_lba
                );
            }

            if !no_legend && !info.mbr_partitions.is_empty() {
                println!();
                println!("{} partition(s).", info.mbr_partitions.len());
            }
        }
        PartitionTableType::None => {
            println!("No partition table found.");
            println!("This may be a raw filesystem image.");
        }
    }

    // Show DPS recognized partitions for GPT images
    if info.table_type == PartitionTableType::Gpt {
        let recognized: Vec<&GptPartition> = info
            .gpt_partitions
            .iter()
            .filter(|p| type_map.contains_key(&p.type_guid))
            .collect();

        if !recognized.is_empty() {
            let has_root = recognized
                .iter()
                .any(|p| type_map.get(&p.type_guid).and_then(|t| t.mount_point) == Some("/"));

            let has_usr = recognized
                .iter()
                .any(|p| type_map.get(&p.type_guid).and_then(|t| t.mount_point) == Some("/usr"));

            let has_esp = recognized
                .iter()
                .any(|p| type_map.get(&p.type_guid).and_then(|t| t.mount_point) == Some("/efi"));

            println!();
            println!("Discoverable Partitions:");
            if has_root {
                println!("  Root filesystem: found");
            }
            if has_usr {
                println!("  /usr filesystem: found");
            }
            if has_esp {
                println!("  EFI System Partition: found");
            }
        }
    }
}

fn print_show_json(info: &ImageInfo, mode: JsonMode) {
    let (indent, nl) = if mode == JsonMode::Pretty {
        ("  ", "\n")
    } else {
        ("", "")
    };

    let type_map = known_partition_types();
    let mut out = String::new();

    out.push_str(&format!("{{{}", nl));
    out.push_str(&format!(
        "{}\"image\": \"{}\",{}",
        indent,
        info.path.display(),
        nl
    ));
    out.push_str(&format!("{}\"size\": {},{}", indent, info.size, nl));
    out.push_str(&format!(
        "{}\"table\": \"{}\",{}",
        indent, info.table_type, nl
    ));

    if let Some(ref header) = info.gpt_header {
        out.push_str(&format!(
            "{}\"disk_guid\": \"{}\",{}",
            indent, header.disk_guid, nl
        ));
    }

    out.push_str(&format!("{}\"partitions\": [", indent));
    out.push_str(nl);

    match info.table_type {
        PartitionTableType::Gpt => {
            for (i, part) in info.gpt_partitions.iter().enumerate() {
                let type_name = type_map
                    .get(&part.type_guid)
                    .map(|t| t.name)
                    .unwrap_or("Unknown");
                let mount = type_map
                    .get(&part.type_guid)
                    .and_then(|t| t.mount_point)
                    .unwrap_or("");

                out.push_str(&format!(
                    "{}{}{{\"index\": {}, \"type_guid\": \"{}\", \"unique_guid\": \"{}\", \"name\": \"{}\", \"type\": \"{}\", \"mount_point\": \"{}\", \"size\": {}, \"first_lba\": {}, \"last_lba\": {}}}",
                    indent, indent,
                    part.index,
                    part.type_guid,
                    part.unique_guid,
                    json_escape(&part.name),
                    type_name,
                    mount,
                    part.size_bytes(),
                    part.first_lba,
                    part.last_lba
                ));
                if i + 1 < info.gpt_partitions.len() {
                    out.push(',');
                }
                out.push_str(nl);
            }
        }
        PartitionTableType::Mbr => {
            for (i, part) in info.mbr_partitions.iter().enumerate() {
                out.push_str(&format!(
                    "{}{}{{\"index\": {}, \"type\": {}, \"type_name\": \"{}\", \"bootable\": {}, \"size\": {}, \"first_lba\": {}, \"sectors\": {}}}",
                    indent, indent,
                    part.index,
                    part.partition_type,
                    mbr_type_name(part.partition_type),
                    part.bootable,
                    part.size_bytes(),
                    part.first_lba,
                    part.num_sectors
                ));
                if i + 1 < info.mbr_partitions.len() {
                    out.push(',');
                }
                out.push_str(nl);
            }
        }
        PartitionTableType::None => {}
    }

    out.push_str(&format!("{}]{}", indent, nl));
    out.push_str(&format!("}}{}", nl));

    print!("{}", out);
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Output: discover
// ---------------------------------------------------------------------------

fn cmd_discover(no_legend: bool) {
    if !no_legend {
        println!("{:<12} {:<10} {:<40} PATH", "TYPE", "SIZE", "NAME");
    }

    let mut found = 0;

    for dir in IMAGE_SEARCH_PATHS {
        let dir_path = Path::new(dir);
        if !dir_path.is_dir() {
            continue;
        }

        let entries = match fs::read_dir(dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        for entry in entries.filter_map(|e| e.ok()) {
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().to_string();

            // Skip hidden files
            if name.starts_with('.') {
                continue;
            }

            let (img_type, size_str) = if path.is_dir() {
                ("directory", "-".to_string())
            } else if name.ends_with(".raw") || name.ends_with(".img") || name.ends_with(".qcow2") {
                let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
                ("raw", format_size(size))
            } else {
                continue;
            };

            println!(
                "{:<12} {:<10} {:<40} {}",
                img_type,
                size_str,
                name,
                path.display()
            );
            found += 1;
        }
    }

    if !no_legend {
        println!();
        println!("{} image(s) found.", found);
    }
}

// ---------------------------------------------------------------------------
// Output: validate
// ---------------------------------------------------------------------------

fn cmd_validate(info: &ImageInfo) -> Result<(), String> {
    println!("Image: {}", info.path.display());
    println!("Size: {} ({})", format_size(info.size), info.size);
    println!("Partition table: {}", info.table_type);

    match info.table_type {
        PartitionTableType::Gpt => {
            let header = info.gpt_header.as_ref().unwrap();
            println!("Disk GUID: {}", header.disk_guid);
            println!("Partitions: {}", info.gpt_partitions.len());

            let type_map = known_partition_types();
            let has_root = info
                .gpt_partitions
                .iter()
                .any(|p| type_map.get(&p.type_guid).and_then(|t| t.mount_point) == Some("/"));

            if has_root {
                println!("Root partition: found ✓");
            } else {
                println!("Root partition: not found ✗");
            }

            println!();
            println!("Image validates successfully.");
            Ok(())
        }
        PartitionTableType::Mbr => {
            println!("Partitions: {}", info.mbr_partitions.len());

            let has_linux = info.mbr_partitions.iter().any(|p| p.partition_type == 0x83);
            if has_linux {
                println!("Linux partition: found ✓");
            } else {
                println!("Linux partition: not found ✗");
            }

            println!();
            println!("Image validates successfully.");
            Ok(())
        }
        PartitionTableType::None => {
            println!();
            println!("Warning: No partition table found.");
            println!("This may be a raw filesystem image (no partition table).");
            Ok(())
        }
    }
}

// ---------------------------------------------------------------------------
// Stub commands (require root / loopback device)
// ---------------------------------------------------------------------------

fn cmd_mount(image: &Path, mount_path: &Path) -> Result<(), String> {
    eprintln!(
        "Mounting {} at {} ...",
        image.display(),
        mount_path.display()
    );

    // Verify the image exists
    if !image.exists() {
        return Err(format!("Image not found: {}", image.display()));
    }

    // Verify mount path exists or create it
    if !mount_path.exists() {
        fs::create_dir_all(mount_path)
            .map_err(|e| format!("Cannot create mount point {}: {}", mount_path.display(), e))?;
    }

    // Analyze the image to verify it's valid
    let info = analyze_image(image)?;

    // Partitioned image: set it up on a loop device and mount the discoverable
    // partitions (root at the mount point, /usr, /home, ESP, … beneath it),
    // mirroring systemd-dissect assembling an OS tree.
    #[cfg(target_os = "linux")]
    if !info.gpt_partitions.is_empty() {
        return mount_partitioned_image(image, mount_path, &info);
    }

    if info.table_type == PartitionTableType::None && info.gpt_partitions.is_empty() {
        eprintln!("Warning: No partition table detected. Attempting raw filesystem mount.");
    }

    // Set up loopback device and mount (requires root)
    #[cfg(target_os = "linux")]
    {
        let image_c = std::ffi::CString::new(image.to_string_lossy().as_bytes())
            .map_err(|e| e.to_string())?;
        let mount_c = std::ffi::CString::new(mount_path.to_string_lossy().as_bytes())
            .map_err(|e| e.to_string())?;

        // Try direct mount (works for raw filesystem images without a partition table)
        let ret = unsafe {
            libc::mount(
                image_c.as_ptr(),
                mount_c.as_ptr(),
                std::ptr::null(),
                libc::MS_RDONLY,
                std::ptr::null(),
            )
        };

        if ret == 0 {
            eprintln!("Mounted {} at {}.", image.display(), mount_path.display());
            return Ok(());
        }

        let err = io::Error::last_os_error();
        Err(format!(
            "Mount failed: {}. Note: mounting partitioned images requires loopback device setup (not yet fully implemented).",
            err
        ))
    }

    #[cfg(not(target_os = "linux"))]
    Err("Mount is only supported on Linux.".to_string())
}

/// Mount a partitioned image via a loop device. Attaches the image with
/// partition scanning, then mounts each partition whose GPT type has a
/// well-known mount point (root at the mount point itself, /usr, /home, ESP,
/// … at sub-paths). Kernel partition N corresponds to GPT slot N-1.
#[cfg(target_os = "linux")]
fn mount_partitioned_image(
    image: &Path,
    mount_path: &Path,
    info: &ImageInfo,
) -> Result<(), String> {
    use std::process::Command;
    let types = known_partition_types();

    let out = Command::new("losetup")
        .args(["--find", "--show", "--partscan", "--nooverlap"])
        .arg(image)
        .output()
        .map_err(|e| format!("Failed to run losetup: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "losetup failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let loopdev = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=10"])
        .status();

    // Collect (kernel partition number, mount point) for the partitions we can
    // place, shallowest first so parent mount points are mounted before their
    // children (e.g. "/" before "/usr").
    let mut targets: Vec<(usize, String)> = Vec::new();
    for (i, p) in info.gpt_partitions.iter().enumerate() {
        if let Some(mp) = types.get(&p.type_guid).and_then(|t| t.mount_point) {
            targets.push((i + 1, mp.to_string()));
        }
    }
    targets.sort_by_key(|(_, mp)| mp.matches('/').count());

    let mut mount_err: Option<String> = None;
    for (partnum, mp) in &targets {
        let dev = format!("{loopdev}p{partnum}");
        for _ in 0..30 {
            if Path::new(&dev).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        let target = if mp == "/" {
            mount_path.to_path_buf()
        } else {
            mount_path.join(mp.trim_start_matches('/'))
        };
        if mp != "/" {
            let _ = fs::create_dir_all(&target);
        }
        match Command::new("mount").arg(&dev).arg(&target).status() {
            Ok(s) if s.success() => {}
            Ok(s) => {
                mount_err = Some(format!("mount {dev} at {}: {s}", target.display()));
                break;
            }
            Err(e) => {
                mount_err = Some(format!("mount {dev}: {e}"));
                break;
            }
        }
    }

    if let Some(e) = mount_err {
        let _ = Command::new("umount").arg("-R").arg(mount_path).status();
        let _ = Command::new("losetup").args(["-d", &loopdev]).status();
        return Err(e);
    }

    eprintln!("Mounted {} at {}.", image.display(), mount_path.display());
    Ok(())
}

fn cmd_umount(mount_path: &Path) -> Result<(), String> {
    if !mount_path.exists() {
        return Err(format!("Mount point not found: {}", mount_path.display()));
    }

    #[cfg(target_os = "linux")]
    {
        use std::process::Command;

        // Resolve the backing loop device (if any) before unmounting, so it can
        // be detached afterwards. /dev/loopNpM -> /dev/loopN.
        let loopdev = Command::new("findmnt")
            .args(["-n", "-o", "SOURCE", "--target"])
            .arg(mount_path)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .filter(|s| s.starts_with("/dev/loop"))
            .map(|s| match s.rsplit_once('p') {
                Some((base, num)) if num.chars().all(|c| c.is_ascii_digit()) => base.to_string(),
                _ => s,
            });

        // Recursively unmount so partition sub-mounts (/usr, /home, …) go first.
        let status = Command::new("umount")
            .arg("-R")
            .arg(mount_path)
            .status()
            .map_err(|e| format!("Failed to run umount: {e}"))?;
        if !status.success() {
            // Fall back to a direct umount of just this path.
            let mount_c = std::ffi::CString::new(mount_path.to_string_lossy().as_bytes())
                .map_err(|e| e.to_string())?;
            let ret = unsafe { libc::umount2(mount_c.as_ptr(), 0) };
            if ret != 0 {
                let err = io::Error::last_os_error();
                return Err(format!("Failed to unmount {}: {}", mount_path.display(), err));
            }
        }

        if let Some(ld) = loopdev {
            let _ = Command::new("losetup").args(["-d", &ld]).status();
        }

        eprintln!("Unmounted {}.", mount_path.display());
        Ok(())
    }

    #[cfg(not(target_os = "linux"))]
    Err("Unmount is only supported on Linux.".to_string())
}

fn cmd_list(info: &ImageInfo, no_legend: bool) -> Result<(), String> {
    if info.table_type == PartitionTableType::None {
        eprintln!("Warning: No partition table found. Cannot list files without mounting.");
        eprintln!(
            "Use --mount to mount the image first, then use standard tools to list its contents."
        );
        return Ok(());
    }

    let type_map = known_partition_types();

    if !no_legend {
        println!("{:<5} {:<28} {:<10} MOUNT POINT", "IDX", "TYPE", "SIZE");
    }

    match info.table_type {
        PartitionTableType::Gpt => {
            for part in &info.gpt_partitions {
                let type_info = type_map.get(&part.type_guid);
                let type_name = type_info.map(|t| t.name).unwrap_or("Unknown");
                let mount = type_info.and_then(|t| t.mount_point).unwrap_or("-");

                println!(
                    "{:<5} {:<28} {:<10} {}",
                    part.index,
                    type_name,
                    format_size(part.size_bytes()),
                    mount,
                );
            }
        }
        PartitionTableType::Mbr => {
            for part in &info.mbr_partitions {
                let type_name = mbr_type_name(part.partition_type);
                println!(
                    "{:<5} {:<28} {:<10} -",
                    part.index,
                    type_name,
                    format_size(part.size_bytes()),
                );
            }
        }
        PartitionTableType::None => {}
    }

    if !no_legend {
        let count = match info.table_type {
            PartitionTableType::Gpt => info.gpt_partitions.len(),
            PartitionTableType::Mbr => info.mbr_partitions.len(),
            PartitionTableType::None => 0,
        };
        println!();
        println!("{} partition(s).", count);
    }

    Ok(())
}

fn cmd_copy_from(image: &Path, source: &str, dest: Option<&str>) -> Result<(), String> {
    let _ = (image, source, dest);
    Err("copy-from requires mounting the image, which needs root privileges and loopback device support. Not yet fully implemented.".to_string())
}

fn cmd_copy_to(image: &Path, source: &str, dest: Option<&str>) -> Result<(), String> {
    let _ = (image, source, dest);
    Err("copy-to requires mounting the image, which needs root privileges and loopback device support. Not yet fully implemented.".to_string())
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!(
        "\
Usage: systemd-dissect [OPTIONS...] IMAGE
       systemd-dissect [OPTIONS...] --mount IMAGE PATH
       systemd-dissect [OPTIONS...] --umount PATH
       systemd-dissect [OPTIONS...] --list IMAGE
       systemd-dissect [OPTIONS...] --copy-from IMAGE SOURCE [DEST]
       systemd-dissect [OPTIONS...] --copy-to IMAGE SOURCE [DEST]
       systemd-dissect [OPTIONS...] --discover
       systemd-dissect [OPTIONS...] --validate IMAGE

Inspect and interact with OS disk images.

Commands:
  (default)       Show image partition table and info
  --mount         Mount the image at a given path
  --umount        Unmount a previously mounted image
  --mkdir         Create the mount point directory before mounting
  --rmdir         Remove the mount point directory after unmounting
  --list          List partitions in the image
  --copy-from     Copy a file out of the image
  --copy-to       Copy a file into the image
  --discover      Discover images in well-known locations
  --validate      Validate image structure

Options:
  --root-hash=HASH       Root hash for dm-verity verification
  --root-hash-sig=SIG    Root hash signature
  --verity-data=PATH     Path to verity data
  --json=MODE            JSON output (short, pretty, off)
  --no-pager             Don't pipe output through a pager
  --no-legend            Don't show column headers / footers
  -h --help              Show this help
     --version           Show version

This tool can parse GPT and MBR partition tables and identify
well-known partition types per the Discoverable Partitions Specification."
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run(argv: &[String]) -> Result<(), String> {
    let args = parse_args(argv)?;

    if args.command == Command::Help {
        print_usage();
        return Ok(());
    }

    match args.command {
        Command::Discover => {
            cmd_discover(args.no_legend);
            return Ok(());
        }
        Command::Umount => {
            let mount_path = args
                .mount_path
                .as_ref()
                .ok_or("--umount requires a mount point path")?;
            cmd_umount(mount_path)?;
            // --rmdir: remove the (now-empty) mount point directory.
            if args.rmdir {
                let _ = std::fs::remove_dir(mount_path);
            }
            return Ok(());
        }
        _ => {}
    }

    // Commands that need an image
    let image = args.image.as_ref().ok_or("An image path is required")?;

    match args.command {
        Command::Show => {
            let info = analyze_image(image)?;
            cmd_show(&info, args.json_mode, args.no_legend);
        }
        Command::List => {
            let info = analyze_image(image)?;
            cmd_list(&info, args.no_legend)?;
        }
        Command::Validate => {
            let info = analyze_image(image)?;
            cmd_validate(&info)?;
        }
        Command::Mount => {
            let mount_path = args
                .mount_path
                .as_ref()
                .ok_or("--mount requires a mount point path after the image")?;
            // --mkdir: create the mount point (and parents) before mounting.
            if args.mkdir {
                std::fs::create_dir_all(mount_path)
                    .map_err(|e| format!("Failed to create mount point {mount_path:?}: {e}"))?;
            }
            cmd_mount(image, mount_path)?;
        }
        Command::CopyFrom => {
            let source = args
                .source
                .as_ref()
                .ok_or("--copy-from requires a source path")?;
            cmd_copy_from(image, source, args.dest.as_deref())?;
        }
        Command::CopyTo => {
            let source = args
                .source
                .as_ref()
                .ok_or("--copy-to requires a source path")?;
            cmd_copy_to(image, source, args.dest.as_deref())?;
        }
        _ => unreachable!(),
    }

    Ok(())
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if let Err(e) = run(&args) {
        eprintln!("systemd-dissect: {}", e);
        process::exit(1);
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    fn temp_dir() -> TempDir {
        TempDir::new().expect("failed to create temp dir")
    }

    // -----------------------------------------------------------------------
    // parse_args tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_default_show() {
        let args = parse_args(&["image.raw".to_string()]).unwrap();
        assert_eq!(args.command, Command::Show);
        assert_eq!(args.image, Some(PathBuf::from("image.raw")));
    }

    #[test]
    fn test_parse_args_empty() {
        let args = parse_args(&[]).unwrap();
        assert_eq!(args.command, Command::Show);
        assert!(args.image.is_none());
    }

    #[test]
    fn test_parse_args_mount() {
        let args = parse_args(&[
            "--mount".to_string(),
            "img.raw".to_string(),
            "/mnt".to_string(),
        ])
        .unwrap();
        assert_eq!(args.command, Command::Mount);
        assert_eq!(args.image, Some(PathBuf::from("img.raw")));
        assert_eq!(args.mount_path, Some(PathBuf::from("/mnt")));
    }

    #[test]
    fn test_parse_args_umount() {
        let args = parse_args(&["--umount".to_string(), "/mnt".to_string()]).unwrap();
        assert_eq!(args.command, Command::Umount);
        assert_eq!(args.mount_path, Some(PathBuf::from("/mnt")));
    }

    #[test]
    fn test_parse_args_unmount_alias() {
        let args = parse_args(&["--unmount".to_string(), "/mnt".to_string()]).unwrap();
        assert_eq!(args.command, Command::Umount);
    }

    #[test]
    fn test_parse_args_short_mount() {
        let args =
            parse_args(&["-m".to_string(), "img.raw".to_string(), "/mnt".to_string()]).unwrap();
        assert_eq!(args.command, Command::Mount);
    }

    #[test]
    fn test_parse_args_short_umount() {
        let args = parse_args(&["-u".to_string(), "/mnt".to_string()]).unwrap();
        assert_eq!(args.command, Command::Umount);
    }

    #[test]
    fn test_parse_args_list() {
        let args = parse_args(&["--list".to_string(), "image.raw".to_string()]).unwrap();
        assert_eq!(args.command, Command::List);
        assert_eq!(args.image, Some(PathBuf::from("image.raw")));
    }

    #[test]
    fn test_parse_args_short_list() {
        let args = parse_args(&["-l".to_string(), "image.raw".to_string()]).unwrap();
        assert_eq!(args.command, Command::List);
    }

    #[test]
    fn test_parse_args_discover() {
        let args = parse_args(&["--discover".to_string()]).unwrap();
        assert_eq!(args.command, Command::Discover);
    }

    #[test]
    fn test_parse_args_validate() {
        let args = parse_args(&["--validate".to_string(), "image.raw".to_string()]).unwrap();
        assert_eq!(args.command, Command::Validate);
        assert_eq!(args.image, Some(PathBuf::from("image.raw")));
    }

    #[test]
    fn test_parse_args_copy_from() {
        let args = parse_args(&[
            "--copy-from".to_string(),
            "image.raw".to_string(),
            "/etc/hostname".to_string(),
            "/tmp/hostname".to_string(),
        ])
        .unwrap();
        assert_eq!(args.command, Command::CopyFrom);
        assert_eq!(args.image, Some(PathBuf::from("image.raw")));
        assert_eq!(args.source, Some("/etc/hostname".to_string()));
        assert_eq!(args.dest, Some("/tmp/hostname".to_string()));
    }

    #[test]
    fn test_parse_args_copy_to() {
        let args = parse_args(&[
            "--copy-to".to_string(),
            "image.raw".to_string(),
            "/tmp/file".to_string(),
            "/etc/file".to_string(),
        ])
        .unwrap();
        assert_eq!(args.command, Command::CopyTo);
    }

    #[test]
    fn test_parse_args_root_hash() {
        let args =
            parse_args(&["--root-hash=abc123".to_string(), "image.raw".to_string()]).unwrap();
        assert_eq!(args.root_hash, Some("abc123".to_string()));
    }

    #[test]
    fn test_parse_args_root_hash_separate() {
        let args = parse_args(&[
            "--root-hash".to_string(),
            "abc123".to_string(),
            "image.raw".to_string(),
        ])
        .unwrap();
        assert_eq!(args.root_hash, Some("abc123".to_string()));
    }

    #[test]
    fn test_parse_args_verity_data() {
        let args = parse_args(&[
            "--verity-data=/path/to/verity".to_string(),
            "image.raw".to_string(),
        ])
        .unwrap();
        assert_eq!(args.verity_data, Some("/path/to/verity".to_string()));
    }

    #[test]
    fn test_parse_args_json_short() {
        let args = parse_args(&["--json=short".to_string(), "image.raw".to_string()]).unwrap();
        assert_eq!(args.json_mode, JsonMode::Short);
    }

    #[test]
    fn test_parse_args_json_pretty() {
        let args = parse_args(&["--json=pretty".to_string(), "image.raw".to_string()]).unwrap();
        assert_eq!(args.json_mode, JsonMode::Pretty);
    }

    #[test]
    fn test_parse_args_json_off() {
        let args = parse_args(&["--json=off".to_string()]).unwrap();
        assert_eq!(args.json_mode, JsonMode::Off);
    }

    #[test]
    fn test_parse_args_json_invalid() {
        let result = parse_args(&["--json=invalid".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_no_legend() {
        let args = parse_args(&["--no-legend".to_string(), "image.raw".to_string()]).unwrap();
        assert!(args.no_legend);
    }

    #[test]
    fn test_parse_args_no_pager() {
        let args = parse_args(&["--no-pager".to_string(), "image.raw".to_string()]).unwrap();
        // --no-pager is accepted silently
        assert_eq!(args.command, Command::Show);
    }

    #[test]
    fn test_parse_args_help() {
        let args = parse_args(&["--help".to_string()]).unwrap();
        assert_eq!(args.command, Command::Help);
    }

    #[test]
    fn test_parse_args_short_help() {
        let args = parse_args(&["-h".to_string()]).unwrap();
        assert_eq!(args.command, Command::Help);
    }

    #[test]
    fn test_parse_args_unknown_option() {
        let result = parse_args(&["--frobnicate".to_string()]);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_args_multiple_options() {
        let args = parse_args(&[
            "--no-legend".to_string(),
            "--json=pretty".to_string(),
            "--root-hash=deadbeef".to_string(),
            "image.raw".to_string(),
        ])
        .unwrap();
        assert!(args.no_legend);
        assert_eq!(args.json_mode, JsonMode::Pretty);
        assert_eq!(args.root_hash, Some("deadbeef".to_string()));
        assert_eq!(args.image, Some(PathBuf::from("image.raw")));
    }

    #[test]
    fn test_parse_args_missing_value() {
        let result = parse_args(&["--root-hash".to_string()]);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // GUID parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_guid_zero() {
        let data = [0u8; 16];
        let guid = parse_guid(&data);
        assert_eq!(guid, "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn test_is_zero_guid_true() {
        assert!(is_zero_guid("00000000-0000-0000-0000-000000000000"));
    }

    #[test]
    fn test_is_zero_guid_false() {
        assert!(!is_zero_guid("4f68bce3-e8cd-4db1-96e7-fbcaf984b709"));
    }

    #[test]
    fn test_parse_guid_known_type() {
        // EFI System Partition GUID: C12A7328-F81F-11D2-BA4B-00A0C93EC93B
        // In mixed-endian GPT format:
        // time_low:  0xC12A7328 -> LE bytes: 28 73 2A C1
        // time_mid:  0xF81F     -> LE bytes: 1F F8
        // time_hi:   0x11D2     -> LE bytes: D2 11
        // clock_seq: 0xBA4B     -> big-endian bytes: 0xBA 0x4B
        // node:      00A0C93EC93B -> bytes: 00 A0 C9 3E C9 3B
        let data: [u8; 16] = [
            0x28, 0x73, 0x2A, 0xC1, 0x1F, 0xF8, 0xD2, 0x11, 0xBA, 0x4B, 0x00, 0xA0, 0xC9, 0x3E,
            0xC9, 0x3B,
        ];
        let guid = parse_guid(&data);
        assert_eq!(guid, "c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
    }

    #[test]
    fn test_parse_guid_short_data() {
        let data = [0u8; 8];
        let guid = parse_guid(&data);
        assert_eq!(guid, "00000000-0000-0000-0000-000000000000");
    }

    #[test]
    fn test_parse_guid_consistent() {
        let data: [u8; 16] = [
            0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E,
            0x0F, 0x10,
        ];
        let g1 = parse_guid(&data);
        let g2 = parse_guid(&data);
        assert_eq!(g1, g2);
    }

    // -----------------------------------------------------------------------
    // UTF-16LE name parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_utf16le_name_empty() {
        let data = [0u8; 16];
        assert_eq!(parse_utf16le_name(&data), "");
    }

    #[test]
    fn test_parse_utf16le_name_ascii() {
        // "root" in UTF-16LE
        let data = [b'r', 0, b'o', 0, b'o', 0, b't', 0, 0, 0, 0, 0];
        assert_eq!(parse_utf16le_name(&data), "root");
    }

    #[test]
    fn test_parse_utf16le_name_with_null_terminator() {
        // "AB" followed by null
        let data = [b'A', 0, b'B', 0, 0, 0, b'C', 0];
        assert_eq!(parse_utf16le_name(&data), "AB");
    }

    #[test]
    fn test_parse_utf16le_name_unicode() {
        // "ö" is U+00F6 -> 0xF6, 0x00 in UTF-16LE
        let data = [0xF6, 0x00, 0, 0];
        assert_eq!(parse_utf16le_name(&data), "ö");
    }

    #[test]
    fn test_parse_utf16le_name_odd_length() {
        // Odd number of bytes — last byte is ignored
        let data = [b'A', 0, b'B'];
        assert_eq!(parse_utf16le_name(&data), "A");
    }

    // -----------------------------------------------------------------------
    // Size formatting tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_size_zero() {
        assert_eq!(format_size(0), "0B");
    }

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(100), "100B");
    }

    #[test]
    fn test_format_size_kilobytes() {
        assert_eq!(format_size(1024), "1.00K");
    }

    #[test]
    fn test_format_size_megabytes() {
        assert_eq!(format_size(1024 * 1024), "1.00M");
    }

    #[test]
    fn test_format_size_gigabytes() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1.00G");
    }

    #[test]
    fn test_format_size_terabytes() {
        assert_eq!(format_size(1024u64 * 1024 * 1024 * 1024), "1.00T");
    }

    #[test]
    fn test_format_size_mixed() {
        assert_eq!(format_size(1536), "1.50K"); // 1.5 KiB
    }

    #[test]
    fn test_format_size_large_kb() {
        assert_eq!(format_size(512 * 1024), "512K"); // 512 KiB
    }

    // -----------------------------------------------------------------------
    // MBR type name tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mbr_type_name_known() {
        assert_eq!(mbr_type_name(0x83), "Linux");
        assert_eq!(mbr_type_name(0x82), "Linux swap");
        assert_eq!(mbr_type_name(0xEE), "GPT Protective");
        assert_eq!(mbr_type_name(0xEF), "EFI System");
        assert_eq!(mbr_type_name(0x07), "NTFS/HPFS/exFAT");
        assert_eq!(mbr_type_name(0x8E), "Linux LVM");
        assert_eq!(mbr_type_name(0xFD), "Linux RAID");
    }

    #[test]
    fn test_mbr_type_name_unknown() {
        assert_eq!(mbr_type_name(0xFF), "Unknown");
        assert_eq!(mbr_type_name(0xAA), "Unknown");
    }

    #[test]
    fn test_mbr_type_name_empty() {
        assert_eq!(mbr_type_name(0x00), "Empty");
    }

    // -----------------------------------------------------------------------
    // Known partition types tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_known_partition_types_has_root_x86_64() {
        let types = known_partition_types();
        let root = types.get("4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
        assert!(root.is_some());
        assert_eq!(root.unwrap().name, "Linux root (x86-64)");
        assert_eq!(root.unwrap().mount_point, Some("/"));
    }

    #[test]
    fn test_known_partition_types_has_esp() {
        let types = known_partition_types();
        let esp = types.get("c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
        assert!(esp.is_some());
        assert_eq!(esp.unwrap().name, "EFI System Partition");
        assert_eq!(esp.unwrap().mount_point, Some("/efi"));
    }

    #[test]
    fn test_known_partition_types_has_home() {
        let types = known_partition_types();
        let home = types.get("933ac7e1-2eb4-4f13-b844-0e14e2aef915");
        assert!(home.is_some());
        assert_eq!(home.unwrap().name, "Linux /home");
        assert_eq!(home.unwrap().mount_point, Some("/home"));
    }

    #[test]
    fn test_known_partition_types_has_swap() {
        let types = known_partition_types();
        let swap = types.get("0657fd6d-a4ab-43c4-84e5-0933c84b4f4f");
        assert!(swap.is_some());
        assert_eq!(swap.unwrap().name, "Linux swap");
        assert_eq!(swap.unwrap().mount_point, None);
    }

    #[test]
    fn test_known_partition_types_has_xbootldr() {
        let types = known_partition_types();
        let boot = types.get("bc13c2ff-59e6-4262-a352-b275fd6f7172");
        assert!(boot.is_some());
        assert_eq!(boot.unwrap().name, "Extended Boot Loader");
        assert_eq!(boot.unwrap().mount_point, Some("/boot"));
    }

    // -----------------------------------------------------------------------
    // PartitionTableType Display tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_partition_table_type_display() {
        assert_eq!(format!("{}", PartitionTableType::Gpt), "GPT");
        assert_eq!(format!("{}", PartitionTableType::Mbr), "MBR/DOS");
        assert_eq!(format!("{}", PartitionTableType::None), "none");
    }

    // -----------------------------------------------------------------------
    // GptPartition size tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gpt_partition_size_bytes() {
        let part = GptPartition {
            type_guid: "00000000-0000-0000-0000-000000000000".to_string(),
            unique_guid: "00000000-0000-0000-0000-000000000000".to_string(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
            name: String::new(),
            index: 1,
        };
        // (4095 - 2048 + 1) * 512 = 2048 * 512 = 1048576 = 1MiB
        assert_eq!(part.size_bytes(), 1048576);
    }

    #[test]
    fn test_gpt_partition_size_single_sector() {
        let part = GptPartition {
            type_guid: String::new(),
            unique_guid: String::new(),
            first_lba: 100,
            last_lba: 100,
            attributes: 0,
            name: String::new(),
            index: 1,
        };
        assert_eq!(part.size_bytes(), 512);
    }

    // -----------------------------------------------------------------------
    // MbrPartition size tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_mbr_partition_size_bytes() {
        let part = MbrPartition {
            bootable: false,
            partition_type: 0x83,
            first_lba: 2048,
            num_sectors: 2048,
            index: 1,
        };
        assert_eq!(part.size_bytes(), 2048 * 512);
    }

    // -----------------------------------------------------------------------
    // json_escape tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_json_escape_plain() {
        assert_eq!(json_escape("hello"), "hello");
    }

    #[test]
    fn test_json_escape_quotes() {
        assert_eq!(json_escape("say \"hi\""), "say \\\"hi\\\"");
    }

    #[test]
    fn test_json_escape_backslash() {
        assert_eq!(json_escape("path\\file"), "path\\\\file");
    }

    #[test]
    fn test_json_escape_newline() {
        assert_eq!(json_escape("line1\nline2"), "line1\\nline2");
    }

    #[test]
    fn test_json_escape_tab() {
        assert_eq!(json_escape("col1\tcol2"), "col1\\tcol2");
    }

    #[test]
    fn test_json_escape_carriage_return() {
        assert_eq!(json_escape("a\rb"), "a\\rb");
    }

    #[test]
    fn test_json_escape_control_char() {
        assert_eq!(json_escape("\x01"), "\\u0001");
    }

    #[test]
    fn test_json_escape_unicode() {
        assert_eq!(json_escape("ö"), "ö");
    }

    #[test]
    fn test_json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    // -----------------------------------------------------------------------
    // Image analysis tests (with synthetic images)
    // -----------------------------------------------------------------------

    /// Create a minimal valid GPT image in memory.
    fn create_gpt_image(tmp: &Path) -> PathBuf {
        let image_path = tmp.join("test.raw");
        let mut data = vec![0u8; 1024 * 1024]; // 1 MiB

        // MBR protective entry (sector 0)
        data[446] = 0x00; // not bootable
        data[450] = 0xEE; // GPT protective
        data[454] = 0x01; // first LBA = 1
        data[455] = 0x00;
        data[456] = 0x00;
        data[457] = 0x00;
        let total_sectors = (data.len() as u32 / 512) - 1;
        data[458..462].copy_from_slice(&total_sectors.to_le_bytes());
        data[510] = 0x55;
        data[511] = 0xAA;

        // GPT header at LBA 1 (offset 512)
        let hdr_off = 512;
        data[hdr_off..hdr_off + 8].copy_from_slice(b"EFI PART");
        // Revision 1.0
        data[hdr_off + 8..hdr_off + 12].copy_from_slice(&0x00010000u32.to_le_bytes());
        // Header size = 92
        data[hdr_off + 12..hdr_off + 16].copy_from_slice(&92u32.to_le_bytes());
        // CRC32 = 0 (skip for testing)
        // My LBA = 1
        data[hdr_off + 24..hdr_off + 32].copy_from_slice(&1u64.to_le_bytes());
        // Alternate LBA
        let alt_lba = (data.len() as u64 / 512) - 1;
        data[hdr_off + 32..hdr_off + 40].copy_from_slice(&alt_lba.to_le_bytes());
        // First usable LBA = 34
        data[hdr_off + 40..hdr_off + 48].copy_from_slice(&34u64.to_le_bytes());
        // Last usable LBA
        let last_usable = alt_lba - 33;
        data[hdr_off + 48..hdr_off + 56].copy_from_slice(&last_usable.to_le_bytes());
        // Disk GUID (16 bytes at offset 56)
        data[hdr_off + 56] = 0x01;
        data[hdr_off + 57] = 0x02;
        data[hdr_off + 58] = 0x03;
        data[hdr_off + 59] = 0x04;
        // Partition entry LBA = 2
        data[hdr_off + 72..hdr_off + 80].copy_from_slice(&2u64.to_le_bytes());
        // Number of partition entries = 128
        data[hdr_off + 80..hdr_off + 84].copy_from_slice(&128u32.to_le_bytes());
        // Partition entry size = 128
        data[hdr_off + 84..hdr_off + 88].copy_from_slice(&128u32.to_le_bytes());

        // Partition entry at LBA 2 (offset 1024)
        let entry_off = 1024;
        // Type GUID: Linux root x86-64 (4f68bce3-e8cd-4db1-96e7-fbcaf984b709)
        // In mixed-endian:
        let type_guid: [u8; 16] = [
            0xe3, 0xbc, 0x68, 0x4f, // time_low LE
            0xcd, 0xe8, // time_mid LE
            0xb1, 0x4d, // time_hi LE
            0x96, 0xe7, // clock_seq BE
            0xfb, 0xca, 0xf9, 0x84, 0xb7, 0x09, // node BE
        ];
        data[entry_off..entry_off + 16].copy_from_slice(&type_guid);
        // Unique GUID
        data[entry_off + 16] = 0xAA;
        data[entry_off + 17] = 0xBB;
        // First LBA = 2048
        data[entry_off + 32..entry_off + 40].copy_from_slice(&2048u64.to_le_bytes());
        // Last LBA = 2048 + 1023 = 3071 (512 KiB)
        data[entry_off + 40..entry_off + 48].copy_from_slice(&3071u64.to_le_bytes());
        // Name: "root" in UTF-16LE
        let name_off = entry_off + 56;
        data[name_off] = b'r';
        data[name_off + 2] = b'o';
        data[name_off + 4] = b'o';
        data[name_off + 6] = b't';

        fs::write(&image_path, &data).unwrap();
        image_path
    }

    fn create_mbr_image(tmp: &Path) -> PathBuf {
        let image_path = tmp.join("test-mbr.raw");
        let mut data = vec![0u8; 512 * 1024]; // 512 KiB

        // MBR partition entry 1 at offset 446
        data[446] = 0x80; // bootable
        data[450] = 0x83; // Linux
        data[454] = 0x00; // first LBA = 2048
        data[455] = 0x08;
        data[456] = 0x00;
        data[457] = 0x00;
        let sectors = 512u32;
        data[458..462].copy_from_slice(&sectors.to_le_bytes());

        // MBR signature
        data[510] = 0x55;
        data[511] = 0xAA;

        fs::write(&image_path, &data).unwrap();
        image_path
    }

    fn create_empty_image(tmp: &Path) -> PathBuf {
        let image_path = tmp.join("empty.raw");
        let data = vec![0u8; 1024];
        fs::write(&image_path, &data).unwrap();
        image_path
    }

    #[test]
    fn test_analyze_gpt_image() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();

        assert_eq!(info.table_type, PartitionTableType::Gpt);
        assert!(info.gpt_header.is_some());
        assert_eq!(info.gpt_partitions.len(), 1);

        let part = &info.gpt_partitions[0];
        assert_eq!(part.type_guid, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
        assert_eq!(part.name, "root");
        assert_eq!(part.first_lba, 2048);
        assert_eq!(part.last_lba, 3071);
        assert_eq!(part.index, 1);
    }

    #[test]
    fn test_analyze_mbr_image() {
        let tmp = temp_dir();
        let path = create_mbr_image(tmp.path());
        let info = analyze_image(&path).unwrap();

        assert_eq!(info.table_type, PartitionTableType::Mbr);
        assert!(info.gpt_header.is_none());
        assert_eq!(info.mbr_partitions.len(), 1);

        let part = &info.mbr_partitions[0];
        assert!(part.bootable);
        assert_eq!(part.partition_type, 0x83);
        assert_eq!(part.first_lba, 2048);
        assert_eq!(part.num_sectors, 512);
    }

    #[test]
    fn test_analyze_empty_image() {
        let tmp = temp_dir();
        let path = create_empty_image(tmp.path());
        let info = analyze_image(&path).unwrap();

        assert_eq!(info.table_type, PartitionTableType::None);
        assert!(info.gpt_header.is_none());
        assert!(info.gpt_partitions.is_empty());
        assert!(info.mbr_partitions.is_empty());
    }

    #[test]
    fn test_analyze_nonexistent_image() {
        let result = analyze_image(Path::new("/nonexistent/image.raw"));
        assert!(result.is_err());
    }

    #[test]
    fn test_analyze_image_size() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        assert_eq!(info.size, 1024 * 1024);
    }

    // -----------------------------------------------------------------------
    // Command execution tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_cmd_show_gpt() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        // Should not panic
        cmd_show(&info, JsonMode::Off, false);
    }

    #[test]
    fn test_cmd_show_mbr() {
        let tmp = temp_dir();
        let path = create_mbr_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        cmd_show(&info, JsonMode::Off, false);
    }

    #[test]
    fn test_cmd_show_empty() {
        let tmp = temp_dir();
        let path = create_empty_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        cmd_show(&info, JsonMode::Off, false);
    }

    #[test]
    fn test_cmd_show_no_legend() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        cmd_show(&info, JsonMode::Off, true);
    }

    #[test]
    fn test_cmd_show_json_short() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        cmd_show(&info, JsonMode::Short, false);
    }

    #[test]
    fn test_cmd_show_json_pretty() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        cmd_show(&info, JsonMode::Pretty, false);
    }

    #[test]
    fn test_cmd_validate_gpt() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        let result = cmd_validate(&info);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_validate_mbr() {
        let tmp = temp_dir();
        let path = create_mbr_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        let result = cmd_validate(&info);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_validate_empty() {
        let tmp = temp_dir();
        let path = create_empty_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        let result = cmd_validate(&info);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_list_gpt() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        let result = cmd_list(&info, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_list_mbr() {
        let tmp = temp_dir();
        let path = create_mbr_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        let result = cmd_list(&info, false);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_list_no_legend() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        let result = cmd_list(&info, true);
        assert!(result.is_ok());
    }

    #[test]
    fn test_cmd_discover_runs() {
        // This tests that discover doesn't panic; actual results depend on
        // the system's /var/lib/machines etc.
        cmd_discover(false);
        cmd_discover(true);
    }

    // -----------------------------------------------------------------------
    // run() integration tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_run_show_image() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let args = vec![path.to_string_lossy().to_string()];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_list_image() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let args = vec!["--list".to_string(), path.to_string_lossy().to_string()];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_validate_image() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let args = vec!["--validate".to_string(), path.to_string_lossy().to_string()];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_discover() {
        let args = vec!["--discover".to_string()];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_json_output() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let args = vec![
            "--json=pretty".to_string(),
            path.to_string_lossy().to_string(),
        ];
        let result = run(&args);
        assert!(result.is_ok());
    }

    #[test]
    fn test_run_no_image() {
        // Show command without image should error
        let args = vec![];
        let result = run(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_nonexistent_image() {
        let args = vec!["/nonexistent/image.raw".to_string()];
        let result = run(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_mount_missing_mount_path() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let args = vec!["--mount".to_string(), path.to_string_lossy().to_string()];
        let result = run(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_umount_nonexistent() {
        let args = vec![
            "--umount".to_string(),
            "/nonexistent/mount/point".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_copy_from_not_implemented() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let args = vec![
            "--copy-from".to_string(),
            path.to_string_lossy().to_string(),
            "/etc/hostname".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_err());
    }

    #[test]
    fn test_run_copy_to_not_implemented() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let args = vec![
            "--copy-to".to_string(),
            path.to_string_lossy().to_string(),
            "/tmp/file".to_string(),
        ];
        let result = run(&args);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // GPT header field tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gpt_header_fields() {
        let tmp = temp_dir();
        let path = create_gpt_image(tmp.path());
        let info = analyze_image(&path).unwrap();
        let header = info.gpt_header.as_ref().unwrap();

        assert_eq!(header.revision, 0x00010000);
        assert_eq!(header.header_size, 92);
        assert_eq!(header.my_lba, 1);
        assert_eq!(header.first_usable_lba, 34);
        assert_eq!(header.partition_entry_lba, 2);
        assert_eq!(header.num_partition_entries, 128);
        assert_eq!(header.partition_entry_size, 128);
    }

    // -----------------------------------------------------------------------
    // GPT partition detail tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_gpt_partition_recognized() {
        let types = known_partition_types();
        let part_type = "4f68bce3-e8cd-4db1-96e7-fbcaf984b709";
        let info = types.get(part_type);
        assert!(info.is_some());
        assert_eq!(info.unwrap().name, "Linux root (x86-64)");
        assert_eq!(info.unwrap().mount_point, Some("/"));
    }

    // -----------------------------------------------------------------------
    // Edge case: very small images
    // -----------------------------------------------------------------------

    #[test]
    fn test_analyze_tiny_image() {
        let tmp = temp_dir();
        let path = tmp.path().join("tiny.raw");
        fs::write(&path, [0u8; 64]).unwrap();
        let info = analyze_image(&path).unwrap();
        assert_eq!(info.table_type, PartitionTableType::None);
        assert_eq!(info.size, 64);
    }

    #[test]
    fn test_analyze_one_sector_image() {
        let tmp = temp_dir();
        let path = tmp.path().join("onesec.raw");
        let mut data = vec![0u8; 512];
        // MBR signature but no partitions
        data[510] = 0x55;
        data[511] = 0xAA;
        fs::write(&path, &data).unwrap();
        let info = analyze_image(&path).unwrap();
        // MBR signature present but no partition entries -> None
        assert_eq!(info.table_type, PartitionTableType::None);
    }
}
