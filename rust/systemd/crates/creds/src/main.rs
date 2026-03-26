//! systemd-creds — Lists, shows, encrypts and decrypts service credentials.
//!
//! A drop-in replacement for `systemd-creds(1)` supporting the following
//! subcommands:
//!
//! - `list`          — List credentials passed into the current execution context
//! - `cat`           — Show contents of specified credentials
//! - `setup`         — Generate a host encryption key for credentials
//! - `encrypt`       — Encrypt a credential for use with LoadCredentialEncrypted=/SetCredentialEncrypted=
//! - `decrypt`       — Decrypt an encrypted credential
//! - `has-tpm2`      — Report whether TPM2 is available
//!
//! Encryption uses AES-256-GCM keyed by a SHA-256 hash of a secret
//! concatenated with the credential name (matching systemd's approach).
//! Encrypted credentials are Base64-encoded for safe embedding in unit files.
//!
//! The host key is stored at `/var/lib/systemd/credential.secret` (256 bytes
//! of random data, readable only by root).
//!
//! Supported key modes (`--with-key=`):
//! - `host`      — AES key derived from host secret + credential name
//! - `tpm2`      — AES key derived from a TPM2-sealed random secret + credential name
//! - `host+tpm2` — AES key derived from host secret + TPM2-sealed secret + credential name
//! - `null`      — AES key derived from credential name only (no security, for testing)
//! - `auto`      — prefer host, fall back to tpm2, then error
//! - `auto-initrd` — prefer tpm2, fall back to null
//!
//! TPM2 sealing binds credentials to PCR values (default: PCR 7, Secure Boot
//! policy) via direct communication with `/dev/tpmrm0`.

mod tpm2;

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{AeadCore, Aes256Gcm, Nonce};
use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use clap::{Parser, Subcommand};
use sha2::{Digest, Sha256};
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::{MetadataExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

// ---------------------------------------------------------------------------
// CLI definition
// ---------------------------------------------------------------------------

#[derive(Parser, Debug)]
#[command(
    name = "systemd-creds",
    about = "Lists, shows, encrypts and decrypts service credentials",
    version
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,

    /// When used with list/cat, operate on system credentials instead of
    /// the current execution context.
    #[arg(long, global = true)]
    system: bool,

    /// Suppress additional informational output.
    #[arg(short, long, global = true)]
    quiet: bool,

    /// Do not print column headers or footer hints.
    #[arg(long, global = true)]
    no_legend: bool,

    /// Do not pipe output into a pager.
    #[arg(long, global = true)]
    no_pager: bool,

    /// Credential name (used by encrypt/decrypt).
    #[arg(long, global = true)]
    name: Option<String>,

    /// Control trailing newline: auto, yes, no. (global alias for cat/encrypt)
    #[arg(long, global = true)]
    newline: Option<String>,

    /// Transcode output: base64, unbase64, hex, unhex. (global alias for cat)
    #[arg(long, global = true)]
    transcode: Option<String>,

    /// JSON output format: pretty, short, off. (global alias for list)
    #[arg(long, global = true)]
    json: Option<String>,

    /// Encryption key type: host, tpm2, host+tpm2, null, auto, auto-initrd.
    #[arg(long, global = true, default_value = "auto")]
    with_key: String,

    /// Shortcut for --with-key=host.
    #[arg(short = 'H', long, global = true)]
    host: bool,

    /// Shortcut for --with-key=tpm2.
    #[arg(short = 'T', long, global = true)]
    tpm2: bool,

    /// PCR indices to bind TPM2-sealed credentials to.
    #[arg(long, global = true, default_value = "7")]
    tpm2_pcrs: String,

    /// Timestamp to embed (microseconds since epoch, or "now").
    #[arg(long, global = true)]
    timestamp: Option<String>,

    /// Expiry timestamp (0 or empty = never).
    #[arg(long, global = true)]
    not_after: Option<String>,

    /// Show output as SetCredentialEncrypted= line.
    #[arg(short = 'p', long, global = true)]
    pretty: bool,

    /// Allow decrypting null-key credentials even on secure-boot systems.
    #[arg(long, global = true)]
    allow_null: bool,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// List credentials passed into the current execution context.
    List {
        /// Output format: pretty, short, off.
        #[arg(long, default_value = "off")]
        json: String,
    },

    /// Show contents of specified credentials.
    Cat {
        /// Credential names to display.
        #[arg(required = true)]
        credentials: Vec<String>,

        /// Transcode output: base64, unbase64, hex, unhex.
        #[arg(long)]
        transcode: Option<String>,

        /// Control trailing newline: auto, yes, no.
        #[arg(long, default_value = "auto")]
        newline: String,
    },

    /// Generate a host encryption key for credentials.
    Setup,

    /// Encrypt a credential.
    Encrypt {
        /// Input file path or "-" for stdin.
        input: String,

        /// Output file path or "-" for stdout.
        output: String,
    },

    /// Decrypt an encrypted credential.
    Decrypt {
        /// Input file path or "-" for stdin.
        input: String,

        /// Output file path or "-" for stdout (default).
        output: Option<String>,

        /// Transcode output: base64, unbase64, hex, unhex.
        #[arg(long)]
        transcode: Option<String>,

        /// Control trailing newline: auto, yes, no.
        #[arg(long, default_value = "auto")]
        newline: String,
    },

    /// Report whether a TPM2 device is available.
    #[command(name = "has-tpm2")]
    HasTpm2,
}

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Path to the host encryption key.
const HOST_KEY_PATH: &str = "/var/lib/systemd/credential.secret";

/// Size of the host key in bytes.
const HOST_KEY_SIZE: usize = 256;

/// Path for system-level credentials (container pass-through).
const SYSTEM_CREDENTIALS_DIR: &str = "/run/credentials/@system";

/// Magic bytes identifying an encrypted credential blob (before base64).
/// "sHc\0" — matches systemd's CRED_MAGIC.
const CRED_MAGIC: [u8; 4] = [0x73, 0x48, 0x63, 0x00];

/// AES-256-GCM IV (nonce) size.
const AES_IV_SIZE: usize = 12;

/// AES-256-GCM authentication tag size.
const AES_TAG_SIZE: usize = 16;

// Sealing types (stored in the credential header).
const SEAL_NULL: u32 = 0;
const SEAL_HOST: u32 = 1;
const SEAL_TPM2: u32 = 2;
const SEAL_HOST_TPM2: u32 = 3;

// ---------------------------------------------------------------------------
// Credential header wire format
// ---------------------------------------------------------------------------
// All integers are little-endian.
//
// Offset  Size   Field
// ------  ----   -----
//  0       4     magic ("sHc\0")
//  4       4     seal_type (le32)
//  8       8     timestamp (le64, usec since epoch)
// 16       8     not_after (le64, usec since epoch, 0=never)
// 24       4     name_len (le32)
// 28       N     name (UTF-8, not NUL-terminated)
// 28+N    12     iv (AES-GCM nonce)
// 40+N    ...    ciphertext (includes 16-byte GCM tag appended by aes-gcm)
//
// The whole blob is then base64-encoded for storage/transport.

const HEADER_FIXED_SIZE: usize = 4 + 4 + 8 + 8 + 4; // 28 bytes

// ---------------------------------------------------------------------------
// Helper functions
// ---------------------------------------------------------------------------

/// Get the credentials directory for the current context.
fn credentials_dir(system: bool) -> Option<PathBuf> {
    if system {
        let p = PathBuf::from(SYSTEM_CREDENTIALS_DIR);
        if p.is_dir() { Some(p) } else { None }
    } else {
        std::env::var("CREDENTIALS_DIRECTORY")
            .ok()
            .filter(|s| !s.is_empty())
            .map(PathBuf::from)
    }
}

/// Get the encrypted credentials directory for the current context.
fn encrypted_credentials_dir() -> Option<PathBuf> {
    std::env::var("ENCRYPTED_CREDENTIALS_DIRECTORY")
        .ok()
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .filter(|p| p.is_dir())
}

/// Read the host key from disk, or return an error message.
fn read_host_key() -> Result<Vec<u8>, String> {
    fs::read(HOST_KEY_PATH).map_err(|e| {
        format!(
            "Failed to read host key from {HOST_KEY_PATH}: {e}\n\
             Hint: Run 'systemd-creds setup' first to generate a host key."
        )
    })
}

/// Derive an AES-256 key from a secret and credential name.
///
/// key = SHA-256(secret || credential_name_bytes)
fn derive_key(secret: &[u8], cred_name: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(secret);
    hasher.update(cred_name.as_bytes());
    hasher.finalize().into()
}

/// Derive an AES-256 key from host key, TPM2 secret, and credential name.
///
/// key = SHA-256(host_key || tpm2_secret || credential_name_bytes)
fn derive_host_tpm2_key(host_key: &[u8], tpm2_secret: &[u8], cred_name: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(host_key);
    hasher.update(tpm2_secret);
    hasher.update(cred_name.as_bytes());
    hasher.finalize().into()
}

/// Derive a fixed zero-length AES-256 key for null-sealed credentials.
/// key = SHA-256("") — effectively a well-known constant.
fn derive_null_key(cred_name: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(cred_name.as_bytes());
    hasher.finalize().into()
}

/// Get current time as microseconds since the Unix epoch.
/// Global timestamp override for testing/CLI. When non-zero, `now_usec()` returns this value.
static TIMESTAMP_OVERRIDE: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn now_usec() -> u64 {
    let ovr = TIMESTAMP_OVERRIDE.load(std::sync::atomic::Ordering::Relaxed);
    if ovr != 0 {
        return ovr;
    }
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_micros() as u64
}

/// Read all data from a file path or stdin ("-").
fn read_input(path: &str) -> Result<Vec<u8>, String> {
    if path == "-" {
        let mut buf = Vec::new();
        io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("Failed to read from stdin: {e}"))?;
        Ok(buf)
    } else {
        fs::read(path).map_err(|e| format!("Failed to read {path:?}: {e}"))
    }
}

/// Write data to a file path or stdout ("-").
fn write_output(path: &str, data: &[u8]) -> Result<(), String> {
    if path == "-" {
        io::stdout()
            .write_all(data)
            .map_err(|e| format!("Failed to write to stdout: {e}"))
    } else {
        fs::write(path, data).map_err(|e| format!("Failed to write {path:?}: {e}"))
    }
}

/// Derive the credential name from a file path's filename component.
fn name_from_path(path: &str) -> Option<String> {
    if path == "-" {
        return None;
    }
    Path::new(path)
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        // Strip common extensions used for credential files.
        .map(|n| n.strip_suffix(".cred").unwrap_or(&n).to_string())
}

/// Classify the security state of a credential file for `list` output.
fn security_state(path: &Path) -> &'static str {
    // Check if the backing filesystem is ramfs/tmpfs (secure, non-swappable).
    // We approximate this by checking if the path is under /run (tmpfs).
    let is_tmpfs = path.to_string_lossy().starts_with("/run");

    let mode = match fs::metadata(path) {
        Ok(m) => m.mode() & 0o777,
        Err(_) => return "insecure",
    };

    if mode != 0o400 {
        "insecure"
    } else if is_tmpfs {
        "secure"
    } else {
        "weak"
    }
}

