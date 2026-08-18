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

/// `com.atproto.repo.strongRef`: an immutable pointer to a specific record
/// version (its at-uri plus the CID of the exact bytes). Strings only.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct StrongRef {
    pub uri: String,
    pub cid: String,
}

/// A record shaped like the drafted `com.example.wiki.post` lexicon: the public
/// feed post. Strings and refs only (the no-floats rule is structural). Field
/// declaration order is irrelevant to the wire bytes: DAG-CBOR sorts map keys
/// canonically (length-first, then bytewise), which `serde_ipld_dagcbor` does.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiPost {
    #[serde(rename = "$type")]
    pub record_type: String,
    pub text: String,
    /// Optional at-uri of the public group/event this was posted in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<String>,
    /// Optional thread parent (a strongRef, per the lexicon).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply: Option<StrongRef>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl WikiPost {
    pub const NSID: &'static str = "com.example.wiki.post";
}

/// A record shaped like the drafted `com.example.wiki.comment` lexicon: a public
/// comment on a public content item, threaded via `parent`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiComment {
    #[serde(rename = "$type")]
    pub record_type: String,
    /// The public record this comments on (required).
    pub subject: StrongRef,
    pub text: String,
    /// Optional: the comment this replies to (threading).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<StrongRef>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl WikiComment {
    pub const NSID: &'static str = "com.example.wiki.comment";
}

/// A record shaped like the drafted `com.example.wiki.resolution` lexicon: the
/// published outcome of a motion/election (authored by the org's DID). `status`
/// is an OPEN string set (`knownValues`), never a breaking enum, so it is a
/// plain `String` carrying `carried`/`rejected`/`withdrawn` (or a future value).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiResolution {
    #[serde(rename = "$type")]
    pub record_type: String,
    pub title: String,
    /// Optional rendered resolution body.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    pub status: String,
    /// Optional at-uri of the public group/event this belongs to.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub context: Option<String>,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl WikiResolution {
    pub const NSID: &'static str = "com.example.wiki.resolution";
}

/// A record shaped like the drafted `com.example.wiki.reaction` lexicon: a
/// member's emoji reaction to a public item, addressed by a strongRef. Strings
/// and a ref only. `emoji` is a single grapheme cluster (may be a ZWJ sequence).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct WikiReaction {
    #[serde(rename = "$type")]
    pub record_type: String,
    /// The public record being reacted to.
    pub subject: StrongRef,
    /// The reaction emoji (one grapheme).
    pub emoji: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
}

impl WikiReaction {
    pub const NSID: &'static str = "com.example.wiki.reaction";
}
