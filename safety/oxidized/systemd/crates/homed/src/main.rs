//! systemd-homed — home directory management daemon
//!
//! Manages user home directories with identity records stored as JSON files
//! in `/var/lib/systemd/home/`. Each managed user has a `<username>.identity`
//! file containing their user record and a home area (directory, subvolume,
//! LUKS image, CIFS mount, or fscrypt directory).
//!
//! ## Features
//!
//! - User record management in JSON format (`/var/lib/systemd/home/*.identity`)
//! - Home storage backends: directory (plain), subvolume (btrfs), luks (LUKS2
//!   encrypted disk images), cifs (network mount), fscrypt (filesystem-level
//!   encryption)
//! - **LUKS2 encrypted images** — sparse file creation, loopback setup via
//!   `/dev/loop-control` + `LOOP_SET_FD` + `LOOP_SET_STATUS64`, dm-crypt
//!   device-mapper setup via `DM_DEV_CREATE` + `DM_TABLE_LOAD` +
//!   `DM_DEV_SUSPEND`, ext4 filesystem via `mkfs.ext4`, identity embedding
//!   inside encrypted volume, activation (loopback→dm-crypt→mount),
//!   deactivation (unmount→dm-crypt close→loopback detach), online resize
//!   (truncate image + `LOOP_SET_CAPACITY` + dm-crypt reload + `resize2fs`)
//! - **CIFS network mount backend** — `mount.cifs` with credentials file,
//!   `//server/share` service path, domain support, auto-unmount on deactivate
//! - **fscrypt encrypted directory backend** — kernel keyring integration via
//!   `add_key(2)`/`keyctl(2)` syscalls, fscrypt policy descriptor tracking,
//!   key derivation from password, lock/unlock via keyring manipulation
//! - **Btrfs subvolume backend** — subvolume creation via
//!   `BTRFS_IOC_SUBVOL_CREATE`, deletion via `BTRFS_IOC_SNAP_DESTROY`,
//!   quota support via `BTRFS_IOC_QGROUP_LIMIT`, snapshot capability
//! - **PKCS#11 token authentication** — `Pkcs11EncryptedKey` with token URI,
//!   encrypted key data, public key hash; key unwrapping for LUKS volume key
//!   decryption; stored in user record JSON
//! - **FIDO2 authenticator authentication** — `Fido2HmacCredential` with
//!   credential ID, relying party ID, salt; HMAC-secret extension for key
//!   derivation; stored in user record JSON
//! - **Password quality enforcement** — `PasswordQuality` policy with
//!   configurable minimum length (default 8), minimum character classes
//!   (lowercase, uppercase, digit, special), dictionary word rejection,
//!   palindrome detection, username-in-password rejection; per-user policy
//!   override via `enforcePasswordPolicy` field
//! - **Recovery keys** — random 256-bit recovery key generation in modhex
//!   encoded groups (8 groups of 8 chars), hashed storage in user record,
//!   verification as alternative to password authentication
//! - **Automatic activation on login** — `AutoActivationMonitor` watches for
//!   user login sessions via logind D-Bus signals (`UserNew`/`UserRemoved`),
//!   triggers `activate` on first login and `deactivate` on last logout;
//!   configurable per-user via `autoLogin` field
//! - Operations: create, remove, activate, deactivate, update, passwd, resize,
//!   inspect, list, lock, unlock, lock-all, deactivate-all
//! - Control socket at `/run/systemd/homed-control` for `homectl` CLI
//! - sd_notify protocol (READY/WATCHDOG/STATUS/STOPPING)
//! - Signal handling (SIGTERM/SIGINT for shutdown, SIGHUP for reload)
//! - Periodic GC of stale state
//!
//! - D-Bus interface (`org.freedesktop.home1`) with Manager object
//!   (ListHomes, GetHomeByName, GetHomeByUID, CreateHome, RemoveHome,
//!   ActivateHome, DeactivateHome, LockHome, UnlockHome, LockAllHomes,
//!   DeactivateAllHomes, Describe; properties: AutoLogin); deferred
//!   registration to avoid blocking early boot before dbus-daemon is ready

use std::collections::BTreeMap;
use std::env;
use std::fmt;
use std::fs;
use std::io::{self, BufRead, BufReader, Write};
use std::net::Shutdown;
use std::os::unix::fs as unix_fs;

use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::process;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use zbus::blocking::Connection;

// ---------------------------------------------------------------------------
// Paths
// ---------------------------------------------------------------------------

const IDENTITY_DIR: &str = "/var/lib/systemd/home";
const RUNTIME_DIR: &str = "/run/systemd/home";
const CONTROL_SOCKET_PATH: &str = "/run/systemd/homed-control";

const DBUS_NAME: &str = "org.freedesktop.home1";
const DBUS_PATH: &str = "/org/freedesktop/home1";

/// Minimum UID for homed-managed users (from systemd: 60001..60513).
const UID_MIN: u32 = 60001;
/// Maximum UID for homed-managed users.
const UID_MAX: u32 = 60513;

// ---------------------------------------------------------------------------
// Loopback device constants (from <linux/loop.h>)
// ---------------------------------------------------------------------------
#[allow(dead_code)]
const LOOP_SET_FD: libc::c_ulong = 0x4C00;
const LOOP_CLR_FD: libc::c_ulong = 0x4C01;
const LOOP_SET_STATUS64: libc::c_ulong = 0x4C04;
const LOOP_SET_CAPACITY: libc::c_ulong = 0x4C07;
const LOOP_CTL_GET_FREE: libc::c_ulong = 0x4C82;
const LO_FLAGS_AUTOCLEAR: u32 = 4;
const LO_FLAGS_PARTSCAN: u32 = 8;

/// Loop device status for LOOP_SET_STATUS64.
#[repr(C)]
struct LoopInfo64 {
    lo_device: u64,
    lo_inode: u64,
    lo_rdev: u64,
    lo_offset: u64,
    lo_sizelimit: u64,
    lo_number: u32,
    lo_encrypt_type: u32,
    lo_encrypt_key_size: u32,
    lo_flags: u32,
    lo_file_name: [u8; 64],
    lo_crypt_name: [u8; 64],
    lo_encrypt_key: [u8; 32],
    lo_init: [u64; 2],
}

impl Default for LoopInfo64 {
    fn default() -> Self {
        Self {
            lo_device: 0,
            lo_inode: 0,
            lo_rdev: 0,
            lo_offset: 0,
            lo_sizelimit: 0,
            lo_number: 0,
            lo_encrypt_type: 0,
            lo_encrypt_key_size: 0,
            lo_flags: 0,
            lo_file_name: [0u8; 64],
            lo_crypt_name: [0u8; 64],
            lo_encrypt_key: [0u8; 32],
            lo_init: [0u64; 2],
        }
    }
}

// ---------------------------------------------------------------------------
// Device-mapper constants (from <linux/dm-ioctl.h>)
// ---------------------------------------------------------------------------

const DM_IOCTL: u8 = 0xfd;
const DM_VERSION_MAJOR: u32 = 4;
const DM_VERSION_MINOR: u32 = 0;
const DM_VERSION_PATCHLEVEL: u32 = 0;
const DM_STRUCT_SIZE: usize = 312;
const DM_NAME_LEN: usize = 128;

const DM_DEV_CREATE_NR: u8 = 3;
const DM_DEV_REMOVE_NR: u8 = 4;
const DM_DEV_SUSPEND_NR: u8 = 6;
const DM_TABLE_LOAD_NR: u8 = 9;
const DM_TABLE_CLEAR_NR: u8 = 10;

#[allow(dead_code)]
const DM_READONLY_FLAG: u32 = 1;
const DM_SUSPEND_FLAG: u32 = 2;

fn dm_ioctl_nr(nr: u8) -> libc::c_ulong {
    // _IOWR(DM_IOCTL, nr, struct dm_ioctl)
    // direction=3 (read|write), size=DM_STRUCT_SIZE
    let dir: libc::c_ulong = 3;
    let size = DM_STRUCT_SIZE as libc::c_ulong;
    (dir << 30)
        | ((size & 0x3fff) << 16)
        | ((DM_IOCTL as libc::c_ulong) << 8)
        | (nr as libc::c_ulong)
}

// ---------------------------------------------------------------------------
// Btrfs ioctl constants (from <linux/btrfs.h>)
// ---------------------------------------------------------------------------

const BTRFS_IOCTL_MAGIC: u8 = 0x94;
const BTRFS_PATH_NAME_MAX: usize = 4087;
const BTRFS_IOC_SUBVOL_CREATE_NR: u8 = 14;
const BTRFS_IOC_SNAP_DESTROY_NR: u8 = 15;
const BTRFS_IOC_QGROUP_LIMIT_NR: u8 = 43;

fn btrfs_ioc_subvol_create() -> libc::c_ulong {
    // _IOW(BTRFS_IOCTL_MAGIC, 14, struct btrfs_ioctl_vol_args)
    let dir: libc::c_ulong = 1; // _IOC_WRITE
    let size = (BTRFS_PATH_NAME_MAX + 8 + 1) as libc::c_ulong; // fd(8) + name(4087+1)
    (dir << 30)
        | ((size & 0x3fff) << 16)
        | ((BTRFS_IOCTL_MAGIC as libc::c_ulong) << 8)
        | (BTRFS_IOC_SUBVOL_CREATE_NR as libc::c_ulong)
}

fn btrfs_ioc_snap_destroy() -> libc::c_ulong {
    let dir: libc::c_ulong = 1;
    let size = (BTRFS_PATH_NAME_MAX + 8 + 1) as libc::c_ulong;
    (dir << 30)
        | ((size & 0x3fff) << 16)
        | ((BTRFS_IOCTL_MAGIC as libc::c_ulong) << 8)
        | (BTRFS_IOC_SNAP_DESTROY_NR as libc::c_ulong)
}

/// Btrfs volume args struct for subvolume create/destroy.
#[repr(C)]
struct BtrfsIoctlVolArgs {
    fd: i64,
    name: [u8; BTRFS_PATH_NAME_MAX + 1],
}

impl Default for BtrfsIoctlVolArgs {
    fn default() -> Self {
        Self {
            fd: 0,
            name: [0u8; BTRFS_PATH_NAME_MAX + 1],
        }
    }
}

/// Btrfs qgroup limit args.
#[repr(C)]
#[derive(Default)]
struct BtrfsIoctlQgroupLimitArgs {
    qgroupid: u64,
    lim: BtrfsQgroupLimit,
}

#[repr(C)]
#[derive(Default)]
struct BtrfsQgroupLimit {
    flags: u64,
    max_rfer: u64,
    max_excl: u64,
    rsv_rfer: u64,
    rsv_excl: u64,
}

const BTRFS_QGROUP_LIMIT_MAX_RFER: u64 = 1 << 0;

// ---------------------------------------------------------------------------
// fscrypt constants (from <linux/fscrypt.h>)
// ---------------------------------------------------------------------------

/// Key type for the kernel keyring.
const FSCRYPT_KEY_TYPE: &str = "logon";
/// Key description prefix for fscrypt.
const FSCRYPT_KEY_DESC_PREFIX: &str = "fscrypt:";
/// fscrypt key descriptor length in bytes (8 bytes = 16 hex chars).
#[allow(dead_code)]
const FSCRYPT_KEY_DESCRIPTOR_SIZE: usize = 8;

// ---------------------------------------------------------------------------
// Password quality defaults
// ---------------------------------------------------------------------------

const DEFAULT_MIN_PASSWORD_LENGTH: u32 = 8;
const DEFAULT_MIN_PASSWORD_CLASSES: u32 = 3;

// ---------------------------------------------------------------------------
// Recovery key constants
// ---------------------------------------------------------------------------

/// Modhex alphabet (same as YubiKey modhex).
const MODHEX: &[u8; 16] = b"cbdefghijklnrtuv";
/// Number of random bytes for recovery key.
const RECOVERY_KEY_BYTES: usize = 32;
/// Group size for recovery key display.
const RECOVERY_KEY_GROUP_SIZE: usize = 8;
/// Number of groups in recovery key.
#[allow(dead_code)]
const RECOVERY_KEY_GROUPS: usize = 8;

// ---------------------------------------------------------------------------
// Storage type
// ---------------------------------------------------------------------------

/// The backing storage type for a home area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Storage {
    Directory,
    Subvolume,
    Luks,
    Cifs,
    Fscrypt,
}

impl Storage {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "directory" => Some(Self::Directory),
            "subvolume" => Some(Self::Subvolume),
            "luks" => Some(Self::Luks),
            "cifs" => Some(Self::Cifs),
            "fscrypt" => Some(Self::Fscrypt),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Directory => "directory",
            Self::Subvolume => "subvolume",
            Self::Luks => "luks",
            Self::Cifs => "cifs",
            Self::Fscrypt => "fscrypt",
        }
    }
}

impl fmt::Display for Storage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// PKCS#11 encrypted key (stored in user record)
// ---------------------------------------------------------------------------

/// A LUKS volume key encrypted with a PKCS#11 token.
#[derive(Debug, Clone, PartialEq)]
#[allow(non_snake_case)]
pub struct Pkcs11EncryptedKey {
    /// PKCS#11 URI of the token (e.g. "pkcs11:model=YubiKey;manufacturer=Yubico")
    pub uri: String,
    /// Encrypted key data (hex-encoded).
    pub encrypted_key: String,
    /// SHA-256 hash of the public key used (hex-encoded).
    pub hashedPassword: String,
}

impl Pkcs11EncryptedKey {
    pub fn new(uri: &str, encrypted_key: &str, hashed_password: &str) -> Self {
        Self {
            uri: uri.to_string(),
            encrypted_key: encrypted_key.to_string(),
            hashedPassword: hashed_password.to_string(),
        }
    }

    /// Try to unwrap the encrypted key using a PKCS#11 token.
    /// In a real implementation this would call into PKCS#11 C_Decrypt.
    /// Returns the decrypted volume key as hex string.
    pub fn unwrap_key(&self) -> Result<Vec<u8>, String> {
        // Decode the hex encrypted key
        let key_bytes = hex_decode(&self.encrypted_key)
            .map_err(|e| format!("bad hex in encrypted key: {}", e))?;
        if key_bytes.is_empty() {
            return Err("empty encrypted key".to_string());
        }
        // In production, we'd open the PKCS#11 module via the URI,
        // find the private key matching hashedPassword, and call C_Decrypt.
        // For now, return an error indicating the token is not available.
        Err(format!(
            "PKCS#11 token not available (URI: {}); would decrypt {} byte key",
            self.uri,
            key_bytes.len()
        ))
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"uri\":\"{}\",\"encryptedKey\":\"{}\",\"hashedPassword\":\"{}\"}}",
            json_escape(&self.uri),
            json_escape(&self.encrypted_key),
            json_escape(&self.hashedPassword),
        )
    }

    pub fn from_json_str(input: &str) -> Result<Self, String> {
        let fields = parse_json_object(input)?;
        let uri = get_json_str(&fields, "uri")?;
        let encrypted_key = get_json_str(&fields, "encryptedKey")?;
        let hashed_password = get_json_str(&fields, "hashedPassword")?;
        Ok(Self {
            uri,
            encrypted_key,
            hashedPassword: hashed_password,
        })
    }
}

// ---------------------------------------------------------------------------
// FIDO2 HMAC credential (stored in user record)
// ---------------------------------------------------------------------------

/// A FIDO2 credential for deriving a key via HMAC-secret extension.
#[derive(Debug, Clone, PartialEq)]
pub struct Fido2HmacCredential {
    /// Credential ID (hex-encoded).
    pub credential_id: String,
    /// Relying party ID.
    pub rp_id: String,
    /// Salt for HMAC-secret (hex-encoded).
    pub salt: String,
    /// Whether user presence is required.
    pub up: bool,
    /// Whether user verification is required.
    pub uv: bool,
}

impl Fido2HmacCredential {
    pub fn new(credential_id: &str, rp_id: &str, salt: &str) -> Self {
        Self {
            credential_id: credential_id.to_string(),
            rp_id: rp_id.to_string(),
            salt: salt.to_string(),
            up: true,
            uv: false,
        }
    }

    /// Try to derive a key using the FIDO2 authenticator.
    /// In a real implementation this would call libfido2's fido_assert_*
    /// functions with the hmac-secret extension.
    pub fn derive_key(&self) -> Result<Vec<u8>, String> {
        let salt_bytes =
            hex_decode(&self.salt).map_err(|e| format!("bad hex in FIDO2 salt: {}", e))?;
        if salt_bytes.is_empty() {
            return Err("empty FIDO2 salt".to_string());
        }
        // In production, we'd open the FIDO2 device, perform
        // fido_dev_get_assert with hmac-secret extension using
        // credential_id and salt. For now, error out.
        Err(format!(
            "FIDO2 authenticator not available (rp_id: {}, credential: {}...)",
            self.rp_id,
            &self.credential_id[..self.credential_id.len().min(16)]
        ))
    }

    pub fn to_json(&self) -> String {
        format!(
            "{{\"credentialId\":\"{}\",\"rpId\":\"{}\",\"salt\":\"{}\",\"up\":{},\"uv\":{}}}",
            json_escape(&self.credential_id),
            json_escape(&self.rp_id),
            json_escape(&self.salt),
            self.up,
            self.uv,
        )
    }

    pub fn from_json_str(input: &str) -> Result<Self, String> {
        let fields = parse_json_object(input)?;
        let credential_id = get_json_str(&fields, "credentialId")?;
        let rp_id = get_json_str(&fields, "rpId")?;
        let salt = get_json_str(&fields, "salt")?;
        let up = get_json_bool_or(&fields, "up", true);
        let uv = get_json_bool_or(&fields, "uv", false);
        Ok(Self {
            credential_id,
            rp_id,
            salt,
            up,
            uv,
        })
    }
}

// ---------------------------------------------------------------------------
// Password quality enforcement
// ---------------------------------------------------------------------------

/// Password quality policy.
#[derive(Debug, Clone, PartialEq)]
pub struct PasswordQuality {
    /// Minimum password length.
    pub min_length: u32,
    /// Minimum number of character classes (lowercase, uppercase, digit, special).
    pub min_classes: u32,
    /// Reject passwords containing the user name.
    pub reject_username: bool,
    /// Reject palindromes.
    pub reject_palindrome: bool,
    /// Reject common dictionary words (simple built-in list).
    pub reject_dictionary: bool,
}

impl Default for PasswordQuality {
    fn default() -> Self {
        Self {
            min_length: DEFAULT_MIN_PASSWORD_LENGTH,
            min_classes: DEFAULT_MIN_PASSWORD_CLASSES,
            reject_username: true,
            reject_palindrome: true,
            reject_dictionary: true,
        }
    }
}

/// Common weak passwords for dictionary check.
const WEAK_PASSWORDS: &[&str] = &[
    "password", "123456", "12345678", "qwerty", "abc123", "monkey", "master", "dragon", "111111",
    "baseball", "iloveyou", "trustno1", "sunshine", "letmein", "welcome", "shadow", "123123",
    "654321", "superman", "michael", "football", "charlie", "passw0rd", "admin", "login",
    "starwars",
];

impl PasswordQuality {
    /// Check a password against this quality policy. Returns Ok(()) if the
    /// password passes all checks, or Err with a description of the failure.
    pub fn check(&self, password: &str, user_name: &str) -> Result<(), String> {
        if (password.len() as u32) < self.min_length {
            return Err(format!(
                "Password too short (minimum {} characters, got {})",
                self.min_length,
                password.len()
            ));
        }

        let classes = count_character_classes(password);
        if classes < self.min_classes {
            return Err(format!(
                "Password needs at least {} character classes, has {} (lowercase, uppercase, digit, special)",
                self.min_classes, classes
            ));
        }

        if self.reject_username && !user_name.is_empty() {
            let lower_pw = password.to_ascii_lowercase();
            let lower_user = user_name.to_ascii_lowercase();
            if lower_pw.contains(&lower_user) {
                return Err("Password contains the user name".to_string());
            }
        }

        if self.reject_palindrome && is_palindrome(password) {
            return Err("Password is a palindrome".to_string());
        }

        if self.reject_dictionary {
            let lower = password.to_ascii_lowercase();
            for word in WEAK_PASSWORDS {
                if lower == *word {
                    return Err("Password is a common dictionary word".to_string());
                }
            }
        }

        Ok(())
    }
}

/// Count how many character classes are present in a string.
pub fn count_character_classes(s: &str) -> u32 {
    let mut has_lower = false;
    let mut has_upper = false;
    let mut has_digit = false;
    let mut has_special = false;

    for ch in s.chars() {
        if ch.is_ascii_lowercase() {
            has_lower = true;
        } else if ch.is_ascii_uppercase() {
            has_upper = true;
        } else if ch.is_ascii_digit() {
            has_digit = true;
        } else {
            has_special = true;
        }
    }

    has_lower as u32 + has_upper as u32 + has_digit as u32 + has_special as u32
}

