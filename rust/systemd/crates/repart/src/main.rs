//! systemd-repart — Declarative GPT partition manager.
//!
//! A drop-in replacement for `systemd-repart(8)`. Grows existing partitions and
//! creates new ones based on `repart.d/*.conf` definition files.
//!
//! Usage:
//!
//!   systemd-repart [OPTIONS...] [BLOCKDEVICE]
//!       Repartition the specified block device or image file according to
//!       repart.d/*.conf definitions.
//!
//!   systemd-repart --help | -h
//!       Show help.
//!
//!   systemd-repart --version
//!       Show version.
//!
//! Key options:
//!   --dry-run=BOOL          Only show what would be done (default: yes)
//!   --empty=MODE            How to handle empty disks (refuse/allow/require/force/create)
//!   --size=BYTES|auto       Size for image file (with --empty=create)
//!   --definitions=PATH      Read *.conf from PATH instead of default dirs
//!   --seed=UUID|random      Seed for UUID generation
//!   --factory-reset=BOOL    Remove partitions marked FactoryReset=yes
//!   --can-factory-reset     Check if factory reset is possible
//!   --pretty=BOOL           Show user-friendly table
//!   --json=MODE             JSON output (off/short/pretty)
//!   --no-pager              No pager
//!   --no-legend             No legend
//!   --sector-size=BYTES     Sector size (default: 512)
//!   --architecture=ARCH     Override architecture for type resolution
//!   --offline=BOOL|auto     Build image without loop devices
//!   --list-devices          List candidate block devices
//!   --split=BOOL            Generate split artifacts

use std::collections::HashMap;
use std::fmt;
use std::fs;
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::process;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const GPT_SIGNATURE: &[u8; 8] = b"EFI PART";
const SECTOR_SIZE_DEFAULT: u64 = 512;
const GPT_HEADER_LBA: u64 = 1;
const GPT_HEADER_MIN_SIZE: usize = 92;
const GPT_HEADER_SIZE: usize = 92;
const GPT_ENTRY_SIZE: u64 = 128;
const MBR_SIGNATURE: [u8; 2] = [0x55, 0xAA];
const GPT_REVISION_1_0: u32 = 0x00010000;

/// Minimum partition size (4 KiB).
const MIN_PARTITION_SIZE: u64 = 4096;

/// Default minimum size for partitions (10 MiB).
const DEFAULT_SIZE_MIN: u64 = 10 * 1024 * 1024;

/// Default weight for partitions.
const DEFAULT_WEIGHT: u32 = 1000;

/// Maximum number of GPT partition entries (standard).
const MAX_GPT_ENTRIES: u32 = 128;

/// A GPT partition label holds 36 UTF-16 code units.
const GPT_LABEL_MAX_UTF16: usize = 36;

/// Alignment grain libfdisk uses for a fresh GPT's first usable LBA.
const GPT_FIRST_LBA_GRAIN: u64 = 1024 * 1024;

/// Standard repart.d search paths.
const DEFINITION_SEARCH_PATHS: &[&str] = &[
    "/etc/repart.d",
    "/run/repart.d",
    "/usr/local/lib/repart.d",
    "/usr/lib/repart.d",
];

const ZERO_GUID: &str = "00000000-0000-0000-0000-000000000000";

// ---------------------------------------------------------------------------
// GPT partition type identifiers → UUIDs
// ---------------------------------------------------------------------------

fn partition_type_uuid(identifier: &str, arch: &str) -> Option<String> {
    // Resolve "root" → "root-{arch}", "usr" → "usr-{arch}", etc.
    let resolved = resolve_arch_type(identifier, arch);
    let key = resolved.as_deref().unwrap_or(identifier);

    match key {
        "esp" => Some("c12a7328-f81f-11d2-ba4b-00a0c93ec93b".into()),
        "xbootldr" => Some("bc13c2ff-59e6-4262-a352-b275fd6f7172".into()),
        "swap" => Some("0657fd6d-a4ab-43c4-84e5-0933c84b4f4f".into()),
        "home" => Some("933ac7e1-2eb4-4f13-b844-0e14e2aef915".into()),
        "srv" => Some("3b8f8425-20e0-4f3b-907f-1a25a76f98e8".into()),
        "var" => Some("4d21b016-b534-45c2-a9fb-5c16e091fd2d".into()),
        "tmp" => Some("7ec6f557-3bc5-4aca-b293-16ef5df639d1".into()),
        "linux-generic" => Some("0fc63daf-8483-4772-8e79-3d69d8477de4".into()),

        // Root partitions
        "root-x86-64" => Some("4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into()),
        "root-x86" => Some("44479540-f297-41b2-9af7-d131d5f0458a".into()),
        "root-arm64" => Some("b921b045-1df0-41c3-af44-4c6f280d3fae".into()),
        "root-arm" => Some("69dad710-2ce4-4e3c-b16c-21a1d49abed3".into()),
        "root-riscv64" => Some("72ec70a6-cf74-40e6-bd49-4bda08e8f224".into()),
        "root-riscv32" => Some("60d5a7fe-8e7d-435c-b714-3dd8162144e1".into()),
        "root-loongarch64" => Some("77055800-792c-4f94-b39a-98c91b762bb6".into()),
        "root-s390x" => Some("08a7acea-624c-4a20-91e8-6e0fa67d23f9".into()),
        "root-s390" => Some("5eead065-799b-4e73-8540-2ccb9ed25df2".into()),
        "root-ppc64-le" => Some("c31c45e6-3f39-412e-80f4-68b47e6fa78e".into()),
        "root-ppc64" => Some("912ade1d-a839-4913-8964-a10eee08fbd2".into()),
        "root-ppc" => Some("1de3f1ef-fa98-47b5-8dcd-4a860a654d78".into()),
        "root-ia64" => Some("993d8d3d-f80e-4225-855a-9daf8ed7ea97".into()),
        "root-mips-le" => Some("37c58c8a-d913-4156-a25f-48b1b64e07f0".into()),
        "root-mips64-le" => Some("700bda43-7a34-4507-b179-eeb93d7a7ca3".into()),
        "root-parisc" => Some("1aacdb3b-5444-4138-bd9e-e5c2239b2346".into()),

        // Root verity
        "root-x86-64-verity" => Some("2c7357ed-ebd2-46d9-aec1-23d437ec2bf5".into()),
        "root-x86-verity" => Some("d13c5d3b-b5d1-422a-b29f-9454fdc89d76".into()),
        "root-arm64-verity" => Some("df3300ce-d69f-4c92-978c-9bfb0f38d820".into()),
        "root-arm-verity" => Some("7386cdf2-203c-47a9-a498-f2ecce45a2d6".into()),
        "root-riscv64-verity" => Some("b6ed5582-440b-4209-b8da-5ff7c419ea3d".into()),
        "root-riscv32-verity" => Some("ae0253be-1167-4007-ac68-43926c14c5de".into()),

        // Root verity signature
        "root-x86-64-verity-sig" => Some("41092b05-9fc8-4523-994f-2def0408b176".into()),
        "root-arm64-verity-sig" => Some("6db69de6-29f4-4758-a7a5-962190f00ce3".into()),

        // /usr partitions
        "usr-x86-64" => Some("8484680c-9521-48c6-9c11-b0720656f69e".into()),
        "usr-x86" => Some("75250d76-8cc6-458e-bd66-bd47cc81a812".into()),
        "usr-arm64" => Some("b0e01050-ee5f-4390-949a-9101b17104e9".into()),
        "usr-arm" => Some("7d0359a3-02b3-4f0a-865c-654403e70625".into()),
        "usr-riscv64" => Some("beaec34b-8442-439b-a40b-984381ed097d".into()),
        "usr-riscv32" => Some("b933fb22-5c3f-4f91-af90-e2bb0fa50702".into()),

        // /usr verity
        "usr-x86-64-verity" => Some("77ff5f63-e7b6-4633-acf4-1565b864c0e6".into()),
        "usr-arm64-verity" => Some("c215d751-7bcd-4649-be90-6627490a4c05".into()),

        // /usr verity signature
        "usr-x86-64-verity-sig" => Some("e7bb33fb-06cf-4e81-8273-e543b413e2e2".into()),
        "usr-arm64-verity-sig" => Some("c2b17d5b-f01e-4994-bda5-26171d070c68".into()),

        _ => {
            // If it looks like a raw UUID, return it as-is
            if key.len() == 36 && key.chars().filter(|c| *c == '-').count() == 4 {
                Some(key.to_lowercase())
            } else {
                None
            }
        }
    }
}

fn resolve_arch_type(identifier: &str, arch: &str) -> Option<String> {
    match identifier {
        "root" => Some(format!("root-{arch}")),
        "root-verity" => Some(format!("root-{arch}-verity")),
        "root-verity-sig" => Some(format!("root-{arch}-verity-sig")),
        "usr" => Some(format!("usr-{arch}")),
        "usr-verity" => Some(format!("usr-{arch}-verity")),
        "usr-verity-sig" => Some(format!("usr-{arch}-verity-sig")),
        _ => None,
    }
}

/// Whether repart knows how to grow this partition type's file system,
/// mirroring `gpt_partition_type_knows_growfs`: root, usr, home, srv, var, tmp
/// and xbootldr, but not their verity siblings, which are read-only by nature.
fn type_knows_growfs(type_uuid: &str) -> bool {
    match type_uuid_to_identifier(&type_uuid.to_lowercase()) {
        Some(id) => {
            matches!(id, "home" | "srv" | "var" | "tmp" | "xbootldr")
                || ((id.starts_with("root-") || id.starts_with("usr-"))
                    && !id.ends_with("-verity"))
        }
        None => false,
    }
}

fn type_uuid_to_identifier(uuid: &str) -> Option<&'static str> {
    match uuid {
        "c12a7328-f81f-11d2-ba4b-00a0c93ec93b" => Some("esp"),
        "bc13c2ff-59e6-4262-a352-b275fd6f7172" => Some("xbootldr"),
        "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f" => Some("swap"),
        "933ac7e1-2eb4-4f13-b844-0e14e2aef915" => Some("home"),
        "3b8f8425-20e0-4f3b-907f-1a25a76f98e8" => Some("srv"),
        "4d21b016-b534-45c2-a9fb-5c16e091fd2d" => Some("var"),
        "7ec6f557-3bc5-4aca-b293-16ef5df639d1" => Some("tmp"),
        "0fc63daf-8483-4772-8e79-3d69d8477de4" => Some("linux-generic"),
        "4f68bce3-e8cd-4db1-96e7-fbcaf984b709" => Some("root-x86-64"),
        "44479540-f297-41b2-9af7-d131d5f0458a" => Some("root-x86"),
        "b921b045-1df0-41c3-af44-4c6f280d3fae" => Some("root-arm64"),
        "69dad710-2ce4-4e3c-b16c-21a1d49abed3" => Some("root-arm"),
        "72ec70a6-cf74-40e6-bd49-4bda08e8f224" => Some("root-riscv64"),
        "8484680c-9521-48c6-9c11-b0720656f69e" => Some("usr-x86-64"),
        "b0e01050-ee5f-4390-949a-9101b17104e9" => Some("usr-arm64"),
        "2c7357ed-ebd2-46d9-aec1-23d437ec2bf5" => Some("root-x86-64-verity"),
        "77ff5f63-e7b6-4633-acf4-1565b864c0e6" => Some("usr-x86-64-verity"),
        _ => None,
    }
}

fn detect_architecture() -> &'static str {
    #[cfg(target_arch = "x86_64")]
    {
        "x86-64"
    }
    #[cfg(target_arch = "aarch64")]
    {
        "arm64"
    }
    #[cfg(target_arch = "x86")]
    {
        "x86"
    }
    #[cfg(target_arch = "arm")]
    {
        "arm"
    }
    #[cfg(target_arch = "riscv64")]
    {
        "riscv64"
    }
    #[cfg(target_arch = "riscv32")]
    {
        "riscv32"
    }
    #[cfg(target_arch = "s390x")]
    {
        "s390x"
    }
    #[cfg(target_arch = "powerpc64")]
    {
        "ppc64-le"
    }
    #[cfg(not(any(
        target_arch = "x86_64",
        target_arch = "aarch64",
        target_arch = "x86",
        target_arch = "arm",
        target_arch = "riscv64",
        target_arch = "riscv32",
        target_arch = "s390x",
        target_arch = "powerpc64"
    )))]
    {
        "x86-64"
    }
}

// ---------------------------------------------------------------------------
// GUID helpers
// ---------------------------------------------------------------------------

fn parse_guid(data: &[u8]) -> String {
    if data.len() < 16 {
        return ZERO_GUID.to_string();
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

fn encode_guid(guid: &str) -> [u8; 16] {
    let hex: String = guid.replace('-', "");
    if hex.len() != 32 {
        return [0u8; 16];
    }
    let bytes: Vec<u8> = (0..32)
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i + 2], 16).unwrap_or(0))
        .collect();
    if bytes.len() != 16 {
        return [0u8; 16];
    }
    // Mixed endian encoding for GPT GUIDs:
    // time_low (4 bytes LE), time_mid (2 bytes LE), time_hi (2 bytes LE), rest big-endian
    let mut out = [0u8; 16];
    // time_low: bytes 0-3, stored little-endian
    out[0] = bytes[3];
    out[1] = bytes[2];
    out[2] = bytes[1];
    out[3] = bytes[0];
    // time_mid: bytes 4-5, stored little-endian
    out[4] = bytes[5];
    out[5] = bytes[4];
    // time_hi: bytes 6-7, stored little-endian
    out[6] = bytes[7];
    out[7] = bytes[6];
    // clock_seq and node: bytes 8-15, stored as-is (big-endian)
    out[8..16].copy_from_slice(&bytes[8..16]);
    out
}

fn is_zero_guid(guid: &str) -> bool {
    guid == ZERO_GUID
}

/// HMAC-SHA256, as upstream uses to derive every reproducible UUID.
fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    use sha2::{Digest, Sha256};
    const BLOCK_SIZE: usize = 64;

    let mut padded_key = [0u8; BLOCK_SIZE];
    if key.len() > BLOCK_SIZE {
        padded_key[..32].copy_from_slice(&Sha256::digest(key));
    } else {
        padded_key[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; BLOCK_SIZE];
    let mut opad = [0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= padded_key[i];
        opad[i] ^= padded_key[i];
    }

    let inner = {
        let mut h = Sha256::new();
        h.update(ipad);
        h.update(data);
        h.finalize()
    };
    let outer = {
        let mut h = Sha256::new();
        h.update(opad);
        h.update(inner);
        h.finalize()
    };

    let mut out = [0u8; 32];
    out.copy_from_slice(&outer);
    out
}

/// The 16 raw bytes of a UUID in text order, ignoring dashes.
fn uuid_bytes(uuid: &str) -> Option<[u8; 16]> {
    let hex: String = uuid.chars().filter(|c| *c != '-').collect();
    if hex.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, b) in out.iter_mut().enumerate() {
        *b = u8::from_str_radix(hex.get(i * 2..i * 2 + 2)?, 16).ok()?;
    }
    Some(out)
}

/// Take the first half of a digest and mark it as a v4 UUID, as
/// `id128_make_v4_uuid` does.
fn make_v4_uuid(digest: &[u8; 32]) -> String {
    let mut b = [0u8; 16];
    b.copy_from_slice(&digest[..16]);
    b[6] = (b[6] & 0x0F) | 0x40;
    b[8] = (b[8] & 0x3F) | 0x80;

    let h: String = b.iter().map(|x| format!("{x:02x}")).collect();
    format!(
        "{}-{}-{}-{}-{}",
        &h[0..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..32]
    )
    .to_uppercase()
}

/// The HMAC key for a seed: its 16 raw bytes when it is a UUID (or a bare
/// machine ID), otherwise the string itself, so an unusual seed still derives
/// something stable rather than failing.
fn seed_key(seed: &str) -> Vec<u8> {
    match uuid_bytes(seed) {
        Some(b) => b.to_vec(),
        None => seed.as_bytes().to_vec(),
    }
}

/// Derive a UUID from the seed and a token, mirroring upstream `derive_uuid`.
///
/// The seed is the HMAC *key*, never the plaintext, because the seed is
/// typically the machine ID and must not leak into the result.
fn derive_uuid(seed: &str, token: &str) -> String {
    make_v4_uuid(&hmac_sha256(&seed_key(seed), token.as_bytes()))
}

/// Derive a partition's UUID from the seed, its type and its instance number,
/// mirroring upstream `partition_acquire_uuid`.
///
/// The first partition of a given type hashes the bare type UUID; every later
/// one appends its LE64 instance number, so adding a second partition of a type
/// does not renumber the first.
fn derive_partition_uuid(seed: &str, type_uuid: &str, instance: u64) -> String {
    let type_bytes = match uuid_bytes(type_uuid) {
        Some(b) => b.to_vec(),
        None => type_uuid.as_bytes().to_vec(),
    };

    let mut msg = type_bytes;
    if instance > 0 {
        msg.extend_from_slice(&instance.to_le_bytes());
    }

    make_v4_uuid(&hmac_sha256(&seed_key(seed), &msg))
}

fn generate_random_uuid() -> String {
    // Read from /dev/urandom for random UUID
    let mut bytes = [0u8; 16];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut bytes);
    } else {
        // Fallback: use process ID and timestamp
        let pid = process::id();
        let ptr = &bytes as *const _ as u64;
        bytes[0..4].copy_from_slice(&pid.to_le_bytes());
        bytes[4..12].copy_from_slice(&ptr.to_le_bytes());
    }
    // Set version 4 and variant 1
    bytes[6] = (bytes[6] & 0x0F) | 0x40;
    bytes[8] = (bytes[8] & 0x3F) | 0x80;

    format!(
        "{:08x}-{:04x}-{:04x}-{:02x}{:02x}-{:02x}{:02x}{:02x}{:02x}{:02x}{:02x}",
        u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
        u16::from_le_bytes([bytes[4], bytes[5]]),
        u16::from_le_bytes([bytes[6], bytes[7]]),
        bytes[8],
        bytes[9],
        bytes[10],
        bytes[11],
        bytes[12],
        bytes[13],
        bytes[14],
        bytes[15]
    )
}

// ---------------------------------------------------------------------------
// CRC32 for GPT
// ---------------------------------------------------------------------------

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFFFFFF;
    for &byte in data {
        crc ^= byte as u32;
        for _ in 0..8 {
            if crc & 1 != 0 {
                crc = (crc >> 1) ^ 0xEDB88320;
            } else {
                crc >>= 1;
            }
        }
    }
    !crc
}

// ---------------------------------------------------------------------------
// Size parsing
// ---------------------------------------------------------------------------

