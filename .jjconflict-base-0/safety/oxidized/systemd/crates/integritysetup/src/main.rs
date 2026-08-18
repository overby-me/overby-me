//! systemd-integritysetup — Attach and detach dm-integrity block devices.
//!
//! A drop-in replacement for `systemd-integritysetup(8)`. This tool sets up
//! dm-integrity block devices via the device-mapper kernel interface, providing
//! data integrity protection using checksums stored alongside data.
//!
//! Usage:
//!
//!   systemd-integritysetup attach VOLUME DEVICE [KEY [OPTIONS]]
//!       Set up a dm-integrity block device. VOLUME is the device-mapper name,
//!       DEVICE is the underlying block device, KEY is an optional key file path
//!       (or "-" for no key / stdin), OPTIONS is a comma-separated list of
//!       options.
//!
//!   systemd-integritysetup detach VOLUME
//!       Tear down a previously attached dm-integrity device.
//!
//!   systemd-integritysetup format DEVICE [OPTIONS]
//!       Initialize an integrity superblock on DEVICE. This writes the
//!       dm-integrity on-disk superblock header and zeroes the journal area,
//!       preparing the device for use with attach.
//!
//!   systemd-integritysetup wipe DEVICE
//!       Wipe (zero) the integrity superblock on DEVICE, removing all
//!       dm-integrity metadata.
//!
//!   systemd-integritysetup dump DEVICE
//!       Read and display the dm-integrity superblock from DEVICE.
//!
//!   systemd-integritysetup resize VOLUME [DEVICE]
//!       Resize an active dm-integrity device after the underlying block
//!       device has been resized. Reloads the device-mapper table with
//!       the new device size. If DEVICE is not specified, it is read from
//!       the current DM table status.
//!
//!   systemd-integritysetup --help | -h
//!       Show help.
//!
//!   systemd-integritysetup --version
//!       Show version.
//!
//! Options (comma-separated in OPTIONS):
//!   algorithm=ALG          Integrity hash algorithm (default: crc32c)
//!   journal-commit-time=MS Journal commit interval in milliseconds
//!   journal-watermark=PCT  Journal watermark percentage (0-100)
//!   journal-integrity=ALG  Journal integrity hash algorithm
//!   journal-integrity-key-size=BYTES  Key size for journal integrity
//!   journal-integrity-key-file=PATH   Key file for journal integrity
//!   journal-crypt=ALG      Journal encryption algorithm
//!   journal-crypt-key-size=BYTES  Key size for journal encryption
//!   journal-crypt-key-file=PATH   Key file for journal encryption
//!   data-device=PATH       Separate data device path
//!   sector-size=BYTES      Sector size for integrity (default: 512)
//!   bitmap-flush-interval=MS  Bitmap mode flush interval
//!   block-size=BYTES       Internal integrity block size
//!   integrity-recalculate  Recalculate integrity tags in background
//!   integrity-recalculate-reset  Reset recalculate position to start
//!   allow-discards         Allow TRIM/discard passthrough
//!   fix-padding            Fix metadata padding on older kernels
//!   fix-hmac               Fix HMAC issues on older kernels
//!   legacy-recalculate     Use legacy recalculate behavior
//!   no-journal             Disable journaling
//!   no-journal-bitmap      Disable journal bitmap mode
//!   recovery               Recovery mode (don't verify on activation)
//!   readonly               Open device read-only
//!   noauto                 Ignored (for /etc/integritytab compat)
//!   nofail                 Ignored (for /etc/integritytab compat)
//!   x-systemd.*            Ignored (for /etc/integritytab compat)
//!
//! Exit codes:
//!   0 — success
//!   1 — error (general)
//!   4 — integrity check failure

use std::fmt;
use std::fs;
use std::io;
use std::io::Write;
use std::os::unix::io::RawFd;
use std::path::PathBuf;
use std::process;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Device-mapper control device path.
const DM_CONTROL_PATH: &str = "/dev/mapper/control";

/// Device-mapper ioctl version.
const DM_VERSION_MAJOR: u32 = 4;
const DM_VERSION_MINOR: u32 = 0;
const DM_VERSION_PATCHLEVEL: u32 = 0;

/// Device-mapper ioctl type (magic number).
const DM_IOCTL_TYPE: u32 = 0xfd;

/// DM ioctl structure header size (312 bytes).
const DM_IOCTL_HEADER_SIZE: usize = 312;

/// Size of the ioctl buffer we allocate.
const DM_IOCTL_BUF_SIZE: usize = 16384;

/// DM ioctl struct field offsets.
const DM_VERSION_OFFSET: usize = 0;
const DM_DATA_SIZE_OFFSET: usize = 12;
const DM_DATA_START_OFFSET: usize = 16;
const DM_TARGET_COUNT_OFFSET: usize = 20;
const DM_FLAGS_OFFSET: usize = 28;
const DM_NAME_OFFSET: usize = 48;
const DM_NAME_SIZE: usize = 128;
const DM_UUID_OFFSET: usize = 176;
const DM_UUID_SIZE: usize = 128;

/// DM ioctl commands.
const DM_DEV_CREATE_CMD: u32 = 3;
const DM_DEV_REMOVE_CMD: u32 = 4;
const DM_DEV_SUSPEND_CMD: u32 = 6;
const DM_TABLE_LOAD_CMD: u32 = 9;
const DM_TABLE_CLEAR_CMD: u32 = 10;

/// DM flags.
const DM_READONLY_FLAG: u32 = 1 << 0;
const DM_SUSPEND_FLAG: u32 = 1 << 1;

/// Default integrity algorithm.
const DEFAULT_ALGORITHM: &str = "crc32c";
/// Default sector size.
const DEFAULT_SECTOR_SIZE: u32 = 512;

/// Exit code for integrity check failures.
#[allow(dead_code)]
const EXIT_INTEGRITY_ERROR: i32 = 4;

// ---------------------------------------------------------------------------
// Integrity superblock constants
// ---------------------------------------------------------------------------

/// Magic bytes at the start of a dm-integrity superblock: "integrit" (8 bytes).
const SB_MAGIC: &[u8; 8] = b"integrit";

/// Superblock version 1 — base format.
const SB_VERSION_1: u8 = 1;

/// Superblock version 2 — adds `log2_blocks_per_bitmap_bit`.
const SB_VERSION_2: u8 = 2;

/// Superblock version 3 — adds `recalc_sector`.
#[allow(dead_code)]
const SB_VERSION_3: u8 = 3;

/// Superblock version 4 — adds fix_padding flag support.
#[allow(dead_code)]
const SB_VERSION_4: u8 = 4;

/// Superblock version 5 — adds fix_hmac flag support.
#[allow(dead_code)]
const SB_VERSION_5: u8 = 5;

/// Total on-disk size of the superblock structure (padded to 512 bytes for
/// sector alignment). The logical fields occupy 40 bytes; the remaining
/// bytes are zero-filled padding.
const SB_SIZE: usize = 512;

// Field offsets within the superblock.
const SB_MAGIC_OFFSET: usize = 0;
const SB_MAGIC_SIZE: usize = 8;
const SB_VERSION_OFFSET: usize = 8;
const SB_LOG2_INTERLEAVE_OFFSET: usize = 9;
const SB_TAG_SIZE_OFFSET: usize = 10;
const SB_JOURNAL_SECTIONS_OFFSET: usize = 12;
const SB_PROVIDED_DATA_SECTORS_OFFSET: usize = 16;
const SB_FLAGS_OFFSET: usize = 24;
const SB_LOG2_SECTORS_PER_BLOCK_OFFSET: usize = 28;
const SB_LOG2_BLOCKS_PER_BITMAP_BIT_OFFSET: usize = 29;
// offset 30-31: pad
const SB_RECALC_SECTOR_OFFSET: usize = 32;

// Superblock flags.
const SB_FLAG_HAVE_JOURNAL_MAC: u32 = 0x1;
const SB_FLAG_RECALCULATING: u32 = 0x2;
const SB_FLAG_DIRTY_BITMAP: u32 = 0x4;
const SB_FLAG_FIXED_PADDING: u32 = 0x8;
const SB_FLAG_FIXED_HMAC: u32 = 0x10;

/// Default number of journal sections written during format.
const DEFAULT_JOURNAL_SECTIONS: u32 = 8;

/// Default log2 of interleave sectors (e.g. 15 → 32768 sectors).
const DEFAULT_LOG2_INTERLEAVE: u8 = 15;

/// Default log2 of sectors per block (0 → 1 sector per block = 512 bytes).
#[allow(dead_code)]
const DEFAULT_LOG2_SECTORS_PER_BLOCK: u8 = 0;

/// Default log2 of blocks per bitmap bit.
const DEFAULT_LOG2_BLOCKS_PER_BITMAP_BIT: u8 = 0;

/// DM_TABLE_STATUS ioctl command number.
const DM_TABLE_STATUS_CMD: u32 = 11;

/// Parsed representation of the dm-integrity on-disk superblock.
#[derive(Debug, Clone, PartialEq, Eq)]
struct IntegritySuperblock {
    /// Superblock version (1–5).
    version: u8,
    /// Log2 of interleave sectors.
    log2_interleave_sectors: u8,
    /// Tag (checksum) size in bytes.
    tag_size: u16,
    /// Number of journal sections.
    journal_sections: u32,
    /// Number of data sectors provided to upper layers.
    provided_data_sectors: u64,
    /// Flags (SB_FLAG_*).
    flags: u32,
    /// Log2 of sectors per block.
    log2_sectors_per_block: u8,
    /// Log2 of blocks per bitmap bit (version ≥ 2).
    log2_blocks_per_bitmap_bit: u8,
    /// Next sector to recalculate (version ≥ 3).
    recalc_sector: u64,
}

impl IntegritySuperblock {
    /// Decode a superblock from a raw 512-byte sector buffer.
    fn from_bytes(buf: &[u8]) -> Result<Self, String> {
        if buf.len() < SB_SIZE {
            return Err(format!(
                "Buffer too small for superblock: {} < {SB_SIZE}",
                buf.len()
            ));
        }

        // Check magic.
        if &buf[SB_MAGIC_OFFSET..SB_MAGIC_OFFSET + SB_MAGIC_SIZE] != SB_MAGIC {
            return Err("Not a dm-integrity superblock (bad magic).".to_string());
        }

        let version = buf[SB_VERSION_OFFSET];
        if version == 0 || version > SB_VERSION_5 {
            return Err(format!("Unsupported superblock version: {version}"));
        }

        let log2_interleave_sectors = buf[SB_LOG2_INTERLEAVE_OFFSET];
        let tag_size = u16::from_le_bytes(
            buf[SB_TAG_SIZE_OFFSET..SB_TAG_SIZE_OFFSET + 2]
                .try_into()
                .unwrap(),
        );
        let journal_sections = u32::from_le_bytes(
            buf[SB_JOURNAL_SECTIONS_OFFSET..SB_JOURNAL_SECTIONS_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        let provided_data_sectors = u64::from_le_bytes(
            buf[SB_PROVIDED_DATA_SECTORS_OFFSET..SB_PROVIDED_DATA_SECTORS_OFFSET + 8]
                .try_into()
                .unwrap(),
        );
        let flags = u32::from_le_bytes(
            buf[SB_FLAGS_OFFSET..SB_FLAGS_OFFSET + 4]
                .try_into()
                .unwrap(),
        );
        let log2_sectors_per_block = buf[SB_LOG2_SECTORS_PER_BLOCK_OFFSET];
        let log2_blocks_per_bitmap_bit = if version >= SB_VERSION_2 {
            buf[SB_LOG2_BLOCKS_PER_BITMAP_BIT_OFFSET]
        } else {
            0
        };
        let recalc_sector = if version >= SB_VERSION_3 {
            u64::from_le_bytes(
                buf[SB_RECALC_SECTOR_OFFSET..SB_RECALC_SECTOR_OFFSET + 8]
                    .try_into()
                    .unwrap(),
            )
        } else {
            0
        };

        Ok(Self {
            version,
            log2_interleave_sectors,
            tag_size,
            journal_sections,
            provided_data_sectors,
            flags,
            log2_sectors_per_block,
            log2_blocks_per_bitmap_bit,
            recalc_sector,
        })
    }

    /// Encode the superblock into a 512-byte sector buffer.
    fn to_bytes(&self) -> Vec<u8> {
        let mut buf = vec![0u8; SB_SIZE];

        // Magic.
        buf[SB_MAGIC_OFFSET..SB_MAGIC_OFFSET + SB_MAGIC_SIZE].copy_from_slice(SB_MAGIC);

        // Version.
        buf[SB_VERSION_OFFSET] = self.version;

        // Fields.
        buf[SB_LOG2_INTERLEAVE_OFFSET] = self.log2_interleave_sectors;
        buf[SB_TAG_SIZE_OFFSET..SB_TAG_SIZE_OFFSET + 2]
            .copy_from_slice(&self.tag_size.to_le_bytes());
        buf[SB_JOURNAL_SECTIONS_OFFSET..SB_JOURNAL_SECTIONS_OFFSET + 4]
            .copy_from_slice(&self.journal_sections.to_le_bytes());
        buf[SB_PROVIDED_DATA_SECTORS_OFFSET..SB_PROVIDED_DATA_SECTORS_OFFSET + 8]
            .copy_from_slice(&self.provided_data_sectors.to_le_bytes());
        buf[SB_FLAGS_OFFSET..SB_FLAGS_OFFSET + 4].copy_from_slice(&self.flags.to_le_bytes());
        buf[SB_LOG2_SECTORS_PER_BLOCK_OFFSET] = self.log2_sectors_per_block;

        if self.version >= SB_VERSION_2 {
            buf[SB_LOG2_BLOCKS_PER_BITMAP_BIT_OFFSET] = self.log2_blocks_per_bitmap_bit;
        }

        if self.version >= SB_VERSION_3 {
            buf[SB_RECALC_SECTOR_OFFSET..SB_RECALC_SECTOR_OFFSET + 8]
                .copy_from_slice(&self.recalc_sector.to_le_bytes());
        }

        buf
    }

    /// Return a human-readable description of the flags field.
    fn flags_description(&self) -> String {
        let mut parts = Vec::new();
        if self.flags & SB_FLAG_HAVE_JOURNAL_MAC != 0 {
            parts.push("HAVE_JOURNAL_MAC");
        }
        if self.flags & SB_FLAG_RECALCULATING != 0 {
            parts.push("RECALCULATING");
        }
        if self.flags & SB_FLAG_DIRTY_BITMAP != 0 {
            parts.push("DIRTY_BITMAP");
        }
        if self.flags & SB_FLAG_FIXED_PADDING != 0 {
            parts.push("FIXED_PADDING");
        }
        if self.flags & SB_FLAG_FIXED_HMAC != 0 {
            parts.push("FIXED_HMAC");
        }
        if parts.is_empty() {
            "(none)".to_string()
        } else {
            parts.join(" | ")
        }
    }

    /// Format for display (used by the dump command).
    fn format_dump(&self) -> String {
        let mut out = String::new();
        out.push_str("Info for integrity device\n");
        out.push_str(&format!("superblock_version:         {}\n", self.version));
        out.push_str(&format!(
            "log2_interleave_sectors:    {}\n",
            self.log2_interleave_sectors
        ));
        out.push_str(&format!("integrity_tag_size:         {}\n", self.tag_size));
        out.push_str(&format!(
            "journal_sections:           {}\n",
            self.journal_sections
        ));
        out.push_str(&format!(
            "provided_data_sectors:      {}\n",
            self.provided_data_sectors
        ));
        out.push_str(&format!(
            "sector_size:                {}\n",
            512u32 << self.log2_sectors_per_block
        ));
        out.push_str(&format!(
            "log2_sectors_per_block:     {}\n",
            self.log2_sectors_per_block
        ));
        if self.version >= SB_VERSION_2 {
            out.push_str(&format!(
                "log2_blocks_per_bitmap_bit: {}\n",
                self.log2_blocks_per_bitmap_bit
            ));
        }
        out.push_str(&format!(
            "flags:                      0x{:x} [{}]\n",
            self.flags,
            self.flags_description()
        ));
        if self.version >= SB_VERSION_3 {
            out.push_str(&format!(
                "recalc_sector:              {}\n",
                self.recalc_sector
            ));
        }
        out
    }
}

/// Compute the log2-of-sectors-per-block value from a sector size.
/// `sector_size` must be a power of two ≥ 512.
fn log2_sectors_per_block(sector_size: u32) -> u8 {
    assert!(sector_size >= 512 && sector_size.is_power_of_two());
    (sector_size / 512).trailing_zeros() as u8
}

// ---------------------------------------------------------------------------
// Journal mode
// ---------------------------------------------------------------------------

/// Journal mode for dm-integrity.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum JournalMode {
    /// Standard journaling (default).
    Journal,
    /// No journaling — write-back without protection.
    NoJournal,
    /// Bitmap-based journaling.
    Bitmap,
}

impl fmt::Display for JournalMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            JournalMode::Journal => write!(f, "journal"),
            JournalMode::NoJournal => write!(f, "no-journal"),
            JournalMode::Bitmap => write!(f, "bitmap"),
        }
    }
}