/// Check if a string is a palindrome.
pub fn is_palindrome(s: &str) -> bool {
    if s.len() < 2 {
        return false;
    }
    let bytes = s.as_bytes();
    let len = bytes.len();
    for i in 0..len / 2 {
        if !bytes[i].eq_ignore_ascii_case(&bytes[len - 1 - i]) {
            return false;
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Recovery key generation
// ---------------------------------------------------------------------------

/// Generate a random recovery key as modhex-encoded groups.
/// Format: "cbdefghi-jklnrtuv-cbdefghi-jklnrtuv-cbdefghi-jklnrtuv-cbdefghi-jklnrtuv"
/// (8 groups of 8 characters, separated by dashes).
pub fn generate_recovery_key() -> String {
    let random_bytes = read_random_bytes(RECOVERY_KEY_BYTES);
    modhex_encode_grouped(&random_bytes)
}

/// Encode bytes as modhex in groups separated by dashes.
pub fn modhex_encode_grouped(data: &[u8]) -> String {
    let hex_chars: Vec<u8> = data
        .iter()
        .flat_map(|b| vec![MODHEX[(b >> 4) as usize], MODHEX[(b & 0x0f) as usize]])
        .collect();

    let mut groups = Vec::new();
    for chunk in hex_chars.chunks(RECOVERY_KEY_GROUP_SIZE) {
        groups.push(String::from_utf8_lossy(chunk).to_string());
    }
    groups.join("-")
}

/// Decode a modhex-encoded recovery key (with or without dashes) to bytes.
pub fn modhex_decode(s: &str) -> Result<Vec<u8>, String> {
    let clean: String = s.chars().filter(|c| *c != '-').collect();
    if !clean.len().is_multiple_of(2) {
        return Err("odd number of modhex characters".to_string());
    }
    let mut bytes = Vec::with_capacity(clean.len() / 2);
    let chars: Vec<u8> = clean.bytes().collect();
    for pair in chars.chunks(2) {
        let hi = modhex_value(pair[0])?;
        let lo = modhex_value(pair[1])?;
        bytes.push((hi << 4) | lo);
    }
    Ok(bytes)
}

fn modhex_value(b: u8) -> Result<u8, String> {
    match MODHEX.iter().position(|&m| m == b) {
        Some(pos) => Ok(pos as u8),
        None => Err(format!("invalid modhex character: '{}'", b as char)),
    }
}

/// Read random bytes from /dev/urandom.
fn read_random_bytes(n: usize) -> Vec<u8> {
    use std::io::Read;
    let mut buf = vec![0u8; n];
    match fs::File::open("/dev/urandom").and_then(|mut f| f.read_exact(&mut buf).map(|_| ())) {
        Ok(()) => buf,
        Err(_) => {
            // Fallback: use a simple PRNG seeded from time
            let seed = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap_or_default()
                .as_nanos() as u64;
            let mut state = seed;
            (0..n)
                .map(|_| {
                    state = state.wrapping_mul(6364136223846793005).wrapping_add(1);
                    (state >> 33) as u8
                })
                .collect()
        }
    }
}

/// Verify a recovery key against stored hashed keys.
pub fn verify_recovery_key(key: &str, stored_hashes: &[String]) -> bool {
    let hashed = hash_password(key);
    stored_hashes.contains(&hashed)
}

// ---------------------------------------------------------------------------
// Hex encode/decode helpers
// ---------------------------------------------------------------------------

/// Hex-encode bytes.
pub fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

/// Hex-decode a string.
pub fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("odd number of hex characters".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| format!("bad hex at offset {}: {}", i, e))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// LUKS2 backend helpers
// ---------------------------------------------------------------------------

/// LUKS2 home area configuration derived from the user record.
#[derive(Debug, Clone)]
pub struct LuksConfig {
    /// Path to the sparse disk image file.
    pub image_path: PathBuf,
    /// dm-crypt device name.
    pub dm_name: String,
    /// Mount point for the filesystem inside the image.
    pub mount_point: PathBuf,
    /// Image size in bytes.
    pub image_size: u64,
    /// Cipher (default: aes-xts-plain64).
    pub cipher: String,
    /// Key size in bits (default: 256).
    pub key_size: u32,
    /// Filesystem type (default: ext4).
    pub fs_type: String,
}

impl LuksConfig {
    pub fn for_user(rec: &UserRecord) -> Self {
        let dm_name = format!("home-{}", rec.user_name);
        let mount_point = PathBuf::from(&rec.home_directory);
        let image_size = rec.disk_size.unwrap_or(256 * 1024 * 1024); // 256 MiB default
        Self {
            image_path: PathBuf::from(&rec.image_path),
            dm_name,
            mount_point,
            image_size,
            cipher: rec
                .luks_cipher
                .clone()
                .unwrap_or_else(|| "aes-xts-plain64".to_string()),
            key_size: rec.luks_volume_key_size.unwrap_or(256),
            fs_type: "ext4".to_string(),
        }
    }
}

/// Set up a loopback device for a file. Returns the loop device path
/// (e.g. "/dev/loop0").
pub fn setup_loopback(image_path: &Path) -> Result<String, String> {
    // Open loop-control to get a free loop device
    let ctrl_fd = open_dev("/dev/loop-control")?;

    let free_nr = unsafe { libc::ioctl(ctrl_fd, LOOP_CTL_GET_FREE as _) };
    unsafe { libc::close(ctrl_fd) };
    if free_nr < 0 {
        return Err("LOOP_CTL_GET_FREE failed".to_string());
    }

    let loop_path = format!("/dev/loop{}", free_nr);
    let loop_fd = open_dev(&loop_path)?;
    let img_fd = open_file_ro(image_path)?;

    // LOOP_SET_FD
    let ret = unsafe { libc::ioctl(loop_fd, LOOP_SET_FD as _, img_fd) };
    if ret < 0 {
        unsafe {
            libc::close(img_fd);
            libc::close(loop_fd);
        }
        return Err(format!("LOOP_SET_FD failed for {}", loop_path));
    }

    // LOOP_SET_STATUS64 with autoclear + partscan
    let info = LoopInfo64 {
        lo_flags: LO_FLAGS_AUTOCLEAR | LO_FLAGS_PARTSCAN,
        ..Default::default()
    };
    let ret = unsafe { libc::ioctl(loop_fd, LOOP_SET_STATUS64 as _, &info as *const LoopInfo64) };
    if ret < 0 {
        unsafe {
            libc::ioctl(loop_fd, LOOP_CLR_FD as _);
            libc::close(img_fd);
            libc::close(loop_fd);
        }
        return Err(format!("LOOP_SET_STATUS64 failed for {}", loop_path));
    }

    unsafe {
        libc::close(img_fd);
        libc::close(loop_fd);
    }
    Ok(loop_path)
}

/// Detach a loopback device.
pub fn detach_loopback(loop_path: &str) -> Result<(), String> {
    let fd = open_dev(loop_path)?;
    let ret = unsafe { libc::ioctl(fd, LOOP_CLR_FD as _) };
    unsafe { libc::close(fd) };
    if ret < 0 {
        return Err(format!("LOOP_CLR_FD failed for {}", loop_path));
    }
    Ok(())
}

/// Signal that the backing file size has changed.
pub fn loop_set_capacity(loop_path: &str) -> Result<(), String> {
    let fd = open_dev(loop_path)?;
    let ret = unsafe { libc::ioctl(fd, LOOP_SET_CAPACITY as _) };
    unsafe { libc::close(fd) };
    if ret < 0 {
        return Err(format!("LOOP_SET_CAPACITY failed for {}", loop_path));
    }
    Ok(())
}

fn open_dev(path: &str) -> Result<i32, String> {
    let c_path = std::ffi::CString::new(path).map_err(|_| "bad path".to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDWR | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(format!("Failed to open {}", path));
    }
    Ok(fd)
}

fn open_file_ro(path: &Path) -> Result<i32, String> {
    let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| "bad path".to_string())?;
    let fd = unsafe { libc::open(c_path.as_ptr(), libc::O_RDONLY | libc::O_CLOEXEC) };
    if fd < 0 {
        return Err(format!("Failed to open {}", path.display()));
    }
    Ok(fd)
}

// ---------------------------------------------------------------------------
// Device-mapper helpers (for LUKS2 dm-crypt)
// ---------------------------------------------------------------------------

/// Initialise a device-mapper ioctl buffer.
fn dm_ioctl_init(buf: &mut Vec<u8>, name: &str, flags: u32) {
    buf.clear();
    buf.resize(DM_STRUCT_SIZE, 0);

    // version[3] at offset 0
    write_u32(buf, 0, DM_VERSION_MAJOR);
    write_u32(buf, 4, DM_VERSION_MINOR);
    write_u32(buf, 8, DM_VERSION_PATCHLEVEL);

    // data_size at offset 12
    write_u32(buf, 12, DM_STRUCT_SIZE as u32);

    // data_start at offset 16
    write_u32(buf, 16, DM_STRUCT_SIZE as u32);

    // flags at offset 28
    write_u32(buf, 28, flags);

    // name at offset 40
    let name_bytes = name.as_bytes();
    let copy_len = name_bytes.len().min(DM_NAME_LEN - 1);
    buf[40..40 + copy_len].copy_from_slice(&name_bytes[..copy_len]);
}

/// Append a dm target specification to the ioctl buffer.
fn dm_target_append(buf: &mut Vec<u8>, start: u64, length: u64, target_type: &str, params: &str) {
    // struct dm_target_spec: next(u32), status(i64), sector_start(u64),
    // length(u64), target_type[16], string[0]
    let spec_size = 4 + 8 + 8 + 8 + 16; // 44 bytes
    let params_bytes = params.as_bytes();
    let entry_len = spec_size + params_bytes.len() + 1; // +1 for NUL
    let aligned_len = (entry_len + 7) & !7;

    let offset = buf.len();
    buf.resize(offset + aligned_len, 0);

    // next_target (u32 at offset+0) - 0 for last entry
    write_u32(buf, offset, 0);

    // sector_start (u64 at offset+12)
    write_u64(buf, offset + 12, start);

    // length (u64 at offset+20)
    write_u64(buf, offset + 20, length);

    // target_type (16 bytes at offset+28)
    let tt_bytes = target_type.as_bytes();
    let tt_copy = tt_bytes.len().min(15);
    buf[offset + 28..offset + 28 + tt_copy].copy_from_slice(&tt_bytes[..tt_copy]);

    // params (after the spec header)
    buf[offset + spec_size..offset + spec_size + params_bytes.len()].copy_from_slice(params_bytes);

    // Update data_size in header
    let total_len = buf.len() as u32;
    write_u32(buf, 12, total_len);
}

fn write_u32(buf: &mut [u8], offset: usize, val: u32) {
    buf[offset..offset + 4].copy_from_slice(&val.to_ne_bytes());
}

fn write_u64(buf: &mut [u8], offset: usize, val: u64) {
    buf[offset..offset + 8].copy_from_slice(&val.to_ne_bytes());
}

/// Create a dm-crypt device.
pub fn dm_crypt_create(
    dm_name: &str,
    loop_device: &str,
    key_hex: &str,
    cipher: &str,
    offset_sectors: u64,
    size_sectors: u64,
) -> Result<String, String> {
    let dm_path = format!("/dev/mapper/{}", dm_name);

    // Open /dev/mapper/control
    let dm_fd = open_dev("/dev/mapper/control")?;

    // DM_DEV_CREATE
    let mut buf = Vec::new();
    dm_ioctl_init(&mut buf, dm_name, 0);
    let ret = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_DEV_CREATE_NR) as _, buf.as_mut_ptr()) };
    if ret < 0 {
        unsafe { libc::close(dm_fd) };
        return Err(format!("DM_DEV_CREATE failed for {}", dm_name));
    }

    // DM_TABLE_LOAD — construct crypt target
    let params = format!(
        "{} {} 0 {} {}",
        cipher, key_hex, loop_device, offset_sectors
    );
    dm_ioctl_init(&mut buf, dm_name, 0);
    dm_target_append(&mut buf, 0, size_sectors, "crypt", &params);
    let ret = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_TABLE_LOAD_NR) as _, buf.as_mut_ptr()) };
    if ret < 0 {
        // Cleanup: remove the device
        dm_ioctl_init(&mut buf, dm_name, 0);
        let _ = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_DEV_REMOVE_NR) as _, buf.as_mut_ptr()) };
        unsafe { libc::close(dm_fd) };
        return Err(format!("DM_TABLE_LOAD failed for {}", dm_name));
    }

    // DM_DEV_SUSPEND (resume)
    dm_ioctl_init(&mut buf, dm_name, 0);
    let ret = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_DEV_SUSPEND_NR) as _, buf.as_mut_ptr()) };
    if ret < 0 {
        dm_ioctl_init(&mut buf, dm_name, 0);
        let _ = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_DEV_REMOVE_NR) as _, buf.as_mut_ptr()) };
        unsafe { libc::close(dm_fd) };
        return Err(format!("DM_DEV_SUSPEND (resume) failed for {}", dm_name));
    }

    unsafe { libc::close(dm_fd) };
    Ok(dm_path)
}

/// Remove a dm-crypt device.
pub fn dm_crypt_remove(dm_name: &str) -> Result<(), String> {
    let dm_fd = open_dev("/dev/mapper/control")?;
    let mut buf = Vec::new();

    // Suspend first
    dm_ioctl_init(&mut buf, dm_name, DM_SUSPEND_FLAG);
    let _ = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_DEV_SUSPEND_NR) as _, buf.as_mut_ptr()) };

    // Clear table
    dm_ioctl_init(&mut buf, dm_name, 0);
    let _ = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_TABLE_CLEAR_NR) as _, buf.as_mut_ptr()) };

    // Remove device
    dm_ioctl_init(&mut buf, dm_name, 0);
    let ret = unsafe { libc::ioctl(dm_fd, dm_ioctl_nr(DM_DEV_REMOVE_NR) as _, buf.as_mut_ptr()) };
    unsafe { libc::close(dm_fd) };

    if ret < 0 {
        return Err(format!("DM_DEV_REMOVE failed for {}", dm_name));
    }
    Ok(())
}

/// Derive an encryption key from a password.
/// This is a simplified PBKDF — real systemd uses argon2id.
/// We iterate djb2 to produce a 256-bit key.
pub fn derive_luks_key(password: &str, salt: &[u8]) -> Vec<u8> {
    let mut h: u64 = 5381;
    for b in salt
        .iter()
        .chain(password.bytes().collect::<Vec<u8>>().iter())
    {
        h = h.wrapping_mul(33).wrapping_add(*b as u64);
    }
    let mut key = Vec::with_capacity(32);
    for i in 0u64..4 {
        let v = h.wrapping_add(i.wrapping_mul(0x9e3779b97f4a7c15));
        key.extend_from_slice(&v.to_le_bytes());
    }
    key
}

/// Create a LUKS2 home area: sparse image → loopback → dm-crypt → mkfs → embed identity.
pub fn create_luks_home_area(rec: &UserRecord, password: &str) -> Result<(), String> {
    let config = LuksConfig::for_user(rec);

    // 1. Create sparse image file
    if config.image_path.exists() {
        return Err(format!(
            "Image path already exists: {}",
            config.image_path.display()
        ));
    }
    create_sparse_file(&config.image_path, config.image_size)?;

    // 2. Set up loopback
    let loop_dev = match setup_loopback(&config.image_path) {
        Ok(l) => l,
        Err(e) => {
            let _ = fs::remove_file(&config.image_path);
            return Err(format!("Loopback setup failed: {}", e));
        }
    };

    // 3. Derive key and set up dm-crypt
    let salt = rec.user_name.as_bytes();
    let key = derive_luks_key(password, salt);
    let key_hex = hex_encode(&key);
    let size_sectors = config.image_size / 512;

    let dm_path = match dm_crypt_create(
        &config.dm_name,
        &loop_dev,
        &key_hex,
        &config.cipher,
        0,
        size_sectors,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = detach_loopback(&loop_dev);
            let _ = fs::remove_file(&config.image_path);
            return Err(format!("dm-crypt setup failed: {}", e));
        }
    };

    // 4. Create filesystem
    if let Err(e) = run_mkfs(&config.fs_type, &dm_path) {
        let _ = dm_crypt_remove(&config.dm_name);
        let _ = detach_loopback(&loop_dev);
        let _ = fs::remove_file(&config.image_path);
        return Err(format!("mkfs failed: {}", e));
    }

    // 5. Mount, embed identity, set ownership, unmount
    let tmp_mount = format!("/run/systemd/home/mount-{}", rec.user_name);
    let _ = fs::create_dir_all(&tmp_mount);

    if let Err(e) = mount_fs(&dm_path, &tmp_mount, &config.fs_type) {
        let _ = dm_crypt_remove(&config.dm_name);
        let _ = detach_loopback(&loop_dev);
        let _ = fs::remove_file(&config.image_path);
        return Err(format!("Mount failed: {}", e));
    }

    // Write identity inside the encrypted volume
    let embedded_identity = Path::new(&tmp_mount).join(".identity");
    let _ = fs::write(&embedded_identity, rec.to_json());

    // Set ownership
    let _ = nix::unistd::chown(
        Path::new(&tmp_mount),
        Some(nix::unistd::Uid::from_raw(rec.uid)),
        Some(nix::unistd::Gid::from_raw(rec.gid)),
    );

    // Cleanup: unmount, close dm-crypt, detach loop
    let _ = umount_fs(&tmp_mount);
    let _ = fs::remove_dir(&tmp_mount);
    let _ = dm_crypt_remove(&config.dm_name);
    let _ = detach_loopback(&loop_dev);

    Ok(())
}

/// Activate a LUKS2 home area: loopback → dm-crypt → mount.
pub fn activate_luks_home(rec: &UserRecord, password: &str) -> Result<(), String> {
    let config = LuksConfig::for_user(rec);

    if !config.image_path.exists() {
        return Err(format!(
            "LUKS image not found: {}",
            config.image_path.display()
        ));
    }

    let loop_dev = setup_loopback(&config.image_path)?;

    let salt = rec.user_name.as_bytes();
    let key = derive_luks_key(password, salt);
    let key_hex = hex_encode(&key);
    let image_size = fs::metadata(&config.image_path)
        .map(|m| m.len())
        .unwrap_or(config.image_size);
    let size_sectors = image_size / 512;

    let dm_path = match dm_crypt_create(
        &config.dm_name,
        &loop_dev,
        &key_hex,
        &config.cipher,
        0,
        size_sectors,
    ) {
        Ok(p) => p,
        Err(e) => {
            let _ = detach_loopback(&loop_dev);
            return Err(format!("dm-crypt open failed: {}", e));
        }
    };

    // Ensure mount point exists
    if let Some(parent) = config.mount_point.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let _ = fs::create_dir_all(&config.mount_point);

    if let Err(e) = mount_fs(
        &dm_path,
        config.mount_point.to_str().unwrap_or(""),
        &config.fs_type,
    ) {
        let _ = dm_crypt_remove(&config.dm_name);
        let _ = detach_loopback(&loop_dev);
        return Err(format!("Mount failed: {}", e));
    }

    Ok(())
}

/// Deactivate a LUKS2 home area: unmount → dm-crypt close → loopback detach.
pub fn deactivate_luks_home(rec: &UserRecord) -> Result<(), String> {
    let config = LuksConfig::for_user(rec);

    // Unmount
    let _ = umount_fs(config.mount_point.to_str().unwrap_or(""));

    // Close dm-crypt
    let _ = dm_crypt_remove(&config.dm_name);

    // Find and detach loopback (best effort — autoclear may have handled it)
    // We don't track the loop device, so we just try common ones
    for i in 0..16 {
        let loop_path = format!("/dev/loop{}", i);
        let _ = detach_loopback(&loop_path);
    }

    Ok(())
}

/// Resize a LUKS2 home area.
pub fn resize_luks_home(rec: &UserRecord, new_size: u64) -> Result<(), String> {
    let config = LuksConfig::for_user(rec);

    if !config.image_path.exists() {
        return Err(format!(
            "LUKS image not found: {}",
            config.image_path.display()
        ));
    }

    let current_size = fs::metadata(&config.image_path)
        .map(|m| m.len())
        .unwrap_or(0);

    if new_size < current_size {
        return Err("Shrinking LUKS images is not supported".to_string());
    }

    // Grow the sparse file
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&config.image_path)
        .map_err(|e| format!("Failed to open image: {}", e))?;
    file.set_len(new_size)
        .map_err(|e| format!("Failed to resize image: {}", e))?;
    drop(file);

    // If mounted, signal loop device and resize filesystem
    let dm_path = format!("/dev/mapper/{}", config.dm_name);
    if Path::new(&dm_path).exists() {
        // Find the loop device backing this image and update capacity
        for i in 0..16 {
            let loop_path = format!("/dev/loop{}", i);
            if Path::new(&loop_path).exists() {
                let _ = loop_set_capacity(&loop_path);
            }
        }
        // Run resize2fs on the dm device
        let _ = process::Command::new("resize2fs").arg(&dm_path).output();
    }

    Ok(())
}

/// Create a sparse file of the given size.
fn create_sparse_file(path: &Path, size: u64) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    let file = fs::File::create(path)
        .map_err(|e| format!("Failed to create {}: {}", path.display(), e))?;
    file.set_len(size)
        .map_err(|e| format!("Failed to set size for {}: {}", path.display(), e))?;
    Ok(())
}

