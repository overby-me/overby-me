//! systemd-veritysetup — Attach and detach dm-verity integrity-protected block devices.
//!
//! A drop-in replacement for `systemd-veritysetup(8)`. This tool sets up
//! dm-verity verified block devices via the device-mapper kernel interface.
//!
//! Usage:
//!
//!   systemd-veritysetup attach VOLUME DATA_DEVICE HASH_DEVICE ROOT_HASH [OPTIONS]
//!       Set up a dm-verity verified block device. VOLUME is the device-mapper
//!       name, DATA_DEVICE is the underlying data block device, HASH_DEVICE is
//!       the device containing the hash tree, ROOT_HASH is the hex-encoded root
//!       hash, and OPTIONS is a comma-separated list of options.
//!
//!   systemd-veritysetup detach VOLUME
//!       Tear down a previously attached dm-verity device.
//!
//!   systemd-veritysetup --help | -h
//!       Show help.
//!
//!   systemd-veritysetup --version
//!       Show version.
//!
//! Options (comma-separated in OPTIONS):
//!   format=N             Verity format version (1 or 2, default: 1)
//!   hash=HASH            Hash algorithm (default: sha256)
//!   data-block-size=N    Data block size in bytes (default: 4096)
//!   hash-block-size=N    Hash block size in bytes (default: 4096)
//!   data-blocks=N        Number of data blocks
//!   hash-offset=N        Hash area offset in bytes on hash device
//!   salt=HEX             Hex-encoded salt (default: empty)
//!   fec-device=PATH      FEC (Forward Error Correction) device path
//!   fec-offset=N         FEC area offset in bytes
//!   fec-roots=N          FEC parity bytes (default: 2)
//!   root-hash-signature=PATH  Root hash signature file for kernel verification
//!   restart-on-corruption   Restart system on data corruption
//!   ignore-corruption       Ignore data corruption
//!   panic-on-corruption     Kernel panic on data corruption
//!   ignore-zero-blocks      Do not verify zero blocks
//!   check-at-most-once      Check data blocks at most once
//!   readonly                Open device read-only (default for verity)
//!   noauto                  Ignored (for /etc/veritytab compat)
//!   nofail                  Ignored (for /etc/veritytab compat)
//!   x-systemd.*             Ignored (for /etc/veritytab compat)
//!
//! Exit codes:
//!   0 — success
//!   1 — error (general)
//!   2 — hash verification error

use std::fmt;
use std::fs;
use std::io;
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
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

/// dm-verity defaults.
const DEFAULT_VERITY_VERSION: u32 = 1;
const DEFAULT_HASH_ALGORITHM: &str = "sha256";
const DEFAULT_DATA_BLOCK_SIZE: u32 = 4096;
const DEFAULT_HASH_BLOCK_SIZE: u32 = 4096;
const DEFAULT_FEC_ROOTS: u32 = 2;

/// Exit code for hash verification errors.
#[allow(dead_code)]
const EXIT_HASH_ERROR: i32 = 2;

// ---------------------------------------------------------------------------
// Parsed options from the comma-separated OPTIONS argument
// ---------------------------------------------------------------------------

/// Error handling mode for dm-verity corruption.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ErrorMode {
    /// Default: return I/O error to caller.
    Eio,
    /// Restart the system on corruption.
    RestartOnCorruption,
    /// Ignore corruption (log and continue).
    IgnoreCorruption,
    /// Kernel panic on corruption.
    PanicOnCorruption,
}

impl fmt::Display for ErrorMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorMode::Eio => write!(f, "eio"),
            ErrorMode::RestartOnCorruption => write!(f, "restart_on_corruption"),
            ErrorMode::IgnoreCorruption => write!(f, "ignore_corruption"),
            ErrorMode::PanicOnCorruption => write!(f, "panic_on_corruption"),
        }
    }
}

/// Parsed veritysetup options from the OPTIONS string.
#[derive(Debug, Clone)]
struct VerityOptions {
    /// Verity format version (1 or 2).
    format_version: u32,
    /// Hash algorithm (e.g., "sha256").
    hash_algorithm: String,
    /// Data block size in bytes.
    data_block_size: u32,
    /// Hash block size in bytes.
    hash_block_size: u32,
    /// Number of data blocks (0 = auto-detect from device size).
    data_blocks: u64,
    /// Hash area offset in bytes on the hash device.
    hash_offset: u64,
    /// Hex-encoded salt (empty string for no salt).
    salt: String,
    /// FEC device path.
    fec_device: Option<PathBuf>,
    /// FEC area offset in bytes.
    fec_offset: u64,
    /// FEC parity bytes.
    fec_roots: u32,
    /// Root hash signature file for kernel signature verification.
    root_hash_signature: Option<PathBuf>,
    /// Error handling mode.
    error_mode: ErrorMode,
    /// Ignore zero blocks.
    ignore_zero_blocks: bool,
    /// Check data blocks at most once.
    check_at_most_once: bool,
    /// Open read-only (default true for verity).
    readonly: bool,
}