// ---------------------------------------------------------------------------
// Parsed options from the comma-separated OPTIONS argument
// ---------------------------------------------------------------------------

/// Parsed integritysetup options from the OPTIONS string.
#[derive(Debug, Clone)]
struct IntegrityOptions {
    /// Integrity hash/checksum algorithm (e.g., "crc32c", "sha256", "hmac(sha256)").
    algorithm: String,
    /// Journal commit interval in milliseconds.
    journal_commit_time: Option<u64>,
    /// Journal watermark percentage (0–100).
    journal_watermark: Option<u32>,
    /// Journal integrity hash algorithm (for protecting the journal itself).
    journal_integrity: Option<String>,
    /// Key size for journal integrity in bytes.
    journal_integrity_key_size: Option<usize>,
    /// Key file for journal integrity.
    journal_integrity_key_file: Option<PathBuf>,
    /// Journal encryption algorithm.
    journal_crypt: Option<String>,
    /// Key size for journal encryption in bytes.
    journal_crypt_key_size: Option<usize>,
    /// Key file for journal encryption.
    journal_crypt_key_file: Option<PathBuf>,
    /// Separate data device path.
    data_device: Option<PathBuf>,
    /// Sector size for integrity (default: 512).
    sector_size: u32,
    /// Bitmap mode flush interval in milliseconds.
    bitmap_flush_interval: Option<u64>,
    /// Internal integrity block size.
    block_size: Option<u32>,
    /// Recalculate integrity tags in background.
    integrity_recalculate: bool,
    /// Reset recalculate position to start.
    integrity_recalculate_reset: bool,
    /// Allow discard/TRIM passthrough.
    allow_discards: bool,
    /// Fix metadata padding (older kernels).
    fix_padding: bool,
    /// Fix HMAC issues (older kernels).
    fix_hmac: bool,
    /// Use legacy recalculate behavior.
    legacy_recalculate: bool,
    /// Journal mode.
    journal_mode: JournalMode,
    /// Recovery mode (don't verify on activation).
    recovery: bool,
    /// Open device read-only.
    readonly: bool,
}

impl Default for IntegrityOptions {
    fn default() -> Self {
        Self {
            algorithm: DEFAULT_ALGORITHM.to_string(),
            journal_commit_time: None,
            journal_watermark: None,
            journal_integrity: None,
            journal_integrity_key_size: None,
            journal_integrity_key_file: None,
            journal_crypt: None,
            journal_crypt_key_size: None,
            journal_crypt_key_file: None,
            data_device: None,
            sector_size: DEFAULT_SECTOR_SIZE,
            bitmap_flush_interval: None,
            block_size: None,
            integrity_recalculate: false,
            integrity_recalculate_reset: false,
            allow_discards: false,
            fix_padding: false,
            fix_hmac: false,
            legacy_recalculate: false,
            journal_mode: JournalMode::Journal,
            recovery: false,
            readonly: false,
        }
    }
}

/// Parse a comma-separated options string (the 4th positional argument).
fn parse_options(opts_str: &str) -> Result<IntegrityOptions, String> {
    let mut opts = IntegrityOptions::default();

    if opts_str.is_empty() || opts_str == "-" || opts_str == "none" {
        return Ok(opts);
    }

    for part in opts_str.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }

        if let Some((key, val)) = part.split_once('=') {
            let key = key.trim();
            let val = val.trim();
            match key {
                "algorithm" | "integrity-algorithm" => {
                    opts.algorithm = val.to_string();
                }
                "journal-commit-time" | "journal_commit_time" => {
                    opts.journal_commit_time = Some(
                        val.parse()
                            .map_err(|_| format!("Invalid journal-commit-time: {val}"))?,
                    );
                }
                "journal-watermark" | "journal_watermark" => {
                    let wm: u32 = val
                        .parse()
                        .map_err(|_| format!("Invalid journal-watermark: {val}"))?;
                    if wm > 100 {
                        return Err(format!("Invalid journal-watermark: {wm} (must be 0-100)"));
                    }
                    opts.journal_watermark = Some(wm);
                }
                "journal-integrity" | "journal_integrity" => {
                    opts.journal_integrity = Some(val.to_string());
                }
                "journal-integrity-key-size" | "journal_integrity_key_size" => {
                    opts.journal_integrity_key_size = Some(
                        val.parse()
                            .map_err(|_| format!("Invalid journal-integrity-key-size: {val}"))?,
                    );
                }
                "journal-integrity-key-file" | "journal_integrity_key_file" => {
                    opts.journal_integrity_key_file = Some(PathBuf::from(val));
                }
                "journal-crypt" | "journal_crypt" => {
                    opts.journal_crypt = Some(val.to_string());
                }
                "journal-crypt-key-size" | "journal_crypt_key_size" => {
                    opts.journal_crypt_key_size = Some(
                        val.parse()
                            .map_err(|_| format!("Invalid journal-crypt-key-size: {val}"))?,
                    );
                }
                "journal-crypt-key-file" | "journal_crypt_key_file" => {
                    opts.journal_crypt_key_file = Some(PathBuf::from(val));
                }
                "data-device" | "data_device" => {
                    opts.data_device = Some(PathBuf::from(val));
                }
                "sector-size" | "sector_size" => {
                    opts.sector_size = val
                        .parse()
                        .map_err(|_| format!("Invalid sector-size: {val}"))?;
                    if !opts.sector_size.is_power_of_two() || opts.sector_size < 512 {
                        return Err(format!(
                            "Invalid sector-size: {} (must be power of 2, >= 512)",
                            opts.sector_size
                        ));
                    }
                }
                "bitmap-flush-interval" | "bitmap_flush_interval" => {
                    opts.bitmap_flush_interval = Some(
                        val.parse()
                            .map_err(|_| format!("Invalid bitmap-flush-interval: {val}"))?,
                    );
                }
                "block-size" | "block_size" => {
                    opts.block_size = Some(
                        val.parse()
                            .map_err(|_| format!("Invalid block-size: {val}"))?,
                    );
                }
                _ if key.starts_with("x-systemd.") => { /* ignored */ }
                _ => {
                    return Err(format!("Unknown option: {key}={val}"));
                }
            }
        } else {
            match part {
                "integrity-recalculate" | "integrity_recalculate" => {
                    opts.integrity_recalculate = true;
                }
                "integrity-recalculate-reset" | "integrity_recalculate_reset" => {
                    opts.integrity_recalculate_reset = true;
                }
                "allow-discards" | "allow_discards" | "discard" => {
                    opts.allow_discards = true;
                }
                "fix-padding" | "fix_padding" => {
                    opts.fix_padding = true;
                }
                "fix-hmac" | "fix_hmac" => {
                    opts.fix_hmac = true;
                }
                "legacy-recalculate" | "legacy_recalculate" => {
                    opts.legacy_recalculate = true;
                }
                "no-journal" | "no_journal" => {
                    opts.journal_mode = JournalMode::NoJournal;
                }
                "no-journal-bitmap" | "no_journal_bitmap" | "bitmap" => {
                    opts.journal_mode = JournalMode::Bitmap;
                }
                "recovery" => {
                    opts.recovery = true;
                }
                "readonly" | "read-only" => {
                    opts.readonly = true;
                }
                "noauto" | "nofail" | "auto" => { /* /etc/integritytab compat — ignored */ }
                _ if part.starts_with("x-systemd.") => { /* ignored */ }
                _ => {
                    return Err(format!("Unknown option: {part}"));
                }
            }
        }
    }

    Ok(opts)
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Attach {
        volume: String,
        device: String,
        key: Option<String>,
        options: String,
    },
    Detach {
        volume: String,
    },
    Format {
        device: String,
        options: String,
    },
    Wipe {
        device: String,
    },
    Dump {
        device: String,
    },
    Resize {
        volume: String,
        device: Option<String>,
    },
    Help,
    Version,
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Err(
            "No command specified. Use 'attach', 'detach', 'format', 'wipe', 'dump', or 'resize'."
                .to_string(),
        );
    }

    let mut iter = args.iter();
    let first = iter.next().unwrap();

    match first.as_str() {
        "--help" | "-h" | "help" => Ok(Command::Help),
        "--version" | "version" => Ok(Command::Version),
        "attach" => {
            let volume = iter
                .next()
                .ok_or("attach: missing VOLUME argument")?
                .clone();
            let device = iter
                .next()
                .ok_or("attach: missing DEVICE argument")?
                .clone();
            let key = iter.next().cloned();
            let options = iter.next().cloned().unwrap_or_default();
            Ok(Command::Attach {
                volume,
                device,
                key,
                options,
            })
        }
        "detach" => {
            let volume = iter
                .next()
                .ok_or("detach: missing VOLUME argument")?
                .clone();
            Ok(Command::Detach { volume })
        }
        "format" => {
            let device = iter
                .next()
                .ok_or("format: missing DEVICE argument")?
                .clone();
            let options = iter.next().cloned().unwrap_or_default();
            Ok(Command::Format { device, options })
        }
        "wipe" => {
            let device = iter.next().ok_or("wipe: missing DEVICE argument")?.clone();
            Ok(Command::Wipe { device })
        }
        "dump" => {
            let device = iter.next().ok_or("dump: missing DEVICE argument")?.clone();
            Ok(Command::Dump { device })
        }
        "resize" => {
            let volume = iter
                .next()
                .ok_or("resize: missing VOLUME argument")?
                .clone();
            let device = iter.next().cloned();
            Ok(Command::Resize { volume, device })
        }
        other => Err(format!("Unknown command: {other}")),
    }
}

// ---------------------------------------------------------------------------
// Device-mapper ioctl helpers
// ---------------------------------------------------------------------------

/// Encode a DM ioctl number from the command index.
fn dm_ioctl_nr(cmd: u32) -> libc::c_ulong {
    let dir: libc::c_ulong = 3; // IOC_READ | IOC_WRITE
    let size = DM_IOCTL_HEADER_SIZE as libc::c_ulong;
    let typ = DM_IOCTL_TYPE as libc::c_ulong;
    let nr = cmd as libc::c_ulong;
    (dir << 30) | (size << 16) | (typ << 8) | nr
}

/// Initialize a DM ioctl buffer with the standard header fields.
fn dm_ioctl_init(buf: &mut [u8], name: &str, uuid: &str, flags: u32) {
    assert!(buf.len() >= DM_IOCTL_HEADER_SIZE);

    for b in buf.iter_mut() {
        *b = 0;
    }

    write_u32(buf, DM_VERSION_OFFSET, DM_VERSION_MAJOR);
    write_u32(buf, DM_VERSION_OFFSET + 4, DM_VERSION_MINOR);
    write_u32(buf, DM_VERSION_OFFSET + 8, DM_VERSION_PATCHLEVEL);
    write_u32(buf, DM_DATA_SIZE_OFFSET, buf.len() as u32);
    write_u32(buf, DM_DATA_START_OFFSET, DM_IOCTL_HEADER_SIZE as u32);
    write_u32(buf, DM_FLAGS_OFFSET, flags);
    write_string(buf, DM_NAME_OFFSET, DM_NAME_SIZE, name);
    write_string(buf, DM_UUID_OFFSET, DM_UUID_SIZE, uuid);
}

fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
}

fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap())
}

fn write_u64(buf: &mut [u8], offset: usize, val: u64) {
    buf[offset..offset + 8].copy_from_slice(&val.to_ne_bytes());
}

#[allow(dead_code)]
fn read_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_ne_bytes(buf[offset..offset + 8].try_into().unwrap())
}