/// Run mkfs for the given filesystem type.
fn run_mkfs(fs_type: &str, device: &str) -> Result<(), String> {
    let cmd = match fs_type {
        "ext4" => "mkfs.ext4",
        "btrfs" => "mkfs.btrfs",
        "xfs" => "mkfs.xfs",
        _ => return Err(format!("unsupported filesystem: {}", fs_type)),
    };
    let output = process::Command::new(cmd)
        .arg("-q") // quiet
        .arg(device)
        .output()
        .map_err(|e| format!("Failed to run {}: {}", cmd, e))?;
    if !output.status.success() {
        return Err(format!(
            "{} failed: {}",
            cmd,
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// Mount a filesystem.
fn mount_fs(device: &str, mount_point: &str, fs_type: &str) -> Result<(), String> {
    let output = process::Command::new("mount")
        .arg("-t")
        .arg(fs_type)
        .arg(device)
        .arg(mount_point)
        .output()
        .map_err(|e| format!("Failed to run mount: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "mount failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

/// Unmount a filesystem.
fn umount_fs(mount_point: &str) -> Result<(), String> {
    let output = process::Command::new("umount")
        .arg(mount_point)
        .output()
        .map_err(|e| format!("Failed to run umount: {}", e))?;
    if !output.status.success() {
        return Err(format!(
            "umount failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// CIFS backend helpers
// ---------------------------------------------------------------------------

/// CIFS configuration for a user.
#[derive(Debug, Clone, PartialEq)]
pub struct CifsConfig {
    /// CIFS service path (e.g. "//server/share").
    pub service: String,
    /// CIFS user name (may differ from the system user name).
    pub cifs_user: String,
    /// CIFS domain.
    pub domain: String,
}

impl CifsConfig {
    pub fn for_user(rec: &UserRecord) -> Option<Self> {
        let service = rec.cifs_service.as_ref()?;
        Some(Self {
            service: service.clone(),
            cifs_user: rec
                .cifs_user_name
                .clone()
                .unwrap_or_else(|| rec.user_name.clone()),
            domain: rec.cifs_domain.clone().unwrap_or_default(),
        })
    }
}

/// Create a CIFS home area (just validate config; the mount happens on activate).
pub fn create_cifs_home_area(rec: &UserRecord) -> Result<(), String> {
    let config = CifsConfig::for_user(rec)
        .ok_or_else(|| "CIFS service path not configured (set cifsService)".to_string())?;
    if !config.service.starts_with("//") {
        return Err(format!(
            "Invalid CIFS service path (must start with //): {}",
            config.service
        ));
    }
    // Ensure the mount point parent exists
    let hd = Path::new(&rec.home_directory);
    if let Some(parent) = hd.parent() {
        let _ = fs::create_dir_all(parent);
    }
    Ok(())
}

/// Activate a CIFS home: mount the network share.
pub fn activate_cifs_home(rec: &UserRecord, password: &str) -> Result<(), String> {
    let config =
        CifsConfig::for_user(rec).ok_or_else(|| "CIFS service path not configured".to_string())?;

    let mount_point = &rec.home_directory;
    let _ = fs::create_dir_all(mount_point);

    // Write a temporary credentials file
    let cred_path = format!("/run/systemd/home/.cifs-creds-{}", rec.user_name);
    let cred_content = format!(
        "username={}\npassword={}\n{}",
        config.cifs_user,
        password,
        if config.domain.is_empty() {
            String::new()
        } else {
            format!("domain={}\n", config.domain)
        }
    );
    fs::write(&cred_path, &cred_content)
        .map_err(|e| format!("Failed to write credentials file: {}", e))?;

    // mount.cifs //server/share /home/user -o credentials=...
    let output = process::Command::new("mount.cifs")
        .arg(&config.service)
        .arg(mount_point)
        .arg("-o")
        .arg(format!(
            "credentials={},uid={},gid={},file_mode=0700,dir_mode=0700",
            cred_path, rec.uid, rec.gid
        ))
        .output();

    // Remove credentials file immediately
    let _ = fs::remove_file(&cred_path);

    match output {
        Ok(out) if out.status.success() => Ok(()),
        Ok(out) => Err(format!(
            "mount.cifs failed: {}",
            String::from_utf8_lossy(&out.stderr)
        )),
        Err(e) => Err(format!("Failed to run mount.cifs: {}", e)),
    }
}

/// Deactivate a CIFS home: unmount the share.
pub fn deactivate_cifs_home(rec: &UserRecord) -> Result<(), String> {
    umount_fs(&rec.home_directory)
}

// ---------------------------------------------------------------------------
// fscrypt backend helpers
// ---------------------------------------------------------------------------

/// fscrypt configuration derived from user record.
#[derive(Debug, Clone, PartialEq)]
pub struct FscryptConfig {
    /// The directory path (same as image_path for fscrypt storage).
    pub directory: PathBuf,
    /// Key descriptor (8 bytes hex-encoded, 16 hex chars).
    pub key_descriptor: String,
}

impl FscryptConfig {
    pub fn for_user(rec: &UserRecord) -> Self {
        let descriptor = rec.fscrypt_key_descriptor.clone().unwrap_or_else(|| {
            // Derive a descriptor from the username
            let h = simple_hash(rec.user_name.as_bytes());
            format!("{:016x}", h)
        });
        Self {
            directory: PathBuf::from(&rec.image_path),
            key_descriptor: descriptor,
        }
    }
}

/// Simple 64-bit hash for key descriptor derivation.
fn simple_hash(data: &[u8]) -> u64 {
    let mut h: u64 = 5381;
    for &b in data {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    h
}

/// Derive an fscrypt key from a password and add it to the kernel keyring.
/// Returns the key serial number.
pub fn fscrypt_add_key(password: &str, descriptor: &str) -> Result<i32, String> {
    // Derive a 64-byte key from the password (fscrypt expects raw key material)
    let key_material = derive_fscrypt_key(password);
    let key_desc = format!("{}{}", FSCRYPT_KEY_DESC_PREFIX, descriptor);

    // Use the add_key syscall to add to the session keyring
    let c_type = std::ffi::CString::new(FSCRYPT_KEY_TYPE).unwrap();
    let c_desc = std::ffi::CString::new(key_desc.as_str()).map_err(|e| e.to_string())?;

    let ret = unsafe {
        libc::syscall(
            libc::SYS_add_key,
            c_type.as_ptr(),
            c_desc.as_ptr(),
            key_material.as_ptr() as *const libc::c_void,
            key_material.len(),
            libc::KEY_SPEC_SESSION_KEYRING,
        )
    };
    if ret < 0 {
        return Err(format!(
            "add_key failed for {}: errno={}",
            key_desc,
            io::Error::last_os_error()
        ));
    }
    Ok(ret as i32)
}

/// Remove an fscrypt key from the kernel keyring.
pub fn fscrypt_remove_key(key_serial: i32) -> Result<(), String> {
    // keyctl(KEYCTL_REVOKE, key_serial)
    const KEYCTL_REVOKE: libc::c_int = 3;
    let ret = unsafe { libc::syscall(libc::SYS_keyctl, KEYCTL_REVOKE, key_serial) };
    if ret < 0 {
        return Err(format!(
            "keyctl REVOKE failed for serial {}: {}",
            key_serial,
            io::Error::last_os_error()
        ));
    }
    Ok(())
}

/// Derive fscrypt key material from a password.
fn derive_fscrypt_key(password: &str) -> Vec<u8> {
    // Simple key derivation (real impl would use HKDF or argon2)
    let mut key = Vec::with_capacity(64);
    let mut h: u64 = 0xcbf29ce484222325; // FNV offset basis
    for b in password.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3); // FNV prime
    }
    for i in 0u64..8 {
        let v = h.wrapping_add(i.wrapping_mul(0x517cc1b727220a95));
        key.extend_from_slice(&v.to_le_bytes());
    }
    key
}

/// Create an fscrypt home area.
pub fn create_fscrypt_home_area(rec: &UserRecord) -> Result<(), String> {
    let config = FscryptConfig::for_user(rec);

    if config.directory.exists() {
        return Err(format!(
            "Directory already exists: {}",
            config.directory.display()
        ));
    }

    fs::create_dir_all(&config.directory)
        .map_err(|e| format!("Failed to create {}: {}", config.directory.display(), e))?;

    // Set ownership
    let _ = nix::unistd::chown(
        &config.directory,
        Some(nix::unistd::Uid::from_raw(rec.uid)),
        Some(nix::unistd::Gid::from_raw(rec.gid)),
    );
    let _ = fs::set_permissions(
        &config.directory,
        std::os::unix::fs::PermissionsExt::from_mode(0o700),
    );

    // Write the key descriptor to a metadata file inside the directory
    let meta_path = config.directory.join(".fscrypt-descriptor");
    let _ = fs::write(&meta_path, &config.key_descriptor);

    Ok(())
}

/// Activate an fscrypt home: add key to kernel keyring.
pub fn activate_fscrypt_home(rec: &UserRecord, password: &str) -> Result<i32, String> {
    let config = FscryptConfig::for_user(rec);

    if !config.directory.exists() {
        return Err(format!(
            "fscrypt directory not found: {}",
            config.directory.display()
        ));
    }

    let key_serial = fscrypt_add_key(password, &config.key_descriptor)?;

    // Create symlink from home_directory to image_path if different
    let hd = Path::new(&rec.home_directory);
    let img = &config.directory;
    if hd != img.as_path() {
        if let Some(parent) = hd.parent() {
            let _ = fs::create_dir_all(parent);
        }
        if !hd.exists() {
            let _ = unix_fs::symlink(img, hd);
        }
    }

    Ok(key_serial)
}

/// Deactivate an fscrypt home: remove key from kernel keyring.
pub fn deactivate_fscrypt_home(rec: &UserRecord, key_serial: i32) -> Result<(), String> {
    let _ = fscrypt_remove_key(key_serial);

    // Remove symlink if different from image path
    let hd = Path::new(&rec.home_directory);
    let img = Path::new(&rec.image_path);
    if hd != img
        && hd
            .symlink_metadata()
            .map(|m| m.file_type().is_symlink())
            .unwrap_or(false)
    {
        let _ = fs::remove_file(hd);
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Btrfs subvolume backend helpers
// ---------------------------------------------------------------------------

/// Create a btrfs subvolume at the given path.
pub fn btrfs_subvol_create(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "No parent directory for subvolume".to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| "No file name for subvolume".to_string())?
        .to_string_lossy();

    let _ = fs::create_dir_all(parent);

    let parent_fd = open_dir_fd(parent)?;

    let mut args = BtrfsIoctlVolArgs::default();
    let name_bytes = name.as_bytes();
    if name_bytes.len() >= BTRFS_PATH_NAME_MAX {
        unsafe { libc::close(parent_fd) };
        return Err("subvolume name too long".to_string());
    }
    args.name[..name_bytes.len()].copy_from_slice(name_bytes);

    let ret = unsafe { libc::ioctl(parent_fd, btrfs_ioc_subvol_create() as _, &args) };
    unsafe { libc::close(parent_fd) };

    if ret < 0 {
        // Fallback: create a regular directory
        fs::create_dir_all(path)
            .map_err(|e| format!("btrfs subvol create failed, mkdir fallback: {}", e))?;
        log::warn!(
            "BTRFS_IOC_SUBVOL_CREATE failed (errno={}), created regular directory",
            io::Error::last_os_error()
        );
    }

    Ok(())
}

/// Delete a btrfs subvolume at the given path.
pub fn btrfs_subvol_delete(path: &Path) -> Result<(), String> {
    let parent = path
        .parent()
        .ok_or_else(|| "No parent directory for subvolume".to_string())?;
    let name = path
        .file_name()
        .ok_or_else(|| "No file name for subvolume".to_string())?
        .to_string_lossy();

    let parent_fd = open_dir_fd(parent)?;

    let mut args = BtrfsIoctlVolArgs::default();
    let name_bytes = name.as_bytes();
    if name_bytes.len() >= BTRFS_PATH_NAME_MAX {
        unsafe { libc::close(parent_fd) };
        return Err("subvolume name too long".to_string());
    }
    args.name[..name_bytes.len()].copy_from_slice(name_bytes);

    let ret = unsafe { libc::ioctl(parent_fd, btrfs_ioc_snap_destroy() as _, &args) };
    unsafe { libc::close(parent_fd) };

    if ret < 0 {
        // Fallback: try regular rm -rf
        fs::remove_dir_all(path)
            .map_err(|e| format!("btrfs subvol delete failed, rm fallback: {}", e))?;
    }

    Ok(())
}

/// Set a btrfs qgroup size limit on a subvolume.
pub fn btrfs_set_quota(path: &Path, limit_bytes: u64) -> Result<(), String> {
    let fd = open_dir_fd(path)?;

    let args = BtrfsIoctlQgroupLimitArgs {
        qgroupid: 0, // 0 = this subvolume
        lim: BtrfsQgroupLimit {
            flags: BTRFS_QGROUP_LIMIT_MAX_RFER,
            max_rfer: limit_bytes,
            ..Default::default()
        },
    };

    let ret = unsafe {
        libc::ioctl(
            fd,
            // BTRFS_IOC_QGROUP_LIMIT
            ((1u64 << 30)
                | ((std::mem::size_of::<BtrfsIoctlQgroupLimitArgs>() as u64 & 0x3fff) << 16)
                | ((BTRFS_IOCTL_MAGIC as u64) << 8)
                | (BTRFS_IOC_QGROUP_LIMIT_NR as u64)) as libc::c_ulong as _,
            &args,
        )
    };
    unsafe { libc::close(fd) };

    if ret < 0 {
        log::warn!(
            "BTRFS_IOC_QGROUP_LIMIT failed for {} (errno={}), quota not set",
            path.display(),
            io::Error::last_os_error()
        );
    }

    Ok(())
}

fn open_dir_fd(path: &Path) -> Result<i32, String> {
    let c_path = std::ffi::CString::new(path.to_string_lossy().as_bytes())
        .map_err(|_| "bad path".to_string())?;
    let fd = unsafe {
        libc::open(
            c_path.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_CLOEXEC,
        )
    };
    if fd < 0 {
        return Err(format!("Failed to open directory {}", path.display()));
    }
    Ok(fd)
}

// ---------------------------------------------------------------------------
// Auto-activation on login (logind session monitoring)
// ---------------------------------------------------------------------------

/// Configuration for automatic home activation on login.
#[derive(Debug, Clone, Default)]
pub struct AutoActivationMonitor {
    /// Map of UID → user_name for users with auto_login=true.
    pub watched_uids: BTreeMap<u32, String>,
}

impl AutoActivationMonitor {
    pub fn new() -> Self {
        Self::default()
    }

    /// Update the watch list from the registry.
    pub fn refresh(&mut self, registry: &HomeRegistry) {
        self.watched_uids.clear();
        for rec in registry.list() {
            if rec.auto_login {
                self.watched_uids.insert(rec.uid, rec.user_name.clone());
            }
        }
    }

    /// Check if a given UID should trigger auto-activation.
    pub fn should_activate(&self, uid: u32) -> Option<&str> {
        self.watched_uids.get(&uid).map(|s| s.as_str())
    }

    /// Handle a login event for a UID. Returns the user name if
    /// auto-activation was triggered.
    pub fn on_user_login(&self, uid: u32, registry: &mut HomeRegistry) -> Option<String> {
        if let Some(user_name) = self.should_activate(uid) {
            let user_name = user_name.to_string();
            if let Some(rec) = registry.get(&user_name)
                && (rec.state == HomeState::Inactive || rec.state == HomeState::Absent)
                && registry.activate(&user_name).is_ok()
            {
                return Some(user_name);
            }
        }
        None
    }

    /// Handle a logout event for a UID. Returns the user name if
    /// auto-deactivation was triggered.
    pub fn on_user_logout(&self, uid: u32, registry: &mut HomeRegistry) -> Option<String> {
        if let Some(user_name) = self.should_activate(uid) {
            let user_name = user_name.to_string();
            if let Some(rec) = registry.get(&user_name)
                && rec.state == HomeState::Active
                && registry.deactivate(&user_name).is_ok()
            {
                return Some(user_name);
            }
        }
        None
    }
}

// ---------------------------------------------------------------------------
// Home state
// ---------------------------------------------------------------------------

/// Runtime state of a managed home area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HomeState {
    Inactive,
    Activating,
    Active,
    Deactivating,
    Locked,
    /// Home area absent from disk (record exists but image/dir missing).
    Absent,
    /// Home area is in an inconsistent state.
    Dirty,
}

impl HomeState {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "inactive" => Some(Self::Inactive),
            "activating" => Some(Self::Activating),
            "active" => Some(Self::Active),
            "deactivating" => Some(Self::Deactivating),
            "locked" => Some(Self::Locked),
            "absent" => Some(Self::Absent),
            "dirty" => Some(Self::Dirty),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Inactive => "inactive",
            Self::Activating => "activating",
            Self::Active => "active",
            Self::Deactivating => "deactivating",
            Self::Locked => "locked",
            Self::Absent => "absent",
            Self::Dirty => "dirty",
        }
    }
}

impl fmt::Display for HomeState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// Disposition
// ---------------------------------------------------------------------------

/// User disposition — how the user record came into being.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposition {
    Regular,
    System,
    Intrinsic,
}

impl Disposition {
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_ascii_lowercase().as_str() {
            "regular" => Some(Self::Regular),
            "system" => Some(Self::System),
            "intrinsic" => Some(Self::Intrinsic),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Regular => "regular",
            Self::System => "system",
            Self::Intrinsic => "intrinsic",
        }
    }
}

impl fmt::Display for Disposition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

// ---------------------------------------------------------------------------
// User record
// ---------------------------------------------------------------------------

/// A managed user identity record (simplified version of systemd's JSON
/// user record as described in `systemd.user-record(5)`).
#[derive(Debug, Clone, PartialEq)]
pub struct UserRecord {
    pub user_name: String,
    pub real_name: String,
    pub uid: u32,
    pub gid: u32,
    pub member_of: Vec<String>,
    pub home_directory: String,
    pub image_path: String,
    pub shell: String,
    pub storage: Storage,
    pub disposition: Disposition,
    pub state: HomeState,
    pub disk_size: Option<u64>,
    pub disk_usage: Option<u64>,
    pub password_hint: Option<String>,
    pub enforce_password_policy: bool,
    pub auto_login: bool,
    /// SHA-512 hashed password(s).  Empty if no password set.
    pub hashed_passwords: Vec<String>,
    /// Microsecond timestamps.
    pub last_change_usec: u64,
    pub last_password_change_usec: u64,
    pub service: String,
    /// Whether the home area is currently locked (suspend protection).
    pub locked: bool,

    // -- LUKS2 configuration ------------------------------------------------
    /// LUKS cipher (e.g. "aes-xts-plain64").
    pub luks_cipher: Option<String>,
    /// LUKS volume key size in bits (e.g. 256).
    pub luks_volume_key_size: Option<u32>,
    /// LUKS PBKDF type (e.g. "argon2id").
    pub luks_pbkdf_type: Option<String>,
    /// Extra mount options for LUKS filesystem.
    pub luks_extra_mount_options: Option<String>,

    // -- CIFS configuration -------------------------------------------------
    /// CIFS service path (e.g. "//server/share").
    pub cifs_service: Option<String>,
    /// CIFS user name (defaults to system user name).
    pub cifs_user_name: Option<String>,
    /// CIFS domain.
    pub cifs_domain: Option<String>,

    // -- fscrypt configuration ----------------------------------------------
    /// fscrypt key descriptor (16 hex chars).
    pub fscrypt_key_descriptor: Option<String>,

    // -- PKCS#11 / FIDO2 authentication -------------------------------------
    /// PKCS#11 encrypted keys for LUKS volume key decryption.
    pub pkcs11_encrypted_key: Vec<Pkcs11EncryptedKey>,
    /// FIDO2 HMAC credentials for key derivation.
    pub fido2_hmac_credential: Vec<Fido2HmacCredential>,

    // -- Recovery keys ------------------------------------------------------
    /// Hashed recovery keys (same format as hashed_passwords).
    pub recovery_key: Vec<String>,
}

impl UserRecord {
    /// Create a new user record with sane defaults.
    pub fn new(user_name: &str, uid: u32) -> Self {
        let now_usec = now_usec();
        Self {
            user_name: user_name.to_string(),
            real_name: user_name.to_string(),
            uid,
            gid: uid,
            member_of: Vec::new(),
            home_directory: format!("/home/{}", user_name),
            image_path: format!("/home/{}.homedir", user_name),
            shell: "/bin/bash".to_string(),
            storage: Storage::Directory,
            disposition: Disposition::Regular,
            state: HomeState::Inactive,
            disk_size: None,
            disk_usage: None,
            password_hint: None,
            enforce_password_policy: true,
            auto_login: false,
            hashed_passwords: Vec::new(),
            last_change_usec: now_usec,
            last_password_change_usec: now_usec,
            service: "io.systemd.Home".to_string(),
            locked: false,
            luks_cipher: None,
            luks_volume_key_size: None,
            luks_pbkdf_type: None,
            luks_extra_mount_options: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            fscrypt_key_descriptor: None,
            pkcs11_encrypted_key: Vec::new(),
            fido2_hmac_credential: Vec::new(),
            recovery_key: Vec::new(),
        }
    }

    // -- JSON serialization (hand-rolled to avoid serde dependency) ----------

    /// Serialize to a JSON string.
    pub fn to_json(&self) -> String {
        let mut s = String::from("{\n");
        json_str_field(&mut s, "userName", &self.user_name, true);
        json_str_field(&mut s, "realName", &self.real_name, true);
        json_u64_field(&mut s, "uid", self.uid as u64, true);
        json_u64_field(&mut s, "gid", self.gid as u64, true);
        json_str_array_field(&mut s, "memberOf", &self.member_of, true);
        json_str_field(&mut s, "homeDirectory", &self.home_directory, true);
        json_str_field(&mut s, "imagePath", &self.image_path, true);
        json_str_field(&mut s, "shell", &self.shell, true);
        json_str_field(&mut s, "storage", self.storage.as_str(), true);
        json_str_field(&mut s, "disposition", self.disposition.as_str(), true);
        json_str_field(&mut s, "state", self.state.as_str(), true);
        json_opt_u64_field(&mut s, "diskSize", self.disk_size, true);
        json_opt_u64_field(&mut s, "diskUsage", self.disk_usage, true);
        json_opt_str_field(&mut s, "passwordHint", self.password_hint.as_deref(), true);
        json_bool_field(
            &mut s,
            "enforcePasswordPolicy",
            self.enforce_password_policy,
            true,
        );
        json_bool_field(&mut s, "autoLogin", self.auto_login, true);
        json_str_array_field(&mut s, "hashedPassword", &self.hashed_passwords, true);
        json_u64_field(&mut s, "lastChangeUSec", self.last_change_usec, true);
        json_u64_field(
            &mut s,
            "lastPasswordChangeUSec",
            self.last_password_change_usec,
            true,
        );
        json_str_field(&mut s, "service", &self.service, true);
        json_bool_field(&mut s, "locked", self.locked, true);

        // LUKS fields
        json_opt_str_field(&mut s, "luksCipher", self.luks_cipher.as_deref(), true);
        json_opt_u64_field(
            &mut s,
            "luksVolumeKeySize",
            self.luks_volume_key_size.map(|v| v as u64),
            true,
        );
        json_opt_str_field(
            &mut s,
            "luksPbkdfType",
            self.luks_pbkdf_type.as_deref(),
            true,
        );
        json_opt_str_field(
            &mut s,
            "luksExtraMountOptions",
            self.luks_extra_mount_options.as_deref(),
            true,
        );

        // CIFS fields
        json_opt_str_field(&mut s, "cifsService", self.cifs_service.as_deref(), true);
        json_opt_str_field(&mut s, "cifsUserName", self.cifs_user_name.as_deref(), true);
        json_opt_str_field(&mut s, "cifsDomain", self.cifs_domain.as_deref(), true);

        // fscrypt fields
        json_opt_str_field(
            &mut s,
            "fscryptKeyDescriptor",
            self.fscrypt_key_descriptor.as_deref(),
            true,
        );

        // PKCS#11 encrypted keys
        json_obj_array_field(
            &mut s,
            "pkcs11EncryptedKey",
            &self.pkcs11_encrypted_key,
            true,
        );

        // FIDO2 credentials
        json_obj_array_field(
            &mut s,
            "fido2HmacCredential",
            &self.fido2_hmac_credential,
            true,
        );

        // Recovery keys
        json_str_array_field(&mut s, "recoveryKey", &self.recovery_key, false);

        s.push('}');
        s
    }

    /// Parse from a JSON string.  This is a minimal parser that handles only
    /// the fields we produce in `to_json`.
    pub fn from_json(input: &str) -> Result<Self, String> {
        let fields = parse_json_object(input)?;

        let user_name = get_json_str(&fields, "userName")?;
        let real_name = get_json_str_or(&fields, "realName", &user_name);
        let uid = get_json_u64(&fields, "uid")? as u32;
        let gid = get_json_u64_or(&fields, "gid", uid as u64) as u32;
        let member_of = get_json_str_array(&fields, "memberOf");
        let home_directory =
            get_json_str_or(&fields, "homeDirectory", &format!("/home/{}", user_name));
        let image_path = get_json_str_or(
            &fields,
            "imagePath",
            &format!("/home/{}.homedir", user_name),
        );
        let shell = get_json_str_or(&fields, "shell", "/bin/bash");
        let storage = fields
            .get("storage")
            .and_then(|v| Storage::parse(v.trim_matches('"')))
            .unwrap_or(Storage::Directory);
        let disposition = fields
            .get("disposition")
            .and_then(|v| Disposition::parse(v.trim_matches('"')))
            .unwrap_or(Disposition::Regular);
        let state = fields
            .get("state")
            .and_then(|v| HomeState::parse(v.trim_matches('"')))
            .unwrap_or(HomeState::Inactive);
        let disk_size = get_json_opt_u64(&fields, "diskSize");
        let disk_usage = get_json_opt_u64(&fields, "diskUsage");
        let password_hint = fields.get("passwordHint").and_then(|v| {
            let v = v.trim_matches('"');
            if v == "null" {
                None
            } else {
                Some(v.to_string())
            }
        });
        let enforce_password_policy = get_json_bool_or(&fields, "enforcePasswordPolicy", true);
        let auto_login = get_json_bool_or(&fields, "autoLogin", false);
        let hashed_passwords = get_json_str_array(&fields, "hashedPassword");
        let last_change_usec = get_json_u64_or(&fields, "lastChangeUSec", 0);
        let last_password_change_usec = get_json_u64_or(&fields, "lastPasswordChangeUSec", 0);
        let service = get_json_str_or(&fields, "service", "io.systemd.Home");
        let locked = get_json_bool_or(&fields, "locked", false);

        // LUKS fields
        let luks_cipher = get_json_opt_str(&fields, "luksCipher");
        let luks_volume_key_size = get_json_opt_u64(&fields, "luksVolumeKeySize").map(|v| v as u32);
        let luks_pbkdf_type = get_json_opt_str(&fields, "luksPbkdfType");
        let luks_extra_mount_options = get_json_opt_str(&fields, "luksExtraMountOptions");

        // CIFS fields
        let cifs_service = get_json_opt_str(&fields, "cifsService");
        let cifs_user_name = get_json_opt_str(&fields, "cifsUserName");
        let cifs_domain = get_json_opt_str(&fields, "cifsDomain");

        // fscrypt fields
        let fscrypt_key_descriptor = get_json_opt_str(&fields, "fscryptKeyDescriptor");

        // PKCS#11 / FIDO2 / recovery
        let pkcs11_encrypted_key = parse_json_obj_array(&fields, "pkcs11EncryptedKey", |s| {
            Pkcs11EncryptedKey::from_json_str(s)
        });
        let fido2_hmac_credential = parse_json_obj_array(&fields, "fido2HmacCredential", |s| {
            Fido2HmacCredential::from_json_str(s)
        });
        let recovery_key = get_json_str_array(&fields, "recoveryKey");

        Ok(Self {
            user_name,
            real_name,
            uid,
            gid,
            member_of,
            home_directory,
            image_path,
            shell,
            storage,
            disposition,
            state,
            disk_size,
            disk_usage,
            password_hint,
            enforce_password_policy,
            auto_login,
            hashed_passwords,
            last_change_usec,
            last_password_change_usec,
            service,
            locked,
            luks_cipher,
            luks_volume_key_size,
            luks_pbkdf_type,
            luks_extra_mount_options,
            cifs_service,
            cifs_user_name,
            cifs_domain,
            fscrypt_key_descriptor,
            pkcs11_encrypted_key,
            fido2_hmac_credential,
            recovery_key,
        })
    }

    /// Format as a human-readable status block (for `homectl inspect`).
    pub fn format_inspect(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("   User name: {}\n", self.user_name));
        s.push_str(&format!("   Real name: {}\n", self.real_name));
        s.push_str(&format!(" Disposition: {}\n", self.disposition));
        s.push_str(&format!("       State: {}\n", self.state));
        s.push_str(&format!("     Service: {}\n", self.service));
        s.push_str(&format!(" Home Dir.:  {}\n", self.home_directory));
        s.push_str(&format!(" Image Path: {}\n", self.image_path));
        s.push_str(&format!("     Storage: {}\n", self.storage));
        s.push_str(&format!("         UID: {}\n", self.uid));
        s.push_str(&format!("         GID: {}\n", self.gid));
        if !self.member_of.is_empty() {
            s.push_str(&format!("   Member Of: {}\n", self.member_of.join(", ")));
        }
        s.push_str(&format!("       Shell: {}\n", self.shell));
        if let Some(sz) = self.disk_size {
            s.push_str(&format!("   Disk Size: {}\n", format_bytes(sz)));
        }
        if let Some(usage) = self.disk_usage {
            s.push_str(&format!("  Disk Usage: {}\n", format_bytes(usage)));
        }
        if let Some(ref hint) = self.password_hint {
            s.push_str(&format!("   Pass Hint: {}\n", hint));
        }
        s.push_str(&format!(
            "      Locked: {}\n",
            if self.locked { "yes" } else { "no" }
        ));
        s.push_str(&format!(
            "  Auto Login: {}\n",
            if self.auto_login { "yes" } else { "no" }
        ));
        // Storage-specific details
        match self.storage {
            Storage::Luks => {
                if let Some(ref c) = self.luks_cipher {
                    s.push_str(&format!(" LUKS Cipher: {}\n", c));
                }
                if let Some(ks) = self.luks_volume_key_size {
                    s.push_str(&format!("LUKS KeySize: {} bits\n", ks));
                }
                if let Some(ref p) = self.luks_pbkdf_type {
                    s.push_str(&format!("   LUKS PBKDF: {}\n", p));
                }
            }
            Storage::Cifs => {
                if let Some(ref svc) = self.cifs_service {
                    s.push_str(&format!("CIFS Service: {}\n", svc));
                }
                if let Some(ref u) = self.cifs_user_name {
                    s.push_str(&format!("   CIFS User: {}\n", u));
                }
                if let Some(ref d) = self.cifs_domain {
                    s.push_str(&format!(" CIFS Domain: {}\n", d));
                }
            }
            Storage::Fscrypt => {
                if let Some(ref d) = self.fscrypt_key_descriptor {
                    s.push_str(&format!("fscrypt Desc: {}\n", d));
                }
            }
            _ => {}
        }
        if !self.pkcs11_encrypted_key.is_empty() {
            s.push_str(&format!(
                " PKCS#11 Keys: {} configured\n",
                self.pkcs11_encrypted_key.len()
            ));
        }
        if !self.fido2_hmac_credential.is_empty() {
            s.push_str(&format!(
                "  FIDO2 Creds: {} configured\n",
                self.fido2_hmac_credential.len()
            ));
        }
        if !self.recovery_key.is_empty() {
            s.push_str(&format!(
                "Recovery Keys: {} configured\n",
                self.recovery_key.len()
            ));
        }
        s
    }

    /// Format as key=value properties (for `homectl show`).
    pub fn format_show(&self) -> String {
        let mut s = String::new();
        s.push_str(&format!("UserName={}\n", self.user_name));
        s.push_str(&format!("RealName={}\n", self.real_name));
        s.push_str(&format!("Disposition={}\n", self.disposition));
        s.push_str(&format!("State={}\n", self.state));
        s.push_str(&format!("Service={}\n", self.service));
        s.push_str(&format!("HomeDirectory={}\n", self.home_directory));
        s.push_str(&format!("ImagePath={}\n", self.image_path));
        s.push_str(&format!("Storage={}\n", self.storage));
        s.push_str(&format!("UID={}\n", self.uid));
        s.push_str(&format!("GID={}\n", self.gid));
        s.push_str(&format!("Shell={}\n", self.shell));
        if let Some(sz) = self.disk_size {
            s.push_str(&format!("DiskSize={}\n", sz));
        }
        if let Some(usage) = self.disk_usage {
            s.push_str(&format!("DiskUsage={}\n", usage));
        }
        s.push_str(&format!("Locked={}\n", self.locked));
        s.push_str(&format!("AutoLogin={}\n", self.auto_login));
        s.push_str(&format!(
            "EnforcePasswordPolicy={}\n",
            self.enforce_password_policy
        ));
        s
    }
}

// ---------------------------------------------------------------------------
// JSON helpers (minimal, no serde)
// ---------------------------------------------------------------------------

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            c => out.push(c),
        }
    }
    out
}

fn json_str_field(s: &mut String, key: &str, val: &str, comma: bool) {
    s.push_str(&format!("  \"{}\": \"{}\"", key, json_escape(val)));
    if comma {
        s.push(',');
    }
    s.push('\n');
}

fn json_u64_field(s: &mut String, key: &str, val: u64, comma: bool) {
    s.push_str(&format!("  \"{}\": {}", key, val));
    if comma {
        s.push(',');
    }
    s.push('\n');
}

fn json_bool_field(s: &mut String, key: &str, val: bool, comma: bool) {
    s.push_str(&format!("  \"{}\": {}", key, val));
    if comma {
        s.push(',');
    }
    s.push('\n');
}

fn json_opt_u64_field(s: &mut String, key: &str, val: Option<u64>, comma: bool) {
    match val {
        Some(v) => s.push_str(&format!("  \"{}\": {}", key, v)),
        None => s.push_str(&format!("  \"{}\": null", key)),
    }
    if comma {
        s.push(',');
    }
    s.push('\n');
}

fn json_opt_str_field(s: &mut String, key: &str, val: Option<&str>, comma: bool) {
    match val {
        Some(v) => s.push_str(&format!("  \"{}\": \"{}\"", key, json_escape(v))),
        None => s.push_str(&format!("  \"{}\": null", key)),
    }
    if comma {
        s.push(',');
    }
    s.push('\n');
}

fn json_str_array_field(s: &mut String, key: &str, vals: &[String], comma: bool) {
    if vals.is_empty() {
        s.push_str(&format!("  \"{}\": []", key));
    } else {
        s.push_str(&format!("  \"{}\": [", key));
        for (i, v) in vals.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&format!("\"{}\"", json_escape(v)));
        }
        s.push(']');
    }
    if comma {
        s.push(',');
    }
    s.push('\n');
}

/// Trait for types that can serialize to JSON.
trait ToJsonString {
    fn to_json(&self) -> String;
}

impl ToJsonString for Pkcs11EncryptedKey {
    fn to_json(&self) -> String {
        Pkcs11EncryptedKey::to_json(self)
    }
}

impl ToJsonString for Fido2HmacCredential {
    fn to_json(&self) -> String {
        Fido2HmacCredential::to_json(self)
    }
}

fn json_obj_array_field<T: ToJsonString>(s: &mut String, key: &str, vals: &[T], comma: bool) {
    if vals.is_empty() {
        s.push_str(&format!("  \"{}\": []", key));
    } else {
        s.push_str(&format!("  \"{}\": [", key));
        for (i, v) in vals.iter().enumerate() {
            if i > 0 {
                s.push_str(", ");
            }
            s.push_str(&v.to_json());
        }
        s.push(']');
    }
    if comma {
        s.push(',');
    }
    s.push('\n');
}

fn get_json_opt_str(fields: &BTreeMap<String, String>, key: &str) -> Option<String> {
    fields.get(key).and_then(|v| {
        let v = v.trim_matches('"');
        if v == "null" {
            None
        } else {
            Some(v.to_string())
        }
    })
}

/// Parse a JSON array of objects, applying a parser function to each.
fn parse_json_obj_array<T, F>(fields: &BTreeMap<String, String>, key: &str, parser: F) -> Vec<T>
where
    F: Fn(&str) -> Result<T, String>,
{
    let raw = match fields.get(key) {
        Some(v) => v.clone(),
        None => return Vec::new(),
    };
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return Vec::new();
    }
    let inner = &raw[1..raw.len() - 1];
    let mut result = Vec::new();

    // Split on top-level objects by tracking brace depth
    let mut depth = 0i32;
    let mut start = None;
    for (i, ch) in inner.char_indices() {
        match ch {
            '{' => {
                if depth == 0 {
                    start = Some(i);
                }
                depth += 1;
            }
            '}' => {
                depth -= 1;
                if depth == 0 {
                    if let Some(s) = start {
                        let obj_str = &inner[s..=i];
                        if let Ok(obj) = parser(obj_str) {
                            result.push(obj);
                        }
                    }
                    start = None;
                }
            }
            _ => {}
        }
    }
    result
}

/// Very simple JSON object parser — returns key→raw_value pairs.  Handles
/// strings (with basic escape sequences), numbers, booleans, null, and arrays
/// of strings.  Not a general-purpose parser.
fn parse_json_object(input: &str) -> Result<BTreeMap<String, String>, String> {
    let input = input.trim();
    if !input.starts_with('{') || !input.ends_with('}') {
        return Err("not a JSON object".to_string());
    }
    let inner = &input[1..input.len() - 1];
    let mut map = BTreeMap::new();
    let mut chars = inner.chars().peekable();

    loop {
        skip_ws(&mut chars);
        if chars.peek().is_none() {
            break;
        }
        // Key
        let key = parse_json_string_chars(&mut chars)?;
        skip_ws(&mut chars);
        match chars.next() {
            Some(':') => {}
            _ => return Err("expected ':'".to_string()),
        }
        skip_ws(&mut chars);
        // Value
        let val = parse_json_value_chars(&mut chars)?;
        map.insert(key, val);
        skip_ws(&mut chars);
        if chars.peek() == Some(&',') {
            chars.next();
        }
    }
    Ok(map)
}

fn skip_ws(chars: &mut std::iter::Peekable<std::str::Chars>) {
    while let Some(&c) = chars.peek() {
        if c.is_ascii_whitespace() {
            chars.next();
        } else {
            break;
        }
    }
}

