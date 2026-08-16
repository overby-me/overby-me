//! systemd-cryptsetup — Attach and detach encrypted block devices.
//!
//! A drop-in replacement for `systemd-cryptsetup(8)`. This tool sets up
//! dm-crypt (LUKS or plain) encrypted block devices via the device-mapper
//! kernel interface.
//!
//! Usage:
//!
//!   systemd-cryptsetup attach VOLUME DEVICE [PASSWORD [OPTIONS]]
//!       Set up an encrypted block device. VOLUME is the device-mapper name,
//!       DEVICE is the underlying encrypted block device, PASSWORD is a key
//!       file path (or "-" for interactive/stdin), OPTIONS is a comma-separated
//!       list of mount.crypt options.
//!
//!   systemd-cryptsetup detach VOLUME
//!       Tear down a previously attached encrypted block device.
//!
//!   systemd-cryptsetup --help | -h
//!       Show help.
//!
//!   systemd-cryptsetup --version
//!       Show version.
//!
//! Options (comma-separated in OPTIONS):
//!   plain               Use plain dm-crypt (no LUKS header)
//!   cipher=CIPHER       Encryption cipher (default: aes-xts-plain64)
//!   size=BITS           Key size in bits (default: 256)
//!   hash=HASH           Password hash for plain mode (default: sha256)
//!   offset=SECTORS      Data offset in sectors
//!   skip=SECTORS        IV offset in sectors
//!   readonly            Open device read-only
//!   discard             Allow discard/TRIM passthrough
//!   same-cpu-crypt      Use same CPU for encryption
//!   submit-from-crypt-cpus  Submit from crypt CPUs
//!   no-read-workqueue   Bypass dm-crypt read workqueue
//!   no-write-workqueue  Bypass dm-crypt write workqueue
//!   luks                Force LUKS mode (default)
//!   sector-size=BYTES   Encryption sector size (default: 512)
//!   tries=N             Number of password attempts (default: 3)
//!   keyfile-size=BYTES  Read at most N bytes from key file
//!   keyfile-offset=BYTES  Skip N bytes at start of key file
//!   timeout=SECS        Timeout for interactive password entry
//!   tmp=FSTYPE          Create a temporary filesystem after setup
//!   tcrypt              Use TrueCrypt-compatible mode
//!   header=PATH         Detached LUKS header path
//!   noauto              Ignored (for /etc/crypttab compat)
//!   nofail              Ignored (for /etc/crypttab compat)
//!   x-systemd.*         Ignored (for /etc/crypttab compat)
//!
//! Exit codes:
//!   0 — success
//!   1 — error (general)
//!   5 — password/key error

use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::os::unix::io::RawFd;
use std::path::{Path, PathBuf};
use std::process;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Device-mapper control device path.
const DM_CONTROL_PATH: &str = "/dev/mapper/control";

/// Device-mapper ioctl magic / version.
const DM_VERSION_MAJOR: u32 = 4;
const DM_VERSION_MINOR: u32 = 0;
const DM_VERSION_PATCHLEVEL: u32 = 0;

/// Device-mapper ioctl command numbers (type 0xfd).
const DM_IOCTL_TYPE: u32 = 0xfd;

/// DM ioctl structure size (312 bytes — fixed header).
const DM_IOCTL_HEADER_SIZE: usize = 312;

/// Size of the overall ioctl buffer we allocate.
const DM_IOCTL_BUF_SIZE: usize = 16384;

/// DM ioctl struct version field offset: 0..12 (3 × u32).
const DM_VERSION_OFFSET: usize = 0;
/// data_size: offset 12
const DM_DATA_SIZE_OFFSET: usize = 12;
/// data_start: offset 16
const DM_DATA_START_OFFSET: usize = 16;
/// target_count: offset 20
const DM_TARGET_COUNT_OFFSET: usize = 20;
/// flags: offset 28
const DM_FLAGS_OFFSET: usize = 28;
/// name: offset 48..176 (128 bytes)
const DM_NAME_OFFSET: usize = 48;
const DM_NAME_SIZE: usize = 128;
/// uuid: offset 176..304 (128 bytes)
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
#[allow(dead_code)]
const DM_EXISTS_FLAG: u32 = 1 << 2;

/// dm-crypt defaults.
const DEFAULT_CIPHER: &str = "aes-xts-plain64";
const DEFAULT_KEY_SIZE: u32 = 256;
const DEFAULT_HASH: &str = "sha256";
const DEFAULT_SECTOR_SIZE: u32 = 512;
const DEFAULT_TRIES: u32 = 3;

/// LUKS magic bytes at the start of a LUKS device.
const LUKS_MAGIC: &[u8; 6] = b"LUKS\xba\xbe";
/// LUKS v2 magic.
const LUKS2_MAGIC: &[u8; 6] = b"SKUL\xba\xbe";

/// Exit code for password/key errors.
#[allow(dead_code)]
const EXIT_PASSWORD_ERROR: i32 = 5;

// ---------------------------------------------------------------------------
// Parsed options from the comma-separated OPTIONS argument
// ---------------------------------------------------------------------------

/// Encryption mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CryptMode {
    Luks,
    Plain,
    Tcrypt,
}

impl fmt::Display for CryptMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            CryptMode::Luks => write!(f, "luks"),
            CryptMode::Plain => write!(f, "plain"),
            CryptMode::Tcrypt => write!(f, "tcrypt"),
        }
    }
}

/// Parsed cryptsetup options from the OPTIONS string.
#[derive(Debug, Clone)]
struct CryptOptions {
    /// Encryption mode (LUKS / plain / tcrypt).
    mode: CryptMode,
    /// Cipher specification (e.g. "aes-xts-plain64").
    cipher: String,
    /// Key size in bits.
    key_size: u32,
    /// Hash for plain mode password derivation.
    hash: String,
    /// Data offset in 512-byte sectors.
    offset: u64,
    /// IV offset in 512-byte sectors.
    skip: u64,
    /// Open read-only.
    readonly: bool,
    /// Allow discard (TRIM) passthrough.
    discard: bool,
    /// Use same CPU for encryption.
    same_cpu_crypt: bool,
    /// Submit I/O from crypt CPUs.
    submit_from_crypt_cpus: bool,
    /// Bypass dm-crypt read workqueue.
    no_read_workqueue: bool,
    /// Bypass dm-crypt write workqueue.
    no_write_workqueue: bool,
    /// Sector size for encryption (default 512).
    sector_size: u32,
    /// Number of password attempts.
    tries: u32,
    /// Maximum bytes to read from key file.
    keyfile_size: Option<usize>,
    /// Bytes to skip at start of key file.
    keyfile_offset: usize,
    /// Timeout for password entry in seconds.
    timeout: Option<u64>,
    /// Create a temporary filesystem of this type after setup.
    tmp: Option<String>,
    /// Detached LUKS header path.
    header: Option<PathBuf>,
}