impl Default for VerityOptions {
    fn default() -> Self {
        Self {
            format_version: DEFAULT_VERITY_VERSION,
            hash_algorithm: DEFAULT_HASH_ALGORITHM.to_string(),
            data_block_size: DEFAULT_DATA_BLOCK_SIZE,
            hash_block_size: DEFAULT_HASH_BLOCK_SIZE,
            data_blocks: 0,
            hash_offset: 0,
            salt: String::new(),
            fec_device: None,
            fec_offset: 0,
            fec_roots: DEFAULT_FEC_ROOTS,
            root_hash_signature: None,
            error_mode: ErrorMode::Eio,
            ignore_zero_blocks: false,
            check_at_most_once: false,
            readonly: true,
        }
    }
}

/// Parse a comma-separated options string.
fn parse_options(opts_str: &str) -> Result<VerityOptions, String> {
    let mut opts = VerityOptions::default();

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
                "format" => {
                    opts.format_version =
                        val.parse().map_err(|_| format!("Invalid format: {val}"))?;
                    if opts.format_version != 1 && opts.format_version != 2 {
                        return Err(format!(
                            "Invalid verity format version: {} (must be 1 or 2)",
                            opts.format_version
                        ));
                    }
                }
                "hash" => opts.hash_algorithm = val.to_string(),
                "data-block-size" | "data_block_size" => {
                    opts.data_block_size = val
                        .parse()
                        .map_err(|_| format!("Invalid data-block-size: {val}"))?;
                }
                "hash-block-size" | "hash_block_size" => {
                    opts.hash_block_size = val
                        .parse()
                        .map_err(|_| format!("Invalid hash-block-size: {val}"))?;
                }
                "data-blocks" | "data_blocks" => {
                    opts.data_blocks = val
                        .parse()
                        .map_err(|_| format!("Invalid data-blocks: {val}"))?;
                }
                "hash-offset" | "hash_offset" => {
                    opts.hash_offset = val
                        .parse()
                        .map_err(|_| format!("Invalid hash-offset: {val}"))?;
                }
                "salt" => {
                    // Validate hex encoding.
                    if !val.is_empty() && val != "-" {
                        validate_hex(val).map_err(|_| format!("Invalid salt hex: {val}"))?;
                    }
                    opts.salt = if val == "-" {
                        String::new()
                    } else {
                        val.to_string()
                    };
                }
                "fec-device" | "fec_device" => {
                    opts.fec_device = Some(PathBuf::from(val));
                }
                "fec-offset" | "fec_offset" => {
                    opts.fec_offset = val
                        .parse()
                        .map_err(|_| format!("Invalid fec-offset: {val}"))?;
                }
                "fec-roots" | "fec_roots" => {
                    opts.fec_roots = val
                        .parse()
                        .map_err(|_| format!("Invalid fec-roots: {val}"))?;
                }
                "root-hash-signature" | "root_hash_signature" => {
                    opts.root_hash_signature = Some(PathBuf::from(val));
                }
                _ if key.starts_with("x-systemd.") => { /* ignored */ }
                _ => {
                    return Err(format!("Unknown option: {key}={val}"));
                }
            }
        } else {
            match part {
                "restart-on-corruption" | "restart_on_corruption" => {
                    opts.error_mode = ErrorMode::RestartOnCorruption;
                }
                "ignore-corruption" | "ignore_corruption" => {
                    opts.error_mode = ErrorMode::IgnoreCorruption;
                }
                "panic-on-corruption" | "panic_on_corruption" => {
                    opts.error_mode = ErrorMode::PanicOnCorruption;
                }
                "ignore-zero-blocks" | "ignore_zero_blocks" => {
                    opts.ignore_zero_blocks = true;
                }
                "check-at-most-once" | "check_at_most_once" => {
                    opts.check_at_most_once = true;
                }
                "readonly" | "read-only" => {
                    opts.readonly = true;
                }
                "noauto" | "nofail" | "auto" => { /* compat — ignored */ }
                _ if part.starts_with("x-systemd.") => { /* ignored */ }
                _ => {
                    return Err(format!("Unknown option: {part}"));
                }
            }
        }
    }

    Ok(opts)
}