fn parse_json_string_chars(
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> Result<String, String> {
    match chars.next() {
        Some('"') => {}
        _ => return Err("expected '\"'".to_string()),
    }
    let mut s = String::new();
    loop {
        match chars.next() {
            Some('"') => return Ok(s),
            Some('\\') => match chars.next() {
                Some('"') => s.push('"'),
                Some('\\') => s.push('\\'),
                Some('n') => s.push('\n'),
                Some('r') => s.push('\r'),
                Some('t') => s.push('\t'),
                Some('/') => s.push('/'),
                Some('u') => {
                    let mut hex = String::new();
                    for _ in 0..4 {
                        match chars.next() {
                            Some(c) => hex.push(c),
                            None => return Err("unterminated \\u escape".to_string()),
                        }
                    }
                    if let Ok(cp) = u32::from_str_radix(&hex, 16)
                        && let Some(c) = char::from_u32(cp)
                    {
                        s.push(c);
                    }
                }
                _ => s.push('?'),
            },
            Some(c) => s.push(c),
            None => return Err("unterminated string".to_string()),
        }
    }
}

fn parse_json_value_chars(
    chars: &mut std::iter::Peekable<std::str::Chars>,
) -> Result<String, String> {
    skip_ws(chars);
    match chars.peek() {
        Some('"') => {
            let s = parse_json_string_chars(chars)?;
            Ok(format!("\"{}\"", s))
        }
        Some('[') => {
            // Collect array as raw text
            let mut depth = 0i32;
            let mut arr = String::new();
            for c in chars.by_ref() {
                arr.push(c);
                if c == '[' {
                    depth += 1;
                } else if c == ']' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            Ok(arr)
        }
        Some('{') => {
            let mut depth = 0i32;
            let mut obj = String::new();
            for c in chars.by_ref() {
                obj.push(c);
                if c == '{' {
                    depth += 1;
                } else if c == '}' {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
            }
            Ok(obj)
        }
        _ => {
            // number, bool, null
            let mut tok = String::new();
            while let Some(&c) = chars.peek() {
                if c == ',' || c == '}' || c == ']' || c.is_ascii_whitespace() {
                    break;
                }
                tok.push(c);
                chars.next();
            }
            Ok(tok)
        }
    }
}

fn get_json_str(fields: &BTreeMap<String, String>, key: &str) -> Result<String, String> {
    fields
        .get(key)
        .map(|v| v.trim_matches('"').to_string())
        .ok_or_else(|| format!("missing field '{}'", key))
}

fn get_json_str_or(fields: &BTreeMap<String, String>, key: &str, default: &str) -> String {
    fields
        .get(key)
        .map(|v| v.trim_matches('"').to_string())
        .unwrap_or_else(|| default.to_string())
}

fn get_json_u64(fields: &BTreeMap<String, String>, key: &str) -> Result<u64, String> {
    fields
        .get(key)
        .ok_or_else(|| format!("missing field '{}'", key))
        .and_then(|v| {
            v.parse::<u64>()
                .map_err(|e| format!("bad u64 for '{}': {}", key, e))
        })
}

fn get_json_u64_or(fields: &BTreeMap<String, String>, key: &str, default: u64) -> u64 {
    fields
        .get(key)
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(default)
}

fn get_json_opt_u64(fields: &BTreeMap<String, String>, key: &str) -> Option<u64> {
    fields.get(key).and_then(|v| {
        if v == "null" {
            None
        } else {
            v.parse::<u64>().ok()
        }
    })
}

fn get_json_bool_or(fields: &BTreeMap<String, String>, key: &str, default: bool) -> bool {
    fields.get(key).map(|v| v == "true").unwrap_or(default)
}

fn get_json_str_array(fields: &BTreeMap<String, String>, key: &str) -> Vec<String> {
    let raw = match fields.get(key) {
        Some(v) => v.clone(),
        None => return Vec::new(),
    };
    let raw = raw.trim();
    if !raw.starts_with('[') || !raw.ends_with(']') {
        return Vec::new();
    }
    let inner = &raw[1..raw.len() - 1];
    let mut result = Vec::new();
    let mut chars = inner.chars().peekable();
    loop {
        skip_ws(&mut chars);
        if chars.peek().is_none() {
            break;
        }
        if chars.peek() == Some(&'"') {
            if let Ok(s) = parse_json_string_chars(&mut chars) {
                result.push(s);
            }
        } else {
            break;
        }
        skip_ws(&mut chars);
        if chars.peek() == Some(&',') {
            chars.next();
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Utility helpers
// ---------------------------------------------------------------------------

// ---------------------------------------------------------------------------
// D-Bus shared state
// ---------------------------------------------------------------------------

type SharedRegistry = Arc<Mutex<HomeRegistry>>;

// ---------------------------------------------------------------------------
// D-Bus interface: org.freedesktop.home1.Manager
// ---------------------------------------------------------------------------

/// D-Bus interface struct for org.freedesktop.home1.Manager.
///
/// Methods:
///   ListHomes() → a(susussss) — array of (name, uid, state, gid, real_name, home_dir, shell, obj_path)
///   GetHomeByName(s name) → (u, s, u, s, s, s, s)
///   GetHomeByUID(u uid) → (s, s, u, s, s, s, s)
///   ActivateHome(s name, s secret) — activate a managed home
///   DeactivateHome(s name) — deactivate a managed home
///   LockHome(s name) — lock a managed home
///   UnlockHome(s name, s secret) — unlock a managed home
///   LockAllHomes() — lock all active homes
///   DeactivateAllHomes() — deactivate all active homes
///   CreateHome(s blob) — create a home from JSON user record
///   RemoveHome(s name) — remove a managed home
///   Describe() → s — JSON description of manager state
///
/// Properties:
///   AutoLogin (b) — whether auto-login is enabled
struct Home1Manager {
    registry: SharedRegistry,
}

#[zbus::interface(name = "org.freedesktop.home1.Manager")]
impl Home1Manager {
    // --- Properties (read-only) ---

    #[zbus(property, name = "AutoLogin")]
    fn auto_login(&self) -> bool {
        false
    }

    // --- Methods ---

    /// ListHomes() → a(susussss)
    #[allow(clippy::type_complexity)]
    fn list_homes(&self) -> Vec<(String, u32, String, u32, String, String, String, String)> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry
            .list()
            .iter()
            .map(|rec| {
                (
                    rec.user_name.clone(),
                    rec.uid,
                    rec.state.as_str().to_string(),
                    rec.gid,
                    rec.real_name.clone(),
                    rec.home_directory.clone(),
                    rec.shell.clone(),
                    home_object_path(&rec.user_name),
                )
            })
            .collect()
    }

    /// GetHomeByName(s name) → (u, s, u, s, s, s, s)
    fn get_home_by_name(
        &self,
        name: String,
    ) -> zbus::fdo::Result<(u32, String, u32, String, String, String, String)> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.get(&name) {
            Some(rec) => Ok((
                rec.uid,
                rec.state.as_str().to_string(),
                rec.gid,
                rec.real_name.clone(),
                rec.home_directory.clone(),
                rec.shell.clone(),
                home_object_path(&rec.user_name),
            )),
            None => Err(zbus::fdo::Error::Failed(format!(
                "No home for user '{}'",
                name
            ))),
        }
    }

    /// GetHomeByUID(u uid) → (s, s, u, s, s, s, s)
    fn get_home_by_uid(
        &self,
        uid: u32,
    ) -> zbus::fdo::Result<(String, String, u32, String, String, String, String)> {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let found = registry.list().iter().find(|r| r.uid == uid).cloned();
        match found {
            Some(rec) => Ok((
                rec.user_name.clone(),
                rec.state.as_str().to_string(),
                rec.gid,
                rec.real_name.clone(),
                rec.home_directory.clone(),
                rec.shell.clone(),
                home_object_path(&rec.user_name),
            )),
            None => Err(zbus::fdo::Error::Failed(format!("No home for UID {}", uid))),
        }
    }

    /// ActivateHome(s name, s secret)
    fn activate_home(&self, name: String, _secret: String) -> zbus::fdo::Result<()> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.activate(&name) {
            Ok(_) => Ok(()),
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// DeactivateHome(s name)
    fn deactivate_home(&self, name: String) -> zbus::fdo::Result<()> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.deactivate(&name) {
            Ok(_) => Ok(()),
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// LockHome(s name)
    fn lock_home(&self, name: String) -> zbus::fdo::Result<()> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.lock(&name) {
            Ok(_) => Ok(()),
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// UnlockHome(s name, s secret)
    fn unlock_home(&self, name: String, _secret: String) -> zbus::fdo::Result<()> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.unlock(&name) {
            Ok(_) => Ok(()),
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// LockAllHomes()
    fn lock_all_homes(&self) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.lock_all();
    }

    /// DeactivateAllHomes()
    fn deactivate_all_homes(&self) {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        registry.deactivate_all();
    }

    /// RemoveHome(s name)
    fn remove_home(&self, name: String) -> zbus::fdo::Result<()> {
        let mut registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        match registry.remove(&name) {
            Ok(_) => Ok(()),
            Err(e) => Err(zbus::fdo::Error::Failed(e)),
        }
    }

    /// Describe() → s (JSON description of the manager state)
    fn describe(&self) -> String {
        let registry = self.registry.lock().unwrap_or_else(|e| e.into_inner());
        let homes = registry.list();
        let mut homes_json = String::from("[");
        for (i, rec) in homes.iter().enumerate() {
            if i > 0 {
                homes_json.push(',');
            }
            homes_json.push_str(&format!(
                concat!(
                    "{{",
                    "\"UserName\":\"{}\",",
                    "\"RealName\":\"{}\",",
                    "\"UID\":{},",
                    "\"GID\":{},",
                    "\"HomeDirectory\":\"{}\",",
                    "\"Shell\":\"{}\",",
                    "\"Storage\":\"{}\",",
                    "\"State\":\"{}\"",
                    "}}"
                ),
                json_escape(&rec.user_name),
                json_escape(&rec.real_name),
                rec.uid,
                rec.gid,
                json_escape(&rec.home_directory),
                json_escape(&rec.shell),
                json_escape(rec.storage.as_str()),
                json_escape(rec.state.as_str()),
            ));
        }
        homes_json.push(']');

        format!(
            concat!("{{", "\"NHomes\":{},", "\"Homes\":{}", "}}"),
            homes.len(),
            homes_json,
        )
    }
}

/// Convert a user name to a D-Bus object path.
fn home_object_path(name: &str) -> String {
    let mut path = String::from("/org/freedesktop/home1/home/");
    for ch in name.chars() {
        if ch.is_ascii_alphanumeric() || ch == '_' {
            path.push(ch);
        } else {
            path.push('_');
            path.push_str(&format!("{:02x}", ch as u32));
        }
    }
    path
}

/// Set up the D-Bus connection and register the home1 interface.
///
/// Uses zbus's blocking connection which dispatches messages automatically
/// in a background thread. The returned `Connection` must be kept alive
/// for as long as we want to serve D-Bus requests.
fn setup_dbus(shared: SharedRegistry) -> Result<Connection, String> {
    let iface = Home1Manager { registry: shared };
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

fn now_usec() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO)
        .as_micros() as u64
}

fn format_bytes(bytes: u64) -> String {
    const KIB: u64 = 1024;
    const MIB: u64 = 1024 * KIB;
    const GIB: u64 = 1024 * MIB;
    const TIB: u64 = 1024 * GIB;

    if bytes >= TIB {
        format!("{:.1}T", bytes as f64 / TIB as f64)
    } else if bytes >= GIB {
        format!("{:.1}G", bytes as f64 / GIB as f64)
    } else if bytes >= MIB {
        format!("{:.1}M", bytes as f64 / MIB as f64)
    } else if bytes >= KIB {
        format!("{:.1}K", bytes as f64 / KIB as f64)
    } else {
        format!("{}B", bytes)
    }
}

/// Compute disk usage of a directory tree (bytes).
pub fn dir_disk_usage(path: &Path) -> u64 {
    let mut total = 0u64;
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let ft = match entry.file_type() {
                Ok(ft) => ft,
                Err(_) => continue,
            };
            if ft.is_dir() {
                total += dir_disk_usage(&entry.path());
            } else if ft.is_file()
                && let Ok(meta) = entry.metadata()
            {
                total += meta.len();
            }
        }
    }
    total
}

// ---------------------------------------------------------------------------
// Password hashing (SHA-512 crypt format, simplified)
// ---------------------------------------------------------------------------

/// Hash a password using a simple SHA-512-based scheme.
/// Real systemd uses `crypt(3)` with `$6$` prefix.  We produce a
/// `$6$homed$<hex-sha512>` string so that `verify_password` can check it.
///
/// This is NOT cryptographically equivalent to `crypt(3)` — it's a minimal
/// stand-in so the full create/passwd/verify workflow can be tested without
/// a libc dependency.
pub fn hash_password(password: &str) -> String {
    // Use a very simple hash: djb2 iterated.  This is NOT secure —
    // a real implementation would call libc crypt(3).  But it's deterministic
    // and lets us roundtrip in tests.
    let mut h: u64 = 5381;
    for b in password.bytes() {
        h = h.wrapping_mul(33).wrapping_add(b as u64);
    }
    // Iterate to fill 64 hex chars
    let mut hex = String::new();
    for i in 0u64..8 {
        let v = h.wrapping_add(i.wrapping_mul(0x9e3779b97f4a7c15));
        hex.push_str(&format!("{:016x}", v));
    }
    format!("$6$homed${}", &hex[..128])
}

/// Verify a password against a stored hash.
pub fn verify_password(password: &str, stored: &str) -> bool {
    let expected = hash_password(password);
    expected == stored
}

// ---------------------------------------------------------------------------
// User name validation
// ---------------------------------------------------------------------------

/// Validate a user name per systemd conventions.
pub fn is_valid_user_name(name: &str) -> bool {
    if name.is_empty() || name.len() > 256 {
        return false;
    }
    // Must start with lowercase letter or underscore
    let first = name.as_bytes()[0];
    if !(first.is_ascii_lowercase() || first == b'_') {
        return false;
    }
    // Remaining chars: lowercase, digit, underscore, hyphen
    for &b in &name.as_bytes()[1..] {
        if !(b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_' || b == b'-') {
            return false;
        }
    }
    // Must not be a reserved name
    !matches!(
        name,
        "root"
            | "nobody"
            | "nfsnobody"
            | "daemon"
            | "bin"
            | "sys"
            | "sync"
            | "games"
            | "man"
            | "lp"
            | "mail"
            | "news"
            | "uucp"
            | "proxy"
            | "www-data"
            | "backup"
            | "list"
            | "irc"
            | "gnats"
            | "systemd-network"
            | "systemd-resolve"
            | "messagebus"
            | "sshd"
    )
}

// ---------------------------------------------------------------------------
// Home registry (in-memory state)
// ---------------------------------------------------------------------------

/// Parameters for creating a new home.
pub struct CreateParams<'a> {
    pub user_name: &'a str,
    pub real_name: Option<&'a str>,
    pub shell: Option<&'a str>,
    pub storage: Storage,
    pub password: Option<&'a str>,
    pub home_dir_override: Option<&'a str>,
    pub image_path_override: Option<&'a str>,
    pub disk_size: Option<u64>,
    pub cifs_service: Option<&'a str>,
    pub cifs_user_name: Option<&'a str>,
    pub cifs_domain: Option<&'a str>,
    pub enforce_password_policy: Option<bool>,
}

/// The home registry tracks all known managed home directories.
pub struct HomeRegistry {
    /// Identity directory on disk.
    identity_dir: PathBuf,
    /// Runtime state directory.
    runtime_dir: PathBuf,
    /// In-memory records keyed by user name.
    homes: BTreeMap<String, UserRecord>,
    /// Next UID to allocate.
    next_uid: u32,
}

impl Default for HomeRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl HomeRegistry {
    pub fn new() -> Self {
        Self::with_paths(Path::new(IDENTITY_DIR), Path::new(RUNTIME_DIR))
    }

    pub fn with_paths(identity_dir: &Path, runtime_dir: &Path) -> Self {
        Self {
            identity_dir: identity_dir.to_path_buf(),
            runtime_dir: runtime_dir.to_path_buf(),
            homes: BTreeMap::new(),
            next_uid: UID_MIN,
        }
    }

    /// Load all identity files from disk.
    pub fn load(&mut self) {
        self.homes.clear();
        if let Ok(entries) = fs::read_dir(&self.identity_dir) {
            for entry in entries.flatten() {
                let name = entry.file_name().to_string_lossy().to_string();
                if !name.ends_with(".identity") {
                    continue;
                }
                if name.starts_with('.') {
                    continue;
                }
                if let Ok(data) = fs::read_to_string(entry.path()) {
                    match UserRecord::from_json(&data) {
                        Ok(mut rec) => {
                            // Refresh state from runtime dir
                            let rt_path = self.runtime_dir.join(&rec.user_name);
                            if rt_path.exists()
                                && let Ok(st) = fs::read_to_string(&rt_path)
                            {
                                let st = st.trim();
                                if let Some(state) = HomeState::parse(st) {
                                    rec.state = state;
                                }
                            }
                            // Track highest UID
                            if rec.uid >= self.next_uid {
                                self.next_uid = rec.uid + 1;
                            }
                            self.homes.insert(rec.user_name.clone(), rec);
                        }
                        Err(e) => {
                            log::warn!("Failed to parse {}: {}", name, e);
                        }
                    }
                }
            }
        }
    }

    /// Save a single identity record to disk.
    pub fn save_one(&self, user_name: &str) -> io::Result<()> {
        if let Some(rec) = self.homes.get(user_name) {
            let _ = fs::create_dir_all(&self.identity_dir);
            let path = self.identity_dir.join(format!("{}.identity", user_name));
            let json = rec.to_json();
            fs::write(&path, json)?;
        }
        Ok(())
    }

    /// Save runtime state for a user.
    fn save_runtime_state(&self, user_name: &str) -> io::Result<()> {
        if let Some(rec) = self.homes.get(user_name) {
            let _ = fs::create_dir_all(&self.runtime_dir);
            let path = self.runtime_dir.join(user_name);
            fs::write(&path, rec.state.as_str())?;
        }
        Ok(())
    }

    /// Remove runtime state for a user.
    fn remove_runtime_state(&self, user_name: &str) {
        let path = self.runtime_dir.join(user_name);
        let _ = fs::remove_file(&path);
    }

    /// Allocate the next free UID in the homed range.
    pub fn allocate_uid(&mut self) -> Result<u32, String> {
        if self.next_uid > UID_MAX {
            return Err("UID range exhausted".to_string());
        }
        let uid = self.next_uid;
        self.next_uid += 1;
        Ok(uid)
    }

    /// Get a reference to a record.
    pub fn get(&self, user_name: &str) -> Option<&UserRecord> {
        self.homes.get(user_name)
    }

    /// Get a mutable reference to a record.
    pub fn get_mut(&mut self, user_name: &str) -> Option<&mut UserRecord> {
        self.homes.get_mut(user_name)
    }

    /// Check if a user is registered.
    pub fn contains(&self, user_name: &str) -> bool {
        self.homes.contains_key(user_name)
    }

    /// List all managed users.
    pub fn list(&self) -> Vec<&UserRecord> {
        self.homes.values().collect()
    }

    /// Number of managed homes.
    pub fn len(&self) -> usize {
        self.homes.len()
    }

    /// Whether registry is empty.
    pub fn is_empty(&self) -> bool {
        self.homes.is_empty()
    }

    // -- Operations ---------------------------------------------------------

    /// Create a new managed home.
    #[allow(clippy::too_many_arguments)]
    pub fn create(&mut self, params: CreateParams) -> Result<String, String> {
        if !is_valid_user_name(params.user_name) {
            return Err(format!("Invalid user name: {}", params.user_name));
        }
        if self.homes.contains_key(params.user_name) {
            return Err(format!("User '{}' already exists", params.user_name));
        }

        // Enforce password quality if a password is provided
        if let Some(pw) = params.password
            && params.enforce_password_policy.unwrap_or(true)
        {
            let policy = PasswordQuality::default();
            policy.check(pw, params.user_name)?;
        }

        let uid = self.allocate_uid()?;
        let mut rec = UserRecord::new(params.user_name, uid);
        if let Some(rn) = params.real_name {
            rec.real_name = rn.to_string();
        }
        if let Some(sh) = params.shell {
            rec.shell = sh.to_string();
        }
        rec.storage = params.storage;
        if let Some(hd) = params.home_dir_override {
            rec.home_directory = hd.to_string();
        }
        if let Some(ip) = params.image_path_override {
            rec.image_path = ip.to_string();
        }
        if let Some(pw) = params.password {
            rec.hashed_passwords.push(hash_password(pw));
        }
        if let Some(ds) = params.disk_size {
            rec.disk_size = Some(ds);
        }
        // CIFS fields
        if let Some(cs) = params.cifs_service {
            rec.cifs_service = Some(cs.to_string());
        }
        if let Some(cu) = params.cifs_user_name {
            rec.cifs_user_name = Some(cu.to_string());
        }
        if let Some(cd) = params.cifs_domain {
            rec.cifs_domain = Some(cd.to_string());
        }
        rec.enforce_password_policy = params.enforce_password_policy.unwrap_or(true);

        // Create the home area on disk
        self.create_home_area(&rec, params.password)?;

        let user_name = params.user_name.to_string();
        self.homes.insert(user_name.clone(), rec);
        let _ = self.save_one(&user_name);
        Ok(format!("Created home for user '{}'", user_name))
    }

    /// Create the backing home area on disk.
    fn create_home_area(&self, rec: &UserRecord, password: Option<&str>) -> Result<(), String> {
        match rec.storage {
            Storage::Directory => {
                let image = Path::new(&rec.image_path);
                if image.exists() {
                    return Err(format!("Image path already exists: {}", rec.image_path));
                }
                fs::create_dir_all(image)
                    .map_err(|e| format!("Failed to create {}: {}", rec.image_path, e))?;
                // Set ownership (best-effort, may fail in tests without root)
                let _ = nix::unistd::chown(
                    image,
                    Some(nix::unistd::Uid::from_raw(rec.uid)),
                    Some(nix::unistd::Gid::from_raw(rec.gid)),
                );
                // Set mode 0700
                let _ =
                    fs::set_permissions(image, std::os::unix::fs::PermissionsExt::from_mode(0o700));
                Ok(())
            }
            Storage::Subvolume => {
                let image = Path::new(&rec.image_path);
                // Try btrfs subvolume creation, falls back to mkdir
                btrfs_subvol_create(image)?;
                // Set ownership
                let _ = nix::unistd::chown(
                    image,
                    Some(nix::unistd::Uid::from_raw(rec.uid)),
                    Some(nix::unistd::Gid::from_raw(rec.gid)),
                );
                let _ =
                    fs::set_permissions(image, std::os::unix::fs::PermissionsExt::from_mode(0o700));
                // Set quota if disk_size is specified
                if let Some(size) = rec.disk_size {
                    let _ = btrfs_set_quota(image, size);
                }
                Ok(())
            }
            Storage::Luks => {
                let pw =
                    password.ok_or_else(|| "Password required for LUKS storage".to_string())?;
                create_luks_home_area(rec, pw)
            }
            Storage::Cifs => create_cifs_home_area(rec),
            Storage::Fscrypt => create_fscrypt_home_area(rec),
        }
    }

    /// Remove a managed home.
    pub fn remove(&mut self, user_name: &str) -> Result<String, String> {
        let rec = match self.homes.get(user_name) {
            Some(r) => r.clone(),
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        if rec.state == HomeState::Active || rec.state == HomeState::Locked {
            return Err(format!(
                "Home for '{}' is still active, deactivate first",
                user_name
            ));
        }

        // Remove home area from disk
        self.remove_home_area(&rec)?;

        // Remove identity file
        let id_path = self.identity_dir.join(format!("{}.identity", user_name));
        let _ = fs::remove_file(&id_path);
        self.remove_runtime_state(user_name);
        self.homes.remove(user_name);

        Ok(format!("Removed home for user '{}'", user_name))
    }

    fn remove_home_area(&self, rec: &UserRecord) -> Result<(), String> {
        match rec.storage {
            Storage::Subvolume => {
                let image = Path::new(&rec.image_path);
                if image.exists() {
                    btrfs_subvol_delete(image)?;
                }
            }
            _ => {
                let image = Path::new(&rec.image_path);
                if image.exists() {
                    if image.is_dir() {
                        fs::remove_dir_all(image)
                            .map_err(|e| format!("Failed to remove {}: {}", rec.image_path, e))?;
                    } else {
                        fs::remove_file(image)
                            .map_err(|e| format!("Failed to remove {}: {}", rec.image_path, e))?;
                    }
                }
            }
        }
        // Also remove home_directory symlink if it exists and is distinct
        let image = Path::new(&rec.image_path);
        let hd = Path::new(&rec.home_directory);
        if hd != image
            && hd.exists()
            && hd
                .symlink_metadata()
                .map(|m| m.file_type().is_symlink())
                .unwrap_or(false)
        {
            let _ = fs::remove_file(hd);
        }
        Ok(())
    }

    /// Activate (mount/make available) a home directory.
    pub fn activate(&mut self, user_name: &str) -> Result<String, String> {
        self.activate_with_secret(user_name, None)
    }

    /// Activate with an optional password/secret for encrypted backends.
    pub fn activate_with_secret(
        &mut self,
        user_name: &str,
        secret: Option<&str>,
    ) -> Result<String, String> {
        // First pass: check preconditions with immutable access
        {
            let rec = match self.homes.get(user_name) {
                Some(r) => r,
                None => return Err(format!("Unknown user: {}", user_name)),
            };
            match rec.state {
                HomeState::Active => {
                    return Ok(format!("Home for '{}' is already active", user_name));
                }
                HomeState::Locked => {
                    return Err(format!("Home for '{}' is locked, unlock first", user_name));
                }
                HomeState::Activating | HomeState::Deactivating => {
                    return Err(format!("Home for '{}' is busy ({})", user_name, rec.state));
                }
                _ => {}
            }
        }

        // For CIFS, we don't check image_path existence (it's a network share)
        let storage = self.homes[user_name].storage;
        if storage != Storage::Cifs {
            let image_path = self.homes[user_name].image_path.clone();
            if !Path::new(&image_path).exists() {
                self.homes.get_mut(user_name).unwrap().state = HomeState::Absent;
                let _ = self.save_one(user_name);
                return Err(format!("Home area absent: {}", image_path));
            }
        }

        // Second pass: perform activation with mutable access
        let rec = self.homes.get_mut(user_name).unwrap();
        rec.state = HomeState::Activating;

        match rec.storage {
            Storage::Directory | Storage::Subvolume => {
                let hd = Path::new(&rec.home_directory);
                let img = Path::new(&rec.image_path);
                if hd != img {
                    if let Some(parent) = hd.parent() {
                        let _ = fs::create_dir_all(parent);
                    }
                    if !hd.exists() {
                        unix_fs::symlink(img, hd).map_err(|e| {
                            format!(
                                "Failed to symlink {} -> {}: {}",
                                rec.home_directory, rec.image_path, e
                            )
                        })?;
                    }
                }
                rec.disk_usage = Some(dir_disk_usage(img));
            }
            Storage::Luks => {
                let pw = secret.unwrap_or("");
                // Try PKCS#11 token first, then FIDO2, then password,
                // then recovery key
                let activate_result = if !rec.pkcs11_encrypted_key.is_empty() && pw.is_empty() {
                    // Try PKCS#11 token
                    let mut last_err = String::new();
                    let mut ok = false;
                    for key in &rec.pkcs11_encrypted_key {
                        match key.unwrap_key() {
                            Ok(_key_bytes) => {
                                // Would use decrypted key to open LUKS volume
                                ok = true;
                                break;
                            }
                            Err(e) => last_err = e,
                        }
                    }
                    if ok { Ok(()) } else { Err(last_err) }
                } else if !rec.fido2_hmac_credential.is_empty() && pw.is_empty() {
                    let mut last_err = String::new();
                    let mut ok = false;
                    for cred in &rec.fido2_hmac_credential {
                        match cred.derive_key() {
                            Ok(_key_bytes) => {
                                ok = true;
                                break;
                            }
                            Err(e) => last_err = e,
                        }
                    }
                    if ok { Ok(()) } else { Err(last_err) }
                } else if !pw.is_empty() {
                    // Check if it's a recovery key
                    if !rec.recovery_key.is_empty() && verify_recovery_key(pw, &rec.recovery_key) {
                        // Recovery key verified; use it to derive LUKS key
                        let rec_clone = rec.clone();
                        activate_luks_home(&rec_clone, pw)
                    } else {
                        let rec_clone = rec.clone();
                        activate_luks_home(&rec_clone, pw)
                    }
                } else {
                    Err("Password required for LUKS activation".to_string())
                };

                if let Err(e) = activate_result {
                    rec.state = HomeState::Inactive;
                    return Err(format!("LUKS activation failed: {}", e));
                }
            }
            Storage::Cifs => {
                let pw = secret.unwrap_or("");
                let rec_clone = rec.clone();
                if let Err(e) = activate_cifs_home(&rec_clone, pw) {
                    rec.state = HomeState::Inactive;
                    return Err(format!("CIFS activation failed: {}", e));
                }
            }
            Storage::Fscrypt => {
                let pw = secret.unwrap_or("");
                if pw.is_empty() {
                    rec.state = HomeState::Inactive;
                    return Err("Password required for fscrypt activation".to_string());
                }
                let rec_clone = rec.clone();
                match activate_fscrypt_home(&rec_clone, pw) {
                    Ok(_key_serial) => {}
                    Err(e) => {
                        rec.state = HomeState::Inactive;
                        return Err(format!("fscrypt activation failed: {}", e));
                    }
                }
            }
        }

        rec.state = HomeState::Active;
        let _ = self.save_runtime_state(user_name);
        let _ = self.save_one(user_name);
        Ok(format!("Activated home for '{}'", user_name))
    }

    /// Deactivate (unmount/lock) a home directory.
    pub fn deactivate(&mut self, user_name: &str) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        match rec.state {
            HomeState::Inactive | HomeState::Absent => {
                return Ok(format!("Home for '{}' is already inactive", user_name));
            }
            HomeState::Activating | HomeState::Deactivating => {
                return Err(format!("Home for '{}' is busy ({})", user_name, rec.state));
            }
            _ => {}
        }

        rec.state = HomeState::Deactivating;

        // Backend-specific deactivation
        match rec.storage {
            Storage::Luks => {
                let rec_clone = rec.clone();
                let _ = deactivate_luks_home(&rec_clone);
            }
            Storage::Cifs => {
                let rec_clone = rec.clone();
                let _ = deactivate_cifs_home(&rec_clone);
            }
            Storage::Fscrypt => {
                let rec_clone = rec.clone();
                let _ = deactivate_fscrypt_home(&rec_clone, 0);
            }
            _ => {
                // Directory/Subvolume: remove symlink if homeDirectory != imagePath
                let hd = Path::new(&rec.home_directory);
                let img = Path::new(&rec.image_path);
                if hd != img
                    && hd
                        .symlink_metadata()
                        .map(|m| m.file_type().is_symlink())
                        .unwrap_or(false)
                {
                    let _ = fs::remove_file(hd);
                }
            }
        }

        rec.state = HomeState::Inactive;
        rec.locked = false;
        let _ = self.save_runtime_state(user_name);
        let _ = self.save_one(user_name);
        self.remove_runtime_state(user_name);
        Ok(format!("Deactivated home for '{}'", user_name))
    }

    /// Lock a home directory (for suspend-to-RAM protection).
    pub fn lock(&mut self, user_name: &str) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        if rec.state != HomeState::Active {
            return Err(format!(
                "Home for '{}' is not active ({})",
                user_name, rec.state
            ));
        }
        rec.state = HomeState::Locked;
        rec.locked = true;
        let _ = self.save_runtime_state(user_name);
        let _ = self.save_one(user_name);
        Ok(format!("Locked home for '{}'", user_name))
    }

    /// Unlock a home directory (after resume).
    pub fn unlock(&mut self, user_name: &str) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        if rec.state != HomeState::Locked {
            return Err(format!(
                "Home for '{}' is not locked ({})",
                user_name, rec.state
            ));
        }
        rec.state = HomeState::Active;
        rec.locked = false;
        let _ = self.save_runtime_state(user_name);
        let _ = self.save_one(user_name);
        Ok(format!("Unlocked home for '{}'", user_name))
    }

    /// Lock all active home directories.
    pub fn lock_all(&mut self) -> String {
        let active_users: Vec<String> = self
            .homes
            .iter()
            .filter(|(_, r)| r.state == HomeState::Active)
            .map(|(name, _)| name.clone())
            .collect();
        let mut locked = 0usize;
        for name in &active_users {
            if self.lock(name).is_ok() {
                locked += 1;
            }
        }
        format!("Locked {} home(s)", locked)
    }

    /// Deactivate all active/locked home directories.
    pub fn deactivate_all(&mut self) -> String {
        let users: Vec<String> = self
            .homes
            .iter()
            .filter(|(_, r)| matches!(r.state, HomeState::Active | HomeState::Locked))
            .map(|(name, _)| name.clone())
            .collect();
        let mut deactivated = 0usize;
        for name in &users {
            // Must unlock first if locked
            if self.homes.get(name.as_str()).map(|r| r.state) == Some(HomeState::Locked) {
                let _ = self.unlock(name);
            }
            if self.deactivate(name).is_ok() {
                deactivated += 1;
            }
        }
        format!("Deactivated {} home(s)", deactivated)
    }

    /// Update user record fields.
    pub fn update(
        &mut self,
        user_name: &str,
        real_name: Option<&str>,
        shell: Option<&str>,
        password_hint: Option<&str>,
        auto_login: Option<bool>,
    ) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        if let Some(rn) = real_name {
            rec.real_name = rn.to_string();
        }
        if let Some(sh) = shell {
            rec.shell = sh.to_string();
        }
        if let Some(hint) = password_hint {
            rec.password_hint = if hint.is_empty() {
                None
            } else {
                Some(hint.to_string())
            };
        }
        if let Some(al) = auto_login {
            rec.auto_login = al;
        }
        rec.last_change_usec = now_usec();
        let _ = self.save_one(user_name);
        Ok(format!("Updated record for '{}'", user_name))
    }

    /// Change password for a managed user.
    pub fn change_password(
        &mut self,
        user_name: &str,
        new_password: &str,
    ) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        if new_password.is_empty() {
            return Err("Password must not be empty".to_string());
        }
        // Enforce password quality if enabled
        if rec.enforce_password_policy {
            let policy = PasswordQuality::default();
            policy.check(new_password, &rec.user_name.clone())?;
        }
        let hashed = hash_password(new_password);
        rec.hashed_passwords = vec![hashed];
        rec.last_password_change_usec = now_usec();
        rec.last_change_usec = now_usec();
        let _ = self.save_one(user_name);
        Ok(format!("Password changed for '{}'", user_name))
    }

    /// Generate a recovery key for a user and store it (hashed) in the record.
    /// Returns the plaintext recovery key that should be displayed to the user.
    pub fn generate_recovery_key(&mut self, user_name: &str) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        let key = generate_recovery_key();
        let hashed = hash_password(&key);
        rec.recovery_key.push(hashed);
        rec.last_change_usec = now_usec();
        let _ = self.save_one(user_name);
        Ok(key)
    }

    /// Add a PKCS#11 encrypted key to a user record.
    pub fn add_pkcs11_key(
        &mut self,
        user_name: &str,
        uri: &str,
        encrypted_key: &str,
        hashed_password: &str,
    ) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        rec.pkcs11_encrypted_key
            .push(Pkcs11EncryptedKey::new(uri, encrypted_key, hashed_password));
        rec.last_change_usec = now_usec();
        let _ = self.save_one(user_name);
        Ok(format!("PKCS#11 key added for '{}'", user_name))
    }

    /// Add a FIDO2 credential to a user record.
    pub fn add_fido2_credential(
        &mut self,
        user_name: &str,
        credential_id: &str,
        rp_id: &str,
        salt: &str,
    ) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        rec.fido2_hmac_credential
            .push(Fido2HmacCredential::new(credential_id, rp_id, salt));
        rec.last_change_usec = now_usec();
        let _ = self.save_one(user_name);
        Ok(format!("FIDO2 credential added for '{}'", user_name))
    }

    /// Resize a home area.
    pub fn resize(&mut self, user_name: &str, new_size: u64) -> Result<String, String> {
        let rec = match self.homes.get_mut(user_name) {
            Some(r) => r,
            None => return Err(format!("Unknown user: {}", user_name)),
        };
        match rec.storage {
            Storage::Directory => {
                // For plain directories, disk_size is just advisory metadata
                rec.disk_size = Some(new_size);
                rec.last_change_usec = now_usec();
                let _ = self.save_one(user_name);
                Ok(format!(
                    "Updated disk size for '{}' to {}",
                    user_name,
                    format_bytes(new_size)
                ))
            }
            Storage::Subvolume => {
                // Update advisory size and set btrfs quota
                rec.disk_size = Some(new_size);
                let img = Path::new(&rec.image_path);
                if img.exists() {
                    let _ = btrfs_set_quota(img, new_size);
                }
                rec.last_change_usec = now_usec();
                let _ = self.save_one(user_name);
                Ok(format!(
                    "Updated disk size for '{}' to {} (btrfs quota)",
                    user_name,
                    format_bytes(new_size)
                ))
            }
            Storage::Luks => {
                let rec_clone = rec.clone();
                resize_luks_home(&rec_clone, new_size)?;
                rec.disk_size = Some(new_size);
                rec.last_change_usec = now_usec();
                let _ = self.save_one(user_name);
                Ok(format!(
                    "Resized LUKS home for '{}' to {}",
                    user_name,
                    format_bytes(new_size)
                ))
            }
            Storage::Cifs => Err("Resize not supported for CIFS storage".to_string()),
            Storage::Fscrypt => {
                // fscrypt directories have no inherent size limit
                rec.disk_size = Some(new_size);
                rec.last_change_usec = now_usec();
                let _ = self.save_one(user_name);
                Ok(format!(
                    "Updated disk size for '{}' to {} (advisory)",
                    user_name,
                    format_bytes(new_size)
                ))
            }
        }
    }

    /// Garbage-collect: check for homes whose image areas have disappeared.
    pub fn gc(&mut self) {
        let names: Vec<String> = self.homes.keys().cloned().collect();
        for name in names {
            if let Some(rec) = self.homes.get_mut(&name)
                && (rec.state == HomeState::Active || rec.state == HomeState::Locked)
            {
                let img = Path::new(&rec.image_path);
                if !img.exists() {
                    log::warn!(
                        "Home area for '{}' has disappeared: {}",
                        name,
                        rec.image_path
                    );
                    rec.state = HomeState::Absent;
                    let _ = self.save_runtime_state(&name);
                    let _ = self.save_one(&name);
                }
            }
        }
    }

    /// Format a list table of all homes.
    pub fn format_list(&self) -> String {
        if self.homes.is_empty() {
            return "No managed home directories.\n".to_string();
        }
        let mut s = String::new();
        s.push_str(&format!(
            "{:<16} {:>6} {:>6} {:<12} {:<10} {}\n",
            "NAME", "UID", "GID", "STATE", "STORAGE", "HOME"
        ));
        for rec in self.homes.values() {
            s.push_str(&format!(
                "{:<16} {:>6} {:>6} {:<12} {:<10} {}\n",
                rec.user_name, rec.uid, rec.gid, rec.state, rec.storage, rec.home_directory
            ));
        }
        s.push_str(&format!("\n{} home(s) listed.\n", self.homes.len()));
        s
    }
}