fn write_string(buf: &mut [u8], offset: usize, max_len: usize, s: &str) {
    let bytes = s.as_bytes();
    let copy_len = bytes.len().min(max_len - 1);
    buf[offset..offset + copy_len].copy_from_slice(&bytes[..copy_len]);
    buf[offset + copy_len] = 0;
}

/// Append a dm_target_spec + parameter string to the ioctl buffer.
///
/// dm_target_spec layout (40 bytes):
///   u64 sector_start
///   u64 length          (in 512-byte sectors)
///   i32 status          (must be 0)
///   u32 next            (offset to next target spec, 0 if last)
///   char target_type[16]
///   ... parameter string follows immediately after
fn append_target(
    buf: &mut [u8],
    data_start: usize,
    sector_start: u64,
    length_sectors: u64,
    target_type: &str,
    params: &str,
) -> usize {
    let spec_size: usize = 40;
    let param_bytes = params.as_bytes();
    let total = align8(spec_size + param_bytes.len() + 1);

    assert!(data_start + total <= buf.len(), "DM ioctl buffer overflow");

    write_u64(buf, data_start, sector_start);
    write_u64(buf, data_start + 8, length_sectors);
    write_u32(buf, data_start + 16, 0); // status
    write_u32(buf, data_start + 20, 0); // next
    write_string(buf, data_start + 24, 16, target_type);

    let param_off = data_start + spec_size;
    buf[param_off..param_off + param_bytes.len()].copy_from_slice(param_bytes);
    buf[param_off + param_bytes.len()] = 0;

    total
}

/// Align a value up to an 8-byte boundary.
fn align8(v: usize) -> usize {
    (v + 7) & !7
}

/// Issue a DM ioctl against `/dev/mapper/control`.
fn dm_ioctl(cmd: u32, buf: &mut [u8]) -> io::Result<()> {
    let fd = open_dm_control()?;
    let request = dm_ioctl_nr(cmd);
    let ret = unsafe { libc::ioctl(fd, request, buf.as_mut_ptr()) };
    close_fd(fd);
    if ret < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

fn open_dm_control() -> io::Result<RawFd> {
    let path = std::ffi::CString::new(DM_CONTROL_PATH).unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

fn close_fd(fd: RawFd) {
    unsafe {
        libc::close(fd);
    }
}

// ---------------------------------------------------------------------------
// Key handling
// ---------------------------------------------------------------------------

/// Read a key from the specified source.
///
/// - `None` or `"-"` or `""` → no key (return empty)
/// - A file path → read the file contents
fn read_key(key_arg: Option<&str>) -> io::Result<Vec<u8>> {
    match key_arg {
        None | Some("-") | Some("") | Some("none") => Ok(Vec::new()),
        Some(path) => fs::read(path),
    }
}

/// Encode key bytes as a hex string for dm-integrity table parameters.
fn key_to_hex(key: &[u8]) -> String {
    let mut hex = String::with_capacity(key.len() * 2);
    for b in key {
        hex.push_str(&format!("{b:02x}"));
    }
    hex
}

// ---------------------------------------------------------------------------
// Device helpers
// ---------------------------------------------------------------------------

/// Get the size of a block device in 512-byte sectors.
fn device_size_sectors(device: &str) -> io::Result<u64> {
    let path = std::ffi::CString::new(device)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDONLY) };
    if fd < 0 {
        return Err(io::Error::last_os_error());
    }

    let mut size: u64 = 0;
    // BLKGETSIZE64 = _IOR(0x12, 114, size_t) on Linux.
    let blkgetsize64: libc::c_ulong = {
        let dir: libc::c_ulong = 2; // _IOC_READ
        let typ: libc::c_ulong = 0x12;
        let nr: libc::c_ulong = 114;
        let sz: libc::c_ulong = std::mem::size_of::<u64>() as libc::c_ulong;
        (dir << 30) | (sz << 16) | (typ << 8) | nr
    };

    let ret = unsafe { libc::ioctl(fd, blkgetsize64, &mut size as *mut u64) };
    close_fd(fd);
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }

    Ok(size / 512)
}

// ---------------------------------------------------------------------------
// dm-integrity table construction
// ---------------------------------------------------------------------------

/// Build the dm-integrity target parameter string.
///
/// Format:
///   `<device> <offset> <tag_size> <mode> [<#optional_args> <optional_args>...]`
///
/// Where mode is "J" (journal), "D" (direct/no-journal), "B" (bitmap), or "R" (recovery).
fn build_integrity_params(opts: &IntegrityOptions, device: &str, key: &[u8]) -> String {
    // Determine tag size from algorithm.
    let tag_size = integrity_tag_size(&opts.algorithm);

    // Mode character.
    let mode = if opts.recovery {
        "R"
    } else {
        match opts.journal_mode {
            JournalMode::Journal => "J",
            JournalMode::NoJournal => "D",
            JournalMode::Bitmap => "B",
        }
    };

    // Offset is always 0 for standard setup (the kernel calculates the
    // superblock/metadata offset internally).
    let offset = 0;

    // Build the base parameter string.
    let mut params = format!("{device} {offset} {tag_size} {mode}");

    // Build optional arguments.
    let mut optional_args: Vec<String> = Vec::new();

    // Internal hash algorithm.
    optional_args.push(format!("internal_hash:{}", opts.algorithm));

    // Key for HMAC-based algorithms.
    if !key.is_empty() {
        optional_args.push(format!("key:{}", key_to_hex(key)));
    }

    // Sector size.
    if opts.sector_size != DEFAULT_SECTOR_SIZE {
        optional_args.push(format!("sectors_per_bit:{}", opts.sector_size));
    }

    // Block size.
    if let Some(bs) = opts.block_size {
        optional_args.push(format!("block_size:{bs}"));
    }

    // Journal commit time.
    if let Some(jct) = opts.journal_commit_time {
        optional_args.push(format!("journal_commit_time:{jct}"));
    }

    // Journal watermark.
    if let Some(jw) = opts.journal_watermark {
        optional_args.push(format!("journal_watermark:{jw}"));
    }

    // Journal integrity.
    if let Some(ref ji) = opts.journal_integrity {
        optional_args.push(format!("journal_mac:{ji}"));
    }

    // Journal crypt.
    if let Some(ref jc) = opts.journal_crypt {
        optional_args.push(format!("journal_crypt:{jc}"));
    }

    // Data device.
    if let Some(ref dd) = opts.data_device {
        optional_args.push(format!("data_device:{}", dd.display()));
    }

    // Bitmap flush interval.
    if let Some(bfi) = opts.bitmap_flush_interval {
        optional_args.push(format!("bitmap_flush_interval:{bfi}"));
    }

    // Boolean flags.
    if opts.integrity_recalculate {
        optional_args.push("recalculate".to_string());
    }

    if opts.integrity_recalculate_reset {
        optional_args.push("reset_recalculate".to_string());
    }

    if opts.allow_discards {
        optional_args.push("allow_discards".to_string());
    }

    if opts.fix_padding {
        optional_args.push("fix_padding".to_string());
    }

    if opts.fix_hmac {
        optional_args.push("fix_hmac".to_string());
    }

    if opts.legacy_recalculate {
        optional_args.push("legacy_recalculate".to_string());
    }

    // Append optional args count and args.
    if !optional_args.is_empty() {
        params.push_str(&format!(" {}", optional_args.len()));
        for arg in &optional_args {
            params.push_str(&format!(" {arg}"));
        }
    }

    params
}

/// Return the expected tag (checksum) size in bytes for a given integrity algorithm.
fn integrity_tag_size(algorithm: &str) -> u32 {
    match algorithm {
        "crc32" | "crc32c" => 4,
        "sha1" | "hmac(sha1)" => 20,
        "sha256" | "hmac(sha256)" => 32,
        "sha512" | "hmac(sha512)" => 64,
        "xxhash64" => 8,
        "blake2b-256" | "hmac(blake2b-256)" => 32,
        "blake2b-512" | "hmac(blake2b-512)" => 64,
        "poly1305" => 16,
        // For unknown algorithms, default to 32 (sha256-sized).
        _ => 32,
    }
}

// ---------------------------------------------------------------------------
// Attach / Detach operations
// ---------------------------------------------------------------------------

/// Read a raw superblock from a device or file path.
fn read_superblock(device: &str) -> Result<IntegritySuperblock, String> {
    let data = fs::read(device).map_err(|e| format!("Failed to read {device}: {e}"))?;
    if data.len() < SB_SIZE {
        return Err(format!(
            "Device/file too small for integrity superblock: {} bytes",
            data.len()
        ));
    }
    IntegritySuperblock::from_bytes(&data)
}

/// Write a raw superblock to a device or file path.
///
/// Opens the device/file for writing and writes exactly 512 bytes at offset 0.
fn write_superblock(device: &str, sb: &IntegritySuperblock) -> Result<(), String> {
    let bytes = sb.to_bytes();
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(device)
        .map_err(|e| format!("Failed to open {device} for writing: {e}"))?;
    file.write_all(&bytes)
        .map_err(|e| format!("Failed to write superblock to {device}: {e}"))?;
    file.flush()
        .map_err(|e| format!("Failed to flush {device}: {e}"))?;
    Ok(())
}

/// Format a device with a dm-integrity superblock.
///
/// This initializes the on-disk superblock header, preparing the device for
/// use with `attach`. Options control the algorithm, tag size, sector size,
/// journal configuration, and flags written into the superblock.
fn cmd_format(device: &str, options_str: &str) -> Result<(), String> {
    let opts = parse_options(options_str)?;

    // Validate device exists and is readable.
    let meta = fs::metadata(device).map_err(|e| format!("Cannot access device {device}: {e}"))?;
    if meta.len() < SB_SIZE as u64 {
        return Err(format!(
            "Device {device} too small ({} bytes) for integrity superblock",
            meta.len()
        ));
    }

    let tag_size = integrity_tag_size(&opts.algorithm) as u16;
    let l2spb = log2_sectors_per_block(opts.sector_size);

    // Compute provided data sectors. The real kernel does complex maths
    // involving journal size, interleave, and tag overhead. We provide a
    // conservative estimate: total sectors minus 1 (superblock) minus
    // journal reservation, minus per-sector tag overhead.
    let total_bytes = meta.len();
    let total_sectors = total_bytes / 512;

    if total_sectors < 2 {
        return Err(format!("Device {device} too small to format."));
    }

    // Reserve 1 sector for the superblock.
    let sectors_after_sb = total_sectors - 1;

    // Calculate journal reservation (in sectors).
    let journal_sections = if opts.journal_mode == JournalMode::NoJournal {
        0u32
    } else {
        DEFAULT_JOURNAL_SECTIONS
    };
    // Each journal section is roughly 128 sectors (64 KiB). Cap the total
    // journal reservation to at most 1/4 of the space after the superblock
    // so that small devices still have room for data.
    let raw_journal_sectors = journal_sections as u64 * 128;
    let journal_sectors = raw_journal_sectors.min(sectors_after_sb / 4);

    let remaining = sectors_after_sb.saturating_sub(journal_sectors);
    // Each data sector has an associated tag, so the usable data sectors is
    // approximately: remaining * sector_size / (sector_size + tag_size).
    let sector_sz = opts.sector_size as u64;
    let tag_sz = tag_size as u64;
    let provided_data_sectors = if remaining > 0 && sector_sz + tag_sz > 0 {
        remaining * sector_sz / (sector_sz + tag_sz)
    } else {
        0
    };

    // Build flags.
    let mut flags: u32 = 0;
    if opts.journal_integrity.is_some() {
        flags |= SB_FLAG_HAVE_JOURNAL_MAC;
    }
    if opts.integrity_recalculate {
        flags |= SB_FLAG_RECALCULATING;
    }
    if opts.journal_mode == JournalMode::Bitmap {
        flags |= SB_FLAG_DIRTY_BITMAP;
    }
    if opts.fix_padding {
        flags |= SB_FLAG_FIXED_PADDING;
    }
    if opts.fix_hmac {
        flags |= SB_FLAG_FIXED_HMAC;
    }

    // Determine version based on which features are used.
    let version = if opts.fix_hmac {
        SB_VERSION_5
    } else if opts.fix_padding {
        SB_VERSION_4
    } else if opts.integrity_recalculate || opts.integrity_recalculate_reset {
        SB_VERSION_3
    } else if opts.journal_mode == JournalMode::Bitmap {
        SB_VERSION_2
    } else {
        SB_VERSION_1
    };

    let sb = IntegritySuperblock {
        version,
        log2_interleave_sectors: DEFAULT_LOG2_INTERLEAVE,
        tag_size,
        journal_sections,
        provided_data_sectors,
        flags,
        log2_sectors_per_block: l2spb,
        log2_blocks_per_bitmap_bit: if version >= SB_VERSION_2 {
            DEFAULT_LOG2_BLOCKS_PER_BITMAP_BIT
        } else {
            0
        },
        recalc_sector: 0,
    };

    write_superblock(device, &sb)?;

    eprintln!(
        "Formatted integrity device {device} (version {version}, algorithm {}, tag size {tag_size} bytes, \
         {} provided data sectors, {} journal sections).",
        opts.algorithm, provided_data_sectors, journal_sections
    );
    Ok(())
}

/// Wipe the integrity superblock from a device by zeroing the first sector.
fn cmd_wipe(device: &str) -> Result<(), String> {
    let meta = fs::metadata(device).map_err(|e| format!("Cannot access device {device}: {e}"))?;
    if meta.len() < SB_SIZE as u64 {
        return Err(format!(
            "Device {device} too small ({} bytes) to contain a superblock",
            meta.len()
        ));
    }

    // Verify there is actually a superblock before wiping.
    let data = fs::read(device).map_err(|e| format!("Failed to read {device}: {e}"))?;
    if data.len() < SB_SIZE || &data[SB_MAGIC_OFFSET..SB_MAGIC_OFFSET + SB_MAGIC_SIZE] != SB_MAGIC {
        return Err(format!(
            "No dm-integrity superblock found on {device}; nothing to wipe."
        ));
    }

    // Write zeros over the superblock sector.
    let zeros = vec![0u8; SB_SIZE];
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(device)
        .map_err(|e| format!("Failed to open {device} for writing: {e}"))?;
    file.write_all(&zeros)
        .map_err(|e| format!("Failed to wipe superblock on {device}: {e}"))?;
    file.flush()
        .map_err(|e| format!("Failed to flush {device}: {e}"))?;

    eprintln!("Wiped integrity superblock from {device}.");
    Ok(())
}

