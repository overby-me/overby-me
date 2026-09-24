//! The order-independent hash a permissioned repo is summed up by (proposal
//! 0016, "Commit digest"): LtHash over BLAKE3.

use sha2::{Digest, Sha256};

const LANES: usize = 1024;

/// 1024 lanes of `u16`, each record added or removed lane-wise with wraparound,
/// so the state depends on the set of records alone and a write costs one pass.
#[derive(Clone, PartialEq, Eq)]
pub struct SetHash([u16; LANES]);

impl Default for SetHash {
    fn default() -> Self {
        SetHash([0; LANES])
    }
}

impl std::fmt::Debug for SetHash {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "SetHash({:02x?})", &self.digest()[..4])
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

    /// The state itself, for a syncer to keep beside its copy between pulls.
    pub fn to_bytes(&self) -> Vec<u8> {
        self.0.iter().flat_map(|lane| lane.to_le_bytes()).collect()
    }

    pub fn from_bytes(bytes: &[u8]) -> Option<SetHash> {
        if bytes.len() != LANES * 2 {
            return None;
        }
        let mut out = [0u16; LANES];
        for (lane, pair) in out.iter_mut().zip(bytes.chunks_exact(2)) {
            *lane = u16::from_le_bytes([pair[0], pair[1]]);
        }
        Some(SetHash(out))
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_order_of_writes_leaves_no_trace() {
        let mut one = SetHash::default();
        one.add("c", "a", "cid1");
        one.add("c", "b", "cid2");
        one.add("c", "gone", "cid3");
        one.remove("c", "gone", "cid3");
        let mut other = SetHash::default();
        other.add("c", "b", "cid2");
        other.add("c", "a", "cid1");
        assert_eq!(one.digest(), other.digest());
        assert_ne!(one.digest(), SetHash::default().digest());
        assert_eq!(SetHash::from_bytes(&one.to_bytes()), Some(one));
    }
}
