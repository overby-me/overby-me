//! TPM2 device communication for credential sealing/unsealing.
//!
//! This module implements direct communication with the Linux kernel's TPM2
//! resource manager (`/dev/tpmrm0`) using raw TPM2 command buffers. No C
//! library dependency is required — all marshaling is done in pure Rust.
//!
//! The supported operations are:
//! - **Seal**: encrypt a secret so it can only be recovered when the system's
//!   PCR values match the values at seal time.
//! - **Unseal**: recover a previously sealed secret (requires matching PCRs).
//!
//! The TPM2 flow for sealing:
//! 1. Create a primary key (SRK) in the owner hierarchy
//! 2. Read current PCR values for the selected PCR mask
//! 3. Compute a trial policy digest (PolicyPCR)
//! 4. Create a sealed data object bound to that policy
//! 5. Return the TPM2B_PRIVATE and TPM2B_PUBLIC blobs
//!
//! The TPM2 flow for unsealing:
//! 1. Create the same primary key (SRK)
//! 2. Load the sealed object from saved private/public blobs
//! 3. Start a policy session
//! 4. Satisfy PolicyPCR (TPM checks current PCR values)
//! 5. Unseal the data object
//! 6. Return the plaintext secret

use sha2::{Digest, Sha256};
use std::fs;
use std::io::{Read, Write};
use std::path::Path;

// ---------------------------------------------------------------------------
// TPM2 constants
// ---------------------------------------------------------------------------

/// Command/response tag: no authorization sessions.
const TPM2_ST_NO_SESSIONS: u16 = 0x8001;
/// Command/response tag: with authorization sessions.
const TPM2_ST_SESSIONS: u16 = 0x8002;

/// Command codes.
const TPM2_CC_CREATE_PRIMARY: u32 = 0x0000_0131;
const TPM2_CC_CREATE: u32 = 0x0000_0153;
const TPM2_CC_LOAD: u32 = 0x0000_0157;
const TPM2_CC_UNSEAL: u32 = 0x0000_015E;
const TPM2_CC_FLUSH_CONTEXT: u32 = 0x0000_0165;
const TPM2_CC_START_AUTH_SESSION: u32 = 0x0000_0176;
const TPM2_CC_POLICY_PCR: u32 = 0x0000_017F;
const TPM2_CC_PCR_READ: u32 = 0x0000_017E;

/// Well-known handles.
const TPM2_RH_OWNER: u32 = 0x4000_0001;
const TPM2_RH_NULL: u32 = 0x4000_0007;
const TPM2_RS_PW: u32 = 0x4000_0009;

/// Success response code.
const TPM2_RC_SUCCESS: u32 = 0x0000_0000;

/// Algorithm IDs.
pub const TPM2_ALG_RSA: u16 = 0x0001;
pub const TPM2_ALG_SHA256: u16 = 0x000B;
const TPM2_ALG_AES: u16 = 0x0006;
const TPM2_ALG_KEYEDHASH: u16 = 0x0008;
const TPM2_ALG_NULL: u16 = 0x0010;
pub const TPM2_ALG_ECC: u16 = 0x0023;
const TPM2_ALG_CFB: u16 = 0x0043;

/// ECC curve IDs.
const TPM2_ECC_NIST_P256: u16 = 0x0003;

/// Session type: policy session.
const TPM2_SE_POLICY: u8 = 0x01;

/// TPMA_OBJECT attribute bits.
const TPMA_OBJECT_FIXED_TPM: u32 = 1 << 1;
const TPMA_OBJECT_FIXED_PARENT: u32 = 1 << 4;
const TPMA_OBJECT_SENSITIVE_DATA_ORIGIN: u32 = 1 << 5;
const TPMA_OBJECT_USER_WITH_AUTH: u32 = 1 << 6;
const TPMA_OBJECT_NO_DA: u32 = 1 << 10;
const TPMA_OBJECT_RESTRICTED: u32 = 1 << 16;
const TPMA_OBJECT_DECRYPT: u32 = 1 << 17;

/// Object attributes for the Storage Root Key (SRK).
const SRK_ATTRIBUTES: u32 = TPMA_OBJECT_FIXED_TPM
    | TPMA_OBJECT_FIXED_PARENT
    | TPMA_OBJECT_SENSITIVE_DATA_ORIGIN
    | TPMA_OBJECT_USER_WITH_AUTH
    | TPMA_OBJECT_NO_DA
    | TPMA_OBJECT_RESTRICTED
    | TPMA_OBJECT_DECRYPT;

/// Object attributes for sealed data.
const SEALED_ATTRIBUTES: u32 = TPMA_OBJECT_FIXED_TPM | TPMA_OBJECT_FIXED_PARENT | TPMA_OBJECT_NO_DA;

/// SHA-256 digest size.
const SHA256_DIGEST_SIZE: usize = 32;

/// Maximum TPM2 response buffer size.
const TPM2_MAX_RESPONSE_SIZE: usize = 4096;

/// Default PCR mask: PCR 7 (Secure Boot policy).
pub const DEFAULT_PCR_MASK: u32 = 1 << 7;

// ---------------------------------------------------------------------------
// Public types
// ---------------------------------------------------------------------------

/// A TPM2-sealed blob containing the data needed to unseal a secret.
#[derive(Clone, Debug)]
pub struct Tpm2SealedBlob {
    /// Which PCRs the secret is bound to (bitmask).
    pub pcr_mask: u32,
    /// The PCR hash algorithm (e.g. `TPM2_ALG_SHA256`).
    pub pcr_bank: u16,
    /// The primary key algorithm (e.g. `TPM2_ALG_ECC` or `TPM2_ALG_RSA`).
    pub primary_alg: u16,
    /// The TPM2B_PRIVATE marshaled data.
    pub private: Vec<u8>,
    /// The TPM2B_PUBLIC marshaled data.
    pub public: Vec<u8>,
}

impl Tpm2SealedBlob {
    /// Serialize this blob for embedding in a credential.
    pub fn serialize(&self) -> Vec<u8> {
        let mut buf = Vec::new();
        buf.extend_from_slice(&self.pcr_mask.to_le_bytes());
        buf.extend_from_slice(&self.pcr_bank.to_le_bytes());
        buf.extend_from_slice(&self.primary_alg.to_le_bytes());
        buf.extend_from_slice(&(self.private.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.private);
        buf.extend_from_slice(&(self.public.len() as u32).to_le_bytes());
        buf.extend_from_slice(&self.public);
        buf
    }

    /// Deserialize a blob from a byte slice. Returns the blob and the number
    /// of bytes consumed.
    pub fn deserialize(data: &[u8]) -> Result<(Self, usize), String> {
        // Minimum: pcr_mask(4) + pcr_bank(2) + primary_alg(2) + priv_len(4) + pub_len(4) = 16
        if data.len() < 16 {
            return Err("TPM2 blob too short for header fields".into());
        }

        let mut pos = 0;

        let pcr_mask = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap());
        pos += 4;
        let pcr_bank = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
        pos += 2;
        let primary_alg = u16::from_le_bytes(data[pos..pos + 2].try_into().unwrap());
        pos += 2;

        let private_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if data.len() < pos + private_len + 4 {
            return Err("TPM2 blob too short for private data".into());
        }
        let private = data[pos..pos + private_len].to_vec();
        pos += private_len;

        let public_len = u32::from_le_bytes(data[pos..pos + 4].try_into().unwrap()) as usize;
        pos += 4;
        if data.len() < pos + public_len {
            return Err("TPM2 blob too short for public data".into());
        }
        let public = data[pos..pos + public_len].to_vec();
        pos += public_len;

        Ok((
            Tpm2SealedBlob {
                pcr_mask,
                pcr_bank,
                primary_alg,
                private,
                public,
            },
            pos,
        ))
    }
}