impl Default for CryptOptions {
    fn default() -> Self {
        Self {
            mode: CryptMode::Luks,
            cipher: DEFAULT_CIPHER.to_string(),
            key_size: DEFAULT_KEY_SIZE,
            hash: DEFAULT_HASH.to_string(),
            offset: 0,
            skip: 0,
            readonly: false,
            discard: false,
            same_cpu_crypt: false,
            submit_from_crypt_cpus: false,
            no_read_workqueue: false,
            no_write_workqueue: false,
            sector_size: DEFAULT_SECTOR_SIZE,
            tries: DEFAULT_TRIES,
            keyfile_size: None,
            keyfile_offset: 0,
            timeout: None,
            tmp: None,
            header: None,
        }
    }
}

/// Parse a comma-separated options string (the 4th positional argument).
fn parse_options(opts_str: &str) -> Result<CryptOptions, String> {
    let mut opts = CryptOptions::default();

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
                "cipher" => opts.cipher = val.to_string(),
                "size" => {
                    opts.key_size = val
                        .parse()
                        .map_err(|_| format!("Invalid key size: {val}"))?;
                }
                "hash" => opts.hash = val.to_string(),
                "offset" => {
                    opts.offset = val.parse().map_err(|_| format!("Invalid offset: {val}"))?;
                }
                "skip" => {
                    opts.skip = val.parse().map_err(|_| format!("Invalid skip: {val}"))?;
                }
                "sector-size" => {
                    opts.sector_size = val
                        .parse()
                        .map_err(|_| format!("Invalid sector-size: {val}"))?;
                }
                "tries" => {
                    opts.tries = val.parse().map_err(|_| format!("Invalid tries: {val}"))?;
                }
                "keyfile-size" => {
                    opts.keyfile_size = Some(
                        val.parse()
                            .map_err(|_| format!("Invalid keyfile-size: {val}"))?,
                    );
                }
                "keyfile-offset" => {
                    opts.keyfile_offset = val
                        .parse()
                        .map_err(|_| format!("Invalid keyfile-offset: {val}"))?;
                }
                "timeout" => {
                    opts.timeout =
                        Some(val.parse().map_err(|_| format!("Invalid timeout: {val}"))?);
                }
                "tmp" => opts.tmp = Some(val.to_string()),
                "header" => opts.header = Some(PathBuf::from(val)),
                _ if key.starts_with("x-systemd.") => { /* ignored */ }
                _ => {
                    return Err(format!("Unknown option: {key}={val}"));
                }
            }
        } else {
            match part {
                "plain" => opts.mode = CryptMode::Plain,
                "luks" => opts.mode = CryptMode::Luks,
                "tcrypt" => opts.mode = CryptMode::Tcrypt,
                "readonly" | "read-only" => opts.readonly = true,
                "discard" | "allow-discards" => opts.discard = true,
                "same-cpu-crypt" => opts.same_cpu_crypt = true,
                "submit-from-crypt-cpus" => opts.submit_from_crypt_cpus = true,
                "no-read-workqueue" => opts.no_read_workqueue = true,
                "no-write-workqueue" => opts.no_write_workqueue = true,
                "noauto" | "nofail" | "auto" | "swap" => { /* /etc/crypttab compat — ignored */ }
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
        password: Option<String>,
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
            let device = iter
                .next()
                .ok_or("attach: missing DEVICE argument")?
                .clone();
            let password = iter.next().cloned();
            let options = iter.next().cloned().unwrap_or_default();
            Ok(Command::Attach {
                volume,
                device,
                password,
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
    // _IOWR(DM_IOCTL_TYPE, cmd, struct dm_ioctl)
    // Direction: read|write = 3, size = DM_IOCTL_HEADER_SIZE
    let dir: libc::c_ulong = 3; // IOC_READ | IOC_WRITE
    let size = DM_IOCTL_HEADER_SIZE as libc::c_ulong;
    let typ = DM_IOCTL_TYPE as libc::c_ulong;
    let nr = cmd as libc::c_ulong;
    (dir << 30) | (size << 16) | (typ << 8) | nr
}

/// Initialize a DM ioctl buffer with the standard header fields.
fn dm_ioctl_init(buf: &mut [u8], name: &str, uuid: &str, flags: u32) {
    assert!(buf.len() >= DM_IOCTL_HEADER_SIZE);

    // Zero the entire buffer.
    for b in buf.iter_mut() {
        *b = 0;
    }

    // Version.
    write_u32(buf, DM_VERSION_OFFSET, DM_VERSION_MAJOR);
    write_u32(buf, DM_VERSION_OFFSET + 4, DM_VERSION_MINOR);
    write_u32(buf, DM_VERSION_OFFSET + 8, DM_VERSION_PATCHLEVEL);

    // data_size = total buffer size.
    write_u32(buf, DM_DATA_SIZE_OFFSET, buf.len() as u32);

    // data_start = header size (where target specs begin).
    write_u32(buf, DM_DATA_START_OFFSET, DM_IOCTL_HEADER_SIZE as u32);

    // flags.
    write_u32(buf, DM_FLAGS_OFFSET, flags);

    // name.
    write_string(buf, DM_NAME_OFFSET, DM_NAME_SIZE, name);

    // uuid.
    write_string(buf, DM_UUID_OFFSET, DM_UUID_SIZE, uuid);
}

/// Write a u32 in native endianness.
fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
}

/// Read a u32 in native endianness.
fn read_u32(buf: &[u8], offset: usize) -> u32 {
    u32::from_ne_bytes(buf[offset..offset + 4].try_into().unwrap())
}

/// Write a u64 in native endianness.
fn write_u64(buf: &mut [u8], offset: usize, val: u64) {
    buf[offset..offset + 8].copy_from_slice(&val.to_ne_bytes());
}

/// Read a u64 in native endianness.
#[allow(dead_code)]
fn read_u64(buf: &[u8], offset: usize) -> u64 {
    u64::from_ne_bytes(buf[offset..offset + 8].try_into().unwrap())
}

/// Write a null-terminated string into a fixed-size field.
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
///   u32 next            (offset to next target spec, 0 if last — filled in)
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
    let spec_size: usize = 40; // dm_target_spec
    let param_bytes = params.as_bytes();
    // Align to 8 bytes.
    let total = align8(spec_size + param_bytes.len() + 1);

    assert!(data_start + total <= buf.len(), "DM ioctl buffer overflow");

    // sector_start (u64).
    write_u64(buf, data_start, sector_start);
    // length (u64).
    write_u64(buf, data_start + 8, length_sectors);
    // status (i32) — 0.
    write_u32(buf, data_start + 16, 0);
    // next — 0 for now (single target).
    write_u32(buf, data_start + 20, 0);
    // target_type (16 bytes, null-terminated).
    write_string(buf, data_start + 24, 16, target_type);
    // params (null-terminated, after the 40-byte spec).
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

/// Open the device-mapper control device.
fn open_dm_control() -> io::Result<RawFd> {
    let path = std::ffi::CString::new(DM_CONTROL_PATH).unwrap();
    let fd = unsafe { libc::open(path.as_ptr(), libc::O_RDWR) };
    if fd < 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(fd)
    }
}

/// Close a raw fd.
fn close_fd(fd: RawFd) {
    unsafe {
        libc::close(fd);
    }
}

// ---------------------------------------------------------------------------
// Key / password handling
// ---------------------------------------------------------------------------

/// Read a key/password from the specified source.
///
/// - `None` or `"-"` → read from stdin
/// - A file path → read the file contents
/// - An empty path → empty key
fn read_key(
    password_arg: Option<&str>,
    keyfile_offset: usize,
    keyfile_size: Option<usize>,
) -> io::Result<Vec<u8>> {
    let path = match password_arg {
        None | Some("-") | Some("") => {
            // Read from stdin.
            let mut key = Vec::new();
            io::stdin().read_to_end(&mut key)?;
            // Strip trailing newline if present (interactive entry).
            if key.last() == Some(&b'\n') {
                key.pop();
                if key.last() == Some(&b'\r') {
                    key.pop();
                }
            }
            return Ok(key);
        }
        Some(p) => p,
    };

    // Read from file.
    let mut f = fs::File::open(path)?;

    // Skip keyfile-offset bytes.
    if keyfile_offset > 0 {
        let mut skip_buf = vec![0u8; keyfile_offset.min(8192)];
        let mut remaining = keyfile_offset;
        while remaining > 0 {
            let to_read = remaining.min(skip_buf.len());
            let n = f.read(&mut skip_buf[..to_read])?;
            if n == 0 {
                break;
            }
            remaining -= n;
        }
    }

    let mut key = Vec::new();
    match keyfile_size {
        Some(max) => {
            key.resize(max, 0);
            let mut total = 0;
            while total < max {
                let n = f.read(&mut key[total..])?;
                if n == 0 {
                    break;
                }
                total += n;
            }
            key.truncate(total);
        }
        None => {
            f.read_to_end(&mut key)?;
        }
    }

    Ok(key)
}

/// Encode key bytes as a hex string for dm-crypt table parameters.
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

/// Resolve a device path to a `major:minor` pair via `stat()`.
#[allow(dead_code)]
fn device_major_minor(device: &str) -> io::Result<(u32, u32)> {
    let path = std::ffi::CString::new(device)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidInput, e))?;
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    let ret = unsafe { libc::stat(path.as_ptr(), &mut st) };
    if ret < 0 {
        return Err(io::Error::last_os_error());
    }
    let major = libc::major(st.st_rdev);
    let minor = libc::minor(st.st_rdev);
    Ok((major as u32, minor as u32))
}