/// Check whether a TPM2 device is available.
fn tpm2_available() -> bool {
    tpm2::is_tpm2_available()
}

/// Check whether we're running in a container (simple heuristic).
fn in_container() -> bool {
    // systemd-detect-virt --container
    if let Ok(content) = fs::read_to_string("/run/systemd/container") {
        return !content.trim().is_empty();
    }
    // Check for /.dockerenv or /run/.containerenv
    Path::new("/.dockerenv").exists() || Path::new("/run/.containerenv").exists()
}

/// Transcode data according to the specified mode.
fn transcode(data: &[u8], mode: &str) -> Result<Vec<u8>, String> {
    match mode {
        "no" | "0" | "false" => Ok(data.to_vec()),
        "base64" => Ok(BASE64.encode(data).into_bytes()),
        "unbase64" => {
            let s = String::from_utf8_lossy(data);
            let cleaned: String = s.chars().filter(|c| !c.is_whitespace()).collect();
            BASE64
                .decode(&cleaned)
                .map_err(|e| format!("Failed to decode base64: {e}"))
        }
        "hex" => Ok(hex_encode(data).into_bytes()),
        "unhex" => {
            let s = String::from_utf8_lossy(data);
            let cleaned: String = s.chars().filter(|c| c.is_ascii_hexdigit()).collect();
            hex_decode(&cleaned)
        }
        other => Err(format!(
            "Unknown transcode mode: {other:?}\nSupported: base64, unbase64, hex, unhex"
        )),
    }
}

fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{b:02x}")).collect()
}

fn hex_decode(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("Hex string has odd length".to_string());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| format!("Invalid hex at offset {i}: {e}"))
        })
        .collect()
}

// ---------------------------------------------------------------------------
// Encrypt / Decrypt
// ---------------------------------------------------------------------------

/// Inner encryption function that accepts pre-computed TPM2 data.
///
/// This is separated from `encrypt_credential` so that tests can supply
/// synthetic TPM2 blobs without requiring real TPM2 hardware.
///
/// `tpm2_blob` — if `Some`, the `(plaintext_secret, sealed_blob)` pair is
/// embedded directly into the credential. If `None` and the seal type
/// requires TPM2, the caller must have already handled it (see
/// `encrypt_credential`).
fn encrypt_credential_inner(
    plaintext: &[u8],
    cred_name: &str,
    seal_type: u32,
    timestamp: u64,
    not_after: u64,
    host_key: Option<&[u8]>,
    tpm2_blob: Option<(Vec<u8>, tpm2::Tpm2SealedBlob)>,
) -> Result<Vec<u8>, String> {
    // Derive the encryption key.
    let aes_key = match seal_type {
        SEAL_NULL => derive_null_key(cred_name),
        SEAL_HOST => {
            let hk = host_key.ok_or_else(|| "Host key required for SEAL_HOST".to_string())?;
            derive_key(hk, cred_name)
        }
        SEAL_TPM2 => {
            let (tpm2_secret, _) = tpm2_blob
                .as_ref()
                .ok_or_else(|| "TPM2 blob required for SEAL_TPM2".to_string())?;
            derive_key(tpm2_secret, cred_name)
        }
        SEAL_HOST_TPM2 => {
            let hk = host_key.ok_or_else(|| "Host key required for SEAL_HOST_TPM2".to_string())?;
            let (tpm2_secret, _) = tpm2_blob
                .as_ref()
                .ok_or_else(|| "TPM2 blob required for SEAL_HOST_TPM2".to_string())?;
            derive_host_tpm2_key(hk, tpm2_secret, cred_name)
        }
        _ => {
            return Err(format!("Unsupported seal type: {seal_type}"));
        }
    };

    let cipher = Aes256Gcm::new_from_slice(&aes_key)
        .map_err(|e| format!("Failed to initialize AES-256-GCM: {e}"))?;

    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);

    // Encrypt (ciphertext includes the 16-byte GCM tag appended).
    let ciphertext = cipher
        .encrypt(&nonce, plaintext)
        .map_err(|e| format!("Encryption failed: {e}"))?;

    // Build the wire-format blob.
    let name_bytes = cred_name.as_bytes();
    let name_len = name_bytes.len() as u32;

    let tpm2_data = tpm2_blob.as_ref().map(|(_, b)| b.serialize());
    let tpm2_data_len = tpm2_data.as_ref().map_or(0, |d| d.len());

    let total_size =
        HEADER_FIXED_SIZE + name_bytes.len() + tpm2_data_len + AES_IV_SIZE + ciphertext.len();
    let mut blob = Vec::with_capacity(total_size);

    // Header
    blob.extend_from_slice(&CRED_MAGIC);
    blob.extend_from_slice(&seal_type.to_le_bytes());
    blob.extend_from_slice(&timestamp.to_le_bytes());
    blob.extend_from_slice(&not_after.to_le_bytes());
    blob.extend_from_slice(&name_len.to_le_bytes());
    blob.extend_from_slice(name_bytes);

    // TPM2 sealed blob (only for TPM2 and host+tpm2 modes)
    if let Some(ref tpm2_data) = tpm2_data {
        blob.extend_from_slice(tpm2_data);
    }

    // IV
    blob.extend_from_slice(nonce.as_slice());

    // Ciphertext (with appended GCM tag)
    blob.extend_from_slice(&ciphertext);

    Ok(blob)
}

/// Encrypt plaintext into our credential wire format (before base64 encoding).
///
/// For `SEAL_TPM2` and `SEAL_HOST_TPM2`, a random 32-byte secret is generated,
/// sealed to the TPM2, and the sealed blob is embedded in the credential. The
/// AES key is derived from the TPM2 secret (and optionally the host key).
fn encrypt_credential(
    plaintext: &[u8],
    cred_name: &str,
    seal_type: u32,
    timestamp: u64,
    not_after: u64,
    pcr_mask: u32,
) -> Result<Vec<u8>, String> {
    // For TPM2 modes, generate a random secret and seal it.
    let tpm2_blob = if seal_type == SEAL_TPM2 || seal_type == SEAL_HOST_TPM2 {
        let tpm2_secret = generate_random_bytes(32);
        let blob = tpm2::tpm2_seal_secret(
            &tpm2_secret,
            pcr_mask,
            tpm2::TPM2_ALG_SHA256,
            tpm2::TPM2_ALG_ECC,
        )?;
        Some((tpm2_secret, blob))
    } else {
        None
    };

    // Read host key if needed.
    let host_key = if seal_type == SEAL_HOST || seal_type == SEAL_HOST_TPM2 {
        Some(read_host_key()?)
    } else {
        None
    };

    encrypt_credential_inner(
        plaintext,
        cred_name,
        seal_type,
        timestamp,
        not_after,
        host_key.as_deref(),
        tpm2_blob,
    )
}

/// Parsed credential header and extracted TPM2 blob (if present).
///
/// Used by `decrypt_credential` and exposed for testing the wire-format
/// parsing independently of the actual TPM2 unseal step.
#[derive(Debug)]
struct ParsedCredentialHeader {
    seal_type: u32,
    #[allow(dead_code)]
    timestamp: u64,
    #[allow(dead_code)]
    not_after: u64,
    cred_name: String,
    tpm2_blob: Option<(tpm2::Tpm2SealedBlob, usize)>,
    /// Offset where the IV begins (after header + name + optional TPM2 blob).
    data_start: usize,
}

/// Parse a credential blob header and extract the embedded TPM2 blob (if any)
/// without performing the actual TPM2 unseal.
///
/// This is separated from `decrypt_credential` so that tests can inspect the
/// wire format and verify TPM2 blob embedding without requiring hardware.
fn parse_credential_header(
    blob: &[u8],
    expected_name: Option<&str>,
) -> Result<ParsedCredentialHeader, String> {
    if blob.len() < HEADER_FIXED_SIZE {
        return Err("Credential blob too short for header".to_string());
    }

    // Parse header.
    if blob[0..4] != CRED_MAGIC {
        return Err("Invalid credential magic — not an encrypted credential".to_string());
    }

    let seal_type = u32::from_le_bytes(blob[4..8].try_into().unwrap());
    let timestamp = u64::from_le_bytes(blob[8..16].try_into().unwrap());
    let not_after = u64::from_le_bytes(blob[16..24].try_into().unwrap());
    let name_len = u32::from_le_bytes(blob[24..28].try_into().unwrap()) as usize;

    let name_end = HEADER_FIXED_SIZE + name_len;
    if blob.len() < name_end + AES_IV_SIZE {
        return Err("Credential blob too short for name + IV".to_string());
    }

    let cred_name = std::str::from_utf8(&blob[HEADER_FIXED_SIZE..name_end])
        .map_err(|e| format!("Invalid UTF-8 in credential name: {e}"))?
        .to_string();

    // Validate credential name if requested.
    if let Some(expected) = expected_name
        && !expected.is_empty()
        && !cred_name.is_empty()
        && expected != cred_name
    {
        return Err(format!(
            "Credential name mismatch: expected {expected:?}, got {:?}",
            cred_name
        ));
    }

    // Check expiry.
    if not_after != 0 {
        let now = now_usec();
        if now > not_after {
            return Err(format!(
                "Credential has expired (not-after: {not_after} µs, now: {now} µs)"
            ));
        }
    }

    // For TPM2 modes, extract (but do not unseal) the TPM2 blob.
    let (tpm2_blob, data_start) = if seal_type == SEAL_TPM2 || seal_type == SEAL_HOST_TPM2 {
        let tpm2_data = &blob[name_end..];
        let (parsed_blob, consumed) = tpm2::Tpm2SealedBlob::deserialize(tpm2_data)
            .map_err(|e| format!("Failed to parse TPM2 blob: {e}"))?;
        (Some((parsed_blob, consumed)), name_end + consumed)
    } else {
        (None, name_end)
    };

    Ok(ParsedCredentialHeader {
        seal_type,
        timestamp,
        not_after,
        cred_name,
        tpm2_blob,
        data_start,
    })
}