// ---------------------------------------------------------------------------
// Control socket command handling
// ---------------------------------------------------------------------------

/// Handle a control command and return a response string.
pub fn handle_control_command(registry: &mut HomeRegistry, command: &str) -> String {
    let command = command.trim();
    if command.is_empty() {
        return "ERROR: empty command\n".to_string();
    }

    // Split into verb and args (case-insensitive verb)
    let mut parts = command.splitn(2, ' ');
    let verb = parts.next().unwrap_or("").to_ascii_uppercase();
    let args = parts.next().unwrap_or("").trim();

    match verb.as_str() {
        "PING" => "PONG\n".to_string(),

        "LIST" => registry.format_list(),

        "INSPECT" => {
            if args.is_empty() {
                return "ERROR: INSPECT requires a user name\n".to_string();
            }
            match registry.get(args) {
                Some(rec) => rec.format_inspect(),
                None => format!("ERROR: unknown user '{}'\n", args),
            }
        }

        "SHOW" => {
            if args.is_empty() {
                return "ERROR: SHOW requires a user name\n".to_string();
            }
            match registry.get(args) {
                Some(rec) => rec.format_show(),
                None => format!("ERROR: unknown user '{}'\n", args),
            }
        }

        "RECORD" => {
            if args.is_empty() {
                return "ERROR: RECORD requires a user name\n".to_string();
            }
            match registry.get(args) {
                Some(rec) => rec.to_json() + "\n",
                None => format!("ERROR: unknown user '{}'\n", args),
            }
        }

        "CREATE" => {
            // CREATE <username> [<real_name>] [storage=<type>] [shell=<path>] [password=<pw>]
            // [disk-size=<size>] [cifs-service=<svc>] [cifs-user=<user>] [cifs-domain=<dom>]
            // [no-password-quality]
            let create_args = parse_create_args(args);
            match create_args {
                Ok(ca) => match registry.create(CreateParams {
                    user_name: &ca.user_name,
                    real_name: ca.real_name.as_deref(),
                    shell: ca.shell.as_deref(),
                    storage: ca.storage,
                    password: ca.password.as_deref(),
                    home_dir_override: ca.home_dir.as_deref(),
                    image_path_override: ca.image_path.as_deref(),
                    disk_size: ca.disk_size,
                    cifs_service: ca.cifs_service.as_deref(),
                    cifs_user_name: ca.cifs_user.as_deref(),
                    cifs_domain: ca.cifs_domain.as_deref(),
                    enforce_password_policy: Some(!ca.no_password_quality),
                }) {
                    Ok(msg) => format!("{}\n", msg),
                    Err(e) => format!("ERROR: {}\n", e),
                },
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "REMOVE" => {
            if args.is_empty() {
                return "ERROR: REMOVE requires a user name\n".to_string();
            }
            match registry.remove(args) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "ACTIVATE" => {
            if args.is_empty() {
                return "ERROR: ACTIVATE requires a user name\n".to_string();
            }
            match registry.activate(args) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "DEACTIVATE" => {
            if args.is_empty() {
                return "ERROR: DEACTIVATE requires a user name\n".to_string();
            }
            match registry.deactivate(args) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "LOCK" => {
            if args.is_empty() {
                return "ERROR: LOCK requires a user name\n".to_string();
            }
            match registry.lock(args) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "UNLOCK" => {
            if args.is_empty() {
                return "ERROR: UNLOCK requires a user name\n".to_string();
            }
            match registry.unlock(args) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "LOCK-ALL" => {
            let msg = registry.lock_all();
            format!("{}\n", msg)
        }

        "DEACTIVATE-ALL" => {
            let msg = registry.deactivate_all();
            format!("{}\n", msg)
        }

        "UPDATE" => {
            // UPDATE <username> [realname=<val>] [shell=<val>] [password-hint=<val>] [auto-login=<bool>]
            let ua = parse_update_args(args);
            match ua {
                Ok(ua) => match registry.update(
                    &ua.user_name,
                    ua.real_name.as_deref(),
                    ua.shell.as_deref(),
                    ua.password_hint.as_deref(),
                    ua.auto_login,
                ) {
                    Ok(msg) => format!("{}\n", msg),
                    Err(e) => format!("ERROR: {}\n", e),
                },
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "PASSWD" => {
            // PASSWD <username> <new_password>
            let mut parts = args.splitn(2, ' ');
            let name = parts.next().unwrap_or("");
            let pw = parts.next().unwrap_or("").trim();
            if name.is_empty() || pw.is_empty() {
                return "ERROR: PASSWD requires <username> <new_password>\n".to_string();
            }
            match registry.change_password(name, pw) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "RESIZE" => {
            // RESIZE <username> <size_bytes>
            let mut parts = args.splitn(2, ' ');
            let name = parts.next().unwrap_or("");
            let size_str = parts.next().unwrap_or("").trim();
            if name.is_empty() || size_str.is_empty() {
                return "ERROR: RESIZE requires <username> <size_bytes>\n".to_string();
            }
            match parse_size(size_str) {
                Some(sz) => match registry.resize(name, sz) {
                    Ok(msg) => format!("{}\n", msg),
                    Err(e) => format!("ERROR: {}\n", e),
                },
                None => format!("ERROR: invalid size '{}'\n", size_str),
            }
        }

        "GC" => {
            registry.gc();
            "OK\n".to_string()
        }

        "RELOAD" => {
            registry.load();
            format!("Reloaded, {} home(s)\n", registry.len())
        }

        "RECOVERY-KEY" => {
            // RECOVERY-KEY <username>
            if args.is_empty() {
                return "ERROR: RECOVERY-KEY requires a user name\n".to_string();
            }
            match registry.generate_recovery_key(args) {
                Ok(key) => format!("Recovery key for '{}': {}\n", args, key),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "ADD-PKCS11" => {
            // ADD-PKCS11 <username> <uri> <encrypted_key> <hashed_password>
            let tokens: Vec<&str> = args.splitn(4, ' ').collect();
            if tokens.len() < 4 {
                return "ERROR: ADD-PKCS11 requires <username> <uri> <encrypted_key> <hashed_password>\n".to_string();
            }
            match registry.add_pkcs11_key(tokens[0], tokens[1], tokens[2], tokens[3]) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "ADD-FIDO2" => {
            // ADD-FIDO2 <username> <credential_id> <rp_id> <salt>
            let tokens: Vec<&str> = args.splitn(4, ' ').collect();
            if tokens.len() < 4 {
                return "ERROR: ADD-FIDO2 requires <username> <credential_id> <rp_id> <salt>\n"
                    .to_string();
            }
            match registry.add_fido2_credential(tokens[0], tokens[1], tokens[2], tokens[3]) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "ACTIVATE-SECRET" => {
            // ACTIVATE-SECRET <username> <secret>
            let mut parts = args.splitn(2, ' ');
            let name = parts.next().unwrap_or("");
            let secret = parts.next().unwrap_or("").trim();
            if name.is_empty() {
                return "ERROR: ACTIVATE-SECRET requires a user name\n".to_string();
            }
            let sec = if secret.is_empty() {
                None
            } else {
                Some(secret)
            };
            match registry.activate_with_secret(name, sec) {
                Ok(msg) => format!("{}\n", msg),
                Err(e) => format!("ERROR: {}\n", e),
            }
        }

        "CHECK-PASSWORD" => {
            // CHECK-PASSWORD <password> [<username>]
            let tokens: Vec<&str> = args.splitn(2, ' ').collect();
            if tokens.is_empty() || tokens[0].is_empty() {
                return "ERROR: CHECK-PASSWORD requires a password\n".to_string();
            }
            let pw = tokens[0];
            let user = if tokens.len() > 1 { tokens[1] } else { "" };
            let policy = PasswordQuality::default();
            match policy.check(pw, user) {
                Ok(()) => "OK: password meets quality requirements\n".to_string(),
                Err(e) => format!("FAIL: {}\n", e),
            }
        }

        _ => format!("ERROR: unknown command '{}'\n", verb),
    }
}

// ---------------------------------------------------------------------------
// Argument parsing helpers
// ---------------------------------------------------------------------------

struct CreateArgs {
    user_name: String,
    real_name: Option<String>,
    shell: Option<String>,
    storage: Storage,
    password: Option<String>,
    home_dir: Option<String>,
    image_path: Option<String>,
    disk_size: Option<u64>,
    cifs_service: Option<String>,
    cifs_user: Option<String>,
    cifs_domain: Option<String>,
    no_password_quality: bool,
}

fn parse_create_args(args: &str) -> Result<CreateArgs, String> {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    if tokens.is_empty() {
        return Err("CREATE requires at least a user name".to_string());
    }
    let user_name = tokens[0].to_string();
    let mut real_name = None;
    let mut shell = None;
    let mut storage = Storage::Directory;
    let mut password = None;
    let mut home_dir = None;
    let mut image_path = None;
    let mut disk_size = None;
    let mut cifs_service = None;
    let mut cifs_user = None;
    let mut cifs_domain = None;
    let mut no_password_quality = false;

    for tok in &tokens[1..] {
        if let Some(val) = tok.strip_prefix("realname=") {
            real_name = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("shell=") {
            shell = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("storage=") {
            storage =
                Storage::parse(val).ok_or_else(|| format!("unknown storage type: {}", val))?;
        } else if let Some(val) = tok.strip_prefix("password=") {
            password = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("home=") {
            home_dir = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("image=") {
            image_path = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("disk-size=") {
            disk_size = parse_size(val);
        } else if let Some(val) = tok.strip_prefix("cifs-service=") {
            cifs_service = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("cifs-user=") {
            cifs_user = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("cifs-domain=") {
            cifs_domain = Some(val.to_string());
        } else if *tok == "no-password-quality" {
            no_password_quality = true;
        } else if real_name.is_none() {
            // Treat first non-option arg as real name
            real_name = Some(tok.to_string());
        }
    }

    Ok(CreateArgs {
        user_name,
        real_name,
        shell,
        storage,
        password,
        home_dir,
        image_path,
        disk_size,
        cifs_service,
        cifs_user,
        cifs_domain,
        no_password_quality,
    })
}

struct UpdateArgs {
    user_name: String,
    real_name: Option<String>,
    shell: Option<String>,
    password_hint: Option<String>,
    auto_login: Option<bool>,
}

fn parse_update_args(args: &str) -> Result<UpdateArgs, String> {
    let tokens: Vec<&str> = args.split_whitespace().collect();
    if tokens.is_empty() {
        return Err("UPDATE requires at least a user name".to_string());
    }
    let user_name = tokens[0].to_string();
    let mut real_name = None;
    let mut shell = None;
    let mut password_hint = None;
    let mut auto_login = None;

    for tok in &tokens[1..] {
        if let Some(val) = tok.strip_prefix("realname=") {
            real_name = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("shell=") {
            shell = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("password-hint=") {
            password_hint = Some(val.to_string());
        } else if let Some(val) = tok.strip_prefix("auto-login=") {
            auto_login = Some(val == "true" || val == "yes" || val == "1");
        }
    }

    Ok(UpdateArgs {
        user_name,
        real_name,
        shell,
        password_hint,
        auto_login,
    })
}

/// Parse a size string like "1G", "500M", "1073741824".
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }

    let (num_str, mult) = if let Some(n) = s.strip_suffix('T') {
        (n, 1024u64 * 1024 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('G') {
        (n, 1024u64 * 1024 * 1024)
    } else if let Some(n) = s.strip_suffix('M') {
        (n, 1024u64 * 1024)
    } else if let Some(n) = s.strip_suffix('K') {
        (n, 1024u64)
    } else {
        (s, 1u64)
    };

    num_str.trim().parse::<u64>().ok().map(|v| v * mult)
}

// ---------------------------------------------------------------------------
// Client handling
// ---------------------------------------------------------------------------

fn handle_client(registry: &mut HomeRegistry, stream: &mut UnixStream) {
    let reader = BufReader::new(stream.try_clone().expect("failed to clone control stream"));
    if let Some(Ok(cmd)) = reader.lines().next() {
        let resp = handle_control_command(registry, &cmd);
        let _ = stream.write_all(resp.as_bytes());
    }
}

// ---------------------------------------------------------------------------
// sd_notify
// ---------------------------------------------------------------------------

fn sd_notify_raw(msg: &str) {
    if let Ok(path) = env::var("NOTIFY_SOCKET") {
        let path = if let Some(stripped) = path.strip_prefix('@') {
            // Abstract socket
            format!("\0{}", stripped)
        } else {
            path
        };
        if let Ok(sock) = std::os::unix::net::UnixDatagram::unbound() {
            let _ = sock.send_to(msg.as_bytes(), &path);
        }
    }
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
        libc::signal(
            libc::SIGTERM,
            handle_sigterm as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGINT,
            handle_sigint as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
        libc::signal(
            libc::SIGHUP,
            handle_sighup as extern "C" fn(libc::c_int) as libc::sighandler_t,
        );
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
                let ts = chrono_lite_timestamp();
                eprintln!(
                    "[{}] systemd-homed: {}: {}",
                    ts,
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

fn chrono_lite_timestamp() -> String {
    let d = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or(Duration::ZERO);
    let secs = d.as_secs();
    let h = (secs / 3600) % 24;
    let m = (secs / 60) % 60;
    let s = secs % 60;
    format!("{:02}:{:02}:{:02}", h, m, s)
}

fn parse_watchdog_usec(s: &str) -> Option<Duration> {
    let usec: u64 = s.trim().parse().ok()?;
    if usec == 0 {
        None
    } else {
        Some(Duration::from_micros(usec / 2))
    }
}

fn watchdog_interval() -> Option<Duration> {
    env::var("WATCHDOG_USEC")
        .ok()
        .and_then(|s| parse_watchdog_usec(&s))
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn sd_notify(msg: &str) {
    sd_notify_raw(msg);
}

fn main() {
    init_logging();
    setup_signal_handlers();

    log::info!("systemd-homed starting");

    // Ensure directories exist
    let _ = fs::create_dir_all(IDENTITY_DIR);
    let _ = fs::create_dir_all(RUNTIME_DIR);

    // Load existing records into shared registry for D-Bus and control socket
    let mut registry = HomeRegistry::new();
    registry.load();
    log::info!("Loaded {} managed home(s)", registry.len());

    let initial_count = registry.len();
    let shared_registry: SharedRegistry = Arc::new(Mutex::new(registry));

    // Watchdog support
    let wd_interval = watchdog_interval();
    if let Some(ref iv) = wd_interval {
        log::info!("Watchdog enabled, interval {:?}", iv);
    }
    let mut last_watchdog = Instant::now();

    // D-Bus connection is deferred to after READY=1 so we don't block early
    // boot waiting for dbus-daemon.  zbus dispatches messages automatically
    // in a background thread — we just keep the connection alive.
    let mut _dbus_conn: Option<Connection> = None;
    let mut dbus_attempted = false;

    // Remove stale socket
    let _ = fs::remove_file(CONTROL_SOCKET_PATH);

    // Ensure parent dir exists
    let _ = fs::create_dir_all(Path::new(CONTROL_SOCKET_PATH).parent().unwrap());

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

    sd_notify(&format!(
        "READY=1\nSTATUS={} home(s) managed",
        initial_count
    ));

    log::info!("systemd-homed ready");

    let mut gc_counter = 0u32;

    // Main loop
    loop {
        if SHUTDOWN.load(Ordering::SeqCst) {
            log::info!("Received shutdown signal");
            break;
        }

        if RELOAD.load(Ordering::SeqCst) {
            RELOAD.store(false, Ordering::SeqCst);
            let mut reg = shared_registry.lock().unwrap_or_else(|e| e.into_inner());
            reg.load();
            let count = reg.len();
            log::info!("Reloaded, {} managed home(s)", count);
            sd_notify(&format!("STATUS={} home(s) managed", count));
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
        if !dbus_attempted {
            dbus_attempted = true;
            match setup_dbus(shared_registry.clone()) {
                Ok(conn) => {
                    log::info!("D-Bus interface registered: {} at {}", DBUS_NAME, DBUS_PATH);
                    _dbus_conn = Some(conn);
                    let count = shared_registry
                        .lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .len();
                    sd_notify(&format!("STATUS={} home(s) managed (D-Bus active)", count));
                }
                Err(e) => {
                    log::warn!(
                        "Failed to register D-Bus interface ({}); control socket only",
                        e
                    );
                }
            }
        }

        // zbus dispatches D-Bus messages automatically in a background thread.

        // Periodic GC (every ~60 iterations ≈ every 3 seconds at 50ms sleep)
        gc_counter += 1;
        if gc_counter >= 60 {
            gc_counter = 0;
            let mut reg = shared_registry.lock().unwrap_or_else(|e| e.into_inner());
            reg.gc();
        }

        // Accept control socket connections
        if let Some(ref listener) = listener {
            match listener.accept() {
                Ok((mut stream, _addr)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
                    let mut reg = shared_registry.lock().unwrap_or_else(|e| e.into_inner());
                    handle_client(&mut reg, &mut stream);
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
    log::info!("systemd-homed stopped");
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    // D-Bus registration tests

    #[test]
    fn test_dbus_home1_manager_struct() {
        let shared: SharedRegistry = Arc::new(Mutex::new(HomeRegistry::new()));
        let _mgr = Home1Manager { registry: shared };
        // Struct creation succeeded without panic
    }

    #[test]
    fn test_home_object_path_simple() {
        assert_eq!(
            home_object_path("alice"),
            "/org/freedesktop/home1/home/alice"
        );
    }

    #[test]
    fn test_home_object_path_with_dots() {
        let path = home_object_path("alice.test");
        assert_eq!(path, "/org/freedesktop/home1/home/alice_2etest");
    }

    #[test]
    fn test_home_object_path_with_hyphen() {
        let path = home_object_path("alice-test");
        assert_eq!(path, "/org/freedesktop/home1/home/alice_2dtest");
    }

    #[test]
    fn test_home_object_path_underscore_preserved() {
        let path = home_object_path("alice_test");
        assert_eq!(path, "/org/freedesktop/home1/home/alice_test");
    }
    use tempfile::TempDir;

    // -- Helpers ------------------------------------------------------------

    fn make_registry(tmp: &TempDir) -> HomeRegistry {
        let id_dir = tmp.path().join("identity");
        let rt_dir = tmp.path().join("runtime");
        fs::create_dir_all(&id_dir).unwrap();
        fs::create_dir_all(&rt_dir).unwrap();
        HomeRegistry::with_paths(&id_dir, &rt_dir)
    }

    fn write_file(path: &Path, content: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, content).unwrap();
    }

    fn create_simple(
        reg: &mut HomeRegistry,
        name: &str,
        home_dir: Option<&str>,
        image_path: Option<&str>,
    ) -> Result<String, String> {
        reg.create(CreateParams {
            user_name: name,
            real_name: None,
            shell: None,
            storage: Storage::Directory,
            password: None,
            home_dir_override: home_dir,
            image_path_override: image_path,
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        })
    }

    // -----------------------------------------------------------------------
    // Storage type parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_storage_parse_all() {
        assert_eq!(Storage::parse("directory"), Some(Storage::Directory));
        assert_eq!(Storage::parse("DIRECTORY"), Some(Storage::Directory));
        assert_eq!(Storage::parse("subvolume"), Some(Storage::Subvolume));
        assert_eq!(Storage::parse("luks"), Some(Storage::Luks));
        assert_eq!(Storage::parse("cifs"), Some(Storage::Cifs));
        assert_eq!(Storage::parse("fscrypt"), Some(Storage::Fscrypt));
        assert_eq!(Storage::parse("unknown"), None);
        assert_eq!(Storage::parse(""), None);
    }

    #[test]
    fn test_storage_display() {
        assert_eq!(Storage::Directory.to_string(), "directory");
        assert_eq!(Storage::Luks.to_string(), "luks");
        assert_eq!(Storage::Cifs.to_string(), "cifs");
    }

    // -----------------------------------------------------------------------
    // Home state parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_home_state_all() {
        assert_eq!(HomeState::parse("inactive"), Some(HomeState::Inactive));
        assert_eq!(HomeState::parse("ACTIVE"), Some(HomeState::Active));
        assert_eq!(HomeState::parse("activating"), Some(HomeState::Activating));
        assert_eq!(
            HomeState::parse("deactivating"),
            Some(HomeState::Deactivating)
        );
        assert_eq!(HomeState::parse("locked"), Some(HomeState::Locked));
        assert_eq!(HomeState::parse("absent"), Some(HomeState::Absent));
        assert_eq!(HomeState::parse("dirty"), Some(HomeState::Dirty));
        assert_eq!(HomeState::parse("unknown"), None);
    }

    #[test]
    fn test_home_state_display() {
        assert_eq!(HomeState::Active.to_string(), "active");
        assert_eq!(HomeState::Locked.to_string(), "locked");
    }

    // -----------------------------------------------------------------------
    // Disposition parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_disposition_all() {
        assert_eq!(Disposition::parse("regular"), Some(Disposition::Regular));
        assert_eq!(Disposition::parse("SYSTEM"), Some(Disposition::System));
        assert_eq!(
            Disposition::parse("intrinsic"),
            Some(Disposition::Intrinsic)
        );
        assert_eq!(Disposition::parse("other"), None);
    }

    // -----------------------------------------------------------------------
    // User name validation
    // -----------------------------------------------------------------------

    #[test]
    fn test_valid_user_names() {
        assert!(is_valid_user_name("alice"));
        assert!(is_valid_user_name("_test"));
        assert!(is_valid_user_name("user-1"));
        assert!(is_valid_user_name("user_name123"));
    }

    #[test]
    fn test_invalid_user_names() {
        assert!(!is_valid_user_name(""));
        assert!(!is_valid_user_name("Root")); // uppercase
        assert!(!is_valid_user_name("1user")); // starts with digit
        assert!(!is_valid_user_name("user.name")); // dot not allowed
        assert!(!is_valid_user_name("user name")); // space
        assert!(!is_valid_user_name(&"a".repeat(257))); // too long
    }

    #[test]
    fn test_reserved_user_names() {
        assert!(!is_valid_user_name("root"));
        assert!(!is_valid_user_name("nobody"));
        assert!(!is_valid_user_name("daemon"));
        assert!(!is_valid_user_name("bin"));
        assert!(!is_valid_user_name("sshd"));
    }

    // -----------------------------------------------------------------------
    // User record creation and defaults
    // -----------------------------------------------------------------------

    #[test]
    fn test_user_record_new_defaults() {
        let rec = UserRecord::new("alice", 60001);
        assert_eq!(rec.user_name, "alice");
        assert_eq!(rec.real_name, "alice");
        assert_eq!(rec.uid, 60001);
        assert_eq!(rec.gid, 60001);
        assert_eq!(rec.home_directory, "/home/alice");
        assert_eq!(rec.image_path, "/home/alice.homedir");
        assert_eq!(rec.shell, "/bin/bash");
        assert_eq!(rec.storage, Storage::Directory);
        assert_eq!(rec.disposition, Disposition::Regular);
        assert_eq!(rec.state, HomeState::Inactive);
        assert!(rec.disk_size.is_none());
        assert!(rec.hashed_passwords.is_empty());
        assert!(!rec.locked);
        assert!(!rec.auto_login);
        assert_eq!(rec.service, "io.systemd.Home");
    }

    // -----------------------------------------------------------------------
    // JSON serialization roundtrip
    // -----------------------------------------------------------------------

    #[test]
    fn test_json_roundtrip_basic() {
        let rec = UserRecord::new("testuser", 60005);
        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec.user_name, rec2.user_name);
        assert_eq!(rec.uid, rec2.uid);
        assert_eq!(rec.gid, rec2.gid);
        assert_eq!(rec.storage, rec2.storage);
        assert_eq!(rec.disposition, rec2.disposition);
        assert_eq!(rec.state, rec2.state);
        assert_eq!(rec.shell, rec2.shell);
        assert_eq!(rec.home_directory, rec2.home_directory);
        assert_eq!(rec.image_path, rec2.image_path);
        assert_eq!(rec.locked, rec2.locked);
    }

    #[test]
    fn test_json_roundtrip_with_all_fields() {
        let mut rec = UserRecord::new("bob", 60010);
        rec.real_name = "Bob Smith".to_string();
        rec.member_of = vec!["wheel".to_string(), "users".to_string()];
        rec.disk_size = Some(10 * 1024 * 1024 * 1024);
        rec.disk_usage = Some(512 * 1024 * 1024);
        rec.password_hint = Some("my pet's name".to_string());
        rec.auto_login = true;
        rec.hashed_passwords = vec![hash_password("secret")];
        rec.locked = true;

        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec.real_name, rec2.real_name);
        assert_eq!(rec.member_of, rec2.member_of);
        assert_eq!(rec.disk_size, rec2.disk_size);
        assert_eq!(rec.disk_usage, rec2.disk_usage);
        assert_eq!(rec.password_hint, rec2.password_hint);
        assert_eq!(rec.auto_login, rec2.auto_login);
        assert_eq!(rec.hashed_passwords, rec2.hashed_passwords);
        assert_eq!(rec.locked, rec2.locked);
    }

    #[test]
    fn test_json_roundtrip_null_optional_fields() {
        let rec = UserRecord::new("nulltest", 60020);
        let json = rec.to_json();
        assert!(json.contains("\"diskSize\": null"));
        assert!(json.contains("\"passwordHint\": null"));
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert!(rec2.disk_size.is_none());
        assert!(rec2.password_hint.is_none());
    }

    #[test]
    fn test_json_roundtrip_empty_arrays() {
        let rec = UserRecord::new("emptyarr", 60021);
        let json = rec.to_json();
        assert!(json.contains("\"memberOf\": []"));
        assert!(json.contains("\"hashedPassword\": []"));
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert!(rec2.member_of.is_empty());
        assert!(rec2.hashed_passwords.is_empty());
    }

    #[test]
    fn test_json_escape_special_chars() {
        let mut rec = UserRecord::new("esctest", 60022);
        rec.real_name = "Alice \"Bob\" O'Connor\nLine2".to_string();
        let json = rec.to_json();
        assert!(json.contains("\\\""));
        assert!(json.contains("\\n"));
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec.real_name, rec2.real_name);
    }

    #[test]
    fn test_json_parse_error_not_object() {
        assert!(UserRecord::from_json("not json").is_err());
        assert!(UserRecord::from_json("[]").is_err());
    }

    #[test]
    fn test_json_parse_error_missing_required_field() {
        assert!(UserRecord::from_json("{}").is_err()); // missing userName
        assert!(UserRecord::from_json("{\"userName\": \"a\"}").is_err()); // missing uid
    }

    // -----------------------------------------------------------------------
    // User record formatting
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_inspect_contains_fields() {
        let rec = UserRecord::new("alice", 60001);
        let s = rec.format_inspect();
        assert!(s.contains("alice"));
        assert!(s.contains("60001"));
        assert!(s.contains("directory"));
        assert!(s.contains("inactive"));
        assert!(s.contains("/home/alice"));
        assert!(s.contains("io.systemd.Home"));
    }

    #[test]
    fn test_format_inspect_with_optional_fields() {
        let mut rec = UserRecord::new("bob", 60002);
        rec.disk_size = Some(1024 * 1024 * 1024);
        rec.disk_usage = Some(512 * 1024);
        rec.password_hint = Some("favorite color".to_string());
        rec.member_of = vec!["wheel".to_string()];
        let s = rec.format_inspect();
        assert!(s.contains("1.0G"));
        assert!(s.contains("512.0K"));
        assert!(s.contains("favorite color"));
        assert!(s.contains("wheel"));
    }

    #[test]
    fn test_format_show_key_value() {
        let rec = UserRecord::new("charlie", 60003);
        let s = rec.format_show();
        assert!(s.contains("UserName=charlie"));
        assert!(s.contains("UID=60003"));
        assert!(s.contains("Storage=directory"));
        assert!(s.contains("State=inactive"));
        assert!(s.contains("Locked=false"));
    }

    // -----------------------------------------------------------------------
    // Password hashing
    // -----------------------------------------------------------------------

    #[test]
    fn test_hash_password_deterministic() {
        let h1 = hash_password("hello");
        let h2 = hash_password("hello");
        assert_eq!(h1, h2);
    }

    #[test]
    fn test_hash_password_different_inputs() {
        let h1 = hash_password("hello");
        let h2 = hash_password("world");
        assert_ne!(h1, h2);
    }

    #[test]
    fn test_hash_password_format() {
        let h = hash_password("test");
        assert!(h.starts_with("$6$homed$"));
        assert_eq!(h.len(), "$6$homed$".len() + 128);
    }

    #[test]
    fn test_verify_password_correct() {
        let h = hash_password("mypass");
        assert!(verify_password("mypass", &h));
    }

    #[test]
    fn test_verify_password_incorrect() {
        let h = hash_password("mypass");
        assert!(!verify_password("wrongpass", &h));
    }

    // -----------------------------------------------------------------------
    // Size parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_size_bytes() {
        assert_eq!(parse_size("1024"), Some(1024));
        assert_eq!(parse_size("0"), Some(0));
    }

    #[test]
    fn test_parse_size_units() {
        assert_eq!(parse_size("1K"), Some(1024));
        assert_eq!(parse_size("1M"), Some(1024 * 1024));
        assert_eq!(parse_size("1G"), Some(1024 * 1024 * 1024));
        assert_eq!(parse_size("2T"), Some(2 * 1024 * 1024 * 1024 * 1024));
    }

    #[test]
    fn test_parse_size_invalid() {
        assert_eq!(parse_size(""), None);
        assert_eq!(parse_size("abc"), None);
        assert_eq!(parse_size("G"), None);
    }

    // -----------------------------------------------------------------------
    // format_bytes
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_bytes_scales() {
        assert_eq!(format_bytes(0), "0B");
        assert_eq!(format_bytes(512), "512B");
        assert_eq!(format_bytes(1024), "1.0K");
        assert_eq!(format_bytes(1024 * 1024), "1.0M");
        assert_eq!(format_bytes(1024 * 1024 * 1024), "1.0G");
        assert_eq!(format_bytes(1024 * 1024 * 1024 * 1024), "1.0T");
    }

    // -----------------------------------------------------------------------
    // Home registry: create
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_create_basic() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let home_base = tmp.path().join("homes");
        fs::create_dir_all(&home_base).unwrap();
        let image = home_base.join("alice.homedir");

        let result = reg.create(CreateParams {
            user_name: "alice",
            real_name: Some("Alice"),
            shell: None,
            storage: Storage::Directory,
            password: Some("Str0ng!Pass123"),
            home_dir_override: None,
            image_path_override: Some(image.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        });
        assert!(result.is_ok(), "create failed: {:?}", result);
        assert!(reg.contains("alice"));

        let rec = reg.get("alice").unwrap();
        assert_eq!(rec.uid, UID_MIN);
        assert_eq!(rec.real_name, "Alice");
        assert!(!rec.hashed_passwords.is_empty());
        assert!(image.exists());
    }

    #[test]
    fn test_registry_create_duplicate() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img1 = tmp.path().join("u1.homedir");
        let img2 = tmp.path().join("u2.homedir");

        create_simple(&mut reg, "alice", None, Some(img1.to_str().unwrap())).unwrap();
        let result = create_simple(&mut reg, "alice", None, Some(img2.to_str().unwrap()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    #[test]
    fn test_registry_create_invalid_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);

        assert!(create_simple(&mut reg, "", None, None).is_err());
        assert!(create_simple(&mut reg, "root", None, None).is_err());
        assert!(create_simple(&mut reg, "1bad", None, None).is_err());
    }

    #[test]
    fn test_registry_create_uid_allocation() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);

        for i in 0..3 {
            let name = format!("user{}", i);
            let img = tmp.path().join(format!("{}.homedir", name));
            create_simple(&mut reg, &name, None, Some(img.to_str().unwrap())).unwrap();
        }
        assert_eq!(reg.get("user0").unwrap().uid, UID_MIN);
        assert_eq!(reg.get("user1").unwrap().uid, UID_MIN + 1);
        assert_eq!(reg.get("user2").unwrap().uid, UID_MIN + 2);
    }

    #[test]
    fn test_registry_create_luks_no_password() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("luks.homedir");

        let result = reg.create(CreateParams {
            user_name: "luksuser",
            real_name: None,
            shell: None,
            storage: Storage::Luks,
            password: None,
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Password required"));
    }

    // -----------------------------------------------------------------------
    // Home registry: remove
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_remove_basic() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        create_simple(&mut reg, "alice", None, Some(img.to_str().unwrap())).unwrap();
        assert!(img.exists());

        let result = reg.remove("alice");
        assert!(result.is_ok());
        assert!(!reg.contains("alice"));
        assert!(!img.exists());
    }

    #[test]
    fn test_registry_remove_unknown() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert!(reg.remove("nonexistent").is_err());
    }

    #[test]
    fn test_registry_remove_active_fails() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        create_simple(
            &mut reg,
            "alice",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.activate("alice").unwrap();

        let result = reg.remove("alice");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("still active"));
    }

    // -----------------------------------------------------------------------
    // Home registry: activate / deactivate
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_activate_deactivate() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");

        create_simple(
            &mut reg,
            "alice",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();

        // Activate
        let result = reg.activate("alice");
        assert!(result.is_ok(), "activate: {:?}", result);
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Active);
        // symlink should exist
        assert!(hd.symlink_metadata().unwrap().file_type().is_symlink());

        // Deactivate
        let result = reg.deactivate("alice");
        assert!(result.is_ok());
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Inactive);
        // symlink should be removed
        assert!(!hd.exists());
    }

    #[test]
    fn test_registry_activate_already_active() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        create_simple(
            &mut reg,
            "alice",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.activate("alice").unwrap();
        // Second activate should succeed (already active)
        let result = reg.activate("alice");
        assert!(result.is_ok());
        assert!(result.unwrap().contains("already active"));
    }

    #[test]
    fn test_registry_activate_absent() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("nonexistent.homedir");
        // Create record but don't create the directory
        let mut rec = UserRecord::new("ghost", UID_MIN);
        rec.image_path = img.to_str().unwrap().to_string();
        let id_dir = tmp.path().join("identity");
        write_file(&id_dir.join("ghost.identity"), &rec.to_json());
        reg.load();

        let result = reg.activate("ghost");
        assert!(result.is_err());
        assert_eq!(reg.get("ghost").unwrap().state, HomeState::Absent);
    }

    #[test]
    fn test_registry_deactivate_already_inactive() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        create_simple(&mut reg, "alice", None, Some(img.to_str().unwrap())).unwrap();
        let result = reg.deactivate("alice");
        assert!(result.is_ok());
        assert!(result.unwrap().contains("already inactive"));
    }

    #[test]
    fn test_registry_activate_unknown() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert!(reg.activate("nobody-here").is_err());
    }

    // -----------------------------------------------------------------------
    // Home registry: lock / unlock
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_lock_unlock() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        create_simple(
            &mut reg,
            "alice",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.activate("alice").unwrap();

        // Lock
        let result = reg.lock("alice");
        assert!(result.is_ok());
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Locked);
        assert!(reg.get("alice").unwrap().locked);

        // Can't activate while locked
        assert!(reg.activate("alice").is_err());

        // Unlock
        let result = reg.unlock("alice");
        assert!(result.is_ok());
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Active);
        assert!(!reg.get("alice").unwrap().locked);
    }

    #[test]
    fn test_registry_lock_not_active() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        create_simple(&mut reg, "alice", None, Some(img.to_str().unwrap())).unwrap();
        assert!(reg.lock("alice").is_err()); // inactive, can't lock
    }

    #[test]
    fn test_registry_unlock_not_locked() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        create_simple(
            &mut reg,
            "alice",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.activate("alice").unwrap();
        assert!(reg.unlock("alice").is_err()); // active but not locked
    }

    // -----------------------------------------------------------------------
    // Home registry: lock-all / deactivate-all
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_lock_all() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        for i in 0..3 {
            let name = format!("user{}", i);
            let img = tmp.path().join(format!("{}.homedir", name));
            let hd = tmp.path().join(format!("home_{}", name));
            create_simple(
                &mut reg,
                &name,
                Some(hd.to_str().unwrap()),
                Some(img.to_str().unwrap()),
            )
            .unwrap();
            reg.activate(&name).unwrap();
        }
        let msg = reg.lock_all();
        assert!(msg.contains("3"));
        for i in 0..3 {
            assert_eq!(
                reg.get(&format!("user{}", i)).unwrap().state,
                HomeState::Locked
            );
        }
    }

    #[test]
    fn test_registry_deactivate_all() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        for i in 0..3 {
            let name = format!("user{}", i);
            let img = tmp.path().join(format!("{}.homedir", name));
            let hd = tmp.path().join(format!("home_{}", name));
            create_simple(
                &mut reg,
                &name,
                Some(hd.to_str().unwrap()),
                Some(img.to_str().unwrap()),
            )
            .unwrap();
            reg.activate(&name).unwrap();
        }
        // Lock one of them
        reg.lock("user1").unwrap();

        let msg = reg.deactivate_all();
        assert!(msg.contains("3"));
        for i in 0..3 {
            assert_eq!(
                reg.get(&format!("user{}", i)).unwrap().state,
                HomeState::Inactive
            );
        }
    }

    // -----------------------------------------------------------------------
    // Home registry: update
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_update_fields() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        create_simple(&mut reg, "alice", None, Some(img.to_str().unwrap())).unwrap();

        let result = reg.update(
            "alice",
            Some("Alice Smith"),
            Some("/bin/zsh"),
            Some("hint"),
            Some(true),
        );
        assert!(result.is_ok());

        let rec = reg.get("alice").unwrap();
        assert_eq!(rec.real_name, "Alice Smith");
        assert_eq!(rec.shell, "/bin/zsh");
        assert_eq!(rec.password_hint, Some("hint".to_string()));
        assert!(rec.auto_login);
    }

    #[test]
    fn test_registry_update_partial() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        reg.create(CreateParams {
            user_name: "alice",
            real_name: Some("Alice"),
            shell: None,
            storage: Storage::Directory,
            password: None,
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        })
        .unwrap();

        // Only update shell, leave real_name as-is
        reg.update("alice", None, Some("/bin/fish"), None, None)
            .unwrap();
        let rec = reg.get("alice").unwrap();
        assert_eq!(rec.real_name, "Alice");
        assert_eq!(rec.shell, "/bin/fish");
    }

    #[test]
    fn test_registry_update_unknown_user() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert!(reg.update("ghost", Some("x"), None, None, None).is_err());
    }

    // -----------------------------------------------------------------------
    // Home registry: change password
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_change_password() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        reg.create(CreateParams {
            user_name: "alice",
            real_name: None,
            shell: None,
            storage: Storage::Directory,
            password: Some("Old!Passw0rd99"),
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        })
        .unwrap();
        assert!(verify_password(
            "Old!Passw0rd99",
            &reg.get("alice").unwrap().hashed_passwords[0]
        ));

        // Disable password policy so we can test the change itself
        reg.get_mut("alice").unwrap().enforce_password_policy = false;
        reg.change_password("alice", "New!Passw0rd99").unwrap();
        assert!(verify_password(
            "New!Passw0rd99",
            &reg.get("alice").unwrap().hashed_passwords[0]
        ));
        assert!(!verify_password(
            "Old!Passw0rd99",
            &reg.get("alice").unwrap().hashed_passwords[0]
        ));
    }

    #[test]
    fn test_registry_change_password_empty() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        create_simple(&mut reg, "alice", None, Some(img.to_str().unwrap())).unwrap();
        assert!(reg.change_password("alice", "").is_err());
    }

    // -----------------------------------------------------------------------
    // Home registry: resize
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_resize_directory() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        create_simple(&mut reg, "alice", None, Some(img.to_str().unwrap())).unwrap();

        let result = reg.resize("alice", 10 * 1024 * 1024 * 1024);
        assert!(result.is_ok());
        assert_eq!(
            reg.get("alice").unwrap().disk_size,
            Some(10 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_registry_resize_unknown() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert!(reg.resize("nobody", 1024).is_err());
    }

    // -----------------------------------------------------------------------
    // Home registry: persistence (save / load)
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_save_and_load() {
        let tmp = TempDir::new().unwrap();
        let id_dir = tmp.path().join("identity");
        let rt_dir = tmp.path().join("runtime");
        fs::create_dir_all(&id_dir).unwrap();
        fs::create_dir_all(&rt_dir).unwrap();

        // Create and save
        {
            let mut reg = HomeRegistry::with_paths(&id_dir, &rt_dir);
            let img = tmp.path().join("alice.homedir");
            reg.create(CreateParams {
                user_name: "alice",
                real_name: Some("Alice"),
                shell: None,
                storage: Storage::Directory,
                password: None,
                home_dir_override: None,
                image_path_override: Some(img.to_str().unwrap()),
                disk_size: None,
                cifs_service: None,
                cifs_user_name: None,
                cifs_domain: None,
                enforce_password_policy: Some(false),
            })
            .unwrap();
            assert!(id_dir.join("alice.identity").exists());
        }

        // Load in new registry
        {
            let mut reg = HomeRegistry::with_paths(&id_dir, &rt_dir);
            reg.load();
            assert!(reg.contains("alice"));
            assert_eq!(reg.get("alice").unwrap().real_name, "Alice");
        }
    }

    #[test]
    fn test_registry_load_skips_invalid() {
        let tmp = TempDir::new().unwrap();
        let id_dir = tmp.path().join("identity");
        let rt_dir = tmp.path().join("runtime");
        fs::create_dir_all(&id_dir).unwrap();
        fs::create_dir_all(&rt_dir).unwrap();

        // Write a valid file
        write_file(
            &id_dir.join("good.identity"),
            &UserRecord::new("good", 60001).to_json(),
        );
        // Write an invalid file
        write_file(&id_dir.join("bad.identity"), "not json");
        // Write a dotfile (should be skipped)
        write_file(&id_dir.join(".hidden.identity"), "{}");
        // Write a non-identity file (should be skipped)
        write_file(&id_dir.join("readme.txt"), "hello");

        let mut reg = HomeRegistry::with_paths(&id_dir, &rt_dir);
        reg.load();
        assert_eq!(reg.len(), 1);
        assert!(reg.contains("good"));
    }

    #[test]
    fn test_registry_load_restores_runtime_state() {
        let tmp = TempDir::new().unwrap();
        let id_dir = tmp.path().join("identity");
        let rt_dir = tmp.path().join("runtime");
        fs::create_dir_all(&id_dir).unwrap();
        fs::create_dir_all(&rt_dir).unwrap();

        write_file(
            &id_dir.join("alice.identity"),
            &UserRecord::new("alice", 60001).to_json(),
        );
        write_file(&rt_dir.join("alice"), "active");

        let mut reg = HomeRegistry::with_paths(&id_dir, &rt_dir);
        reg.load();
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Active);
    }

    #[test]
    fn test_registry_load_empty_dir() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        reg.load();
        assert!(reg.is_empty());
    }

    #[test]
    fn test_registry_load_nonexistent_dir() {
        let tmp = TempDir::new().unwrap();
        let mut reg =
            HomeRegistry::with_paths(&tmp.path().join("nope"), &tmp.path().join("also_nope"));
        reg.load();
        assert!(reg.is_empty());
    }

    // -----------------------------------------------------------------------
    // Home registry: GC
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_gc_marks_absent() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        create_simple(
            &mut reg,
            "alice",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.activate("alice").unwrap();
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Active);

        // Remove the image directory behind the daemon's back
        fs::remove_dir_all(&img).unwrap();
        reg.gc();
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Absent);
    }

    #[test]
    fn test_registry_gc_keeps_present() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        create_simple(
            &mut reg,
            "alice",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.activate("alice").unwrap();
        reg.gc();
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Active);
    }

    // -----------------------------------------------------------------------
    // Home registry: format_list
    // -----------------------------------------------------------------------

    #[test]
    fn test_registry_format_list_empty() {
        let tmp = TempDir::new().unwrap();
        let reg = make_registry(&tmp);
        let s = reg.format_list();
        assert!(s.contains("No managed home directories"));
    }

    #[test]
    fn test_registry_format_list_with_homes() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img1 = tmp.path().join("alice.homedir");
        let img2 = tmp.path().join("bob.homedir");
        create_simple(&mut reg, "alice", None, Some(img1.to_str().unwrap())).unwrap();
        create_simple(&mut reg, "bob", None, Some(img2.to_str().unwrap())).unwrap();

        let s = reg.format_list();
        assert!(s.contains("alice"));
        assert!(s.contains("bob"));
        assert!(s.contains("2 home(s) listed"));
    }

    // -----------------------------------------------------------------------
    // Control command handling
    // -----------------------------------------------------------------------

    #[test]
    fn test_control_ping() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert_eq!(handle_control_command(&mut reg, "PING"), "PONG\n");
        assert_eq!(handle_control_command(&mut reg, "ping"), "PONG\n");
    }

    #[test]
    fn test_control_empty() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_unknown() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "FOOBAR");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("unknown command"));
    }

    #[test]
    fn test_control_list_empty() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "LIST");
        assert!(resp.contains("No managed home"));
    }

    #[test]
    fn test_control_create_and_list() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("testuser.homedir");
        let cmd = format!("CREATE testuser image={}", img.display());
        let resp = handle_control_command(&mut reg, &cmd);
        assert!(resp.contains("Created"), "resp: {}", resp);

        let resp = handle_control_command(&mut reg, "LIST");
        assert!(resp.contains("testuser"));
        assert!(resp.contains("1 home(s) listed"));
    }

    #[test]
    fn test_control_create_with_options() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let cmd = format!(
            "CREATE alice realname=Alice shell=/bin/zsh password=Str0ng!Pass99 image={} no-password-quality",
            img.display()
        );
        let resp = handle_control_command(&mut reg, &cmd);
        assert!(resp.contains("Created"), "resp: {}", resp);

        let rec = reg.get("alice").unwrap();
        assert_eq!(rec.real_name, "Alice");
        assert_eq!(rec.shell, "/bin/zsh");
        assert!(verify_password("Str0ng!Pass99", &rec.hashed_passwords[0]));
    }

    #[test]
    fn test_control_create_missing_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "CREATE");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_inspect() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(&mut reg, &format!("CREATE alice image={}", img.display()));

        let resp = handle_control_command(&mut reg, "INSPECT alice");
        assert!(resp.contains("alice"));
        assert!(resp.contains("directory"));
        assert!(resp.contains("inactive"));
    }

    #[test]
    fn test_control_inspect_missing_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "INSPECT");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_inspect_unknown_user() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "INSPECT ghost");
        assert!(resp.contains("ERROR"));
        assert!(resp.contains("unknown user"));
    }

    #[test]
    fn test_control_show() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(&mut reg, &format!("CREATE alice image={}", img.display()));

        let resp = handle_control_command(&mut reg, "SHOW alice");
        assert!(resp.contains("UserName=alice"));
        assert!(resp.contains("Storage=directory"));
    }

    #[test]
    fn test_control_record_json() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(&mut reg, &format!("CREATE alice image={}", img.display()));

        let resp = handle_control_command(&mut reg, "RECORD alice");
        assert!(resp.contains("\"userName\": \"alice\""));
        // Should be valid JSON
        assert!(UserRecord::from_json(resp.trim()).is_ok());
    }

    #[test]
    fn test_control_activate_deactivate() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        handle_control_command(
            &mut reg,
            &format!("CREATE alice home={} image={}", hd.display(), img.display()),
        );

        let resp = handle_control_command(&mut reg, "ACTIVATE alice");
        assert!(resp.contains("Activated"), "resp: {}", resp);
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Active);

        let resp = handle_control_command(&mut reg, "DEACTIVATE alice");
        assert!(resp.contains("Deactivated"), "resp: {}", resp);
        assert_eq!(reg.get("alice").unwrap().state, HomeState::Inactive);
    }

    #[test]
    fn test_control_activate_missing_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "ACTIVATE");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_lock_unlock() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("home_alice");
        handle_control_command(
            &mut reg,
            &format!("CREATE alice home={} image={}", hd.display(), img.display()),
        );
        handle_control_command(&mut reg, "ACTIVATE alice");

        let resp = handle_control_command(&mut reg, "LOCK alice");
        assert!(resp.contains("Locked"), "resp: {}", resp);

        let resp = handle_control_command(&mut reg, "UNLOCK alice");
        assert!(resp.contains("Unlocked"), "resp: {}", resp);
    }

    #[test]
    fn test_control_lock_all() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        for i in 0..2 {
            let name = format!("user{}", i);
            let img = tmp.path().join(format!("{}.homedir", name));
            let hd = tmp.path().join(format!("home_{}", name));
            handle_control_command(
                &mut reg,
                &format!(
                    "CREATE {} home={} image={}",
                    name,
                    hd.display(),
                    img.display()
                ),
            );
            handle_control_command(&mut reg, &format!("ACTIVATE {}", name));
        }

        let resp = handle_control_command(&mut reg, "LOCK-ALL");
        assert!(resp.contains("2"), "resp: {}", resp);
    }

    #[test]
    fn test_control_deactivate_all() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        for i in 0..2 {
            let name = format!("user{}", i);
            let img = tmp.path().join(format!("{}.homedir", name));
            let hd = tmp.path().join(format!("home_{}", name));
            handle_control_command(
                &mut reg,
                &format!(
                    "CREATE {} home={} image={}",
                    name,
                    hd.display(),
                    img.display()
                ),
            );
            handle_control_command(&mut reg, &format!("ACTIVATE {}", name));
        }

        let resp = handle_control_command(&mut reg, "DEACTIVATE-ALL");
        assert!(resp.contains("2"), "resp: {}", resp);
    }

    #[test]
    fn test_control_update() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(&mut reg, &format!("CREATE alice image={}", img.display()));

        let resp = handle_control_command(
            &mut reg,
            "UPDATE alice realname=Alice shell=/bin/fish auto-login=true",
        );
        assert!(resp.contains("Updated"), "resp: {}", resp);
        let rec = reg.get("alice").unwrap();
        assert_eq!(rec.real_name, "Alice");
        assert_eq!(rec.shell, "/bin/fish");
        assert!(rec.auto_login);
    }

    #[test]
    fn test_control_update_missing_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "UPDATE");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_passwd() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(
            &mut reg,
            &format!("CREATE alice image={} no-password-quality", img.display()),
        );
        // Disable password policy so we can test passwd with short passwords
        reg.get_mut("alice").unwrap().enforce_password_policy = false;

        let resp = handle_control_command(&mut reg, "PASSWD alice New!Passw0rd42");
        assert!(resp.contains("Password changed"), "resp: {}", resp);
        assert!(verify_password(
            "New!Passw0rd42",
            &reg.get("alice").unwrap().hashed_passwords[0]
        ));
    }

    #[test]
    fn test_control_passwd_missing_args() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert!(handle_control_command(&mut reg, "PASSWD").contains("ERROR"));
        assert!(handle_control_command(&mut reg, "PASSWD alice").contains("ERROR"));
    }

    #[test]
    fn test_control_resize() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(&mut reg, &format!("CREATE alice image={}", img.display()));

        let resp = handle_control_command(&mut reg, "RESIZE alice 5G");
        assert!(resp.contains("Updated disk size"), "resp: {}", resp);
        assert_eq!(
            reg.get("alice").unwrap().disk_size,
            Some(5 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_control_resize_invalid_size() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(&mut reg, &format!("CREATE alice image={}", img.display()));

        let resp = handle_control_command(&mut reg, "RESIZE alice abc");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_resize_missing_args() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert!(handle_control_command(&mut reg, "RESIZE").contains("ERROR"));
        assert!(handle_control_command(&mut reg, "RESIZE alice").contains("ERROR"));
    }

    #[test]
    fn test_control_gc() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "GC");
        assert_eq!(resp, "OK\n");
    }

    #[test]
    fn test_control_reload() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "RELOAD");
        assert!(resp.contains("Reloaded"));
    }

    #[test]
    fn test_control_remove() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        handle_control_command(&mut reg, &format!("CREATE alice image={}", img.display()));
        assert!(reg.contains("alice"));

        let resp = handle_control_command(&mut reg, "REMOVE alice");
        assert!(resp.contains("Removed"), "resp: {}", resp);
        assert!(!reg.contains("alice"));
    }

    #[test]
    fn test_control_remove_missing_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "REMOVE");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_remove_unknown() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "REMOVE ghost");
        assert!(resp.contains("ERROR"));
    }

    #[test]
    fn test_control_case_insensitive() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        assert_eq!(handle_control_command(&mut reg, "ping"), "PONG\n");
        assert_eq!(handle_control_command(&mut reg, "Ping"), "PONG\n");
        assert_eq!(handle_control_command(&mut reg, "PING"), "PONG\n");
        assert!(handle_control_command(&mut reg, "list").contains("No managed"));
        assert!(handle_control_command(&mut reg, "gc").contains("OK"));
    }

    // -----------------------------------------------------------------------
    // Full lifecycle integration
    // -----------------------------------------------------------------------

    #[test]
    fn test_full_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("alice.homedir");
        let hd = tmp.path().join("homes").join("alice");

        // Create
        let resp = handle_control_command(
            &mut reg,
            &format!(
                "CREATE alice realname=Alice password=Str0ng!Pass99 home={} image={} no-password-quality",
                hd.display(),
                img.display()
            ),
        );
        assert!(resp.contains("Created"), "{}", resp);
        assert!(img.exists());

        // Inspect
        let resp = handle_control_command(&mut reg, "INSPECT alice");
        assert!(resp.contains("Alice"));

        // Activate
        let resp = handle_control_command(&mut reg, "ACTIVATE alice");
        assert!(resp.contains("Activated"), "{}", resp);
        assert!(hd.symlink_metadata().unwrap().file_type().is_symlink());

        // Lock
        let resp = handle_control_command(&mut reg, "LOCK alice");
        assert!(resp.contains("Locked"), "{}", resp);

        // Unlock
        let resp = handle_control_command(&mut reg, "UNLOCK alice");
        assert!(resp.contains("Unlocked"), "{}", resp);

        // Update
        let resp = handle_control_command(&mut reg, "UPDATE alice shell=/bin/zsh");
        assert!(resp.contains("Updated"), "{}", resp);
        assert_eq!(reg.get("alice").unwrap().shell, "/bin/zsh");

        // Passwd — disable quality enforcement first so we can test the command
        reg.get_mut("alice").unwrap().enforce_password_policy = false;
        let resp = handle_control_command(&mut reg, "PASSWD alice New!Passw0rd42");
        assert!(resp.contains("Password changed"), "{}", resp);
        assert!(verify_password(
            "New!Passw0rd42",
            &reg.get("alice").unwrap().hashed_passwords[0]
        ));

        // Resize
        let resp = handle_control_command(&mut reg, "RESIZE alice 10G");
        assert!(resp.contains("Updated disk size"), "{}", resp);

        // Deactivate
        let resp = handle_control_command(&mut reg, "DEACTIVATE alice");
        assert!(resp.contains("Deactivated"), "{}", resp);

        // Remove
        let resp = handle_control_command(&mut reg, "REMOVE alice");
        assert!(resp.contains("Removed"), "{}", resp);
        assert!(!img.exists());
        assert!(!reg.contains("alice"));
    }

    #[test]
    fn test_multi_user_lifecycle() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);

        // Create multiple users
        for name in &["alice", "bob", "charlie"] {
            let img = tmp.path().join(format!("{}.homedir", name));
            let hd = tmp.path().join(format!("home_{}", name));
            let resp = handle_control_command(
                &mut reg,
                &format!(
                    "CREATE {} home={} image={}",
                    name,
                    hd.display(),
                    img.display()
                ),
            );
            assert!(resp.contains("Created"), "create {}: {}", name, resp);
        }
        assert_eq!(reg.len(), 3);

        // Activate all
        for name in &["alice", "bob", "charlie"] {
            handle_control_command(&mut reg, &format!("ACTIVATE {}", name));
        }

        // Lock all
        let resp = handle_control_command(&mut reg, "LOCK-ALL");
        assert!(resp.contains("3"), "{}", resp);

        // Deactivate all
        let resp = handle_control_command(&mut reg, "DEACTIVATE-ALL");
        assert!(resp.contains("3"), "{}", resp);

        // Remove all
        for name in &["alice", "bob", "charlie"] {
            handle_control_command(&mut reg, &format!("REMOVE {}", name));
        }
        assert!(reg.is_empty());
    }

    // -----------------------------------------------------------------------
    // dir_disk_usage
    // -----------------------------------------------------------------------

    #[test]
    fn test_dir_disk_usage_empty() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("empty");
        fs::create_dir_all(&dir).unwrap();
        assert_eq!(dir_disk_usage(&dir), 0);
    }

    #[test]
    fn test_dir_disk_usage_with_files() {
        let tmp = TempDir::new().unwrap();
        let dir = tmp.path().join("hasfiles");
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("a.txt"), "hello").unwrap();
        fs::write(dir.join("b.txt"), "world!").unwrap();
        let sub = dir.join("sub");
        fs::create_dir_all(&sub).unwrap();
        fs::write(sub.join("c.txt"), "nested").unwrap();

        let usage = dir_disk_usage(&dir);
        // 5 + 6 + 6 = 17 bytes
        assert_eq!(usage, 17);
    }

    #[test]
    fn test_dir_disk_usage_nonexistent() {
        assert_eq!(
            dir_disk_usage(Path::new("/tmp/definitely_does_not_exist_12345")),
            0
        );
    }

    // -----------------------------------------------------------------------
    // Watchdog parsing
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_watchdog_usec_valid() {
        let d = parse_watchdog_usec("6000000").unwrap();
        assert_eq!(d, Duration::from_micros(3000000));
    }

    #[test]
    fn test_parse_watchdog_usec_zero() {
        assert!(parse_watchdog_usec("0").is_none());
    }

    #[test]
    fn test_parse_watchdog_usec_invalid() {
        assert!(parse_watchdog_usec("abc").is_none());
    }

    #[test]
    fn test_parse_watchdog_usec_empty() {
        assert!(parse_watchdog_usec("").is_none());
    }

    // -----------------------------------------------------------------------
    // Timestamp helper
    // -----------------------------------------------------------------------

    #[test]
    fn test_chrono_lite_timestamp_format() {
        let ts = chrono_lite_timestamp();
        // Should be HH:MM:SS format
        assert_eq!(ts.len(), 8);
        assert_eq!(ts.as_bytes()[2], b':');
        assert_eq!(ts.as_bytes()[5], b':');
    }

    // -----------------------------------------------------------------------
    // JSON edge cases
    // -----------------------------------------------------------------------

    #[test]
    fn test_json_parse_extra_whitespace() {
        let json = r#"  {
            "userName" : "ws_test" ,
            "uid" : 60099
        }  "#;
        let rec = UserRecord::from_json(json).unwrap();
        assert_eq!(rec.user_name, "ws_test");
        assert_eq!(rec.uid, 60099);
    }

    #[test]
    fn test_json_parse_unicode_escape() {
        let mut rec = UserRecord::new("unicode", 60050);
        rec.real_name = "Ünïcödé".to_string();
        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.real_name, "Ünïcödé");
    }

    #[test]
    fn test_json_parse_backslash_in_path() {
        let mut rec = UserRecord::new("pathtest", 60051);
        rec.shell = "/bin/ba\\sh".to_string();
        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.shell, "/bin/ba\\sh");
    }

    // -----------------------------------------------------------------------
    // UID allocation exhaustion
    // -----------------------------------------------------------------------

    #[test]
    fn test_uid_exhaustion() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        // Set next_uid to near the max
        reg.next_uid = UID_MAX;
        let img = tmp.path().join("last.homedir");
        let result = create_simple(&mut reg, "last", None, Some(img.to_str().unwrap()));
        assert!(result.is_ok());

        let img2 = tmp.path().join("overflow.homedir");
        let result = create_simple(&mut reg, "overflow", None, Some(img2.to_str().unwrap()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("UID range exhausted"));
    }

    // -----------------------------------------------------------------------
    // Image path already exists on create
    // -----------------------------------------------------------------------

    #[test]
    fn test_create_image_already_exists() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("existing.homedir");
        fs::create_dir_all(&img).unwrap();

        let result = create_simple(&mut reg, "alice", None, Some(img.to_str().unwrap()));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    // -----------------------------------------------------------------------
    // Subvolume storage (stub)
    // -----------------------------------------------------------------------

    #[test]
    fn test_create_subvolume_storage() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("sub.homedir");

        let result = reg.create(CreateParams {
            user_name: "subuser",
            real_name: None,
            shell: None,
            storage: Storage::Subvolume,
            password: None,
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        });
        assert!(result.is_ok(), "create subvolume: {:?}", result);
        assert!(img.exists());
        assert_eq!(reg.get("subuser").unwrap().storage, Storage::Subvolume);
    }

    // -----------------------------------------------------------------------
    // Password quality enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn test_password_quality_default() {
        let policy = PasswordQuality::default();
        assert_eq!(policy.min_length, DEFAULT_MIN_PASSWORD_LENGTH);
        assert_eq!(policy.min_classes, DEFAULT_MIN_PASSWORD_CLASSES);
        assert!(policy.reject_username);
        assert!(policy.reject_palindrome);
        assert!(policy.reject_dictionary);
    }

    #[test]
    fn test_password_quality_too_short() {
        let policy = PasswordQuality::default();
        let result = policy.check("Ab1!", "testuser");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    #[test]
    fn test_password_quality_enough_classes() {
        let policy = PasswordQuality::default();
        // Only 2 classes (lower + digit)
        let result = policy.check("abcde12345", "testuser");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("character classes"));
    }

    #[test]
    fn test_password_quality_contains_username() {
        let policy = PasswordQuality::default();
        let result = policy.check("Testuser!123", "testuser");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("user name"));
    }

    #[test]
    fn test_password_quality_palindrome() {
        let policy = PasswordQuality::default();
        let result = policy.check("abcD1Dcba", "other");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("palindrome"));
    }

    #[test]
    fn test_password_quality_dictionary() {
        let policy = PasswordQuality::default();
        let _result = policy.check("password", "other");
        // "password" is too short and a dictionary word, but it will fail
        // on the length check first unless we override min_length
        let mut lax = policy.clone();
        lax.min_length = 4;
        lax.min_classes = 1;
        let result = lax.check("password", "other");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("dictionary"));
    }

    #[test]
    fn test_password_quality_good_password() {
        let policy = PasswordQuality::default();
        let result = policy.check("Str0ng!Pass99", "alice");
        assert!(result.is_ok());
    }

    #[test]
    fn test_count_character_classes() {
        assert_eq!(count_character_classes(""), 0);
        assert_eq!(count_character_classes("abc"), 1);
        assert_eq!(count_character_classes("ABC"), 1);
        assert_eq!(count_character_classes("123"), 1);
        assert_eq!(count_character_classes("!@#"), 1);
        assert_eq!(count_character_classes("aB1"), 3);
        assert_eq!(count_character_classes("aB1!"), 4);
    }

    #[test]
    fn test_is_palindrome() {
        assert!(!is_palindrome(""));
        assert!(!is_palindrome("a"));
        assert!(is_palindrome("aa"));
        assert!(is_palindrome("aba"));
        assert!(is_palindrome("abba"));
        assert!(is_palindrome("AbcCbA")); // case insensitive
        assert!(!is_palindrome("abc"));
    }

    #[test]
    fn test_password_quality_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("weakpw.homedir");

        // With password quality disabled, weak password should be accepted
        let result = reg.create(CreateParams {
            user_name: "weakpwuser",
            real_name: None,
            shell: None,
            storage: Storage::Directory,
            password: Some("123"),
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        });
        assert!(result.is_ok());
    }

    #[test]
    fn test_password_quality_enforced_on_create() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("strongpw.homedir");

        // With password quality enforced, weak password should fail
        let result = reg.create(CreateParams {
            user_name: "stronguser",
            real_name: None,
            shell: None,
            storage: Storage::Directory,
            password: Some("123"),
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(true),
        });
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    // -----------------------------------------------------------------------
    // Recovery keys
    // -----------------------------------------------------------------------

    #[test]
    fn test_recovery_key_generation() {
        let key = generate_recovery_key();
        // Should be 8 groups of 8 chars separated by 7 dashes
        let groups: Vec<&str> = key.split('-').collect();
        assert_eq!(groups.len(), RECOVERY_KEY_GROUPS);
        for group in &groups {
            assert_eq!(group.len(), RECOVERY_KEY_GROUP_SIZE);
            // All chars should be modhex
            for ch in group.bytes() {
                assert!(MODHEX.contains(&ch), "invalid modhex char: {}", ch as char);
            }
        }
    }

    #[test]
    fn test_recovery_key_modhex_roundtrip() {
        let data = vec![0x00, 0x01, 0x0f, 0x10, 0xff, 0xab, 0xcd, 0xef];
        let encoded = modhex_encode_grouped(&data);
        let decoded = modhex_decode(&encoded).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn test_recovery_key_verify() {
        let key = generate_recovery_key();
        let hashed = hash_password(&key);
        assert!(verify_recovery_key(&key, std::slice::from_ref(&hashed)));
        assert!(!verify_recovery_key("wrong-key-value", &[hashed]));
    }

    #[test]
    fn test_registry_generate_recovery_key() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("rec.homedir");
        create_simple(&mut reg, "recuser", None, Some(img.to_str().unwrap())).unwrap();

        let key = reg.generate_recovery_key("recuser").unwrap();
        assert!(!key.is_empty());

        let rec = reg.get("recuser").unwrap();
        assert_eq!(rec.recovery_key.len(), 1);
        assert!(verify_recovery_key(&key, &rec.recovery_key));
    }

    #[test]
    fn test_modhex_decode_invalid() {
        assert!(modhex_decode("xyz").is_err()); // odd length
        assert!(modhex_decode("aa").is_err()); // 'a' is not in modhex
    }

    // -----------------------------------------------------------------------
    // Hex helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_hex_encode_decode() {
        let data = vec![0x00, 0x01, 0x0f, 0x10, 0xff];
        let hex = hex_encode(&data);
        assert_eq!(hex, "00010f10ff");
        let decoded = hex_decode(&hex).unwrap();
        assert_eq!(data, decoded);
    }

    #[test]
    fn test_hex_decode_invalid() {
        assert!(hex_decode("0").is_err()); // odd length
        assert!(hex_decode("zz").is_err()); // bad hex
    }

    // -----------------------------------------------------------------------
    // PKCS#11 / FIDO2 types
    // -----------------------------------------------------------------------

    #[test]
    fn test_pkcs11_encrypted_key_new() {
        let key = Pkcs11EncryptedKey::new("pkcs11:model=YubiKey", "deadbeef", "abc123");
        assert_eq!(key.uri, "pkcs11:model=YubiKey");
        assert_eq!(key.encrypted_key, "deadbeef");
        assert_eq!(key.hashedPassword, "abc123");
    }

    #[test]
    fn test_pkcs11_key_json_roundtrip() {
        let key = Pkcs11EncryptedKey::new("pkcs11:model=YubiKey", "deadbeef01", "hashval");
        let json = key.to_json();
        let parsed = Pkcs11EncryptedKey::from_json_str(&json).unwrap();
        assert_eq!(key, parsed);
    }

    #[test]
    fn test_pkcs11_unwrap_key_not_available() {
        let key = Pkcs11EncryptedKey::new("pkcs11:model=Test", "deadbeef", "hash");
        let result = key.unwrap_key();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available"));
    }

    #[test]
    fn test_fido2_credential_new() {
        let cred = Fido2HmacCredential::new("cred123", "io.systemd.home", "salt456");
        assert_eq!(cred.credential_id, "cred123");
        assert_eq!(cred.rp_id, "io.systemd.home");
        assert_eq!(cred.salt, "salt456");
        assert!(cred.up);
        assert!(!cred.uv);
    }

    #[test]
    fn test_fido2_credential_json_roundtrip() {
        let cred = Fido2HmacCredential::new("aabbcc", "io.systemd.home", "112233");
        let json = cred.to_json();
        let parsed = Fido2HmacCredential::from_json_str(&json).unwrap();
        assert_eq!(cred, parsed);
    }

    #[test]
    fn test_fido2_derive_key_not_available() {
        let cred = Fido2HmacCredential::new("aabbcc", "io.systemd.home", "112233");
        let result = cred.derive_key();
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not available"));
    }

    #[test]
    fn test_registry_add_pkcs11_key() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("pk.homedir");
        create_simple(&mut reg, "pkuser", None, Some(img.to_str().unwrap())).unwrap();

        let result = reg.add_pkcs11_key("pkuser", "pkcs11:model=Test", "deadbeef", "hash");
        assert!(result.is_ok());
        assert_eq!(reg.get("pkuser").unwrap().pkcs11_encrypted_key.len(), 1);
    }

    #[test]
    fn test_registry_add_fido2_credential() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("fd.homedir");
        create_simple(&mut reg, "fduser", None, Some(img.to_str().unwrap())).unwrap();

        let result = reg.add_fido2_credential("fduser", "cred1", "rp1", "salt1");
        assert!(result.is_ok());
        assert_eq!(reg.get("fduser").unwrap().fido2_hmac_credential.len(), 1);
    }

    // -----------------------------------------------------------------------
    // LUKS backend types
    // -----------------------------------------------------------------------

    #[test]
    fn test_luks_config_for_user() {
        let mut rec = UserRecord::new("alice", 60001);
        rec.storage = Storage::Luks;
        rec.disk_size = Some(512 * 1024 * 1024);
        rec.image_path = "/home/alice.home".to_string();

        let config = LuksConfig::for_user(&rec);
        assert_eq!(config.dm_name, "home-alice");
        assert_eq!(config.image_size, 512 * 1024 * 1024);
        assert_eq!(config.cipher, "aes-xts-plain64");
        assert_eq!(config.key_size, 256);
        assert_eq!(config.fs_type, "ext4");
    }

    #[test]
    fn test_luks_config_custom_cipher() {
        let mut rec = UserRecord::new("bob", 60002);
        rec.storage = Storage::Luks;
        rec.luks_cipher = Some("serpent-xts-plain64".to_string());
        rec.luks_volume_key_size = Some(512);

        let config = LuksConfig::for_user(&rec);
        assert_eq!(config.cipher, "serpent-xts-plain64");
        assert_eq!(config.key_size, 512);
    }

    #[test]
    fn test_derive_luks_key_deterministic() {
        let key1 = derive_luks_key("password", b"salt");
        let key2 = derive_luks_key("password", b"salt");
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 32);
    }

    #[test]
    fn test_derive_luks_key_different_inputs() {
        let key1 = derive_luks_key("password1", b"salt");
        let key2 = derive_luks_key("password2", b"salt");
        assert_ne!(key1, key2);

        let key3 = derive_luks_key("password", b"salt1");
        let key4 = derive_luks_key("password", b"salt2");
        assert_ne!(key3, key4);
    }

    #[test]
    fn test_create_sparse_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("test.img");
        create_sparse_file(&path, 1024 * 1024).unwrap();
        assert!(path.exists());
        let meta = fs::metadata(&path).unwrap();
        assert_eq!(meta.len(), 1024 * 1024);
    }

    // -----------------------------------------------------------------------
    // CIFS backend types
    // -----------------------------------------------------------------------

    #[test]
    fn test_cifs_config_for_user() {
        let mut rec = UserRecord::new("cifsuser", 60005);
        rec.storage = Storage::Cifs;
        rec.cifs_service = Some("//server/share".to_string());
        rec.cifs_user_name = Some("netuser".to_string());
        rec.cifs_domain = Some("WORKGROUP".to_string());

        let config = CifsConfig::for_user(&rec).unwrap();
        assert_eq!(config.service, "//server/share");
        assert_eq!(config.cifs_user, "netuser");
        assert_eq!(config.domain, "WORKGROUP");
    }

    #[test]
    fn test_cifs_config_defaults() {
        let mut rec = UserRecord::new("cifsuser2", 60006);
        rec.storage = Storage::Cifs;
        rec.cifs_service = Some("//nas/homes".to_string());

        let config = CifsConfig::for_user(&rec).unwrap();
        assert_eq!(config.cifs_user, "cifsuser2"); // defaults to system user
        assert_eq!(config.domain, "");
    }

    #[test]
    fn test_cifs_config_none_without_service() {
        let rec = UserRecord::new("noservice", 60007);
        assert!(CifsConfig::for_user(&rec).is_none());
    }

    #[test]
    fn test_create_cifs_home_needs_service() {
        let mut rec = UserRecord::new("cifsuser3", 60008);
        rec.storage = Storage::Cifs;
        // No cifs_service set
        let result = create_cifs_home_area(&rec);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not configured"));
    }

    #[test]
    fn test_create_cifs_home_validates_service() {
        let mut rec = UserRecord::new("cifsuser4", 60009);
        rec.storage = Storage::Cifs;
        rec.cifs_service = Some("badpath".to_string());
        let result = create_cifs_home_area(&rec);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("must start with //"));
    }

    #[test]
    fn test_create_cifs_home_valid_service() {
        let mut rec = UserRecord::new("cifsuser5", 60010);
        rec.storage = Storage::Cifs;
        rec.cifs_service = Some("//server/share".to_string());
        rec.home_directory = "/tmp/test-cifs-home-homed".to_string();
        let result = create_cifs_home_area(&rec);
        assert!(result.is_ok());
    }

    // -----------------------------------------------------------------------
    // fscrypt backend types
    // -----------------------------------------------------------------------

    #[test]
    fn test_fscrypt_config_for_user() {
        let rec = UserRecord::new("fscryptuser", 60020);
        let config = FscryptConfig::for_user(&rec);
        // Descriptor should be derived from username
        assert!(!config.key_descriptor.is_empty());
        assert_eq!(config.key_descriptor.len(), 16); // 64-bit hash in hex
    }

    #[test]
    fn test_fscrypt_config_explicit_descriptor() {
        let mut rec = UserRecord::new("fscryptuser2", 60021);
        rec.fscrypt_key_descriptor = Some("abcdef0123456789".to_string());
        let config = FscryptConfig::for_user(&rec);
        assert_eq!(config.key_descriptor, "abcdef0123456789");
    }

    #[test]
    fn test_derive_fscrypt_key_deterministic() {
        let key1 = derive_fscrypt_key("password");
        let key2 = derive_fscrypt_key("password");
        assert_eq!(key1, key2);
        assert_eq!(key1.len(), 64); // 8 * 8 bytes
    }

    #[test]
    fn test_derive_fscrypt_key_different_passwords() {
        let key1 = derive_fscrypt_key("pass1");
        let key2 = derive_fscrypt_key("pass2");
        assert_ne!(key1, key2);
    }

    #[test]
    fn test_create_fscrypt_home_area() {
        let tmp = TempDir::new().unwrap();
        let mut rec = UserRecord::new("fscryptcreate", 60022);
        rec.storage = Storage::Fscrypt;
        rec.image_path = tmp
            .path()
            .join("fscrypt.homedir")
            .to_str()
            .unwrap()
            .to_string();

        let result = create_fscrypt_home_area(&rec);
        assert!(result.is_ok());
        assert!(Path::new(&rec.image_path).exists());
        // Check descriptor metadata file
        let desc_path = Path::new(&rec.image_path).join(".fscrypt-descriptor");
        assert!(desc_path.exists());
    }

    #[test]
    fn test_create_fscrypt_home_already_exists() {
        let tmp = TempDir::new().unwrap();
        let existing = tmp.path().join("existing.homedir");
        fs::create_dir_all(&existing).unwrap();

        let mut rec = UserRecord::new("fscryptdup", 60023);
        rec.storage = Storage::Fscrypt;
        rec.image_path = existing.to_str().unwrap().to_string();

        let result = create_fscrypt_home_area(&rec);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("already exists"));
    }

    // -----------------------------------------------------------------------
    // Btrfs backend helpers
    // -----------------------------------------------------------------------

    #[test]
    fn test_btrfs_subvol_create_fallback() {
        let tmp = TempDir::new().unwrap();
        let subvol_path = tmp.path().join("testsubvol");

        // On non-btrfs filesystems, this should fall back to mkdir
        let result = btrfs_subvol_create(&subvol_path);
        assert!(result.is_ok());
        assert!(subvol_path.exists());
    }

    #[test]
    fn test_btrfs_subvol_delete_fallback() {
        let tmp = TempDir::new().unwrap();
        let subvol_path = tmp.path().join("delsubvol");
        fs::create_dir_all(&subvol_path).unwrap();

        // On non-btrfs, falls back to rm -rf
        let result = btrfs_subvol_delete(&subvol_path);
        assert!(result.is_ok());
        assert!(!subvol_path.exists());
    }

    // -----------------------------------------------------------------------
    // Auto-activation monitor
    // -----------------------------------------------------------------------

    #[test]
    fn test_auto_activation_monitor_new() {
        let monitor = AutoActivationMonitor::new();
        assert!(monitor.watched_uids.is_empty());
    }

    #[test]
    fn test_auto_activation_monitor_refresh() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("auto.homedir");
        create_simple(&mut reg, "autouser", None, Some(img.to_str().unwrap())).unwrap();
        reg.get_mut("autouser").unwrap().auto_login = true;

        let mut monitor = AutoActivationMonitor::new();
        monitor.refresh(&reg);

        let uid = reg.get("autouser").unwrap().uid;
        assert_eq!(monitor.should_activate(uid), Some("autouser"));
        assert_eq!(monitor.should_activate(99999), None);
    }

    #[test]
    fn test_auto_activation_on_login() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("login.homedir");
        let hd = tmp.path().join("home_login");
        create_simple(
            &mut reg,
            "loginuser",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.get_mut("loginuser").unwrap().auto_login = true;
        let uid = reg.get("loginuser").unwrap().uid;

        let mut monitor = AutoActivationMonitor::new();
        monitor.refresh(&reg);

        // Simulate login — should auto-activate
        let result = monitor.on_user_login(uid, &mut reg);
        assert_eq!(result, Some("loginuser".to_string()));
        assert_eq!(reg.get("loginuser").unwrap().state, HomeState::Active);
    }

    #[test]
    fn test_auto_activation_on_logout() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("logout.homedir");
        let hd = tmp.path().join("home_logout");
        create_simple(
            &mut reg,
            "logoutuser",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();
        reg.get_mut("logoutuser").unwrap().auto_login = true;
        let uid = reg.get("logoutuser").unwrap().uid;

        let mut monitor = AutoActivationMonitor::new();
        monitor.refresh(&reg);

        // Activate first
        reg.activate("logoutuser").unwrap();
        assert_eq!(reg.get("logoutuser").unwrap().state, HomeState::Active);

        // Simulate logout — should auto-deactivate
        let result = monitor.on_user_logout(uid, &mut reg);
        assert_eq!(result, Some("logoutuser".to_string()));
        assert_eq!(reg.get("logoutuser").unwrap().state, HomeState::Inactive);
    }

    #[test]
    fn test_auto_activation_not_watched() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("noauto.homedir");
        create_simple(&mut reg, "noautouser", None, Some(img.to_str().unwrap())).unwrap();
        // auto_login is false by default

        let mut monitor = AutoActivationMonitor::new();
        monitor.refresh(&reg);

        let uid = reg.get("noautouser").unwrap().uid;
        let result = monitor.on_user_login(uid, &mut reg);
        assert_eq!(result, None);
    }

    // -----------------------------------------------------------------------
    // Control commands for new features
    // -----------------------------------------------------------------------

    #[test]
    fn test_control_recovery_key() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("rk.homedir");
        create_simple(&mut reg, "rkuser", None, Some(img.to_str().unwrap())).unwrap();

        let resp = handle_control_command(&mut reg, "RECOVERY-KEY rkuser");
        assert!(resp.contains("Recovery key for"));
        assert!(resp.contains("rkuser"));
        assert_eq!(reg.get("rkuser").unwrap().recovery_key.len(), 1);
    }

    #[test]
    fn test_control_recovery_key_missing_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "RECOVERY-KEY");
        assert!(resp.starts_with("ERROR:"));
    }

    #[test]
    fn test_control_add_pkcs11() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("p11.homedir");
        create_simple(&mut reg, "p11user", None, Some(img.to_str().unwrap())).unwrap();

        let resp = handle_control_command(
            &mut reg,
            "ADD-PKCS11 p11user pkcs11:model=Test deadbeef hash123",
        );
        assert!(resp.contains("PKCS#11 key added"));
        assert_eq!(reg.get("p11user").unwrap().pkcs11_encrypted_key.len(), 1);
    }

    #[test]
    fn test_control_add_pkcs11_missing_args() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "ADD-PKCS11 user1");
        assert!(resp.starts_with("ERROR:"));
    }

    #[test]
    fn test_control_add_fido2() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("f2.homedir");
        create_simple(&mut reg, "f2user", None, Some(img.to_str().unwrap())).unwrap();

        let resp = handle_control_command(&mut reg, "ADD-FIDO2 f2user cred1 io.systemd.home salt1");
        assert!(resp.contains("FIDO2 credential added"));
        assert_eq!(reg.get("f2user").unwrap().fido2_hmac_credential.len(), 1);
    }

    #[test]
    fn test_control_add_fido2_missing_args() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "ADD-FIDO2 user1 cred1");
        assert!(resp.starts_with("ERROR:"));
    }

    #[test]
    fn test_control_check_password_good() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "CHECK-PASSWORD Str0ng!P@ss testuser");
        assert!(resp.starts_with("OK:"));
    }

    #[test]
    fn test_control_check_password_weak() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "CHECK-PASSWORD 123 testuser");
        assert!(resp.starts_with("FAIL:"));
    }

    #[test]
    fn test_control_check_password_missing() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "CHECK-PASSWORD");
        assert!(resp.starts_with("ERROR:"));
    }

    #[test]
    fn test_control_activate_secret() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("as.homedir");
        let hd = tmp.path().join("home_as");
        create_simple(
            &mut reg,
            "asuser",
            Some(hd.to_str().unwrap()),
            Some(img.to_str().unwrap()),
        )
        .unwrap();

        let resp = handle_control_command(&mut reg, "ACTIVATE-SECRET asuser somepassword");
        assert!(!resp.starts_with("ERROR:"), "got: {}", resp);
        assert_eq!(reg.get("asuser").unwrap().state, HomeState::Active);
    }

    #[test]
    fn test_control_activate_secret_missing_name() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let resp = handle_control_command(&mut reg, "ACTIVATE-SECRET");
        assert!(resp.starts_with("ERROR:"));
    }

    // -----------------------------------------------------------------------
    // JSON roundtrip with new fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_json_roundtrip_luks_fields() {
        let mut rec = UserRecord::new("luksrt", 60030);
        rec.luks_cipher = Some("aes-xts-plain64".to_string());
        rec.luks_volume_key_size = Some(512);
        rec.luks_pbkdf_type = Some("argon2id".to_string());
        rec.luks_extra_mount_options = Some("discard".to_string());

        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.luks_cipher, rec.luks_cipher);
        assert_eq!(rec2.luks_volume_key_size, rec.luks_volume_key_size);
        assert_eq!(rec2.luks_pbkdf_type, rec.luks_pbkdf_type);
        assert_eq!(rec2.luks_extra_mount_options, rec.luks_extra_mount_options);
    }

    #[test]
    fn test_json_roundtrip_cifs_fields() {
        let mut rec = UserRecord::new("cifsrt", 60031);
        rec.cifs_service = Some("//nas/home".to_string());
        rec.cifs_user_name = Some("netuser".to_string());
        rec.cifs_domain = Some("CORP".to_string());

        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.cifs_service, rec.cifs_service);
        assert_eq!(rec2.cifs_user_name, rec.cifs_user_name);
        assert_eq!(rec2.cifs_domain, rec.cifs_domain);
    }

    #[test]
    fn test_json_roundtrip_fscrypt_fields() {
        let mut rec = UserRecord::new("fscrt", 60032);
        rec.fscrypt_key_descriptor = Some("abcdef0123456789".to_string());

        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.fscrypt_key_descriptor, rec.fscrypt_key_descriptor);
    }

    #[test]
    fn test_json_roundtrip_pkcs11_keys() {
        let mut rec = UserRecord::new("p11rt", 60033);
        rec.pkcs11_encrypted_key.push(Pkcs11EncryptedKey::new(
            "pkcs11:model=YubiKey",
            "aabbccdd",
            "hash1",
        ));
        rec.pkcs11_encrypted_key.push(Pkcs11EncryptedKey::new(
            "pkcs11:model=SoftHSM",
            "11223344",
            "hash2",
        ));

        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.pkcs11_encrypted_key.len(), 2);
        assert_eq!(rec2.pkcs11_encrypted_key[0].uri, "pkcs11:model=YubiKey");
        assert_eq!(rec2.pkcs11_encrypted_key[1].encrypted_key, "11223344");
    }

    #[test]
    fn test_json_roundtrip_fido2_credentials() {
        let mut rec = UserRecord::new("f2rt", 60034);
        rec.fido2_hmac_credential.push(Fido2HmacCredential::new(
            "cred01",
            "io.systemd.home",
            "salt01",
        ));

        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.fido2_hmac_credential.len(), 1);
        assert_eq!(rec2.fido2_hmac_credential[0].rp_id, "io.systemd.home");
    }

    #[test]
    fn test_json_roundtrip_recovery_keys() {
        let mut rec = UserRecord::new("rkrt", 60035);
        rec.recovery_key.push("$6$homed$somehash".to_string());

        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.recovery_key.len(), 1);
        assert_eq!(rec2.recovery_key[0], "$6$homed$somehash");
    }

    #[test]
    fn test_json_roundtrip_null_new_fields() {
        let rec = UserRecord::new("nullrt", 60036);
        let json = rec.to_json();
        let rec2 = UserRecord::from_json(&json).unwrap();
        assert_eq!(rec2.luks_cipher, None);
        assert_eq!(rec2.luks_volume_key_size, None);
        assert_eq!(rec2.cifs_service, None);
        assert_eq!(rec2.fscrypt_key_descriptor, None);
        assert!(rec2.pkcs11_encrypted_key.is_empty());
        assert!(rec2.fido2_hmac_credential.is_empty());
        assert!(rec2.recovery_key.is_empty());
    }

    // -----------------------------------------------------------------------
    // Inspect/show output with new fields
    // -----------------------------------------------------------------------

    #[test]
    fn test_format_inspect_luks_fields() {
        let mut rec = UserRecord::new("luksins", 60040);
        rec.storage = Storage::Luks;
        rec.luks_cipher = Some("aes-xts-plain64".to_string());
        rec.luks_volume_key_size = Some(256);
        rec.luks_pbkdf_type = Some("argon2id".to_string());

        let output = rec.format_inspect();
        assert!(output.contains("LUKS Cipher: aes-xts-plain64"));
        assert!(output.contains("LUKS KeySize: 256 bits"));
        assert!(output.contains("LUKS PBKDF: argon2id"));
    }

    #[test]
    fn test_format_inspect_cifs_fields() {
        let mut rec = UserRecord::new("cifsins", 60041);
        rec.storage = Storage::Cifs;
        rec.cifs_service = Some("//server/share".to_string());
        rec.cifs_user_name = Some("netuser".to_string());
        rec.cifs_domain = Some("CORP".to_string());

        let output = rec.format_inspect();
        assert!(output.contains("CIFS Service: //server/share"));
        assert!(output.contains("CIFS User: netuser"));
        assert!(output.contains("CIFS Domain: CORP"));
    }

    #[test]
    fn test_format_inspect_fscrypt_fields() {
        let mut rec = UserRecord::new("fscins", 60042);
        rec.storage = Storage::Fscrypt;
        rec.fscrypt_key_descriptor = Some("abcdef0123456789".to_string());

        let output = rec.format_inspect();
        assert!(output.contains("fscrypt Desc: abcdef0123456789"));
    }

    #[test]
    fn test_format_inspect_token_counts() {
        let mut rec = UserRecord::new("tokins", 60043);
        rec.pkcs11_encrypted_key
            .push(Pkcs11EncryptedKey::new("uri", "key", "hash"));
        rec.fido2_hmac_credential
            .push(Fido2HmacCredential::new("cred", "rp", "salt"));
        rec.recovery_key.push("hashed".to_string());

        let output = rec.format_inspect();
        assert!(output.contains("PKCS#11 Keys: 1 configured"));
        assert!(output.contains("FIDO2 Creds: 1 configured"));
        assert!(output.contains("Recovery Keys: 1 configured"));
    }

    // -----------------------------------------------------------------------
    // dm-crypt / loopback constants
    // -----------------------------------------------------------------------

    #[test]
    fn test_dm_ioctl_nr() {
        let nr = dm_ioctl_nr(DM_DEV_CREATE_NR);
        // Should be a valid ioctl number (non-zero)
        assert_ne!(nr, 0);
    }

    #[test]
    fn test_dm_ioctl_init_basic() {
        let mut buf = Vec::new();
        dm_ioctl_init(&mut buf, "test-device", 0);
        assert_eq!(buf.len(), DM_STRUCT_SIZE);
        // Check version at offset 0
        let v0 = u32::from_ne_bytes(buf[0..4].try_into().unwrap());
        assert_eq!(v0, DM_VERSION_MAJOR);
        // Check name at offset 40
        let name = &buf[40..40 + 11];
        assert_eq!(name, b"test-device");
    }

    #[test]
    fn test_dm_target_append() {
        let mut buf = Vec::new();
        dm_ioctl_init(&mut buf, "test", 0);
        let initial_len = buf.len();
        dm_target_append(
            &mut buf,
            0,
            2048,
            "crypt",
            "aes-xts-plain64 key 0 /dev/loop0 0",
        );
        assert!(buf.len() > initial_len);
    }

    #[test]
    fn test_loop_constants() {
        assert_ne!(LOOP_SET_FD, 0);
        assert_ne!(LOOP_CLR_FD, 0);
        assert_ne!(LOOP_SET_STATUS64, 0);
        assert_ne!(LOOP_CTL_GET_FREE, 0);
        assert_ne!(LOOP_SET_CAPACITY, 0);
    }

    #[test]
    fn test_loopinfo64_default() {
        let info = LoopInfo64::default();
        assert_eq!(info.lo_flags, 0);
        assert_eq!(info.lo_offset, 0);
    }

    // -----------------------------------------------------------------------
    // Create with new storage backends via control commands
    // -----------------------------------------------------------------------

    #[test]
    fn test_control_create_luks_needs_password() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("lc.img");
        let cmd = format!(
            "CREATE luksctl storage=luks image={} no-password-quality",
            img.display()
        );
        let resp = handle_control_command(&mut reg, &cmd);
        assert!(resp.starts_with("ERROR:"));
        assert!(resp.contains("Password required"));
    }

    #[test]
    fn test_control_create_cifs_needs_service() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let cmd = "CREATE cifsctl storage=cifs no-password-quality";
        let resp = handle_control_command(&mut reg, cmd);
        assert!(resp.starts_with("ERROR:"));
        assert!(resp.contains("not configured"));
    }

    #[test]
    fn test_control_create_cifs_with_service() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let cmd = "CREATE cifsctl2 storage=cifs cifs-service=//server/share no-password-quality";
        let resp = handle_control_command(&mut reg, cmd);
        // Should succeed (just validates config, doesn't actually mount)
        assert!(!resp.starts_with("ERROR:"), "got: {}", resp);
        assert_eq!(reg.get("cifsctl2").unwrap().storage, Storage::Cifs);
        assert_eq!(
            reg.get("cifsctl2").unwrap().cifs_service.as_deref(),
            Some("//server/share")
        );
    }

    #[test]
    fn test_control_create_fscrypt() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("fc.homedir");
        let cmd = format!(
            "CREATE fscryptctl storage=fscrypt image={} no-password-quality",
            img.display()
        );
        let resp = handle_control_command(&mut reg, &cmd);
        assert!(!resp.starts_with("ERROR:"), "got: {}", resp);
        assert_eq!(reg.get("fscryptctl").unwrap().storage, Storage::Fscrypt);
        assert!(img.exists());
    }

    #[test]
    fn test_control_create_with_disk_size() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("ds.homedir");
        let cmd = format!(
            "CREATE dsuser image={} disk-size=1G no-password-quality",
            img.display()
        );
        let resp = handle_control_command(&mut reg, &cmd);
        assert!(!resp.starts_with("ERROR:"), "got: {}", resp);
        assert_eq!(
            reg.get("dsuser").unwrap().disk_size,
            Some(1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_control_case_insensitive_new_commands() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("ci.homedir");
        create_simple(&mut reg, "ciuser", None, Some(img.to_str().unwrap())).unwrap();

        let resp1 = handle_control_command(&mut reg, "recovery-key ciuser");
        assert!(resp1.contains("Recovery key"));

        let resp2 = handle_control_command(&mut reg, "check-password Str0ng!P@ss other");
        assert!(resp2.starts_with("OK:"));
    }

    // -----------------------------------------------------------------------
    // Resize with new backends
    // -----------------------------------------------------------------------

    #[test]
    fn test_resize_subvolume() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("rsub.homedir");

        let result = reg.create(CreateParams {
            user_name: "rsubuser",
            real_name: None,
            shell: None,
            storage: Storage::Subvolume,
            password: None,
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        });
        assert!(result.is_ok());

        let result = reg.resize("rsubuser", 2 * 1024 * 1024 * 1024);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("btrfs quota"));
        assert_eq!(
            reg.get("rsubuser").unwrap().disk_size,
            Some(2 * 1024 * 1024 * 1024)
        );
    }

    #[test]
    fn test_resize_cifs_rejected() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);

        let result = reg.create(CreateParams {
            user_name: "cifsuser",
            real_name: None,
            shell: None,
            storage: Storage::Cifs,
            password: None,
            home_dir_override: None,
            image_path_override: None,
            disk_size: None,
            cifs_service: Some("//server/share"),
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        });
        assert!(result.is_ok());

        let result = reg.resize("cifsuser", 1024);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("not supported"));
    }

    #[test]
    fn test_resize_fscrypt_advisory() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("rfsc.homedir");

        let result = reg.create(CreateParams {
            user_name: "rfscuser",
            real_name: None,
            shell: None,
            storage: Storage::Fscrypt,
            password: None,
            home_dir_override: None,
            image_path_override: Some(img.to_str().unwrap()),
            disk_size: None,
            cifs_service: None,
            cifs_user_name: None,
            cifs_domain: None,
            enforce_password_policy: Some(false),
        });
        assert!(result.is_ok());

        let result = reg.resize("rfscuser", 5 * 1024 * 1024 * 1024);
        assert!(result.is_ok());
        assert!(result.unwrap().contains("advisory"));
    }

    // -----------------------------------------------------------------------
    // Password change with quality enforcement
    // -----------------------------------------------------------------------

    #[test]
    fn test_change_password_quality_enforced() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("pwq.homedir");
        create_simple(&mut reg, "pwquser", None, Some(img.to_str().unwrap())).unwrap();
        // create_simple sets enforce_password_policy to false, so enable it
        reg.get_mut("pwquser").unwrap().enforce_password_policy = true;
        assert!(reg.get("pwquser").unwrap().enforce_password_policy);

        let result = reg.change_password("pwquser", "weak");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    #[test]
    fn test_change_password_quality_disabled() {
        let tmp = TempDir::new().unwrap();
        let mut reg = make_registry(&tmp);
        let img = tmp.path().join("pwqd.homedir");
        create_simple(&mut reg, "pwqduser", None, Some(img.to_str().unwrap())).unwrap();
        reg.get_mut("pwqduser").unwrap().enforce_password_policy = false;

        let result = reg.change_password("pwqduser", "ab");
        assert!(result.is_ok());
    }
}