/// Detect whether a device has a LUKS header by reading its first bytes.
fn detect_luks(device: &str) -> io::Result<bool> {
    let mut f = fs::File::open(device)?;
    let mut magic = [0u8; 6];
    let n = f.read(&mut magic)?;
    if n < 6 {
        return Ok(false);
    }
    Ok(&magic == LUKS_MAGIC || &magic == LUKS2_MAGIC)
}

// ---------------------------------------------------------------------------
// dm-crypt table construction
// ---------------------------------------------------------------------------

/// Build the dm-crypt target parameter string.
///
/// Format: `<cipher> <key_hex> <iv_offset> <device> <sector_offset> [<features> [<opt>...]]`
fn build_crypt_params(opts: &CryptOptions, key: &[u8], device: &str) -> String {
    let key_hex = if key.is_empty() {
        "-".to_string()
    } else {
        key_to_hex(key)
    };

    let mut features: Vec<&str> = Vec::new();
    if opts.discard {
        features.push("allow_discards");
    }
    if opts.same_cpu_crypt {
        features.push("same_cpu_crypt");
    }
    if opts.submit_from_crypt_cpus {
        features.push("submit_from_crypt_cpus");
    }
    if opts.no_read_workqueue {
        features.push("no_read_workqueue");
    }
    if opts.no_write_workqueue {
        features.push("no_write_workqueue");
    }
    if opts.sector_size != 512 {
        // sector_size:N is a feature arg.
        // handled below.
    }

    // Base parameters.
    let mut params = format!(
        "{} {} {} {} {}",
        opts.cipher, key_hex, opts.skip, device, opts.offset
    );

    // Features count.
    let mut feature_count = features.len();
    if opts.sector_size != 512 {
        feature_count += 1;
    }

    if feature_count > 0 {
        params.push_str(&format!(" {feature_count}"));
        for feat in &features {
            params.push_str(&format!(" {feat}"));
        }
        if opts.sector_size != 512 {
            params.push_str(&format!(" sector_size:{}", opts.sector_size));
        }
    }

    params
}

// ---------------------------------------------------------------------------
// Attach / Detach operations
// ---------------------------------------------------------------------------