/// Inner decryption function that accepts a pre-provided TPM2 secret.
///
/// This allows testing the decryption path with synthetic TPM2 data
/// without requiring real TPM2 hardware.
fn decrypt_credential_inner(
    blob: &[u8],
    expected_name: Option<&str>,
    allow_null: bool,
    host_key: Option<&[u8]>,
    tpm2_secret: Option<&[u8]>,
) -> Result<(Vec<u8>, String), String> {
    let header = parse_credential_header(blob, expected_name)?;

    // Extract IV and ciphertext.
    if blob.len() < header.data_start + AES_IV_SIZE {
        return Err("Credential blob too short for IV".to_string());
    }
    let iv_start = header.data_start;
    let iv_end = iv_start + AES_IV_SIZE;
    let iv = &blob[iv_start..iv_end];
    let ciphertext = &blob[iv_end..];

    if ciphertext.len() < AES_TAG_SIZE {
        return Err("Credential blob too short for ciphertext + tag".to_string());
    }

    // Derive the decryption key.
    let aes_key = match header.seal_type {
        SEAL_NULL => {
            if !allow_null && is_secure_boot_enabled() {
                return Err(
                    "Refusing to decrypt null-sealed credential on a Secure Boot system.\n\
                     Use --allow-null to override."
                        .to_string(),
                );
            }
            derive_null_key(&header.cred_name)
        }
        SEAL_HOST => {
            let hk =
                host_key.ok_or_else(|| "Host key required for SEAL_HOST decryption".to_string())?;
            derive_key(hk, &header.cred_name)
        }
        SEAL_TPM2 => {
            let ts = tpm2_secret
                .ok_or_else(|| "TPM2 secret required for SEAL_TPM2 decryption".to_string())?;
            derive_key(ts, &header.cred_name)
        }
        SEAL_HOST_TPM2 => {
            let hk = host_key
                .ok_or_else(|| "Host key required for SEAL_HOST_TPM2 decryption".to_string())?;
            let ts = tpm2_secret
                .ok_or_else(|| "TPM2 secret required for SEAL_HOST_TPM2 decryption".to_string())?;
            derive_host_tpm2_key(hk, ts, &header.cred_name)
        }
        other => {
            return Err(format!("Unknown seal type: {other}"));
        }
    };

    let cipher = Aes256Gcm::new_from_slice(&aes_key)
        .map_err(|e| format!("Failed to initialize AES-256-GCM: {e}"))?;

    let nonce = Nonce::from_slice(iv);

    let plaintext = cipher.decrypt(nonce, ciphertext).map_err(|_| {
        "Decryption failed: authentication tag mismatch.\n\
             The credential may have been encrypted with a different host key, \
             or the data may be corrupted."
            .to_string()
    })?;

    Ok((plaintext, header.cred_name))
}

/// Decrypt a credential blob (after base64 decoding).
///
/// For `SEAL_TPM2` and `SEAL_HOST_TPM2` credentials, the TPM2 sealed blob is
/// read from the credential, unsealed via the TPM2, and used to derive the
/// decryption key.
fn decrypt_credential(
    blob: &[u8],
    expected_name: Option<&str>,
    allow_null: bool,
) -> Result<(Vec<u8>, String), String> {
    // Parse header to determine seal type and extract TPM2 blob.
    let header = parse_credential_header(blob, expected_name)?;

    // For TPM2 modes, unseal the TPM2 blob.
    let tpm2_secret = if header.seal_type == SEAL_TPM2 || header.seal_type == SEAL_HOST_TPM2 {
        let (tpm2_blob, _consumed) = header
            .tpm2_blob
            .as_ref()
            .ok_or_else(|| "No TPM2 blob found in credential".to_string())?;
        let secret =
            tpm2::tpm2_unseal_secret(tpm2_blob).map_err(|e| format!("TPM2 unseal failed: {e}"))?;
        Some(secret)
    } else {
        None
    };

    // Read host key if needed.
    let host_key = if header.seal_type == SEAL_HOST || header.seal_type == SEAL_HOST_TPM2 {
        Some(read_host_key()?)
    } else {
        None
    };

    decrypt_credential_inner(
        blob,
        expected_name,
        allow_null,
        host_key.as_deref(),
        tpm2_secret.as_deref(),
    )
}

/// Simple check for UEFI Secure Boot state.
fn is_secure_boot_enabled() -> bool {
    // Check the EFI variable (SecureBoot-8be4df61-...)
    let path = "/sys/firmware/efi/efivars/SecureBoot-8be4df61-93ca-11d2-aa0d-00e098032b8c";
    if let Ok(data) = fs::read(path) {
        // The variable is 5 bytes: 4-byte attributes + 1-byte value.
        // Value of 1 means Secure Boot is enabled.
        if data.len() >= 5 {
            return data[4] == 1;
        }
    }
    false
}

// ---------------------------------------------------------------------------
// Command implementations
// ---------------------------------------------------------------------------

/// `systemd-creds list`
fn cmd_list(system: bool, quiet: bool, no_legend: bool, json_mode: &str) {
    let dir = credentials_dir(system);
    let enc_dir = if !system {
        encrypted_credentials_dir()
    } else {
        None
    };

    if dir.is_none() && enc_dir.is_none() {
        if system {
            if !no_legend && !quiet {
                println!("No credentials passed to system.");
            }
            return;
        }
        if !quiet {
            eprintln!(
                "No credentials directory set.\n\
                 Hint: $CREDENTIALS_DIRECTORY is not set. This command is intended \
                 to be run from within a service context."
            );
        }
        process::exit(1);
    }

    let mut entries: Vec<(String, u64, String)> = Vec::new();
    let mut seen_names: std::collections::HashSet<String> = std::collections::HashSet::new();

    // Read plain credentials.
    if let Some(ref d) = dir
        && d.is_dir()
        && let Ok(rd) = fs::read_dir(d)
    {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let security = security_state(&path).to_string();
            seen_names.insert(name.clone());
            entries.push((name, size, security));
        }
    }

    // Read encrypted credentials.
    if let Some(ref d) = enc_dir
        && d.is_dir()
        && let Ok(rd) = fs::read_dir(d)
    {
        for entry in rd.flatten() {
            let path = entry.path();
            if !path.is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if seen_names.contains(&name) {
                continue; // Plain credential takes precedence
            }
            let size = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
            let security = security_state(&path).to_string();
            entries.push((name, size, security));
        }
    }

    entries.sort_by(|a, b| a.0.cmp(&b.0));

    match json_mode {
        "pretty" | "short" => {
            let json_entries: Vec<String> = entries
                .iter()
                .map(|(name, size, security)| {
                    if json_mode == "pretty" {
                        format!(
                            "  {{\n    \"name\" : \"{}\",\n    \"size\" : {},\n    \"secure\" : \"{}\"\n  }}",
                            name, size, security
                        )
                    } else {
                        format!(
                            "{{\"name\":\"{}\",\"size\":{},\"secure\":\"{}\"}}",
                            name, size, security
                        )
                    }
                })
                .collect();
            if json_mode == "pretty" {
                println!("[\n{}\n]", json_entries.join(",\n"));
            } else {
                println!("[{}]", json_entries.join(","));
            }
        }
        _ => {
            // Default table output
            if !no_legend && !entries.is_empty() {
                println!("{:<40} {:>10} SECURE", "NAME", "SIZE");
            }

            for (name, size, security) in &entries {
                println!("{name:<40} {size:>10} {security}");
            }

            if !no_legend {
                if entries.is_empty() && system {
                    println!("No credentials.");
                } else {
                    println!("\n{} credentials listed.", entries.len());
                }
            }
        }
    }
}

/// `systemd-creds cat`
fn cmd_cat(
    credentials: &[String],
    system: bool,
    quiet: bool,
    transcode_mode: Option<&str>,
    newline: &str,
    global_name: Option<&str>,
    json_mode: Option<&str>,
) {
    let dir = credentials_dir(system);
    let enc_dir = if !system {
        encrypted_credentials_dir()
    } else {
        None
    };

    if dir.is_none() && enc_dir.is_none() {
        if !quiet {
            if system {
                eprintln!("No system credentials directory found ({SYSTEM_CREDENTIALS_DIR}).");
            } else {
                eprintln!(
                    "No credentials directory set.\n\
                     Hint: $CREDENTIALS_DIRECTORY is not set."
                );
            }
        }
        process::exit(1);
    }

    let mut failed = false;

    for cred_name in credentials {
        // Try plain credentials directory first.
        let plain_path = dir.as_ref().map(|d| d.join(cred_name));
        let found_plain = plain_path.as_ref().is_some_and(|p| p.is_file());

        // Try encrypted credentials directory.
        let enc_path = enc_dir.as_ref().map(|d| d.join(cred_name));
        let found_enc = !found_plain && enc_path.as_ref().is_some_and(|p| p.is_file());

        if !found_plain && !found_enc {
            eprintln!("Credential {cred_name:?} not found.");
            failed = true;
            continue;
        }

        let mut data = if found_plain {
            match fs::read(plain_path.as_ref().unwrap()) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Failed to read credential {cred_name:?}: {e}");
                    failed = true;
                    continue;
                }
            }
        } else {
            // Encrypted credential — read and decrypt.
            let enc_data = match fs::read(enc_path.as_ref().unwrap()) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Failed to read encrypted credential {cred_name:?}: {e}");
                    failed = true;
                    continue;
                }
            };
            // The file is base64-encoded.
            let blob = match BASE64.decode(
                enc_data
                    .iter()
                    .copied()
                    .filter(|b| !b.is_ascii_whitespace())
                    .collect::<Vec<u8>>(),
            ) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("Failed to decode encrypted credential {cred_name:?}: {e}");
                    failed = true;
                    continue;
                }
            };
            // Use global --name if provided, otherwise the credential filename.
            let decrypt_name = global_name.filter(|n| !n.is_empty()).unwrap_or(cred_name);
            match decrypt_credential(&blob, Some(decrypt_name), true) {
                Ok((plaintext, _)) => plaintext,
                Err(e) => {
                    // Retry without name constraint (for unnamed credentials).
                    match decrypt_credential(&blob, None, true) {
                        Ok((plaintext, _)) => plaintext,
                        Err(_) => {
                            eprintln!("Failed to decrypt credential {cred_name:?}: {e}");
                            failed = true;
                            continue;
                        }
                    }
                }
            }
        };

        if let Some(mode) = transcode_mode {
            data = match transcode(&data, mode) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("Failed to transcode credential {cred_name:?}: {e}");
                    failed = true;
                    continue;
                }
            };
        }

        // Handle --json=short/pretty: parse credential as JSON and re-serialize.
        if let Some(jm) = json_mode
            && (jm == "short" || jm == "pretty")
        {
            let text = String::from_utf8_lossy(&data);
            match serde_json::from_str::<serde_json::Value>(text.as_ref()) {
                Ok(val) => {
                    let serialized = if jm == "pretty" {
                        serde_json::to_string_pretty(&val).unwrap()
                    } else {
                        serde_json::to_string(&val).unwrap()
                    };
                    data = format!("{serialized}\n").into_bytes();
                }
                Err(e) => {
                    eprintln!("Failed to parse credential {cred_name:?} as JSON: {e}");
                    failed = true;
                    continue;
                }
            }
        }

        let stdout = io::stdout();
        let mut out = stdout.lock();
        if out.write_all(&data).is_err() {
            break;
        }

        // Handle trailing newline.
        // --newline only takes effect when stdout is a tty; when piped,
        // no newline is ever appended regardless of the setting.
        let is_tty = unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 };
        if is_tty {
            let needs_newline = match newline {
                "yes" => true,
                "no" => false,
                // "auto": add newline if data doesn't already end with one.
                _ => !data.ends_with(b"\n"),
            };
            if needs_newline {
                let _ = out.write_all(b"\n");
            }
        }
    }

    if failed {
        process::exit(1);
    }
}