// ---------------------------------------------------------------------------
// Command builder
// ---------------------------------------------------------------------------

/// Builds a TPM2 command buffer with big-endian marshaling.
struct CmdBuilder {
    buf: Vec<u8>,
}

impl CmdBuilder {
    /// Create a new command with the given tag and command code.
    /// The size field is set to a placeholder and patched by `finalize()`.
    fn new(tag: u16, command_code: u32) -> Self {
        let mut buf = Vec::with_capacity(128);
        buf.extend_from_slice(&tag.to_be_bytes()); // tag
        buf.extend_from_slice(&0u32.to_be_bytes()); // size placeholder
        buf.extend_from_slice(&command_code.to_be_bytes()); // CC
        CmdBuilder { buf }
    }

    fn put_u8(&mut self, v: u8) {
        self.buf.push(v);
    }

    fn put_u16(&mut self, v: u16) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn put_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_be_bytes());
    }

    fn put_bytes(&mut self, data: &[u8]) {
        self.buf.extend_from_slice(data);
    }

    /// Write a TPM2B structure: u16 size prefix + data.
    fn put_tpm2b(&mut self, data: &[u8]) {
        self.put_u16(data.len() as u16);
        self.buf.extend_from_slice(data);
    }

    /// Write the standard password authorization area (empty password).
    fn put_pw_auth(&mut self) {
        // Auth area contents (9 bytes):
        //   sessionHandle: TPM2_RS_PW (4 bytes)
        //   nonceTpm: TPM2B empty (2 bytes)
        //   sessionAttributes: continueSession (1 byte)
        //   hmac: TPM2B empty (2 bytes)
        let auth_size: u32 = 4 + 2 + 1 + 2; // = 9
        self.put_u32(auth_size);
        self.put_u32(TPM2_RS_PW);
        self.put_u16(0); // nonce
        self.put_u8(0x01); // continueSession
        self.put_u16(0); // hmac
    }

    /// Write a policy-session authorization area.
    fn put_policy_auth(&mut self, session_handle: u32) {
        let auth_size: u32 = 4 + 2 + 1 + 2; // = 9
        self.put_u32(auth_size);
        self.put_u32(session_handle);
        self.put_u16(0); // nonce
        self.put_u8(0x01); // continueSession
        self.put_u16(0); // hmac
    }

    /// Patch the size field and return the final command buffer.
    fn finalize(&mut self) -> &[u8] {
        let size = self.buf.len() as u32;
        self.buf[2..6].copy_from_slice(&size.to_be_bytes());
        &self.buf
    }
}

// ---------------------------------------------------------------------------
// Response parser
// ---------------------------------------------------------------------------

/// Parses a TPM2 response buffer.
#[derive(Debug)]
struct RespParser<'a> {
    buf: &'a [u8],
    pos: usize,
}

impl<'a> RespParser<'a> {
    /// Parse the response header. Returns an error if the response code is
    /// not TPM2_RC_SUCCESS.
    fn new(buf: &'a [u8]) -> Result<Self, String> {
        if buf.len() < 10 {
            return Err("TPM2 response too short for header".into());
        }
        let _tag = u16::from_be_bytes([buf[0], buf[1]]);
        let size = u32::from_be_bytes([buf[2], buf[3], buf[4], buf[5]]) as usize;
        let rc = u32::from_be_bytes([buf[6], buf[7], buf[8], buf[9]]);

        if size > buf.len() {
            return Err(format!(
                "TPM2 response size ({size}) exceeds buffer length ({})",
                buf.len()
            ));
        }

        if rc != TPM2_RC_SUCCESS {
            return Err(format!("TPM2 error: response code 0x{rc:08X}"));
        }

        Ok(RespParser {
            buf: &buf[..size],
            pos: 10,
        })
    }

    fn remaining(&self) -> usize {
        self.buf.len().saturating_sub(self.pos)
    }

    fn get_u16(&mut self) -> Result<u16, String> {
        if self.remaining() < 2 {
            return Err("TPM2 response: unexpected end reading u16".into());
        }
        let v = u16::from_be_bytes([self.buf[self.pos], self.buf[self.pos + 1]]);
        self.pos += 2;
        Ok(v)
    }

