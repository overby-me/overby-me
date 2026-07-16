//! Round-2 item 13: prove the decided DRISL/DAG-CBOR + CIDv1 encode path with
//! known-answer and round-trip vectors, using sample records shaped like the
//! drafted lexicons. A wrong CID means every published record is rejected or
//! mis-addressed by the network; this is the same class of load-bearing
//! unknown the blind-signature spike retired. The tests in `tests/` are the
//! exact encode path that migrates into the AppView publish seam.

use cid::Cid;
use multihash::Multihash;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// DAG-CBOR's multicodec code (structured data).
pub const DAG_CBOR_CODEC: u64 = 0x71;
/// SHA2-256's multihash code.
pub const SHA2_256: u64 = 0x12;

/// Encode a value as deterministic DAG-CBOR bytes.
pub fn encode<T: Serialize>(
    value: &T,
) -> Result<Vec<u8>, serde_ipld_dagcbor::EncodeError<std::collections::TryReserveError>> {
    serde_ipld_dagcbor::to_vec(value)
}

/// Decode DAG-CBOR bytes back into a value.
pub fn decode<'a, T: Deserialize<'a>>(
    bytes: &'a [u8],
) -> Result<T, serde_ipld_dagcbor::DecodeError<std::convert::Infallible>> {
    serde_ipld_dagcbor::from_slice(bytes)
}

/// The CIDv1 (dag-cbor, sha2-256) of an encoded record: exactly how atproto
/// addresses a record's bytes.
pub fn cid_of(bytes: &[u8]) -> Cid {
    let digest = Sha256::digest(bytes);
    let mh = Multihash::<64>::wrap(SHA2_256, &digest).expect("32-byte digest fits");
    Cid::new_v1(DAG_CBOR_CODEC, mh)
}

/// A sample record shaped like the drafted com.example.wiki.post lexicon
/// (integers and strings only: the no-floats rule is structural).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SamplePost {
    #[serde(rename = "$type")]
    pub record_type: String,
    pub text: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<String>,
}
