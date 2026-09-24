//! Keeping a copy of one repo of a space (proposal 0016, "Sync"): follow the
//! log, hold the result to the signed commit, and start over when they differ.
//! Correctness rests on the hash and not on having seen every operation, so a
//! missed notification or a dropped log costs a transfer and nothing else.

use crate::client::{Auth, Host, Record};
use crate::{Error, SetHash};
use serde_json::Value;

/// What a syncer keeps of a repo between pulls, beside the records themselves.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Copy {
    /// The revision the copy is at. `None` before the first pull.
    pub rev: Option<String>,
    pub hash: SetHash,
}

#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Put {
        collection: String,
        rkey: String,
        cid: String,
        value: Value,
    },
    Delete {
        collection: String,
        rkey: String,
    },
}

#[derive(Debug, PartialEq)]
pub enum Pulled {
    /// The copy was already at the host's revision.
    Nothing,
    /// What changed since, in order.
    Changes(Vec<Change>),
    /// The log could not get the copy there: this is everything the repo
    /// holds, and whatever else the copy has is gone.
    Everything(Vec<Record>),
}

/// Bring `copy` to where the host is, and say what that took. `author_key` is
/// the `#atproto` key of the repo's DID, which the commit has to be signed by.
pub async fn pull(
    host: &Host,
    auth: Auth<'_>,
    space: &str,
    repo: &str,
    author_key: &str,
    copy: &mut Copy,
) -> Result<Pulled, Error> {
    if let Some(since) = copy.rev.clone() {
        // A host that no longer has `since` answers with an error or with a
        // log that does not add up. Both end in the full listing below.
        if let Ok((ops, Some(commit))) = host.ops_since(auth, space, repo, Some(&since)).await {
            let mut hash = copy.hash.clone();
            let mut changes = Vec::new();
            for op in &ops {
                if let Some(prev) = &op.prev {
                    hash.remove(&op.collection, &op.rkey, prev);
                }
                match (&op.cid, &op.value) {
                    (Some(cid), value) => {
                        hash.add(&op.collection, &op.rkey, cid);
                        // A state the path has since left comes without its
                        // value, and the operation that replaced it follows.
                        if let Some(value) = value {
                            changes.push(Change::Put {
                                collection: op.collection.clone(),
                                rkey: op.rkey.clone(),
                                cid: cid.clone(),
                                value: value.clone(),
                            });
                        }
                    }
                    (None, _) => changes.push(Change::Delete {
                        collection: op.collection.clone(),
                        rkey: op.rkey.clone(),
                    }),
                }
            }
            if hash.digest().as_slice() == commit.hash
                && commit.verify(space, repo, author_key).is_ok()
            {
                *copy = Copy {
                    rev: Some(commit.rev),
                    hash,
                };
                return Ok(match changes.is_empty() {
                    true => Pulled::Nothing,
                    false => Pulled::Changes(changes),
                });
            }
        }
    }

    // A write can land between the listing and the commit, so the two are
    // asked for until they agree, a few times.
    for _ in 0..4 {
        let records = host.records(auth, space, repo).await?;
        let commit = host.latest_commit(auth, space, repo).await?;
        let mut hash = SetHash::default();
        for record in &records {
            hash.add(&record.collection, &record.rkey, &record.cid);
        }
        if hash.digest().as_slice() != commit.hash {
            continue;
        }
        commit
            .verify(space, repo, author_key)
            .map_err(|e| Error::Unverified(format!("the commit of {repo}: {e:?}")))?;
        *copy = Copy {
            rev: Some(commit.rev),
            hash,
        };
        return Ok(Pulled::Everything(records));
    }
    Err(Error::Unverified(format!(
        "what {repo} lists never added up to its commit"
    )))
}