fn parse_size(s: &str) -> Result<u64, String> {
    let s = s.trim();
    if s.is_empty() {
        return Err("empty size".into());
    }
    if s.eq_ignore_ascii_case("infinity") {
        return Ok(u64::MAX);
    }

    let (num_str, multiplier) = if let Some(n) = s.strip_suffix('K').or_else(|| s.strip_suffix('k'))
    {
        (n, 1024u64)
    } else if let Some(n) = s.strip_suffix('M').or_else(|| s.strip_suffix('m')) {
        (n, 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('G').or_else(|| s.strip_suffix('g')) {
        (n, 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('T').or_else(|| s.strip_suffix('t')) {
        (n, 1024u64 * 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('P').or_else(|| s.strip_suffix('p')) {
        (n, 1024u64 * 1024 * 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('E').or_else(|| s.strip_suffix('e')) {
        (n, 1024u64 * 1024 * 1024 * 1024 * 1024 * 1024)
    } else {
        (s, 1u64)
    };

    let num: u64 = num_str
        .trim()
        .parse()
        .map_err(|_| format!("invalid size: {s}"))?;
    num.checked_mul(multiplier)
        .ok_or_else(|| format!("size overflow: {s}"))
}

fn format_size(bytes: u64) -> String {
    const UNITS: &[(u64, &str)] = &[
        (1024 * 1024 * 1024 * 1024, "T"),
        (1024 * 1024 * 1024, "G"),
        (1024 * 1024, "M"),
        (1024, "K"),
    ];

    for &(threshold, suffix) in UNITS {
        if bytes >= threshold {
            let whole = bytes / threshold;
            let frac = ((bytes % threshold) * 10) / threshold;
            if frac > 0 {
                return format!("{whole}.{frac}{suffix}");
            }
            return format!("{whole}{suffix}");
        }
    }
    format!("{bytes}B")
}

// ---------------------------------------------------------------------------
// Partition definition file parsing
// ---------------------------------------------------------------------------

/// Parsed `repart.d/*.conf` partition definition.
#[derive(Debug, Clone)]
#[allow(dead_code)]
struct PartitionDefinition {
    /// Filename (without directory).
    filename: String,
    /// Full path to the definition file.
    path: PathBuf,
    /// GPT partition type UUID.
    type_uuid: String,
    /// Type identifier (e.g. "root-x86-64", "home", "swap").
    type_id: Option<String>,
    /// Partition label.
    label: Option<String>,
    /// Partition UUID (or "null" for all-zero).
    uuid: Option<String>,
    /// Priority for dropping when space is insufficient.
    priority: i32,
    /// Weight for space distribution (0..1000000).
    weight: u32,
    /// Padding weight (0..1000000).
    padding_weight: u32,
    /// Minimum size in bytes.
    size_min: u64,
    /// Maximum size in bytes.
    size_max: u64,
    /// Minimum padding in bytes.
    padding_min: u64,
    /// Maximum padding in bytes.
    padding_max: u64,
    /// Mark for factory reset removal.
    factory_reset: bool,
    /// 64-bit GPT partition flags.
    flags: u64,
    /// NoAuto flag (bit 63).
    no_auto: Option<bool>,
    /// ReadOnly flag (bit 60).
    read_only: Option<bool>,
    /// GrowFileSystem flag (bit 59).
    grow_fs: Option<bool>,
    /// Filesystem to format (e.g. "ext4", "vfat", "swap").
    format: Option<String>,
    /// Files to copy into the partition.
    copy_files: Vec<String>,
    /// Path or "auto" for block-level copy.
    copy_blocks: Option<String>,
    /// Directories to create.
    make_directories: Vec<String>,
    /// Encryption mode.
    encrypt: Option<String>,
    /// Verity mode (off/data/hash/signature).
    verity: Option<String>,
    /// Verity match key.
    verity_match_key: Option<String>,
    /// SplitName template.
    split_name: Option<String>,
    /// Minimize mode (off/best/guess).
    minimize: Option<String>,
    /// Supplementary target definition name.
    supplement_for: Option<String>,
    /// MountPoint setting.
    mount_point: Option<String>,
    /// EncryptedVolume setting.
    encrypted_volume: Option<String>,
    /// Compression algorithm.
    compression: Option<String>,
    /// Compression level.
    compression_level: Option<String>,
}

impl Default for PartitionDefinition {
    fn default() -> Self {
        Self {
            filename: String::new(),
            path: PathBuf::new(),
            type_uuid: ZERO_GUID.to_string(),
            type_id: None,
            label: None,
            uuid: None,
            priority: 0,
            weight: DEFAULT_WEIGHT,
            padding_weight: 0,
            size_min: DEFAULT_SIZE_MIN,
            size_max: u64::MAX,
            padding_min: 0,
            padding_max: u64::MAX,
            factory_reset: false,
            flags: 0,
            no_auto: None,
            read_only: None,
            grow_fs: None,
            format: None,
            copy_files: Vec::new(),
            copy_blocks: None,
            make_directories: Vec::new(),
            encrypt: None,
            verity: None,
            verity_match_key: None,
            split_name: None,
            minimize: None,
            supplement_for: None,
            mount_point: None,
            encrypted_volume: None,
            compression: None,
            compression_level: None,
        }
    }
}

impl PartitionDefinition {
    /// Compute the effective 64-bit flags field.
    fn effective_flags(&self) -> u64 {
        let mut f = self.flags;
        if let Some(true) = self.no_auto {
            f |= 1u64 << 63;
        } else if let Some(false) = self.no_auto {
            f &= !(1u64 << 63);
        }
        if let Some(true) = self.read_only {
            f |= 1u64 << 60;
        } else if let Some(false) = self.read_only {
            f &= !(1u64 << 60);
        }
        match self.grow_fs {
            Some(true) => f |= 1u64 << 59,
            Some(false) => f &= !(1u64 << 59),
            // Upstream defaults growfs ON for every type whose file system it
            // knows how to grow, unless the partition is read-only. A plain
            // `Type=home` therefore carries GUID:59 with no GrowFileSystem= in
            // the definition at all, which is what sfdisk reports as
            // attrs="GUID:59".
            None => {
                if type_knows_growfs(&self.type_uuid) && !matches!(self.read_only, Some(true)) {
                    f |= 1u64 << 59;
                }
            }
        }
        f
    }

    /// Derive a label from the partition type if none is set.
    fn effective_label(&self) -> String {
        if let Some(ref l) = self.label {
            l.clone()
        } else if let Some(ref id) = self.type_id {
            id.replace('-', " ")
        } else if let Some(name) = type_uuid_to_identifier(&self.type_uuid) {
            name.replace('-', " ")
        } else {
            "Linux".to_string()
        }
    }
}

fn parse_bool(s: &str) -> Result<bool, String> {
    match s.to_lowercase().as_str() {
        "1" | "yes" | "true" | "on" => Ok(true),
        "0" | "no" | "false" | "off" => Ok(false),
        _ => Err(format!("invalid boolean: {s}")),
    }
}

fn parse_partition_definition(path: &Path, arch: &str) -> Result<PartitionDefinition, String> {
    let content =
        fs::read_to_string(path).map_err(|e| format!("Cannot read {}: {}", path.display(), e))?;

    let filename = path
        .file_name()
        .unwrap_or_default()
        .to_string_lossy()
        .to_string();

    let mut def = PartitionDefinition {
        filename: filename.clone(),
        path: path.to_path_buf(),
        ..Default::default()
    };

    let mut in_partition_section = false;

    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }
        if line.starts_with('[') {
            in_partition_section = line.eq_ignore_ascii_case("[partition]");
            continue;
        }
        if !in_partition_section {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let key = key.trim();
        let value = value.trim();

        match key {
            "Type" => {
                if let Some(uuid) = partition_type_uuid(value, arch) {
                    def.type_uuid = uuid;
                    def.type_id = Some(value.to_string());
                } else {
                    return Err(format!("Unknown partition type: {value}"));
                }
            }
            "Label" => {
                // A GPT label holds 36 UTF-16 code units. Upstream validates
                // here and IGNORES an over-long one, so an earlier Label= in
                // the same file survives; silently truncating instead wrote a
                // name nobody asked for. A definition may legitimately offer a
                // long label and a short fallback in that order.
                if value.encode_utf16().count() > GPT_LABEL_MAX_UTF16 {
                    eprintln!(
                        "Partition label too long for GPT table, ignoring: \"{value}\""
                    );
                } else {
                    def.label = if value.is_empty() {
                        None
                    } else {
                        Some(value.to_string())
                    };
                }
            }
            "UUID" => {
                def.uuid = if value.is_empty() {
                    None
                } else if value == "null" {
                    Some(ZERO_GUID.to_string())
                } else {
                    Some(value.to_lowercase())
                };
            }
            "Priority" => {
                def.priority = value
                    .parse()
                    .map_err(|_| format!("Invalid Priority: {value}"))?;
            }
            "Weight" => {
                def.weight = value
                    .parse()
                    .map_err(|_| format!("Invalid Weight: {value}"))?;
                if def.weight > 1_000_000 {
                    return Err(format!("Weight out of range: {value}"));
                }
            }
            "PaddingWeight" => {
                def.padding_weight = value
                    .parse()
                    .map_err(|_| format!("Invalid PaddingWeight: {value}"))?;
                if def.padding_weight > 1_000_000 {
                    return Err(format!("PaddingWeight out of range: {value}"));
                }
            }
            "SizeMinBytes" => {
                def.size_min = parse_size(value)?;
            }
            "SizeMaxBytes" => {
                def.size_max = parse_size(value)?;
            }
            "PaddingMinBytes" => {
                def.padding_min = parse_size(value)?;
            }
            "PaddingMaxBytes" => {
                def.padding_max = parse_size(value)?;
            }
            "FactoryReset" => {
                def.factory_reset = parse_bool(value)?;
            }
            "Flags" => {
                let v = if let Some(hex) = value.strip_prefix("0x") {
                    u64::from_str_radix(hex, 16).map_err(|_| format!("Invalid Flags: {value}"))?
                } else if let Some(bin) = value.strip_prefix("0b") {
                    u64::from_str_radix(bin, 2).map_err(|_| format!("Invalid Flags: {value}"))?
                } else {
                    value
                        .parse()
                        .map_err(|_| format!("Invalid Flags: {value}"))?
                };
                def.flags = v;
            }
            "NoAuto" => {
                def.no_auto = Some(parse_bool(value)?);
            }
            "ReadOnly" => {
                def.read_only = Some(parse_bool(value)?);
            }
            "GrowFileSystem" => {
                def.grow_fs = Some(parse_bool(value)?);
            }
            "Format" => {
                def.format = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "CopyFiles" => {
                if value.is_empty() {
                    def.copy_files.clear();
                } else {
                    def.copy_files.push(value.to_string());
                }
            }
            "CopyBlocks" => {
                def.copy_blocks = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "MakeDirectories" => {
                if value.is_empty() {
                    def.make_directories.clear();
                } else {
                    for dir in value.split_whitespace() {
                        def.make_directories.push(dir.to_string());
                    }
                }
            }
            "Encrypt" => {
                def.encrypt =
                    if value.is_empty() || value == "off" || value == "no" || value == "false" {
                        None
                    } else {
                        Some(value.to_string())
                    };
            }
            "Verity" => {
                def.verity = if value.is_empty() || value == "off" {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "VerityMatchKey" => {
                def.verity_match_key = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "SplitName" => {
                def.split_name = if value.is_empty() || value == "-" {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "Minimize" => {
                def.minimize = match value.to_lowercase().as_str() {
                    "" | "off" | "no" | "false" | "0" => None,
                    "best" | "yes" | "true" | "1" => Some("best".into()),
                    "guess" => Some("guess".into()),
                    _ => return Err(format!("Invalid Minimize: {value}")),
                };
            }
            "SupplementFor" => {
                def.supplement_for = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "MountPoint" => {
                def.mount_point = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "EncryptedVolume" => {
                def.encrypted_volume = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "Compression" => {
                def.compression = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "CompressionLevel" => {
                def.compression_level = if value.is_empty() {
                    None
                } else {
                    Some(value.to_string())
                };
            }
            "VerityDataBlockSizeBytes"
            | "VerityHashBlockSizeBytes"
            | "KeyFile"
            | "TPM2PCRs"
            | "Subvolumes"
            | "DefaultSubvolume"
            | "MakeSymlinks"
            | "ExcludeFiles"
            | "ExcludeFilesTarget"
            | "VolumeLabel"
            | "AddValidateFS"
            | "FileSystemSectorSize" => {
                // Recognized but not yet implemented — silently accept
            }
            _ => {
                // Unknown key — silently ignore for forward compatibility
            }
        }
    }

    Ok(def)
}

/// Load partition definitions from a list of directories.
/// Files are deduplicated by filename (first occurrence wins) and sorted.
fn load_definitions(dirs: &[&str], arch: &str) -> Result<Vec<PartitionDefinition>, String> {
    let mut seen: HashMap<String, PathBuf> = HashMap::new();

    for dir in dirs {
        let dir_path = Path::new(dir);
        let entries = match fs::read_dir(dir_path) {
            Ok(e) => e,
            Err(_) => continue,
        };

        let mut names: Vec<(String, PathBuf)> = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if !name.ends_with(".conf") {
                continue;
            }
            names.push((name, entry.path()));
        }
        names.sort_by(|a, b| a.0.cmp(&b.0));

        for (name, path) in names {
            seen.entry(name).or_insert(path);
        }
    }

    let mut files: Vec<(String, PathBuf)> = seen.into_iter().collect();
    files.sort_by(|a, b| a.0.cmp(&b.0));

    let mut defs = Vec::new();
    for (_, path) in &files {
        defs.push(parse_partition_definition(path, arch)?);
    }

    Ok(defs)
}

/// Distribute free space across weighted claims, handing on whatever a capped
/// claim cannot take.
///
/// Each entry is `(weight, cap)`, the cap being the most that claim can still
/// absorb. A single weighted pass gives everyone their share and then clamps
/// the ones that overshoot, which silently wastes the difference: on a 1 GiB
/// disk holding an unbounded home and a swap capped at 64 MiB, home was left
/// with half the disk and roughly 400 MiB went unused. Settling one capped
/// claim at a time and sharing the rest out again is what upstream does.
fn distribute_weighted(entries: &[(u64, u64)], free: u64) -> Vec<u64> {
    let mut shares = vec![0u64; entries.len()];
    let mut active: Vec<usize> = (0..entries.len()).filter(|&i| entries[i].0 > 0).collect();
    let mut free = free;

    loop {
        let total: u64 = active.iter().map(|&i| entries[i].0).sum();
        if total == 0 || free == 0 {
            break;
        }

        let weighted = |i: usize| (free as u128 * entries[i].0 as u128 / total as u128) as u64;

        match active.iter().position(|&i| weighted(i) >= entries[i].1) {
            Some(pos) => {
                let i = active.remove(pos);
                shares[i] = entries[i].1;
                free -= entries[i].1;
            }
            None => {
                for &i in &active {
                    shares[i] = weighted(i);
                }
                break;
            }
        }
    }

    shares
}

/// Resolve a `--include-partitions=`/`--exclude-partitions=` entry to a type
/// UUID. Entries may be a designator such as `home` or a bare UUID.
fn filter_entry_uuid(entry: &str, arch: &str) -> Option<String> {
    partition_type_uuid(entry, arch).or_else(|| uuid_bytes(entry).map(|_| entry.to_uppercase()))
}

/// Apply the partition type filter, mirroring upstream `partition_type_exclude`.
///
/// The two options are one list and one mode, not two independent lists: with
/// an include list anything unlisted is dropped, with an exclude list anything
/// listed is dropped, and with neither everything is kept.
fn filter_definitions_by_type(
    defs: Vec<PartitionDefinition>,
    args: &Args,
    arch: &str,
) -> Vec<PartitionDefinition> {
    let (list, include) = if !args.include_partitions.is_empty() {
        (&args.include_partitions, true)
    } else if !args.exclude_partitions.is_empty() {
        (&args.exclude_partitions, false)
    } else {
        return defs;
    };

    let wanted: Vec<String> = list
        .iter()
        .filter_map(|e| filter_entry_uuid(e, arch))
        .collect();

    defs.into_iter()
        .filter(|d| {
            let listed = wanted.iter().any(|u| u.eq_ignore_ascii_case(&d.type_uuid));
            if listed { include } else { !include }
        })
        .collect()
}

fn load_definitions_from_dir(dir: &str, arch: &str) -> Result<Vec<PartitionDefinition>, String> {
    load_definitions(&[dir], arch)
}

// ---------------------------------------------------------------------------
// GPT data structures
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct GptHeader {
    revision: u32,
    header_size: u32,
    header_crc32: u32,
    my_lba: u64,
    alternate_lba: u64,
    first_usable_lba: u64,
    last_usable_lba: u64,
    disk_guid: String,
    partition_entry_lba: u64,
    num_partition_entries: u32,
    partition_entry_size: u32,
    partition_entries_crc32: u32,
}

#[derive(Debug, Clone)]
#[allow(dead_code)]
struct GptPartition {
    type_guid: String,
    unique_guid: String,
    first_lba: u64,
    last_lba: u64,
    attributes: u64,
    name: String,
    /// Slot index in the partition table (0-based internally).
    slot_index: u32,
}

#[allow(dead_code)]
impl GptPartition {
    fn size_bytes(&self, sector_size: u64) -> u64 {
        (self.last_lba - self.first_lba + 1) * sector_size
    }

    fn size_sectors(&self) -> u64 {
        self.last_lba - self.first_lba + 1
    }
}

// ---------------------------------------------------------------------------
// GPT reading
// ---------------------------------------------------------------------------

fn read_gpt_header(file: &mut fs::File, sector_size: u64) -> io::Result<Option<GptHeader>> {
    let mut buf = vec![0u8; sector_size as usize];

    file.seek(SeekFrom::Start(GPT_HEADER_LBA * sector_size))?;
    let n = file.read(&mut buf)?;
    if n < GPT_HEADER_MIN_SIZE {
        return Ok(None);
    }

    if &buf[0..8] != GPT_SIGNATURE {
        return Ok(None);
    }

    let revision = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
    let header_size = u32::from_le_bytes([buf[12], buf[13], buf[14], buf[15]]);
    let header_crc32_val = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
    let my_lba = u64::from_le_bytes(buf[24..32].try_into().unwrap());
    let alternate_lba = u64::from_le_bytes(buf[32..40].try_into().unwrap());
    let first_usable_lba = u64::from_le_bytes(buf[40..48].try_into().unwrap());
    let last_usable_lba = u64::from_le_bytes(buf[48..56].try_into().unwrap());
    let disk_guid = parse_guid(&buf[56..72]);
    let partition_entry_lba = u64::from_le_bytes(buf[72..80].try_into().unwrap());
    let num_partition_entries = u32::from_le_bytes([buf[80], buf[81], buf[82], buf[83]]);
    let partition_entry_size = u32::from_le_bytes([buf[84], buf[85], buf[86], buf[87]]);
    let partition_entries_crc32_val = u32::from_le_bytes([buf[88], buf[89], buf[90], buf[91]]);

    Ok(Some(GptHeader {
        revision,
        header_size,
        header_crc32: header_crc32_val,
        my_lba,
        alternate_lba,
        first_usable_lba,
        last_usable_lba,
        disk_guid,
        partition_entry_lba,
        num_partition_entries,
        partition_entry_size,
        partition_entries_crc32: partition_entries_crc32_val,
    }))
}

fn read_gpt_partitions(
    file: &mut fs::File,
    header: &GptHeader,
    sector_size: u64,
) -> io::Result<Vec<GptPartition>> {
    let mut partitions = Vec::new();
    let entry_size = header.partition_entry_size.max(GPT_ENTRY_SIZE as u32) as u64;
    let start_offset = header.partition_entry_lba * sector_size;

    for i in 0..header.num_partition_entries {
        let offset = start_offset + (i as u64) * entry_size;
        file.seek(SeekFrom::Start(offset))?;

        let mut entry = vec![0u8; entry_size as usize];
        let n = file.read(&mut entry)?;
        if n < 128 {
            break;
        }

        let type_guid = parse_guid(&entry[0..16]);
        if is_zero_guid(&type_guid) {
            continue;
        }

        let unique_guid = parse_guid(&entry[16..32]);
        let first_lba = u64::from_le_bytes(entry[32..40].try_into().unwrap());
        let last_lba = u64::from_le_bytes(entry[40..48].try_into().unwrap());
        let attributes = u64::from_le_bytes(entry[48..56].try_into().unwrap());
        let name = parse_utf16le_name(&entry[56..entry_size.min(128) as usize]);

        partitions.push(GptPartition {
            type_guid,
            unique_guid,
            first_lba,
            last_lba,
            attributes,
            name,
            slot_index: i,
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

fn encode_utf16le_name(name: &str) -> [u8; 72] {
    let mut buf = [0u8; 72];
    for (i, code_unit) in name.encode_utf16().take(GPT_LABEL_MAX_UTF16).enumerate() {
        let bytes = code_unit.to_le_bytes();
        buf[i * 2] = bytes[0];
        buf[i * 2 + 1] = bytes[1];
    }
    buf
}

// ---------------------------------------------------------------------------
// GPT writing
// ---------------------------------------------------------------------------

fn write_protective_mbr(
    file: &mut fs::File,
    disk_size_sectors: u64,
    sector_size: u64,
) -> io::Result<()> {
    let mut mbr = [0u8; 512];

    // Protective MBR partition entry at offset 446
    // Status: 0x00 (not bootable)
    mbr[446] = 0x00;
    // CHS of first sector: 0x00, 0x02, 0x00
    mbr[447] = 0x00;
    mbr[448] = 0x02;
    mbr[449] = 0x00;
    // Type: 0xEE (GPT protective)
    mbr[450] = 0xEE;
    // CHS of last sector: 0xFF, 0xFF, 0xFF
    mbr[451] = 0xFF;
    mbr[452] = 0xFF;
    mbr[453] = 0xFF;
    // First LBA: 1
    mbr[454..458].copy_from_slice(&1u32.to_le_bytes());
    // Number of sectors (capped at 0xFFFFFFFF)
    let num_sectors = if disk_size_sectors > 0xFFFFFFFF {
        0xFFFFFFFF_u32
    } else if disk_size_sectors > 1 {
        (disk_size_sectors - 1) as u32
    } else {
        1
    };
    mbr[458..462].copy_from_slice(&num_sectors.to_le_bytes());

    // MBR signature
    mbr[510] = MBR_SIGNATURE[0];
    mbr[511] = MBR_SIGNATURE[1];

    // Write. For sector sizes > 512, pad with zeros.
    file.seek(SeekFrom::Start(0))?;
    file.write_all(&mbr)?;
    if sector_size > 512 {
        let pad = vec![0u8; (sector_size - 512) as usize];
        file.write_all(&pad)?;
    }

    Ok(())
}

fn build_partition_entry(part: &GptPartition) -> Vec<u8> {
    let mut entry = vec![0u8; GPT_ENTRY_SIZE as usize];

    let type_bytes = encode_guid(&part.type_guid);
    entry[0..16].copy_from_slice(&type_bytes);

    let unique_bytes = encode_guid(&part.unique_guid);
    entry[16..32].copy_from_slice(&unique_bytes);

    entry[32..40].copy_from_slice(&part.first_lba.to_le_bytes());
    entry[40..48].copy_from_slice(&part.last_lba.to_le_bytes());
    entry[48..56].copy_from_slice(&part.attributes.to_le_bytes());

    let name_bytes = encode_utf16le_name(&part.name);
    entry[56..128].copy_from_slice(&name_bytes);

    entry
}

fn build_gpt_header(
    disk_guid: &str,
    partitions: &[GptPartition],
    disk_size_sectors: u64,
    sector_size: u64,
    is_backup: bool,
) -> Vec<u8> {
    let num_entries = MAX_GPT_ENTRIES;
    let entry_size = GPT_ENTRY_SIZE as u32;

    // Build partition entries array
    let mut entries_data = vec![0u8; (num_entries * entry_size) as usize];
    for part in partitions {
        let idx = part.slot_index as usize;
        if idx < num_entries as usize {
            let entry = build_partition_entry(part);
            let offset = idx * entry_size as usize;
            entries_data[offset..offset + entry_size as usize].copy_from_slice(&entry);
        }
    }

    let entries_crc = crc32(&entries_data);

    // The entries area: for primary, starts at LBA 2; for backup, before the backup header
    let entries_sectors = (num_entries as u64 * entry_size as u64).div_ceil(sector_size);

    let (my_lba, alternate_lba, partition_entry_lba) = if is_backup {
        let backup_header_lba = disk_size_sectors - 1;
        let backup_entries_start = backup_header_lba - entries_sectors;
        (backup_header_lba, 1u64, backup_entries_start)
    } else {
        let primary_entries_start = 2u64;
        (1u64, disk_size_sectors - 1, primary_entries_start)
    };

    // libfdisk, which upstream repart drives, opens a GPT's usable area on its
    // 1 MiB grain rather than immediately after the entry array, so a fresh
    // label reports first-lba 2048 at a 512-byte sector size. Never advance
    // past a partition that already starts earlier, so rewriting a table laid
    // out against the bare minimum keeps describing itself correctly.
    let minimum_first_usable = 2 + entries_sectors;
    let aligned_first_usable = align_up(minimum_first_usable, GPT_FIRST_LBA_GRAIN / sector_size);
    let first_usable_lba = match partitions.iter().map(|p| p.first_lba).min() {
        Some(earliest) => aligned_first_usable.min(earliest.max(minimum_first_usable)),
        None => aligned_first_usable,
    };
    let last_usable_lba = disk_size_sectors - 1 - entries_sectors - 1;

    let mut header = vec![0u8; sector_size as usize];

    // Signature
    header[0..8].copy_from_slice(GPT_SIGNATURE);
    // Revision 1.0
    header[8..12].copy_from_slice(&GPT_REVISION_1_0.to_le_bytes());
    // Header size
    header[12..16].copy_from_slice(&(GPT_HEADER_SIZE as u32).to_le_bytes());
    // Header CRC32 (placeholder — filled in below)
    header[16..20].copy_from_slice(&0u32.to_le_bytes());
    // Reserved
    header[20..24].copy_from_slice(&0u32.to_le_bytes());
    // MyLBA
    header[24..32].copy_from_slice(&my_lba.to_le_bytes());
    // AlternateLBA
    header[32..40].copy_from_slice(&alternate_lba.to_le_bytes());
    // FirstUsableLBA
    header[40..48].copy_from_slice(&first_usable_lba.to_le_bytes());
    // LastUsableLBA
    header[48..56].copy_from_slice(&last_usable_lba.to_le_bytes());
    // DiskGUID
    let guid_bytes = encode_guid(disk_guid);
    header[56..72].copy_from_slice(&guid_bytes);
    // PartitionEntryLBA
    header[72..80].copy_from_slice(&partition_entry_lba.to_le_bytes());
    // NumberOfPartitionEntries
    header[80..84].copy_from_slice(&num_entries.to_le_bytes());
    // SizeOfPartitionEntry
    header[84..88].copy_from_slice(&entry_size.to_le_bytes());
    // PartitionEntryArrayCRC32
    header[88..92].copy_from_slice(&entries_crc.to_le_bytes());

    // Compute header CRC32
    let hdr_crc = crc32(&header[0..GPT_HEADER_SIZE]);
    header[16..20].copy_from_slice(&hdr_crc.to_le_bytes());

    // Return header + entries
    let mut result = header;
    result.extend_from_slice(&entries_data);
    result
}

fn write_gpt(
    file: &mut fs::File,
    disk_guid: &str,
    partitions: &[GptPartition],
    disk_size: u64,
    sector_size: u64,
) -> io::Result<()> {
    let disk_size_sectors = disk_size / sector_size;

    // 1. Write protective MBR
    write_protective_mbr(file, disk_size_sectors, sector_size)?;

    // 2. Build and write primary GPT header + entries (at LBA 1 + LBA 2..)
    let primary = build_gpt_header(disk_guid, partitions, disk_size_sectors, sector_size, false);
    let header_data = &primary[..sector_size as usize];
    let entries_data = &primary[sector_size as usize..];

    file.seek(SeekFrom::Start(sector_size))?;
    file.write_all(header_data)?;
    file.seek(SeekFrom::Start(2 * sector_size))?;
    file.write_all(entries_data)?;

    // 3. Build and write backup GPT (entries + header at end of disk)
    let backup = build_gpt_header(disk_guid, partitions, disk_size_sectors, sector_size, true);
    let backup_header_data = &backup[..sector_size as usize];
    let backup_entries_data = &backup[sector_size as usize..];

    let num_entries = MAX_GPT_ENTRIES;
    let entry_size = GPT_ENTRY_SIZE as u32;
    let entries_total_size = num_entries as u64 * entry_size as u64;
    let entries_sectors = entries_total_size.div_ceil(sector_size);

    let backup_entries_start = (disk_size_sectors - 1 - entries_sectors) * sector_size;
    file.seek(SeekFrom::Start(backup_entries_start))?;
    file.write_all(backup_entries_data)?;

    let backup_header_start = (disk_size_sectors - 1) * sector_size;
    file.seek(SeekFrom::Start(backup_header_start))?;
    file.write_all(backup_header_data)?;

    file.flush()?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Partition matching algorithm
// ---------------------------------------------------------------------------

#[derive(Debug, Clone)]
struct MatchedPartition {
    /// The definition this partition corresponds to.
    definition_index: usize,
    /// Existing partition (None if new).
    existing: Option<GptPartition>,
    /// Allocated size in bytes.
    allocated_size: u64,
    /// Padding after this partition in bytes.
    padding_size: u64,
    /// Assigned UUID.
    assigned_uuid: String,
    /// Assigned label.
    assigned_label: String,
    /// Whether this partition is new.
    is_new: bool,
    /// Whether this partition was grown.
    is_grown: bool,
    /// Start LBA (set during layout).
    start_lba: u64,
    /// End LBA (set during layout).
    end_lba: u64,
    /// Slot index in partition table.
    slot_index: u32,
}

/// Match existing partitions to definition files by GPT type UUID.
fn match_partitions(
    defs: &[PartitionDefinition],
    existing: &[GptPartition],
) -> Vec<MatchedPartition> {
    // Group definitions by type UUID
    let mut type_counters: HashMap<String, usize> = HashMap::new();

    // Group existing partitions by type UUID (sorted by slot_index)
    let mut existing_by_type: HashMap<String, Vec<&GptPartition>> = HashMap::new();
    for part in existing {
        existing_by_type
            .entry(part.type_guid.clone())
            .or_default()
            .push(part);
    }
    // Sort each group by slot_index
    for group in existing_by_type.values_mut() {
        group.sort_by_key(|p| p.slot_index);
    }

    let mut matched = Vec::new();

    for (i, def) in defs.iter().enumerate() {
        let counter = type_counters.entry(def.type_uuid.clone()).or_insert(0);
        let existing_group = existing_by_type.get(&def.type_uuid);

        if let Some(group) = existing_group
            && let Some(&part) = group.get(*counter)
        {
            // Matched to existing partition
            matched.push(MatchedPartition {
                definition_index: i,
                existing: Some(part.clone()),
                allocated_size: 0,
                padding_size: 0,
                assigned_uuid: part.unique_guid.clone(),
                assigned_label: if part.name.is_empty() {
                    def.effective_label()
                } else {
                    part.name.clone()
                },
                is_new: false,
                is_grown: false,
                start_lba: part.first_lba,
                end_lba: part.last_lba,
                slot_index: part.slot_index,
            });
            *counter += 1;
            continue;
        }

        // No existing match — this is a new partition
        matched.push(MatchedPartition {
            definition_index: i,
            existing: None,
            allocated_size: 0,
            padding_size: 0,
            assigned_uuid: String::new(),
            assigned_label: def.effective_label(),
            is_new: true,
            is_grown: false,
            start_lba: 0,
            end_lba: 0,
            slot_index: 0,
        });

        *counter += 1;
    }

    matched
}

// ---------------------------------------------------------------------------
// Space allocation
// ---------------------------------------------------------------------------

fn align_up(val: u64, align: u64) -> u64 {
    if align == 0 {
        return val;
    }
    val.div_ceil(align) * align
}

#[allow(dead_code)]
fn align_down(val: u64, align: u64) -> u64 {
    if align == 0 {
        return val;
    }
    val / align * align
}

/// Find free space regions on disk, given existing partitions.
fn find_free_regions(
    existing: &[GptPartition],
    first_usable_lba: u64,
    last_usable_lba: u64,
) -> Vec<(u64, u64)> {
    let mut occupied: Vec<(u64, u64)> =
        existing.iter().map(|p| (p.first_lba, p.last_lba)).collect();
    occupied.sort_by_key(|&(start, _)| start);

    let mut free = Vec::new();
    let mut cursor = first_usable_lba;

    for (start, end) in &occupied {
        if cursor < *start {
            free.push((cursor, *start - 1));
        }
        cursor = end + 1;
    }

    if cursor <= last_usable_lba {
        free.push((cursor, last_usable_lba));
    }

    free
}

/// Allocate space for new/grown partitions using weight-based distribution.
fn allocate_space(
    defs: &[PartitionDefinition],
    matched: &mut [MatchedPartition],
    first_usable_lba: u64,
    last_usable_lba: u64,
    sector_size: u64,
    existing: &[GptPartition],
    seed: &str,
) -> Result<(), String> {
    // Find the highest existing slot index
    let max_existing_slot = existing.iter().map(|p| p.slot_index).max().unwrap_or(0);
    let mut next_slot = if existing.is_empty() {
        0
    } else {
        max_existing_slot + 1
    };

    // Calculate total available space
    let free_regions = find_free_regions(existing, first_usable_lba, last_usable_lba);
    let total_free_sectors: u64 = free_regions
        .iter()
        .map(|(start, end)| end - start + 1)
        .sum();
    let total_free_bytes = total_free_sectors * sector_size;

    // Collect items that need space (new partitions + growth of existing ones)
    struct SpaceRequest {
        matched_idx: usize,
        min_bytes: u64,
        max_bytes: u64,
        weight: u32,
        padding_min: u64,
        padding_max: u64,
        padding_weight: u32,
    }

    let mut requests: Vec<SpaceRequest> = Vec::new();
    #[allow(unused_mut)]
    let mut fixed_bytes: u64 = 0; // space consumed by minimums

    for (i, m) in matched.iter().enumerate() {
        let def = &defs[m.definition_index];

        if !m.is_new {
            // Existing partition — check if it can grow
            let current_size = m.existing.as_ref().unwrap().size_bytes(sector_size);
            if current_size < def.size_max && def.weight > 0 {
                // Can grow
                let grow_min = 0; // don't require minimum growth
                let grow_max = def.size_max.saturating_sub(current_size);
                requests.push(SpaceRequest {
                    matched_idx: i,
                    min_bytes: grow_min,
                    max_bytes: grow_max,
                    weight: def.weight,
                    padding_min: def.padding_min,
                    padding_max: def.padding_max,
                    padding_weight: def.padding_weight,
                });
                fixed_bytes += grow_min + def.padding_min;
            }
            continue;
        }

        // New partition
        let min = def.size_min.max(MIN_PARTITION_SIZE);
        let max = def.size_max;
        requests.push(SpaceRequest {
            matched_idx: i,
            min_bytes: min,
            max_bytes: max,
            weight: def.weight,
            padding_min: def.padding_min,
            padding_max: def.padding_max,
            padding_weight: def.padding_weight,
        });
        fixed_bytes += min + def.padding_min;
    }

    // Check if minimums fit
    if fixed_bytes > total_free_bytes && !requests.is_empty() {
        // Try dropping partitions by priority
        let mut droppable: Vec<usize> = requests
            .iter()
            .enumerate()
            .filter(|(_, r)| {
                let def = &defs[matched[r.matched_idx].definition_index];
                def.priority > 0 && matched[r.matched_idx].is_new
            })
            .map(|(i, _)| i)
            .collect();

        // Sort by priority descending (highest priority number = lowest importance)
        droppable.sort_by(|a, b| {
            let pa = defs[matched[requests[*a].matched_idx].definition_index].priority;
            let pb = defs[matched[requests[*b].matched_idx].definition_index].priority;
            pb.cmp(&pa)
        });

        for &drop_idx in &droppable {
            let r = &requests[drop_idx];
            fixed_bytes = fixed_bytes.saturating_sub(r.min_bytes + r.padding_min);
            // Mark as dropped by zeroing weight
            // (We'll filter these out below)
        }

        if fixed_bytes > total_free_bytes {
            return Err(format!(
                "Not enough disk space: need at least {} but only {} available",
                format_size(fixed_bytes),
                format_size(total_free_bytes)
            ));
        }
    }

    // Distribute remaining space by weight. Each request contributes two
    // claims, the partition and its padding, so a capped partition hands its
    // surplus to whatever else can still grow.
    let remaining = total_free_bytes.saturating_sub(fixed_bytes);
    let mut claims: Vec<(u64, u64)> = Vec::with_capacity(requests.len() * 2);
    for req in &requests {
        let m = &matched[req.matched_idx];
        let cap = if !m.is_new && m.existing.is_some() {
            req.max_bytes
        } else {
            req.max_bytes.saturating_sub(req.min_bytes)
        };
        claims.push((req.weight as u64, cap));
        claims.push((
            req.padding_weight as u64,
            req.padding_max.saturating_sub(req.padding_min),
        ));
    }
    let shares = distribute_weighted(&claims, remaining);

    // Upstream lays partitions out on a 4096-byte grain, not on the sector
    // size, so a size that is sector-aligned but not grain-aligned is rounded
    // down to the grain.
    let grain = std::cmp::max(4096, sector_size);

    // Upstream numbers partition UUIDs per type, in definition order: the first
    // partition of a type hashes the bare type UUID and later ones append their
    // instance number. Numbering by slot instead would change every partition's
    // UUID whenever an unrelated type was added ahead of it.
    let mut type_counts: HashMap<String, u64> = HashMap::new();
    let type_instance: Vec<u64> = matched
        .iter()
        .map(|m| {
            let counter = type_counts
                .entry(defs[m.definition_index].type_uuid.clone())
                .or_insert(0);
            let instance = *counter;
            *counter += 1;
            instance
        })
        .collect();

    for (req_idx, req) in requests.iter().enumerate() {
        let m = &mut matched[req.matched_idx];
        let def = &defs[m.definition_index];

        if !m.is_new
            && let Some(existing) = &m.existing
        {
            // Growth of existing partition
            let growth = shares[req_idx * 2].min(req.max_bytes);
            let current_size = existing.size_bytes(sector_size);
            m.allocated_size = current_size + growth;
            m.is_grown = growth > 0;
        } else {
            // New partition
            let size = (req.min_bytes + shares[req_idx * 2])
                .min(req.max_bytes)
                .max(req.min_bytes);
            // Round DOWN to the sector size. The weight shares already sum to
            // exactly the free space, so rounding each partition UP would push
            // the total past the available region and leave the last partition
            // with nowhere to go ("no free region of N available"). Rounding
            // down keeps the sum within the region (min is sector-aligned, so a
            // partition never drops below its minimum) at the cost of a tiny
            // unused tail.
            let aligned_size = align_down(size, grain);
            m.allocated_size = aligned_size;

            // Assign UUID
            if let Some(ref uuid) = def.uuid {
                m.assigned_uuid = uuid.clone();
            } else {
                m.assigned_uuid =
                    derive_partition_uuid(seed, &def.type_uuid, type_instance[req.matched_idx]);
            }

            m.slot_index = next_slot;
            next_slot += 1;
        }

        // Padding
        let padding_share = shares[req_idx * 2 + 1];
        m.padding_size = (req.padding_min + padding_share)
            .min(req.padding_max)
            .max(req.padding_min);
    }

    // Now lay out new partitions in free regions
    // Sort free regions by size (smallest first) for best-fit
    let mut remaining_free = find_free_regions(existing, first_usable_lba, last_usable_lba);

    // Lay out new partitions: place each in the smallest free region that fits
    for m in matched.iter_mut() {
        if !m.is_new || m.allocated_size == 0 {
            continue;
        }

        let needed_sectors = m.allocated_size / sector_size;
        let padding_sectors = m.padding_size / sector_size;
        let total_needed = needed_sectors + padding_sectors;

        // Find smallest fitting region
        remaining_free.sort_by_key(|&(start, end)| end - start + 1);

        let mut placed = false;
        for region in remaining_free.iter_mut() {
            let region_sectors = region.1 - region.0 + 1;
            if region_sectors >= total_needed {
                m.start_lba = region.0;
                m.end_lba = region.0 + needed_sectors - 1;

                // Shrink region
                region.0 += total_needed;
                placed = true;
                break;
            }
        }

        if !placed {
            // Try without padding
            for region in remaining_free.iter_mut() {
                let region_sectors = region.1 - region.0 + 1;
                if region_sectors >= needed_sectors {
                    m.start_lba = region.0;
                    m.end_lba = region.0 + needed_sectors - 1;
                    m.padding_size = 0;
                    region.0 += needed_sectors;
                    placed = true;
                    break;
                }
            }
        }

        if !placed {
            let def = &defs[m.definition_index];
            if def.priority > 0 {
                // Drop this partition silently
                m.allocated_size = 0;
                m.is_new = false; // Mark as not-new so it's skipped
            } else {
                return Err(format!(
                    "Cannot place partition '{}': no free region of {} available",
                    def.filename,
                    format_size(m.allocated_size)
                ));
            }
        }

        // Remove empty regions
        remaining_free.retain(|&(start, end)| start <= end);
    }

    // Handle growth of existing partitions.
    // For simplicity, we only grow the last sector of the partition if there's
    // adjacent free space.
    for m in matched.iter_mut() {
        if !m.is_grown || m.existing.is_none() {
            continue;
        }
        let existing_part = m.existing.as_ref().unwrap();
        let current_end = existing_part.last_lba;
        let desired_sectors = m.allocated_size / sector_size;
        let desired_end = existing_part.first_lba + desired_sectors - 1;

        if desired_end > current_end {
            // Check if the space after current_end is free
            let mut can_grow = false;
            for region in remaining_free.iter_mut() {
                if region.0 == current_end + 1 {
                    let available = region.1 - region.0 + 1;
                    let needed = desired_end - current_end;
                    if available >= needed {
                        m.end_lba = desired_end;
                        m.start_lba = existing_part.first_lba;
                        region.0 += needed;
                        can_grow = true;
                        break;
                    }
                }
            }
            if !can_grow {
                // Can't grow — keep existing size
                m.allocated_size = existing_part.size_bytes(sector_size);
                m.is_grown = false;
                m.end_lba = existing_part.last_lba;
                m.start_lba = existing_part.first_lba;
            }
        } else {
            m.start_lba = existing_part.first_lba;
            m.end_lba = existing_part.last_lba;
            m.is_grown = false;
        }

        remaining_free.retain(|&(start, end)| start <= end);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// mkfs support
// ---------------------------------------------------------------------------

fn run_mkfs(device_path: &str, fstype: &str, label: &str, _uuid: &str) -> Result<(), String> {
    let (cmd, args): (&str, Vec<String>) = match fstype {
        "ext4" => (
            "mkfs.ext4",
            vec!["-q".into(), "-L".into(), label.into(), device_path.into()],
        ),
        "ext2" => (
            "mkfs.ext2",
            vec!["-q".into(), "-L".into(), label.into(), device_path.into()],
        ),
        "ext3" => (
            "mkfs.ext3",
            vec!["-q".into(), "-L".into(), label.into(), device_path.into()],
        ),
        "xfs" => (
            "mkfs.xfs",
            vec!["-q".into(), "-L".into(), label.into(), device_path.into()],
        ),
        "btrfs" => (
            "mkfs.btrfs",
            vec!["-q".into(), "-L".into(), label.into(), device_path.into()],
        ),
        "vfat" | "fat32" | "fat16" | "fat" => (
            "mkfs.vfat",
            vec![
                "-n".into(),
                label.chars().take(11).collect::<String>(),
                device_path.into(),
            ],
        ),
        "swap" => (
            "mkswap",
            vec!["-L".into(), label.into(), device_path.into()],
        ),
        "erofs" => (
            "mkfs.erofs",
            vec!["-L".into(), label.into(), device_path.into()],
        ),
        "squashfs" => (
            "mksquashfs",
            vec![
                "/dev/null".into(), // dummy source
                device_path.into(),
            ],
        ),
        "empty" => {
            // No filesystem — just leave it
            return Ok(());
        }
        _ => return Err(format!("Unsupported filesystem type: {fstype}")),
    };

    eprintln!("Formatting partition as {fstype}...");
    let status = std::process::Command::new(cmd)
        .args(&args)
        .status()
        .map_err(|e| format!("Failed to run {cmd}: {e}"))?;

    if !status.success() {
        return Err(format!("{cmd} failed with exit status {status}"));
    }

    Ok(())
}

/// Format newly created partitions inside an image file. The image is attached
/// to a loop device with partition scanning so each partition appears as
/// `/dev/loopNp<num>`, then mkfs is run on each. Best-effort: the partition
/// table is already written, so a formatting failure is surfaced but does not
/// undo it. Requires privileges + loop device support (available in the test VM).
fn format_new_partitions(
    device_path: &str,
    matched: &[MatchedPartition],
    defs: &[PartitionDefinition],
) -> Result<(), String> {
    use std::process::Command;

    // Collect (kernel partition number, fstype, label, def index) for partitions
    // that need a filesystem. Kernel partition nodes are numbered 1-based in GPT
    // slot order, so the node for slot S is loopNp(S+1).
    let mut to_format: Vec<(u32, String, String, usize)> = Vec::new();
    for m in matched {
        if !m.is_new || m.allocated_size == 0 {
            continue;
        }
        let def = &defs[m.definition_index];
        if let Some(ref fstype) = def.format
            && fstype != "empty"
        {
            to_format.push((
                m.slot_index + 1,
                fstype.clone(),
                m.assigned_label.clone(),
                m.definition_index,
            ));
        }
    }
    if to_format.is_empty() {
        return Ok(());
    }

    // Attach the image to a loop device with a scanned partition table.
    let out = Command::new("losetup")
        .args(["--find", "--show", "--partscan", "--nooverlap", device_path])
        .output()
        .map_err(|e| format!("Failed to run losetup: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "losetup failed: {}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    let loopdev = String::from_utf8_lossy(&out.stdout).trim().to_string();

    // Let the partition device nodes appear.
    let _ = Command::new("udevadm")
        .args(["settle", "--timeout=10"])
        .status();

    let mut format_err: Option<String> = None;
    for (partnum, fstype, label, def_idx) in &to_format {
        let part_dev = format!("{loopdev}p{partnum}");
        // Wait briefly for the node in case udev is slow.
        for _ in 0..30 {
            if std::path::Path::new(&part_dev).exists() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(100));
        }
        if let Err(e) = run_mkfs(&part_dev, fstype, label, "") {
            format_err = Some(format!("mkfs {part_dev} ({fstype}): {e}"));
            break;
        }
        // Emit user.validatefs.* xattrs on filesystems that support them (i.e.
        // not vfat/swap). Failures here are non-fatal — the filesystem is
        // already created.
        if !matches!(fstype.as_str(), "vfat" | "fat" | "fat32" | "fat16" | "swap")
            && let Err(e) = write_validatefs_xattrs(&part_dev, defs, *def_idx)
        {
            eprintln!("Warning: failed to set validatefs xattrs on {part_dev}: {e}");
        }
    }

    // Detach the loop device regardless of the formatting outcome.
    let _ = Command::new("losetup").args(["-d", &loopdev]).status();

    match format_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Mount a freshly formatted partition and write the `user.validatefs.*`
/// extended attributes to its filesystem root, mirroring upstream
/// systemd-repart's AddValidateFS= behaviour. The label / type-uuid attributes
/// gather the partition and its verity siblings (partitions sharing the same
/// VerityMatchKey=); the mount-point attribute comes from MountPoint= or the
/// partition type's well-known mount point.
fn write_validatefs_xattrs(
    part_dev: &str,
    defs: &[PartitionDefinition],
    def_idx: usize,
) -> Result<(), String> {
    use std::process::Command;
    let def = &defs[def_idx];

    // Gather this partition and its verity siblings (same non-empty
    // VerityMatchKey=), then collect their labels and type UUIDs, sorted for a
    // reproducible order.
    let mut labels: Vec<String> = Vec::new();
    let mut type_uuids: Vec<String> = Vec::new();
    let has_sibling_key = def.verity_match_key.as_deref().is_some_and(|k| !k.is_empty());
    for (i, d) in defs.iter().enumerate() {
        let is_self = i == def_idx;
        let is_sibling = has_sibling_key && d.verity_match_key == def.verity_match_key;
        if is_self || is_sibling {
            labels.push(d.effective_label());
            type_uuids.push(d.type_uuid.clone());
        }
    }
    labels.sort();
    labels.dedup();
    type_uuids.sort();
    type_uuids.dedup();

    // Mount point: explicit MountPoint= wins, else the type's default.
    let mount_point = def
        .mount_point
        .clone()
        .or_else(|| default_mount_point(&def.type_uuid).map(str::to_owned));

    // Mount the partition on a temporary directory.
    let tmp = format!("/tmp/.repart-validatefs-{}", std::process::id());
    let _ = std::fs::create_dir_all(&tmp);
    let status = Command::new("mount")
        .arg(part_dev)
        .arg(&tmp)
        .status()
        .map_err(|e| format!("mount: {e}"))?;
    if !status.success() {
        let _ = std::fs::remove_dir(&tmp);
        return Err(format!("mount {part_dev} at {tmp} failed"));
    }

    let set = |name: &str, values: &[String]| -> Result<(), String> {
        // Values are joined with NUL and NUL-terminated, matching upstream's
        // strv encoding (which setfattr cannot express via argv).
        let mut buf: Vec<u8> = Vec::new();
        for v in values {
            buf.extend_from_slice(v.as_bytes());
            buf.push(0);
        }
        setxattr_at(&tmp, name, &buf)
    };

    let mut xattr_err: Option<String> = None;
    if let Err(e) = set("user.validatefs.gpt_label", &labels) {
        xattr_err = Some(e);
    } else if let Err(e) = set("user.validatefs.gpt_type_uuid", &type_uuids) {
        xattr_err = Some(e);
    } else if let Some(mp) = mount_point
        && let Err(e) = set("user.validatefs.mount_point", &[mp])
    {
        xattr_err = Some(e);
    }

    let _ = Command::new("umount").arg(&tmp).status();
    let _ = std::fs::remove_dir(&tmp);

    match xattr_err {
        Some(e) => Err(e),
        None => Ok(()),
    }
}

/// Set an extended attribute on a path via the raw syscall (setfattr cannot
/// carry NUL-separated values through argv).
fn setxattr_at(path: &str, name: &str, value: &[u8]) -> Result<(), String> {
    let c_path = std::ffi::CString::new(path).map_err(|e| e.to_string())?;
    let c_name = std::ffi::CString::new(name).map_err(|e| e.to_string())?;
    let ret = unsafe {
        libc::setxattr(
            c_path.as_ptr(),
            c_name.as_ptr(),
            value.as_ptr().cast(),
            value.len(),
            0,
        )
    };
    if ret != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(())
}

/// The well-known mount point for a GPT partition type, used when no explicit
/// MountPoint= is set. ESP/swap/verity have none (not validatefs-mounted here).
fn default_mount_point(type_uuid: &str) -> Option<&'static str> {
    let id = type_uuid_to_identifier(type_uuid)?;
    if id.starts_with("root") && !id.contains("verity") {
        Some("/")
    } else if id.starts_with("usr") && !id.contains("verity") {
        Some("/usr")
    } else {
        match id {
            "home" => Some("/home"),
            "srv" => Some("/srv"),
            "var" => Some("/var"),
            "tmp" => Some("/var/tmp"),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------------
// CLI
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EmptyMode {
    Refuse,
    Allow,
    Require,
    Force,
    Create,
}

impl fmt::Display for EmptyMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EmptyMode::Refuse => write!(f, "refuse"),
            EmptyMode::Allow => write!(f, "allow"),
            EmptyMode::Require => write!(f, "require"),
            EmptyMode::Force => write!(f, "force"),
            EmptyMode::Create => write!(f, "create"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JsonMode {
    Off,
    Short,
    Pretty,
}

#[derive(Debug)]
struct Args {
    device: Option<String>,
    dry_run: bool,
    empty: EmptyMode,
    size: Option<String>,
    definitions: Vec<String>,
    seed: Option<String>,
    factory_reset: bool,
    can_factory_reset: bool,
    pretty: Option<bool>,
    json_mode: JsonMode,
    no_pager: bool,
    no_legend: bool,
    sector_size: u64,
    architecture: Option<String>,
    offline: Option<bool>,
    split: bool,
    root: Option<String>,
    copy_source: Option<String>,
    key_file: Option<String>,
    list_devices: bool,
    include_partitions: Vec<String>,
    exclude_partitions: Vec<String>,
    defer_partitions: Vec<String>,
}

impl Default for Args {
    fn default() -> Self {
        Self {
            device: None,
            dry_run: true,
            empty: EmptyMode::Refuse,
            size: None,
            definitions: Vec::new(),
            seed: None,
            factory_reset: false,
            can_factory_reset: false,
            pretty: None,
            json_mode: JsonMode::Off,
            no_pager: false,
            no_legend: false,
            sector_size: SECTOR_SIZE_DEFAULT,
            architecture: None,
            offline: None,
            split: false,
            root: None,
            copy_source: None,
            key_file: None,
            list_devices: false,
            include_partitions: Vec::new(),
            exclude_partitions: Vec::new(),
            defer_partitions: Vec::new(),
        }
    }
}

fn parse_args(argv: &[String]) -> Result<Args, String> {
    let mut args = Args::default();
    let mut i = 0;

    while i < argv.len() {
        let arg = &argv[i];

        if arg == "--help" || arg == "-h" {
            print_usage();
            process::exit(0);
        }

        if arg == "--version" {
            println!("systemd-repart 256 (rust-systemd)");
            process::exit(0);
        }

        if let Some(val) = arg.strip_prefix("--dry-run=") {
            args.dry_run = parse_bool(val)?;
        } else if arg == "--dry-run" {
            // Bare --dry-run means yes (be safe)
            args.dry_run = true;
        } else if let Some(val) = arg.strip_prefix("--empty=") {
            args.empty = match val {
                "refuse" => EmptyMode::Refuse,
                "allow" => EmptyMode::Allow,
                "require" => EmptyMode::Require,
                "force" => EmptyMode::Force,
                "create" => EmptyMode::Create,
                _ => return Err(format!("Invalid --empty value: {val}")),
            };
        } else if let Some(val) = arg.strip_prefix("--size=") {
            args.size = Some(val.to_string());
        } else if arg == "--size" {
            i += 1;
            args.size = Some(argv.get(i).ok_or("--size requires a value")?.clone());
        } else if let Some(val) = arg.strip_prefix("--definitions=") {
            args.definitions.push(val.to_string());
        } else if arg == "--definitions" {
            i += 1;
            args.definitions
                .push(argv.get(i).ok_or("--definitions requires a value")?.clone());
        } else if let Some(val) = arg.strip_prefix("--seed=") {
            args.seed = Some(val.to_string());
        } else if arg == "--seed" {
            i += 1;
            args.seed = Some(argv.get(i).ok_or("--seed requires a value")?.clone());
        } else if let Some(val) = arg.strip_prefix("--factory-reset=") {
            args.factory_reset = parse_bool(val)?;
        } else if arg == "--factory-reset" {
            args.factory_reset = true;
        } else if arg == "--can-factory-reset" {
            args.can_factory_reset = true;
        } else if let Some(val) = arg.strip_prefix("--pretty=") {
            args.pretty = Some(parse_bool(val)?);
        } else if arg == "--pretty" {
            args.pretty = Some(true);
        } else if let Some(val) = arg.strip_prefix("--json=") {
            args.json_mode = match val {
                "off" => JsonMode::Off,
                "short" => JsonMode::Short,
                "pretty" => JsonMode::Pretty,
                _ => return Err(format!("Invalid --json value: {val}")),
            };
        } else if arg == "--no-pager" {
            args.no_pager = true;
        } else if arg == "--no-legend" {
            args.no_legend = true;
        } else if let Some(val) = arg.strip_prefix("--sector-size=") {
            args.sector_size = val
                .parse()
                .map_err(|_| format!("Invalid --sector-size: {val}"))?;
            if !args.sector_size.is_power_of_two()
                || args.sector_size < 512
                || args.sector_size > 4096
            {
                return Err(format!(
                    "Sector size must be a power of 2 between 512 and 4096: {val}"
                ));
            }
        } else if let Some(val) = arg.strip_prefix("--architecture=") {
            args.architecture = Some(val.to_string());
        } else if arg == "--architecture" {
            i += 1;
            args.architecture = Some(
                argv.get(i)
                    .ok_or("--architecture requires a value")?
                    .clone(),
            );
        } else if let Some(val) = arg.strip_prefix("--offline=") {
            if val == "auto" {
                args.offline = None;
            } else {
                args.offline = Some(parse_bool(val)?);
            }
        } else if let Some(val) = arg.strip_prefix("--split=") {
            args.split = parse_bool(val)?;
        } else if arg == "--split" {
            args.split = true;
        } else if let Some(val) = arg.strip_prefix("--root=") {
            args.root = Some(val.to_string());
        } else if arg == "--root" {
            i += 1;
            args.root = Some(argv.get(i).ok_or("--root requires a value")?.clone());
        } else if let Some(val) = arg.strip_prefix("--copy-source=") {
            args.copy_source = Some(val.to_string());
        } else if arg == "--copy-source" || arg == "-s" {
            i += 1;
            args.copy_source = Some(argv.get(i).ok_or("--copy-source requires a value")?.clone());
        } else if let Some(val) = arg.strip_prefix("--key-file=") {
            args.key_file = Some(val.to_string());
        } else if arg == "--list-devices" {
            args.list_devices = true;
        } else if let Some(val) = arg.strip_prefix("--include-partitions=") {
            args.include_partitions
                .extend(val.split(',').map(|s| s.trim().to_string()));
        } else if let Some(val) = arg.strip_prefix("--exclude-partitions=") {
            args.exclude_partitions
                .extend(val.split(',').map(|s| s.trim().to_string()));
        } else if let Some(val) = arg.strip_prefix("--defer-partitions=") {
            args.defer_partitions
                .extend(val.split(',').map(|s| s.trim().to_string()));
        } else if let Some(val) = arg.strip_prefix("--make-ddi=") {
            // Shortcut: --make-ddi=sysext/confext/portable implies various options
            match val {
                "sysext" | "confext" | "portable" => {
                    args.empty = EmptyMode::Create;
                    if args.size.is_none() {
                        args.size = Some("auto".into());
                    }
                    if args.seed.is_none() {
                        args.seed = Some("random".into());
                    }
                }
                _ => return Err(format!("Invalid --make-ddi value: {val}")),
            }
        } else if arg == "-S" || arg == "-C" || arg == "-P" {
            args.empty = EmptyMode::Create;
            if args.size.is_none() {
                args.size = Some("auto".into());
            }
            if args.seed.is_none() {
                args.seed = Some("random".into());
            }
        } else if arg == "-" {
            // Special: "-" means size query mode (no device)
            args.device = Some("-".to_string());
        } else if arg.starts_with('-') {
            // Unknown option — ignore for forward compat
        } else {
            // Positional argument = device
            args.device = Some(arg.clone());
        }

        i += 1;
    }

    // Upstream overrides --dry-run after parsing, in this order:
    // --can-factory-reset only reports, so it implies a dry run and opens
    // everything read-only; --empty=create makes a file that did not exist a
    // moment ago, so there is nothing to protect and it implies --dry-run=no.
    // Without the second rule an --empty=create invocation with no explicit
    // --dry-run= prints its table and writes nothing.
    if args.can_factory_reset {
        args.dry_run = true;
    } else if args.empty == EmptyMode::Create {
        args.dry_run = false;
    }

    Ok(args)
}

// ---------------------------------------------------------------------------
// Output
// ---------------------------------------------------------------------------

fn print_partition_table(
    defs: &[PartitionDefinition],
    matched: &[MatchedPartition],
    sector_size: u64,
    no_legend: bool,
) {
    if !no_legend {
        println!(
            "{:<20} {:>10} {:>10} {:>10} {:>10}  {:<36}  {:<16}",
            "TYPE", "MIN", "MAX", "SIZE", "PADDING", "UUID", "LABEL"
        );
    }

    for m in matched {
        let def = &defs[m.definition_index];

        let type_name = def
            .type_id
            .as_deref()
            .or_else(|| type_uuid_to_identifier(&def.type_uuid))
            .unwrap_or("unknown");

        let status = if m.is_new {
            "new"
        } else if m.is_grown {
            "grow"
        } else {
            "existing"
        };

        let size = if m.allocated_size > 0 {
            format_size(m.allocated_size)
        } else if !m.is_new {
            let current_size = m
                .existing
                .as_ref()
                .map(|p| p.size_bytes(sector_size))
                .unwrap_or(0);
            format_size(current_size)
        } else {
            "-".to_string()
        };

        let padding = if m.padding_size > 0 {
            format_size(m.padding_size)
        } else {
            "-".to_string()
        };

        let min = if def.size_min == DEFAULT_SIZE_MIN {
            "10M".to_string()
        } else {
            format_size(def.size_min)
        };

        let max = if def.size_max == u64::MAX {
            "-".to_string()
        } else {
            format_size(def.size_max)
        };

        let uuid = if m.assigned_uuid.is_empty() {
            "-"
        } else {
            &m.assigned_uuid
        };

        println!(
            "{:<20} {:>10} {:>10} {:>10} {:>10}  {:<36}  {:<16}  ({})",
            type_name, min, max, size, padding, uuid, m.assigned_label, status
        );
    }
}

fn print_json_output(
    defs: &[PartitionDefinition],
    matched: &[MatchedPartition],
    sector_size: u64,
    pretty: bool,
) {
    let indent = if pretty { "  " } else { "" };
    let nl = if pretty { "\n" } else { "" };
    let sep = if pretty { " " } else { "" };

    print!("[{nl}");

    for (i, m) in matched.iter().enumerate() {
        let def = &defs[m.definition_index];

        let type_name = def
            .type_id
            .as_deref()
            .or_else(|| type_uuid_to_identifier(&def.type_uuid))
            .unwrap_or("unknown");

        let size = if m.allocated_size > 0 {
            m.allocated_size
        } else if let Some(ref existing) = m.existing {
            existing.size_bytes(sector_size)
        } else {
            0
        };

        print!("{indent}{{");
        print!(
            "{nl}{indent}{indent}\"type\":{sep}\"{}\",",
            json_escape(type_name)
        );
        print!(
            "{nl}{indent}{indent}\"label\":{sep}\"{}\",",
            json_escape(&m.assigned_label)
        );
        print!(
            "{nl}{indent}{indent}\"uuid\":{sep}\"{}\",",
            json_escape(&m.assigned_uuid)
        );
        print!(
            "{nl}{indent}{indent}\"file\":{sep}\"{}\",",
            json_escape(&def.filename)
        );
        print!("{nl}{indent}{indent}\"node\":{sep}\"\",");
        print!(
            "{nl}{indent}{indent}\"offset\":{sep}{},",
            m.start_lba * sector_size
        );
        print!(
            "{nl}{indent}{indent}\"old_size\":{sep}{},",
            m.existing
                .as_ref()
                .map(|p| p.size_bytes(sector_size))
                .unwrap_or(0)
        );
        print!("{nl}{indent}{indent}\"raw_size\":{sep}{size},");
        print!("{nl}{indent}{indent}\"old_padding\":{sep}0,");
        print!(
            "{nl}{indent}{indent}\"raw_padding\":{sep}{},",
            m.padding_size
        );
        print!(
            "{nl}{indent}{indent}\"activity\":{sep}\"{}\"",
            if m.is_new {
                "create"
            } else if m.is_grown {
                "resize"
            } else {
                "unchanged"
            }
        );
        print!("{nl}{indent}}}");

        if i + 1 < matched.len() {
            print!(",");
        }
        print!("{nl}");
    }

    println!("]");
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
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

fn list_block_devices() {
    let sys_block = Path::new("/sys/block");
    let entries = match fs::read_dir(sys_block) {
        Ok(e) => e,
        Err(e) => {
            eprintln!("Cannot read /sys/block: {e}");
            return;
        }
    };

    println!("{:<20} {:<40} {:>12}", "DEVICE", "NODE", "SIZE");

    for entry in entries.flatten() {
        let name = entry.file_name().to_string_lossy().to_string();
        // Skip loop, ram, and dm devices
        if name.starts_with("loop")
            || name.starts_with("ram")
            || name.starts_with("dm-")
            || name.starts_with("zram")
        {
            continue;
        }

        let dev_path = format!("/dev/{name}");
        let size_path = entry.path().join("size");
        let size = fs::read_to_string(&size_path)
            .ok()
            .and_then(|s| s.trim().parse::<u64>().ok())
            .map(|sectors| format_size(sectors * 512))
            .unwrap_or_else(|| "?".to_string());

        println!("{:<20} {:<40} {:>12}", name, dev_path, size);
    }
}

// ---------------------------------------------------------------------------
// Main logic
// ---------------------------------------------------------------------------

fn run(argv: &[String]) -> Result<i32, String> {
    let args = parse_args(argv)?;

    if args.list_devices {
        list_block_devices();
        return Ok(0);
    }

    let arch = args
        .architecture
        .as_deref()
        .unwrap_or_else(|| detect_architecture());

    // Load definitions
    let defs = if args.definitions.is_empty() {
        let dirs: Vec<&str> = DEFINITION_SEARCH_PATHS.to_vec();
        load_definitions(&dirs, arch)?
    } else {
        let mut all_defs = Vec::new();
        for dir in &args.definitions {
            all_defs.extend(load_definitions_from_dir(dir, arch)?);
        }
        all_defs
    };

    // --include-partitions=/--exclude-partitions= select by partition TYPE.
    // Both were parsed and then never consulted, so a caller's explicit filter
    // was silently ignored and the partitions it excluded were created anyway.
    let defs = filter_definitions_by_type(defs, &args, arch);

    // Having no definitions is not by itself a reason to stop. Upstream reads
    // the definitions and then carries straight on to find_root(),
    // resize_backing_fd() and context_load_partition_table(), so
    // `--empty=create --size=1G` writes an empty GPT image even when no
    // *.conf matched. Returning early here skipped the image creation
    // entirely, and the caller then failed on the missing file.
    if defs.is_empty() && args.empty != EmptyMode::Create {
        eprintln!("No partition definitions found.");
        return Ok(0);
    }

    // --can-factory-reset mode
    if args.can_factory_reset {
        let has_factory_reset = defs.iter().any(|d| d.factory_reset);
        if has_factory_reset {
            println!("Factory reset is supported.");
            return Ok(0);
        } else {
            return Ok(1);
        }
    }

    // Determine seed
    let seed = match args.seed.as_deref() {
        Some("random") => generate_random_uuid(),
        Some(s) => s.to_string(),
        None => {
            // Try to read machine-id
            let root = args.root.as_deref().unwrap_or("");
            let machine_id_path = if root.is_empty() {
                "/etc/machine-id".to_string()
            } else {
                format!("{root}/etc/machine-id")
            };
            fs::read_to_string(&machine_id_path)
                .map(|s| s.trim().to_string())
                .unwrap_or_else(|_| generate_random_uuid())
        }
    };

    // Determine device path
    let device_path = match args.device.as_deref() {
        Some("-") | Some("") => {
            // Size query mode: output minimum disk size
            let total_min: u64 = defs
                .iter()
                .map(|d| d.size_min.max(MIN_PARTITION_SIZE) + d.padding_min)
                .sum();
            // Add GPT overhead
            let entries_size = MAX_GPT_ENTRIES as u64 * GPT_ENTRY_SIZE;
            let overhead = args.sector_size * 2 // MBR + GPT header
                + entries_size * 2 // Primary + backup entries
                + args.sector_size; // Extra alignment
            let total = align_up(total_min + overhead, args.sector_size);
            println!("{total}");
            return Ok(0);
        }
        Some(p) => p.to_string(),
        None => {
            // Default: operate on root filesystem's backing device
            eprintln!("No device specified. Use --help for usage.");
            return Err("No device specified".into());
        }
    };

    let device_path_obj = PathBuf::from(&device_path);
    let sector_size = args.sector_size;

    // Determine if we're working with a file or block device
    let is_file = !device_path_obj.starts_with("/dev/")
        || (device_path_obj.exists()
            && fs::metadata(&device_path_obj)
                .map(|m| m.is_file())
                .unwrap_or(false));
    let device_exists = device_path_obj.exists();

    // Handle --empty=create
    if args.empty == EmptyMode::Create {
        if device_exists && !is_file {
            return Err("--empty=create requires a file path, not a block device".into());
        }

        // Determine size
        let file_size = match args.size.as_deref() {
            Some("auto") => {
                let total_min: u64 = defs
                    .iter()
                    .map(|d| d.size_min.max(MIN_PARTITION_SIZE) + d.padding_min)
                    .sum();
                let entries_size = MAX_GPT_ENTRIES as u64 * GPT_ENTRY_SIZE;
                // The front reservation is the whole 1 MiB grain the first
                // usable LBA is aligned to, not just the MBR, header and entry
                // array: sizing it any smaller leaves a usable area too short
                // for the partitions that were just measured.
                let overhead = GPT_FIRST_LBA_GRAIN + entries_size + sector_size;
                align_up(total_min + overhead, sector_size)
            }
            Some(s) => parse_size(s)?,
            None => return Err("--empty=create requires --size".into()),
        };

        // Create or resize the file
        let file = fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(&device_path)
            .map_err(|e| format!("Cannot create {device_path}: {e}"))?;
        file.set_len(file_size)
            .map_err(|e| format!("Cannot set file size: {e}"))?;
        drop(file);
    }

    // Open the device/image
    let mut file = fs::OpenOptions::new()
        .read(true)
        .write(!args.dry_run)
        .open(&device_path)
        .map_err(|e| format!("Cannot open {device_path}: {e}"))?;

    let file_size = file
        .seek(SeekFrom::End(0))
        .map_err(|e| format!("Cannot seek: {e}"))?;
    file.seek(SeekFrom::Start(0))
        .map_err(|e| format!("Cannot seek: {e}"))?;

    // Read existing partition table
    let existing_header = read_gpt_header(&mut file, sector_size)
        .map_err(|e| format!("Cannot read GPT header: {e}"))?;

    let (disk_guid, first_usable_lba, last_usable_lba, existing_partitions) = if let Some(ref hdr) =
        existing_header
    {
        let parts = read_gpt_partitions(&mut file, hdr, sector_size)
            .map_err(|e| format!("Cannot read partitions: {e}"))?;
        (
            hdr.disk_guid.clone(),
            hdr.first_usable_lba,
            hdr.last_usable_lba,
            parts,
        )
    } else {
        // No partition table exists
        match args.empty {
            EmptyMode::Refuse => {
                return Err(
                        "Device has no partition table. Use --empty=allow or --empty=force to create one."
                            .into(),
                    );
            }
            EmptyMode::Require | EmptyMode::Allow | EmptyMode::Force | EmptyMode::Create => {
                let disk_size_sectors = file_size / sector_size;
                let entries_sectors =
                    (MAX_GPT_ENTRIES as u64 * GPT_ENTRY_SIZE).div_ceil(sector_size);
                // libfdisk, which upstream repart drives, starts a fresh GPT's
                // usable area on its 1 MiB grain rather than immediately after
                // the entry array, so a new label reports first-lba 2048 at a
                // 512-byte sector size, not 34.
                let first_usable =
                    align_up(2 + entries_sectors, GPT_FIRST_LBA_GRAIN / sector_size);
                let last_usable = disk_size_sectors - 1 - entries_sectors - 1;
                let guid = derive_uuid(&seed, "disk-uuid");
                (guid, first_usable, last_usable, Vec::new())
            }
        }
    };

    // Check --empty=require (must not have existing table)
    if args.empty == EmptyMode::Require && existing_header.is_some() {
        return Err(
            "Device already has a partition table but --empty=require was specified".into(),
        );
    }

    // If --empty=force, discard existing partitions
    let working_partitions = if args.empty == EmptyMode::Force {
        Vec::new()
    } else {
        existing_partitions.clone()
    };

    // Factory reset: remove partitions marked FactoryReset=yes
    let working_partitions = if args.factory_reset {
        let matched_temp = match_partitions(&defs, &working_partitions);
        let factory_reset_slots: Vec<u32> = matched_temp
            .iter()
            .filter(|m| !m.is_new && defs[m.definition_index].factory_reset)
            .filter_map(|m| m.existing.as_ref().map(|p| p.slot_index))
            .collect();

        if !factory_reset_slots.is_empty() {
            eprintln!(
                "Factory reset: removing {} partition(s)",
                factory_reset_slots.len()
            );
        }

        working_partitions
            .into_iter()
            .filter(|p| !factory_reset_slots.contains(&p.slot_index))
            .collect()
    } else {
        working_partitions
    };

    // Match definitions to existing partitions
    let mut matched = match_partitions(&defs, &working_partitions);

    // Allocate space
    allocate_space(
        &defs,
        &mut matched,
        first_usable_lba,
        last_usable_lba,
        sector_size,
        &working_partitions,
        &seed,
    )?;

    // Check if anything changed
    // Writing a label onto a disk that had none is itself a change, even when
    // no partition is added: `--empty=create` on a fresh image exists to
    // produce exactly that, an empty but valid GPT.
    let creates_label = existing_header.is_none();
    let has_changes = creates_label || matched.iter().any(|m| m.is_new || m.is_grown);

    // Output
    match args.json_mode {
        JsonMode::Short => {
            print_json_output(&defs, &matched, sector_size, false);
        }
        JsonMode::Pretty => {
            print_json_output(&defs, &matched, sector_size, true);
        }
        JsonMode::Off => {
            let show_pretty = args
                .pretty
                .unwrap_or_else(|| unsafe { libc::isatty(libc::STDOUT_FILENO) != 0 });

            if show_pretty || args.dry_run {
                print_partition_table(&defs, &matched, sector_size, args.no_legend);
            }
        }
    }

    if args.dry_run {
        if has_changes {
            eprintln!("Dry run complete. Pass --dry-run=no to apply changes.");
        } else {
            eprintln!("No changes needed.");
        }
        return Ok(0);
    }

    if !has_changes {
        eprintln!("No changes needed.");
        return Ok(0);
    }

    // Build final partition list
    let mut final_partitions: Vec<GptPartition> = Vec::new();

    // Keep existing partitions that aren't being modified
    for existing_part in &working_partitions {
        let is_modified = matched.iter().any(|m| {
            !m.is_new
                && m.existing
                    .as_ref()
                    .map(|p| p.slot_index == existing_part.slot_index)
                    .unwrap_or(false)
                && m.is_grown
        });

        if !is_modified {
            final_partitions.push(existing_part.clone());
        }
    }

    // Add grown partitions with new sizes
    for m in &matched {
        if !m.is_new
            && m.is_grown
            && let Some(ref existing) = m.existing
        {
            let def = &defs[m.definition_index];
            final_partitions.push(GptPartition {
                type_guid: existing.type_guid.clone(),
                unique_guid: existing.unique_guid.clone(),
                first_lba: existing.first_lba,
                last_lba: m.end_lba,
                attributes: def.effective_flags(),
                name: m.assigned_label.clone(),
                slot_index: existing.slot_index,
            });
        }
    }

    // Add new partitions
    for m in &matched {
        if m.is_new && m.allocated_size > 0 {
            let def = &defs[m.definition_index];
            final_partitions.push(GptPartition {
                type_guid: def.type_uuid.clone(),
                unique_guid: m.assigned_uuid.clone(),
                first_lba: m.start_lba,
                last_lba: m.end_lba,
                attributes: def.effective_flags(),
                name: m.assigned_label.clone(),
                slot_index: m.slot_index,
            });
        }
    }

    // Write the partition table
    eprintln!("Writing partition table...");
    write_gpt(
        &mut file,
        &disk_guid,
        &final_partitions,
        file_size,
        sector_size,
    )
    .map_err(|e| format!("Failed to write GPT: {e}"))?;

    eprintln!("Partition table written successfully.");

    // Format new partitions inside the image via a loop device. Only for image
    // files (a loop device is attached to the whole image and partition-scanned).
    if is_file
        && let Err(e) = format_new_partitions(&device_path, &matched, &defs)
    {
        eprintln!("Warning: partition formatting failed: {e}");
    }

    Ok(0)
}

fn print_usage() {
    println!(
        "Usage: systemd-repart [OPTIONS...] [BLOCKDEVICE]

Grow and add partitions based on repart.d/*.conf definitions.

Options:
  --dry-run=BOOL              Only show what would be done (default: yes)
  --empty=MODE                How to handle empty disks:
                              refuse (default), allow, require, force, create
  --size=BYTES|auto           Size for new image file (with --empty=create)
  --definitions=PATH          Read *.conf from PATH (may be repeated)
  --seed=UUID|random          Seed for deterministic UUID generation
  --factory-reset[=BOOL]      Remove FactoryReset=yes partitions
  --can-factory-reset         Check if factory reset is possible (exit code)
  --pretty[=BOOL]             Show user-friendly table
  --json=MODE                 JSON output: off, short, pretty
  --no-pager                  Do not pipe output into a pager
  --no-legend                 Do not print column headers/footer
  --sector-size=BYTES         Sector size: 512 (default) or 4096
  --architecture=ARCH         Override architecture for type resolution
  --offline=BOOL|auto         Build image without loop devices
  --split[=BOOL]              Generate split artifacts
  --root=PATH                 Root directory for repart.d/ and machine-id
  --copy-source=PATH          Source directory for CopyFiles=
  -s PATH                     Alias for --copy-source=
  --key-file=PATH             Encryption key file for LUKS2
  --list-devices              List candidate block devices
  --include-partitions=LIST   Only operate on these partition types
  --exclude-partitions=LIST   Do not operate on these partition types
  --defer-partitions=LIST     Defer these partition types
  --make-ddi=TYPE             Generate a DDI (sysext/confext/portable)
  -S                          Shortcut for --make-ddi=sysext
  -C                          Shortcut for --make-ddi=confext
  -P                          Shortcut for --make-ddi=portable
  -h, --help                  Show this help
  --version                   Show version

If BLOCKDEVICE is '-' or empty, output minimum disk size for definitions.
If no device is specified, operate on the root filesystem's backing device."
    );
}

fn main() {
    let argv: Vec<String> = std::env::args().skip(1).collect();

    match run(&argv) {
        Ok(code) => process::exit(code),
        Err(e) => {
            eprintln!("Error: {e}");
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
    use tempfile::TempDir;

    fn args(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    // -----------------------------------------------------------------------
    // Argument parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_empty() {
        let a = parse_args(&args(&[])).unwrap();
        assert!(a.device.is_none());
        assert!(a.dry_run);
        assert_eq!(a.empty, EmptyMode::Refuse);
        assert_eq!(a.sector_size, 512);
        assert!(!a.factory_reset);
        assert!(!a.can_factory_reset);
        assert!(!a.split);
        assert_eq!(a.json_mode, JsonMode::Off);
    }

    #[test]
    fn test_parse_args_device() {
        let a = parse_args(&args(&["/dev/sda"])).unwrap();
        assert_eq!(a.device.as_deref(), Some("/dev/sda"));
    }

    #[test]
    fn test_parse_args_dry_run_no() {
        let a = parse_args(&args(&["--dry-run=no"])).unwrap();
        assert!(!a.dry_run);
    }

    #[test]
    fn test_parse_args_dry_run_yes() {
        let a = parse_args(&args(&["--dry-run=yes"])).unwrap();
        assert!(a.dry_run);
    }

    #[test]
    fn test_parse_args_dry_run_bare() {
        let a = parse_args(&args(&["--dry-run"])).unwrap();
        assert!(a.dry_run);
    }

    #[test]
    fn test_parse_args_empty_modes() {
        for (val, expected) in &[
            ("refuse", EmptyMode::Refuse),
            ("allow", EmptyMode::Allow),
            ("require", EmptyMode::Require),
            ("force", EmptyMode::Force),
            ("create", EmptyMode::Create),
        ] {
            let a = parse_args(&args(&[&format!("--empty={val}")])).unwrap();
            assert_eq!(a.empty, *expected);
        }
    }

    #[test]
    fn test_parse_args_empty_invalid() {
        assert!(parse_args(&args(&["--empty=invalid"])).is_err());
    }

    #[test]
    fn test_parse_args_size_equals() {
        let a = parse_args(&args(&["--size=1G"])).unwrap();
        assert_eq!(a.size.as_deref(), Some("1G"));
    }

    #[test]
    fn test_parse_args_size_separate() {
        let a = parse_args(&args(&["--size", "512M"])).unwrap();
        assert_eq!(a.size.as_deref(), Some("512M"));
    }

    #[test]
    fn test_parse_args_size_auto() {
        let a = parse_args(&args(&["--size=auto"])).unwrap();
        assert_eq!(a.size.as_deref(), Some("auto"));
    }

    #[test]
    fn test_parse_args_definitions_equals() {
        let a = parse_args(&args(&["--definitions=/tmp/defs"])).unwrap();
        assert_eq!(a.definitions, vec!["/tmp/defs"]);
    }

    #[test]
    fn test_parse_args_definitions_separate() {
        let a = parse_args(&args(&["--definitions", "/tmp/defs"])).unwrap();
        assert_eq!(a.definitions, vec!["/tmp/defs"]);
    }

    #[test]
    fn test_parse_args_definitions_multiple() {
        let a = parse_args(&args(&["--definitions=/a", "--definitions=/b"])).unwrap();
        assert_eq!(a.definitions, vec!["/a", "/b"]);
    }

    #[test]
    fn test_parse_args_seed_equals() {
        let a = parse_args(&args(&["--seed=random"])).unwrap();
        assert_eq!(a.seed.as_deref(), Some("random"));
    }

    #[test]
    fn test_parse_args_seed_uuid() {
        let a = parse_args(&args(&["--seed=12345678-1234-1234-1234-123456789abc"])).unwrap();
        assert_eq!(
            a.seed.as_deref(),
            Some("12345678-1234-1234-1234-123456789abc")
        );
    }

    #[test]
    fn test_parse_args_factory_reset() {
        let a = parse_args(&args(&["--factory-reset"])).unwrap();
        assert!(a.factory_reset);
    }

    #[test]
    fn test_parse_args_factory_reset_bool() {
        let a = parse_args(&args(&["--factory-reset=yes"])).unwrap();
        assert!(a.factory_reset);
        let a = parse_args(&args(&["--factory-reset=no"])).unwrap();
        assert!(!a.factory_reset);
    }

    #[test]
    fn test_parse_args_can_factory_reset() {
        let a = parse_args(&args(&["--can-factory-reset"])).unwrap();
        assert!(a.can_factory_reset);
    }

    #[test]
    fn test_parse_args_pretty() {
        let a = parse_args(&args(&["--pretty"])).unwrap();
        assert_eq!(a.pretty, Some(true));
        let a = parse_args(&args(&["--pretty=no"])).unwrap();
        assert_eq!(a.pretty, Some(false));
    }

    #[test]
    fn test_parse_args_json_modes() {
        for (val, expected) in &[
            ("off", JsonMode::Off),
            ("short", JsonMode::Short),
            ("pretty", JsonMode::Pretty),
        ] {
            let a = parse_args(&args(&[&format!("--json={val}")])).unwrap();
            assert_eq!(a.json_mode, *expected);
        }
    }

    #[test]
    fn test_parse_args_json_invalid() {
        assert!(parse_args(&args(&["--json=bad"])).is_err());
    }

    #[test]
    fn test_parse_args_no_pager() {
        let a = parse_args(&args(&["--no-pager"])).unwrap();
        assert!(a.no_pager);
    }

    #[test]
    fn test_parse_args_no_legend() {
        let a = parse_args(&args(&["--no-legend"])).unwrap();
        assert!(a.no_legend);
    }

    #[test]
    fn test_parse_args_sector_size() {
        let a = parse_args(&args(&["--sector-size=4096"])).unwrap();
        assert_eq!(a.sector_size, 4096);
    }

    #[test]
    fn test_parse_args_sector_size_invalid() {
        assert!(parse_args(&args(&["--sector-size=123"])).is_err());
        assert!(parse_args(&args(&["--sector-size=256"])).is_err());
    }

    #[test]
    fn test_parse_args_architecture_equals() {
        let a = parse_args(&args(&["--architecture=arm64"])).unwrap();
        assert_eq!(a.architecture.as_deref(), Some("arm64"));
    }

    #[test]
    fn test_parse_args_architecture_separate() {
        let a = parse_args(&args(&["--architecture", "x86-64"])).unwrap();
        assert_eq!(a.architecture.as_deref(), Some("x86-64"));
    }

    #[test]
    fn test_parse_args_offline() {
        let a = parse_args(&args(&["--offline=yes"])).unwrap();
        assert_eq!(a.offline, Some(true));
        let a = parse_args(&args(&["--offline=auto"])).unwrap();
        assert_eq!(a.offline, None);
    }

    #[test]
    fn test_parse_args_split() {
        let a = parse_args(&args(&["--split"])).unwrap();
        assert!(a.split);
        let a = parse_args(&args(&["--split=no"])).unwrap();
        assert!(!a.split);
    }

    #[test]
    fn test_parse_args_root() {
        let a = parse_args(&args(&["--root=/mnt"])).unwrap();
        assert_eq!(a.root.as_deref(), Some("/mnt"));
    }

    #[test]
    fn test_parse_args_copy_source() {
        let a = parse_args(&args(&["--copy-source=/tree"])).unwrap();
        assert_eq!(a.copy_source.as_deref(), Some("/tree"));
    }

    #[test]
    fn test_parse_args_short_copy_source() {
        let a = parse_args(&args(&["-s", "/tree"])).unwrap();
        assert_eq!(a.copy_source.as_deref(), Some("/tree"));
    }

    #[test]
    fn test_parse_args_key_file() {
        let a = parse_args(&args(&["--key-file=/path/to/key"])).unwrap();
        assert_eq!(a.key_file.as_deref(), Some("/path/to/key"));
    }

    #[test]
    fn test_parse_args_list_devices() {
        let a = parse_args(&args(&["--list-devices"])).unwrap();
        assert!(a.list_devices);
    }

    #[test]
    fn test_parse_args_include_partitions() {
        let a = parse_args(&args(&["--include-partitions=root,home"])).unwrap();
        assert_eq!(a.include_partitions, vec!["root", "home"]);
    }

    #[test]
    fn test_parse_args_exclude_partitions() {
        let a = parse_args(&args(&["--exclude-partitions=swap"])).unwrap();
        assert_eq!(a.exclude_partitions, vec!["swap"]);
    }

    #[test]
    fn test_parse_args_defer_partitions() {
        let a = parse_args(&args(&["--defer-partitions=root-verity-sig"])).unwrap();
        assert_eq!(a.defer_partitions, vec!["root-verity-sig"]);
    }

    #[test]
    fn test_parse_args_make_ddi_sysext() {
        let a = parse_args(&args(&["--make-ddi=sysext"])).unwrap();
        assert_eq!(a.empty, EmptyMode::Create);
        assert_eq!(a.size.as_deref(), Some("auto"));
        assert_eq!(a.seed.as_deref(), Some("random"));
    }

    #[test]
    fn test_parse_args_short_s() {
        let a = parse_args(&args(&["-S"])).unwrap();
        assert_eq!(a.empty, EmptyMode::Create);
    }

    #[test]
    fn test_parse_args_combined() {
        let a = parse_args(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=1G",
            "--definitions=/defs",
            "--seed=random",
            "--pretty=yes",
            "--json=off",
            "--no-legend",
            "--sector-size=4096",
            "--architecture=arm64",
            "/tmp/image.raw",
        ]))
        .unwrap();
        assert!(!a.dry_run);
        assert_eq!(a.empty, EmptyMode::Create);
        assert_eq!(a.size.as_deref(), Some("1G"));
        assert_eq!(a.definitions, vec!["/defs"]);
        assert_eq!(a.seed.as_deref(), Some("random"));
        assert_eq!(a.pretty, Some(true));
        assert_eq!(a.json_mode, JsonMode::Off);
        assert!(a.no_legend);
        assert_eq!(a.sector_size, 4096);
        assert_eq!(a.architecture.as_deref(), Some("arm64"));
        assert_eq!(a.device.as_deref(), Some("/tmp/image.raw"));
    }

    #[test]
    fn test_parse_args_unknown_option_ignored() {
        // Unknown options should be silently ignored
        let a = parse_args(&args(&["--unknown-future-flag", "/dev/sda"])).unwrap();
        assert_eq!(a.device.as_deref(), Some("/dev/sda"));
    }

    // -----------------------------------------------------------------------
    // Size parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_size_plain() {
        assert_eq!(parse_size("1024").unwrap(), 1024);
    }

    #[test]
    fn test_parse_size_k() {
        assert_eq!(parse_size("4K").unwrap(), 4096);
        assert_eq!(parse_size("4k").unwrap(), 4096);
    }

    #[test]
    fn test_parse_size_m() {
        assert_eq!(parse_size("10M").unwrap(), 10 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_g() {
        assert_eq!(parse_size("1G").unwrap(), 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_t() {
        assert_eq!(parse_size("1T").unwrap(), 1024u64 * 1024 * 1024 * 1024);
    }

    #[test]
    fn test_parse_size_p() {
        assert_eq!(
            parse_size("1P").unwrap(),
            1024u64 * 1024 * 1024 * 1024 * 1024
        );
    }

    #[test]
    fn test_parse_size_infinity() {
        assert_eq!(parse_size("infinity").unwrap(), u64::MAX);
    }

    #[test]
    fn test_parse_size_empty() {
        assert!(parse_size("").is_err());
    }

    #[test]
    fn test_parse_size_invalid() {
        assert!(parse_size("abc").is_err());
    }

    #[test]
    fn test_parse_size_whitespace() {
        assert_eq!(parse_size("  512M  ").unwrap(), 512 * 1024 * 1024);
    }

    // -----------------------------------------------------------------------
    // Format size
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_size_bytes() {
        assert_eq!(format_size(0), "0B");
        assert_eq!(format_size(512), "512B");
    }

    #[test]
    fn test_format_size_k() {
        assert_eq!(format_size(1024), "1K");
        assert_eq!(format_size(4096), "4K");
    }

    #[test]
    fn test_format_size_m() {
        assert_eq!(format_size(1024 * 1024), "1M");
        assert_eq!(format_size(10 * 1024 * 1024), "10M");
    }

    #[test]
    fn test_format_size_g() {
        assert_eq!(format_size(1024 * 1024 * 1024), "1G");
    }

    #[test]
    fn test_format_size_t() {
        assert_eq!(format_size(1024u64 * 1024 * 1024 * 1024), "1T");
    }

    #[test]
    fn test_format_size_fractional() {
        assert_eq!(format_size(1536 * 1024), "1.5M");
    }

    // -----------------------------------------------------------------------
    // GUID helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_guid_zero() {
        let data = [0u8; 16];
        assert_eq!(parse_guid(&data), ZERO_GUID);
    }

    #[test]
    fn test_is_zero_guid_true() {
        assert!(is_zero_guid(ZERO_GUID));
    }

    #[test]
    fn test_is_zero_guid_false() {
        assert!(!is_zero_guid("c12a7328-f81f-11d2-ba4b-00a0c93ec93b"));
    }

    #[test]
    fn test_encode_decode_guid_roundtrip() {
        let guid = "c12a7328-f81f-11d2-ba4b-00a0c93ec93b";
        let encoded = encode_guid(guid);
        let decoded = parse_guid(&encoded);
        assert_eq!(decoded, guid);
    }

    #[test]
    fn test_encode_decode_guid_zero() {
        let encoded = encode_guid(ZERO_GUID);
        assert_eq!(encoded, [0u8; 16]);
        let decoded = parse_guid(&encoded);
        assert_eq!(decoded, ZERO_GUID);
    }

    #[test]
    fn test_encode_guid_short() {
        let encoded = encode_guid("invalid");
        assert_eq!(encoded, [0u8; 16]);
    }

    /// home.conf in TEST-58-REPART sets a short Label= then a too-long one.
    /// The over-long value must be ignored so the short one survives, rather
    /// than truncated to 36 characters.
    #[test]
    fn test_label_too_long_is_ignored_not_truncated() {
        let tmp = TempDir::new().unwrap();
        let conf = tmp.path().join("home.conf");
        fs::write(
            &conf,
            "[Partition]\nType=home\nLabel=home-first\n\
             Label=home-always-too-long-xxxxxxxxxxxxxx-%v\nFormat=vfat\n",
        )
        .unwrap();

        let def = parse_partition_definition(&conf, "x86-64").unwrap();
        assert_eq!(def.label.as_deref(), Some("home-first"));
    }

    /// A label of exactly 36 UTF-16 units still fits and must be accepted.
    #[test]
    fn test_label_at_limit_is_accepted() {
        let tmp = TempDir::new().unwrap();
        let conf = tmp.path().join("home.conf");
        let exactly_36 = "x".repeat(36);
        fs::write(
            &conf,
            format!("[Partition]\nType=home\nLabel={exactly_36}\n"),
        )
        .unwrap();

        let def = parse_partition_definition(&conf, "x86-64").unwrap();
        assert_eq!(def.label.as_deref(), Some(exactly_36.as_str()));
    }

    /// growfs defaults on for types repart can grow, so `Type=home` alone
    /// carries GUID:59; swap cannot grow and must not.
    #[test]
    fn test_growfs_defaults_on_for_growable_types() {
        let mk = |t: &str| {
            let mut d = PartitionDefinition::default();
            d.type_uuid = partition_type_uuid(t, "x86-64").unwrap();
            d
        };
        const GROWFS: u64 = 1u64 << 59;

        assert_eq!(mk("home").effective_flags() & GROWFS, GROWFS);
        assert_eq!(mk("root").effective_flags() & GROWFS, GROWFS);
        assert_eq!(mk("swap").effective_flags() & GROWFS, 0);
        assert_eq!(mk("esp").effective_flags() & GROWFS, 0);

        // Read-only suppresses the default.
        let mut ro = mk("home");
        ro.read_only = Some(true);
        assert_eq!(ro.effective_flags() & GROWFS, 0);

        // An explicit GrowFileSystem=no still wins.
        let mut off = mk("home");
        off.grow_fs = Some(false);
        assert_eq!(off.effective_flags() & GROWFS, 0);
    }

    /// The exact split TEST-58-REPART step 2 asserts: a 1 GiB image with an
    /// unbounded home and a swap capped at 64 MiB, after 92 MiB of padding is
    /// reserved. Home must absorb everything swap cannot take.
    #[test]
    fn test_distribute_weighted_redistributes_capped_surplus() {
        let usable = (2097118u64 - 2048 + 1) * 512;
        let free = usable - 92 * 1024 * 1024;
        let swap_cap = 64 * 1024 * 1024;

        // home (unbounded), home padding (no weight), swap (capped), swap padding.
        let claims = [(1000, u64::MAX), (0, 0), (1000, swap_cap), (0, 0)];
        let shares = distribute_weighted(&claims, free);

        assert_eq!(shares[2], swap_cap, "swap takes exactly its cap");
        assert_eq!(shares[0], free - swap_cap, "home absorbs the surplus");

        // And with the 4096-byte grain applied, that is the asserted layout.
        assert_eq!(align_down(shares[0], 4096) / 512, 1775576);
        assert_eq!(shares[2] / 512, 131072);
        assert_eq!(2048 + align_down(shares[0], 4096) / 512, 1777624);
    }

    /// Without any cap in play the split stays proportional.
    #[test]
    fn test_distribute_weighted_proportional_when_uncapped() {
        let shares = distribute_weighted(&[(1000, u64::MAX), (3000, u64::MAX)], 4000);
        assert_eq!(shares, vec![1000, 3000]);
    }

    /// A zero-weight claim never takes space, and a zero cap settles at once
    /// rather than looping forever.
    #[test]
    fn test_distribute_weighted_degenerate_claims() {
        assert_eq!(distribute_weighted(&[(0, 100), (1000, 50)], 500), vec![0, 50]);
        assert_eq!(distribute_weighted(&[(1000, 0), (1000, 0)], 500), vec![0, 0]);
        assert_eq!(distribute_weighted(&[], 500), Vec::<u64>::new());
    }

    /// TEST-58-REPART passes --include-partitions=home,swap over definitions
    /// that also declare root twice, and expects only home and swap on disk.
    #[test]
    fn test_filter_definitions_include() {
        let mk = |t: &str| {
            let mut d = PartitionDefinition::default();
            d.type_uuid = partition_type_uuid(t, "x86-64").unwrap();
            d
        };
        let defs = vec![mk("home"), mk("root"), mk("root"), mk("swap")];

        let mut a = Args::default();
        a.include_partitions = vec!["home".into(), "swap".into()];
        let kept = filter_definitions_by_type(defs.clone(), &a, "x86-64");
        assert_eq!(kept.len(), 2);
        assert_eq!(kept[0].type_uuid, partition_type_uuid("home", "x86-64").unwrap());
        assert_eq!(kept[1].type_uuid, partition_type_uuid("swap", "x86-64").unwrap());

        // Exclude is the same list with the opposite sense.
        let mut b = Args::default();
        b.exclude_partitions = vec!["root".into()];
        let kept = filter_definitions_by_type(defs.clone(), &b, "x86-64");
        assert_eq!(kept.len(), 2);

        // With neither option nothing is dropped.
        let kept = filter_definitions_by_type(defs, &Args::default(), "x86-64");
        assert_eq!(kept.len(), 4);
    }

    /// A bare type UUID is accepted alongside a designator.
    #[test]
    fn test_filter_definitions_accepts_raw_uuid() {
        let mut d = PartitionDefinition::default();
        d.type_uuid = partition_type_uuid("swap", "x86-64").unwrap();

        let mut a = Args::default();
        a.include_partitions = vec![d.type_uuid.to_lowercase()];
        assert_eq!(filter_definitions_by_type(vec![d], &a, "x86-64").len(), 1);
    }

    /// The exact values TEST-58-REPART asserts for its fixed seed. These pin
    /// the whole derivation: HMAC-SHA256 keyed by the seed's raw bytes, first
    /// half of the digest, marked as v4.
    #[test]
    fn test_derive_uuid_matches_upstream() {
        let seed = "750b6cd5c4ae4012a15e7be3c29e6a47";
        assert_eq!(
            derive_uuid(seed, "disk-uuid"),
            "1D2CE291-7CCE-4F7D-BC83-FDB49AD74EBD"
        );
    }

    #[test]
    fn test_derive_partition_uuid_matches_upstream() {
        let seed = "750b6cd5c4ae4012a15e7be3c29e6a47";
        // home, first instance: the bare type UUID is hashed, with no counter.
        assert_eq!(
            derive_partition_uuid(seed, "933AC7E1-2EB4-4F13-B844-0E14E2AEF915", 0),
            "4980595D-D74A-483A-AA9E-9903879A0EE5"
        );
        // swap, first instance.
        assert_eq!(
            derive_partition_uuid(seed, "0657FD6D-A4AB-43C4-84E5-0933C84B4F4F", 0),
            "78C92DB8-3D2B-4823-B0DC-792B78F66F1E"
        );
    }

    /// A later instance of the same type appends its LE64 counter, so it must
    /// differ from the first without disturbing it.
    #[test]
    fn test_derive_partition_uuid_instances_differ() {
        let seed = "750b6cd5c4ae4012a15e7be3c29e6a47";
        let t = "933AC7E1-2EB4-4F13-B844-0E14E2AEF915";
        let first = derive_partition_uuid(seed, t, 0);
        let second = derive_partition_uuid(seed, t, 1);
        assert_ne!(first, second);
        assert_eq!(first, derive_partition_uuid(seed, t, 0));
    }

    #[test]
    fn test_uuid_bytes_roundtrip() {
        // Dashed and bare forms parse identically; a machine ID has no dashes.
        assert_eq!(
            uuid_bytes("750b6cd5-c4ae-4012-a15e-7be3c29e6a47"),
            uuid_bytes("750b6cd5c4ae4012a15e7be3c29e6a47")
        );
        assert!(uuid_bytes("too-short").is_none());
        assert!(uuid_bytes("zzzzzzzzzzzzzzzzzzzzzzzzzzzzzzzz").is_none());
    }

    #[test]
    fn test_generate_uuid_has_version_4() {
        let uuid = derive_uuid("750b6cd5c4ae4012a15e7be3c29e6a47", "name");
        // Version nibble is at position 14 (in the 3rd group, first char)
        let parts: Vec<&str> = uuid.split('-').collect();
        assert_eq!(parts.len(), 5);
        assert!(parts[2].starts_with('4'));
    }

    #[test]
    fn test_generate_random_uuid_is_uuid() {
        let uuid = generate_random_uuid();
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.chars().filter(|c| *c == '-').count(), 4);
    }

    // -----------------------------------------------------------------------
    // CRC32
    // -----------------------------------------------------------------------

    #[test]
    fn test_crc32_empty() {
        assert_eq!(crc32(&[]), 0x00000000);
    }

    #[test]
    fn test_crc32_known_value() {
        // CRC32 of "123456789" should be 0xCBF43926
        assert_eq!(crc32(b"123456789"), 0xCBF43926);
    }

    #[test]
    fn test_crc32_zero_byte() {
        let result = crc32(&[0]);
        assert_ne!(result, 0); // Non-trivial
    }

    // -----------------------------------------------------------------------
    // Partition type identifiers
    // -----------------------------------------------------------------------

    #[test]
    fn test_partition_type_uuid_esp() {
        assert_eq!(
            partition_type_uuid("esp", "x86-64").unwrap(),
            "c12a7328-f81f-11d2-ba4b-00a0c93ec93b"
        );
    }

    #[test]
    fn test_partition_type_uuid_swap() {
        assert_eq!(
            partition_type_uuid("swap", "x86-64").unwrap(),
            "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f"
        );
    }

    #[test]
    fn test_partition_type_uuid_home() {
        assert_eq!(
            partition_type_uuid("home", "x86-64").unwrap(),
            "933ac7e1-2eb4-4f13-b844-0e14e2aef915"
        );
    }

    #[test]
    fn test_partition_type_uuid_root_resolves_arch() {
        assert_eq!(
            partition_type_uuid("root", "x86-64").unwrap(),
            "4f68bce3-e8cd-4db1-96e7-fbcaf984b709"
        );
        assert_eq!(
            partition_type_uuid("root", "arm64").unwrap(),
            "b921b045-1df0-41c3-af44-4c6f280d3fae"
        );
    }

    #[test]
    fn test_partition_type_uuid_root_explicit() {
        assert_eq!(
            partition_type_uuid("root-x86-64", "arm64").unwrap(),
            "4f68bce3-e8cd-4db1-96e7-fbcaf984b709"
        );
    }

    #[test]
    fn test_partition_type_uuid_usr_resolves_arch() {
        assert_eq!(
            partition_type_uuid("usr", "x86-64").unwrap(),
            "8484680c-9521-48c6-9c11-b0720656f69e"
        );
    }

    #[test]
    fn test_partition_type_uuid_verity_resolves_arch() {
        assert_eq!(
            partition_type_uuid("root-verity", "x86-64").unwrap(),
            "2c7357ed-ebd2-46d9-aec1-23d437ec2bf5"
        );
    }

    #[test]
    fn test_partition_type_uuid_raw_uuid() {
        let raw = "12345678-1234-1234-1234-123456789abc";
        assert_eq!(partition_type_uuid(raw, "x86-64").unwrap(), raw);
    }

    #[test]
    fn test_partition_type_uuid_unknown() {
        assert!(partition_type_uuid("nonexistent", "x86-64").is_none());
    }

    #[test]
    fn test_type_uuid_to_identifier_known() {
        assert_eq!(
            type_uuid_to_identifier("c12a7328-f81f-11d2-ba4b-00a0c93ec93b"),
            Some("esp")
        );
        assert_eq!(
            type_uuid_to_identifier("0657fd6d-a4ab-43c4-84e5-0933c84b4f4f"),
            Some("swap")
        );
    }

    #[test]
    fn test_type_uuid_to_identifier_unknown() {
        assert_eq!(
            type_uuid_to_identifier("12345678-1234-1234-1234-123456789abc"),
            None
        );
    }

    // -----------------------------------------------------------------------
    // Partition definition parsing
    // -----------------------------------------------------------------------

    fn write_conf(dir: &Path, name: &str, content: &str) -> PathBuf {
        let path = dir.join(name);
        fs::write(&path, content).unwrap();
        path
    }

    #[test]
    fn test_parse_definition_minimal() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(tmp.path(), "50-root.conf", "[Partition]\nType=root\n");
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.type_uuid, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
        assert_eq!(def.type_id.as_deref(), Some("root"));
        assert_eq!(def.filename, "50-root.conf");
        assert_eq!(def.priority, 0);
        assert_eq!(def.weight, 1000);
        assert_eq!(def.size_min, DEFAULT_SIZE_MIN);
        assert_eq!(def.size_max, u64::MAX);
        assert!(!def.factory_reset);
    }

    #[test]
    fn test_parse_definition_all_fields() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "10-esp.conf",
            "[Partition]
Type=esp
Label=EFI System
UUID=12345678-1234-1234-1234-123456789abc
Priority=1
Weight=500
PaddingWeight=100
SizeMinBytes=100M
SizeMaxBytes=500M
PaddingMinBytes=1M
PaddingMaxBytes=10M
FactoryReset=yes
Flags=0x8000000000000000
NoAuto=yes
ReadOnly=no
GrowFileSystem=yes
Format=vfat
MakeDirectories=/EFI /EFI/BOOT
Encrypt=off
Verity=off
SplitName=%t
Minimize=off
",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.type_uuid, "c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
        assert_eq!(def.label.as_deref(), Some("EFI System"));
        assert_eq!(
            def.uuid.as_deref(),
            Some("12345678-1234-1234-1234-123456789abc")
        );
        assert_eq!(def.priority, 1);
        assert_eq!(def.weight, 500);
        assert_eq!(def.padding_weight, 100);
        assert_eq!(def.size_min, 100 * 1024 * 1024);
        assert_eq!(def.size_max, 500 * 1024 * 1024);
        assert_eq!(def.padding_min, 1024 * 1024);
        assert_eq!(def.padding_max, 10 * 1024 * 1024);
        assert!(def.factory_reset);
        assert_eq!(def.flags, 0x8000000000000000);
        assert_eq!(def.no_auto, Some(true));
        assert_eq!(def.read_only, Some(false));
        assert_eq!(def.grow_fs, Some(true));
        assert_eq!(def.format.as_deref(), Some("vfat"));
        assert_eq!(def.make_directories, vec!["/EFI", "/EFI/BOOT"]);
        assert!(def.encrypt.is_none());
        assert!(def.verity.is_none());
        assert_eq!(def.split_name.as_deref(), Some("%t"));
        assert!(def.minimize.is_none());
    }

    #[test]
    fn test_parse_definition_swap() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "70-swap.conf",
            "[Partition]
Type=swap
SizeMinBytes=64M
SizeMaxBytes=1G
Priority=1
Weight=333
",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.type_uuid, "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f");
        assert_eq!(def.size_min, 64 * 1024 * 1024);
        assert_eq!(def.size_max, 1024 * 1024 * 1024);
        assert_eq!(def.priority, 1);
        assert_eq!(def.weight, 333);
    }

    #[test]
    fn test_parse_definition_home() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(tmp.path(), "60-home.conf", "[Partition]\nType=home\n");
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.type_uuid, "933ac7e1-2eb4-4f13-b844-0e14e2aef915");
    }

    #[test]
    fn test_parse_definition_comments_blanks() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "# Comment line\n\n; Another comment\n[Partition]\n# inside\nType=root\n\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.type_uuid, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
    }

    #[test]
    fn test_parse_definition_empty_values_reset() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nLabel=MyRoot\nLabel=\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert!(def.label.is_none());
    }

    #[test]
    fn test_parse_definition_unknown_section_ignored() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Unknown]\nFoo=bar\n[Partition]\nType=root\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.type_uuid, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
    }

    #[test]
    fn test_parse_definition_unknown_type_fails() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-bad.conf",
            "[Partition]\nType=nonexistent-type\n",
        );
        assert!(parse_partition_definition(&path, "x86-64").is_err());
    }

    #[test]
    fn test_parse_definition_uuid_null() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nUUID=null\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.uuid.as_deref(), Some(ZERO_GUID));
    }

    #[test]
    fn test_parse_definition_weight_out_of_range() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nWeight=2000000\n",
        );
        assert!(parse_partition_definition(&path, "x86-64").is_err());
    }

    #[test]
    fn test_parse_definition_format_swap() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "70-swap.conf",
            "[Partition]\nType=swap\nFormat=swap\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.format.as_deref(), Some("swap"));
    }

    #[test]
    fn test_parse_definition_format_ext4() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "60-home.conf",
            "[Partition]\nType=home\nFormat=ext4\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.format.as_deref(), Some("ext4"));
    }

    #[test]
    fn test_parse_definition_copy_files() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nCopyFiles=/source:/target\nCopyFiles=/src2\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.copy_files, vec!["/source:/target", "/src2"]);
    }

    #[test]
    fn test_parse_definition_copy_files_reset() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nCopyFiles=/src\nCopyFiles=\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert!(def.copy_files.is_empty());
    }

    #[test]
    fn test_parse_definition_encrypt_key_file() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nEncrypt=key-file\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.encrypt.as_deref(), Some("key-file"));
    }

    #[test]
    fn test_parse_definition_verity_data() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nVerity=data\nVerityMatchKey=root\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.verity.as_deref(), Some("data"));
        assert_eq!(def.verity_match_key.as_deref(), Some("root"));
    }

    #[test]
    fn test_parse_definition_minimize_best() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nMinimize=best\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.minimize.as_deref(), Some("best"));
    }

    #[test]
    fn test_parse_definition_minimize_yes() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nMinimize=yes\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.minimize.as_deref(), Some("best"));
    }

    #[test]
    fn test_parse_definition_minimize_guess() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nMinimize=guess\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.minimize.as_deref(), Some("guess"));
    }

    #[test]
    fn test_parse_definition_minimize_invalid() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nMinimize=invalid\n",
        );
        assert!(parse_partition_definition(&path, "x86-64").is_err());
    }

    #[test]
    fn test_parse_definition_flags_hex() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nFlags=0xDEAD\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.flags, 0xDEAD);
    }

    #[test]
    fn test_parse_definition_flags_binary() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nFlags=0b1010\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.flags, 0b1010);
    }

    #[test]
    fn test_parse_definition_flags_decimal() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nFlags=42\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.flags, 42);
    }

    #[test]
    fn test_parse_definition_mount_point() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "60-home.conf",
            "[Partition]\nType=home\nMountPoint=/home\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.mount_point.as_deref(), Some("/home"));
    }

    #[test]
    fn test_parse_definition_compression() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nFormat=erofs\nCompression=lz4\nCompressionLevel=9\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.compression.as_deref(), Some("lz4"));
        assert_eq!(def.compression_level.as_deref(), Some("9"));
    }

    #[test]
    fn test_parse_definition_supplement_for() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "20-xbootldr.conf",
            "[Partition]\nType=xbootldr\nSupplementFor=10-esp\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.supplement_for.as_deref(), Some("10-esp"));
    }

    #[test]
    fn test_parse_definition_split_name_dash() {
        let tmp = TempDir::new().unwrap();
        let path = write_conf(
            tmp.path(),
            "50-root.conf",
            "[Partition]\nType=root\nSplitName=-\n",
        );
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert!(def.split_name.is_none());
    }

    // -----------------------------------------------------------------------
    // Definition loading
    // -----------------------------------------------------------------------

    #[test]
    fn test_load_definitions_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let defs = load_definitions_from_dir(tmp.path().to_str().unwrap(), "x86-64").unwrap();
        assert!(defs.is_empty());
    }

    #[test]
    fn test_load_definitions_nonexistent_dir() {
        let defs = load_definitions(&["/nonexistent/path"], "x86-64").unwrap();
        assert!(defs.is_empty());
    }

    #[test]
    fn test_load_definitions_sorted() {
        let tmp = TempDir::new().unwrap();
        write_conf(tmp.path(), "60-home.conf", "[Partition]\nType=home\n");
        write_conf(tmp.path(), "50-root.conf", "[Partition]\nType=root\n");
        write_conf(tmp.path(), "70-swap.conf", "[Partition]\nType=swap\n");

        let defs = load_definitions_from_dir(tmp.path().to_str().unwrap(), "x86-64").unwrap();
        assert_eq!(defs.len(), 3);
        assert_eq!(defs[0].filename, "50-root.conf");
        assert_eq!(defs[1].filename, "60-home.conf");
        assert_eq!(defs[2].filename, "70-swap.conf");
    }

    #[test]
    fn test_load_definitions_skips_non_conf() {
        let tmp = TempDir::new().unwrap();
        write_conf(tmp.path(), "50-root.conf", "[Partition]\nType=root\n");
        write_conf(tmp.path(), "notes.txt", "just notes");
        write_conf(tmp.path(), ".hidden.conf", "[Partition]\nType=home\n");

        let defs = load_definitions_from_dir(tmp.path().to_str().unwrap(), "x86-64").unwrap();
        // .hidden.conf does end with .conf so it will be included
        assert!(defs.iter().any(|d| d.filename == "50-root.conf"));
        assert!(!defs.iter().any(|d| d.filename == "notes.txt"));
    }

    #[test]
    fn test_load_definitions_dedup_first_wins() {
        let tmp1 = TempDir::new().unwrap();
        let tmp2 = TempDir::new().unwrap();
        write_conf(
            tmp1.path(),
            "50-root.conf",
            "[Partition]\nType=root\nLabel=First\n",
        );
        write_conf(
            tmp2.path(),
            "50-root.conf",
            "[Partition]\nType=root\nLabel=Second\n",
        );

        let defs = load_definitions(
            &[tmp1.path().to_str().unwrap(), tmp2.path().to_str().unwrap()],
            "x86-64",
        )
        .unwrap();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].label.as_deref(), Some("First"));
    }

    // -----------------------------------------------------------------------
    // Effective flags
    // -----------------------------------------------------------------------

    #[test]
    fn test_effective_flags_no_auto() {
        let def = PartitionDefinition {
            no_auto: Some(true),
            ..Default::default()
        };
        assert_eq!(def.effective_flags() & (1u64 << 63), 1u64 << 63);
    }

    #[test]
    fn test_effective_flags_read_only() {
        let def = PartitionDefinition {
            read_only: Some(true),
            ..Default::default()
        };
        assert_eq!(def.effective_flags() & (1u64 << 60), 1u64 << 60);
    }

    #[test]
    fn test_effective_flags_grow_fs() {
        let def = PartitionDefinition {
            grow_fs: Some(true),
            ..Default::default()
        };
        assert_eq!(def.effective_flags() & (1u64 << 59), 1u64 << 59);
    }

    #[test]
    fn test_effective_flags_override_base() {
        let def = PartitionDefinition {
            flags: 0xFFFFFFFFFFFFFFFF,
            no_auto: Some(false),
            ..Default::default()
        };
        assert_eq!(def.effective_flags() & (1u64 << 63), 0);
    }

    #[test]
    fn test_effective_flags_combined() {
        let def = PartitionDefinition {
            no_auto: Some(true),
            read_only: Some(true),
            grow_fs: Some(true),
            ..Default::default()
        };
        let f = def.effective_flags();
        assert_ne!(f & (1u64 << 63), 0);
        assert_ne!(f & (1u64 << 60), 0);
        assert_ne!(f & (1u64 << 59), 0);
    }

    // -----------------------------------------------------------------------
    // Effective label
    // -----------------------------------------------------------------------

    #[test]
    fn test_effective_label_explicit() {
        let def = PartitionDefinition {
            label: Some("MyLabel".into()),
            ..Default::default()
        };
        assert_eq!(def.effective_label(), "MyLabel");
    }

    #[test]
    fn test_effective_label_from_type_id() {
        let def = PartitionDefinition {
            type_id: Some("root-x86-64".into()),
            ..Default::default()
        };
        assert_eq!(def.effective_label(), "root x86 64");
    }

    #[test]
    fn test_effective_label_from_type_uuid() {
        let def = PartitionDefinition {
            type_uuid: "c12a7328-f81f-11d2-ba4b-00a0c93ec93b".into(),
            ..Default::default()
        };
        assert_eq!(def.effective_label(), "esp");
    }

    #[test]
    fn test_effective_label_fallback() {
        let def = PartitionDefinition::default();
        assert_eq!(def.effective_label(), "Linux");
    }

    // -----------------------------------------------------------------------
    // UTF-16LE name encoding
    // -----------------------------------------------------------------------

    #[test]
    fn test_encode_utf16le_name_ascii() {
        let buf = encode_utf16le_name("EFI");
        assert_eq!(buf[0], b'E');
        assert_eq!(buf[1], 0);
        assert_eq!(buf[2], b'F');
        assert_eq!(buf[3], 0);
        assert_eq!(buf[4], b'I');
        assert_eq!(buf[5], 0);
        assert_eq!(buf[6], 0);
        assert_eq!(buf[7], 0);
    }

    #[test]
    fn test_encode_utf16le_name_empty() {
        let buf = encode_utf16le_name("");
        assert_eq!(buf, [0u8; 72]);
    }

    #[test]
    fn test_encode_decode_utf16le_roundtrip() {
        let original = "Linux root";
        let encoded = encode_utf16le_name(original);
        let decoded = parse_utf16le_name(&encoded);
        assert_eq!(decoded, original);
    }

    // -----------------------------------------------------------------------
    // Partition matching
    // -----------------------------------------------------------------------

    #[test]
    fn test_match_partitions_all_new() {
        let defs = vec![
            PartitionDefinition {
                type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                ..Default::default()
            },
            PartitionDefinition {
                type_uuid: "933ac7e1-2eb4-4f13-b844-0e14e2aef915".into(),
                ..Default::default()
            },
        ];

        let matched = match_partitions(&defs, &[]);
        assert_eq!(matched.len(), 2);
        assert!(matched[0].is_new);
        assert!(matched[1].is_new);
    }

    #[test]
    fn test_match_partitions_existing_matched() {
        let defs = vec![PartitionDefinition {
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            ..Default::default()
        }];

        let existing = vec![GptPartition {
            type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            unique_guid: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
            name: "root".into(),
            slot_index: 0,
        }];

        let matched = match_partitions(&defs, &existing);
        assert_eq!(matched.len(), 1);
        assert!(!matched[0].is_new);
        assert_eq!(
            matched[0].assigned_uuid,
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa"
        );
    }

    #[test]
    fn test_match_partitions_mixed() {
        let defs = vec![
            PartitionDefinition {
                type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                ..Default::default()
            },
            PartitionDefinition {
                type_uuid: "933ac7e1-2eb4-4f13-b844-0e14e2aef915".into(),
                ..Default::default()
            },
        ];

        let existing = vec![GptPartition {
            type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            unique_guid: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
            name: "root".into(),
            slot_index: 0,
        }];

        let matched = match_partitions(&defs, &existing);
        assert_eq!(matched.len(), 2);
        assert!(!matched[0].is_new); // root matched
        assert!(matched[1].is_new); // home new
    }

    #[test]
    fn test_match_partitions_multiple_same_type() {
        let defs = vec![
            PartitionDefinition {
                filename: "50-root.conf".into(),
                type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                ..Default::default()
            },
            PartitionDefinition {
                filename: "70-root-b.conf".into(),
                type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                ..Default::default()
            },
        ];

        let existing = vec![GptPartition {
            type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            unique_guid: "cccccccc-cccc-cccc-cccc-cccccccccccc".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
            name: "root-a".into(),
            slot_index: 0,
        }];

        let matched = match_partitions(&defs, &existing);
        assert_eq!(matched.len(), 2);
        assert!(!matched[0].is_new); // First matched
        assert!(matched[1].is_new); // Second is new
    }

    // -----------------------------------------------------------------------
    // Free space regions
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_free_regions_empty_disk() {
        let free = find_free_regions(&[], 34, 2047);
        assert_eq!(free, vec![(34, 2047)]);
    }

    #[test]
    fn test_find_free_regions_one_partition() {
        let parts = vec![GptPartition {
            type_guid: "test".into(),
            unique_guid: "test".into(),
            first_lba: 100,
            last_lba: 199,
            attributes: 0,
            name: "test".into(),
            slot_index: 0,
        }];
        let free = find_free_regions(&parts, 34, 2047);
        assert_eq!(free, vec![(34, 99), (200, 2047)]);
    }

    #[test]
    fn test_find_free_regions_full_disk() {
        let parts = vec![GptPartition {
            type_guid: "test".into(),
            unique_guid: "test".into(),
            first_lba: 34,
            last_lba: 2047,
            attributes: 0,
            name: "test".into(),
            slot_index: 0,
        }];
        let free = find_free_regions(&parts, 34, 2047);
        assert!(free.is_empty());
    }

    #[test]
    fn test_find_free_regions_gap_between() {
        let parts = vec![
            GptPartition {
                type_guid: "t1".into(),
                unique_guid: "u1".into(),
                first_lba: 34,
                last_lba: 99,
                attributes: 0,
                name: "p1".into(),
                slot_index: 0,
            },
            GptPartition {
                type_guid: "t2".into(),
                unique_guid: "u2".into(),
                first_lba: 200,
                last_lba: 2047,
                attributes: 0,
                name: "p2".into(),
                slot_index: 1,
            },
        ];
        let free = find_free_regions(&parts, 34, 2047);
        assert_eq!(free, vec![(100, 199)]);
    }

    // -----------------------------------------------------------------------
    // Space allocation
    // -----------------------------------------------------------------------

    #[test]
    fn test_allocate_space_single_new_partition() {
        let defs = vec![PartitionDefinition {
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            size_min: 4096,
            size_max: u64::MAX,
            weight: 1000,
            ..Default::default()
        }];

        let mut matched = match_partitions(&defs, &[]);
        let result = allocate_space(
            &defs,
            &mut matched,
            34,   // first usable
            2047, // last usable
            512,  // sector size
            &[],  // no existing
            "test-seed",
        );
        assert!(result.is_ok());
        assert_eq!(matched.len(), 1);
        assert!(matched[0].is_new);
        assert!(matched[0].allocated_size >= 4096);
        assert!(matched[0].start_lba >= 34);
        assert!(matched[0].end_lba <= 2047);
        assert!(!matched[0].assigned_uuid.is_empty());
    }

    #[test]
    fn test_allocate_space_two_partitions_weighted() {
        let defs = vec![
            PartitionDefinition {
                type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                size_min: 4096,
                size_max: u64::MAX,
                weight: 2000,
                ..Default::default()
            },
            PartitionDefinition {
                type_uuid: "933ac7e1-2eb4-4f13-b844-0e14e2aef915".into(),
                size_min: 4096,
                size_max: u64::MAX,
                weight: 1000,
                ..Default::default()
            },
        ];

        let mut matched = match_partitions(&defs, &[]);
        let result = allocate_space(&defs, &mut matched, 34, 100000, 512, &[], "test-seed");
        assert!(result.is_ok());
        assert_eq!(matched.len(), 2);
        // First partition should get roughly 2x the space of second
        // (minus minimums). This is approximate due to rounding.
        assert!(matched[0].allocated_size > matched[1].allocated_size);
    }

    #[test]
    fn test_allocate_space_fixed_size() {
        let fixed = 1024 * 1024; // 1 MiB
        let defs = vec![PartitionDefinition {
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            size_min: fixed,
            size_max: fixed,
            weight: 1000,
            ..Default::default()
        }];

        let mut matched = match_partitions(&defs, &[]);
        let result = allocate_space(&defs, &mut matched, 34, 100000, 512, &[], "test-seed");
        assert!(result.is_ok());
        assert_eq!(matched[0].allocated_size, fixed);
    }

    #[test]
    fn test_allocate_space_uuid_from_seed() {
        let defs = vec![PartitionDefinition {
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            size_min: 4096,
            ..Default::default()
        }];

        let mut matched = match_partitions(&defs, &[]);
        allocate_space(&defs, &mut matched, 34, 2047, 512, &[], "my-seed").unwrap();

        let uuid = &matched[0].assigned_uuid;
        assert!(!uuid.is_empty());
        assert_ne!(uuid, ZERO_GUID);

        // Same seed should give same UUID
        let mut matched2 = match_partitions(&defs, &[]);
        allocate_space(&defs, &mut matched2, 34, 2047, 512, &[], "my-seed").unwrap();
        assert_eq!(matched2[0].assigned_uuid, *uuid);
    }

    #[test]
    fn test_allocate_space_explicit_uuid() {
        let defs = vec![PartitionDefinition {
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            uuid: Some("12345678-1234-1234-1234-123456789abc".into()),
            size_min: 4096,
            ..Default::default()
        }];

        let mut matched = match_partitions(&defs, &[]);
        allocate_space(&defs, &mut matched, 34, 2047, 512, &[], "seed").unwrap();
        assert_eq!(
            matched[0].assigned_uuid,
            "12345678-1234-1234-1234-123456789abc"
        );
    }

    // -----------------------------------------------------------------------
    // GPT writing and reading roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn test_write_read_gpt_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let img_path = tmp.path().join("test.img");

        let disk_size: u64 = 10 * 1024 * 1024; // 10 MiB
        let sector_size: u64 = 512;

        // Create image file
        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(disk_size).unwrap();
        }

        let disk_guid = "aabbccdd-1122-3344-5566-778899aabbcc";
        let partitions = vec![GptPartition {
            type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            unique_guid: "11111111-2222-3333-4444-555555555555".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
            name: "root".into(),
            slot_index: 0,
        }];

        // Write
        {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&img_path)
                .unwrap();
            write_gpt(&mut f, disk_guid, &partitions, disk_size, sector_size).unwrap();
        }

        // Read back
        {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, sector_size).unwrap().unwrap();
            assert_eq!(hdr.disk_guid, disk_guid);
            assert_eq!(hdr.revision, GPT_REVISION_1_0);
            assert_eq!(hdr.num_partition_entries, MAX_GPT_ENTRIES);
            assert_eq!(hdr.partition_entry_size, GPT_ENTRY_SIZE as u32);

            let parts = read_gpt_partitions(&mut f, &hdr, sector_size).unwrap();
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].type_guid, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
            assert_eq!(parts[0].unique_guid, "11111111-2222-3333-4444-555555555555");
            assert_eq!(parts[0].first_lba, 2048);
            assert_eq!(parts[0].last_lba, 4095);
            assert_eq!(parts[0].name, "root");
        }
    }

    #[test]
    fn test_write_read_gpt_multiple_partitions() {
        let tmp = TempDir::new().unwrap();
        let img_path = tmp.path().join("test2.img");

        let disk_size: u64 = 100 * 1024 * 1024; // 100 MiB
        let sector_size: u64 = 512;

        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(disk_size).unwrap();
        }

        let disk_guid = "12345678-abcd-ef01-2345-6789abcdef01";
        let partitions = vec![
            GptPartition {
                type_guid: "c12a7328-f81f-11d2-ba4b-00a0c93ec93b".into(),
                unique_guid: "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa".into(),
                first_lba: 2048,
                last_lba: 4095,
                attributes: 0,
                name: "EFI System".into(),
                slot_index: 0,
            },
            GptPartition {
                type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                unique_guid: "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb".into(),
                first_lba: 4096,
                last_lba: 102400,
                attributes: 0,
                name: "Linux root".into(),
                slot_index: 1,
            },
            GptPartition {
                type_guid: "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f".into(),
                unique_guid: "cccccccc-cccc-cccc-cccc-cccccccccccc".into(),
                first_lba: 102401,
                last_lba: 112640,
                attributes: 0,
                name: "swap".into(),
                slot_index: 2,
            },
        ];

        {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&img_path)
                .unwrap();
            write_gpt(&mut f, disk_guid, &partitions, disk_size, sector_size).unwrap();
        }

        {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, sector_size).unwrap().unwrap();
            assert_eq!(hdr.disk_guid, disk_guid);

            let parts = read_gpt_partitions(&mut f, &hdr, sector_size).unwrap();
            assert_eq!(parts.len(), 3);
            assert_eq!(parts[0].name, "EFI System");
            assert_eq!(parts[1].name, "Linux root");
            assert_eq!(parts[2].name, "swap");
            assert_eq!(parts[0].type_guid, "c12a7328-f81f-11d2-ba4b-00a0c93ec93b");
            assert_eq!(parts[1].type_guid, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
            assert_eq!(parts[2].type_guid, "0657fd6d-a4ab-43c4-84e5-0933c84b4f4f");
        }
    }

    #[test]
    fn test_write_read_gpt_with_attributes() {
        let tmp = TempDir::new().unwrap();
        let img_path = tmp.path().join("test3.img");
        let disk_size: u64 = 10 * 1024 * 1024;
        let sector_size: u64 = 512;

        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(disk_size).unwrap();
        }

        let flags: u64 = (1u64 << 63) | (1u64 << 60) | (1u64 << 59);
        let partitions = vec![GptPartition {
            type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            unique_guid: "dddddddd-dddd-dddd-dddd-dddddddddddd".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: flags,
            name: "flagged".into(),
            slot_index: 0,
        }];

        {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&img_path)
                .unwrap();
            write_gpt(&mut f, ZERO_GUID, &partitions, disk_size, sector_size).unwrap();
        }

        {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, sector_size).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, sector_size).unwrap();
            assert_eq!(parts.len(), 1);
            assert_eq!(parts[0].attributes, flags);
        }
    }

    #[test]
    fn test_write_gpt_empty_partition_table() {
        let tmp = TempDir::new().unwrap();
        let img_path = tmp.path().join("empty.img");
        let disk_size: u64 = 10 * 1024 * 1024;
        let sector_size: u64 = 512;

        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(disk_size).unwrap();
        }

        {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&img_path)
                .unwrap();
            write_gpt(&mut f, ZERO_GUID, &[], disk_size, sector_size).unwrap();
        }

        {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, sector_size).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, sector_size).unwrap();
            assert!(parts.is_empty());
        }
    }

    #[test]
    fn test_protective_mbr_written() {
        let tmp = TempDir::new().unwrap();
        let img_path = tmp.path().join("mbr.img");
        let disk_size: u64 = 10 * 1024 * 1024;
        let sector_size: u64 = 512;

        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(disk_size).unwrap();
        }

        {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&img_path)
                .unwrap();
            write_gpt(&mut f, ZERO_GUID, &[], disk_size, sector_size).unwrap();
        }

        {
            let mut f = fs::File::open(&img_path).unwrap();
            let mut mbr = [0u8; 512];
            f.read_exact(&mut mbr).unwrap();
            // Check MBR signature
            assert_eq!(mbr[510], 0x55);
            assert_eq!(mbr[511], 0xAA);
            // Check protective partition type
            assert_eq!(mbr[450], 0xEE);
            // Check first LBA is 1
            assert_eq!(
                u32::from_le_bytes([mbr[454], mbr[455], mbr[456], mbr[457]]),
                1
            );
        }
    }

    // -----------------------------------------------------------------------
    // GPT header CRC validation
    // -----------------------------------------------------------------------

    #[test]
    fn test_gpt_header_crc32_valid() {
        let tmp = TempDir::new().unwrap();
        let img_path = tmp.path().join("crc.img");
        let disk_size: u64 = 10 * 1024 * 1024;
        let sector_size: u64 = 512;

        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(disk_size).unwrap();
        }

        {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&img_path)
                .unwrap();
            write_gpt(
                &mut f,
                "aabb0011-2233-4455-6677-8899aabbccdd",
                &[],
                disk_size,
                sector_size,
            )
            .unwrap();
        }

        {
            let mut f = fs::File::open(&img_path).unwrap();
            f.seek(SeekFrom::Start(sector_size)).unwrap();
            let mut header_bytes = vec![0u8; GPT_HEADER_SIZE];
            f.read_exact(&mut header_bytes).unwrap();

            // Extract stored CRC
            let stored_crc = u32::from_le_bytes([
                header_bytes[16],
                header_bytes[17],
                header_bytes[18],
                header_bytes[19],
            ]);

            // Zero the CRC field and recompute
            header_bytes[16] = 0;
            header_bytes[17] = 0;
            header_bytes[18] = 0;
            header_bytes[19] = 0;
            let computed_crc = crc32(&header_bytes);

            assert_eq!(stored_crc, computed_crc);
        }
    }

    // -----------------------------------------------------------------------
    // Full integration: create image with definitions
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_create_image_from_definitions() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=1M\nSizeMaxBytes=4M\n",
        )
        .unwrap();
        fs::write(
            defs_dir.join("60-home.conf"),
            "[Partition]\nType=home\nSizeMinBytes=1M\n",
        )
        .unwrap();

        let img_path = tmp.path().join("system.img");
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=10M",
            "--seed=test-seed-12345",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok(), "run failed: {:?}", result.err());

        // Verify the image
        let mut f = fs::File::open(&img_path).unwrap();
        let file_size = f.seek(SeekFrom::End(0)).unwrap();
        assert_eq!(file_size, 10 * 1024 * 1024);

        f.seek(SeekFrom::Start(0)).unwrap();
        let hdr = read_gpt_header(&mut f, 512).unwrap().unwrap();
        let parts = read_gpt_partitions(&mut f, &hdr, 512).unwrap();
        assert_eq!(parts.len(), 2);

        // First should be root type
        assert_eq!(parts[0].type_guid, "4f68bce3-e8cd-4db1-96e7-fbcaf984b709");
        // Second should be home type
        assert_eq!(parts[1].type_guid, "933ac7e1-2eb4-4f13-b844-0e14e2aef915");

        // Both should have non-zero UUIDs
        assert!(!is_zero_guid(&parts[0].unique_guid));
        assert!(!is_zero_guid(&parts[1].unique_guid));
    }

    #[test]
    fn test_full_dry_run_no_changes() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let img_path = tmp.path().join("dry.img");
        // Create image first (large enough for GPT overhead + 10M default min)
        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(20 * 1024 * 1024).unwrap();
        }

        // Dry run (default) should not write anything
        let result = run(&args(&[
            "--empty=allow",
            "--seed=seed",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        // Verify no GPT header was written (file should still be all zeros at GPT offset)
        let mut f = fs::File::open(&img_path).unwrap();
        let hdr = read_gpt_header(&mut f, 512).unwrap();
        assert!(hdr.is_none()); // No GPT header
    }

    #[test]
    fn test_full_refuse_empty_disk() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();
        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let img_path = tmp.path().join("refuse.img");
        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(10 * 1024 * 1024).unwrap();
        }

        let result = run(&args(&[
            "--dry-run=no",
            "--empty=refuse",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_err());
    }

    #[test]
    fn test_full_size_query_mode() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=100M\n",
        )
        .unwrap();
        fs::write(
            defs_dir.join("60-swap.conf"),
            "[Partition]\nType=swap\nSizeMinBytes=64M\n",
        )
        .unwrap();

        let result = run(&args(&[
            &format!("--definitions={}", defs_dir.display()),
            "-",
        ]));
        assert!(result.is_ok());
    }

    #[test]
    fn test_full_can_factory_reset_yes() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nFactoryReset=yes\n",
        )
        .unwrap();

        let result = run(&args(&[
            "--can-factory-reset",
            &format!("--definitions={}", defs_dir.display()),
        ]));
        assert_eq!(result.unwrap(), 0);
    }

    #[test]
    fn test_full_can_factory_reset_no() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let result = run(&args(&[
            "--can-factory-reset",
            &format!("--definitions={}", defs_dir.display()),
        ]));
        assert_eq!(result.unwrap(), 1);
    }

    #[test]
    fn test_full_json_output() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();
        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let img_path = tmp.path().join("json.img");
        let result = run(&args(&[
            "--empty=create",
            "--size=20M",
            "--seed=random",
            "--json=short",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());
    }

    #[test]
    fn test_full_pretty_output() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();
        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let img_path = tmp.path().join("pretty.img");
        let result = run(&args(&[
            "--empty=create",
            "--size=20M",
            "--seed=random",
            "--pretty",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());
    }

    #[test]
    fn test_full_no_definitions_found() {
        let tmp = TempDir::new().unwrap();
        let empty_dir = tmp.path().join("empty");
        fs::create_dir(&empty_dir).unwrap();

        let result = run(&args(&[
            &format!("--definitions={}", empty_dir.display()),
            "/dev/null",
        ]));
        assert!(result.is_ok()); // Should succeed with "No partition definitions found"
    }

    #[test]
    fn test_full_existing_table_incremental() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        // Define root and home
        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=1M\nSizeMaxBytes=2M\n",
        )
        .unwrap();
        fs::write(
            defs_dir.join("60-home.conf"),
            "[Partition]\nType=home\nSizeMinBytes=1M\n",
        )
        .unwrap();

        let img_path = tmp.path().join("incremental.img");

        // First pass: create image with both partitions
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=10M",
            "--seed=incr-seed",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok(), "First pass failed: {:?}", result.err());

        // Read what was created
        let (root_uuid, home_uuid) = {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, 512).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, 512).unwrap();
            assert_eq!(parts.len(), 2);
            (parts[0].unique_guid.clone(), parts[1].unique_guid.clone())
        };

        // Second pass: same definitions, should detect no changes needed
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=allow",
            "--seed=incr-seed",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        // Verify UUIDs are preserved
        {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, 512).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, 512).unwrap();
            assert_eq!(parts.len(), 2);
            assert_eq!(parts[0].unique_guid, root_uuid);
            assert_eq!(parts[1].unique_guid, home_uuid);
        }
    }

    // -----------------------------------------------------------------------
    // Align helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_align_up() {
        assert_eq!(align_up(0, 512), 0);
        assert_eq!(align_up(1, 512), 512);
        assert_eq!(align_up(512, 512), 512);
        assert_eq!(align_up(513, 512), 1024);
        assert_eq!(align_up(1024, 512), 1024);
        assert_eq!(align_up(100, 0), 100);
    }

    #[test]
    fn test_align_down() {
        assert_eq!(align_down(0, 512), 0);
        assert_eq!(align_down(1, 512), 0);
        assert_eq!(align_down(512, 512), 512);
        assert_eq!(align_down(1023, 512), 512);
        assert_eq!(align_down(1024, 512), 1024);
        assert_eq!(align_down(100, 0), 100);
    }

    // -----------------------------------------------------------------------
    // JSON escaping
    // -----------------------------------------------------------------------

    #[test]
    fn test_json_escape_plain() {
        assert_eq!(json_escape("hello"), "hello");
    }

    #[test]
    fn test_json_escape_quotes() {
        assert_eq!(json_escape("say \"hello\""), "say \\\"hello\\\"");
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
    fn test_json_escape_control_char() {
        assert_eq!(json_escape("\x01"), "\\u0001");
    }

    #[test]
    fn test_json_escape_empty() {
        assert_eq!(json_escape(""), "");
    }

    // -----------------------------------------------------------------------
    // parse_bool
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_bool_true_variants() {
        assert!(parse_bool("1").unwrap());
        assert!(parse_bool("yes").unwrap());
        assert!(parse_bool("true").unwrap());
        assert!(parse_bool("on").unwrap());
        assert!(parse_bool("Yes").unwrap());
        assert!(parse_bool("TRUE").unwrap());
    }

    #[test]
    fn test_parse_bool_false_variants() {
        assert!(!parse_bool("0").unwrap());
        assert!(!parse_bool("no").unwrap());
        assert!(!parse_bool("false").unwrap());
        assert!(!parse_bool("off").unwrap());
    }

    #[test]
    fn test_parse_bool_invalid() {
        assert!(parse_bool("maybe").is_err());
        assert!(parse_bool("").is_err());
    }

    // -----------------------------------------------------------------------
    // EmptyMode display
    // -----------------------------------------------------------------------

    #[test]
    fn test_empty_mode_display() {
        assert_eq!(format!("{}", EmptyMode::Refuse), "refuse");
        assert_eq!(format!("{}", EmptyMode::Allow), "allow");
        assert_eq!(format!("{}", EmptyMode::Require), "require");
        assert_eq!(format!("{}", EmptyMode::Force), "force");
        assert_eq!(format!("{}", EmptyMode::Create), "create");
    }

    // -----------------------------------------------------------------------
    // Architecture detection
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_architecture_returns_valid() {
        let arch = detect_architecture();
        assert!(!arch.is_empty());
        // Should be one of the known architectures
        let known = [
            "x86-64", "arm64", "x86", "arm", "riscv64", "riscv32", "s390x", "ppc64-le",
        ];
        assert!(known.contains(&arch), "Unknown architecture: {arch}");
    }

    // -----------------------------------------------------------------------
    // GptPartition methods
    // -----------------------------------------------------------------------

    #[test]
    fn test_gpt_partition_size_bytes() {
        let p = GptPartition {
            type_guid: "test".into(),
            unique_guid: "test".into(),
            first_lba: 100,
            last_lba: 199,
            attributes: 0,
            name: "test".into(),
            slot_index: 0,
        };
        assert_eq!(p.size_bytes(512), 100 * 512);
        assert_eq!(p.size_bytes(4096), 100 * 4096);
    }

    #[test]
    fn test_gpt_partition_size_sectors() {
        let p = GptPartition {
            type_guid: "test".into(),
            unique_guid: "test".into(),
            first_lba: 100,
            last_lba: 199,
            attributes: 0,
            name: "test".into(),
            slot_index: 0,
        };
        assert_eq!(p.size_sectors(), 100);
    }

    #[test]
    fn test_gpt_partition_size_single_sector() {
        let p = GptPartition {
            type_guid: "test".into(),
            unique_guid: "test".into(),
            first_lba: 100,
            last_lba: 100,
            attributes: 0,
            name: "test".into(),
            slot_index: 0,
        };
        assert_eq!(p.size_sectors(), 1);
        assert_eq!(p.size_bytes(512), 512);
    }

    // -----------------------------------------------------------------------
    // Default values
    // -----------------------------------------------------------------------

    #[test]
    fn test_partition_definition_defaults() {
        let def = PartitionDefinition::default();
        assert_eq!(def.type_uuid, ZERO_GUID);
        assert!(def.label.is_none());
        assert!(def.uuid.is_none());
        assert_eq!(def.priority, 0);
        assert_eq!(def.weight, 1000);
        assert_eq!(def.padding_weight, 0);
        assert_eq!(def.size_min, DEFAULT_SIZE_MIN);
        assert_eq!(def.size_max, u64::MAX);
        assert_eq!(def.padding_min, 0);
        assert_eq!(def.padding_max, u64::MAX);
        assert!(!def.factory_reset);
        assert_eq!(def.flags, 0);
        assert!(def.no_auto.is_none());
        assert!(def.read_only.is_none());
        assert!(def.grow_fs.is_none());
        assert!(def.format.is_none());
        assert!(def.copy_files.is_empty());
        assert!(def.copy_blocks.is_none());
        assert!(def.make_directories.is_empty());
        assert!(def.encrypt.is_none());
        assert!(def.verity.is_none());
        assert!(def.verity_match_key.is_none());
        assert!(def.split_name.is_none());
        assert!(def.minimize.is_none());
    }

    #[test]
    fn test_args_defaults() {
        let a = Args::default();
        assert!(a.device.is_none());
        assert!(a.dry_run);
        assert_eq!(a.empty, EmptyMode::Refuse);
        assert!(a.size.is_none());
        assert!(a.definitions.is_empty());
        assert!(a.seed.is_none());
        assert!(!a.factory_reset);
        assert!(!a.can_factory_reset);
        assert!(a.pretty.is_none());
        assert_eq!(a.json_mode, JsonMode::Off);
        assert!(!a.no_pager);
        assert!(!a.no_legend);
        assert_eq!(a.sector_size, 512);
        assert!(a.architecture.is_none());
        assert!(a.offline.is_none());
        assert!(!a.split);
        assert!(a.root.is_none());
        assert!(a.copy_source.is_none());
        assert!(a.key_file.is_none());
        assert!(!a.list_devices);
    }

    // -----------------------------------------------------------------------
    // Constants
    // -----------------------------------------------------------------------

    #[test]
    fn test_gpt_signature() {
        assert_eq!(GPT_SIGNATURE, b"EFI PART");
    }

    #[test]
    fn test_gpt_revision() {
        assert_eq!(GPT_REVISION_1_0, 0x00010000);
    }

    #[test]
    fn test_max_gpt_entries() {
        assert_eq!(MAX_GPT_ENTRIES, 128);
    }

    #[test]
    fn test_gpt_entry_size() {
        assert_eq!(GPT_ENTRY_SIZE, 128);
    }

    #[test]
    fn test_sector_size_default() {
        assert_eq!(SECTOR_SIZE_DEFAULT, 512);
    }

    #[test]
    fn test_min_partition_size() {
        assert_eq!(MIN_PARTITION_SIZE, 4096);
    }

    #[test]
    fn test_default_size_min() {
        assert_eq!(DEFAULT_SIZE_MIN, 10 * 1024 * 1024);
    }

    #[test]
    fn test_default_weight_value() {
        assert_eq!(DEFAULT_WEIGHT, 1000);
    }

    // -----------------------------------------------------------------------
    // resolve_arch_type direct tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_resolve_arch_type_root() {
        assert_eq!(
            resolve_arch_type("root", "x86-64"),
            Some("root-x86-64".into())
        );
    }

    #[test]
    fn test_resolve_arch_type_root_verity() {
        assert_eq!(
            resolve_arch_type("root-verity", "arm64"),
            Some("root-arm64-verity".into())
        );
    }

    #[test]
    fn test_resolve_arch_type_root_verity_sig() {
        assert_eq!(
            resolve_arch_type("root-verity-sig", "x86-64"),
            Some("root-x86-64-verity-sig".into())
        );
    }

    #[test]
    fn test_resolve_arch_type_usr() {
        assert_eq!(
            resolve_arch_type("usr", "riscv64"),
            Some("usr-riscv64".into())
        );
    }

    #[test]
    fn test_resolve_arch_type_usr_verity() {
        assert_eq!(
            resolve_arch_type("usr-verity", "arm64"),
            Some("usr-arm64-verity".into())
        );
    }

    #[test]
    fn test_resolve_arch_type_usr_verity_sig() {
        assert_eq!(
            resolve_arch_type("usr-verity-sig", "x86-64"),
            Some("usr-x86-64-verity-sig".into())
        );
    }

    #[test]
    fn test_resolve_arch_type_non_arch() {
        assert_eq!(resolve_arch_type("esp", "x86-64"), None);
        assert_eq!(resolve_arch_type("swap", "arm64"), None);
        assert_eq!(resolve_arch_type("home", "x86-64"), None);
    }

    // -----------------------------------------------------------------------
    // format_size edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_size_zero() {
        assert_eq!(format_size(0), "0B");
    }

    #[test]
    fn test_format_size_one_byte() {
        assert_eq!(format_size(1), "1B");
    }

    #[test]
    fn test_format_size_just_below_k() {
        assert_eq!(format_size(1023), "1023B");
    }

    #[test]
    fn test_format_size_exact_k() {
        assert_eq!(format_size(1024), "1K");
    }

    #[test]
    fn test_format_size_exact_t() {
        assert_eq!(format_size(1024 * 1024 * 1024 * 1024), "1T");
    }

    // -----------------------------------------------------------------------
    // parse_size E suffix
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_size_e_suffix() {
        let expected = 1024u64 * 1024 * 1024 * 1024 * 1024 * 1024;
        assert_eq!(parse_size("1E").unwrap(), expected);
    }

    #[test]
    fn test_parse_size_e_lowercase() {
        let expected = 2 * 1024u64 * 1024 * 1024 * 1024 * 1024 * 1024;
        assert_eq!(parse_size("2e").unwrap(), expected);
    }

    // -----------------------------------------------------------------------
    // parse_guid / encode_guid edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_guid_short_data() {
        assert_eq!(parse_guid(&[0u8; 4]), ZERO_GUID);
        assert_eq!(parse_guid(&[]), ZERO_GUID);
    }

    #[test]
    fn test_parse_guid_exact_16_bytes() {
        let data = [1u8, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];
        let guid = parse_guid(&data);
        assert_eq!(guid.len(), 36);
        assert_eq!(guid.chars().filter(|c| *c == '-').count(), 4);
    }

    #[test]
    fn test_encode_guid_invalid_length() {
        assert_eq!(encode_guid("short"), [0u8; 16]);
        assert_eq!(encode_guid(""), [0u8; 16]);
    }

    #[test]
    fn test_encode_guid_invalid_hex_chars() {
        // 36 chars with dashes but contains non-hex — should produce zeros for bad nibbles
        let bad = "zzzzzzzz-zzzz-zzzz-zzzz-zzzzzzzzzzzz";
        let result = encode_guid(bad);
        assert_eq!(result, [0u8; 16]);
    }

    // -----------------------------------------------------------------------
    // find_free_regions complex scenarios
    // -----------------------------------------------------------------------

    #[test]
    fn test_find_free_regions_multiple_gaps() {
        let existing = vec![
            GptPartition {
                type_guid: "t".into(),
                unique_guid: "u".into(),
                first_lba: 100,
                last_lba: 199,
                attributes: 0,
                name: String::new(),
                slot_index: 0,
            },
            GptPartition {
                type_guid: "t".into(),
                unique_guid: "u".into(),
                first_lba: 300,
                last_lba: 399,
                attributes: 0,
                name: String::new(),
                slot_index: 1,
            },
        ];
        let free = find_free_regions(&existing, 34, 500);
        // Should have three free regions: [34..99], [200..299], [400..500]
        assert_eq!(free.len(), 3);
        assert_eq!(free[0], (34, 99));
        assert_eq!(free[1], (200, 299));
        assert_eq!(free[2], (400, 500));
    }

    #[test]
    fn test_find_free_regions_partition_at_start() {
        let existing = vec![GptPartition {
            type_guid: "t".into(),
            unique_guid: "u".into(),
            first_lba: 34,
            last_lba: 100,
            attributes: 0,
            name: String::new(),
            slot_index: 0,
        }];
        let free = find_free_regions(&existing, 34, 500);
        assert_eq!(free.len(), 1);
        assert_eq!(free[0], (101, 500));
    }

    #[test]
    fn test_find_free_regions_partition_at_end() {
        let existing = vec![GptPartition {
            type_guid: "t".into(),
            unique_guid: "u".into(),
            first_lba: 400,
            last_lba: 500,
            attributes: 0,
            name: String::new(),
            slot_index: 0,
        }];
        let free = find_free_regions(&existing, 34, 500);
        assert_eq!(free.len(), 1);
        assert_eq!(free[0], (34, 399));
    }

    // -----------------------------------------------------------------------
    // allocate_space with padding
    // -----------------------------------------------------------------------

    #[test]
    fn test_allocate_space_with_padding() {
        let defs = vec![PartitionDefinition {
            filename: "50-root.conf".into(),
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            size_min: 1024 * 1024,
            padding_min: 512 * 1024,
            padding_weight: 100,
            ..Default::default()
        }];

        let mut matched = vec![MatchedPartition {
            definition_index: 0,
            existing: None,
            allocated_size: 0,
            padding_size: 0,
            assigned_uuid: String::new(),
            assigned_label: "root".into(),
            is_new: true,
            is_grown: false,
            start_lba: 0,
            end_lba: 0,
            slot_index: 0,
        }];

        let first_usable = 34;
        let last_usable = 20479; // ~10M disk
        let sector_size = 512;

        let result = allocate_space(
            &defs,
            &mut matched,
            first_usable,
            last_usable,
            sector_size,
            &[],
            "test-seed",
        );
        assert!(result.is_ok(), "allocate_space failed: {:?}", result.err());
        assert!(matched[0].allocated_size >= 1024 * 1024);
        assert!(matched[0].padding_size >= 512 * 1024);
    }

    // -----------------------------------------------------------------------
    // allocate_space not enough space
    // -----------------------------------------------------------------------

    #[test]
    fn test_allocate_space_insufficient_space() {
        let defs = vec![PartitionDefinition {
            filename: "50-root.conf".into(),
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            size_min: 100 * 1024 * 1024, // 100M min
            ..Default::default()
        }];

        let mut matched = vec![MatchedPartition {
            definition_index: 0,
            existing: None,
            allocated_size: 0,
            padding_size: 0,
            assigned_uuid: String::new(),
            assigned_label: "root".into(),
            is_new: true,
            is_grown: false,
            start_lba: 0,
            end_lba: 0,
            slot_index: 0,
        }];

        // Tiny disk: ~50 sectors
        let result = allocate_space(&defs, &mut matched, 34, 84, 512, &[], "seed");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // allocate_space priority dropping
    // -----------------------------------------------------------------------

    #[test]
    fn test_allocate_space_drops_low_priority() {
        let defs = vec![
            PartitionDefinition {
                filename: "50-root.conf".into(),
                type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                size_min: 1024 * 1024,
                size_max: 2 * 1024 * 1024,
                priority: 0, // Cannot be dropped
                ..Default::default()
            },
            PartitionDefinition {
                filename: "60-optional.conf".into(),
                type_uuid: "933ac7e1-2eb4-4f13-b844-0e14e2aef915".into(),
                size_min: 100 * 1024 * 1024, // 100M — won't fit
                priority: 10,                // Can be dropped
                ..Default::default()
            },
        ];

        let mut matched = vec![
            MatchedPartition {
                definition_index: 0,
                existing: None,
                allocated_size: 0,
                padding_size: 0,
                assigned_uuid: String::new(),
                assigned_label: "root".into(),
                is_new: true,
                is_grown: false,
                start_lba: 0,
                end_lba: 0,
                slot_index: 0,
            },
            MatchedPartition {
                definition_index: 1,
                existing: None,
                allocated_size: 0,
                padding_size: 0,
                assigned_uuid: String::new(),
                assigned_label: "optional".into(),
                is_new: true,
                is_grown: false,
                start_lba: 0,
                end_lba: 0,
                slot_index: 0,
            },
        ];

        // 5M disk: enough for root but not for optional 100M partition
        let last_usable = 10239;
        let result = allocate_space(&defs, &mut matched, 34, last_usable, 512, &[], "seed");
        assert!(result.is_ok(), "allocate_space failed: {:?}", result.err());
        // Root should be allocated
        assert!(matched[0].allocated_size >= 1024 * 1024);
    }

    // -----------------------------------------------------------------------
    // run with --empty=force clears existing table
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_empty_force_clears_table() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=1M\nSizeMaxBytes=2M\n",
        )
        .unwrap();

        let img_path = tmp.path().join("force.img");

        // First: create image with one partition
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=10M",
            "--seed=force-test",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        let uuid_before = {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, 512).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, 512).unwrap();
            assert_eq!(parts.len(), 1);
            parts[0].unique_guid.clone()
        };

        // Second: force re-create — should get fresh UUIDs
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=force",
            "--seed=force-test-2",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        let uuid_after = {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, 512).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, 512).unwrap();
            assert_eq!(parts.len(), 1);
            parts[0].unique_guid.clone()
        };

        // UUIDs should differ because seed changed and force discarded the old table
        assert_ne!(uuid_before, uuid_after);
    }

    // -----------------------------------------------------------------------
    // run with --empty=require on existing table should fail
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_empty_require_rejects_existing() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=1M\n",
        )
        .unwrap();

        let img_path = tmp.path().join("require.img");

        // Create image with partition table
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=10M",
            "--seed=require-test",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        // Now try --empty=require — should fail because table exists
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=require",
            "--seed=require-test",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run with factory reset removes marked partitions
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_factory_reset_removes_partition() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        // Root partition: not factory-resettable
        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=1M\nSizeMaxBytes=2M\n",
        )
        .unwrap();
        // Home partition: marked for factory reset
        fs::write(
            defs_dir.join("60-home.conf"),
            "[Partition]\nType=home\nSizeMinBytes=1M\nSizeMaxBytes=2M\nFactoryReset=yes\n",
        )
        .unwrap();

        let img_path = tmp.path().join("factory.img");

        // Create image with both partitions
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=10M",
            "--seed=factory-test",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, 512).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, 512).unwrap();
            assert_eq!(parts.len(), 2);
        }

        // Run with --factory-reset: should remove home, then re-create it
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=allow",
            "--factory-reset",
            "--seed=factory-test",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        {
            let mut f = fs::File::open(&img_path).unwrap();
            let hdr = read_gpt_header(&mut f, 512).unwrap().unwrap();
            let parts = read_gpt_partitions(&mut f, &hdr, 512).unwrap();
            // Root should still exist, home was removed and re-created
            assert_eq!(parts.len(), 2);
        }
    }

    // -----------------------------------------------------------------------
    // Backup GPT header verification
    // -----------------------------------------------------------------------

    #[test]
    fn test_backup_gpt_header_valid() {
        let tmp = TempDir::new().unwrap();
        let img_path = tmp.path().join("backup.img");
        let sector_size: u64 = 512;
        let disk_size: u64 = 10 * 1024 * 1024;
        let disk_sectors = disk_size / sector_size;

        {
            let f = fs::File::create(&img_path).unwrap();
            f.set_len(disk_size).unwrap();
        }

        {
            let mut f = fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&img_path)
                .unwrap();

            let guid = "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee";
            let parts = vec![GptPartition {
                type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
                unique_guid: "11111111-2222-3333-4444-555555555555".into(),
                first_lba: 2048,
                last_lba: 4095,
                attributes: 0,
                name: "root".into(),
                slot_index: 0,
            }];

            write_gpt(&mut f, guid, &parts, disk_size, sector_size).unwrap();
        }

        // Read and verify backup GPT header at end of disk
        let mut f = fs::File::open(&img_path).unwrap();

        // Backup header is at the last sector
        let backup_offset = (disk_sectors - 1) * sector_size;
        f.seek(SeekFrom::Start(backup_offset)).unwrap();
        let mut buf = vec![0u8; sector_size as usize];
        f.read_exact(&mut buf).unwrap();

        // Verify signature
        assert_eq!(&buf[0..8], GPT_SIGNATURE);
        // Verify revision
        let rev = u32::from_le_bytes([buf[8], buf[9], buf[10], buf[11]]);
        assert_eq!(rev, GPT_REVISION_1_0);
        // MyLBA should be the last sector
        let my_lba = u64::from_le_bytes(buf[24..32].try_into().unwrap());
        assert_eq!(my_lba, disk_sectors - 1);
        // AlternateLBA should point to primary header at LBA 1
        let alt_lba = u64::from_le_bytes(buf[32..40].try_into().unwrap());
        assert_eq!(alt_lba, 1);
        // Verify header CRC32 is valid
        let stored_crc = u32::from_le_bytes([buf[16], buf[17], buf[18], buf[19]]);
        let mut check_buf = buf[0..GPT_HEADER_SIZE].to_vec();
        check_buf[16..20].copy_from_slice(&0u32.to_le_bytes());
        let computed_crc = crc32(&check_buf);
        assert_eq!(stored_crc, computed_crc);
    }

    // -----------------------------------------------------------------------
    // build_partition_entry direct test
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_partition_entry_layout() {
        let part = GptPartition {
            type_guid: "c12a7328-f81f-11d2-ba4b-00a0c93ec93b".into(),
            unique_guid: "11111111-2222-3333-4444-555555555555".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0x8000000000000001,
            name: "EFI".into(),
            slot_index: 0,
        };

        let entry = build_partition_entry(&part);
        assert_eq!(entry.len(), GPT_ENTRY_SIZE as usize);

        // Check type GUID is encoded at offset 0
        let type_decoded = parse_guid(&entry[0..16]);
        assert_eq!(type_decoded, "c12a7328-f81f-11d2-ba4b-00a0c93ec93b");

        // Check unique GUID at offset 16
        let unique_decoded = parse_guid(&entry[16..32]);
        assert_eq!(unique_decoded, "11111111-2222-3333-4444-555555555555");

        // Check first LBA at offset 32
        let first = u64::from_le_bytes(entry[32..40].try_into().unwrap());
        assert_eq!(first, 2048);

        // Check last LBA at offset 40
        let last = u64::from_le_bytes(entry[40..48].try_into().unwrap());
        assert_eq!(last, 4095);

        // Check attributes at offset 48
        let attrs = u64::from_le_bytes(entry[48..56].try_into().unwrap());
        assert_eq!(attrs, 0x8000000000000001);

        // Check name at offset 56 (UTF-16LE)
        let name = parse_utf16le_name(&entry[56..128]);
        assert_eq!(name, "EFI");
    }

    // -----------------------------------------------------------------------
    // match_partitions: existing partition with empty name
    // -----------------------------------------------------------------------

    #[test]
    fn test_match_partitions_empty_name_uses_definition_label() {
        let defs = vec![PartitionDefinition {
            filename: "50-root.conf".into(),
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            type_id: Some("root".into()),
            label: Some("my-root".into()),
            ..Default::default()
        }];

        let existing = vec![GptPartition {
            type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            unique_guid: "abcdefab-1234-5678-9abc-def012345678".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
            name: String::new(), // empty name
            slot_index: 0,
        }];

        let matched = match_partitions(&defs, &existing);
        assert_eq!(matched.len(), 1);
        assert!(!matched[0].is_new);
        assert_eq!(matched[0].assigned_label, "my-root");
    }

    #[test]
    fn test_match_partitions_existing_name_preserved() {
        let defs = vec![PartitionDefinition {
            filename: "50-root.conf".into(),
            type_uuid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            type_id: Some("root".into()),
            label: Some("my-root".into()),
            ..Default::default()
        }];

        let existing = vec![GptPartition {
            type_guid: "4f68bce3-e8cd-4db1-96e7-fbcaf984b709".into(),
            unique_guid: "abcdefab-1234-5678-9abc-def012345678".into(),
            first_lba: 2048,
            last_lba: 4095,
            attributes: 0,
            name: "existing-label".into(),
            slot_index: 0,
        }];

        let matched = match_partitions(&defs, &existing);
        assert_eq!(matched.len(), 1);
        assert_eq!(matched[0].assigned_label, "existing-label");
    }

    // -----------------------------------------------------------------------
    // run with --empty=create requires --size
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_empty_create_requires_size() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();
        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let img_path = tmp.path().join("nosize.img");
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--seed=test",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run with --empty=create --size=auto
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_empty_create_auto_size() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=1M\n",
        )
        .unwrap();

        let img_path = tmp.path().join("auto.img");
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=auto",
            "--seed=auto-test",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        // Image should exist and have a valid GPT
        let mut f = fs::File::open(&img_path).unwrap();
        let hdr = read_gpt_header(&mut f, 512).unwrap();
        assert!(hdr.is_some());
    }

    /// `--empty=create` implies `--dry-run=no`: the test drives it with no
    /// `--dry-run=` of its own and still expects an image on disk.
    #[test]
    fn test_parse_args_empty_create_implies_no_dry_run() {
        let a = parse_args(&args(&["--empty=create", "--size=1G", "/tmp/x"])).unwrap();
        assert!(!a.dry_run);
        // An explicit --dry-run=yes does not survive it, matching upstream.
        let a = parse_args(&args(&["--dry-run=yes", "--empty=create", "--size=1G", "/tmp/x"]))
            .unwrap();
        assert!(!a.dry_run);
        // --can-factory-reset wins, and only reports.
        let a = parse_args(&args(&["--can-factory-reset", "--empty=create"])).unwrap();
        assert!(a.dry_run);
    }

    /// With no matching definitions, `--empty=create` must still write the
    /// image. Upstream carries on past an empty definition set, so bailing out
    /// early left the caller with no file at all.
    #[test]
    fn test_full_empty_create_without_definitions() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        let img_path = tmp.path().join("zzz");
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=1G",
            "--seed=empty-defs",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok(), "run failed: {result:?}");

        let mut f = fs::File::open(&img_path).unwrap();
        assert_eq!(f.metadata().unwrap().len(), 1024 * 1024 * 1024);
        let hdr = read_gpt_header(&mut f, 512).unwrap().expect("no GPT header");
        // Matches what `sfdisk -d` reports for an upstream-created empty image.
        assert_eq!(hdr.first_usable_lba, 2048);
        assert_eq!(hdr.last_usable_lba, 2097118);
    }

    // -----------------------------------------------------------------------
    // run with no device specified
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_no_device_errors() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();
        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let result = run(&args(&[
            "--dry-run=no",
            "--empty=allow",
            &format!("--definitions={}", defs_dir.display()),
        ]));
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // run with --sector-size=4096
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_sector_size_4096() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();

        fs::write(
            defs_dir.join("50-root.conf"),
            "[Partition]\nType=root\nSizeMinBytes=1M\nSizeMaxBytes=4M\n",
        )
        .unwrap();

        let img_path = tmp.path().join("sector4k.img");
        let result = run(&args(&[
            "--dry-run=no",
            "--empty=create",
            "--size=20M",
            "--seed=sector-test",
            "--sector-size=4096",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());

        let mut f = fs::File::open(&img_path).unwrap();
        let hdr = read_gpt_header(&mut f, 4096).unwrap();
        assert!(hdr.is_some());
        let parts = read_gpt_partitions(&mut f, &hdr.unwrap(), 4096).unwrap();
        assert_eq!(parts.len(), 1);
    }

    // -----------------------------------------------------------------------
    // run with --json=pretty output
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_json_pretty_output() {
        let tmp = TempDir::new().unwrap();
        let defs_dir = tmp.path().join("defs");
        fs::create_dir(&defs_dir).unwrap();
        fs::write(defs_dir.join("50-root.conf"), "[Partition]\nType=root\n").unwrap();

        let img_path = tmp.path().join("json_pretty.img");
        let result = run(&args(&[
            "--empty=create",
            "--size=20M",
            "--seed=random",
            "--json=pretty",
            &format!("--definitions={}", defs_dir.display()),
            img_path.to_str().unwrap(),
        ]));
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // partition_type_uuid: additional types
    // -----------------------------------------------------------------------

    #[test]
    fn test_partition_type_uuid_srv() {
        assert_eq!(
            partition_type_uuid("srv", "x86-64").unwrap(),
            "3b8f8425-20e0-4f3b-907f-1a25a76f98e8"
        );
    }

    #[test]
    fn test_partition_type_uuid_var() {
        assert_eq!(
            partition_type_uuid("var", "x86-64").unwrap(),
            "4d21b016-b534-45c2-a9fb-5c16e091fd2d"
        );
    }

    #[test]
    fn test_partition_type_uuid_tmp() {
        assert_eq!(
            partition_type_uuid("tmp", "x86-64").unwrap(),
            "7ec6f557-3bc5-4aca-b293-16ef5df639d1"
        );
    }

    #[test]
    fn test_partition_type_uuid_xbootldr() {
        assert_eq!(
            partition_type_uuid("xbootldr", "x86-64").unwrap(),
            "bc13c2ff-59e6-4262-a352-b275fd6f7172"
        );
    }

    #[test]
    fn test_partition_type_uuid_linux_generic() {
        assert_eq!(
            partition_type_uuid("linux-generic", "x86-64").unwrap(),
            "0fc63daf-8483-4772-8e79-3d69d8477de4"
        );
    }

    // -----------------------------------------------------------------------
    // effective_flags: clearing bits when set to false
    // -----------------------------------------------------------------------

    #[test]
    fn test_effective_flags_no_auto_false_clears() {
        let def = PartitionDefinition {
            flags: 1u64 << 63, // no_auto bit set in base
            no_auto: Some(false),
            ..Default::default()
        };
        assert_eq!(def.effective_flags() & (1u64 << 63), 0);
    }

    #[test]
    fn test_effective_flags_read_only_false_clears() {
        let def = PartitionDefinition {
            flags: 1u64 << 60,
            read_only: Some(false),
            ..Default::default()
        };
        assert_eq!(def.effective_flags() & (1u64 << 60), 0);
    }

    #[test]
    fn test_effective_flags_grow_fs_false_clears() {
        let def = PartitionDefinition {
            flags: 1u64 << 59,
            grow_fs: Some(false),
            ..Default::default()
        };
        assert_eq!(def.effective_flags() & (1u64 << 59), 0);
    }

    // -----------------------------------------------------------------------
    // effective_label: fallback to "Linux"
    // -----------------------------------------------------------------------

    #[test]
    fn test_effective_label_unknown_type_fallback() {
        let def = PartitionDefinition {
            type_uuid: "ffffffff-ffff-ffff-ffff-ffffffffffff".into(),
            label: None,
            type_id: None,
            ..Default::default()
        };
        assert_eq!(def.effective_label(), "Linux");
    }

    // -----------------------------------------------------------------------
    // CRC32 additional vectors
    // -----------------------------------------------------------------------

    #[test]
    fn test_crc32_abc() {
        // Known CRC32 of "abc"
        assert_eq!(crc32(b"abc"), 0x352441C2);
    }

    #[test]
    fn test_crc32_large_data() {
        // CRC of 1024 zero bytes
        let data = vec![0u8; 1024];
        let c = crc32(&data);
        // Should be deterministic
        assert_eq!(c, crc32(&data));
        assert_ne!(c, 0);
    }

    // -----------------------------------------------------------------------
    // parse_args: --make-ddi shortcuts
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_make_ddi_confext() {
        let a = parse_args(&args(&["--make-ddi=confext"])).unwrap();
        assert_eq!(a.empty, EmptyMode::Create);
        assert_eq!(a.size.as_deref(), Some("auto"));
        assert_eq!(a.seed.as_deref(), Some("random"));
    }

    #[test]
    fn test_parse_args_make_ddi_portable() {
        let a = parse_args(&args(&["--make-ddi=portable"])).unwrap();
        assert_eq!(a.empty, EmptyMode::Create);
    }

    #[test]
    fn test_parse_args_make_ddi_invalid() {
        assert!(parse_args(&args(&["--make-ddi=bogus"])).is_err());
    }

    #[test]
    fn test_parse_args_short_c() {
        let a = parse_args(&args(&["-C"])).unwrap();
        assert_eq!(a.empty, EmptyMode::Create);
        assert_eq!(a.size.as_deref(), Some("auto"));
    }

    #[test]
    fn test_parse_args_short_p() {
        let a = parse_args(&args(&["-P"])).unwrap();
        assert_eq!(a.empty, EmptyMode::Create);
    }

    // -----------------------------------------------------------------------
    // parse_args: --offline and --sector-size validation
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_offline_auto() {
        let a = parse_args(&args(&["--offline=auto"])).unwrap();
        assert!(a.offline.is_none());
    }

    #[test]
    fn test_parse_args_offline_yes() {
        let a = parse_args(&args(&["--offline=yes"])).unwrap();
        assert_eq!(a.offline, Some(true));
    }

    #[test]
    fn test_parse_args_sector_size_4096() {
        let a = parse_args(&args(&["--sector-size=4096"])).unwrap();
        assert_eq!(a.sector_size, 4096);
    }

    #[test]
    fn test_parse_args_sector_size_non_power_of_two() {
        assert!(parse_args(&args(&["--sector-size=1000"])).is_err());
    }

    #[test]
    fn test_parse_args_sector_size_too_small() {
        assert!(parse_args(&args(&["--sector-size=256"])).is_err());
    }

    #[test]
    fn test_parse_args_sector_size_too_large() {
        assert!(parse_args(&args(&["--sector-size=8192"])).is_err());
    }

    // -----------------------------------------------------------------------
    // parse_args: partition filter lists
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_include_partitions_comma_separated() {
        let a = parse_args(&args(&["--include-partitions=root,home,swap"])).unwrap();
        assert_eq!(a.include_partitions, vec!["root", "home", "swap"]);
    }

    #[test]
    fn test_parse_args_exclude_partitions_comma_separated() {
        let a = parse_args(&args(&["--exclude-partitions=swap"])).unwrap();
        assert_eq!(a.exclude_partitions, vec!["swap"]);
    }

    #[test]
    fn test_parse_args_defer_partitions_comma_separated() {
        let a = parse_args(&args(&["--defer-partitions=home,var"])).unwrap();
        assert_eq!(a.defer_partitions, vec!["home", "var"]);
    }

    // -----------------------------------------------------------------------
    // parse_definition: PaddingWeight and PaddingMinBytes/MaxBytes
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_definition_padding() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("50-pad.conf");
        fs::write(
            &path,
            "[Partition]\nType=root\nPaddingMinBytes=1M\nPaddingMaxBytes=10M\nPaddingWeight=500\n",
        )
        .unwrap();
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.padding_min, 1024 * 1024);
        assert_eq!(def.padding_max, 10 * 1024 * 1024);
        assert_eq!(def.padding_weight, 500);
    }

    #[test]
    fn test_parse_definition_padding_weight_out_of_range() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("50-pad-bad.conf");
        fs::write(&path, "[Partition]\nType=root\nPaddingWeight=9999999\n").unwrap();
        assert!(parse_partition_definition(&path, "x86-64").is_err());
    }

    // -----------------------------------------------------------------------
    // parse_definition: MakeDirectories with multiple entries
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_definition_make_directories() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("50-dirs.conf");
        fs::write(
            &path,
            "[Partition]\nType=root\nMakeDirectories=/foo /bar /baz\n",
        )
        .unwrap();
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.make_directories, vec!["/foo", "/bar", "/baz"]);
    }

    // -----------------------------------------------------------------------
    // parse_definition: CopyBlocks
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_definition_copy_blocks() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("50-blocks.conf");
        fs::write(&path, "[Partition]\nType=root\nCopyBlocks=/dev/sda1\n").unwrap();
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert_eq!(def.copy_blocks.as_deref(), Some("/dev/sda1"));
    }

    #[test]
    fn test_parse_definition_copy_blocks_reset() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("50-blocks-reset.conf");
        fs::write(
            &path,
            "[Partition]\nType=root\nCopyBlocks=/dev/sda1\nCopyBlocks=\n",
        )
        .unwrap();
        let def = parse_partition_definition(&path, "x86-64").unwrap();
        assert!(def.copy_blocks.is_none());
    }

    // -----------------------------------------------------------------------
    // derive_uuid: version and variant bits
    // -----------------------------------------------------------------------

    #[test]
    fn test_generate_uuid_from_seed_version_and_variant() {
        let uuid = derive_uuid("750b6cd5c4ae4012a15e7be3c29e6a47", "name");
        // The v4 marking puts '4' at position 14 (first char of 3rd group).
        assert_eq!(uuid.chars().nth(14), Some('4'));
        // Variant 1: clock_seq high nibble at position 19 should be 8, 9, a, or b
        let variant_char = uuid.chars().nth(19).unwrap().to_ascii_lowercase();
        assert!(
            "89ab".contains(variant_char),
            "variant char '{}' not in 89ab",
            variant_char
        );
    }

    // -----------------------------------------------------------------------
    // generate_random_uuid: format
    // -----------------------------------------------------------------------

    #[test]
    fn test_generate_random_uuid_format() {
        let uuid = generate_random_uuid();
        assert_eq!(uuid.len(), 36);
        assert_eq!(uuid.chars().filter(|c| *c == '-').count(), 4);
        // generate_random_uuid sets bytes[6] = (x & 0x0F) | 0x40 then formats
        // the 3rd group as u16::from_le_bytes([bytes[6], bytes[7]]), so the
        // version nibble '4' appears at position 16 (3rd hex digit of 3rd group).
        assert_eq!(uuid.chars().nth(16), Some('4'));
        // Variant 1: bytes[8] = (x & 0x3F) | 0x80, formatted directly at pos 19
        let variant = uuid.chars().nth(19).unwrap();
        assert!("89ab".contains(variant));
    }

    #[test]
    fn test_generate_random_uuid_unique() {
        let a = generate_random_uuid();
        let b = generate_random_uuid();
        assert_ne!(a, b);
    }

    // -----------------------------------------------------------------------
    // is_zero_guid edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_is_zero_guid_uppercase_false() {
        assert!(!is_zero_guid("00000000-0000-0000-0000-00000000000A"));
    }

    #[test]
    fn test_is_zero_guid_empty_false() {
        assert!(!is_zero_guid(""));
    }

    // -----------------------------------------------------------------------
    // encode_utf16le_name: max length (72 bytes = 36 UTF-16 chars)
    // -----------------------------------------------------------------------

    #[test]
    fn test_encode_utf16le_name_truncates_long_name() {
        let long_name = "A".repeat(100);
        let encoded = encode_utf16le_name(&long_name);
        assert_eq!(encoded.len(), 72);
    }

    // -----------------------------------------------------------------------
    // GPT header fields consistency
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_gpt_header_primary_fields() {
        let disk_sectors: u64 = 20480; // 10M with 512-byte sectors
        let sector_size: u64 = 512;
        let data = build_gpt_header(
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            &[],
            disk_sectors,
            sector_size,
            false,
        );

        let header = &data[..sector_size as usize];

        // Signature
        assert_eq!(&header[0..8], GPT_SIGNATURE);
        // MyLBA = 1 for primary
        let my_lba = u64::from_le_bytes(header[24..32].try_into().unwrap());
        assert_eq!(my_lba, 1);
        // AlternateLBA = last sector
        let alt_lba = u64::from_le_bytes(header[32..40].try_into().unwrap());
        assert_eq!(alt_lba, disk_sectors - 1);
        // FirstUsableLBA > 1
        let first = u64::from_le_bytes(header[40..48].try_into().unwrap());
        assert!(first > 1);
        // LastUsableLBA < disk_sectors - 1
        let last = u64::from_le_bytes(header[48..56].try_into().unwrap());
        assert!(last < disk_sectors - 1);
        // FirstUsable <= LastUsable
        assert!(first <= last);
    }

    #[test]
    fn test_build_gpt_header_backup_fields() {
        let disk_sectors: u64 = 20480;
        let sector_size: u64 = 512;
        let data = build_gpt_header(
            "aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee",
            &[],
            disk_sectors,
            sector_size,
            true,
        );

        let header = &data[..sector_size as usize];

        // MyLBA = last sector for backup
        let my_lba = u64::from_le_bytes(header[24..32].try_into().unwrap());
        assert_eq!(my_lba, disk_sectors - 1);
        // AlternateLBA = 1 for backup
        let alt_lba = u64::from_le_bytes(header[32..40].try_into().unwrap());
        assert_eq!(alt_lba, 1);
    }
}