    fn get_u32(&mut self) -> Result<u32, String> {
        if self.remaining() < 4 {
            return Err("TPM2 response: unexpected end reading u32".into());
        }
        let v = u32::from_be_bytes([
            self.buf[self.pos],
            self.buf[self.pos + 1],
            self.buf[self.pos + 2],
            self.buf[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(v)
    }

    fn get_bytes(&mut self, len: usize) -> Result<&'a [u8], String> {
        if self.remaining() < len {
            return Err(format!(
                "TPM2 response: unexpected end reading {len} bytes (only {} remaining)",
                self.remaining()
            ));
        }
        let data = &self.buf[self.pos..self.pos + len];
        self.pos += len;
        Ok(data)
    }

    /// Read a TPM2B (u16 size + data).
    fn get_tpm2b(&mut self) -> Result<&'a [u8], String> {
        let size = self.get_u16()? as usize;
        self.get_bytes(size)
    }

    fn skip(&mut self, n: usize) -> Result<(), String> {
        if self.remaining() < n {
            return Err(format!(
                "TPM2 response: cannot skip {n} bytes (only {} remaining)",
                self.remaining()
            ));
        }
        self.pos += n;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// TPM2 device I/O
// ---------------------------------------------------------------------------

/// Manages a file handle to the TPM2 resource manager device.
struct Tpm2Device {
    path: String,
}

impl Tpm2Device {
    /// Open the TPM2 resource manager device.
    fn open() -> Result<Self, String> {
        for path in &["/dev/tpmrm0", "/dev/tpm0"] {
            if Path::new(path).exists() {
                return Ok(Tpm2Device {
                    path: path.to_string(),
                });
            }
        }
        Err("No TPM2 device found (/dev/tpmrm0 or /dev/tpm0)".into())
    }

    /// Send a command and receive the response.
    fn transact(&self, cmd: &[u8]) -> Result<Vec<u8>, String> {
        let mut file = fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&self.path)
            .map_err(|e| format!("Failed to open TPM2 device {}: {e}", self.path))?;

        file.write_all(cmd)
            .map_err(|e| format!("Failed to write to TPM2 device: {e}"))?;

        let mut resp = vec![0u8; TPM2_MAX_RESPONSE_SIZE];
        let n = file
            .read(&mut resp)
            .map_err(|e| format!("Failed to read from TPM2 device: {e}"))?;

        if n < 10 {
            return Err(format!("TPM2 response too short: {n} bytes (minimum 10)"));
        }

        resp.truncate(n);
        Ok(resp)
    }
}

// ---------------------------------------------------------------------------
// SRK (Storage Root Key) template builders
// ---------------------------------------------------------------------------

/// Build the TPMT_PUBLIC for an ECC P-256 SRK.
fn build_ecc_srk_template() -> Vec<u8> {
    let mut t = Vec::with_capacity(92);
    // type
    t.extend_from_slice(&TPM2_ALG_ECC.to_be_bytes());
    // nameAlg
    t.extend_from_slice(&TPM2_ALG_SHA256.to_be_bytes());
    // objectAttributes
    t.extend_from_slice(&SRK_ATTRIBUTES.to_be_bytes());
    // authPolicy: TPM2B_DIGEST (empty)
    t.extend_from_slice(&0u16.to_be_bytes());
    // TPMS_ECC_PARMS
    //   symmetric
    t.extend_from_slice(&TPM2_ALG_AES.to_be_bytes()); // algorithm
    t.extend_from_slice(&128u16.to_be_bytes()); // keyBits
    t.extend_from_slice(&TPM2_ALG_CFB.to_be_bytes()); // mode
    //   scheme
    t.extend_from_slice(&TPM2_ALG_NULL.to_be_bytes());
    //   curveID
    t.extend_from_slice(&TPM2_ECC_NIST_P256.to_be_bytes());
    //   kdfScheme
    t.extend_from_slice(&TPM2_ALG_NULL.to_be_bytes());
    // TPMS_ECC_POINT (unique)
    //   x: 32 zero bytes
    t.extend_from_slice(&32u16.to_be_bytes());
    t.extend_from_slice(&[0u8; 32]);
    //   y: 32 zero bytes
    t.extend_from_slice(&32u16.to_be_bytes());
    t.extend_from_slice(&[0u8; 32]);
    t
}

/// Build the TPMT_PUBLIC for an RSA-2048 SRK.
fn build_rsa_srk_template() -> Vec<u8> {
    let mut t = Vec::with_capacity(280);
    // type
    t.extend_from_slice(&TPM2_ALG_RSA.to_be_bytes());
    // nameAlg
    t.extend_from_slice(&TPM2_ALG_SHA256.to_be_bytes());
    // objectAttributes
    t.extend_from_slice(&SRK_ATTRIBUTES.to_be_bytes());
    // authPolicy: TPM2B_DIGEST (empty)
    t.extend_from_slice(&0u16.to_be_bytes());
    // TPMS_RSA_PARMS
    //   symmetric
    t.extend_from_slice(&TPM2_ALG_AES.to_be_bytes()); // algorithm
    t.extend_from_slice(&128u16.to_be_bytes()); // keyBits
    t.extend_from_slice(&TPM2_ALG_CFB.to_be_bytes()); // mode
    //   scheme
    t.extend_from_slice(&TPM2_ALG_NULL.to_be_bytes());
    //   keyBits
    t.extend_from_slice(&2048u16.to_be_bytes());
    //   exponent (0 = default 65537)
    t.extend_from_slice(&0u32.to_be_bytes());
    // unique: TPM2B_PUBLIC_KEY_RSA (256 zero bytes for 2048-bit key)
    t.extend_from_slice(&256u16.to_be_bytes());
    t.extend_from_slice(&[0u8; 256]);
    t
}

// ---------------------------------------------------------------------------
// PCR selection helpers
// ---------------------------------------------------------------------------

/// Build a TPML_PCR_SELECTION structure for the given mask and bank.
fn build_pcr_selection(pcr_mask: u32, pcr_bank: u16) -> Vec<u8> {
    if pcr_mask == 0 {
        // Empty selection: count = 0
        return 0u32.to_be_bytes().to_vec();
    }

    let mut buf = Vec::with_capacity(12);
    // count = 1
    buf.extend_from_slice(&1u32.to_be_bytes());
    // TPMS_PCR_SELECTION
    buf.extend_from_slice(&pcr_bank.to_be_bytes()); // hash
    buf.push(3); // sizeofSelect = 3 (covers PCRs 0-23)
    // PCR select bytes (little-endian bitmask)
    buf.push((pcr_mask & 0xFF) as u8);
    buf.push(((pcr_mask >> 8) & 0xFF) as u8);
    buf.push(((pcr_mask >> 16) & 0xFF) as u8);
    buf
}

/// Build a TPML_PCR_SELECTION for an empty selection (count=0).
fn build_empty_pcr_selection() -> Vec<u8> {
    0u32.to_be_bytes().to_vec()
}

// ---------------------------------------------------------------------------
// Trial policy computation (done entirely in software)
// ---------------------------------------------------------------------------

/// Compute the trial policy digest for PolicyPCR.
///
/// `pcr_values` is a list of (pcr_index, pcr_value) pairs in ascending index
/// order.
fn compute_policy_pcr_digest(
    pcr_mask: u32,
    pcr_bank: u16,
    pcr_values: &[(u32, Vec<u8>)],
) -> [u8; SHA256_DIGEST_SIZE] {
    // 1. Compute the PCR digest: SHA-256 of concatenated PCR values in order.
    let mut pcr_hasher = Sha256::new();
    for (_idx, value) in pcr_values {
        pcr_hasher.update(value);
    }
    let pcr_digest: [u8; 32] = pcr_hasher.finalize().into();

    // 2. Build the TPML_PCR_SELECTION bytes for inclusion in the policy hash.
    let pcr_selection = build_pcr_selection(pcr_mask, pcr_bank);

    // 3. Compute:
    //    policyDigest = SHA-256(zeros(32) || TPM2_CC_PolicyPCR || pcr_selection || pcr_digest)
    let mut hasher = Sha256::new();
    hasher.update([0u8; SHA256_DIGEST_SIZE]); // initial empty policy
    hasher.update(TPM2_CC_POLICY_PCR.to_be_bytes());
    hasher.update(&pcr_selection);
    hasher.update(pcr_digest);
    hasher.finalize().into()
}

// ---------------------------------------------------------------------------
// Low-level TPM2 commands
// ---------------------------------------------------------------------------

/// Create a primary key (SRK) in the owner hierarchy.
/// Returns the object handle.
fn tpm2_create_primary(dev: &Tpm2Device, primary_alg: u16) -> Result<u32, String> {
    let template = match primary_alg {
        TPM2_ALG_ECC => build_ecc_srk_template(),
        TPM2_ALG_RSA => build_rsa_srk_template(),
        other => return Err(format!("Unsupported primary algorithm: 0x{other:04X}")),
    };

    let mut cmd = CmdBuilder::new(TPM2_ST_SESSIONS, TPM2_CC_CREATE_PRIMARY);
    // primaryHandle
    cmd.put_u32(TPM2_RH_OWNER);
    // authorization area (empty password)
    cmd.put_pw_auth();
    // inSensitive: TPM2B_SENSITIVE_CREATE
    //   size(2) + userAuth TPM2B(2) + data TPM2B(2) = 6
    cmd.put_u16(4); // size of TPMS_SENSITIVE_CREATE
    cmd.put_u16(0); // userAuth (empty)
    cmd.put_u16(0); // data (empty)
    // inPublic: TPM2B_PUBLIC
    cmd.put_tpm2b(&template);
    // outsideInfo: TPM2B_DATA (empty)
    cmd.put_u16(0);
    // creationPCR: TPML_PCR_SELECTION (empty)
    let empty_pcrs = build_empty_pcr_selection();
    cmd.put_bytes(&empty_pcrs);

    let resp_buf = dev.transact(cmd.finalize())?;
    let mut resp = RespParser::new(&resp_buf)?;

    // objectHandle
    let handle = resp.get_u32()?;

    Ok(handle)
}

/// Read PCR values for the selected PCRs.
/// Returns a list of (pcr_index, pcr_value) in ascending index order.
fn tpm2_pcr_read(
    dev: &Tpm2Device,
    pcr_mask: u32,
    pcr_bank: u16,
) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, TPM2_CC_PCR_READ);
    // pcrSelectionIn: TPML_PCR_SELECTION
    let selection = build_pcr_selection(pcr_mask, pcr_bank);
    cmd.put_bytes(&selection);

    let resp_buf = dev.transact(cmd.finalize())?;
    let mut resp = RespParser::new(&resp_buf)?;

    // pcrUpdateCounter (u32)
    let _update_counter = resp.get_u32()?;