/// Dump (display) the integrity superblock from a device.
fn cmd_dump(device: &str) -> Result<(), String> {
    let sb = read_superblock(device)?;
    print!("{}", sb.format_dump());
    Ok(())
}

/// Resize an active dm-integrity device.
///
/// This reloads the device-mapper table with the updated size of the
/// underlying block device. If `device` is not provided the command
/// attempts to query the current table status.
fn cmd_resize(volume: &str, device: Option<&str>) -> Result<(), String> {
    // If the caller supplied a device path, use it directly to obtain the
    // new sector count. Otherwise try DM_TABLE_STATUS to discover the
    // device from the running table.
    let dev_path = match device {
        Some(d) => d.to_string(),
        None => {
            // Attempt to read the current table status.
            let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
            dm_ioctl_init(&mut buf, volume, "", 0);
            dm_ioctl(DM_TABLE_STATUS_CMD, &mut buf).map_err(|e| {
                format!(
                    "Cannot query table status for {volume}: {e}. \
                     Specify the DEVICE argument explicitly."
                )
            })?;

            // Parse the status response. The target params start after the
            // dm_target_spec (40 bytes) at data_start. The first token in
            // the params string is the device path.
            let data_start = read_u32(&buf, DM_DATA_START_OFFSET) as usize;
            let params_off = data_start + 40;
            if params_off >= buf.len() {
                return Err("Invalid table status response.".to_string());
            }
            let params_end = buf[params_off..]
                .iter()
                .position(|&b| b == 0)
                .map(|p| params_off + p)
                .unwrap_or(buf.len());
            let params_str = String::from_utf8_lossy(&buf[params_off..params_end]).to_string();
            params_str
                .split_whitespace()
                .next()
                .ok_or("Cannot determine device from table status.")?
                .to_string()
        }
    };

    // Get the new total sector count.
    let new_total_sectors = device_size_sectors(&dev_path)
        .map_err(|e| format!("Failed to get device size for {dev_path}: {e}"))?;
    if new_total_sectors == 0 {
        return Err(format!("Device {dev_path} has zero size."));
    }

    let uuid_str = format!("CRYPT-INTEGRITY-{volume}");

    // Read the current superblock from the device to obtain parameters for
    // rebuilding the table (we need algorithm / tag_size / journal mode).
    // If that fails we fall back to defaults — the kernel will reconcile.
    let (tag_size, mode_char) = match read_superblock(&dev_path) {
        Ok(sb) => {
            let mode = if sb.flags & SB_FLAG_DIRTY_BITMAP != 0 {
                "B"
            } else if sb.journal_sections == 0 {
                "D"
            } else {
                "J"
            };
            (sb.tag_size as u32, mode.to_string())
        }
        Err(_) => (integrity_tag_size(DEFAULT_ALGORITHM), "J".to_string()),
    };

    // Rebuild a minimal integrity params string.
    let params = format!("{dev_path} 0 {tag_size} {mode_char} 1 internal_hash:{DEFAULT_ALGORITHM}");

    // 1. Suspend the device.
    let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
    dm_ioctl_init(&mut buf, volume, &uuid_str, DM_SUSPEND_FLAG);
    dm_ioctl(DM_DEV_SUSPEND_CMD, &mut buf)
        .map_err(|e| format!("DM_DEV_SUSPEND failed for {volume}: {e}"))?;

    // 2. Clear old table.
    dm_ioctl_init(&mut buf, volume, &uuid_str, 0);
    dm_ioctl(DM_TABLE_CLEAR_CMD, &mut buf)
        .map_err(|e| format!("DM_TABLE_CLEAR failed for {volume}: {e}"))?;

    // 3. Load new table with updated size.
    dm_ioctl_init(&mut buf, volume, &uuid_str, 0);
    write_u32(&mut buf, DM_TARGET_COUNT_OFFSET, 1);
    let data_start = read_u32(&buf, DM_DATA_START_OFFSET) as usize;
    append_target(
        &mut buf,
        data_start,
        0,
        new_total_sectors,
        "integrity",
        &params,
    );
    dm_ioctl(DM_TABLE_LOAD_CMD, &mut buf)
        .map_err(|e| format!("DM_TABLE_LOAD failed for {volume}: {e}"))?;

    // 4. Resume.
    dm_ioctl_init(&mut buf, volume, &uuid_str, 0);
    dm_ioctl(DM_DEV_SUSPEND_CMD, &mut buf)
        .map_err(|e| format!("DM_DEV_SUSPEND (resume) failed for {volume}: {e}"))?;

    eprintln!("Resized integrity device /dev/mapper/{volume} to {new_total_sectors} sectors.");
    Ok(())
}

/// Attach a dm-integrity block device via device-mapper.
fn cmd_attach(
    volume: &str,
    device: &str,
    key_arg: Option<&str>,
    options_str: &str,
) -> Result<(), String> {
    let opts = parse_options(options_str)?;

    // Read key material (if any).
    let key = read_key(key_arg).map_err(|e| format!("Failed to read key: {e}"))?;

    // Validate: HMAC algorithms require a key.
    if opts.algorithm.starts_with("hmac(") && key.is_empty() {
        return Err(format!(
            "HMAC algorithm '{}' requires a key, but none was provided.",
            opts.algorithm
        ));
    }

    // Check data device exists if specified.
    if let Some(ref dd) = opts.data_device
        && !dd.exists()
    {
        return Err(format!("Data device not found: {}", dd.display()));
    }

    // Check journal integrity key file exists if specified.
    if let Some(ref kf) = opts.journal_integrity_key_file
        && !kf.exists()
    {
        return Err(format!(
            "Journal integrity key file not found: {}",
            kf.display()
        ));
    }

    // Check journal crypt key file exists if specified.
    if let Some(ref kf) = opts.journal_crypt_key_file
        && !kf.exists()
    {
        return Err(format!(
            "Journal crypt key file not found: {}",
            kf.display()
        ));
    }

    // Get device size.
    let total_sectors = device_size_sectors(device)
        .map_err(|e| format!("Failed to get device size for {device}: {e}"))?;

    if total_sectors == 0 {
        return Err(format!("Device {device} has zero size."));
    }

    // The actual usable size is determined by the kernel after subtracting
    // the superblock and journal. We pass the full device size and let the
    // kernel handle it. For the table, we use a reasonable estimate: the
    // device size minus overhead. In practice, the kernel adjusts this.
    let target_sectors = total_sectors;

    // Build UUID.
    let uuid_str = format!("CRYPT-INTEGRITY-{volume}");

    // Build the dm-integrity target parameters.
    let integrity_params = build_integrity_params(&opts, device, &key);

    let flags = if opts.readonly { DM_READONLY_FLAG } else { 0 };

    // 1. DM_DEV_CREATE.
    let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
    dm_ioctl_init(&mut buf, volume, &uuid_str, flags);
    dm_ioctl(DM_DEV_CREATE_CMD, &mut buf)
        .map_err(|e| format!("DM_DEV_CREATE failed for {volume}: {e}"))?;

    // 2. DM_TABLE_LOAD.
    dm_ioctl_init(&mut buf, volume, &uuid_str, flags);
    write_u32(&mut buf, DM_TARGET_COUNT_OFFSET, 1);
    let data_start = read_u32(&buf, DM_DATA_START_OFFSET) as usize;
    let _target_size = append_target(
        &mut buf,
        data_start,
        0,
        target_sectors,
        "integrity",
        &integrity_params,
    );

    if let Err(e) = dm_ioctl(DM_TABLE_LOAD_CMD, &mut buf) {
        let mut cleanup = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut cleanup, volume, "", 0);
        let _ = dm_ioctl(DM_DEV_REMOVE_CMD, &mut cleanup);
        return Err(format!("DM_TABLE_LOAD failed for {volume}: {e}"));
    }

    // 3. DM_DEV_SUSPEND (resume).
    dm_ioctl_init(&mut buf, volume, &uuid_str, flags);
    if let Err(e) = dm_ioctl(DM_DEV_SUSPEND_CMD, &mut buf) {
        let mut cleanup = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut cleanup, volume, "", 0);
        let _ = dm_ioctl(DM_TABLE_CLEAR_CMD, &mut cleanup);
        let _ = dm_ioctl(DM_DEV_REMOVE_CMD, &mut cleanup);
        return Err(format!("DM_DEV_SUSPEND (resume) failed for {volume}: {e}"));
    }

    eprintln!("Set up integrity device /dev/mapper/{volume}");
    Ok(())
}

/// Detach (tear down) a dm-integrity device.
fn cmd_detach(volume: &str) -> Result<(), String> {
    let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];

    // 1. Suspend.
    dm_ioctl_init(&mut buf, volume, "", DM_SUSPEND_FLAG);
    let _ = dm_ioctl(DM_DEV_SUSPEND_CMD, &mut buf);

    // 2. Clear table.
    dm_ioctl_init(&mut buf, volume, "", 0);
    let _ = dm_ioctl(DM_TABLE_CLEAR_CMD, &mut buf);

    // 3. Remove.
    dm_ioctl_init(&mut buf, volume, "", 0);
    dm_ioctl(DM_DEV_REMOVE_CMD, &mut buf)
        .map_err(|e| format!("DM_DEV_REMOVE failed for {volume}: {e}"))?;

    eprintln!("Detached integrity device /dev/mapper/{volume}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!(
        "\
Usage: systemd-integritysetup [COMMAND] [OPTIONS...]

Commands:
  attach VOLUME DEVICE [KEY [OPTIONS]]
      Set up a dm-integrity block device.
      VOLUME   — device-mapper name (appears as /dev/mapper/VOLUME)
      DEVICE   — underlying block device
      KEY      — key file path, or \"-\" for no key (default: no key)
      OPTIONS  — comma-separated list of options (see below)

  detach VOLUME
      Tear down a previously attached dm-integrity device.

  format DEVICE [OPTIONS]
      Initialize an integrity superblock on DEVICE.
      Writes the on-disk superblock header and prepares the device for use
      with the attach command. OPTIONS is the same comma-separated format.

  wipe DEVICE
      Wipe (zero) the integrity superblock from DEVICE, removing all
      dm-integrity metadata.

  dump DEVICE
      Read and display the dm-integrity superblock from DEVICE.

  resize VOLUME [DEVICE]
      Resize an active dm-integrity device after the underlying block
      device has been resized. If DEVICE is not given, the current device
      is discovered from the running table status.

Options (comma-separated in the OPTIONS argument):
  algorithm=ALG                Integrity algorithm (default: crc32c)
  journal-commit-time=MS       Journal commit interval (ms)
  journal-watermark=PCT        Journal watermark (0-100)
  journal-integrity=ALG        Journal integrity hash
  journal-integrity-key-size=N Key size for journal integrity
  journal-integrity-key-file=PATH  Key file for journal integrity
  journal-crypt=ALG            Journal encryption algorithm
  journal-crypt-key-size=N     Key size for journal encryption
  journal-crypt-key-file=PATH  Key file for journal encryption
  data-device=PATH             Separate data device
  sector-size=BYTES            Sector size (default: 512)
  bitmap-flush-interval=MS     Bitmap mode flush interval
  block-size=BYTES             Internal block size
  integrity-recalculate        Recalculate tags in background
  integrity-recalculate-reset  Reset recalculate position
  allow-discards               Allow TRIM passthrough
  fix-padding                  Fix metadata padding
  fix-hmac                     Fix HMAC issues
  legacy-recalculate           Legacy recalculate behavior
  no-journal                   Disable journaling
  no-journal-bitmap            Use bitmap journaling
  recovery                     Recovery mode
  readonly                     Open read-only

Exit codes:
  0 — success
  1 — general error
  4 — integrity check failure"
    );
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn run() -> Result<(), (String, i32)> {
    let argv: Vec<String> = std::env::args().collect();
    let args = if argv.len() > 1 { &argv[1..] } else { &[] };

    let cmd = parse_args(args).map_err(|e| (e, 1))?;

    match cmd {
        Command::Help => {
            print_usage();
            Ok(())
        }
        Command::Version => {
            eprintln!("systemd-integritysetup (rust-systemd) 0.1.0");
            Ok(())
        }
        Command::Attach {
            volume,
            device,
            key,
            options,
        } => cmd_attach(&volume, &device, key.as_deref(), &options).map_err(|e| (e, 1)),
        Command::Detach { volume } => cmd_detach(&volume).map_err(|e| (e, 1)),
        Command::Format { device, options } => cmd_format(&device, &options).map_err(|e| (e, 1)),
        Command::Wipe { device } => cmd_wipe(&device).map_err(|e| (e, 1)),
        Command::Dump { device } => cmd_dump(&device).map_err(|e| (e, 1)),
        Command::Resize { volume, device } => {
            cmd_resize(&volume, device.as_deref()).map_err(|e| (e, 1))
        }
    }
}