/// `systemd-creds setup`
fn cmd_setup(quiet: bool) {
    let path = Path::new(HOST_KEY_PATH);

    if path.exists() {
        if !quiet {
            println!("Host key already exists at {HOST_KEY_PATH}.");
        }
        return;
    }

    // Ensure parent directory exists.
    if let Some(parent) = path.parent()
        && let Err(e) = fs::create_dir_all(parent)
    {
        eprintln!("Failed to create directory {parent:?}: {e}");
        process::exit(1);
    }

    // Generate random key.
    let key = generate_random_bytes(HOST_KEY_SIZE);

    // Write atomically: write to temp file, set permissions, rename.
    let tmp_path = format!("{HOST_KEY_PATH}.tmp.{}", std::process::id());
    if let Err(e) = fs::write(&tmp_path, &key) {
        eprintln!("Failed to write host key to {tmp_path:?}: {e}");
        process::exit(1);
    }

    // Set permissions to 0o400 (owner read only).
    if let Err(e) = fs::set_permissions(&tmp_path, fs::Permissions::from_mode(0o400)) {
        eprintln!("Failed to set permissions on {tmp_path:?}: {e}");
        let _ = fs::remove_file(&tmp_path);
        process::exit(1);
    }

    // Rename into place.
    if let Err(e) = fs::rename(&tmp_path, HOST_KEY_PATH) {
        eprintln!("Failed to rename {tmp_path:?} to {HOST_KEY_PATH}: {e}");
        let _ = fs::remove_file(&tmp_path);
        process::exit(1);
    }

    if !quiet {
        println!("Created host key at {HOST_KEY_PATH} ({HOST_KEY_SIZE} bytes).");
    }
}

/// Parse a comma-separated list of PCR indices into a bitmask.
fn parse_pcr_mask(pcrs: &str) -> Result<u32, String> {
    let mut mask = 0u32;
    for part in pcrs.split(',') {
        let part = part.trim();
        if part.is_empty() {
            continue;
        }
        let idx: u32 = part
            .parse()
            .map_err(|e| format!("Invalid PCR index {part:?}: {e}"))?;
        if idx >= 24 {
            return Err(format!("PCR index {idx} out of range (0-23)"));
        }
        mask |= 1 << idx;
    }
    if mask == 0 {
        return Err("No PCR indices specified".into());
    }
    Ok(mask)
}

/// `systemd-creds encrypt`
#[allow(clippy::too_many_arguments)]
fn cmd_encrypt(
    input: &str,
    output: &str,
    name: Option<&str>,
    with_key: &str,
    tpm2_pcrs: &str,
    timestamp: Option<&str>,
    not_after: Option<&str>,
    pretty: bool,
    quiet: bool,
) {
    // Determine the credential name.
    let cred_name = match name {
        Some(n) => n.to_string(),
        None => match name_from_path(output) {
            Some(n) => n,
            None => {
                eprintln!(
                    "Cannot determine credential name from output path.\n\
                     Use --name= to specify it explicitly."
                );
                process::exit(1);
            }
        },
    };

    // Determine seal type.
    let seal_type = resolve_seal_type(with_key);

    // If seal type requires the host key, ensure it exists (auto-setup).
    if seal_type == SEAL_HOST && !Path::new(HOST_KEY_PATH).exists() {
        if !quiet {
            eprintln!("Host key not found, generating...");
        }
        cmd_setup(true);
    }

    // Read plaintext.
    let plaintext = match read_input(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    // Timestamps.
    let ts = match timestamp {
        Some(s) => parse_timestamp_usec(s),
        None => now_usec(),
    };
    let na = match not_after {
        Some("0") => 0,
        Some(s) => parse_timestamp_usec(s),
        None => 0,
    };

    // Parse PCR mask for TPM2 modes (default is PCR 7 = Secure Boot policy).
    let pcr_mask = if seal_type == SEAL_TPM2 || seal_type == SEAL_HOST_TPM2 {
        match parse_pcr_mask(tpm2_pcrs) {
            Ok(m) => m,
            Err(e) => {
                eprintln!("Invalid --tpm2-pcrs: {e}");
                process::exit(1);
            }
        }
    } else {
        tpm2::DEFAULT_PCR_MASK // unused for non-TPM2 modes, but keeps a sensible default
    };

    // Encrypt.
    let blob = match encrypt_credential(&plaintext, &cred_name, seal_type, ts, na, pcr_mask) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("Encryption failed: {e}");
            process::exit(1);
        }
    };

    // Base64-encode the blob.
    let b64 = BASE64.encode(&blob);

    if pretty && output == "-" {
        // Output as a unit-file–pasteable SetCredentialEncrypted= line.
        println!("SetCredentialEncrypted={cred_name}: \\");
        // Wrap at 80 chars with continuation backslashes.
        let indent = "        ";
        let wrap_width = 80 - indent.len() - 2; // -2 for " \"
        let chars: Vec<char> = b64.chars().collect();
        let chunks: Vec<String> = chars
            .chunks(wrap_width)
            .map(|c| c.iter().collect::<String>())
            .collect();
        for (i, chunk) in chunks.iter().enumerate() {
            if i < chunks.len() - 1 {
                println!("{indent}{chunk} \\");
            } else {
                println!("{indent}{chunk}");
            }
        }
    } else {
        // Write the base64 output.
        if let Err(e) = write_output(output, b64.as_bytes()) {
            eprintln!("{e}");
            process::exit(1);
        }
        // Add a trailing newline for file output.
        if output != "-" {
            // Already written as complete bytes.
        } else {
            println!();
        }
    }

    if !quiet && output != "-" {
        let type_str = seal_type_name(seal_type);
        eprintln!(
            "Encrypted credential {cred_name:?} ({} bytes plaintext) with {type_str} key.",
            plaintext.len()
        );
    }
}

/// `systemd-creds decrypt`
fn cmd_decrypt(
    input: &str,
    output: Option<&str>,
    name: Option<&str>,
    transcode_mode: Option<&str>,
    newline: &str,
    allow_null: bool,
    quiet: bool,
) {
    // Determine expected credential name.
    let expected_name = match name {
        Some(n) => Some(n.to_string()),
        None => name_from_path(input),
    };

    // Read the base64-encoded credential.
    let b64_data = match read_input(input) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("{e}");
            process::exit(1);
        }
    };

    // Decode base64.
    let b64_str = String::from_utf8_lossy(&b64_data);
    let cleaned: String = b64_str.chars().filter(|c| !c.is_whitespace()).collect();
    let blob = match BASE64.decode(&cleaned) {
        Ok(d) => d,
        Err(e) => {
            eprintln!("Failed to decode base64 credential: {e}");
            process::exit(1);
        }
    };

    // Decrypt.
    let (mut plaintext, cred_name) =
        match decrypt_credential(&blob, expected_name.as_deref(), allow_null) {
            Ok(r) => r,
            Err(e) => {
                eprintln!("{e}");
                process::exit(1);
            }
        };

    // Transcode if requested.
    if let Some(mode) = transcode_mode {
        plaintext = match transcode(&plaintext, mode) {
            Ok(d) => d,
            Err(e) => {
                eprintln!("Failed to transcode: {e}");
                process::exit(1);
            }
        };
    }

    // Write output.
    let out_path = output.unwrap_or("-");
    if let Err(e) = write_output(out_path, &plaintext) {
        eprintln!("{e}");
        process::exit(1);
    }

    // Handle trailing newline.
    // --newline only takes effect when stdout is a tty.
    if out_path == "-" {
        let is_tty = unsafe { libc::isatty(libc::STDOUT_FILENO) == 1 };
        if is_tty {
            let needs_newline = match newline {
                "yes" => true,
                "no" => false,
                _ => !plaintext.ends_with(b"\n"),
            };
            if needs_newline {
                println!();
            }
        }
    }

    if !quiet && out_path != "-" {
        eprintln!(
            "Decrypted credential {cred_name:?} ({} bytes).",
            plaintext.len()
        );
    }
}

/// `systemd-creds has-tpm2`
fn cmd_has_tpm2(quiet: bool) {
    let firmware = tpm2_available();
    let in_ctr = in_container();

    if !quiet {
        if firmware && !in_ctr {
            println!("yes");
            println!("+firmware  (TPM2 device found)");
        } else if firmware && in_ctr {
            println!("yes");
            println!("+firmware  (TPM2 device found, but running in container)");
        } else {
            println!("no");
            println!("-firmware  (no TPM2 device found)");
        }
    }

    if firmware && !in_ctr {
        process::exit(0);
    } else {
        process::exit(1);
    }
}

// ---------------------------------------------------------------------------
// Utilities
// ---------------------------------------------------------------------------

/// Read random bytes from /dev/urandom.
fn generate_random_bytes(len: usize) -> Vec<u8> {
    let mut buf = vec![0u8; len];
    let mut f = fs::File::open("/dev/urandom").expect("Failed to open /dev/urandom");
    io::Read::read_exact(&mut f, &mut buf).expect("Failed to read from /dev/urandom");
    buf
}

/// Resolve the `--with-key` argument to a seal type constant.
fn resolve_seal_type(with_key: &str) -> u32 {
    match with_key {
        "host" => SEAL_HOST,
        "tpm2" => {
            if !tpm2_available() || in_container() {
                eprintln!("TPM2 not available. Use --with-key=host or --with-key=null instead.");
                process::exit(1);
            }
            SEAL_TPM2
        }
        "host+tpm2" => {
            if !tpm2_available() || in_container() {
                eprintln!(
                    "TPM2 not available for host+tpm2 mode. Use --with-key=host or --with-key=null instead."
                );
                process::exit(1);
            }
            SEAL_HOST_TPM2
        }
        "null" => SEAL_NULL,
        "auto" => {
            // Prefer host key if /var/lib/systemd/ is on persistent media.
            // Fall back to null if nothing works.
            if Path::new("/var/lib/systemd").exists() {
                SEAL_HOST
            } else if tpm2_available() && !in_container() {
                SEAL_TPM2
            } else {
                eprintln!(
                    "Cannot determine encryption key: /var/lib/systemd/ not found and \
                     no TPM2 available.\nUse --with-key=null for testing (no security!)."
                );
                process::exit(1);
            }
        }
        "auto-initrd" => {
            if tpm2_available() && !in_container() {
                SEAL_TPM2
            } else {
                SEAL_NULL
            }
        }
        other => {
            eprintln!(
                "Unknown key type: {other:?}\n\
                 Supported: host, tpm2, host+tpm2, null, auto, auto-initrd"
            );
            process::exit(1);
        }
    }
}

/// Human-readable name for a seal type.
fn seal_type_name(seal_type: u32) -> &'static str {
    match seal_type {
        SEAL_NULL => "null",
        SEAL_HOST => "host",
        SEAL_TPM2 => "tpm2",
        SEAL_HOST_TPM2 => "host+tpm2",
        _ => "unknown",
    }
}

/// Parse a timestamp string into microseconds since epoch.
/// Accepts "now", plain integer (microseconds), seconds with "s" suffix,
/// or relative offsets like "+1d", "+2h", "+30m", "+60s".
fn parse_timestamp_usec(s: &str) -> u64 {
    let s = s.trim();
    if s.eq_ignore_ascii_case("now") {
        return now_usec();
    }
    // Relative time: +<N>d, +<N>h, +<N>m, +<N>s (or plain +N = seconds)
    if let Some(rest) = s.strip_prefix('+') {
        let now = now_usec();
        let secs = parse_duration_secs(rest);
        return now + secs * 1_000_000;
    }
    // Negative relative time: reject as invalid for timestamps.
    if s.starts_with('-') {
        eprintln!("Negative relative timestamp not supported: {s:?}");
        process::exit(1);
    }
    if let Some(secs_str) = s.strip_suffix('s')
        && let Ok(secs) = secs_str.parse::<u64>()
    {
        return secs * 1_000_000;
    }
    s.parse::<u64>().unwrap_or_else(|_| {
        eprintln!("Failed to parse timestamp: {s:?}");
        process::exit(1);
    })
}