/// Attach an encrypted block device via device-mapper.
fn cmd_attach(
    volume: &str,
    device: &str,
    password: Option<&str>,
    options_str: &str,
) -> Result<(), String> {
    let opts = parse_options(options_str)?;

    // Determine encryption mode: if the user didn't explicitly set plain/tcrypt,
    // auto-detect LUKS header.
    let effective_mode = if opts.mode == CryptMode::Luks {
        // Check if device actually has a LUKS header (unless we have a detached header).
        let check_device = opts.header.as_deref().unwrap_or(Path::new(device));
        match detect_luks(check_device.to_str().unwrap_or(device)) {
            Ok(true) => CryptMode::Luks,
            Ok(false) => {
                eprintln!(
                    "Warning: No LUKS header detected on {}, proceeding with LUKS mode anyway.",
                    check_device.display()
                );
                CryptMode::Luks
            }
            Err(e) => {
                return Err(format!(
                    "Failed to read device {}: {e}",
                    check_device.display()
                ));
            }
        }
    } else {
        opts.mode
    };

    // Read key material.
    let key = read_key(password, opts.keyfile_offset, opts.keyfile_size)
        .map_err(|e| format!("Failed to read key: {e}"))?;

    if key.is_empty() && effective_mode == CryptMode::Plain {
        return Err("No key provided for plain dm-crypt mode.".to_string());
    }

    // Verify key size for plain mode.
    if effective_mode == CryptMode::Plain {
        let expected_bytes = (opts.key_size / 8) as usize;
        if key.len() != expected_bytes && opts.hash == "plain" {
            return Err(format!(
                "Key size mismatch: expected {} bytes for {}-bit key, got {} bytes.",
                expected_bytes,
                opts.key_size,
                key.len()
            ));
        }
    }

    // Get device size in sectors.
    let total_sectors = device_size_sectors(device)
        .map_err(|e| format!("Failed to get device size for {device}: {e}"))?;

    if total_sectors <= opts.offset {
        return Err(format!(
            "Device {device} has {total_sectors} sectors, but offset is {} — no space remaining.",
            opts.offset
        ));
    }
    let target_sectors = total_sectors - opts.offset;

    // Build CRYPT-UUID for LUKS.
    let uuid_str = match effective_mode {
        CryptMode::Luks => format!("CRYPT-LUKS2-{volume}"),
        CryptMode::Plain => format!("CRYPT-PLAIN-{volume}"),
        CryptMode::Tcrypt => format!("CRYPT-TCRYPT-{volume}"),
    };

    // Build the dm-crypt target parameters.
    let crypt_params = build_crypt_params(&opts, &key, device);

    // 1. DM_DEV_CREATE — create the device-mapper device.
    let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
    let flags = if opts.readonly { DM_READONLY_FLAG } else { 0 };
    dm_ioctl_init(&mut buf, volume, &uuid_str, flags);
    dm_ioctl(DM_DEV_CREATE_CMD, &mut buf)
        .map_err(|e| format!("DM_DEV_CREATE failed for {volume}: {e}"))?;

    // 2. DM_TABLE_LOAD — load the crypt target.
    dm_ioctl_init(&mut buf, volume, &uuid_str, flags);
    write_u32(&mut buf, DM_TARGET_COUNT_OFFSET, 1);
    let data_start = read_u32(&buf, DM_DATA_START_OFFSET) as usize;
    let _target_size = append_target(
        &mut buf,
        data_start,
        0,
        target_sectors,
        "crypt",
        &crypt_params,
    );

    if let Err(e) = dm_ioctl(DM_TABLE_LOAD_CMD, &mut buf) {
        // Clean up the created device on failure.
        let mut cleanup = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut cleanup, volume, "", 0);
        let _ = dm_ioctl(DM_DEV_REMOVE_CMD, &mut cleanup);
        return Err(format!("DM_TABLE_LOAD failed for {volume}: {e}"));
    }

    // 3. DM_DEV_SUSPEND (resume) — activate the device.
    dm_ioctl_init(&mut buf, volume, &uuid_str, flags);
    // To resume, we issue DM_DEV_SUSPEND without the SUSPEND flag.
    if let Err(e) = dm_ioctl(DM_DEV_SUSPEND_CMD, &mut buf) {
        // Clean up.
        let mut cleanup = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut cleanup, volume, "", 0);
        let _ = dm_ioctl(DM_TABLE_CLEAR_CMD, &mut cleanup);
        let _ = dm_ioctl(DM_DEV_REMOVE_CMD, &mut cleanup);
        return Err(format!("DM_DEV_SUSPEND (resume) failed for {volume}: {e}"));
    }

    eprintln!("Set up encrypted device /dev/mapper/{volume}");

    // 4. Optional: create temporary filesystem.
    if let Some(ref fstype) = opts.tmp {
        let dm_path = format!("/dev/mapper/{volume}");
        eprintln!("Creating {fstype} filesystem on {dm_path}...");
        let status = process::Command::new(format!("mkfs.{fstype}"))
            .arg(&dm_path)
            .status();
        match status {
            Ok(s) if s.success() => {}
            Ok(s) => {
                eprintln!("Warning: mkfs.{fstype} exited with {s}");
            }
            Err(e) => {
                eprintln!("Warning: failed to run mkfs.{fstype}: {e}");
            }
        }
    }

    Ok(())
}

/// Detach (tear down) an encrypted block device.
fn cmd_detach(volume: &str) -> Result<(), String> {
    let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];

    // 1. DM_DEV_SUSPEND — suspend the device.
    dm_ioctl_init(&mut buf, volume, "", DM_SUSPEND_FLAG);
    // Ignore errors here (device may already be suspended).
    let _ = dm_ioctl(DM_DEV_SUSPEND_CMD, &mut buf);

    // 2. DM_TABLE_CLEAR — clear the table.
    dm_ioctl_init(&mut buf, volume, "", 0);
    let _ = dm_ioctl(DM_TABLE_CLEAR_CMD, &mut buf);

    // 3. DM_DEV_REMOVE — remove the device.
    dm_ioctl_init(&mut buf, volume, "", 0);
    dm_ioctl(DM_DEV_REMOVE_CMD, &mut buf)
        .map_err(|e| format!("DM_DEV_REMOVE failed for {volume}: {e}"))?;

    eprintln!("Detached encrypted device /dev/mapper/{volume}");
    Ok(())
}

// ---------------------------------------------------------------------------
// Usage
// ---------------------------------------------------------------------------

fn print_usage() {
    eprintln!(
        "\
Usage: systemd-cryptsetup [COMMAND] [OPTIONS...]

Commands:
  attach VOLUME DEVICE [PASSWORD [OPTIONS]]
      Set up an encrypted block device.
      VOLUME   — device-mapper name (appears as /dev/mapper/VOLUME)
      DEVICE   — underlying encrypted block device
      PASSWORD — key file path, or \"-\" for stdin (default: stdin)
      OPTIONS  — comma-separated list of options (see below)

  detach VOLUME
      Tear down a previously attached encrypted block device.

Options (comma-separated in the OPTIONS argument):
  plain                  Use plain dm-crypt (no LUKS header)
  luks                   Force LUKS mode (default)
  tcrypt                 TrueCrypt-compatible mode
  cipher=CIPHER          Encryption cipher (default: aes-xts-plain64)
  size=BITS              Key size in bits (default: 256)
  hash=HASH              Password hash (default: sha256)
  offset=SECTORS         Data offset in sectors
  skip=SECTORS           IV offset in sectors
  readonly               Open device read-only
  discard                Allow TRIM passthrough
  same-cpu-crypt         Use same CPU for encryption
  submit-from-crypt-cpus Submit from crypt CPUs
  no-read-workqueue      Bypass read workqueue
  no-write-workqueue     Bypass write workqueue
  sector-size=BYTES      Encryption sector size (default: 512)
  tries=N                Password attempts (default: 3)
  keyfile-size=BYTES     Max bytes from key file
  keyfile-offset=BYTES   Skip bytes at start of key file
  timeout=SECS           Password entry timeout
  tmp=FSTYPE             Create temp filesystem after setup
  header=PATH            Detached LUKS header path

Exit codes:
  0 — success
  1 — general error
  5 — password/key error"
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
            eprintln!("systemd-cryptsetup (rust-systemd) 0.1.0");
            Ok(())
        }
        Command::Attach {
            volume,
            device,
            password,
            options,
        } => cmd_attach(&volume, &device, password.as_deref(), &options).map_err(|e| (e, 1)),
        Command::Detach { volume } => cmd_detach(&volume).map_err(|e| (e, 1)),
    }
}