    // pcrSelectionOut: TPML_PCR_SELECTION
    let sel_count = resp.get_u32()?;
    for _ in 0..sel_count {
        let _hash = resp.get_u16()?;
        let sizeof_select = resp.get_bytes(1)?[0] as usize;
        resp.skip(sizeof_select)?;
    }

    // pcrValues: TPML_DIGEST
    let digest_count = resp.get_u32()?;
    let mut result = Vec::new();

    // Map digest index back to PCR index
    let mut pcr_indices: Vec<u32> = Vec::new();
    for i in 0..24 {
        if pcr_mask & (1 << i) != 0 {
            pcr_indices.push(i);
        }
    }

    for i in 0..digest_count as usize {
        let digest = resp.get_tpm2b()?;
        if i < pcr_indices.len() {
            result.push((pcr_indices[i], digest.to_vec()));
        }
    }

    Ok(result)
}

/// Start a policy session. Returns the session handle.
fn tpm2_start_auth_session(dev: &Tpm2Device) -> Result<u32, String> {
    let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, TPM2_CC_START_AUTH_SESSION);
    // tpmKey: TPM2_RH_NULL (no salt)
    cmd.put_u32(TPM2_RH_NULL);
    // bind: TPM2_RH_NULL (no bind)
    cmd.put_u32(TPM2_RH_NULL);
    // nonceCaller: TPM2B (32 random bytes)
    let nonce = generate_nonce();
    cmd.put_tpm2b(&nonce);
    // encryptedSalt: TPM2B (empty — no salt)
    cmd.put_u16(0);
    // sessionType: TPM2_SE_POLICY
    cmd.put_u8(TPM2_SE_POLICY);
    // symmetric: TPMT_SYM_DEF (null)
    cmd.put_u16(TPM2_ALG_NULL);
    // authHash: SHA-256
    cmd.put_u16(TPM2_ALG_SHA256);

    let resp_buf = dev.transact(cmd.finalize())?;
    let mut resp = RespParser::new(&resp_buf)?;

    // sessionHandle
    let handle = resp.get_u32()?;

    Ok(handle)
}

/// Execute PolicyPCR on a policy session.
fn tpm2_policy_pcr(
    dev: &Tpm2Device,
    session_handle: u32,
    pcr_mask: u32,
    pcr_bank: u16,
) -> Result<(), String> {
    let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, TPM2_CC_POLICY_PCR);
    // policySession
    cmd.put_u32(session_handle);
    // pcrDigest: TPM2B_DIGEST (empty — TPM will compute from current PCRs)
    cmd.put_u16(0);
    // pcrs: TPML_PCR_SELECTION
    let selection = build_pcr_selection(pcr_mask, pcr_bank);
    cmd.put_bytes(&selection);

    let resp_buf = dev.transact(cmd.finalize())?;
    let _resp = RespParser::new(&resp_buf)?;

    Ok(())
}

/// Create a sealed data object under the given parent key.
/// Returns (TPM2B_PRIVATE, TPM2B_PUBLIC) as raw bytes.
fn tpm2_create_sealed(
    dev: &Tpm2Device,
    parent_handle: u32,
    secret: &[u8],
    policy_digest: &[u8; SHA256_DIGEST_SIZE],
) -> Result<(Vec<u8>, Vec<u8>), String> {
    // Build the sealed object template (TPMT_PUBLIC for KEYEDHASH)
    let mut template = Vec::with_capacity(48);
    // type
    template.extend_from_slice(&TPM2_ALG_KEYEDHASH.to_be_bytes());
    // nameAlg
    template.extend_from_slice(&TPM2_ALG_SHA256.to_be_bytes());
    // objectAttributes
    template.extend_from_slice(&SEALED_ATTRIBUTES.to_be_bytes());
    // authPolicy: the PCR policy digest
    template.extend_from_slice(&(policy_digest.len() as u16).to_be_bytes());
    template.extend_from_slice(policy_digest);
    // TPMS_KEYEDHASH_PARMS: scheme = NULL
    template.extend_from_slice(&TPM2_ALG_NULL.to_be_bytes());
    // unique: TPM2B_DIGEST (empty)
    template.extend_from_slice(&0u16.to_be_bytes());

    let mut cmd = CmdBuilder::new(TPM2_ST_SESSIONS, TPM2_CC_CREATE);
    // parentHandle
    cmd.put_u32(parent_handle);
    // authorization area (empty password)
    cmd.put_pw_auth();
    // inSensitive: TPM2B_SENSITIVE_CREATE
    //   The TPMS_SENSITIVE_CREATE contains:
    //     userAuth: TPM2B (empty, size=0, 2 bytes)
    //     data: TPM2B (the secret)
    let inner_size = 2 + 2 + secret.len(); // userAuth(2) + data_size(2) + data
    cmd.put_u16(inner_size as u16);
    cmd.put_u16(0); // userAuth (empty)
    cmd.put_tpm2b(secret); // data = the secret to seal
    // inPublic: TPM2B_PUBLIC
    cmd.put_tpm2b(&template);
    // outsideInfo: TPM2B_DATA (empty)
    cmd.put_u16(0);
    // creationPCR: TPML_PCR_SELECTION (empty)
    let empty_pcrs = build_empty_pcr_selection();
    cmd.put_bytes(&empty_pcrs);

    let resp_buf = dev.transact(cmd.finalize())?;
    let mut resp = RespParser::new(&resp_buf)?;

    // parameterSize
    let _param_size = resp.get_u32()?;

    // outPrivate: TPM2B_PRIVATE
    let private_data = resp.get_tpm2b()?;

    // outPublic: TPM2B_PUBLIC
    let public_data = resp.get_tpm2b()?;

    Ok((private_data.to_vec(), public_data.to_vec()))
}

/// Load a sealed object into the TPM. Returns the object handle.
fn tpm2_load(
    dev: &Tpm2Device,
    parent_handle: u32,
    private: &[u8],
    public: &[u8],
) -> Result<u32, String> {
    let mut cmd = CmdBuilder::new(TPM2_ST_SESSIONS, TPM2_CC_LOAD);
    // parentHandle
    cmd.put_u32(parent_handle);
    // authorization area (empty password)
    cmd.put_pw_auth();
    // inPrivate: TPM2B_PRIVATE
    cmd.put_tpm2b(private);
    // inPublic: TPM2B_PUBLIC
    cmd.put_tpm2b(public);

    let resp_buf = dev.transact(cmd.finalize())?;
    let mut resp = RespParser::new(&resp_buf)?;

    // objectHandle
    let handle = resp.get_u32()?;

    Ok(handle)
}

/// Unseal a loaded object using a policy session. Returns the plaintext.
fn tpm2_unseal(
    dev: &Tpm2Device,
    object_handle: u32,
    session_handle: u32,
) -> Result<Vec<u8>, String> {
    let mut cmd = CmdBuilder::new(TPM2_ST_SESSIONS, TPM2_CC_UNSEAL);
    // itemHandle
    cmd.put_u32(object_handle);
    // authorization area: policy session
    cmd.put_policy_auth(session_handle);

    let resp_buf = dev.transact(cmd.finalize())?;
    let mut resp = RespParser::new(&resp_buf)?;

    // parameterSize
    let _param_size = resp.get_u32()?;

    // outData: TPM2B_SENSITIVE_DATA
    let data = resp.get_tpm2b()?;

    Ok(data.to_vec())
}

/// Flush a transient object or session from the TPM.
fn tpm2_flush_context(dev: &Tpm2Device, handle: u32) -> Result<(), String> {
    let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, TPM2_CC_FLUSH_CONTEXT);
    cmd.put_u32(handle);

    let resp_buf = dev.transact(cmd.finalize())?;
    let _resp = RespParser::new(&resp_buf)?;
    Ok(())
}

