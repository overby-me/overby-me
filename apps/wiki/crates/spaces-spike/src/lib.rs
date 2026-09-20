//! The spike asked whether a permissioned repo's set hash and signed commit
//! could be checked from Rust against what a real spaces PDS served
//! (`FINDINGS.md`). They could, and the code that did it grew into
//! `crates/atproto-spaces`. What is left here is the repo that PDS served, as a
//! fixture that crate is held to (`tests/fixture.rs`).

pub use atproto_spaces::{CommitError, SetHash, SignedCommit};

pub fn verify(
    commit: &SignedCommit,
    space: &str,
    author: &str,
    author_key: &str,
) -> Result<(), CommitError> {
    commit.verify(space, author, author_key)
}
