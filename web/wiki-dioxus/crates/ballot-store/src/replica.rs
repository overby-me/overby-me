//! Off-node ballot-log replication (the load-bearing E2E-V integrity control,
//! `docs/atproto-stack-decisions.md`): ship the board's append-only entries to an
//! INDEPENDENT replica so the public board can be rebuilt if the primary Turso
//! file is lost. The durability harness only proves single-process crash
//! atomicity, not the loss of the whole node; a bulletin board that lives on one
//! file with no off-node copy degrades the E2E-V argument to trust-the-org.
//!
//! Item 12 built the durable board + the `ReplicationSink` seam; this adds the
//! concrete append-only replica log and the rebuild-from-replica recovery path.
//! [`ReplicaLog`] is an append-only JSONL file — one record per committed cast,
//! written synchronously in the fire-and-forget hook — which IS the shippable
//! artifact: an independent node mirrors this file (rsync / WAL-style shipping).
//! That TRANSPORT is deliberately out of scope here (no wire format is frozen);
//! the integrity mechanism (an append-only replica + a proven rebuild) is what
//! this module provides.

use crate::board::{BoardError, PersistentBoard, ReplicationSink};
use serde::{Deserialize, Serialize};
use std::fs::{File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::Path;
use std::sync::Mutex;

/// One replicated board entry: the position, the unit token (dedup key), and the
/// opaque provisional body. Serialized as one JSONL line in the replica log.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ReplicaRecord {
    position: u64,
    token: Vec<u8>,
    body: Vec<u8>,
}

/// An append-only replica log over a file: each committed board entry is
/// appended as one JSONL record. Synchronous (it fits the fire-and-forget
/// [`ReplicationSink`] hook, which is called after a cast commits), so a real
/// deployment ships this file to an independent node out of band.
pub struct ReplicaLog {
    file: Mutex<File>,
    last_error: Mutex<Option<String>>,
}

impl ReplicaLog {
    /// Open (create if absent) the append-only replica log at `path`.
    pub fn open(path: impl AsRef<Path>) -> std::io::Result<Self> {
        let file = OpenOptions::new().create(true).append(true).open(path)?;
        Ok(Self {
            file: Mutex::new(file),
            last_error: Mutex::new(None),
        })
    }

    /// The last append error, if any. The replication hook is fire-and-forget
    /// (it cannot fail a cast that already committed), so a caller that needs a
    /// durability confirmation for the replica checks this after a cast.
    pub fn last_error(&self) -> Option<String> {
        self.last_error.lock().unwrap().clone()
    }
}

impl ReplicationSink for ReplicaLog {
    fn on_appended(&self, position: u64, token: &[u8], body: &[u8]) {
        let rec = ReplicaRecord {
            position,
            token: token.to_vec(),
            body: body.to_vec(),
        };
        let line = match serde_json::to_string(&rec) {
            Ok(s) => s,
            Err(e) => {
                *self.last_error.lock().unwrap() = Some(e.to_string());
                return;
            }
        };
        let mut file = self.file.lock().unwrap();
        if let Err(e) = writeln!(file, "{line}").and_then(|()| file.flush()) {
            *self.last_error.lock().unwrap() = Some(e.to_string());
        }
    }
}

/// A rebuild-from-replica failure.
#[derive(Debug)]
pub enum RebuildError {
    Io(std::io::Error),
    Json(serde_json::Error),
    Board(BoardError),
}

impl std::fmt::Display for RebuildError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RebuildError::Io(e) => write!(f, "replica read error: {e}"),
            RebuildError::Json(e) => write!(f, "replica record parse error: {e}"),
            RebuildError::Board(e) => write!(f, "board restore error: {e}"),
        }
    }
}
impl std::error::Error for RebuildError {}
impl From<std::io::Error> for RebuildError {
    fn from(e: std::io::Error) -> Self {
        RebuildError::Io(e)
    }
}
impl From<serde_json::Error> for RebuildError {
    fn from(e: serde_json::Error) -> Self {
        RebuildError::Json(e)
    }
}
impl From<BoardError> for RebuildError {
    fn from(e: BoardError) -> Self {
        RebuildError::Board(e)
    }
}