// ---------------------------------------------------------------------------
// Utility
// ---------------------------------------------------------------------------

/// Generate a 32-byte random nonce from /dev/urandom.
fn generate_nonce() -> [u8; 32] {
    let mut buf = [0u8; 32];
    if let Ok(mut f) = fs::File::open("/dev/urandom") {
        let _ = Read::read_exact(&mut f, &mut buf);
    }
    buf
}

/// Try to read PCR values from sysfs as a fallback when TPM2 device I/O
/// for PCR_Read is problematic.
fn read_pcrs_from_sysfs(pcr_mask: u32, pcr_bank: u16) -> Result<Vec<(u32, Vec<u8>)>, String> {
    let bank_name = match pcr_bank {
        TPM2_ALG_SHA256 => "sha256",
        0x0004 => "sha1",
        0x000C => "sha384",
        0x000D => "sha512",
        other => return Err(format!("Unknown PCR bank algorithm 0x{other:04X}")),
    };

    let mut result = Vec::new();
    for i in 0..24u32 {
        if pcr_mask & (1 << i) == 0 {
            continue;
        }

        let path = format!("/sys/class/tpm/tpm0/pcr-{bank_name}/{i}");
        let hex_str = fs::read_to_string(&path)
            .map_err(|e| format!("Failed to read PCR {i} from {path}: {e}"))?;

        let hex_clean: String = hex_str
            .trim()
            .chars()
            .filter(|c| c.is_ascii_hexdigit())
            .collect();
        let bytes = hex_to_bytes(&hex_clean)?;
        result.push((i, bytes));
    }

    Ok(result)
}

/// Decode a hex string to bytes.
fn hex_to_bytes(s: &str) -> Result<Vec<u8>, String> {
    if !s.len().is_multiple_of(2) {
        return Err("Hex string has odd length".into());
    }
    (0..s.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&s[i..i + 2], 16)
                .map_err(|e| format!("Invalid hex at offset {i}: {e}"))
        })
        .collect()
}

/// Read PCR values, trying TPM2 device first then sysfs fallback.
fn read_pcr_values(
    dev: &Tpm2Device,
    pcr_mask: u32,
    pcr_bank: u16,
) -> Result<Vec<(u32, Vec<u8>)>, String> {
    // Try TPM2 PCR_Read first
    match tpm2_pcr_read(dev, pcr_mask, pcr_bank) {
        Ok(values) if !values.is_empty() => return Ok(values),
        Ok(_) => {}  // empty result, fall through
        Err(_) => {} // error, fall through
    }

    // Fallback to sysfs
    read_pcrs_from_sysfs(pcr_mask, pcr_bank)
}

// ---------------------------------------------------------------------------
// High-level API
// ---------------------------------------------------------------------------

/// Seal a secret using the TPM2.
///
/// The secret is sealed to the current values of the PCRs specified by
/// `pcr_mask`. It can only be recovered when the PCR values match.
///
/// Returns a `Tpm2SealedBlob` containing the TPM2B_PRIVATE and TPM2B_PUBLIC
/// needed to later unseal the secret.
pub fn tpm2_seal_secret(
    secret: &[u8],
    pcr_mask: u32,
    pcr_bank: u16,
    primary_alg: u16,
) -> Result<Tpm2SealedBlob, String> {
    if secret.is_empty() {
        return Err("Cannot seal an empty secret".into());
    }
    // TPM2 sealed data is limited to the nameAlg digest size for many TPMs,
    // but the spec allows up to MAX_SYM_DATA (128 bytes typically). We limit
    // to 128 bytes to be safe.
    if secret.len() > 128 {
        return Err(format!(
            "Secret too large for TPM2 sealing: {} bytes (max 128)",
            secret.len()
        ));
    }

    let dev = Tpm2Device::open()?;

    // 1. Read current PCR values.
    let pcr_values = read_pcr_values(&dev, pcr_mask, pcr_bank)?;

    // 2. Compute the trial policy digest.
    let policy_digest = compute_policy_pcr_digest(pcr_mask, pcr_bank, &pcr_values);

    // 3. Create the primary key (SRK).
    let srk_handle = tpm2_create_primary(&dev, primary_alg)?;

    // 4. Create the sealed object.
    let result = tpm2_create_sealed(&dev, srk_handle, secret, &policy_digest);

    // 5. Flush the SRK — always clean up.
    let _ = tpm2_flush_context(&dev, srk_handle);

    let (private, public) = result?;

    Ok(Tpm2SealedBlob {
        pcr_mask,
        pcr_bank,
        primary_alg,
        private,
        public,
    })
}

/// Unseal a secret from a `Tpm2SealedBlob`.
///
/// This will fail if the current PCR values don't match the values at seal
/// time.
pub fn tpm2_unseal_secret(blob: &Tpm2SealedBlob) -> Result<Vec<u8>, String> {
    let dev = Tpm2Device::open()?;

    // 1. Recreate the primary key (SRK) — same template yields same key.
    let srk_handle = tpm2_create_primary(&dev, blob.primary_alg)?;

    // 2. Load the sealed object.
    let obj_handle = match tpm2_load(&dev, srk_handle, &blob.private, &blob.public) {
        Ok(h) => h,
        Err(e) => {
            let _ = tpm2_flush_context(&dev, srk_handle);
            return Err(format!("Failed to load sealed object: {e}"));
        }
    };

    // 3. Start a policy session.
    let session = match tpm2_start_auth_session(&dev) {
        Ok(s) => s,
        Err(e) => {
            let _ = tpm2_flush_context(&dev, obj_handle);
            let _ = tpm2_flush_context(&dev, srk_handle);
            return Err(format!("Failed to start policy session: {e}"));
        }
    };

    // 4. Satisfy PolicyPCR — the TPM checks current PCR values.
    if let Err(e) = tpm2_policy_pcr(&dev, session, blob.pcr_mask, blob.pcr_bank) {
        let _ = tpm2_flush_context(&dev, session);
        let _ = tpm2_flush_context(&dev, obj_handle);
        let _ = tpm2_flush_context(&dev, srk_handle);
        return Err(format!(
            "PolicyPCR failed (PCR values may have changed): {e}"
        ));
    }

    // 5. Unseal.
    let result = tpm2_unseal(&dev, obj_handle, session);

    // 6. Clean up.
    let _ = tpm2_flush_context(&dev, session);
    let _ = tpm2_flush_context(&dev, obj_handle);
    let _ = tpm2_flush_context(&dev, srk_handle);

    result.map_err(|e| format!("Unseal failed: {e}"))
}

