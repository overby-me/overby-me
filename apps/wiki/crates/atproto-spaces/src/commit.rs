//! The signed commit of a permissioned repo (proposal 0016, "Commit
//! signature").

use crate::keys;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;

/// `com.atproto.space.defs#signedCommit`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignedCommit {
    pub hash: Vec<u8>,
    pub ikm: Vec<u8>,
    pub sig: Vec<u8>,
    pub mac: Vec<u8>,
    pub rev: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum CommitError {
    /// The key is no P-256 or secp256k1 multikey.
    Key,
    /// Not the author's signature over this space, author, revision and nonce.
    Signature,
    /// The hash is not the one this commit was made for.
    Mac,
}

/// The bytes that are signed, and that key the MAC: a fixed tag, then each
/// field behind its big-endian `u16` length (TLS 1.3's vector encoding).
pub fn context(space: &str, author: &str, rev: &str, ikm: &[u8]) -> Vec<u8> {
    let mut out = b"atproto-space-v1".to_vec();
    for field in [space.as_bytes(), author.as_bytes(), rev.as_bytes(), ikm] {
        out.extend((field.len() as u16).to_be_bytes());
        out.extend(field);
    }
    out
}

impl SignedCommit {
    /// The signature says WHO committed and the MAC says WHAT. Only the first
    /// is worth anything to a third party, which is the point: the author
    /// signs a nonce, never the hash, so a leaked commit proves nothing of
    /// what they wrote.
    pub fn verify(&self, space: &str, author: &str, author_key: &str) -> Result<(), CommitError> {
        let context = context(space, author, &self.rev, &self.ikm);
        match keys::verifies(author_key, &context, &self.sig) {
            None => return Err(CommitError::Key),
            Some(false) => return Err(CommitError::Signature),
            Some(true) => {}
        }
        // HKDF-Expand (RFC 5869 2.3) for one block: T(1) = HMAC(ikm, info || 1).
        let mut expand = Hmac::<Sha256>::new_from_slice(&self.ikm).map_err(|_| CommitError::Mac)?;
        expand.update(&context);
        expand.update(&[1]);
        let key = expand.finalize().into_bytes();
        let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| CommitError::Mac)?;
        mac.update(&self.hash);
        mac.verify_slice(&self.mac).map_err(|_| CommitError::Mac)
    }
}

/// Bytes as XRPC's JSON has them: `{"$bytes": "<base64, unpadded>"}`.
#[derive(Deserialize)]
struct Bytes {
    #[serde(rename = "$bytes")]
    b64: String,
}

#[derive(Deserialize)]
struct Wire {
    hash: Bytes,
    ikm: Bytes,
    sig: Bytes,
    mac: Bytes,
    rev: String,
}

impl SignedCommit {
    pub fn from_json(value: &serde_json::Value) -> Option<SignedCommit> {
        let wire: Wire = serde_json::from_value(value.clone()).ok()?;
        let bytes = |b: &Bytes| {
            base64::engine::general_purpose::STANDARD_NO_PAD
                .decode(b.b64.trim_end_matches('='))
                .ok()
        };
        Some(SignedCommit {
            hash: bytes(&wire.hash)?,
            ikm: bytes(&wire.ikm)?,
            sig: bytes(&wire.sig)?,
            mac: bytes(&wire.mac)?,
            rev: wire.rev,
        })
    }
}