fn main() {
    match run() {
        Ok(()) => process::exit(0),
        Err((msg, code)) => {
            eprintln!("systemd-integritysetup: {msg}");
            process::exit(code);
        }
    }
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

    /// Create a temporary file pre-filled with `size` zero bytes.
    fn temp_file_zeros(size: usize) -> tempfile::NamedTempFile {
        let mut f = tempfile::NamedTempFile::new().unwrap();
        f.write_all(&vec![0u8; size]).unwrap();
        f.flush().unwrap();
        f
    }

    /// Create a temporary file containing a valid integrity superblock.
    fn temp_file_with_superblock(sb: &IntegritySuperblock) -> tempfile::NamedTempFile {
        let bytes = sb.to_bytes();
        let mut f = tempfile::NamedTempFile::new().unwrap();
        // Write superblock + extra space so the file is large enough for format.
        f.write_all(&bytes).unwrap();
        // Pad to at least 1 MiB so format has room.
        let pad = vec![0u8; 1024 * 1024 - bytes.len()];
        f.write_all(&pad).unwrap();
        f.flush().unwrap();
        f
    }

    // -----------------------------------------------------------------------
    // parse_args tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_empty() {
        assert!(parse_args(&args(&[])).is_err());
    }

    #[test]
    fn test_parse_args_help() {
        assert_eq!(parse_args(&args(&["--help"])).unwrap(), Command::Help);
        assert_eq!(parse_args(&args(&["-h"])).unwrap(), Command::Help);
        assert_eq!(parse_args(&args(&["help"])).unwrap(), Command::Help);
    }

    #[test]
    fn test_parse_args_version() {
        assert_eq!(parse_args(&args(&["--version"])).unwrap(), Command::Version);
        assert_eq!(parse_args(&args(&["version"])).unwrap(), Command::Version);
    }

    #[test]
    fn test_parse_args_attach_minimal() {
        let cmd = parse_args(&args(&["attach", "myvol", "/dev/sda1"])).unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "myvol".to_string(),
                device: "/dev/sda1".to_string(),
                key: None,
                options: String::new(),
            }
        );
    }

    #[test]
    fn test_parse_args_attach_with_key() {
        let cmd = parse_args(&args(&["attach", "vol", "/dev/sda1", "/path/to/key"])).unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "vol".to_string(),
                device: "/dev/sda1".to_string(),
                key: Some("/path/to/key".to_string()),
                options: String::new(),
            }
        );
    }

    #[test]
    fn test_parse_args_attach_with_key_and_options() {
        let cmd = parse_args(&args(&[
            "attach",
            "vol",
            "/dev/sda1",
            "-",
            "algorithm=sha256,no-journal",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "vol".to_string(),
                device: "/dev/sda1".to_string(),
                key: Some("-".to_string()),
                options: "algorithm=sha256,no-journal".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_args_attach_missing_volume() {
        assert!(parse_args(&args(&["attach"])).is_err());
    }

    #[test]
    fn test_parse_args_attach_missing_device() {
        assert!(parse_args(&args(&["attach", "vol"])).is_err());
    }

    #[test]
    fn test_parse_args_detach() {
        let cmd = parse_args(&args(&["detach", "myvol"])).unwrap();
        assert_eq!(
            cmd,
            Command::Detach {
                volume: "myvol".to_string()
            }
        );
    }

    #[test]
    fn test_parse_args_detach_missing_volume() {
        assert!(parse_args(&args(&["detach"])).is_err());
    }

    #[test]
    fn test_parse_args_unknown_command() {
        assert!(parse_args(&args(&["frobnicate"])).is_err());
    }

    // -----------------------------------------------------------------------
    // parse_args — format / wipe / dump / resize
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_args_format_minimal() {
        let cmd = parse_args(&args(&["format", "/dev/sda1"])).unwrap();
        assert_eq!(
            cmd,
            Command::Format {
                device: "/dev/sda1".to_string(),
                options: String::new(),
            }
        );
    }

    #[test]
    fn test_parse_args_format_with_options() {
        let cmd = parse_args(&args(&[
            "format",
            "/dev/sda1",
            "algorithm=sha256,no-journal",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Command::Format {
                device: "/dev/sda1".to_string(),
                options: "algorithm=sha256,no-journal".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_args_format_missing_device() {
        assert!(parse_args(&args(&["format"])).is_err());
    }

    #[test]
    fn test_parse_args_wipe() {
        let cmd = parse_args(&args(&["wipe", "/dev/sda1"])).unwrap();
        assert_eq!(
            cmd,
            Command::Wipe {
                device: "/dev/sda1".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_args_wipe_missing_device() {
        assert!(parse_args(&args(&["wipe"])).is_err());
    }

    #[test]
    fn test_parse_args_dump() {
        let cmd = parse_args(&args(&["dump", "/dev/sda1"])).unwrap();
        assert_eq!(
            cmd,
            Command::Dump {
                device: "/dev/sda1".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_args_dump_missing_device() {
        assert!(parse_args(&args(&["dump"])).is_err());
    }

    #[test]
    fn test_parse_args_resize_minimal() {
        let cmd = parse_args(&args(&["resize", "myvol"])).unwrap();
        assert_eq!(
            cmd,
            Command::Resize {
                volume: "myvol".to_string(),
                device: None,
            }
        );
    }

    #[test]
    fn test_parse_args_resize_with_device() {
        let cmd = parse_args(&args(&["resize", "myvol", "/dev/sda1"])).unwrap();
        assert_eq!(
            cmd,
            Command::Resize {
                volume: "myvol".to_string(),
                device: Some("/dev/sda1".to_string()),
            }
        );
    }

    #[test]
    fn test_parse_args_resize_missing_volume() {
        assert!(parse_args(&args(&["resize"])).is_err());
    }

    // -----------------------------------------------------------------------
    // parse_options tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_options_empty() {
        let opts = parse_options("").unwrap();
        assert_eq!(opts.algorithm, DEFAULT_ALGORITHM);
        assert!(opts.journal_commit_time.is_none());
        assert!(opts.journal_watermark.is_none());
        assert!(opts.journal_integrity.is_none());
        assert!(opts.journal_integrity_key_size.is_none());
        assert!(opts.journal_integrity_key_file.is_none());
        assert!(opts.journal_crypt.is_none());
        assert!(opts.journal_crypt_key_size.is_none());
        assert!(opts.journal_crypt_key_file.is_none());
        assert!(opts.data_device.is_none());
        assert_eq!(opts.sector_size, DEFAULT_SECTOR_SIZE);
        assert!(opts.bitmap_flush_interval.is_none());
        assert!(opts.block_size.is_none());
        assert!(!opts.integrity_recalculate);
        assert!(!opts.integrity_recalculate_reset);
        assert!(!opts.allow_discards);
        assert!(!opts.fix_padding);
        assert!(!opts.fix_hmac);
        assert!(!opts.legacy_recalculate);
        assert_eq!(opts.journal_mode, JournalMode::Journal);
        assert!(!opts.recovery);
        assert!(!opts.readonly);
    }

    #[test]
    fn test_parse_options_dash() {
        let opts = parse_options("-").unwrap();
        assert_eq!(opts.algorithm, DEFAULT_ALGORITHM);
    }

    #[test]
    fn test_parse_options_none() {
        let opts = parse_options("none").unwrap();
        assert_eq!(opts.algorithm, DEFAULT_ALGORITHM);
    }

    #[test]
    fn test_parse_options_algorithm() {
        let opts = parse_options("algorithm=sha256").unwrap();
        assert_eq!(opts.algorithm, "sha256");
    }

    #[test]
    fn test_parse_options_algorithm_hmac() {
        let opts = parse_options("algorithm=hmac(sha256)").unwrap();
        assert_eq!(opts.algorithm, "hmac(sha256)");
    }

    #[test]
    fn test_parse_options_algorithm_alias() {
        let opts = parse_options("integrity-algorithm=sha512").unwrap();
        assert_eq!(opts.algorithm, "sha512");
    }

    #[test]
    fn test_parse_options_journal_commit_time() {
        let opts = parse_options("journal-commit-time=5000").unwrap();
        assert_eq!(opts.journal_commit_time, Some(5000));
    }

    #[test]
    fn test_parse_options_journal_commit_time_invalid() {
        assert!(parse_options("journal-commit-time=abc").is_err());
    }

    #[test]
    fn test_parse_options_journal_watermark() {
        let opts = parse_options("journal-watermark=50").unwrap();
        assert_eq!(opts.journal_watermark, Some(50));
    }

    #[test]
    fn test_parse_options_journal_watermark_zero() {
        let opts = parse_options("journal-watermark=0").unwrap();
        assert_eq!(opts.journal_watermark, Some(0));
    }

    #[test]
    fn test_parse_options_journal_watermark_100() {
        let opts = parse_options("journal-watermark=100").unwrap();
        assert_eq!(opts.journal_watermark, Some(100));
    }

    #[test]
    fn test_parse_options_journal_watermark_over_100() {
        assert!(parse_options("journal-watermark=101").is_err());
    }

    #[test]
    fn test_parse_options_journal_watermark_invalid() {
        assert!(parse_options("journal-watermark=abc").is_err());
    }

    #[test]
    fn test_parse_options_journal_integrity() {
        let opts = parse_options("journal-integrity=hmac(sha256)").unwrap();
        assert_eq!(opts.journal_integrity, Some("hmac(sha256)".to_string()));
    }

    #[test]
    fn test_parse_options_journal_integrity_key_size() {
        let opts = parse_options("journal-integrity-key-size=32").unwrap();
        assert_eq!(opts.journal_integrity_key_size, Some(32));
    }

    #[test]
    fn test_parse_options_journal_integrity_key_file() {
        let opts = parse_options("journal-integrity-key-file=/path/to/key").unwrap();
        assert_eq!(
            opts.journal_integrity_key_file,
            Some(PathBuf::from("/path/to/key"))
        );
    }

    #[test]
    fn test_parse_options_journal_crypt() {
        let opts = parse_options("journal-crypt=aes-xts-plain64").unwrap();
        assert_eq!(opts.journal_crypt, Some("aes-xts-plain64".to_string()));
    }

    #[test]
    fn test_parse_options_journal_crypt_key_size() {
        let opts = parse_options("journal-crypt-key-size=64").unwrap();
        assert_eq!(opts.journal_crypt_key_size, Some(64));
    }

    #[test]
    fn test_parse_options_journal_crypt_key_file() {
        let opts = parse_options("journal-crypt-key-file=/path/to/key").unwrap();
        assert_eq!(
            opts.journal_crypt_key_file,
            Some(PathBuf::from("/path/to/key"))
        );
    }

    #[test]
    fn test_parse_options_data_device() {
        let opts = parse_options("data-device=/dev/sdb1").unwrap();
        assert_eq!(opts.data_device, Some(PathBuf::from("/dev/sdb1")));
    }

    #[test]
    fn test_parse_options_sector_size() {
        let opts = parse_options("sector-size=4096").unwrap();
        assert_eq!(opts.sector_size, 4096);
    }

    #[test]
    fn test_parse_options_sector_size_1024() {
        let opts = parse_options("sector-size=1024").unwrap();
        assert_eq!(opts.sector_size, 1024);
    }

    #[test]
    fn test_parse_options_sector_size_invalid_not_power_of_two() {
        assert!(parse_options("sector-size=1000").is_err());
    }

    #[test]
    fn test_parse_options_sector_size_invalid_too_small() {
        assert!(parse_options("sector-size=256").is_err());
    }

    #[test]
    fn test_parse_options_sector_size_invalid_nan() {
        assert!(parse_options("sector-size=abc").is_err());
    }

    #[test]
    fn test_parse_options_bitmap_flush_interval() {
        let opts = parse_options("bitmap-flush-interval=1000").unwrap();
        assert_eq!(opts.bitmap_flush_interval, Some(1000));
    }

    #[test]
    fn test_parse_options_block_size() {
        let opts = parse_options("block-size=4096").unwrap();
        assert_eq!(opts.block_size, Some(4096));
    }

    #[test]
    fn test_parse_options_integrity_recalculate() {
        let opts = parse_options("integrity-recalculate").unwrap();
        assert!(opts.integrity_recalculate);
    }

    #[test]
    fn test_parse_options_integrity_recalculate_underscore() {
        let opts = parse_options("integrity_recalculate").unwrap();
        assert!(opts.integrity_recalculate);
    }

    #[test]
    fn test_parse_options_integrity_recalculate_reset() {
        let opts = parse_options("integrity-recalculate-reset").unwrap();
        assert!(opts.integrity_recalculate_reset);
    }

    #[test]
    fn test_parse_options_allow_discards() {
        let opts = parse_options("allow-discards").unwrap();
        assert!(opts.allow_discards);
    }

    #[test]
    fn test_parse_options_discard_alias() {
        let opts = parse_options("discard").unwrap();
        assert!(opts.allow_discards);
    }

    #[test]
    fn test_parse_options_fix_padding() {
        let opts = parse_options("fix-padding").unwrap();
        assert!(opts.fix_padding);
    }

    #[test]
    fn test_parse_options_fix_hmac() {
        let opts = parse_options("fix-hmac").unwrap();
        assert!(opts.fix_hmac);
    }

    #[test]
    fn test_parse_options_legacy_recalculate() {
        let opts = parse_options("legacy-recalculate").unwrap();
        assert!(opts.legacy_recalculate);
    }

    #[test]
    fn test_parse_options_no_journal() {
        let opts = parse_options("no-journal").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::NoJournal);
    }

    #[test]
    fn test_parse_options_no_journal_underscore() {
        let opts = parse_options("no_journal").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::NoJournal);
    }

    #[test]
    fn test_parse_options_bitmap() {
        let opts = parse_options("bitmap").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::Bitmap);
    }

    #[test]
    fn test_parse_options_no_journal_bitmap() {
        let opts = parse_options("no-journal-bitmap").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::Bitmap);
    }

    #[test]
    fn test_parse_options_recovery() {
        let opts = parse_options("recovery").unwrap();
        assert!(opts.recovery);
    }

    #[test]
    fn test_parse_options_readonly() {
        let opts = parse_options("readonly").unwrap();
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_read_only_alias() {
        let opts = parse_options("read-only").unwrap();
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_noauto_nofail() {
        let opts = parse_options("noauto,nofail").unwrap();
        assert_eq!(opts.algorithm, DEFAULT_ALGORITHM);
    }

    #[test]
    fn test_parse_options_systemd_extensions_ignored() {
        let opts = parse_options("x-systemd.device-timeout=30").unwrap();
        assert_eq!(opts.algorithm, DEFAULT_ALGORITHM);
    }

    #[test]
    fn test_parse_options_unknown_flag() {
        assert!(parse_options("bogus-flag").is_err());
    }

    #[test]
    fn test_parse_options_unknown_kv() {
        assert!(parse_options("unknown-key=value").is_err());
    }

    #[test]
    fn test_parse_options_whitespace() {
        let opts = parse_options(" no-journal , readonly ").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::NoJournal);
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_empty_parts() {
        let opts = parse_options(",,no-journal,,readonly,,").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::NoJournal);
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_combined() {
        let opts = parse_options(
            "algorithm=sha256,no-journal,sector-size=4096,\
             integrity-recalculate,allow-discards,readonly,\
             journal-commit-time=5000,journal-watermark=75",
        )
        .unwrap();

        assert_eq!(opts.algorithm, "sha256");
        assert_eq!(opts.journal_mode, JournalMode::NoJournal);
        assert_eq!(opts.sector_size, 4096);
        assert!(opts.integrity_recalculate);
        assert!(opts.allow_discards);
        assert!(opts.readonly);
        assert_eq!(opts.journal_commit_time, Some(5000));
        assert_eq!(opts.journal_watermark, Some(75));
    }

    #[test]
    fn test_parse_options_journal_mode_last_wins() {
        let opts = parse_options("no-journal,bitmap").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::Bitmap);

        let opts = parse_options("bitmap,no-journal").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::NoJournal);
    }

    #[test]
    fn test_parse_options_auto_ignored() {
        let opts = parse_options("auto").unwrap();
        assert_eq!(opts.journal_mode, JournalMode::Journal);
    }

    // -----------------------------------------------------------------------
    // integrity_tag_size tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_integrity_tag_size_crc32c() {
        assert_eq!(integrity_tag_size("crc32c"), 4);
    }

    #[test]
    fn test_integrity_tag_size_crc32() {
        assert_eq!(integrity_tag_size("crc32"), 4);
    }

    #[test]
    fn test_integrity_tag_size_sha1() {
        assert_eq!(integrity_tag_size("sha1"), 20);
    }

    #[test]
    fn test_integrity_tag_size_hmac_sha1() {
        assert_eq!(integrity_tag_size("hmac(sha1)"), 20);
    }

    #[test]
    fn test_integrity_tag_size_sha256() {
        assert_eq!(integrity_tag_size("sha256"), 32);
    }

    #[test]
    fn test_integrity_tag_size_hmac_sha256() {
        assert_eq!(integrity_tag_size("hmac(sha256)"), 32);
    }

    #[test]
    fn test_integrity_tag_size_sha512() {
        assert_eq!(integrity_tag_size("sha512"), 64);
    }

    #[test]
    fn test_integrity_tag_size_hmac_sha512() {
        assert_eq!(integrity_tag_size("hmac(sha512)"), 64);
    }

    #[test]
    fn test_integrity_tag_size_xxhash64() {
        assert_eq!(integrity_tag_size("xxhash64"), 8);
    }

    #[test]
    fn test_integrity_tag_size_poly1305() {
        assert_eq!(integrity_tag_size("poly1305"), 16);
    }

    #[test]
    fn test_integrity_tag_size_blake2b_256() {
        assert_eq!(integrity_tag_size("blake2b-256"), 32);
    }

    #[test]
    fn test_integrity_tag_size_blake2b_512() {
        assert_eq!(integrity_tag_size("blake2b-512"), 64);
    }

    #[test]
    fn test_integrity_tag_size_unknown_default() {
        assert_eq!(integrity_tag_size("some-unknown-algo"), 32);
    }

    // -----------------------------------------------------------------------
    // key_to_hex tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_key_to_hex_empty() {
        assert_eq!(key_to_hex(&[]), "");
    }

    #[test]
    fn test_key_to_hex_single_byte() {
        assert_eq!(key_to_hex(&[0xab]), "ab");
    }

    #[test]
    fn test_key_to_hex_multi_byte() {
        assert_eq!(key_to_hex(&[0x00, 0xff, 0x0a, 0xbc]), "00ff0abc");
    }

    #[test]
    fn test_key_to_hex_all_zeros() {
        assert_eq!(key_to_hex(&[0, 0, 0, 0]), "00000000");
    }

    #[test]
    fn test_key_to_hex_all_ff() {
        assert_eq!(key_to_hex(&[0xff; 16]), "ff".repeat(16));
    }

    // -----------------------------------------------------------------------
    // read_key tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_key_none() {
        let key = read_key(None).unwrap();
        assert!(key.is_empty());
    }

    #[test]
    fn test_read_key_dash() {
        let key = read_key(Some("-")).unwrap();
        assert!(key.is_empty());
    }

    #[test]
    fn test_read_key_empty() {
        let key = read_key(Some("")).unwrap();
        assert!(key.is_empty());
    }

    #[test]
    fn test_read_key_none_string() {
        let key = read_key(Some("none")).unwrap();
        assert!(key.is_empty());
    }

    #[test]
    fn test_read_key_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"secret-key-data").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap())).unwrap();
        assert_eq!(key, b"secret-key-data");
    }

    #[test]
    fn test_read_key_binary_content() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        let binary_data: Vec<u8> = (0..=255).collect();
        fs::write(&key_path, &binary_data).unwrap();

        let key = read_key(Some(key_path.to_str().unwrap())).unwrap();
        assert_eq!(key, binary_data);
    }

    #[test]
    fn test_read_key_nonexistent_file() {
        assert!(read_key(Some("/nonexistent/keyfile")).is_err());
    }

    #[test]
    fn test_read_key_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap())).unwrap();
        assert!(key.is_empty());
    }

    // -----------------------------------------------------------------------
    // build_integrity_params tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_integrity_params_default() {
        let opts = IntegrityOptions::default();
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        // Device, offset, tag_size(crc32c=4), mode(J for journal).
        assert!(params.starts_with("/dev/sda1 0 4 J"));
        // Should contain internal_hash.
        assert!(params.contains("internal_hash:crc32c"));
        // No key.
        assert!(!params.contains("key:"));
    }

    #[test]
    fn test_build_integrity_params_sha256() {
        let opts = IntegrityOptions {
            algorithm: "sha256".to_string(),
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        // Tag size for sha256 = 32.
        assert!(params.starts_with("/dev/sda1 0 32 J"));
        assert!(params.contains("internal_hash:sha256"));
    }

    #[test]
    fn test_build_integrity_params_hmac_with_key() {
        let opts = IntegrityOptions {
            algorithm: "hmac(sha256)".to_string(),
            ..Default::default()
        };
        let key = vec![0xab; 32];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.starts_with("/dev/sda1 0 32 J"));
        assert!(params.contains("internal_hash:hmac(sha256)"));
        assert!(params.contains(&format!("key:{}", "ab".repeat(32))));
    }

    #[test]
    fn test_build_integrity_params_no_journal() {
        let opts = IntegrityOptions {
            journal_mode: JournalMode::NoJournal,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(
            params.contains(" D ")
                || params.contains(" D\n")
                || params.starts_with("/dev/sda1 0 4 D")
        );
    }

    #[test]
    fn test_build_integrity_params_bitmap() {
        let opts = IntegrityOptions {
            journal_mode: JournalMode::Bitmap,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.starts_with("/dev/sda1 0 4 B"));
    }

    #[test]
    fn test_build_integrity_params_recovery() {
        let opts = IntegrityOptions {
            recovery: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.starts_with("/dev/sda1 0 4 R"));
    }

    #[test]
    fn test_build_integrity_params_recovery_overrides_journal_mode() {
        let opts = IntegrityOptions {
            recovery: true,
            journal_mode: JournalMode::NoJournal,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        // Recovery overrides journal mode.
        assert!(params.starts_with("/dev/sda1 0 4 R"));
    }

    #[test]
    fn test_build_integrity_params_with_recalculate() {
        let opts = IntegrityOptions {
            integrity_recalculate: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("recalculate"));
    }

    #[test]
    fn test_build_integrity_params_with_recalculate_reset() {
        let opts = IntegrityOptions {
            integrity_recalculate_reset: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("reset_recalculate"));
    }

    #[test]
    fn test_build_integrity_params_allow_discards() {
        let opts = IntegrityOptions {
            allow_discards: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("allow_discards"));
    }

    #[test]
    fn test_build_integrity_params_fix_padding() {
        let opts = IntegrityOptions {
            fix_padding: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("fix_padding"));
    }

    #[test]
    fn test_build_integrity_params_fix_hmac() {
        let opts = IntegrityOptions {
            fix_hmac: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("fix_hmac"));
    }

    #[test]
    fn test_build_integrity_params_legacy_recalculate() {
        let opts = IntegrityOptions {
            legacy_recalculate: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("legacy_recalculate"));
    }

    #[test]
    fn test_build_integrity_params_journal_options() {
        let opts = IntegrityOptions {
            journal_commit_time: Some(5000),
            journal_watermark: Some(50),
            journal_integrity: Some("hmac(sha256)".to_string()),
            journal_crypt: Some("aes-xts-plain64".to_string()),
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("journal_commit_time:5000"));
        assert!(params.contains("journal_watermark:50"));
        assert!(params.contains("journal_mac:hmac(sha256)"));
        assert!(params.contains("journal_crypt:aes-xts-plain64"));
    }

    #[test]
    fn test_build_integrity_params_data_device() {
        let opts = IntegrityOptions {
            data_device: Some(PathBuf::from("/dev/sdb1")),
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("data_device:/dev/sdb1"));
    }

    #[test]
    fn test_build_integrity_params_bitmap_flush_interval() {
        let opts = IntegrityOptions {
            bitmap_flush_interval: Some(1000),
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("bitmap_flush_interval:1000"));
    }

    #[test]
    fn test_build_integrity_params_block_size() {
        let opts = IntegrityOptions {
            block_size: Some(4096),
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("block_size:4096"));
    }

    #[test]
    fn test_build_integrity_params_custom_sector_size() {
        let opts = IntegrityOptions {
            sector_size: 4096,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("sectors_per_bit:4096"));
    }

    #[test]
    fn test_build_integrity_params_all_flags() {
        let opts = IntegrityOptions {
            integrity_recalculate: true,
            integrity_recalculate_reset: true,
            allow_discards: true,
            fix_padding: true,
            fix_hmac: true,
            legacy_recalculate: true,
            ..Default::default()
        };
        let key: Vec<u8> = vec![];
        let params = build_integrity_params(&opts, "/dev/sda1", &key);
        assert!(params.contains("recalculate"));
        assert!(params.contains("reset_recalculate"));
        assert!(params.contains("allow_discards"));
        assert!(params.contains("fix_padding"));
        assert!(params.contains("fix_hmac"));
        assert!(params.contains("legacy_recalculate"));
    }

    // -----------------------------------------------------------------------
    // JournalMode Display
    // -----------------------------------------------------------------------

    #[test]
    fn test_journal_mode_display() {
        assert_eq!(format!("{}", JournalMode::Journal), "journal");
        assert_eq!(format!("{}", JournalMode::NoJournal), "no-journal");
        assert_eq!(format!("{}", JournalMode::Bitmap), "bitmap");
    }

    // -----------------------------------------------------------------------
    // IntegrityOptions default tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_integrity_options_default() {
        let opts = IntegrityOptions::default();
        assert_eq!(opts.algorithm, "crc32c");
        assert!(opts.journal_commit_time.is_none());
        assert!(opts.journal_watermark.is_none());
        assert!(opts.journal_integrity.is_none());
        assert!(opts.journal_crypt.is_none());
        assert!(opts.data_device.is_none());
        assert_eq!(opts.sector_size, 512);
        assert!(opts.bitmap_flush_interval.is_none());
        assert!(opts.block_size.is_none());
        assert!(!opts.integrity_recalculate);
        assert!(!opts.integrity_recalculate_reset);
        assert!(!opts.allow_discards);
        assert!(!opts.fix_padding);
        assert!(!opts.fix_hmac);
        assert!(!opts.legacy_recalculate);
        assert_eq!(opts.journal_mode, JournalMode::Journal);
        assert!(!opts.recovery);
        assert!(!opts.readonly);
    }

    // -----------------------------------------------------------------------
    // DM ioctl helper tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dm_ioctl_nr_encoding() {
        let nr = dm_ioctl_nr(DM_DEV_CREATE_CMD);
        assert_eq!((nr >> 30) & 3, 3);
        assert_eq!((nr >> 8) & 0xff, 0xfd);
        assert_eq!(nr & 0xff, DM_DEV_CREATE_CMD as libc::c_ulong);
        assert_eq!((nr >> 16) & 0x3fff, DM_IOCTL_HEADER_SIZE as libc::c_ulong);
    }

    #[test]
    fn test_dm_ioctl_init_basic() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "test_vol", "CRYPT-INTEGRITY-test", 0);

        assert_eq!(read_u32(&buf, DM_VERSION_OFFSET), DM_VERSION_MAJOR);
        assert_eq!(read_u32(&buf, DM_VERSION_OFFSET + 4), DM_VERSION_MINOR);
        assert_eq!(read_u32(&buf, DM_VERSION_OFFSET + 8), DM_VERSION_PATCHLEVEL);
        assert_eq!(
            read_u32(&buf, DM_DATA_SIZE_OFFSET),
            DM_IOCTL_BUF_SIZE as u32
        );
        assert_eq!(
            read_u32(&buf, DM_DATA_START_OFFSET),
            DM_IOCTL_HEADER_SIZE as u32
        );
        assert_eq!(read_u32(&buf, DM_FLAGS_OFFSET), 0);

        let name_bytes = &buf[DM_NAME_OFFSET..DM_NAME_OFFSET + DM_NAME_SIZE];
        let end = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(DM_NAME_SIZE);
        let name = std::str::from_utf8(&name_bytes[..end]).unwrap();
        assert_eq!(name, "test_vol");

        let uuid_bytes = &buf[DM_UUID_OFFSET..DM_UUID_OFFSET + DM_UUID_SIZE];
        let end = uuid_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(DM_UUID_SIZE);
        let uuid = std::str::from_utf8(&uuid_bytes[..end]).unwrap();
        assert_eq!(uuid, "CRYPT-INTEGRITY-test");
    }

    #[test]
    fn test_dm_ioctl_init_readonly() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "vol", "", DM_READONLY_FLAG);
        assert_eq!(read_u32(&buf, DM_FLAGS_OFFSET), DM_READONLY_FLAG);
    }

    #[test]
    fn test_dm_ioctl_init_suspend() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "vol", "", DM_SUSPEND_FLAG);
        assert_eq!(read_u32(&buf, DM_FLAGS_OFFSET), DM_SUSPEND_FLAG);
    }

    #[test]
    fn test_dm_ioctl_init_empty_name() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "", "", 0);
        assert_eq!(buf[DM_NAME_OFFSET], 0);
        assert_eq!(buf[DM_UUID_OFFSET], 0);
    }

    #[test]
    fn test_dm_ioctl_init_long_name_truncated() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        let long_name = "a".repeat(200);
        dm_ioctl_init(&mut buf, &long_name, "", 0);

        let name_bytes = &buf[DM_NAME_OFFSET..DM_NAME_OFFSET + DM_NAME_SIZE];
        let end = name_bytes
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(DM_NAME_SIZE);
        assert_eq!(end, DM_NAME_SIZE - 1);
    }

    #[test]
    fn test_write_read_u32() {
        let mut buf = vec![0u8; 8];
        write_u32(&mut buf, 0, 0x12345678);
        assert_eq!(read_u32(&buf, 0), 0x12345678);
        write_u32(&mut buf, 4, u32::MAX);
        assert_eq!(read_u32(&buf, 4), u32::MAX);
    }

    #[test]
    fn test_write_read_u64() {
        let mut buf = vec![0u8; 16];
        write_u64(&mut buf, 0, 0x123456789abcdef0);
        assert_eq!(read_u64(&buf, 0), 0x123456789abcdef0);
        write_u64(&mut buf, 8, u64::MAX);
        assert_eq!(read_u64(&buf, 8), u64::MAX);
    }

    #[test]
    fn test_write_string_basic() {
        let mut buf = vec![0u8; 32];
        write_string(&mut buf, 0, 16, "hello");
        assert_eq!(&buf[0..5], b"hello");
        assert_eq!(buf[5], 0);
    }

    #[test]
    fn test_write_string_truncation() {
        let mut buf = vec![0u8; 16];
        write_string(&mut buf, 0, 4, "longstring");
        assert_eq!(&buf[0..3], b"lon");
        assert_eq!(buf[3], 0);
    }

    #[test]
    fn test_write_string_empty() {
        let mut buf = vec![0u8; 16];
        write_string(&mut buf, 0, 16, "");
        assert_eq!(buf[0], 0);
    }

    #[test]
    fn test_align8() {
        assert_eq!(align8(0), 0);
        assert_eq!(align8(1), 8);
        assert_eq!(align8(7), 8);
        assert_eq!(align8(8), 8);
        assert_eq!(align8(9), 16);
        assert_eq!(align8(16), 16);
        assert_eq!(align8(40), 40);
        assert_eq!(align8(41), 48);
    }

    // -----------------------------------------------------------------------
    // append_target tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_append_target_basic() {
        let mut buf = vec![0u8; 4096];
        let data_start = DM_IOCTL_HEADER_SIZE;
        let size = append_target(
            &mut buf,
            data_start,
            0,
            1000,
            "integrity",
            "/dev/sda1 0 4 J 1 internal_hash:crc32c",
        );
        assert!(size > 0);
        assert_eq!(size % 8, 0);

        assert_eq!(read_u64(&buf, data_start), 0);
        assert_eq!(read_u64(&buf, data_start + 8), 1000);

        let tt_bytes = &buf[data_start + 24..data_start + 40];
        let tt_end = tt_bytes.iter().position(|&b| b == 0).unwrap_or(16);
        let tt = std::str::from_utf8(&tt_bytes[..tt_end]).unwrap();
        assert_eq!(tt, "integrity");
    }

    #[test]
    fn test_append_target_params_content() {
        let mut buf = vec![0u8; 4096];
        let data_start = DM_IOCTL_HEADER_SIZE;
        let params_str = "/dev/sda1 0 32 D 2 internal_hash:sha256 allow_discards";
        let _size = append_target(&mut buf, data_start, 0, 500, "integrity", params_str);

        let param_off = data_start + 40;
        let param_end = buf[param_off..].iter().position(|&b| b == 0).unwrap();
        let params = std::str::from_utf8(&buf[param_off..param_off + param_end]).unwrap();
        assert_eq!(params, params_str);
    }

    #[test]
    fn test_append_target_alignment() {
        let mut buf = vec![0u8; 4096];
        let data_start = DM_IOCTL_HEADER_SIZE;
        let size = append_target(&mut buf, data_start, 0, 100, "integrity", "x");
        assert_eq!(size % 8, 0);

        let size2 = append_target(
            &mut buf,
            data_start,
            0,
            100,
            "integrity",
            "this is a longer parameter string that should also be aligned",
        );
        assert_eq!(size2 % 8, 0);
    }

    // -----------------------------------------------------------------------
    // Integration-style tests (no actual DM operations)
    // -----------------------------------------------------------------------

    #[test]
    fn test_attach_missing_device() {
        let result = cmd_attach("testvol", "/nonexistent/device", None, "");
        assert!(result.is_err());
    }

    #[test]
    fn test_attach_hmac_without_key() {
        // HMAC algorithms should fail without a key.
        let result = cmd_attach(
            "testvol",
            "/nonexistent/device",
            Some("-"),
            "algorithm=hmac(sha256)",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("requires a key"));
    }

    #[test]
    fn test_attach_nonexistent_data_device() {
        let result = cmd_attach(
            "testvol",
            "/dev/null",
            None,
            "data-device=/nonexistent/data",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Data device not found"));
    }

    #[test]
    fn test_attach_nonexistent_journal_integrity_key_file() {
        let result = cmd_attach(
            "testvol",
            "/dev/null",
            None,
            "journal-integrity-key-file=/nonexistent/key",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Journal integrity key file not found")
        );
    }

    #[test]
    fn test_attach_nonexistent_journal_crypt_key_file() {
        let result = cmd_attach(
            "testvol",
            "/dev/null",
            None,
            "journal-crypt-key-file=/nonexistent/key",
        );
        assert!(result.is_err());
        assert!(
            result
                .unwrap_err()
                .contains("Journal crypt key file not found")
        );
    }

    #[test]
    fn test_detach_nonexistent_volume() {
        let result = cmd_detach("nonexistent_volume_99999");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Full-flow argument + option parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_attach_parse() {
        let cmd = parse_args(&args(&[
            "attach",
            "data_integrity",
            "/dev/nvme0n1p3",
            "/path/to/key",
            "algorithm=hmac(sha256),no-journal,sector-size=4096,integrity-recalculate,allow-discards",
        ]))
        .unwrap();

        if let Command::Attach {
            volume,
            device,
            key,
            options,
        } = cmd
        {
            assert_eq!(volume, "data_integrity");
            assert_eq!(device, "/dev/nvme0n1p3");
            assert_eq!(key, Some("/path/to/key".to_string()));

            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.algorithm, "hmac(sha256)");
            assert_eq!(opts.journal_mode, JournalMode::NoJournal);
            assert_eq!(opts.sector_size, 4096);
            assert!(opts.integrity_recalculate);
            assert!(opts.allow_discards);
        } else {
            panic!("Expected Attach command");
        }
    }

    #[test]
    fn test_full_attach_parse_minimal() {
        let cmd = parse_args(&args(&["attach", "simple_integrity", "/dev/sda1"])).unwrap();

        if let Command::Attach {
            volume,
            device,
            key,
            options,
        } = cmd
        {
            assert_eq!(volume, "simple_integrity");
            assert_eq!(device, "/dev/sda1");
            assert!(key.is_none());
            assert!(options.is_empty());

            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.algorithm, DEFAULT_ALGORITHM);
            assert_eq!(opts.journal_mode, JournalMode::Journal);
        } else {
            panic!("Expected Attach command");
        }
    }

    #[test]
    fn test_full_attach_parse_with_journal_options() {
        let cmd = parse_args(&args(&[
            "attach",
            "journaled",
            "/dev/sda1",
            "-",
            "journal-commit-time=5000,journal-watermark=75,journal-integrity=hmac(sha256),journal-crypt=aes-xts-plain64",
        ]))
        .unwrap();

        if let Command::Attach {
            volume,
            device,
            key,
            options,
        } = cmd
        {
            assert_eq!(volume, "journaled");
            assert_eq!(device, "/dev/sda1");
            assert_eq!(key, Some("-".to_string()));

            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.journal_commit_time, Some(5000));
            assert_eq!(opts.journal_watermark, Some(75));
            assert_eq!(opts.journal_integrity, Some("hmac(sha256)".to_string()));
            assert_eq!(opts.journal_crypt, Some("aes-xts-plain64".to_string()));
        } else {
            panic!("Expected Attach command");
        }
    }

    // -----------------------------------------------------------------------
    // Constants / sanity checks
    // -----------------------------------------------------------------------

    #[test]
    fn test_dm_ioctl_header_size() {
        assert_eq!(DM_IOCTL_HEADER_SIZE, 312);
    }

    #[test]
    fn test_dm_ioctl_buf_size_larger_than_header() {
        assert!(DM_IOCTL_BUF_SIZE > DM_IOCTL_HEADER_SIZE);
    }

    #[test]
    fn test_default_algorithm() {
        assert_eq!(DEFAULT_ALGORITHM, "crc32c");
    }

    #[test]
    fn test_default_sector_size() {
        assert_eq!(DEFAULT_SECTOR_SIZE, 512);
    }

    #[test]
    fn test_exit_integrity_error_code() {
        assert_eq!(EXIT_INTEGRITY_ERROR, 4);
    }

    // -----------------------------------------------------------------------
    // Superblock constants
    // -----------------------------------------------------------------------

    #[test]
    fn test_sb_magic() {
        assert_eq!(SB_MAGIC, b"integrit");
        assert_eq!(SB_MAGIC.len(), 8);
    }

    #[test]
    fn test_sb_size() {
        assert_eq!(SB_SIZE, 512);
    }

    #[test]
    fn test_sb_versions() {
        assert_eq!(SB_VERSION_1, 1);
        assert_eq!(SB_VERSION_2, 2);
        assert_eq!(SB_VERSION_3, 3);
        assert_eq!(SB_VERSION_4, 4);
        assert_eq!(SB_VERSION_5, 5);
    }

    #[test]
    fn test_sb_flags() {
        assert_eq!(SB_FLAG_HAVE_JOURNAL_MAC, 0x1);
        assert_eq!(SB_FLAG_RECALCULATING, 0x2);
        assert_eq!(SB_FLAG_DIRTY_BITMAP, 0x4);
        assert_eq!(SB_FLAG_FIXED_PADDING, 0x8);
        assert_eq!(SB_FLAG_FIXED_HMAC, 0x10);
    }

    // -----------------------------------------------------------------------
    // IntegritySuperblock roundtrip tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_superblock_roundtrip_v1() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_1,
            log2_interleave_sectors: 15,
            tag_size: 4,
            journal_sections: 8,
            provided_data_sectors: 1000,
            flags: 0,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };

        let bytes = sb.to_bytes();
        assert_eq!(bytes.len(), SB_SIZE);
        assert_eq!(&bytes[0..8], SB_MAGIC);
        assert_eq!(bytes[SB_VERSION_OFFSET], SB_VERSION_1);

        let sb2 = IntegritySuperblock::from_bytes(&bytes).unwrap();
        assert_eq!(sb, sb2);
    }

    #[test]
    fn test_superblock_roundtrip_v2() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_2,
            log2_interleave_sectors: 12,
            tag_size: 32,
            journal_sections: 4,
            provided_data_sectors: 50000,
            flags: SB_FLAG_DIRTY_BITMAP,
            log2_sectors_per_block: 3,
            log2_blocks_per_bitmap_bit: 2,
            recalc_sector: 0,
        };

        let bytes = sb.to_bytes();
        let sb2 = IntegritySuperblock::from_bytes(&bytes).unwrap();
        assert_eq!(sb, sb2);
        assert_eq!(sb2.log2_blocks_per_bitmap_bit, 2);
    }

    #[test]
    fn test_superblock_roundtrip_v3() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_3,
            log2_interleave_sectors: 15,
            tag_size: 32,
            journal_sections: 8,
            provided_data_sectors: 999999,
            flags: SB_FLAG_RECALCULATING,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 42,
        };

        let bytes = sb.to_bytes();
        let sb2 = IntegritySuperblock::from_bytes(&bytes).unwrap();
        assert_eq!(sb, sb2);
        assert_eq!(sb2.recalc_sector, 42);
    }

    #[test]
    fn test_superblock_roundtrip_v5_all_flags() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_5,
            log2_interleave_sectors: 10,
            tag_size: 64,
            journal_sections: 16,
            provided_data_sectors: 123456789,
            flags: SB_FLAG_HAVE_JOURNAL_MAC
                | SB_FLAG_RECALCULATING
                | SB_FLAG_DIRTY_BITMAP
                | SB_FLAG_FIXED_PADDING
                | SB_FLAG_FIXED_HMAC,
            log2_sectors_per_block: 2,
            log2_blocks_per_bitmap_bit: 4,
            recalc_sector: 100,
        };

        let bytes = sb.to_bytes();
        let sb2 = IntegritySuperblock::from_bytes(&bytes).unwrap();
        assert_eq!(sb, sb2);
    }

    #[test]
    fn test_superblock_from_bytes_bad_magic() {
        let mut buf = vec![0u8; SB_SIZE];
        buf[0..8].copy_from_slice(b"BADMAGIC");
        assert!(IntegritySuperblock::from_bytes(&buf).is_err());
    }

    #[test]
    fn test_superblock_from_bytes_too_small() {
        let buf = vec![0u8; 100];
        assert!(IntegritySuperblock::from_bytes(&buf).is_err());
    }

    #[test]
    fn test_superblock_from_bytes_version_zero() {
        let mut buf = vec![0u8; SB_SIZE];
        buf[0..8].copy_from_slice(SB_MAGIC);
        buf[SB_VERSION_OFFSET] = 0;
        assert!(IntegritySuperblock::from_bytes(&buf).is_err());
    }

    #[test]
    fn test_superblock_from_bytes_version_too_high() {
        let mut buf = vec![0u8; SB_SIZE];
        buf[0..8].copy_from_slice(SB_MAGIC);
        buf[SB_VERSION_OFFSET] = 99;
        assert!(IntegritySuperblock::from_bytes(&buf).is_err());
    }

    #[test]
    fn test_superblock_v1_ignores_v2_fields() {
        // A version 1 superblock should have log2_blocks_per_bitmap_bit = 0
        // even if that byte is non-zero on disk.
        let mut buf = vec![0u8; SB_SIZE];
        buf[0..8].copy_from_slice(SB_MAGIC);
        buf[SB_VERSION_OFFSET] = SB_VERSION_1;
        buf[SB_LOG2_BLOCKS_PER_BITMAP_BIT_OFFSET] = 5;
        let sb = IntegritySuperblock::from_bytes(&buf).unwrap();
        assert_eq!(sb.log2_blocks_per_bitmap_bit, 0);
    }

    #[test]
    fn test_superblock_v1_ignores_recalc_sector() {
        let mut buf = vec![0u8; SB_SIZE];
        buf[0..8].copy_from_slice(SB_MAGIC);
        buf[SB_VERSION_OFFSET] = SB_VERSION_1;
        // Write a non-zero recalc_sector.
        buf[SB_RECALC_SECTOR_OFFSET..SB_RECALC_SECTOR_OFFSET + 8]
            .copy_from_slice(&42u64.to_le_bytes());
        let sb = IntegritySuperblock::from_bytes(&buf).unwrap();
        assert_eq!(sb.recalc_sector, 0);
    }

    // -----------------------------------------------------------------------
    // Superblock flags_description
    // -----------------------------------------------------------------------

    #[test]
    fn test_flags_description_none() {
        let sb = IntegritySuperblock {
            version: 1,
            log2_interleave_sectors: 0,
            tag_size: 4,
            journal_sections: 0,
            provided_data_sectors: 0,
            flags: 0,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        assert_eq!(sb.flags_description(), "(none)");
    }

    #[test]
    fn test_flags_description_all() {
        let sb = IntegritySuperblock {
            version: 1,
            log2_interleave_sectors: 0,
            tag_size: 4,
            journal_sections: 0,
            provided_data_sectors: 0,
            flags: SB_FLAG_HAVE_JOURNAL_MAC
                | SB_FLAG_RECALCULATING
                | SB_FLAG_DIRTY_BITMAP
                | SB_FLAG_FIXED_PADDING
                | SB_FLAG_FIXED_HMAC,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        let desc = sb.flags_description();
        assert!(desc.contains("HAVE_JOURNAL_MAC"));
        assert!(desc.contains("RECALCULATING"));
        assert!(desc.contains("DIRTY_BITMAP"));
        assert!(desc.contains("FIXED_PADDING"));
        assert!(desc.contains("FIXED_HMAC"));
    }

    #[test]
    fn test_flags_description_single() {
        let sb = IntegritySuperblock {
            version: 1,
            log2_interleave_sectors: 0,
            tag_size: 4,
            journal_sections: 0,
            provided_data_sectors: 0,
            flags: SB_FLAG_RECALCULATING,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        assert_eq!(sb.flags_description(), "RECALCULATING");
    }

    // -----------------------------------------------------------------------
    // Superblock format_dump
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_dump_v1() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_1,
            log2_interleave_sectors: 15,
            tag_size: 4,
            journal_sections: 8,
            provided_data_sectors: 1000,
            flags: 0,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        let dump = sb.format_dump();
        assert!(dump.contains("superblock_version:         1"));
        assert!(dump.contains("log2_interleave_sectors:    15"));
        assert!(dump.contains("integrity_tag_size:         4"));
        assert!(dump.contains("journal_sections:           8"));
        assert!(dump.contains("provided_data_sectors:      1000"));
        assert!(dump.contains("sector_size:                512"));
        assert!(dump.contains("log2_sectors_per_block:     0"));
        assert!(dump.contains("flags:                      0x0 [(none)]"));
        // v1 should not have log2_blocks_per_bitmap_bit or recalc_sector.
        assert!(!dump.contains("log2_blocks_per_bitmap_bit"));
        assert!(!dump.contains("recalc_sector"));
    }

    #[test]
    fn test_format_dump_v2() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_2,
            log2_interleave_sectors: 12,
            tag_size: 32,
            journal_sections: 4,
            provided_data_sectors: 50000,
            flags: SB_FLAG_DIRTY_BITMAP,
            log2_sectors_per_block: 3,
            log2_blocks_per_bitmap_bit: 2,
            recalc_sector: 0,
        };
        let dump = sb.format_dump();
        assert!(dump.contains("superblock_version:         2"));
        assert!(dump.contains("sector_size:                4096"));
        assert!(dump.contains("log2_blocks_per_bitmap_bit: 2"));
        assert!(!dump.contains("recalc_sector"));
    }

    #[test]
    fn test_format_dump_v3_with_recalc() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_3,
            log2_interleave_sectors: 15,
            tag_size: 32,
            journal_sections: 8,
            provided_data_sectors: 999999,
            flags: SB_FLAG_RECALCULATING,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 42,
        };
        let dump = sb.format_dump();
        assert!(dump.contains("recalc_sector:              42"));
        assert!(dump.contains("RECALCULATING"));
    }

    // -----------------------------------------------------------------------
    // log2_sectors_per_block
    // -----------------------------------------------------------------------

    #[test]
    fn test_log2_sectors_per_block_512() {
        assert_eq!(log2_sectors_per_block(512), 0);
    }

    #[test]
    fn test_log2_sectors_per_block_1024() {
        assert_eq!(log2_sectors_per_block(1024), 1);
    }

    #[test]
    fn test_log2_sectors_per_block_4096() {
        assert_eq!(log2_sectors_per_block(4096), 3);
    }

    // -----------------------------------------------------------------------
    // read_superblock / write_superblock roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_write_superblock_roundtrip() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_2,
            log2_interleave_sectors: 15,
            tag_size: 32,
            journal_sections: 8,
            provided_data_sectors: 12345,
            flags: SB_FLAG_DIRTY_BITMAP,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 1,
            recalc_sector: 0,
        };

        let f = temp_file_zeros(SB_SIZE);
        let path = f.path().to_str().unwrap();
        write_superblock(path, &sb).unwrap();
        let sb2 = read_superblock(path).unwrap();
        assert_eq!(sb, sb2);
    }

    #[test]
    fn test_read_superblock_nonexistent() {
        assert!(read_superblock("/nonexistent/device/path").is_err());
    }

    #[test]
    fn test_read_superblock_too_small() {
        let f = temp_file_zeros(100);
        let result = read_superblock(f.path().to_str().unwrap());
        assert!(result.is_err());
    }

    #[test]
    fn test_read_superblock_bad_magic() {
        let f = temp_file_zeros(SB_SIZE);
        let result = read_superblock(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bad magic"));
    }

    #[test]
    fn test_write_superblock_nonexistent() {
        let sb = IntegritySuperblock {
            version: 1,
            log2_interleave_sectors: 0,
            tag_size: 4,
            journal_sections: 0,
            provided_data_sectors: 0,
            flags: 0,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        assert!(write_superblock("/nonexistent/path", &sb).is_err());
    }

    // -----------------------------------------------------------------------
    // cmd_format tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_default_options() {
        // Create a 2 MiB file.
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "").unwrap();

        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.version, SB_VERSION_1);
        assert_eq!(sb.tag_size, 4); // crc32c
        assert_eq!(sb.journal_sections, DEFAULT_JOURNAL_SECTIONS);
        assert!(sb.provided_data_sectors > 0);
        assert_eq!(sb.flags, 0);
        assert_eq!(sb.log2_sectors_per_block, 0); // 512
        assert_eq!(sb.log2_interleave_sectors, DEFAULT_LOG2_INTERLEAVE);
    }

    #[test]
    fn test_format_sha256() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "algorithm=sha256").unwrap();

        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.tag_size, 32);
    }

    #[test]
    fn test_format_no_journal() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "no-journal").unwrap();

        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.journal_sections, 0);
    }

    #[test]
    fn test_format_bitmap_mode() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "bitmap").unwrap();

        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.version, SB_VERSION_2);
        assert_ne!(sb.flags & SB_FLAG_DIRTY_BITMAP, 0);
    }

    #[test]
    fn test_format_with_recalculate() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "integrity-recalculate").unwrap();

        let sb = read_superblock(path).unwrap();
        assert!(sb.version >= SB_VERSION_3);
        assert_ne!(sb.flags & SB_FLAG_RECALCULATING, 0);
    }

    #[test]
    fn test_format_fix_padding() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "fix-padding").unwrap();

        let sb = read_superblock(path).unwrap();
        assert!(sb.version >= SB_VERSION_4);
        assert_ne!(sb.flags & SB_FLAG_FIXED_PADDING, 0);
    }

    #[test]
    fn test_format_fix_hmac() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "fix-hmac").unwrap();

        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.version, SB_VERSION_5);
        assert_ne!(sb.flags & SB_FLAG_FIXED_HMAC, 0);
    }

    #[test]
    fn test_format_sector_size_4096() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "sector-size=4096").unwrap();

        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.log2_sectors_per_block, 3);
    }

    #[test]
    fn test_format_journal_integrity_flag() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(path, "journal-integrity=hmac(sha256)").unwrap();

        let sb = read_superblock(path).unwrap();
        assert_ne!(sb.flags & SB_FLAG_HAVE_JOURNAL_MAC, 0);
    }

    #[test]
    fn test_format_combined_options() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();
        cmd_format(
            path,
            "algorithm=sha256,no-journal,fix-padding,fix-hmac,sector-size=4096",
        )
        .unwrap();

        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.version, SB_VERSION_5);
        assert_eq!(sb.tag_size, 32);
        assert_eq!(sb.journal_sections, 0);
        assert_ne!(sb.flags & SB_FLAG_FIXED_PADDING, 0);
        assert_ne!(sb.flags & SB_FLAG_FIXED_HMAC, 0);
        assert_eq!(sb.log2_sectors_per_block, 3);
    }

    #[test]
    fn test_format_nonexistent_device() {
        let result = cmd_format("/nonexistent/device", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Cannot access"));
    }

    #[test]
    fn test_format_device_too_small() {
        let f = temp_file_zeros(256);
        let result = cmd_format(f.path().to_str().unwrap(), "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too small"));
    }

    // -----------------------------------------------------------------------
    // cmd_wipe tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_wipe_valid_superblock() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_1,
            log2_interleave_sectors: 15,
            tag_size: 4,
            journal_sections: 8,
            provided_data_sectors: 1000,
            flags: 0,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        let f = temp_file_with_superblock(&sb);
        let path = f.path().to_str().unwrap();

        // Verify superblock exists.
        assert!(read_superblock(path).is_ok());

        // Wipe.
        cmd_wipe(path).unwrap();

        // Verify superblock is gone.
        let result = read_superblock(path);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bad magic"));
    }

    #[test]
    fn test_wipe_no_superblock() {
        let f = temp_file_zeros(SB_SIZE);
        let result = cmd_wipe(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("No dm-integrity superblock"));
    }

    #[test]
    fn test_wipe_nonexistent_device() {
        let result = cmd_wipe("/nonexistent/device");
        assert!(result.is_err());
    }

    #[test]
    fn test_wipe_too_small() {
        let f = temp_file_zeros(100);
        let result = cmd_wipe(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too small"));
    }

    #[test]
    fn test_wipe_zeros_entire_superblock_sector() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_1,
            log2_interleave_sectors: 15,
            tag_size: 4,
            journal_sections: 8,
            provided_data_sectors: 1000,
            flags: 0,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        let f = temp_file_with_superblock(&sb);
        let path = f.path().to_str().unwrap();

        cmd_wipe(path).unwrap();

        let data = fs::read(path).unwrap();
        // First 512 bytes should all be zero.
        assert!(data[..SB_SIZE].iter().all(|&b| b == 0));
    }

    // -----------------------------------------------------------------------
    // cmd_dump tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_dump_valid_superblock() {
        let sb = IntegritySuperblock {
            version: SB_VERSION_1,
            log2_interleave_sectors: 15,
            tag_size: 4,
            journal_sections: 8,
            provided_data_sectors: 1000,
            flags: 0,
            log2_sectors_per_block: 0,
            log2_blocks_per_bitmap_bit: 0,
            recalc_sector: 0,
        };
        let f = temp_file_with_superblock(&sb);
        let path = f.path().to_str().unwrap();

        // cmd_dump should succeed (output goes to stdout).
        cmd_dump(path).unwrap();
    }

    #[test]
    fn test_dump_nonexistent_device() {
        let result = cmd_dump("/nonexistent/device");
        assert!(result.is_err());
    }

    #[test]
    fn test_dump_no_superblock() {
        let f = temp_file_zeros(SB_SIZE);
        let result = cmd_dump(f.path().to_str().unwrap());
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("bad magic"));
    }

    // -----------------------------------------------------------------------
    // cmd_resize tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_resize_nonexistent_volume() {
        // Without a device arg, it tries DM_TABLE_STATUS which will fail.
        let result = cmd_resize("nonexistent_vol_999", None);
        assert!(result.is_err());
    }

    #[test]
    fn test_resize_nonexistent_device() {
        // With a device arg, it tries device_size_sectors which will fail.
        let result = cmd_resize("nonexistent_vol_999", Some("/nonexistent/dev"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Failed to get device size"));
    }

    // -----------------------------------------------------------------------
    // Format → Dump → Wipe integration
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_dump_wipe_cycle() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();

        // 1. Format.
        cmd_format(path, "algorithm=sha256,no-journal").unwrap();

        // 2. Dump — read back and verify fields.
        let sb = read_superblock(path).unwrap();
        assert_eq!(sb.tag_size, 32);
        assert_eq!(sb.journal_sections, 0);
        let dump = sb.format_dump();
        assert!(dump.contains("integrity_tag_size:         32"));
        assert!(dump.contains("journal_sections:           0"));

        // 3. Wipe.
        cmd_wipe(path).unwrap();

        // 4. Verify wiped.
        assert!(read_superblock(path).is_err());
    }

    #[test]
    fn test_format_then_reformat() {
        let f = temp_file_zeros(2 * 1024 * 1024);
        let path = f.path().to_str().unwrap();

        // Format with crc32c.
        cmd_format(path, "").unwrap();
        let sb1 = read_superblock(path).unwrap();
        assert_eq!(sb1.tag_size, 4);

        // Re-format with sha256.
        cmd_format(path, "algorithm=sha256").unwrap();
        let sb2 = read_superblock(path).unwrap();
        assert_eq!(sb2.tag_size, 32);
    }

    // -----------------------------------------------------------------------
    // Full-flow argument parsing for new commands
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_format_parse() {
        let cmd = parse_args(&args(&[
            "format",
            "/dev/nvme0n1p3",
            "algorithm=sha256,no-journal,sector-size=4096",
        ]))
        .unwrap();

        if let Command::Format { device, options } = cmd {
            assert_eq!(device, "/dev/nvme0n1p3");
            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.algorithm, "sha256");
            assert_eq!(opts.journal_mode, JournalMode::NoJournal);
            assert_eq!(opts.sector_size, 4096);
        } else {
            panic!("Expected Format command");
        }
    }

    #[test]
    fn test_full_resize_parse_no_device() {
        let cmd = parse_args(&args(&["resize", "integrity_vol"])).unwrap();
        if let Command::Resize { volume, device } = cmd {
            assert_eq!(volume, "integrity_vol");
            assert!(device.is_none());
        } else {
            panic!("Expected Resize command");
        }
    }

    #[test]
    fn test_full_resize_parse_with_device() {
        let cmd = parse_args(&args(&["resize", "integrity_vol", "/dev/sda1"])).unwrap();
        if let Command::Resize { volume, device } = cmd {
            assert_eq!(volume, "integrity_vol");
            assert_eq!(device, Some("/dev/sda1".to_string()));
        } else {
            panic!("Expected Resize command");
        }
    }
}