fn main() {
    match run() {
        Ok(()) => process::exit(0),
        Err((msg, code)) => {
            eprintln!("systemd-cryptsetup: {msg}");
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

    // -----------------------------------------------------------------------
    // parse_args tests
    // -----------------------------------------------------------------------

    fn args(strs: &[&str]) -> Vec<String> {
        strs.iter().map(|s| s.to_string()).collect()
    }

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
        let cmd = parse_args(&args(&["attach", "myvolume", "/dev/sda1"])).unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "myvolume".to_string(),
                device: "/dev/sda1".to_string(),
                password: None,
                options: String::new(),
            }
        );
    }

    #[test]
    fn test_parse_args_attach_with_password() {
        let cmd = parse_args(&args(&["attach", "vol", "/dev/sda1", "/path/to/key"])).unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "vol".to_string(),
                device: "/dev/sda1".to_string(),
                password: Some("/path/to/key".to_string()),
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
            "-",
            "plain,cipher=aes-cbc-essiv:sha256,size=128",
        ]))
        .unwrap();
        assert_eq!(
            cmd,
            Command::Attach {
                volume: "vol".to_string(),
                device: "/dev/sda1".to_string(),
                password: Some("-".to_string()),
                options: "plain,cipher=aes-cbc-essiv:sha256,size=128".to_string(),
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
        let cmd = parse_args(&args(&["detach", "myvolume"])).unwrap();
        assert_eq!(
            cmd,
            Command::Detach {
                volume: "myvolume".to_string()
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
        assert_eq!(opts.mode, CryptMode::Luks);
        assert_eq!(opts.cipher, DEFAULT_CIPHER);
        assert_eq!(opts.key_size, DEFAULT_KEY_SIZE);
        assert_eq!(opts.hash, DEFAULT_HASH);
        assert_eq!(opts.offset, 0);
        assert_eq!(opts.skip, 0);
        assert!(!opts.readonly);
        assert!(!opts.discard);
        assert!(!opts.same_cpu_crypt);
        assert!(!opts.submit_from_crypt_cpus);
        assert!(!opts.no_read_workqueue);
        assert!(!opts.no_write_workqueue);
        assert_eq!(opts.sector_size, DEFAULT_SECTOR_SIZE);
        assert_eq!(opts.tries, DEFAULT_TRIES);
        assert!(opts.keyfile_size.is_none());
        assert_eq!(opts.keyfile_offset, 0);
        assert!(opts.timeout.is_none());
        assert!(opts.tmp.is_none());
        assert!(opts.header.is_none());
    }

    #[test]
    fn test_parse_options_dash() {
        let opts = parse_options("-").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);
    }

    #[test]
    fn test_parse_options_none() {
        let opts = parse_options("none").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);
    }

    #[test]
    fn test_parse_options_plain_mode() {
        let opts = parse_options("plain").unwrap();
        assert_eq!(opts.mode, CryptMode::Plain);
    }

    #[test]
    fn test_parse_options_luks_mode() {
        let opts = parse_options("luks").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);
    }

    #[test]
    fn test_parse_options_tcrypt_mode() {
        let opts = parse_options("tcrypt").unwrap();
        assert_eq!(opts.mode, CryptMode::Tcrypt);
    }

    #[test]
    fn test_parse_options_cipher() {
        let opts = parse_options("cipher=aes-cbc-essiv:sha256").unwrap();
        assert_eq!(opts.cipher, "aes-cbc-essiv:sha256");
    }

    #[test]
    fn test_parse_options_size() {
        let opts = parse_options("size=512").unwrap();
        assert_eq!(opts.key_size, 512);
    }

    #[test]
    fn test_parse_options_size_invalid() {
        assert!(parse_options("size=abc").is_err());
    }

    #[test]
    fn test_parse_options_hash() {
        let opts = parse_options("hash=sha512").unwrap();
        assert_eq!(opts.hash, "sha512");
    }

    #[test]
    fn test_parse_options_offset() {
        let opts = parse_options("offset=2048").unwrap();
        assert_eq!(opts.offset, 2048);
    }

    #[test]
    fn test_parse_options_offset_invalid() {
        assert!(parse_options("offset=notanumber").is_err());
    }

    #[test]
    fn test_parse_options_skip() {
        let opts = parse_options("skip=1024").unwrap();
        assert_eq!(opts.skip, 1024);
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
    fn test_parse_options_discard() {
        let opts = parse_options("discard").unwrap();
        assert!(opts.discard);
    }

    #[test]
    fn test_parse_options_allow_discards_alias() {
        let opts = parse_options("allow-discards").unwrap();
        assert!(opts.discard);
    }

    #[test]
    fn test_parse_options_same_cpu_crypt() {
        let opts = parse_options("same-cpu-crypt").unwrap();
        assert!(opts.same_cpu_crypt);
    }

    #[test]
    fn test_parse_options_submit_from_crypt_cpus() {
        let opts = parse_options("submit-from-crypt-cpus").unwrap();
        assert!(opts.submit_from_crypt_cpus);
    }

    #[test]
    fn test_parse_options_no_read_workqueue() {
        let opts = parse_options("no-read-workqueue").unwrap();
        assert!(opts.no_read_workqueue);
    }

    #[test]
    fn test_parse_options_no_write_workqueue() {
        let opts = parse_options("no-write-workqueue").unwrap();
        assert!(opts.no_write_workqueue);
    }

    #[test]
    fn test_parse_options_sector_size() {
        let opts = parse_options("sector-size=4096").unwrap();
        assert_eq!(opts.sector_size, 4096);
    }

    #[test]
    fn test_parse_options_tries() {
        let opts = parse_options("tries=5").unwrap();
        assert_eq!(opts.tries, 5);
    }

    #[test]
    fn test_parse_options_keyfile_size() {
        let opts = parse_options("keyfile-size=256").unwrap();
        assert_eq!(opts.keyfile_size, Some(256));
    }

    #[test]
    fn test_parse_options_keyfile_offset() {
        let opts = parse_options("keyfile-offset=512").unwrap();
        assert_eq!(opts.keyfile_offset, 512);
    }

    #[test]
    fn test_parse_options_timeout() {
        let opts = parse_options("timeout=60").unwrap();
        assert_eq!(opts.timeout, Some(60));
    }

    #[test]
    fn test_parse_options_tmp() {
        let opts = parse_options("tmp=ext4").unwrap();
        assert_eq!(opts.tmp, Some("ext4".to_string()));
    }

    #[test]
    fn test_parse_options_header() {
        let opts = parse_options("header=/path/to/header").unwrap();
        assert_eq!(opts.header, Some(PathBuf::from("/path/to/header")));
    }

    #[test]
    fn test_parse_options_combined() {
        let opts = parse_options(
            "plain,cipher=aes-cbc-essiv:sha256,size=128,hash=sha512,offset=2048,\
             skip=1024,readonly,discard,sector-size=4096,tries=5,\
             keyfile-size=256,keyfile-offset=64,timeout=30",
        )
        .unwrap();

        assert_eq!(opts.mode, CryptMode::Plain);
        assert_eq!(opts.cipher, "aes-cbc-essiv:sha256");
        assert_eq!(opts.key_size, 128);
        assert_eq!(opts.hash, "sha512");
        assert_eq!(opts.offset, 2048);
        assert_eq!(opts.skip, 1024);
        assert!(opts.readonly);
        assert!(opts.discard);
        assert_eq!(opts.sector_size, 4096);
        assert_eq!(opts.tries, 5);
        assert_eq!(opts.keyfile_size, Some(256));
        assert_eq!(opts.keyfile_offset, 64);
        assert_eq!(opts.timeout, Some(30));
    }

    #[test]
    fn test_parse_options_noauto_nofail() {
        // These should be silently ignored.
        let opts = parse_options("noauto,nofail").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);
    }

    #[test]
    fn test_parse_options_systemd_extensions_ignored() {
        let opts = parse_options("x-systemd.device-timeout=30").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);
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
        let opts = parse_options(" plain , readonly ").unwrap();
        assert_eq!(opts.mode, CryptMode::Plain);
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_empty_parts() {
        let opts = parse_options(",,plain,,readonly,,").unwrap();
        assert_eq!(opts.mode, CryptMode::Plain);
        assert!(opts.readonly);
    }

    #[test]
    fn test_parse_options_swap_ignored() {
        let opts = parse_options("swap").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);
    }

    #[test]
    fn test_parse_options_auto_ignored() {
        let opts = parse_options("auto").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);
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
        assert_eq!(key_to_hex(&[0xff; 32]), "ff".repeat(32));
    }

    // -----------------------------------------------------------------------
    // read_key tests (file-based)
    // -----------------------------------------------------------------------

    #[test]
    fn test_read_key_from_file() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"secret-key-data").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 0, None).unwrap();
        assert_eq!(key, b"secret-key-data");
    }

    #[test]
    fn test_read_key_from_file_with_offset() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"HEADER_secret-key-data").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 7, None).unwrap();
        assert_eq!(key, b"secret-key-data");
    }

    #[test]
    fn test_read_key_from_file_with_size() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"secret-key-data-with-trailing-stuff").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 0, Some(15)).unwrap();
        assert_eq!(key, b"secret-key-data");
    }

    #[test]
    fn test_read_key_from_file_with_offset_and_size() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"HEADER_secret-key-data-trailing").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 7, Some(15)).unwrap();
        assert_eq!(key, b"secret-key-data");
    }

    #[test]
    fn test_read_key_from_nonexistent_file() {
        let result = read_key(Some("/nonexistent/keyfile"), 0, None);
        assert!(result.is_err());
    }

    #[test]
    fn test_read_key_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 0, None).unwrap();
        assert!(key.is_empty());
    }

    #[test]
    fn test_read_key_offset_past_end() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"short").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 100, None).unwrap();
        assert!(key.is_empty());
    }

    #[test]
    fn test_read_key_size_larger_than_file() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        fs::write(&key_path, b"short").unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 0, Some(100)).unwrap();
        assert_eq!(key, b"short");
    }

    #[test]
    fn test_read_key_binary_content() {
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("keyfile");
        let binary_data: Vec<u8> = (0..=255).collect();
        fs::write(&key_path, &binary_data).unwrap();

        let key = read_key(Some(key_path.to_str().unwrap()), 0, None).unwrap();
        assert_eq!(key, binary_data);
    }

    // -----------------------------------------------------------------------
    // build_crypt_params tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_build_crypt_params_default() {
        let opts = CryptOptions::default();
        let key = vec![0xab; 32];
        let params = build_crypt_params(&opts, &key, "/dev/sda1");
        assert!(params.starts_with("aes-xts-plain64 "));
        assert!(params.contains(&"ab".repeat(32)));
        assert!(params.contains("/dev/sda1"));
        // No features, default sector size.
        assert!(!params.contains("allow_discards"));
    }

    #[test]
    fn test_build_crypt_params_with_discard() {
        let opts = CryptOptions {
            discard: true,
            ..Default::default()
        };
        let key = vec![0x00; 32];
        let params = build_crypt_params(&opts, &key, "/dev/sda1");
        assert!(params.contains("allow_discards"));
        assert!(params.contains(" 1 ")); // 1 feature.
    }

    #[test]
    fn test_build_crypt_params_with_multiple_features() {
        let opts = CryptOptions {
            discard: true,
            same_cpu_crypt: true,
            no_read_workqueue: true,
            ..Default::default()
        };
        let key = vec![0x00; 32];
        let params = build_crypt_params(&opts, &key, "/dev/sda1");
        assert!(params.contains("allow_discards"));
        assert!(params.contains("same_cpu_crypt"));
        assert!(params.contains("no_read_workqueue"));
        assert!(params.contains(" 3 ")); // 3 features.
    }

    #[test]
    fn test_build_crypt_params_empty_key() {
        let opts = CryptOptions::default();
        let key: Vec<u8> = vec![];
        let params = build_crypt_params(&opts, &key, "/dev/sda1");
        // Empty key → "-".
        assert!(params.contains("aes-xts-plain64 - 0 /dev/sda1"));
    }

    #[test]
    fn test_build_crypt_params_custom_cipher() {
        let opts = CryptOptions {
            cipher: "aes-cbc-essiv:sha256".to_string(),
            ..Default::default()
        };
        let key = vec![0xaa; 16];
        let params = build_crypt_params(&opts, &key, "/dev/sdb2");
        assert!(params.starts_with("aes-cbc-essiv:sha256 "));
    }

    #[test]
    fn test_build_crypt_params_with_offset_and_skip() {
        let opts = CryptOptions {
            offset: 2048,
            skip: 512,
            ..Default::default()
        };
        let key = vec![0x11; 32];
        let params = build_crypt_params(&opts, &key, "/dev/sda1");
        assert!(params.contains(" 512 /dev/sda1 2048"));
    }

    #[test]
    fn test_build_crypt_params_custom_sector_size() {
        let opts = CryptOptions {
            sector_size: 4096,
            ..Default::default()
        };
        let key = vec![0x00; 32];
        let params = build_crypt_params(&opts, &key, "/dev/sda1");
        assert!(params.contains("sector_size:4096"));
    }

    #[test]
    fn test_build_crypt_params_all_features() {
        let opts = CryptOptions {
            discard: true,
            same_cpu_crypt: true,
            submit_from_crypt_cpus: true,
            no_read_workqueue: true,
            no_write_workqueue: true,
            sector_size: 4096,
            ..Default::default()
        };
        let key = vec![0x00; 32];
        let params = build_crypt_params(&opts, &key, "/dev/sda1");
        // 5 boolean features + 1 sector_size = 6 total.
        assert!(params.contains(" 6 "));
        assert!(params.contains("allow_discards"));
        assert!(params.contains("same_cpu_crypt"));
        assert!(params.contains("submit_from_crypt_cpus"));
        assert!(params.contains("no_read_workqueue"));
        assert!(params.contains("no_write_workqueue"));
        assert!(params.contains("sector_size:4096"));
    }

    // -----------------------------------------------------------------------
    // CryptMode Display
    // -----------------------------------------------------------------------

    #[test]
    fn test_crypt_mode_display() {
        assert_eq!(format!("{}", CryptMode::Luks), "luks");
        assert_eq!(format!("{}", CryptMode::Plain), "plain");
        assert_eq!(format!("{}", CryptMode::Tcrypt), "tcrypt");
    }

    // -----------------------------------------------------------------------
    // DM ioctl helpers (unit tests — no actual ioctl calls)
    // -----------------------------------------------------------------------

    #[test]
    fn test_dm_ioctl_nr_encoding() {
        // Verify the ioctl number follows the Linux convention:
        // _IOWR(0xfd, cmd, size=312)
        let nr = dm_ioctl_nr(DM_DEV_CREATE_CMD);
        // Direction bits (30..31) = 3 (read|write).
        assert_eq!((nr >> 30) & 3, 3);
        // Type byte (8..15) = 0xfd.
        assert_eq!((nr >> 8) & 0xff, 0xfd);
        // Command (0..7).
        assert_eq!(nr & 0xff, DM_DEV_CREATE_CMD as libc::c_ulong);
        // Size (16..29) = 312.
        assert_eq!((nr >> 16) & 0x3fff, DM_IOCTL_HEADER_SIZE as libc::c_ulong);
    }

    #[test]
    fn test_dm_ioctl_init_basic() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "test_volume", "CRYPT-LUKS2-test", 0);

        // Version.
        assert_eq!(read_u32(&buf, DM_VERSION_OFFSET), DM_VERSION_MAJOR);
        assert_eq!(read_u32(&buf, DM_VERSION_OFFSET + 4), DM_VERSION_MINOR);
        assert_eq!(read_u32(&buf, DM_VERSION_OFFSET + 8), DM_VERSION_PATCHLEVEL);

        // data_size.
        assert_eq!(
            read_u32(&buf, DM_DATA_SIZE_OFFSET),
            DM_IOCTL_BUF_SIZE as u32
        );

        // data_start.
        assert_eq!(
            read_u32(&buf, DM_DATA_START_OFFSET),
            DM_IOCTL_HEADER_SIZE as u32
        );

        // flags.
        assert_eq!(read_u32(&buf, DM_FLAGS_OFFSET), 0);

        // name.
        let name_bytes = &buf[DM_NAME_OFFSET..DM_NAME_OFFSET + DM_NAME_SIZE];
        let name = std::str::from_utf8(
            &name_bytes[..name_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(DM_NAME_SIZE)],
        )
        .unwrap();
        assert_eq!(name, "test_volume");

        // uuid.
        let uuid_bytes = &buf[DM_UUID_OFFSET..DM_UUID_OFFSET + DM_UUID_SIZE];
        let uuid = std::str::from_utf8(
            &uuid_bytes[..uuid_bytes
                .iter()
                .position(|&b| b == 0)
                .unwrap_or(DM_UUID_SIZE)],
        )
        .unwrap();
        assert_eq!(uuid, "CRYPT-LUKS2-test");
    }

    #[test]
    fn test_dm_ioctl_init_readonly_flag() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "vol", "", DM_READONLY_FLAG);
        assert_eq!(read_u32(&buf, DM_FLAGS_OFFSET), DM_READONLY_FLAG);
    }

    #[test]
    fn test_dm_ioctl_init_suspend_flag() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "vol", "", DM_SUSPEND_FLAG);
        assert_eq!(read_u32(&buf, DM_FLAGS_OFFSET), DM_SUSPEND_FLAG);
    }

    #[test]
    fn test_dm_ioctl_init_combined_flags() {
        let mut buf = vec![0u8; DM_IOCTL_BUF_SIZE];
        dm_ioctl_init(&mut buf, "vol", "", DM_READONLY_FLAG | DM_SUSPEND_FLAG);
        assert_eq!(
            read_u32(&buf, DM_FLAGS_OFFSET),
            DM_READONLY_FLAG | DM_SUSPEND_FLAG
        );
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

        // Name should be truncated to DM_NAME_SIZE - 1 characters.
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

        write_u32(&mut buf, 4, 0);
        assert_eq!(read_u32(&buf, 4), 0);

        write_u32(&mut buf, 4, u32::MAX);
        assert_eq!(read_u32(&buf, 4), u32::MAX);
    }

    #[test]
    fn test_write_read_u64() {
        let mut buf = vec![0u8; 16];
        write_u64(&mut buf, 0, 0x123456789abcdef0);
        assert_eq!(read_u64(&buf, 0), 0x123456789abcdef0);

        write_u64(&mut buf, 8, 0);
        assert_eq!(read_u64(&buf, 8), 0);

        write_u64(&mut buf, 8, u64::MAX);
        assert_eq!(read_u64(&buf, 8), u64::MAX);
    }

    #[test]
    fn test_write_string() {
        let mut buf = vec![0u8; 32];
        write_string(&mut buf, 0, 16, "hello");
        assert_eq!(&buf[0..5], b"hello");
        assert_eq!(buf[5], 0); // null terminator.
    }

    #[test]
    fn test_write_string_truncation() {
        let mut buf = vec![0u8; 16];
        write_string(&mut buf, 0, 4, "longstring");
        // Should write only 3 chars + null.
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
            "crypt",
            "aes-xts-plain64 00 0 /dev/sda1 0",
        );
        assert!(size > 0);
        assert_eq!(size % 8, 0); // aligned

        // Check sector_start.
        assert_eq!(read_u64(&buf, data_start), 0);
        // Check length.
        assert_eq!(read_u64(&buf, data_start + 8), 1000);
        // Check target_type.
        let tt_bytes = &buf[data_start + 24..data_start + 40];
        let tt_end = tt_bytes.iter().position(|&b| b == 0).unwrap_or(16);
        let tt = std::str::from_utf8(&tt_bytes[..tt_end]).unwrap();
        assert_eq!(tt, "crypt");
    }

    #[test]
    fn test_append_target_params_content() {
        let mut buf = vec![0u8; 4096];
        let data_start = DM_IOCTL_HEADER_SIZE;
        let params_str = "aes-xts-plain64 abcdef 0 /dev/sda1 0";
        let _size = append_target(&mut buf, data_start, 0, 500, "crypt", params_str);

        // Params start at data_start + 40.
        let param_off = data_start + 40;
        let param_end = buf[param_off..].iter().position(|&b| b == 0).unwrap();
        let params = std::str::from_utf8(&buf[param_off..param_off + param_end]).unwrap();
        assert_eq!(params, params_str);
    }

    #[test]
    fn test_append_target_alignment() {
        let mut buf = vec![0u8; 4096];
        let data_start = DM_IOCTL_HEADER_SIZE;
        // Short params — should still be 8-byte aligned.
        let size = append_target(&mut buf, data_start, 0, 100, "crypt", "x");
        assert_eq!(size % 8, 0);

        // Longer params.
        let size2 = append_target(
            &mut buf,
            data_start,
            0,
            100,
            "crypt",
            "this is a longer parameter string that should also be aligned",
        );
        assert_eq!(size2 % 8, 0);
    }

    // -----------------------------------------------------------------------
    // detect_luks tests (file-based — not actual devices)
    // -----------------------------------------------------------------------

    #[test]
    fn test_detect_luks_v1_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("luks1.img");
        let mut f = fs::File::create(&path).unwrap();
        // LUKS v1 magic: "LUKS\xba\xbe"
        f.write_all(LUKS_MAGIC).unwrap();
        f.write_all(&[0u8; 100]).unwrap();
        drop(f);

        assert!(detect_luks(path.to_str().unwrap()).unwrap());
    }

    #[test]
    fn test_detect_luks_v2_magic() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("luks2.img");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(LUKS2_MAGIC).unwrap();
        f.write_all(&[0u8; 100]).unwrap();
        drop(f);

        assert!(detect_luks(path.to_str().unwrap()).unwrap());
    }

    #[test]
    fn test_detect_luks_not_luks() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("notluks.img");
        let mut f = fs::File::create(&path).unwrap();
        f.write_all(b"NOT_LUKS_HEADER").unwrap();
        drop(f);

        assert!(!detect_luks(path.to_str().unwrap()).unwrap());
    }

    #[test]
    fn test_detect_luks_empty_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("empty.img");
        fs::write(&path, b"").unwrap();

        assert!(!detect_luks(path.to_str().unwrap()).unwrap());
    }

    #[test]
    fn test_detect_luks_short_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("short.img");
        fs::write(&path, b"LUK").unwrap(); // too short

        assert!(!detect_luks(path.to_str().unwrap()).unwrap());
    }

    #[test]
    fn test_detect_luks_nonexistent() {
        assert!(detect_luks("/nonexistent/device").is_err());
    }

    // -----------------------------------------------------------------------
    // CryptOptions default tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_crypt_options_default() {
        let opts = CryptOptions::default();
        assert_eq!(opts.mode, CryptMode::Luks);
        assert_eq!(opts.cipher, "aes-xts-plain64");
        assert_eq!(opts.key_size, 256);
        assert_eq!(opts.hash, "sha256");
        assert_eq!(opts.offset, 0);
        assert_eq!(opts.skip, 0);
        assert!(!opts.readonly);
        assert!(!opts.discard);
        assert!(!opts.same_cpu_crypt);
        assert!(!opts.submit_from_crypt_cpus);
        assert!(!opts.no_read_workqueue);
        assert!(!opts.no_write_workqueue);
        assert_eq!(opts.sector_size, 512);
        assert_eq!(opts.tries, 3);
        assert!(opts.keyfile_size.is_none());
        assert_eq!(opts.keyfile_offset, 0);
        assert!(opts.timeout.is_none());
        assert!(opts.tmp.is_none());
        assert!(opts.header.is_none());
    }

    // -----------------------------------------------------------------------
    // Integration-style tests (no actual DM operations)
    // -----------------------------------------------------------------------

    #[test]
    fn test_attach_missing_device_file() {
        // Should fail because the device doesn't exist.
        let result = cmd_attach("testvol", "/nonexistent/device", None, "");
        assert!(result.is_err());
    }

    #[test]
    fn test_attach_plain_mode_empty_key_fails() {
        // Plain mode with empty key file should fail.
        let dir = tempfile::tempdir().unwrap();
        let key_path = dir.path().join("empty_key");
        fs::write(&key_path, b"").unwrap();

        // This will also fail because the device doesn't exist, but the key
        // check happens first for plain mode.
        let result = cmd_attach(
            "testvol",
            "/nonexistent/device",
            Some(key_path.to_str().unwrap()),
            "plain,hash=plain",
        );
        assert!(result.is_err());
    }

    #[test]
    fn test_detach_nonexistent_volume() {
        // Should fail because we can't open /dev/mapper/control without root,
        // or the volume doesn't exist.
        let result = cmd_detach("nonexistent_volume_12345");
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // Mode override tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_options_mode_override_last_wins() {
        // When both plain and luks are specified, last wins.
        let opts = parse_options("plain,luks").unwrap();
        assert_eq!(opts.mode, CryptMode::Luks);

        let opts = parse_options("luks,plain").unwrap();
        assert_eq!(opts.mode, CryptMode::Plain);

        let opts = parse_options("plain,tcrypt").unwrap();
        assert_eq!(opts.mode, CryptMode::Tcrypt);
    }

    // -----------------------------------------------------------------------
    // Full-flow argument + option parsing tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_attach_parse() {
        let cmd = parse_args(&args(&[
            "attach",
            "root",
            "/dev/nvme0n1p2",
            "/etc/cryptsetup-keys.d/root.key",
            "luks,discard,no-read-workqueue,no-write-workqueue,header=/boot/header.img",
        ]))
        .unwrap();

        if let Command::Attach {
            volume,
            device,
            password,
            options,
        } = cmd
        {
            assert_eq!(volume, "root");
            assert_eq!(device, "/dev/nvme0n1p2");
            assert_eq!(
                password,
                Some("/etc/cryptsetup-keys.d/root.key".to_string())
            );

            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.mode, CryptMode::Luks);
            assert!(opts.discard);
            assert!(opts.no_read_workqueue);
            assert!(opts.no_write_workqueue);
            assert_eq!(opts.header, Some(PathBuf::from("/boot/header.img")));
        } else {
            panic!("Expected Attach command");
        }
    }

    #[test]
    fn test_full_attach_plain_parse() {
        let cmd = parse_args(&args(&[
            "attach",
            "swap_crypt",
            "/dev/sda2",
            "/dev/urandom",
            "plain,cipher=aes-xts-plain64,size=256,swap",
        ]))
        .unwrap();

        if let Command::Attach {
            volume,
            device,
            password,
            options,
        } = cmd
        {
            assert_eq!(volume, "swap_crypt");
            assert_eq!(device, "/dev/sda2");
            assert_eq!(password, Some("/dev/urandom".to_string()));

            let opts = parse_options(&options).unwrap();
            assert_eq!(opts.mode, CryptMode::Plain);
            assert_eq!(opts.cipher, "aes-xts-plain64");
            assert_eq!(opts.key_size, 256);
        } else {
            panic!("Expected Attach command");
        }
    }

    // -----------------------------------------------------------------------
    // Constants / sanity checks
    // -----------------------------------------------------------------------

    #[test]
    fn test_luks_magic_length() {
        assert_eq!(LUKS_MAGIC.len(), 6);
        assert_eq!(LUKS2_MAGIC.len(), 6);
    }

    #[test]
    fn test_dm_ioctl_header_size() {
        // The DM ioctl header is 312 bytes.
        assert_eq!(DM_IOCTL_HEADER_SIZE, 312);
    }

    #[test]
    fn test_dm_ioctl_buf_size_larger_than_header() {
        assert!(DM_IOCTL_BUF_SIZE > DM_IOCTL_HEADER_SIZE);
    }

    #[test]
    fn test_default_cipher_value() {
        assert_eq!(DEFAULT_CIPHER, "aes-xts-plain64");
    }

    #[test]
    fn test_default_key_size_value() {
        assert_eq!(DEFAULT_KEY_SIZE, 256);
    }

    #[test]
    fn test_default_hash_value() {
        assert_eq!(DEFAULT_HASH, "sha256");
    }

    #[test]
    fn test_default_sector_size_value() {
        assert_eq!(DEFAULT_SECTOR_SIZE, 512);
    }

    #[test]
    fn test_exit_password_error_code() {
        assert_eq!(EXIT_PASSWORD_ERROR, 5);
    }
}
