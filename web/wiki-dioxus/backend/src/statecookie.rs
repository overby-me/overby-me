//! The function is stateless, so the ephemeral OAuth link context (PKCE
//! verifier, the DPoP private key, the resolved endpoints, the NHost user id,
//! ...) must survive the redirect to Bluesky and back. It is serialized,
//! encrypted (ChaCha20-Poly1305 with a key derived from `STATE_SECRET`) and set
//! as a short-lived cookie the browser carries to `/atproto/callback`.

use crate::util;
use chacha20poly1305::{
    aead::{Aead, KeyInit},
    ChaCha20Poly1305, Key, Nonce,
};
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

fn cipher(secret: &str) -> ChaCha20Poly1305 {
    let key = Sha256::digest(secret.as_bytes());
    ChaCha20Poly1305::new(Key::from_slice(&key))
}

/// Encrypt any serializable value into an opaque, self-describing blob (a random
/// nonce prefix + AEAD ciphertext, base64url). Used for the link-state cookie and
/// the at-rest atproto session credentials.
pub fn seal<T: Serialize>(secret: &str, state: &T) -> Result<String, String> {
    let plaintext = serde_json::to_vec(state).map_err(|e| e.to_string())?;
    let nonce_bytes = util::random_bytes(12);
    let ct = cipher(secret)
        .encrypt(Nonce::from_slice(&nonce_bytes), plaintext.as_ref())
        .map_err(|e| e.to_string())?;
    let mut out = nonce_bytes;
    out.extend_from_slice(&ct);
    Ok(util::b64url(&out))
}

/// Decrypt a blob produced by [`seal`] back into a value of the expected type.
pub fn open<T: DeserializeOwned>(secret: &str, blob: &str) -> Result<T, String> {
    let raw = util::b64url_decode(blob)?;
    // 12-byte nonce prefix + AEAD ciphertext; `split_at_checked` guards the
    // untrusted length instead of panicking.
    let (nonce_bytes, ct) = raw.split_at_checked(12).ok_or("state cookie too short")?;
    let pt = cipher(secret)
        .decrypt(Nonce::from_slice(nonce_bytes), ct)
        .map_err(|_| "state cookie decrypt failed".to_string())?;
    serde_json::from_slice(&pt).map_err(|e| e.to_string())
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
}
