//! A repo as a real spaces PDS served it (`ghcr.io/bluesky-social/atproto`,
//! tag `pds-spaces-alpha`, 0.5.32, on 2026-09-20): three records written, one
//! edited, one deleted, then `listRepoOps` and `listRecords` with a space
//! credential. Made-up accounts on a PDS that ran on this machine.

use base64::Engine;
use serde::Deserialize;
use spaces_spike::{CommitError, SetHash, SignedCommit, verify};

#[derive(Deserialize)]
struct Bytes {
    #[serde(rename = "$bytes")]
    b64: String,
}

impl Bytes {
    fn get(&self) -> Vec<u8> {
        base64::engine::general_purpose::STANDARD_NO_PAD
            .decode(&self.b64)
            .expect("base64")
    }
}

#[derive(Deserialize)]
struct Commit {
    hash: Bytes,
    ikm: Bytes,
    sig: Bytes,
    mac: Bytes,
    rev: String,
}

#[derive(Deserialize)]
struct Op {
    collection: String,
    rkey: String,
    cid: Option<String>,
    prev: Option<String>,
}

#[derive(Deserialize)]
struct Record {
    collection: String,
    rkey: String,
    cid: String,
}

#[derive(Deserialize)]
struct Fixture {
    space: String,
    author: String,
    #[serde(rename = "publicKeyMultibase")]
    key: String,
    commit: Commit,
    ops: Vec<Op>,
    records: Vec<Record>,
}

fn fixture() -> (Fixture, SignedCommit) {
    let f: Fixture =
        serde_json::from_str(include_str!("../fixtures/commit.json")).expect("the fixture");
    let commit = SignedCommit {
        hash: f.commit.hash.get(),
        ikm: f.commit.ikm.get(),
        sig: f.commit.sig.get(),
        mac: f.commit.mac.get(),
        rev: f.commit.rev.clone(),
    };
    (f, commit)
}

#[test]
fn the_records_a_host_lists_hash_to_what_its_commit_says() {
    let (f, commit) = fixture();
    let mut held = SetHash::default();
    for r in &f.records {
        held.add(&r.collection, &r.rkey, &r.cid);
    }
    assert_eq!(
        held.digest().as_slice(),
        commit.hash,
        "two records are left"
    );
}

/// Incremental sync: the log has the edit and the delete in it, and following
/// it lands on the same state as listing what is there now.
#[test]
fn following_the_operation_log_arrives_at_the_same_hash() {
    let (f, commit) = fixture();
    let mut held = SetHash::default();
    for op in &f.ops {
        if let Some(prev) = &op.prev {
            held.remove(&op.collection, &op.rkey, prev);
        }
        if let Some(cid) = &op.cid {
            held.add(&op.collection, &op.rkey, cid);
        }
    }
    assert_eq!(held.digest().as_slice(), commit.hash);
    assert_eq!(f.ops.len(), 5, "three creates, an edit and a delete");
}

#[test]
fn the_commit_is_the_authors_and_for_this_hash_only() {
    let (f, commit) = fixture();
    assert_eq!(verify(&commit, &f.space, &f.author, &f.key), Ok(()));

    let elsewhere = f.space.replace("c-fixture", "c-another");
    assert_eq!(
        verify(&commit, &elsewhere, &f.author, &f.key),
        Err(CommitError::Signature),
        "a commit is for one space"
    );
    let mut forged = fixture().1;
    forged.hash[0] ^= 1;
    assert_eq!(
        verify(&forged, &f.space, &f.author, &f.key),
        Err(CommitError::Mac),
        "a host cannot serve another state under the author's signature"
    );
}