/// Parse a duration string like "1d", "2h", "30m", "60s", or plain seconds.
fn parse_duration_secs(s: &str) -> u64 {
    if let Some(n) = s.strip_suffix('d') {
        return n.parse::<u64>().unwrap_or(0) * 86400;
    }
    if let Some(n) = s.strip_suffix('h') {
        return n.parse::<u64>().unwrap_or(0) * 3600;
    }
    if let Some(n) = s.strip_suffix('m') {
        return n.parse::<u64>().unwrap_or(0) * 60;
    }
    if let Some(n) = s.strip_suffix('s') {
        return n.parse::<u64>().unwrap_or(0);
    }
    s.parse::<u64>().unwrap_or_else(|_| {
        eprintln!("Failed to parse duration: {s:?}");
        process::exit(1);
    })
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() {
    let cli = Cli::parse();

    // Validate --json values.
    if let Some(ref j) = cli.json {
        match j.as_str() {
            "pretty" | "short" | "off" => {}
            other => {
                eprintln!("Unknown JSON mode: {other:?}");
                process::exit(1);
            }
        }
    }

    // Validate --newline values.
    if let Some(ref n) = cli.newline {
        match n.as_str() {
            "auto" | "yes" | "no" => {}
            other => {
                eprintln!("Unknown newline mode: {other:?}");
                process::exit(1);
            }
        }
    }

    // Validate --transcode values.
    if let Some(ref t) = cli.transcode {
        match t.as_str() {
            "base64" | "unbase64" | "hex" | "unhex" | "no" | "0" | "false" => {}
            other => {
                eprintln!("Unknown transcode mode: {other:?}");
                process::exit(1);
            }
        }
    }

    // Apply --timestamp override for decrypt's expiry checks.
    if let Some(ref ts) = cli.timestamp {
        let usec = parse_timestamp_usec(ts);
        TIMESTAMP_OVERRIDE.store(usec, std::sync::atomic::Ordering::Relaxed);
    }

    let default_list = Command::List {
        json: "off".to_string(),
    };
    let command = cli.command.as_ref().unwrap_or(&default_list);
    match command {
        Command::List { json } => {
            let effective_json = cli.json.as_deref().unwrap_or(json.as_str());
            cmd_list(cli.system, cli.quiet, cli.no_legend, effective_json);
        }

        Command::Cat {
            credentials,
            transcode,
            newline,
        } => {
            // Global --transcode/--newline override subcommand defaults
            let effective_transcode = cli.transcode.as_deref().or(transcode.as_deref());
            let effective_newline = cli.newline.as_deref().unwrap_or(newline.as_str());
            cmd_cat(
                credentials,
                cli.system,
                cli.quiet,
                effective_transcode,
                effective_newline,
                cli.name.as_deref(),
                cli.json.as_deref(),
            );
        }

        Command::Setup => {
            cmd_setup(cli.quiet);
        }

        Command::Encrypt { input, output } => {
            let effective_key = if cli.host {
                "host"
            } else if cli.tpm2 {
                "tpm2"
            } else {
                cli.with_key.as_str()
            };
            cmd_encrypt(
                input,
                output,
                cli.name.as_deref(),
                effective_key,
                &cli.tpm2_pcrs,
                cli.timestamp.as_deref(),
                cli.not_after.as_deref(),
                cli.pretty,
                cli.quiet,
            );
        }

        Command::Decrypt {
            input,
            output,
            transcode,
            newline,
        } => {
            let effective_newline = cli.newline.as_deref().unwrap_or(newline.as_str());
            cmd_decrypt(
                input,
                output.as_deref(),
                cli.name.as_deref(),
                cli.transcode.as_deref().or(transcode.as_deref()),
                effective_newline,
                cli.allow_null,
                cli.quiet,
            );
        }

        Command::HasTpm2 => {
            cmd_has_tpm2(cli.quiet);
        }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Helper: build a synthetic TPM2SealedBlob for testing ----

    fn make_synthetic_tpm2_blob(pcr_mask: u32) -> tpm2::Tpm2SealedBlob {
        tpm2::Tpm2SealedBlob {
            pcr_mask,
            pcr_bank: tpm2::TPM2_ALG_SHA256,
            primary_alg: tpm2::TPM2_ALG_ECC,
            private: vec![0xAA; 64],
            public: vec![0xBB; 48],
        }
    }

    #[test]
    fn test_cred_magic() {
        assert_eq!(&CRED_MAGIC, b"sHc\0");
    }

    #[test]
    fn test_derive_key_deterministic() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let k1 = derive_key(&host_key, "my-credential");
        let k2 = derive_key(&host_key, "my-credential");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_derive_key_different_names_differ() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let k1 = derive_key(&host_key, "credential-a");
        let k2 = derive_key(&host_key, "credential-b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_derive_key_different_keys_differ() {
        let k1_data = vec![0x01u8; HOST_KEY_SIZE];
        let k2_data = vec![0x02u8; HOST_KEY_SIZE];
        let k1 = derive_key(&k1_data, "same-name");
        let k2 = derive_key(&k2_data, "same-name");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_derive_null_key_deterministic() {
        let k1 = derive_null_key("test");
        let k2 = derive_null_key("test");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_derive_null_key_different_names_differ() {
        let k1 = derive_null_key("a");
        let k2 = derive_null_key("b");
        assert_ne!(k1, k2);
    }

    #[test]
    fn test_encrypt_decrypt_null_roundtrip() {
        let plaintext = b"hello, world!";
        let cred_name = "test-cred";
        let timestamp = 1_000_000u64;
        let not_after = 0u64;

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, timestamp, not_after, 0)
            .expect("encryption should succeed");

        let (decrypted, name) =
            decrypt_credential(&blob, Some(cred_name), true).expect("decryption should succeed");

        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_encrypt_decrypt_null_empty_plaintext() {
        let plaintext = b"";
        let cred_name = "empty";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0)
            .expect("encryption should succeed");

        let (decrypted, name) =
            decrypt_credential(&blob, Some(cred_name), true).expect("decryption should succeed");

        assert_eq!(decrypted, plaintext.to_vec());
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_encrypt_decrypt_null_large_payload() {
        let plaintext: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let cred_name = "big-cred";

        let blob = encrypt_credential(&plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0)
            .expect("encryption should succeed");

        let (decrypted, _name) =
            decrypt_credential(&blob, Some(cred_name), true).expect("decryption should succeed");

        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_name_mismatch() {
        let plaintext = b"secret";
        let cred_name = "original-name";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0)
            .expect("encryption should succeed");

        let result = decrypt_credential(&blob, Some("wrong-name"), true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mismatch"));
    }

    #[test]
    fn test_encrypt_decrypt_name_empty_skips_validation() {
        let plaintext = b"secret";
        let cred_name = "some-name";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0)
            .expect("encryption should succeed");

        // Empty expected name → no validation.
        let (decrypted, _name) = decrypt_credential(&blob, Some(""), true)
            .expect("decryption should succeed with empty expected name");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_not_after_expired() {
        let plaintext = b"secret";
        let cred_name = "expiring";
        // Set not_after to 1 µs in the past (epoch + 1).
        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, 0, 1, 0)
            .expect("encryption should succeed");

        let result = decrypt_credential(&blob, Some(cred_name), true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_encrypt_decrypt_not_after_future() {
        let plaintext = b"secret";
        let cred_name = "future";
        // Set not_after far in the future.
        let not_after = now_usec() + 3_600_000_000; // 1 hour from now
        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), not_after, 0)
            .expect("encryption should succeed");

        let (decrypted, _) = decrypt_credential(&blob, Some(cred_name), true)
            .expect("decryption should succeed (not expired)");
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_base64_roundtrip() {
        let plaintext = b"password123";
        let cred_name = "db-password";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0)
            .expect("encryption should succeed");

        // Base64 encode, then decode (simulating file storage).
        let b64 = BASE64.encode(&blob);
        let decoded = BASE64.decode(&b64).expect("base64 decode should succeed");

        let (decrypted, name) =
            decrypt_credential(&decoded, Some(cred_name), true).expect("decryption should succeed");
        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_decrypt_truncated_blob_header() {
        // Too short to contain even the fixed header.
        let blob = vec![0x73, 0x48, 0x63, 0x00, 0x00];
        let result = decrypt_credential(&blob, None, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    #[test]
    fn test_decrypt_bad_magic() {
        let mut blob = vec![0u8; 100];
        blob[0] = 0xFF; // Bad magic
        let result = decrypt_credential(&blob, None, true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("magic"));
    }

    #[test]
    fn test_decrypt_corrupted_ciphertext() {
        let plaintext = b"important data";
        let cred_name = "test";

        let mut blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0)
            .expect("encryption should succeed");

        // Corrupt the last byte (part of the GCM tag).
        let last = blob.len() - 1;
        blob[last] ^= 0xFF;

        let result = decrypt_credential(&blob, Some(cred_name), true);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("authentication tag"));
    }

    #[test]
    fn test_hex_encode() {
        assert_eq!(hex_encode(&[0x00, 0xFF, 0xAB]), "00ffab");
        assert_eq!(hex_encode(&[]), "");
    }

    #[test]
    fn test_hex_decode() {
        assert_eq!(hex_decode("00ffab").unwrap(), vec![0x00, 0xFF, 0xAB]);
        assert_eq!(hex_decode("").unwrap(), vec![]);
        assert!(hex_decode("0").is_err()); // odd length
        assert!(hex_decode("gg").is_err()); // invalid hex
    }

    #[test]
    fn test_transcode_base64() {
        let data = b"Hello, World!";
        let encoded = transcode(data, "base64").unwrap();
        assert_eq!(encoded, b"SGVsbG8sIFdvcmxkIQ==");

        let decoded = transcode(&encoded, "unbase64").unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_transcode_hex() {
        let data = b"\x00\xff\xab";
        let encoded = transcode(data, "hex").unwrap();
        assert_eq!(encoded, b"00ffab");

        let decoded = transcode(&encoded, "unhex").unwrap();
        assert_eq!(decoded, data);
    }

    #[test]
    fn test_transcode_unknown_mode() {
        let result = transcode(b"data", "unknown");
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown transcode mode"));
    }

    #[test]
    fn test_name_from_path_regular() {
        assert_eq!(
            name_from_path("/etc/creds/my-password.cred"),
            Some("my-password".to_string())
        );
    }

    #[test]
    fn test_name_from_path_no_extension() {
        assert_eq!(
            name_from_path("/etc/creds/my-password"),
            Some("my-password".to_string())
        );
    }

    #[test]
    fn test_name_from_path_stdin() {
        assert_eq!(name_from_path("-"), None);
    }

    #[test]
    fn test_name_from_path_just_filename() {
        assert_eq!(
            name_from_path("password.cred"),
            Some("password".to_string())
        );
    }

    #[test]
    fn test_seal_type_name() {
        assert_eq!(seal_type_name(SEAL_NULL), "null");
        assert_eq!(seal_type_name(SEAL_HOST), "host");
        assert_eq!(seal_type_name(SEAL_TPM2), "tpm2");
        assert_eq!(seal_type_name(SEAL_HOST_TPM2), "host+tpm2");
        assert_eq!(seal_type_name(99), "unknown");
    }

    #[test]
    fn test_parse_timestamp_now() {
        let ts = parse_timestamp_usec("now");
        assert!(ts > 0);
    }

    #[test]
    fn test_parse_timestamp_plain_integer() {
        assert_eq!(parse_timestamp_usec("1000000"), 1_000_000);
    }

    #[test]
    fn test_parse_timestamp_seconds_suffix() {
        assert_eq!(parse_timestamp_usec("60s"), 60_000_000);
    }

    #[test]
    fn test_encrypt_different_nonces() {
        // Two encryptions of the same data should produce different blobs
        // because the IV/nonce is random each time.
        let plaintext = b"same data";
        let cred_name = "test";
        let ts = now_usec();

        let blob1 = encrypt_credential(plaintext, cred_name, SEAL_NULL, ts, 0, 0).unwrap();
        let blob2 = encrypt_credential(plaintext, cred_name, SEAL_NULL, ts, 0, 0).unwrap();

        // The blobs should differ (different random nonce).
        assert_ne!(blob1, blob2);

        // But both should decrypt to the same plaintext.
        let (d1, _) = decrypt_credential(&blob1, Some(cred_name), true).unwrap();
        let (d2, _) = decrypt_credential(&blob2, Some(cred_name), true).unwrap();
        assert_eq!(d1, plaintext);
        assert_eq!(d2, plaintext);
    }

    #[test]
    fn test_credential_header_format() {
        let plaintext = b"test";
        let cred_name = "my-cred";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, 42, 100, 0).unwrap();

        // Verify magic.
        assert_eq!(&blob[0..4], &CRED_MAGIC);

        // Verify seal type.
        let seal_type = u32::from_le_bytes(blob[4..8].try_into().unwrap());
        assert_eq!(seal_type, SEAL_NULL);

        // Verify timestamp.
        let timestamp = u64::from_le_bytes(blob[8..16].try_into().unwrap());
        assert_eq!(timestamp, 42);

        // Verify not_after.
        let not_after = u64::from_le_bytes(blob[16..24].try_into().unwrap());
        assert_eq!(not_after, 100);

        // Verify name length.
        let name_len = u32::from_le_bytes(blob[24..28].try_into().unwrap()) as usize;
        assert_eq!(name_len, cred_name.len());

        // Verify name.
        let name = std::str::from_utf8(&blob[28..28 + name_len]).unwrap();
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_encrypt_unicode_credential_name() {
        let plaintext = b"data";
        let cred_name = "cred-日本語";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0).unwrap();
        let (decrypted, name) = decrypt_credential(&blob, Some(cred_name), true).unwrap();
        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_encrypt_binary_plaintext() {
        let plaintext: Vec<u8> = (0..=255).collect();
        let cred_name = "binary";

        let blob = encrypt_credential(&plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0).unwrap();
        let (decrypted, _) = decrypt_credential(&blob, Some(cred_name), true).unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_tpm2_available_check() {
        // Just ensure the function doesn't panic.
        let _ = tpm2_available();
    }

    #[test]
    fn test_in_container_check() {
        // Just ensure the function doesn't panic.
        let _ = in_container();
    }

    #[test]
    fn test_is_secure_boot_check() {
        // Just ensure the function doesn't panic.
        let _ = is_secure_boot_enabled();
    }

    #[test]
    fn test_now_usec_reasonable() {
        let ts = now_usec();
        // Should be after 2020-01-01 and before 2100-01-01.
        assert!(ts > 1_577_836_800_000_000); // 2020-01-01
        assert!(ts < 4_102_444_800_000_000); // 2100-01-01
    }

    #[test]
    fn test_decrypt_no_name_validation() {
        let plaintext = b"data";
        let cred_name = "original";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0).unwrap();

        // None expected name → no validation.
        let (decrypted, name) = decrypt_credential(&blob, None, true).unwrap();
        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_security_state_non_existent_path() {
        let path = Path::new("/nonexistent/path/to/credential");
        assert_eq!(security_state(path), "insecure");
    }

    #[test]
    fn test_encrypt_decrypt_with_empty_name() {
        let plaintext = b"secret";
        let cred_name = "";

        let blob = encrypt_credential(plaintext, cred_name, SEAL_NULL, now_usec(), 0, 0).unwrap();
        let (decrypted, name) = decrypt_credential(&blob, Some(""), true).unwrap();
        assert_eq!(decrypted, plaintext);
        assert_eq!(name, "");
    }

    // ---- derive_host_tpm2_key tests ----

    #[test]
    fn test_derive_host_tpm2_key_deterministic() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let k1 = derive_host_tpm2_key(&host_key, &tpm2_secret, "cred");
        let k2 = derive_host_tpm2_key(&host_key, &tpm2_secret, "cred");
        assert_eq!(k1, k2);
    }

    #[test]
    fn test_derive_host_tpm2_key_differs_from_host_only() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let combined = derive_host_tpm2_key(&host_key, &tpm2_secret, "cred");
        let host_only = derive_key(&host_key, "cred");
        assert_ne!(combined, host_only);
    }

    #[test]
    fn test_derive_host_tpm2_key_differs_from_tpm2_only() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let combined = derive_host_tpm2_key(&host_key, &tpm2_secret, "cred");
        let tpm2_only = derive_key(&tpm2_secret, "cred");
        assert_ne!(combined, tpm2_only);
    }

    #[test]
    fn test_derive_host_tpm2_key_different_secrets_differ() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let s1 = vec![0x01u8; 32];
        let s2 = vec![0x02u8; 32];
        let k1 = derive_host_tpm2_key(&host_key, &s1, "cred");
        let k2 = derive_host_tpm2_key(&host_key, &s2, "cred");
        assert_ne!(k1, k2);
    }

    // ---- parse_pcr_mask tests ----

    #[test]
    fn test_parse_pcr_mask_single() {
        assert_eq!(parse_pcr_mask("7").unwrap(), 1 << 7);
    }

    #[test]
    fn test_parse_pcr_mask_multiple() {
        assert_eq!(
            parse_pcr_mask("0,2,7").unwrap(),
            (1 << 0) | (1 << 2) | (1 << 7)
        );
    }

    #[test]
    fn test_parse_pcr_mask_with_spaces() {
        assert_eq!(parse_pcr_mask(" 7 , 11 ").unwrap(), (1 << 7) | (1 << 11));
    }

    #[test]
    fn test_parse_pcr_mask_out_of_range() {
        assert!(parse_pcr_mask("24").is_err());
        assert!(parse_pcr_mask("99").is_err());
    }

    #[test]
    fn test_parse_pcr_mask_invalid() {
        assert!(parse_pcr_mask("abc").is_err());
    }

    #[test]
    fn test_parse_pcr_mask_empty() {
        assert!(parse_pcr_mask("").is_err());
    }

    #[test]
    fn test_parse_pcr_mask_all_pcrs() {
        let mask = parse_pcr_mask("0,1,2,3,4,5,6,7,8,9,10,11,12,13,14,15,16,17,18,19,20,21,22,23")
            .unwrap();
        assert_eq!(mask, 0x00FFFFFF);
    }

    // ---- SEAL_HOST_TPM2 constant ----

    #[test]
    fn test_seal_host_tpm2_constant() {
        assert_eq!(SEAL_HOST_TPM2, 3);
    }

    // ================================================================
    // TPM2 sealing — encrypt/decrypt roundtrip with synthetic blobs
    // ================================================================

    #[test]
    fn test_encrypt_decrypt_tpm2_roundtrip_synthetic() {
        // Encrypt with SEAL_TPM2 using a synthetic TPM2 blob (no hardware).
        let plaintext = b"tpm2-sealed secret data";
        let cred_name = "tpm2-cred";
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_TPM2,
            now_usec(),
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .expect("TPM2 encryption should succeed");

        // Decrypt with the same synthetic TPM2 secret.
        let (decrypted, name) =
            decrypt_credential_inner(&blob, Some(cred_name), false, None, Some(&tpm2_secret))
                .expect("TPM2 decryption should succeed");

        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_encrypt_decrypt_tpm2_empty_plaintext() {
        let plaintext = b"";
        let cred_name = "tpm2-empty";
        let tpm2_secret = vec![0xFFu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_TPM2,
            now_usec(),
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let (decrypted, _) =
            decrypt_credential_inner(&blob, Some(cred_name), false, None, Some(&tpm2_secret))
                .unwrap();
        assert_eq!(decrypted, plaintext.to_vec());
    }

    #[test]
    fn test_encrypt_decrypt_tpm2_large_payload() {
        let plaintext: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let cred_name = "tpm2-big";
        let tpm2_secret = vec![0x99u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            &plaintext,
            cred_name,
            SEAL_TPM2,
            now_usec(),
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let (decrypted, _) =
            decrypt_credential_inner(&blob, Some(cred_name), false, None, Some(&tpm2_secret))
                .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    #[test]
    fn test_encrypt_decrypt_tpm2_wrong_secret_fails() {
        let plaintext = b"secret";
        let cred_name = "tpm2-wrong";
        let tpm2_secret = vec![0x01u8; 32];
        let wrong_secret = vec![0x02u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_TPM2,
            now_usec(),
            0,
            None,
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        // Decrypt with wrong TPM2 secret should fail (auth tag mismatch).
        let result =
            decrypt_credential_inner(&blob, Some(cred_name), false, None, Some(&wrong_secret));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("authentication tag"));
    }

    #[test]
    fn test_encrypt_decrypt_tpm2_name_mismatch() {
        let plaintext = b"secret";
        let cred_name = "tpm2-original";
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_TPM2,
            now_usec(),
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let result =
            decrypt_credential_inner(&blob, Some("wrong-name"), false, None, Some(&tpm2_secret));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mismatch"));
    }

    #[test]
    fn test_encrypt_decrypt_tpm2_base64_roundtrip() {
        let plaintext = b"tpm2-b64-test";
        let cred_name = "tpm2-b64";
        let tpm2_secret = vec![0xABu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_TPM2,
            now_usec(),
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        // Simulate base64 storage.
        let b64 = BASE64.encode(&blob);
        let decoded = BASE64.decode(&b64).unwrap();

        let (decrypted, name) =
            decrypt_credential_inner(&decoded, Some(cred_name), false, None, Some(&tpm2_secret))
                .unwrap();
        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    // ================================================================
    // host+tpm2 combined mode — encrypt/decrypt roundtrip
    // ================================================================

    #[test]
    fn test_encrypt_decrypt_host_tpm2_roundtrip_synthetic() {
        let plaintext = b"host+tpm2 protected data";
        let cred_name = "combined-cred";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_HOST_TPM2,
            now_usec(),
            0,
            Some(&host_key),
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .expect("host+tpm2 encryption should succeed");

        let (decrypted, name) = decrypt_credential_inner(
            &blob,
            Some(cred_name),
            false,
            Some(&host_key),
            Some(&tpm2_secret),
        )
        .expect("host+tpm2 decryption should succeed");

        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_encrypt_decrypt_host_tpm2_wrong_host_key_fails() {
        let plaintext = b"secret";
        let cred_name = "ht-wrong-host";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let wrong_host_key = vec![0xFFu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_HOST_TPM2,
            now_usec(),
            0,
            Some(&host_key),
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let result = decrypt_credential_inner(
            &blob,
            Some(cred_name),
            false,
            Some(&wrong_host_key),
            Some(&tpm2_secret),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("authentication tag"));
    }

    #[test]
    fn test_encrypt_decrypt_host_tpm2_wrong_tpm2_secret_fails() {
        let plaintext = b"secret";
        let cred_name = "ht-wrong-tpm2";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let wrong_tpm2_secret = vec![0xEEu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_HOST_TPM2,
            now_usec(),
            0,
            Some(&host_key),
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        let result = decrypt_credential_inner(
            &blob,
            Some(cred_name),
            false,
            Some(&host_key),
            Some(&wrong_tpm2_secret),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("authentication tag"));
    }

    #[test]
    fn test_encrypt_decrypt_host_tpm2_missing_host_key_fails() {
        let plaintext = b"secret";
        let cred_name = "ht-no-host";
        let tpm2_secret = vec![0xCDu8; 32];

        // Encrypt requires host key for SEAL_HOST_TPM2.
        let result = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_HOST_TPM2,
            now_usec(),
            0,
            None, // no host key
            Some((tpm2_secret, make_synthetic_tpm2_blob(1 << 7))),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Host key required"));
    }

    #[test]
    fn test_encrypt_decrypt_host_tpm2_missing_tpm2_blob_fails() {
        let plaintext = b"secret";
        let cred_name = "ht-no-tpm2";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];

        // Encrypt requires TPM2 blob for SEAL_HOST_TPM2.
        let result = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_HOST_TPM2,
            now_usec(),
            0,
            Some(&host_key),
            None, // no TPM2 blob
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TPM2 blob required"));
    }

    #[test]
    fn test_encrypt_decrypt_host_tpm2_empty_plaintext() {
        let plaintext = b"";
        let cred_name = "ht-empty";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_HOST_TPM2,
            now_usec(),
            0,
            Some(&host_key),
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let (decrypted, _) = decrypt_credential_inner(
            &blob,
            Some(cred_name),
            false,
            Some(&host_key),
            Some(&tpm2_secret),
        )
        .unwrap();
        assert_eq!(decrypted, plaintext.to_vec());
    }

    #[test]
    fn test_encrypt_decrypt_host_tpm2_large_payload() {
        let plaintext: Vec<u8> = (0..10_000).map(|i| (i % 256) as u8).collect();
        let cred_name = "ht-big";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            &plaintext,
            cred_name,
            SEAL_HOST_TPM2,
            now_usec(),
            0,
            Some(&host_key),
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let (decrypted, _) = decrypt_credential_inner(
            &blob,
            Some(cred_name),
            false,
            Some(&host_key),
            Some(&tpm2_secret),
        )
        .unwrap();
        assert_eq!(decrypted, plaintext);
    }

    // ================================================================
    // TPM2 credential wire format verification
    // ================================================================

    #[test]
    fn test_tpm2_credential_header_seal_type() {
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            b"data",
            "test",
            SEAL_TPM2,
            42,
            0,
            None,
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        // Verify seal type in header.
        let seal_type = u32::from_le_bytes(blob[4..8].try_into().unwrap());
        assert_eq!(seal_type, SEAL_TPM2);
    }

    #[test]
    fn test_host_tpm2_credential_header_seal_type() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            b"data",
            "test",
            SEAL_HOST_TPM2,
            42,
            0,
            Some(&host_key),
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        let seal_type = u32::from_le_bytes(blob[4..8].try_into().unwrap());
        assert_eq!(seal_type, SEAL_HOST_TPM2);
    }

    #[test]
    fn test_tpm2_credential_embeds_blob() {
        let cred_name = "tpm2-embed";
        let tpm2_secret = vec![0x42u8; 32];
        let pcr_mask: u32 = (1 << 7) | (1 << 11);
        let tpm2_blob = make_synthetic_tpm2_blob(pcr_mask);
        let serialized_blob = tpm2_blob.serialize();

        let blob = encrypt_credential_inner(
            b"payload",
            cred_name,
            SEAL_TPM2,
            now_usec(),
            0,
            None,
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        // The serialized TPM2 blob should appear in the credential right
        // after the header + name.
        let name_end = HEADER_FIXED_SIZE + cred_name.len();
        let embedded = &blob[name_end..name_end + serialized_blob.len()];
        assert_eq!(embedded, &serialized_blob);
    }

    #[test]
    fn test_tpm2_credential_blob_larger_than_null() {
        let cred_name = "compare";
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);
        let serialized_size = tpm2_blob.serialize().len();

        let null_blob =
            encrypt_credential_inner(b"data", cred_name, SEAL_NULL, 42, 0, None, None).unwrap();

        let tpm2_cred_blob = encrypt_credential_inner(
            b"data",
            cred_name,
            SEAL_TPM2,
            42,
            0,
            None,
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        // TPM2 credential should be larger by exactly the serialized blob size.
        // (The GCM ciphertext may differ in length by the nonce, but the
        // plaintext is the same, so only the TPM2 blob adds extra bytes.)
        assert!(tpm2_cred_blob.len() > null_blob.len());
        // The difference should be exactly the serialized TPM2 blob size.
        assert_eq!(tpm2_cred_blob.len() - null_blob.len(), serialized_size);
    }

    // ================================================================
    // parse_credential_header tests
    // ================================================================

    #[test]
    fn test_parse_credential_header_null() {
        let blob = encrypt_credential_inner(b"data", "test", SEAL_NULL, 42, 0, None, None).unwrap();

        let header = parse_credential_header(&blob, None).unwrap();
        assert_eq!(header.seal_type, SEAL_NULL);
        assert_eq!(header.timestamp, 42);
        assert_eq!(header.not_after, 0);
        assert_eq!(header.cred_name, "test");
        assert!(header.tpm2_blob.is_none());
        // data_start should be right after header + name.
        assert_eq!(header.data_start, HEADER_FIXED_SIZE + "test".len());
    }

    #[test]
    fn test_parse_credential_header_tpm2() {
        let cred_name = "tpm2-hdr";
        let tpm2_secret = vec![0x42u8; 32];
        let pcr_mask: u32 = 1 << 7;
        let tpm2_blob = make_synthetic_tpm2_blob(pcr_mask);
        let serialized_size = tpm2_blob.serialize().len();

        let blob = encrypt_credential_inner(
            b"data",
            cred_name,
            SEAL_TPM2,
            999,
            0,
            None,
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        let header = parse_credential_header(&blob, None).unwrap();
        assert_eq!(header.seal_type, SEAL_TPM2);
        assert_eq!(header.timestamp, 999);
        assert_eq!(header.cred_name, cred_name);

        // TPM2 blob should be present.
        let (parsed_blob, consumed) = header.tpm2_blob.unwrap();
        assert_eq!(parsed_blob.pcr_mask, pcr_mask);
        assert_eq!(parsed_blob.pcr_bank, tpm2::TPM2_ALG_SHA256);
        assert_eq!(parsed_blob.primary_alg, tpm2::TPM2_ALG_ECC);
        assert_eq!(parsed_blob.private, vec![0xAA; 64]);
        assert_eq!(parsed_blob.public, vec![0xBB; 48]);
        assert_eq!(consumed, serialized_size);

        // data_start should be after header + name + TPM2 blob.
        assert_eq!(
            header.data_start,
            HEADER_FIXED_SIZE + cred_name.len() + serialized_size
        );
    }

    #[test]
    fn test_parse_credential_header_host_tpm2() {
        let cred_name = "ht-hdr";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob((1 << 0) | (1 << 7));

        let blob = encrypt_credential_inner(
            b"data",
            cred_name,
            SEAL_HOST_TPM2,
            12345,
            0,
            Some(&host_key),
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        let header = parse_credential_header(&blob, None).unwrap();
        assert_eq!(header.seal_type, SEAL_HOST_TPM2);
        assert_eq!(header.not_after, 0);
        assert!(header.tpm2_blob.is_some());

        let (parsed_blob, _) = header.tpm2_blob.unwrap();
        assert_eq!(parsed_blob.pcr_mask, (1 << 0) | (1 << 7));
    }

    #[test]
    fn test_parse_credential_header_name_validation() {
        let blob =
            encrypt_credential_inner(b"data", "real-name", SEAL_NULL, 0, 0, None, None).unwrap();

        let result = parse_credential_header(&blob, Some("wrong-name"));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("mismatch"));
    }

    #[test]
    fn test_parse_credential_header_too_short() {
        let result = parse_credential_header(&[0u8; 10], None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too short"));
    }

    #[test]
    fn test_parse_credential_header_bad_magic() {
        let mut blob =
            encrypt_credential_inner(b"data", "test", SEAL_NULL, 0, 0, None, None).unwrap();
        blob[0] = 0xFF;
        let result = parse_credential_header(&blob, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("magic"));
    }

    // ================================================================
    // TPM2 blob with varying PCR selections
    // ================================================================

    #[test]
    fn test_tpm2_credential_pcr_mask_preserved() {
        // Verify different PCR masks are preserved through encrypt → parse.
        for mask in [
            1u32 << 0,
            1 << 7,
            (1 << 0) | (1 << 7) | (1 << 14),
            0x00FFFFFF,
        ] {
            let tpm2_secret = vec![0x42u8; 32];
            let tpm2_blob = make_synthetic_tpm2_blob(mask);

            let blob = encrypt_credential_inner(
                b"x",
                "pcr-test",
                SEAL_TPM2,
                0,
                0,
                None,
                Some((tpm2_secret, tpm2_blob)),
            )
            .unwrap();

            let header = parse_credential_header(&blob, None).unwrap();
            let (parsed_blob, _) = header.tpm2_blob.unwrap();
            assert_eq!(
                parsed_blob.pcr_mask, mask,
                "PCR mask 0x{mask:06X} not preserved"
            );
        }
    }

    #[test]
    fn test_tpm2_credential_different_blob_sizes() {
        // Verify credentials work with different TPM2 blob data sizes.
        for priv_size in [0, 16, 64, 256, 512] {
            for pub_size in [0, 16, 48, 128] {
                let tpm2_secret = vec![0x42u8; 32];
                let tpm2_blob = tpm2::Tpm2SealedBlob {
                    pcr_mask: 1 << 7,
                    pcr_bank: tpm2::TPM2_ALG_SHA256,
                    primary_alg: tpm2::TPM2_ALG_ECC,
                    private: vec![0xAA; priv_size],
                    public: vec![0xBB; pub_size],
                };

                let blob = encrypt_credential_inner(
                    b"data",
                    "size-test",
                    SEAL_TPM2,
                    0,
                    0,
                    None,
                    Some((tpm2_secret.clone(), tpm2_blob)),
                )
                .unwrap();

                let (decrypted, _) = decrypt_credential_inner(
                    &blob,
                    Some("size-test"),
                    false,
                    None,
                    Some(&tpm2_secret),
                )
                .unwrap();
                assert_eq!(decrypted, b"data");
            }
        }
    }

    // ================================================================
    // TPM2 seal types — encrypt_credential_inner error paths
    // ================================================================

    #[test]
    fn test_encrypt_tpm2_requires_tpm2_blob() {
        let result = encrypt_credential_inner(
            b"data", "test", SEAL_TPM2, 0, 0, None, None, // missing TPM2 blob
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("TPM2 blob required"));
    }

    #[test]
    fn test_encrypt_host_requires_host_key() {
        let result = encrypt_credential_inner(
            b"data", "test", SEAL_HOST, 0, 0, None, // missing host key
            None,
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Host key required"));
    }

    #[test]
    fn test_encrypt_unsupported_seal_type() {
        let result = encrypt_credential_inner(b"data", "test", 99, 0, 0, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unsupported seal type"));
    }

    #[test]
    fn test_decrypt_unknown_seal_type() {
        // Build a blob with seal type 99 manually.
        let mut blob =
            encrypt_credential_inner(b"data", "test", SEAL_NULL, 0, 0, None, None).unwrap();
        // Patch seal type to 99.
        blob[4..8].copy_from_slice(&99u32.to_le_bytes());

        let result = decrypt_credential_inner(&blob, None, true, None, None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("Unknown seal type"));
    }

    // ================================================================
    // Cross-mode decryption failure tests
    // ================================================================

    #[test]
    fn test_null_cred_not_decryptable_as_host() {
        // A null-sealed credential cannot be decrypted with a host key
        // (different key derivation path, so auth tag mismatch).
        let blob = encrypt_credential_inner(b"data", "test", SEAL_NULL, 0, 0, None, None).unwrap();
        let host_key = vec![0xABu8; HOST_KEY_SIZE];

        // Patch seal type to HOST so decrypt_credential_inner uses host path.
        let mut modified = blob.clone();
        modified[4..8].copy_from_slice(&SEAL_HOST.to_le_bytes());

        let result = decrypt_credential_inner(&modified, Some("test"), true, Some(&host_key), None);
        assert!(result.is_err());
    }

    #[test]
    fn test_tpm2_cred_not_decryptable_with_wrong_mode() {
        // Encrypt as TPM2, try to decrypt as null (should fail).
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            b"data",
            "test",
            SEAL_TPM2,
            0,
            0,
            None,
            Some((tpm2_secret, tpm2_blob)),
        )
        .unwrap();

        // Patch seal type to NULL (skip TPM2 blob parsing).
        let mut modified = blob.clone();
        modified[4..8].copy_from_slice(&SEAL_NULL.to_le_bytes());

        // Will fail because the null key derivation doesn't match,
        // and the TPM2 blob bytes get misinterpreted as IV/ciphertext.
        let result = decrypt_credential_inner(&modified, Some("test"), true, None, None);
        assert!(result.is_err());
    }

    // ================================================================
    // TPM2 encrypt_credential (full path) — hardware-required tests
    // ================================================================

    #[test]
    fn test_encrypt_credential_tpm2_fails_without_hardware() {
        // On systems without TPM2, encrypt_credential with SEAL_TPM2
        // should fail with a meaningful error.
        if tpm2_available() {
            return; // Skip on systems with TPM2.
        }

        let result = encrypt_credential(b"data", "test", SEAL_TPM2, 0, 0, 1 << 7);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("TPM2") || err.contains("tpm"),
            "Error should mention TPM2: {err}"
        );
    }

    #[test]
    fn test_encrypt_credential_host_tpm2_fails_without_hardware() {
        if tpm2_available() {
            return;
        }

        let result = encrypt_credential(b"data", "test", SEAL_HOST_TPM2, 0, 0, 1 << 7);
        assert!(result.is_err());
        let err = result.unwrap_err();
        assert!(
            err.contains("TPM2") || err.contains("tpm"),
            "Error should mention TPM2: {err}"
        );
    }

    // ================================================================
    // Seal type resolution
    // ================================================================

    #[test]
    fn test_seal_type_constants() {
        assert_eq!(SEAL_NULL, 0);
        assert_eq!(SEAL_HOST, 1);
        assert_eq!(SEAL_TPM2, 2);
        assert_eq!(SEAL_HOST_TPM2, 3);
    }

    #[test]
    fn test_seal_type_name_all_values() {
        assert_eq!(seal_type_name(SEAL_NULL), "null");
        assert_eq!(seal_type_name(SEAL_HOST), "host");
        assert_eq!(seal_type_name(SEAL_TPM2), "tpm2");
        assert_eq!(seal_type_name(SEAL_HOST_TPM2), "host+tpm2");
        assert_eq!(seal_type_name(4), "unknown");
        assert_eq!(seal_type_name(u32::MAX), "unknown");
    }

    // ================================================================
    // TPM2 not-after expiry with TPM2 credentials
    // ================================================================

    #[test]
    fn test_tpm2_credential_expired() {
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        // Set not_after to 1 µs (long expired).
        let blob = encrypt_credential_inner(
            b"expired-data",
            "tpm2-exp",
            SEAL_TPM2,
            0,
            1, // expired
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let result =
            decrypt_credential_inner(&blob, Some("tpm2-exp"), false, None, Some(&tpm2_secret));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_host_tpm2_credential_expired() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let blob = encrypt_credential_inner(
            b"expired",
            "ht-exp",
            SEAL_HOST_TPM2,
            0,
            1,
            Some(&host_key),
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let result = decrypt_credential_inner(
            &blob,
            Some("ht-exp"),
            false,
            Some(&host_key),
            Some(&tpm2_secret),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("expired"));
    }

    #[test]
    fn test_tpm2_credential_not_yet_expired() {
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);
        let not_after = now_usec() + 3_600_000_000; // 1 hour from now

        let blob = encrypt_credential_inner(
            b"valid",
            "tpm2-valid",
            SEAL_TPM2,
            now_usec(),
            not_after,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let (decrypted, _) =
            decrypt_credential_inner(&blob, Some("tpm2-valid"), false, None, Some(&tpm2_secret))
                .unwrap();
        assert_eq!(decrypted, b"valid");
    }

    // ================================================================
    // encrypt_credential_inner with SEAL_HOST (synthetic host key)
    // ================================================================

    #[test]
    fn test_encrypt_decrypt_host_roundtrip_synthetic() {
        let plaintext = b"host-sealed data";
        let cred_name = "host-cred";
        let host_key = vec![0xABu8; HOST_KEY_SIZE];

        let blob = encrypt_credential_inner(
            plaintext,
            cred_name,
            SEAL_HOST,
            now_usec(),
            0,
            Some(&host_key),
            None,
        )
        .unwrap();

        let (decrypted, name) =
            decrypt_credential_inner(&blob, Some(cred_name), false, Some(&host_key), None).unwrap();

        assert_eq!(decrypted, plaintext);
        assert_eq!(name, cred_name);
    }

    #[test]
    fn test_encrypt_decrypt_host_wrong_key_fails() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let wrong_key = vec![0xCDu8; HOST_KEY_SIZE];

        let blob =
            encrypt_credential_inner(b"data", "test", SEAL_HOST, 0, 0, Some(&host_key), None)
                .unwrap();

        let result = decrypt_credential_inner(&blob, Some("test"), false, Some(&wrong_key), None);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("authentication tag"));
    }

    // ================================================================
    // Two different nonces for TPM2 credentials
    // ================================================================

    #[test]
    fn test_tpm2_encrypt_different_nonces() {
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob1 = make_synthetic_tpm2_blob(1 << 7);
        let tpm2_blob2 = make_synthetic_tpm2_blob(1 << 7);

        let blob1 = encrypt_credential_inner(
            b"same",
            "nonce-test",
            SEAL_TPM2,
            42,
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob1)),
        )
        .unwrap();

        let blob2 = encrypt_credential_inner(
            b"same",
            "nonce-test",
            SEAL_TPM2,
            42,
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob2)),
        )
        .unwrap();

        // Different random nonces → different blobs.
        assert_ne!(blob1, blob2);

        // Both should decrypt to the same plaintext.
        let (d1, _) =
            decrypt_credential_inner(&blob1, Some("nonce-test"), false, None, Some(&tpm2_secret))
                .unwrap();
        let (d2, _) =
            decrypt_credential_inner(&blob2, Some("nonce-test"), false, None, Some(&tpm2_secret))
                .unwrap();
        assert_eq!(d1, b"same");
        assert_eq!(d2, b"same");
    }

    // ================================================================
    // TPM2 corrupted credential blob
    // ================================================================

    #[test]
    fn test_tpm2_credential_corrupted_ciphertext() {
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let mut blob = encrypt_credential_inner(
            b"important",
            "tpm2-corrupt",
            SEAL_TPM2,
            0,
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        // Corrupt the last byte.
        let last = blob.len() - 1;
        blob[last] ^= 0xFF;

        let result =
            decrypt_credential_inner(&blob, Some("tpm2-corrupt"), false, None, Some(&tpm2_secret));
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("authentication tag"));
    }

    #[test]
    fn test_host_tpm2_credential_corrupted_ciphertext() {
        let host_key = vec![0xABu8; HOST_KEY_SIZE];
        let tpm2_secret = vec![0xCDu8; 32];
        let tpm2_blob = make_synthetic_tpm2_blob(1 << 7);

        let mut blob = encrypt_credential_inner(
            b"important",
            "ht-corrupt",
            SEAL_HOST_TPM2,
            0,
            0,
            Some(&host_key),
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let last = blob.len() - 1;
        blob[last] ^= 0xFF;

        let result = decrypt_credential_inner(
            &blob,
            Some("ht-corrupt"),
            false,
            Some(&host_key),
            Some(&tpm2_secret),
        );
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("authentication tag"));
    }

    // ================================================================
    // TPM2 RSA primary algorithm in blob
    // ================================================================

    #[test]
    fn test_tpm2_credential_rsa_primary_alg() {
        let tpm2_secret = vec![0x42u8; 32];
        let tpm2_blob = tpm2::Tpm2SealedBlob {
            pcr_mask: 1 << 7,
            pcr_bank: tpm2::TPM2_ALG_SHA256,
            primary_alg: tpm2::TPM2_ALG_RSA,
            private: vec![0xAA; 100],
            public: vec![0xBB; 80],
        };

        let blob = encrypt_credential_inner(
            b"rsa-test",
            "rsa-cred",
            SEAL_TPM2,
            0,
            0,
            None,
            Some((tpm2_secret.clone(), tpm2_blob)),
        )
        .unwrap();

        let header = parse_credential_header(&blob, None).unwrap();
        let (parsed_blob, _) = header.tpm2_blob.unwrap();
        assert_eq!(parsed_blob.primary_alg, tpm2::TPM2_ALG_RSA);

        let (decrypted, _) =
            decrypt_credential_inner(&blob, Some("rsa-cred"), false, None, Some(&tpm2_secret))
                .unwrap();
        assert_eq!(decrypted, b"rsa-test");
    }
}