/// Rebuild `board` (a fresh, empty [`PersistentBoard`]) from a replica log: read
/// every record in append order and [`PersistentBoard::restore_entry`] it at its
/// original position. Returns the number of entries restored. This is the
/// recovery path for a lost primary: the replica log is the source of truth.
pub async fn rebuild_from_replica(
    board: &PersistentBoard,
    log_path: impl AsRef<Path>,
) -> Result<u64, RebuildError> {
    let reader = BufReader::new(File::open(log_path)?);
    let mut restored = 0;
    for line in reader.lines() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        let rec: ReplicaRecord = serde_json::from_str(&line)?;
        board
            .restore_entry(rec.position, &rec.token, &rec.body)
            .await?;
        restored += 1;
    }
    Ok(restored)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballot_spec::{BallotRules, BoardEntry, TokenIssuer, finalize_token, request_token};
    use std::sync::Arc;

    fn rules() -> BallotRules {
        BallotRules {
            options: 3,
            blank: false,
            min: 1,
            max: 1,
        }
    }

    fn valid_entry(issuer: &TokenIssuer, choices: Vec<usize>) -> BoardEntry {
        let pk = issuer.public_key();
        let req = request_token(pk).expect("request");
        let blind_sig = issuer
            .blind_sign(&req.blinding.blind_message)
            .expect("blind sign");
        let signature = finalize_token(pk, &req, &blind_sig).expect("finalize");
        BoardEntry {
            token: req.nullifier,
            msg_randomizer: req.blinding.msg_randomizer,
            signature,
            choices,
        }
    }

    async fn fresh_board() -> PersistentBoard {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build");
        PersistentBoard::open(db.connect().expect("connect"))
            .await
            .expect("open")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn board_rebuilds_from_the_replica_log_after_primary_loss() {
        // A unique replica-log path for this test run.
        let log_path =
            std::env::temp_dir().join(format!("ballot-replica-{}.jsonl", std::process::id()));
        let _ = std::fs::remove_file(&log_path);

        // Primary board with the replica sink attached.
        let replica = Arc::new(ReplicaLog::open(&log_path).expect("open replica"));
        let primary = fresh_board().await.with_replication(replica.clone());
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");

        // Cast three distinct valid ballots: each lands on the primary AND is
        // shipped to the replica log by the sink.
        for choice in [0usize, 1, 2] {
            primary
                .cast(
                    issuer.public_key(),
                    &rules(),
                    valid_entry(&issuer, vec![choice]),
                )
                .await
                .expect("cast");
        }
        assert!(replica.last_error().is_none(), "replica had no write error");
        let original = primary.entries().await.expect("original entries");
        assert_eq!(original.len(), 3);

        // Simulate PRIMARY LOSS: the original Turso file is gone. Rebuild a fresh,
        // empty board purely from the replica log.
        let rebuilt = fresh_board().await;
        assert!(rebuilt.is_empty().await.expect("empty before rebuild"));
        let restored = rebuild_from_replica(&rebuilt, &log_path)
            .await
            .expect("rebuild");
        assert_eq!(restored, 3, "every replicated entry restored");

        // The rebuilt board matches the original byte-for-byte (positions +
        // tokens), so the public board survived the loss of the primary node.
        let recovered = rebuilt.entries().await.expect("rebuilt entries");
        assert_eq!(recovered, original, "rebuilt board == original board");

        // Rebuild is idempotent: re-running over the same log re-materializes
        // nothing new (UNIQUE(token) rejects the duplicates), so a retried
        // recovery is safe.
        let again = rebuild_from_replica(&rebuilt, &log_path).await;
        assert!(again.is_err(), "a second rebuild collides on the dedup key");
        assert_eq!(
            rebuilt.entries().await.expect("still 3"),
            original,
            "a failed re-rebuild left the board unchanged"
        );

        let _ = std::fs::remove_file(&log_path);
    }
}
