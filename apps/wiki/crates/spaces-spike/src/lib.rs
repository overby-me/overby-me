//! The two checks a syncer of an atproto space makes of a permissioned repo
//! (proposal 0016, "Permissioned repos"): that its own copy holds exactly the
//! records the host's does, by an order-independent set hash, and that the
//! commit carrying that hash is the author's.

use hmac::{Hmac, Mac};
use sha2::{Digest, Sha256};

const LANES: usize = 1024;

/// LtHash over a repo's records: 1024 lanes of `u16`, each record added or
/// removed lane-wise with wraparound, so the state depends on the set alone.
#[derive(Clone, PartialEq, Eq)]
pub struct SetHash([u16; LANES]);

impl Default for SetHash {
    fn default() -> Self {
        SetHash([0; LANES])
    }
}

impl SetHash {
    pub fn add(&mut self, collection: &str, rkey: &str, cid: &str) {
        for (lane, by) in self.0.iter_mut().zip(lanes(collection, rkey, cid)) {
            *lane = lane.wrapping_add(by);
        }
    }

    pub fn remove(&mut self, collection: &str, rkey: &str, cid: &str) {
        for (lane, by) in self.0.iter_mut().zip(lanes(collection, rkey, cid)) {
            *lane = lane.wrapping_sub(by);
        }
    }

    /// What a commit carries: SHA-256 of the state as little-endian bytes.
    pub fn digest(&self) -> [u8; 32] {
        let mut hasher = Sha256::new();
        for lane in self.0 {
            hasher.update(lane.to_le_bytes());
        }
        hasher.finalize().into()
    }
}

/// A record's element, `{collection}/{rkey}/{cid}`, stretched to 2048 bytes by
/// BLAKE3 in XOF mode and read as little-endian lanes.
fn lanes(collection: &str, rkey: &str, cid: &str) -> [u16; LANES] {
    let mut stretched = [0u8; LANES * 2];
    blake3::Hasher::new()
        .update(format!("{collection}/{rkey}/{cid}").as_bytes())
        .finalize_xof()
        .fill(&mut stretched);
    let mut out = [0u16; LANES];
    for (lane, pair) in out.iter_mut().zip(stretched.chunks_exact(2)) {
        *lane = u16::from_le_bytes([pair[0], pair[1]]);
    }
    out
}

/// `com.atproto.space.defs#signedCommit`.
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

/// The signature says WHO committed and the MAC says WHAT. Only the first is
/// worth anything to a third party, which is the point: the author signs a
/// nonce, never the hash, so a leaked commit proves nothing of what they wrote.
pub fn verify(
    commit: &SignedCommit,
    space: &str,
    author: &str,
    author_key_multibase: &str,
) -> Result<(), CommitError> {
    let context = context(space, author, &commit.rev, &commit.ikm);
    verify_signature(author_key_multibase, &context, &commit.sig)?;

    // HKDF-Expand (RFC 5869 2.3) for one block: T(1) = HMAC(ikm, info || 0x01).
    let mut expand = Hmac::<Sha256>::new_from_slice(&commit.ikm).map_err(|_| CommitError::Mac)?;
    expand.update(&context);
    expand.update(&[1]);
    let key = expand.finalize().into_bytes();
    let mut mac = Hmac::<Sha256>::new_from_slice(&key).map_err(|_| CommitError::Mac)?;
    mac.update(&commit.hash);
    mac.verify_slice(&commit.mac).map_err(|_| CommitError::Mac)
}

fn verify_signature(multikey: &str, message: &[u8], sig: &[u8]) -> Result<(), CommitError> {
    let (_, bytes) = multibase::decode(multikey).map_err(|_| CommitError::Key)?;
    match bytes.as_slice() {
        // multicodec secp256k1-pub, then the compressed point
        [0xe7, 0x01, point @ ..] => {
            use k256::ecdsa::signature::Verifier;
            let key =
                k256::ecdsa::VerifyingKey::from_sec1_bytes(point).map_err(|_| CommitError::Key)?;
            let sig =
                k256::ecdsa::Signature::from_slice(sig).map_err(|_| CommitError::Signature)?;
            key.verify(message, &sig)
                .map_err(|_| CommitError::Signature)
        }
        // multicodec p256-pub
        [0x80, 0x24, point @ ..] => {
            use p256::ecdsa::signature::Verifier;
            let key =
                p256::ecdsa::VerifyingKey::from_sec1_bytes(point).map_err(|_| CommitError::Key)?;
            let sig =
                p256::ecdsa::Signature::from_slice(sig).map_err(|_| CommitError::Signature)?;
            key.verify(message, &sig)
                .map_err(|_| CommitError::Signature)
        }
        _ => Err(CommitError::Key),
    }
}
