//! The function is stateless, so the ephemeral OAuth link context (PKCE
//! verifier, the DPoP private key, the resolved endpoints, the NHost user id,
//! ...) must survive the redirect to Bluesky and back. It is serialized,
//! encrypted (XChaCha20-Poly1305 with an HKDF-SHA256 key from `STATE_SECRET`) and
//! set as a short-lived cookie the browser carries to `/atproto/callback`.

use crate::util;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce, XChaCha20Poly1305, XNonce,
};
use hkdf::Hkdf;
use serde::{de::DeserializeOwned, Deserialize, Serialize};
use sha2::{Digest, Sha256};

pub const COOKIE_NAME: &str = "atproto_link";

#[derive(Serialize, Deserialize, Debug, PartialEq)]
pub struct LinkState {
    pub nhost_user_id: String,
    pub handle: String,
    pub did: String,
    /// The account's PDS base URL (where authenticated repo writes go).
    pub pds: String,
    pub token_endpoint: String,
    pub code_verifier: String,
    /// b64url of the DPoP key's 32-byte P-256 scalar.
    pub dpop_key: String,
    pub dpop_nonce: Option<String>,
    /// The random `state` value echoed back by the authorization server.
    pub oauth_state: String,
}

/// v1 cipher: an HKDF-SHA256-derived key (with a per-purpose `info` label for
/// domain separation, replacing the bare `SHA-256(secret)`) under XChaCha20-Poly1305,
/// whose 192-bit nonce removes the 96-bit random-nonce birthday ceiling of ChaCha20.
fn cipher_v1(secret: &str) -> XChaCha20Poly1305 {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(None, secret.as_bytes())
        .expand(b"atproto seal v1", &mut key)
        .expect("32 is a valid HKDF-SHA256 output length");
    XChaCha20Poly1305::new(Key::from_slice(&key))
}

/// Legacy v0 cipher (bare `SHA-256(secret)` key + ChaCha20-Poly1305, 12-byte nonce).
/// Kept only so blobs sealed before the v1 hardening (at-rest atproto sessions)
/// still decrypt; nothing new is sealed with it.
fn cipher_v0(secret: &str) -> ChaCha20Poly1305 {
    let key = Sha256::digest(secret.as_bytes());
    ChaCha20Poly1305::new(Key::from_slice(&key))
}

/// Encrypt any serializable value into an opaque, self-describing blob (a random
/// 24-byte XChaCha20 nonce prefix + AEAD ciphertext, base64url). Used for the
/// link-state cookie and the at-rest atproto session credentials.
pub fn seal<T: Serialize>(secret: &str, state: &T) -> Result<String, String> {
    let plaintext = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    let nonce_bytes = util::random_bytes(24);
    let ct = cipher_v1(secret)
        .encrypt(XNonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|e| e.to_string())?;
    let mut out = nonce_bytes;
    out.extend_from_slice(&ct);
    Ok(util::b64url(&out))
}

/// Decrypt a blob produced by [`seal`]. Tries the v1 format (24-byte XChaCha20 nonce
/// + HKDF key) first, then the legacy v0 format (12-byte ChaCha20 nonce + SHA-256
/// key), so sessions sealed before the hardening still open. AEAD authentication
/// makes the trial unambiguous: only the matching format decrypts.
pub fn open<T: DeserializeOwned>(secret: &str, blob: &str) -> Result<T, String> {
    let raw = util::b64url_decode(blob)?;
    let pt = open_v1(secret, &raw)
        .or_else(|| open_v0(secret, &raw))
        .ok_or("state cookie decrypt failed")?;
    serde_json::from_slice(&pt).map_err(|e| e.to_string())
}

fn open_v1(secret: &str, raw: &[u8]) -> Option<Vec<u8>> {
    let (nonce, ct) = raw.split_at_checked(24)?;
    cipher_v1(secret)
        .decrypt(XNonce::from_slice(nonce), ct)
        .ok()
}

fn open_v0(secret: &str, raw: &[u8]) -> Option<Vec<u8>> {
    let (nonce, ct) = raw.split_at_checked(12)?;
    cipher_v0(secret).decrypt(Nonce::from_slice(nonce), ct).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> LinkState {
        LinkState {
            nhost_user_id: "u1".into(),
            handle: "alice.bsky.social".into(),
            did: "did:plc:abc".into(),
            pds: "https://pds.example".into(),
            token_endpoint: "https://pds.example/oauth/token".into(),
            code_verifier: "v".into(),
            dpop_key: "kkk".into(),
            dpop_nonce: Some("n".into()),
            oauth_state: "s".into(),
        }
    }

    #[test]
    fn seal_open_roundtrip() {
        let st = sample();
        let blob = seal("secret", &st).unwrap();
        assert_eq!(open::<LinkState>("secret", &blob).unwrap(), st);
    }

    #[test]
    fn wrong_secret_fails() {
        let blob = seal("secret", &sample()).unwrap();
        assert!(open::<LinkState>("other", &blob).is_err());
    }

    #[test]
    fn opens_legacy_v0_blob() {
        // A blob sealed in the pre-hardening v0 format (SHA-256 key + ChaCha20,
        // 12-byte nonce) must still decrypt via the fallback, so at-rest atproto
        // sessions sealed before the upgrade are not lost.
        let st = sample();
        let plaintext = serde_json::to_vec(&st).unwrap();
        let nonce = util::random_bytes(12);
        let ct = cipher_v0("secret")
            .encrypt(Nonce::from_slice(&nonce), plaintext.as_ref())
            .unwrap();
        let mut out = nonce;
        out.extend_from_slice(&ct);
        let legacy_blob = util::b64url(&out);
        assert_eq!(open::<LinkState>("secret", &legacy_blob).unwrap(), st);
    }
}