/// Check whether a TPM2 device is available for sealing operations.
pub fn is_tpm2_available() -> bool {
    Path::new("/dev/tpmrm0").exists() || Path::new("/dev/tpm0").exists()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // ---- Serialization / deserialization ----

    #[test]
    fn test_sealed_blob_roundtrip() {
        let blob = Tpm2SealedBlob {
            pcr_mask: 0x0080, // PCR 7
            pcr_bank: TPM2_ALG_SHA256,
            primary_alg: TPM2_ALG_ECC,
            private: vec![1, 2, 3, 4, 5],
            public: vec![10, 20, 30],
        };

        let serialized = blob.serialize();
        let (deserialized, consumed) = Tpm2SealedBlob::deserialize(&serialized).unwrap();

        assert_eq!(consumed, serialized.len());
        assert_eq!(deserialized.pcr_mask, 0x0080);
        assert_eq!(deserialized.pcr_bank, TPM2_ALG_SHA256);
        assert_eq!(deserialized.primary_alg, TPM2_ALG_ECC);
        assert_eq!(deserialized.private, vec![1, 2, 3, 4, 5]);
        assert_eq!(deserialized.public, vec![10, 20, 30]);
    }

    #[test]
    fn test_sealed_blob_empty_data() {
        let blob = Tpm2SealedBlob {
            pcr_mask: 0,
            pcr_bank: TPM2_ALG_SHA256,
            primary_alg: TPM2_ALG_RSA,
            private: vec![],
            public: vec![],
        };

        let serialized = blob.serialize();
        let (deserialized, consumed) = Tpm2SealedBlob::deserialize(&serialized).unwrap();
        assert_eq!(consumed, serialized.len());
        assert!(deserialized.private.is_empty());
        assert!(deserialized.public.is_empty());
    }

    #[test]
    fn test_sealed_blob_large_data() {
        let blob = Tpm2SealedBlob {
            pcr_mask: 0x00FF,
            pcr_bank: TPM2_ALG_SHA256,
            primary_alg: TPM2_ALG_ECC,
            private: vec![0xAB; 512],
            public: vec![0xCD; 256],
        };

        let serialized = blob.serialize();
        let (deserialized, consumed) = Tpm2SealedBlob::deserialize(&serialized).unwrap();
        assert_eq!(consumed, serialized.len());
        assert_eq!(deserialized.private.len(), 512);
        assert_eq!(deserialized.public.len(), 256);
    }

    #[test]
    fn test_sealed_blob_deserialize_too_short() {
        let data = vec![0u8; 10]; // less than 16 bytes
        assert!(Tpm2SealedBlob::deserialize(&data).is_err());
    }

    #[test]
    fn test_sealed_blob_deserialize_truncated_private() {
        let mut data = Vec::new();
        data.extend_from_slice(&0x80u32.to_le_bytes()); // pcr_mask
        data.extend_from_slice(&TPM2_ALG_SHA256.to_le_bytes()); // pcr_bank
        data.extend_from_slice(&TPM2_ALG_ECC.to_le_bytes()); // primary_alg
        data.extend_from_slice(&100u32.to_le_bytes()); // private_len = 100
        data.extend_from_slice(&[0u8; 10]); // only 10 bytes, not 100

        assert!(Tpm2SealedBlob::deserialize(&data).is_err());
    }

    #[test]
    fn test_sealed_blob_deserialize_truncated_public() {
        let mut data = Vec::new();
        data.extend_from_slice(&0x80u32.to_le_bytes());
        data.extend_from_slice(&TPM2_ALG_SHA256.to_le_bytes());
        data.extend_from_slice(&TPM2_ALG_ECC.to_le_bytes());
        data.extend_from_slice(&2u32.to_le_bytes()); // private_len = 2
        data.extend_from_slice(&[1, 2]); // private data
        data.extend_from_slice(&50u32.to_le_bytes()); // public_len = 50
        data.extend_from_slice(&[0u8; 10]); // only 10 bytes

        assert!(Tpm2SealedBlob::deserialize(&data).is_err());
    }

    #[test]
    fn test_sealed_blob_with_trailing_data() {
        let blob = Tpm2SealedBlob {
            pcr_mask: 0x0080,
            pcr_bank: TPM2_ALG_SHA256,
            primary_alg: TPM2_ALG_ECC,
            private: vec![1, 2, 3],
            public: vec![4, 5],
        };

        let mut serialized = blob.serialize();
        let expected_consumed = serialized.len();
        // Append trailing data (e.g., the IV and ciphertext that follow in the
        // credential wire format).
        serialized.extend_from_slice(&[0xFF; 100]);

        let (deserialized, consumed) = Tpm2SealedBlob::deserialize(&serialized).unwrap();
        assert_eq!(consumed, expected_consumed);
        assert_eq!(deserialized.private, vec![1, 2, 3]);
        assert_eq!(deserialized.public, vec![4, 5]);
    }

    // ---- PCR selection ----

    #[test]
    fn test_build_pcr_selection_pcr7() {
        let sel = build_pcr_selection(1 << 7, TPM2_ALG_SHA256);
        // count(4) + hash(2) + sizeofSelect(1) + select(3) = 10
        assert_eq!(sel.len(), 10);
        // count = 1
        assert_eq!(u32::from_be_bytes(sel[0..4].try_into().unwrap()), 1);
        // hash = SHA256
        assert_eq!(
            u16::from_be_bytes(sel[4..6].try_into().unwrap()),
            TPM2_ALG_SHA256
        );
        // sizeofSelect = 3
        assert_eq!(sel[6], 3);
        // PCR 7 → bit 7 → byte 0 bit 7 = 0x80
        assert_eq!(sel[7], 0x80);
        assert_eq!(sel[8], 0x00);
        assert_eq!(sel[9], 0x00);
    }

    #[test]
    fn test_build_pcr_selection_empty() {
        let sel = build_pcr_selection(0, TPM2_ALG_SHA256);
        assert_eq!(sel.len(), 4);
        assert_eq!(u32::from_be_bytes(sel[0..4].try_into().unwrap()), 0);
    }

    #[test]
    fn test_build_pcr_selection_multiple() {
        // PCRs 0, 2, 7
        let mask = (1 << 0) | (1 << 2) | (1 << 7);
        let sel = build_pcr_selection(mask, TPM2_ALG_SHA256);
        assert_eq!(sel.len(), 10);
        // byte 0: bits 0,2,7 → 0x85
        assert_eq!(sel[7], 0x85);
    }

    #[test]
    fn test_build_pcr_selection_high_pcrs() {
        // PCR 16 (in byte 2)
        let mask = 1 << 16;
        let sel = build_pcr_selection(mask, TPM2_ALG_SHA256);
        assert_eq!(sel[7], 0x00); // byte 0
        assert_eq!(sel[8], 0x00); // byte 1
        assert_eq!(sel[9], 0x01); // byte 2, bit 0
    }

    // ---- Policy digest computation ----

    #[test]
    fn test_compute_policy_pcr_digest_deterministic() {
        let pcr_values = vec![(7, vec![0u8; 32])];
        let d1 = compute_policy_pcr_digest(1 << 7, TPM2_ALG_SHA256, &pcr_values);
        let d2 = compute_policy_pcr_digest(1 << 7, TPM2_ALG_SHA256, &pcr_values);
        assert_eq!(d1, d2);
    }

    #[test]
    fn test_compute_policy_pcr_digest_different_pcr_values() {
        let v1 = vec![(7, vec![0u8; 32])];
        let v2 = vec![(7, vec![1u8; 32])];
        let d1 = compute_policy_pcr_digest(1 << 7, TPM2_ALG_SHA256, &v1);
        let d2 = compute_policy_pcr_digest(1 << 7, TPM2_ALG_SHA256, &v2);
        assert_ne!(d1, d2);
    }

    #[test]
    fn test_compute_policy_pcr_digest_different_masks() {
        let v1 = vec![(7, vec![0u8; 32])];
        let v2 = vec![(0, vec![0u8; 32])];
        let d1 = compute_policy_pcr_digest(1 << 7, TPM2_ALG_SHA256, &v1);
        let d2 = compute_policy_pcr_digest(1 << 0, TPM2_ALG_SHA256, &v2);
        assert_ne!(d1, d2);
    }

    #[test]
    fn test_compute_policy_digest_is_sha256() {
        let pcr_values = vec![(7, vec![0xAB; 32])];
        let digest = compute_policy_pcr_digest(1 << 7, TPM2_ALG_SHA256, &pcr_values);
        assert_eq!(digest.len(), 32);
    }

    // ---- CmdBuilder ----

    #[test]
    fn test_cmd_builder_basic() {
        let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, 0x12345678);
        cmd.put_u32(0xDEADBEEF);
        let buf = cmd.finalize();

        // tag
        assert_eq!(u16::from_be_bytes([buf[0], buf[1]]), TPM2_ST_NO_SESSIONS);
        // size = 10 (header) + 4 = 14
        assert_eq!(u32::from_be_bytes([buf[2], buf[3], buf[4], buf[5]]), 14);
        // CC
        assert_eq!(
            u32::from_be_bytes([buf[6], buf[7], buf[8], buf[9]]),
            0x12345678
        );
        // data
        assert_eq!(
            u32::from_be_bytes([buf[10], buf[11], buf[12], buf[13]]),
            0xDEADBEEF
        );
    }

    #[test]
    fn test_cmd_builder_tpm2b() {
        let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, 0x00000001);
        cmd.put_tpm2b(&[0xAA, 0xBB, 0xCC]);
        let buf = cmd.finalize();

        // After the 10-byte header: u16 size=3, then 3 bytes
        assert_eq!(u16::from_be_bytes([buf[10], buf[11]]), 3);
        assert_eq!(&buf[12..15], &[0xAA, 0xBB, 0xCC]);
    }

    #[test]
    fn test_cmd_builder_pw_auth() {
        let mut cmd = CmdBuilder::new(TPM2_ST_SESSIONS, 0x00000001);
        cmd.put_pw_auth();
        let buf = cmd.finalize();

        // After 10-byte header:
        // authSize: u32 = 9
        assert_eq!(u32::from_be_bytes([buf[10], buf[11], buf[12], buf[13]]), 9);
        // sessionHandle: TPM2_RS_PW
        assert_eq!(
            u32::from_be_bytes([buf[14], buf[15], buf[16], buf[17]]),
            TPM2_RS_PW
        );
        // nonce size: 0
        assert_eq!(u16::from_be_bytes([buf[18], buf[19]]), 0);
        // sessionAttributes: 0x01
        assert_eq!(buf[20], 0x01);
        // hmac size: 0
        assert_eq!(u16::from_be_bytes([buf[21], buf[22]]), 0);
    }

    // ---- RespParser ----

    #[test]
    fn test_resp_parser_success() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes()); // tag
        resp.extend_from_slice(&18u32.to_be_bytes()); // size
        resp.extend_from_slice(&TPM2_RC_SUCCESS.to_be_bytes()); // rc
        resp.extend_from_slice(&0xCAFEBABEu32.to_be_bytes()); // data
        resp.extend_from_slice(&0x1234u16.to_be_bytes()); // more data
        resp.extend_from_slice(&[0xAA, 0xBB]); // more data

        let mut parser = RespParser::new(&resp).unwrap();
        assert_eq!(parser.get_u32().unwrap(), 0xCAFEBABE);
        assert_eq!(parser.get_u16().unwrap(), 0x1234);
        assert_eq!(parser.get_bytes(2).unwrap(), &[0xAA, 0xBB]);
    }

    #[test]
    fn test_resp_parser_error_code() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        resp.extend_from_slice(&10u32.to_be_bytes());
        resp.extend_from_slice(&0x00000100u32.to_be_bytes()); // error code

        let result = RespParser::new(&resp);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("0x00000100"));
    }

    #[test]
    fn test_resp_parser_too_short() {
        let resp = vec![0u8; 5]; // less than 10 bytes
        assert!(RespParser::new(&resp).is_err());
    }

    #[test]
    fn test_resp_parser_get_tpm2b() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        resp.extend_from_slice(&17u32.to_be_bytes()); // size = 10 + 2 + 5
        resp.extend_from_slice(&TPM2_RC_SUCCESS.to_be_bytes());
        resp.extend_from_slice(&5u16.to_be_bytes()); // TPM2B size
        resp.extend_from_slice(&[1, 2, 3, 4, 5]); // TPM2B data

        let mut parser = RespParser::new(&resp).unwrap();
        let data = parser.get_tpm2b().unwrap();
        assert_eq!(data, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn test_resp_parser_underflow() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        resp.extend_from_slice(&10u32.to_be_bytes()); // size = header only
        resp.extend_from_slice(&TPM2_RC_SUCCESS.to_be_bytes());

        let mut parser = RespParser::new(&resp).unwrap();
        assert!(parser.get_u32().is_err());
    }

    // ---- SRK template builders ----

    #[test]
    fn test_ecc_srk_template_structure() {
        let template = build_ecc_srk_template();
        // Check type field
        assert_eq!(u16::from_be_bytes([template[0], template[1]]), TPM2_ALG_ECC);
        // Check nameAlg
        assert_eq!(
            u16::from_be_bytes([template[2], template[3]]),
            TPM2_ALG_SHA256
        );
        // Check objectAttributes
        assert_eq!(
            u32::from_be_bytes([template[4], template[5], template[6], template[7]]),
            SRK_ATTRIBUTES
        );
    }

    #[test]
    fn test_rsa_srk_template_structure() {
        let template = build_rsa_srk_template();
        // Check type field
        assert_eq!(u16::from_be_bytes([template[0], template[1]]), TPM2_ALG_RSA);
        // Check nameAlg
        assert_eq!(
            u16::from_be_bytes([template[2], template[3]]),
            TPM2_ALG_SHA256
        );
    }

    #[test]
    fn test_ecc_srk_template_has_p256_curve() {
        let template = build_ecc_srk_template();
        // After: type(2) + nameAlg(2) + attrs(4) + authPolicy_size(2) +
        //        sym_alg(2) + sym_bits(2) + sym_mode(2) + scheme(2) = 18
        // curveID is at offset 18
        assert_eq!(
            u16::from_be_bytes([template[18], template[19]]),
            TPM2_ECC_NIST_P256
        );
    }

    #[test]
    fn test_rsa_srk_template_has_2048_bits() {
        let template = build_rsa_srk_template();
        // After: type(2) + nameAlg(2) + attrs(4) + authPolicy_size(2) +
        //        sym_alg(2) + sym_bits(2) + sym_mode(2) + scheme(2) = 18
        // keyBits is at offset 18
        assert_eq!(u16::from_be_bytes([template[18], template[19]]), 2048);
    }

    // ---- Hex decoding ----

    #[test]
    fn test_hex_to_bytes() {
        assert_eq!(hex_to_bytes("00ff").unwrap(), vec![0x00, 0xFF]);
        assert_eq!(hex_to_bytes("").unwrap(), Vec::<u8>::new());
        assert!(hex_to_bytes("0").is_err()); // odd length
    }

    // ---- TPM2 availability ----

    #[test]
    fn test_is_tpm2_available_no_panic() {
        // Should not panic regardless of whether TPM2 is present.
        let _ = is_tpm2_available();
    }

    // ---- Default PCR mask ----

    #[test]
    fn test_default_pcr_mask() {
        assert_eq!(DEFAULT_PCR_MASK, 0x80); // bit 7
        assert_eq!(DEFAULT_PCR_MASK, 1 << 7);
    }

    // ---- Constants ----

    #[test]
    fn test_tpm2_constants() {
        assert_eq!(TPM2_ST_NO_SESSIONS, 0x8001);
        assert_eq!(TPM2_ST_SESSIONS, 0x8002);
        assert_eq!(TPM2_CC_CREATE_PRIMARY, 0x0000_0131);
        assert_eq!(TPM2_CC_CREATE, 0x0000_0153);
        assert_eq!(TPM2_CC_LOAD, 0x0000_0157);
        assert_eq!(TPM2_CC_UNSEAL, 0x0000_015E);
        assert_eq!(TPM2_CC_FLUSH_CONTEXT, 0x0000_0165);
        assert_eq!(TPM2_CC_START_AUTH_SESSION, 0x0000_0176);
        assert_eq!(TPM2_CC_POLICY_PCR, 0x0000_017F);
        assert_eq!(TPM2_CC_PCR_READ, 0x0000_017E);
        assert_eq!(TPM2_ALG_ECC, 0x0023);
        assert_eq!(TPM2_ALG_RSA, 0x0001);
        assert_eq!(TPM2_ALG_SHA256, 0x000B);
    }

    #[test]
    fn test_srk_attributes() {
        assert_ne!(SRK_ATTRIBUTES & TPMA_OBJECT_FIXED_TPM, 0);
        assert_ne!(SRK_ATTRIBUTES & TPMA_OBJECT_FIXED_PARENT, 0);
        assert_ne!(SRK_ATTRIBUTES & TPMA_OBJECT_SENSITIVE_DATA_ORIGIN, 0);
        assert_ne!(SRK_ATTRIBUTES & TPMA_OBJECT_USER_WITH_AUTH, 0);
        assert_ne!(SRK_ATTRIBUTES & TPMA_OBJECT_NO_DA, 0);
        assert_ne!(SRK_ATTRIBUTES & TPMA_OBJECT_RESTRICTED, 0);
        assert_ne!(SRK_ATTRIBUTES & TPMA_OBJECT_DECRYPT, 0);
    }

    #[test]
    fn test_sealed_attributes() {
        assert_ne!(SEALED_ATTRIBUTES & TPMA_OBJECT_FIXED_TPM, 0);
        assert_ne!(SEALED_ATTRIBUTES & TPMA_OBJECT_FIXED_PARENT, 0);
        assert_ne!(SEALED_ATTRIBUTES & TPMA_OBJECT_NO_DA, 0);
        // Sealed data should NOT have these
        assert_eq!(SEALED_ATTRIBUTES & TPMA_OBJECT_USER_WITH_AUTH, 0);
        assert_eq!(SEALED_ATTRIBUTES & TPMA_OBJECT_RESTRICTED, 0);
        assert_eq!(SEALED_ATTRIBUTES & TPMA_OBJECT_DECRYPT, 0);
    }

    // ---- Edge cases for nonce generation ----

    #[test]
    fn test_generate_nonce_correct_length() {
        let nonce = generate_nonce();
        assert_eq!(nonce.len(), 32);
    }

    // ---- Empty PCR selection ----

    #[test]
    fn test_build_empty_pcr_selection() {
        let sel = build_empty_pcr_selection();
        assert_eq!(sel.len(), 4);
        assert_eq!(u32::from_be_bytes(sel[0..4].try_into().unwrap()), 0);
    }

    // ---- CmdBuilder edge cases ----

    #[test]
    fn test_cmd_builder_empty_tpm2b() {
        let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, 0x00000001);
        cmd.put_tpm2b(&[]);
        let buf = cmd.finalize();
        // After 10-byte header: u16 size=0
        assert_eq!(u16::from_be_bytes([buf[10], buf[11]]), 0);
        assert_eq!(buf.len(), 12);
    }

    #[test]
    fn test_cmd_builder_put_u8() {
        let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, 0x00000001);
        cmd.put_u8(0x42);
        let buf = cmd.finalize();
        assert_eq!(buf[10], 0x42);
        assert_eq!(buf.len(), 11);
    }

    #[test]
    fn test_cmd_builder_put_u16() {
        let mut cmd = CmdBuilder::new(TPM2_ST_NO_SESSIONS, 0x00000001);
        cmd.put_u16(0x1234);
        let buf = cmd.finalize();
        assert_eq!(buf[10], 0x12);
        assert_eq!(buf[11], 0x34);
    }

    #[test]
    fn test_cmd_builder_policy_auth() {
        let session_handle = 0x03000001u32;
        let mut cmd = CmdBuilder::new(TPM2_ST_SESSIONS, 0x00000001);
        cmd.put_policy_auth(session_handle);
        let buf = cmd.finalize();

        // authSize = 9
        assert_eq!(u32::from_be_bytes([buf[10], buf[11], buf[12], buf[13]]), 9);
        // sessionHandle
        assert_eq!(
            u32::from_be_bytes([buf[14], buf[15], buf[16], buf[17]]),
            session_handle
        );
    }

    // ---- resp_parser skip ----

    #[test]
    fn test_resp_parser_skip() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        resp.extend_from_slice(&16u32.to_be_bytes());
        resp.extend_from_slice(&TPM2_RC_SUCCESS.to_be_bytes());
        resp.extend_from_slice(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF]);

        let mut parser = RespParser::new(&resp).unwrap();
        parser.skip(4).unwrap();
        assert_eq!(parser.get_bytes(2).unwrap(), &[0xEE, 0xFF]);
    }

    #[test]
    fn test_resp_parser_skip_overflow() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        resp.extend_from_slice(&12u32.to_be_bytes());
        resp.extend_from_slice(&TPM2_RC_SUCCESS.to_be_bytes());
        resp.extend_from_slice(&[0xAA, 0xBB]);

        let mut parser = RespParser::new(&resp).unwrap();
        assert!(parser.skip(10).is_err());
    }

    #[test]
    fn test_resp_parser_remaining() {
        let mut resp = Vec::new();
        resp.extend_from_slice(&TPM2_ST_NO_SESSIONS.to_be_bytes());
        resp.extend_from_slice(&14u32.to_be_bytes());
        resp.extend_from_slice(&TPM2_RC_SUCCESS.to_be_bytes());
        resp.extend_from_slice(&[0u8; 4]);

        let parser = RespParser::new(&resp).unwrap();
        assert_eq!(parser.remaining(), 4);
    }

    // ---- Multiple PCR policy digest ----

    #[test]
    fn test_policy_pcr_digest_multiple_pcrs() {
        let pcr_values = vec![(0, vec![0xAAu8; 32]), (7, vec![0xBBu8; 32])];
        let mask = (1 << 0) | (1 << 7);
        let digest = compute_policy_pcr_digest(mask, TPM2_ALG_SHA256, &pcr_values);
        assert_eq!(digest.len(), 32);

        // Different order of concatenation should produce different digests
        let pcr_values_reversed = vec![(7, vec![0xBBu8; 32]), (0, vec![0xAAu8; 32])];
        let digest2 = compute_policy_pcr_digest(mask, TPM2_ALG_SHA256, &pcr_values_reversed);
        // The digests differ because concatenation order matters for pcrDigest
        assert_ne!(digest, digest2);
    }

    // ---- Tpm2Device::open without TPM2 ----

    #[test]
    fn test_tpm2_device_open_result() {
        // This test checks that open() returns a meaningful result.
        // On systems without TPM2, it should return an error.
        // On systems with TPM2, it should succeed.
        let result = Tpm2Device::open();
        if !is_tpm2_available() {
            assert!(result.is_err());
        }
        // If TPM2 is available, we don't actually transact (that would require
        // root privileges), so just verify the device path was found.
    }

    // ---- tpm2_seal_secret validation ----

    #[test]
    fn test_seal_empty_secret_rejected() {
        // Sealing requires a non-empty secret.
        let result = tpm2_seal_secret(&[], 1 << 7, TPM2_ALG_SHA256, TPM2_ALG_ECC);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("empty"));
    }

    #[test]
    fn test_seal_oversized_secret_rejected() {
        let big_secret = vec![0u8; 256]; // > 128 bytes
        let result = tpm2_seal_secret(&big_secret, 1 << 7, TPM2_ALG_SHA256, TPM2_ALG_ECC);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("too large"));
    }
}