/// Validate that a string is valid hexadecimal.
fn validate_hex(s: &str) -> Result<(), ()> {
    if s.is_empty() {
        return Ok(());
    }
    for c in s.chars() {
        if !c.is_ascii_hexdigit() {
            return Err(());
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq)]
enum Command {
    Attach {
        volume: String,
        data_device: String,
        hash_device: String,
        root_hash: String,
        options: String,
    },
    Detach {
        volume: String,
    },
    Help,
    Version,
}

fn parse_args(args: &[String]) -> Result<Command, String> {
    if args.is_empty() {
        return Err("No command specified. Use 'attach' or 'detach'.".to_string());
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
            let data_device = iter
                .next()
                .ok_or("attach: missing DATA_DEVICE argument")?
                .clone();
            let hash_device = iter
                .next()
                .ok_or("attach: missing HASH_DEVICE argument")?
                .clone();
            let root_hash = iter
                .next()
                .ok_or("attach: missing ROOT_HASH argument")?
                .clone();
            let options = iter.next().cloned().unwrap_or_default();
            Ok(Command::Attach {
                volume,
                data_device,
                hash_device,
                root_hash,
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

/// Read the root hash signature from a file (DER or base64 format).
#[allow(dead_code)]
fn read_root_hash_signature(path: &Path) -> io::Result<Vec<u8>> {
    fs::read(path)
}

// ---------------------------------------------------------------------------
// dm-verity table construction
// ---------------------------------------------------------------------------

/// Build the dm-verity target parameter string.
///
/// Format for verity target:
///   `<version> <data_dev> <hash_dev> <data_block_size> <hash_block_size>
///    <num_data_blocks> <hash_start_block> <algorithm> <digest> <salt>
///    [<num_optional_params> <optional_params>...]`
fn build_verity_params(
    opts: &VerityOptions,
    data_device: &str,
    hash_device: &str,
    root_hash: &str,
    data_sectors: u64,
) -> String {
    let num_data_blocks = if opts.data_blocks > 0 {
        opts.data_blocks
    } else {
        (data_sectors * 512) / opts.data_block_size as u64
    };

    let hash_start_block = opts.hash_offset / opts.hash_block_size as u64;

    let salt = if opts.salt.is_empty() {
        "-".to_string()
    } else {
        opts.salt.clone()
    };

    let mut params = format!(
        "{} {} {} {} {} {} {} {} {} {}",
        opts.format_version,
        data_device,
        hash_device,
        opts.data_block_size,
        opts.hash_block_size,
        num_data_blocks,
        hash_start_block,
        opts.hash_algorithm,
        root_hash,
        salt
    );

    // Count optional parameters.
    let mut optional_params: Vec<String> = Vec::new();

    match opts.error_mode {
        ErrorMode::Eio => {} // default, no param needed
        ErrorMode::RestartOnCorruption => {
            optional_params.push("restart_on_corruption".to_string());
        }
        ErrorMode::IgnoreCorruption => {
            optional_params.push("ignore_corruption".to_string());
        }
        ErrorMode::PanicOnCorruption => {
            optional_params.push("panic_on_corruption".to_string());
        }
    }

    if opts.ignore_zero_blocks {
        optional_params.push("ignore_zero_blocks".to_string());
    }

    if opts.check_at_most_once {
        optional_params.push("check_at_most_once".to_string());
    }

    if let Some(ref fec_dev) = opts.fec_device {
        optional_params.push(format!("use_fec_from_device {}", fec_dev.display()));
        optional_params.push(format!("fec_blocks {num_data_blocks}"));
        if opts.fec_offset > 0 {
            optional_params.push(format!("fec_start {}", opts.fec_offset / 512));
        }
        if opts.fec_roots != DEFAULT_FEC_ROOTS {
            optional_params.push(format!("fec_roots {}", opts.fec_roots));
        }
    }

    if let Some(ref sig_path) = opts.root_hash_signature {
        optional_params.push(format!("root_hash_sig_key_desc {}", sig_path.display()));
    }

    if !optional_params.is_empty() {
        params.push_str(&format!(" {}", optional_params.len()));
        for opt in &optional_params {
            params.push_str(&format!(" {opt}"));
        }
    }

    params
}

// ---------------------------------------------------------------------------
// Attach / Detach operations
// ---------------------------------------------------------------------------

/// Attach a dm-verity device via device-mapper.
fn cmd_attach(
    volume: &str,
    data_device: &str,
    hash_device: &str,
    root_hash: &str,
    options_str: &str,
) -> Result<(), String> {
    let opts = parse_options(options_str)?;

    // Validate root hash is valid hex.
    validate_hex(root_hash)
        .map_err(|_| format!("Invalid root hash (not valid hex): {root_hash}"))?;

    if root_hash.is_empty() {
        return Err("Empty root hash.".to_string());
    }

    // Check root hash signature file exists if specified.
    if let Some(ref sig_path) = opts.root_hash_signature
        && !sig_path.exists()
    {
        return Err(format!(
            "Root hash signature file not found: {}",
            sig_path.display()
        ));
    }

    // Check FEC device exists if specified.
    if let Some(ref fec_dev) = opts.fec_device
        && !fec_dev.exists()
    {
        return Err(format!("FEC device not found: {}", fec_dev.display()));
    }

    // Get data device size in sectors.
    let data_sectors = device_size_sectors(data_device)
        .map_err(|e| format!("Failed to get device size for {data_device}: {e}"))?;

    if data_sectors == 0 {
        return Err(format!("Data device {data_device} has zero size."));
    }

    // Calculate the number of data sectors (aligned to data block size).
    let target_sectors = if opts.data_blocks > 0 {
        opts.data_blocks * (opts.data_block_size as u64) / 512
    } else {
        // Align down to data block boundary.
        let data_block_sectors = opts.data_block_size as u64 / 512;
        (data_sectors / data_block_sectors) * data_block_sectors
    };

    // Build UUID.
    let uuid_str = format!("CRYPT-VERITY-{volume}");

    // Build the dm-verity target parameters.
    let verity_params =
        build_verity_params(&opts, data_device, hash_device, root_hash, data_sectors);

    // verity devices are always read-only.
    let flags = DM_READONLY_FLAG;

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
        "verity",
        &verity_params,
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

    eprintln!("Set up verity device /dev/mapper/{volume}");
    Ok(())
}

/// Detach (tear down) a dm-verity device.
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

    eprintln!("Detached verity device /dev/mapper/{volume}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!(
        "\
Usage: systemd-veritysetup [COMMAND] [OPTIONS...]

Commands:
  attach VOLUME DATA_DEVICE HASH_DEVICE ROOT_HASH [OPTIONS]
      Set up a dm-verity verified block device.
      VOLUME       — device-mapper name (appears as /dev/mapper/VOLUME)
      DATA_DEVICE  — underlying data block device
      HASH_DEVICE  — device containing the hash tree
      ROOT_HASH    — hex-encoded root hash
      OPTIONS      — comma-separated list of options (see below)

  detach VOLUME
      Tear down a previously attached dm-verity device.

Options (comma-separated in the OPTIONS argument):
  format=N              Verity format version (1 or 2, default: 1)
  hash=HASH             Hash algorithm (default: sha256)
  data-block-size=N     Data block size in bytes (default: 4096)
  hash-block-size=N     Hash block size in bytes (default: 4096)
  data-blocks=N         Number of data blocks
  hash-offset=N         Hash area offset in bytes
  salt=HEX              Hex-encoded salt
  fec-device=PATH       FEC device path
  fec-offset=N          FEC area offset in bytes
  fec-roots=N           FEC parity bytes (default: 2)
  root-hash-signature=PATH  Root hash signature file
  restart-on-corruption Restart system on corruption
  ignore-corruption     Ignore corruption (log and continue)
  panic-on-corruption   Kernel panic on corruption
  ignore-zero-blocks    Do not verify zero blocks
  check-at-most-once    Check blocks at most once
  readonly              Open device read-only (default)

Exit codes:
  0 — success
  1 — general error
  2 — hash verification error"
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
            eprintln!("systemd-veritysetup (rust-systemd) 0.1.0");
            Ok(())
        }
        Command::Attach {
            volume,
            data_device,
            hash_device,
            root_hash,
            options,
        } => cmd_attach(&volume, &data_device, &hash_device, &root_hash, &options)
            .map_err(|e| (e, 1)),
        Command::Detach { volume } => cmd_detach(&volume).map_err(|e| (e, 1)),
    }
}

fn main() {
    match run() {
        Ok(()) => process::exit(0),
        Err((msg, code)) => {
            eprintln!("systemd-veritysetup: {msg}");
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

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
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
        let cmd = parse_args(&args(&[
            "attach",
            "rootverity",
            "/dev/sda1",
            "/dev/sda2",
            "abc123def456",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "rootverity".to_string(),
                data_device: "/dev/sda1".to_string(),
                hash_device: "/dev/sda2".to_string(),
                root_hash: "abc123def456".to_string(),
                options: String::new(),
            }
        );
    }

    #[test]
    fn test_parse_args_attach_with_options() {
        let cmd = parse_args(&args(&[
            "attach",
            "vol",
            "/dev/sda1",
            "/dev/sda2",
            "aabbccdd",
            "ignore-corruption,salt=1234",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "vol".to_string(),
                data_device: "/dev/sda1".to_string(),
                hash_device: "/dev/sda2".to_string(),
                root_hash: "aabbccdd".to_string(),
                options: "ignore-corruption,salt=1234".to_string(),
            }
        );
    }

    #[test]
    fn test_parse_args_attach_missing_volume() {
        assert!(parse_args(&args(&["attach"])).is_err());
    }

    #[test]
    fn test_parse_args_attach_missing_data_device() {
        assert!(parse_args(&args(&["attach", "vol"])).is_err());
    }

    #[test]
    fn test_parse_args_attach_missing_hash_device() {
        assert!(parse_args(&args(&["attach", "vol", "/dev/sda1"])).is_err());
    }

    #[test]
    fn test_parse_args_attach_missing_root_hash() {
        assert!(parse_args(&args(&["attach", "vol", "/dev/sda1", "/dev/sda2"])).is_err());
    }

    #[test]
    fn test_parse_args_detach() {
        let cmd = parse_args(&args(&["detach", "rootverity"])).unwrap();
        assert_eq!(
            cmd,
            Command::Detach {
                volume: "rootverity".to_string()
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
    // parse_options tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_options_empty() {
        let opts = parse_options("").unwrap();
        assert_eq!(opts.format_version, DEFAULT_VERITY_VERSION);
        assert_eq!(opts.hash_algorithm, DEFAULT_HASH_ALGORITHM);
        assert_eq!(opts.data_block_size, DEFAULT_DATA_BLOCK_SIZE);
        assert_eq!(opts.hash_block_size, DEFAULT_HASH_BLOCK_SIZE);
        assert_eq!(opts.data_blocks, 0);
        assert_eq!(opts.hash_offset, 0);
        assert!(opts.salt.is_empty());
        assert!(opts.fec_device.is_none());
        assert_eq!(opts.fec_offset, 0);
        assert_eq!(opts.fec_roots, DEFAULT_FEC_ROOTS);
        assert!(opts.root_hash_signature.is_none());
        assert_eq!(opts.error_mode, ErrorMode::Eio);
        assert!(!opts.ignore_zero_blocks);
        assert!(!opts.check_at_most_once);
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_dash() {
        let opts = parse_options("-").unwrap();
        assert_eq!(opts.format_version, DEFAULT_VERITY_VERSION);
    }

    #[test]
    fn test_parse_options_none() {
        let opts = parse_options("none").unwrap();
        assert_eq!(opts.format_version, DEFAULT_VERITY_VERSION);
    }

    #[test]
    fn test_parse_options_format_v1() {
        let opts = parse_options("format=1").unwrap();
        assert_eq!(opts.format_version, 1);
    }

    #[test]
    fn test_parse_options_format_v2() {
        let opts = parse_options("format=2").unwrap();
        assert_eq!(opts.format_version, 2);
    }

    #[test]
    fn test_parse_options_format_invalid() {
        assert!(parse_options("format=3").is_err());
        assert!(parse_options("format=0").is_err());
        assert!(parse_options("format=abc").is_err());
    }

    #[test]
    fn test_parse_options_hash() {
        let opts = parse_options("hash=sha512").unwrap();
        assert_eq!(opts.hash_algorithm, "sha512");
    }

    #[test]
    fn test_parse_options_data_block_size() {
        let opts = parse_options("data-block-size=1024").unwrap();
        assert_eq!(opts.data_block_size, 1024);
    }

    #[test]
    fn test_parse_options_data_block_size_underscore() {
        let opts = parse_options("data_block_size=1024").unwrap();
        assert_eq!(opts.data_block_size, 1024);
    }

    #[test]
    fn test_parse_options_hash_block_size() {
        let opts = parse_options("hash-block-size=512").unwrap();
        assert_eq!(opts.hash_block_size, 512);
    }

    #[test]
    fn test_parse_options_data_blocks() {
        let opts = parse_options("data-blocks=100000").unwrap();
        assert_eq!(opts.data_blocks, 100000);
    }

    #[test]
    fn test_parse_options_hash_offset() {
        let opts = parse_options("hash-offset=4096").unwrap();
        assert_eq!(opts.hash_offset, 4096);
    }

    #[test]
    fn test_parse_options_salt() {
        let opts = parse_options("salt=abcdef0123456789").unwrap();
        assert_eq!(opts.salt, "abcdef0123456789");
    }

    #[test]
    fn test_parse_options_salt_empty() {
        let opts = parse_options("salt=-").unwrap();
        assert!(opts.salt.is_empty());
    }

    #[test]
    fn test_parse_options_salt_invalid_hex() {
        assert!(parse_options("salt=xyz").is_err());
    }

    #[test]
    fn test_parse_options_fec_device() {
        let opts = parse_options("fec-device=/dev/sda3").unwrap();
        assert_eq!(opts.fec_device, Some(PathBuf::from("/dev/sda3")));
    }

    #[test]
    fn test_parse_options_fec_offset() {
        let opts = parse_options("fec-offset=8192").unwrap();
        assert_eq!(opts.fec_offset, 8192);
    }

    #[test]
    fn test_parse_options_fec_roots() {
        let opts = parse_options("fec-roots=4").unwrap();
        assert_eq!(opts.fec_roots, 4);
    }

    #[test]
    fn test_parse_options_root_hash_signature() {
        let opts = parse_options("root-hash-signature=/path/to/sig.der").unwrap();
        assert_eq!(
            opts.root_hash_signature,
            Some(PathBuf::from("/path/to/sig.der"))
        );
    }

    #[test]
    fn test_parse_options_restart_on_corruption() {
        let opts = parse_options("restart-on-corruption").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::RestartOnCorruption);
    }

    #[test]
    fn test_parse_options_restart_on_corruption_underscore() {
        let opts = parse_options("restart_on_corruption").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::RestartOnCorruption);
    }

    #[test]
    fn test_parse_options_ignore_corruption() {
        let opts = parse_options("ignore-corruption").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::IgnoreCorruption);
    }

    #[test]
    fn test_parse_options_panic_on_corruption() {
        let opts = parse_options("panic-on-corruption").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::PanicOnCorruption);
    }

    #[test]
    fn test_parse_options_ignore_zero_blocks() {
        let opts = parse_options("ignore-zero-blocks").unwrap();
        assert!(opts.ignore_zero_blocks);
    }

    #[test]
    fn test_parse_options_check_at_most_once() {
        let opts = parse_options("check-at-most-once").unwrap();
        assert!(opts.check_at_most_once);
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
    fn test_parse_options_combined() {
        let opts = parse_options(
            "format=1,hash=sha512,data-block-size=1024,hash-block-size=512,\
             data-blocks=50000,hash-offset=4096,salt=aabb,\
             ignore-corruption,ignore-zero-blocks,check-at-most-once",
        )
        .unwrap();

        assert_eq!(opts.format_version, 1);
        assert_eq!(opts.hash_algorithm, "sha512");
        assert_eq!(opts.data_block_size, 1024);
        assert_eq!(opts.hash_block_size, 512);
        assert_eq!(opts.data_blocks, 50000);
        assert_eq!(opts.hash_offset, 4096);
        assert_eq!(opts.salt, "aabb");
        assert_eq!(opts.error_mode, ErrorMode::IgnoreCorruption);
        assert!(opts.ignore_zero_blocks);
        assert!(opts.check_at_most_once);
    }

    #[test]
    fn test_parse_options_noauto_nofail() {
        let opts = parse_options("noauto,nofail").unwrap();
        assert_eq!(opts.format_version, DEFAULT_VERITY_VERSION);
    }

    #[test]
    fn test_parse_options_systemd_extensions_ignored() {
        let opts = parse_options("x-systemd.device-timeout=30").unwrap();
        assert_eq!(opts.format_version, DEFAULT_VERITY_VERSION);
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
        let opts = parse_options(" ignore-corruption , readonly ").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::IgnoreCorruption);
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_empty_parts() {
        let opts = parse_options(",,ignore-corruption,,readonly,,").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::IgnoreCorruption);
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_error_mode_last_wins() {
        let opts = parse_options("ignore-corruption,panic-on-corruption").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::PanicOnCorruption);

        let opts = parse_options("panic-on-corruption,restart-on-corruption").unwrap();
        assert_eq!(opts.error_mode, ErrorMode::RestartOnCorruption);
    }

    #[test]
    fn test_parse_options_fec_combined() {
        let opts = parse_options("fec-device=/dev/sda3,fec-offset=8192,fec-roots=4").unwrap();
        assert_eq!(opts.fec_device, Some(PathBuf::from("/dev/sda3")));
        assert_eq!(opts.fec_offset, 8192);
        assert_eq!(opts.fec_roots, 4);
    }

    // -----------------------------------------------------------------------
    // validate_hex tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_validate_hex_valid() {
        assert!(validate_hex("").is_ok());
        assert!(validate_hex("0123456789abcdef").is_ok());
        assert!(validate_hex("ABCDEF").is_ok());
        assert!(validate_hex("aAbBcCdDeEfF").is_ok());
    }

    #[test]
    fn test_validate_hex_invalid() {
        assert!(validate_hex("xyz").is_err());
        assert!(validate_hex("0123g").is_err());
        assert!(validate_hex("hello world").is_err());
        assert!(validate_hex("abc!").is_err());
    }

    // -----------------------------------------------------------------------
    // ErrorMode Display
    // -----------------------------------------------------------------------

    #[test]
    fn test_error_mode_display() {
        assert_eq!(format!("{}", ErrorMode::Eio), "eio");
        assert_eq!(
            format!("{}", ErrorMode::RestartOnCorruption),
            "restart_on_corruption"
        );
        assert_eq!(
            format!("{}", ErrorMode::IgnoreCorruption),
            "ignore_corruption"
        );
        assert_eq!(
            format!("{}", ErrorMode::PanicOnCorruption),
            "panic_on_corruption"
        );
    }

    // -----------------------------------------------------------------------
    // VerityOptions default tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_verity_options_default() {
        let opts = VerityOptions::default();
        assert_eq!(opts.format_version, 1);
        assert_eq!(opts.hash_algorithm, "sha256");
        assert_eq!(opts.data_block_size, 4096);
        assert_eq!(opts.hash_block_size, 4096);
        assert_eq!(opts.data_blocks, 0);
        assert_eq!(opts.hash_offset, 0);
        assert!(opts.salt.is_empty());
        assert!(opts.fec_device.is_none());
        assert_eq!(opts.fec_offset, 0);
        assert_eq!(opts.fec_roots, 2);
        assert!(opts.root_hash_signature.is_none());
        assert_eq!(opts.error_mode, ErrorMode::Eio);
        assert!(!opts.ignore_zero_blocks);
        assert!(!opts.check_at_most_once);
        assert!(opts.readonly);
    }

    // -----------------------------------------------------------------------
    // build_verity_params tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_verity_params_default() {
        let opts = VerityOptions::default();
        let params = build_verity_params(
            &opts,
            "/dev/sda1",
            "/dev/sda2",
            "aabbccdd",
            1024, // 1024 sectors = 524288 bytes = 128 data blocks at 4096
        );
        // Format: version data_dev hash_dev data_bs hash_bs num_data_blocks hash_start_block algo hash salt
        assert!(params.starts_with("1 /dev/sda1 /dev/sda2 4096 4096 128 0 sha256 aabbccdd -"));
    }

    #[test]
    fn test_build_verity_params_with_salt() {
        let opts = VerityOptions {
            salt: "deadbeef".to_string(),
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains(" deadbeef"));
        assert!(!params.contains(" - ")); // salt should not be "-" placeholder
    }

    #[test]
    fn test_build_verity_params_explicit_data_blocks() {
        let opts = VerityOptions {
            data_blocks: 500,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 10000);
        // num_data_blocks should be 500 (explicit), not calculated.
        assert!(params.contains(" 500 "));
    }

    #[test]
    fn test_build_verity_params_with_hash_offset() {
        let opts = VerityOptions {
            hash_offset: 8192,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        // hash_start_block = 8192 / 4096 = 2
        assert!(params.contains(" 2 sha256 "));
    }

    #[test]
    fn test_build_verity_params_custom_algorithm() {
        let opts = VerityOptions {
            hash_algorithm: "sha512".to_string(),
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains(" sha512 "));
    }

    #[test]
    fn test_build_verity_params_format_v2() {
        let opts = VerityOptions {
            format_version: 2,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.starts_with("2 "));
    }

    #[test]
    fn test_build_verity_params_ignore_corruption() {
        let opts = VerityOptions {
            error_mode: ErrorMode::IgnoreCorruption,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains(" 1 ignore_corruption"));
    }

    #[test]
    fn test_build_verity_params_restart_on_corruption() {
        let opts = VerityOptions {
            error_mode: ErrorMode::RestartOnCorruption,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains("restart_on_corruption"));
    }

    #[test]
    fn test_build_verity_params_panic_on_corruption() {
        let opts = VerityOptions {
            error_mode: ErrorMode::PanicOnCorruption,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains("panic_on_corruption"));
    }

    #[test]
    fn test_build_verity_params_ignore_zero_blocks() {
        let opts = VerityOptions {
            ignore_zero_blocks: true,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains("ignore_zero_blocks"));
    }

    #[test]
    fn test_build_verity_params_check_at_most_once() {
        let opts = VerityOptions {
            check_at_most_once: true,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains("check_at_most_once"));
    }

    #[test]
    fn test_build_verity_params_fec() {
        let opts = VerityOptions {
            fec_device: Some(PathBuf::from("/dev/sda3")),
            fec_offset: 4096,
            fec_roots: 4,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains("use_fec_from_device /dev/sda3"));
        assert!(params.contains("fec_blocks 128"));
        assert!(params.contains("fec_start 8")); // 4096 / 512
        assert!(params.contains("fec_roots 4"));
    }

    #[test]
    fn test_build_verity_params_fec_default_roots() {
        let opts = VerityOptions {
            fec_device: Some(PathBuf::from("/dev/sda3")),
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        // Default fec_roots=2, should NOT include fec_roots in optional params.
        assert!(!params.contains("fec_roots"));
    }

    #[test]
    fn test_build_verity_params_root_hash_signature() {
        let opts = VerityOptions {
            root_hash_signature: Some(PathBuf::from("/path/to/sig.der")),
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        assert!(params.contains("root_hash_sig_key_desc /path/to/sig.der"));
    }

    #[test]
    fn test_build_verity_params_multiple_optional() {
        let opts = VerityOptions {
            error_mode: ErrorMode::IgnoreCorruption,
            ignore_zero_blocks: true,
            check_at_most_once: true,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        // Should have 3 optional params.
        assert!(params.contains(" 3 "));
        assert!(params.contains("ignore_corruption"));
        assert!(params.contains("ignore_zero_blocks"));
        assert!(params.contains("check_at_most_once"));
    }

    #[test]
    fn test_build_verity_params_eio_default_no_extra() {
        let opts = VerityOptions::default();
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        // With EIO (default), no optional params should be appended.
        let base = "1 /dev/sda1 /dev/sda2 4096 4096 128 0 sha256 aabb -".to_string();
        assert_eq!(params, base);
    }

    #[test]
    fn test_build_verity_params_custom_block_sizes() {
        let opts = VerityOptions {
            data_block_size: 512,
            hash_block_size: 512,
            ..Default::default()
        };
        let params = build_verity_params(&opts, "/dev/sda1", "/dev/sda2", "aabb", 1024);
        // 1024 sectors * 512 bytes / 512 block_size = 1024 blocks
        assert!(params.contains(" 512 512 1024 "));
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
        dm_ioctl_init(&mut buf, "test_vol", "CRYPT-VERITY-test", DM_READONLY_FLAG);

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
        assert_eq!(read_u32(&buf, DM_FLAGS_OFFSET), DM_READONLY_FLAG);

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
        assert_eq!(uuid, "CRYPT-VERITY-test");
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
            "verity",
            "1 /dev/sda1 /dev/sda2 4096 4096 128 0 sha256 aabb -",
        );
        assert!(size > 0);
        assert_eq!(size % 8, 0);

        assert_eq!(read_u64(&buf, data_start), 0);
        assert_eq!(read_u64(&buf, data_start + 8), 1000);

        let tt_bytes = &buf[data_start + 24..data_start + 40];
        let tt_end = tt_bytes.iter().position(|&b| b == 0).unwrap_or(16);
        let tt = std::str::from_utf8(&tt_bytes[..tt_end]).unwrap();
        assert_eq!(tt, "verity");
    }

    #[test]
    fn test_append_target_params_content() {
        let mut buf = vec![0u8; 4096];
        let data_start = DM_IOCTL_HEADER_SIZE;
        let params_str = "1 /dev/sda1 /dev/sda2 4096 4096 128 0 sha256 aabb deadbeef";
        let _size = append_target(&mut buf, data_start, 0, 500, "verity", params_str);

        let param_off = data_start + 40;
        let param_end = buf[param_off..].iter().position(|&b| b == 0).unwrap();
        let params = std::str::from_utf8(&buf[param_off..param_off + param_end]).unwrap();
        assert_eq!(params, params_str);
    }

    #[test]
    fn test_append_target_alignment() {
        let mut buf = vec![0u8; 4096];
        let data_start = DM_IOCTL_HEADER_SIZE;
        let size = append_target(&mut buf, data_start, 0, 100, "verity", "x");
        assert_eq!(size % 8, 0);

        let size2 = append_target(
            &mut buf,
            data_start,
            0,
            100,
            "verity",
            "this is a longer parameter string that should be aligned too",
        );
        assert_eq!(size2 % 8, 0);
    }

    // -----------------------------------------------------------------------
    // Integration-style tests (no actual DM operations)
    // -----------------------------------------------------------------------

    #[test]
    fn test_attach_missing_device() {
        let result = cmd_attach(
            "testvol",
            "/nonexistent/data",
            "/nonexistent/hash",
            "aabbccdd",
            "",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_attach_empty_root_hash() {
        let result = cmd_attach("testvol", "/dev/null", "/dev/null", "", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Empty root hash"));
    }

    #[test]
    fn test_attach_invalid_root_hash() {
        let result = cmd_attach("testvol", "/dev/null", "/dev/null", "xyz123", "");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not valid hex"));
    }

    #[test]
    fn test_attach_nonexistent_signature_file() {
        let result = cmd_attach(
            "testvol",
            "/dev/null",
            "/dev/null",
            "aabbccdd",
            "root-hash-signature=/nonexistent/sig.der",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not found"));
    }

    #[test]
    fn test_attach_nonexistent_fec_device() {
        let result = cmd_attach(
            "testvol",
            "/dev/null",
            "/dev/null",
            "aabbccdd",
            "fec-device=/nonexistent/fec",
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("FEC device not found"));
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
            "root_verity",
            "/dev/nvme0n1p3",
            "/dev/nvme0n1p4",
            "a1b2c3d4e5f6",
            "hash=sha512,salt=dead,ignore-corruption,ignore-zero-blocks",
        ]))
        .unwrap();

        if let Command::Attach {
            volume,
            data_device,
            hash_device,
            root_hash,
            options,
        } = cmd
        {
            assert_eq!(volume, "root_verity");
            assert_eq!(data_device, "/dev/nvme0n1p3");
            assert_eq!(hash_device, "/dev/nvme0n1p4");
            assert_eq!(root_hash, "a1b2c3d4e5f6");

            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.hash_algorithm, "sha512");
            assert_eq!(opts.salt, "dead");
            assert_eq!(opts.error_mode, ErrorMode::IgnoreCorruption);
            assert!(opts.ignore_zero_blocks);
        } else {
            panic!("Expected Attach command");
        }
    }

    #[test]
    fn test_full_attach_parse_with_fec() {
        let cmd = parse_args(&args(&[
            "attach",
            "usr_verity",
            "/dev/sda3",
            "/dev/sda4",
            "deadbeefcafe",
            "fec-device=/dev/sda5,fec-roots=4,check-at-most-once",
        ]))
        .unwrap();

        if let Command::Attach {
            volume,
            data_device,
            hash_device,
            root_hash,
            options,
        } = cmd
        {
            assert_eq!(volume, "usr_verity");
            let _data_device = data_device;
            let _hash_device = hash_device;
            let _root_hash = root_hash;
            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.fec_device, Some(PathBuf::from("/dev/sda5")));
            assert_eq!(opts.fec_roots, 4);
            assert!(opts.check_at_most_once);
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
    fn test_default_verity_version() {
        assert_eq!(DEFAULT_VERITY_VERSION, 1);
    }

    #[test]
    fn test_default_hash_algorithm() {
        assert_eq!(DEFAULT_HASH_ALGORITHM, "sha256");
    }

    #[test]
    fn test_default_data_block_size() {
        assert_eq!(DEFAULT_DATA_BLOCK_SIZE, 4096);
    }

    #[test]
    fn test_default_hash_block_size() {
        assert_eq!(DEFAULT_HASH_BLOCK_SIZE, 4096);
    }

    #[test]
    fn test_default_fec_roots() {
        assert_eq!(DEFAULT_FEC_ROOTS, 2);
    }

    #[test]
    fn test_exit_hash_error_code() {
        assert_eq!(EXIT_HASH_ERROR, 2);
    }

    // -----------------------------------------------------------------------
    // read_root_hash_signature tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_root_hash_signature_ok() {
        let dir = tempfile::tempdir().unwrap();
        let sig_path = dir.path().join("sig.der");
        fs::write(&sig_path, b"\x30\x82\x01\x22").unwrap();
        let data = read_root_hash_signature(&sig_path).unwrap();
        assert_eq!(data, b"\x30\x82\x01\x22");
    }

    #[test]
    fn test_read_root_hash_signature_nonexistent() {
        assert!(read_root_hash_signature(Path::new("/nonexistent/sig.der")).is_err());
    }

    #[test]
    fn test_read_root_hash_signature_empty() {
        let dir = tempfile::tempdir().unwrap();
        let sig_path = dir.path().join("empty.der");
        fs::write(&sig_path, b"").unwrap();
        let data = read_root_hash_signature(&sig_path).unwrap();
        assert!(data.is_empty());
    }
}
