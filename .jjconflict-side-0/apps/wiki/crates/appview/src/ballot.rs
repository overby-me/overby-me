//! The AppView's ballot-service seam: bind `ballot-store`'s durable board +
//! roster to THIS crate's Turso datastore. The pure scheme (`ballot-spec`) and
//! its persistent layer (`ballot-store`) were built and property-tested
//! standalone; this module is where they become part of the running AppView.
//!
//! Two entry points:
//! - [`init_ballot_schema`] applies the board + roster DDL alongside the entity
//!   and runtime tables (called from [`crate::db::Db::init_schema`]). Both DDLs
//!   are `CREATE TABLE IF NOT EXISTS`, so this is idempotent on a persistent file.
//! - [`open_board`] hands out a [`PersistentBoard`] over a fresh connection, with
//!   the off-node replica sink attached when a replica-log path is configured
//!   (`config.ballot_replica_log`) so every committed cast is shipped to an
//!   independent node -- the load-bearing E2E-V integrity control.
//!
//! The XRPC ballot procedures (open a poll, issue a token, cast, tally) build on
//! this seam in a later slice; wiring the durable core in first keeps that slice
//! to handlers over an already-live store.

use crate::db::{Db, DbError};
use ballot_store::{BoardError, PersistentBoard, ReplicaLog};
use std::sync::Arc;

/// Apply the ballot DDL (public board + private roster) to the datastore. Both
/// halves are `CREATE TABLE IF NOT EXISTS`, so this is safe to run on every boot,
/// including an already-initialized persistent file. Runs after the entity and
/// runtime DDL in [`crate::db::Db::init_schema`].
pub async fn init_ballot_schema(db: &Db) -> Result<(), DbError> {
    let conn = db.acquire().await?;
    conn.execute_batch(ballot_store::BOARD_DDL).await?;
    conn.execute_batch(ballot_store::BALLOT_DDL).await?;
    Ok(())
}

/// Open a durable [`PersistentBoard`] over a fresh datastore connection. When
/// `replica_log` is a non-empty path, the off-node replica sink is attached so
/// every committed cast is appended to that append-only log (which
/// `ballot_store::transport` mirrors to an independent node); an empty path
/// disables replication (dev/tests). The board's own DDL is idempotent, so this
/// composes cleanly with [`init_ballot_schema`].
pub async fn open_board(db: &Db, replica_log: &str) -> Result<PersistentBoard, BallotError> {
    let conn = db.acquire().await?;
    let board = PersistentBoard::open(conn).await?;
    if replica_log.is_empty() {
        Ok(board)
    } else {
        let replica = Arc::new(ReplicaLog::open(replica_log)?);
        Ok(board.with_replication(replica))
    }
}

/// A ballot-wiring failure: the datastore, the board core, or the replica log.
#[derive(Debug)]
pub enum BallotError {
    Db(DbError),
    Board(BoardError),
    Replica(std::io::Error),
}

impl std::fmt::Display for BallotError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BallotError::Db(e) => write!(f, "ballot datastore error: {e}"),
            BallotError::Board(e) => write!(f, "ballot board error: {e}"),
            BallotError::Replica(e) => write!(f, "ballot replica-log error: {e}"),
        }
    }
}
impl std::error::Error for BallotError {}
impl From<DbError> for BallotError {
    fn from(e: DbError) -> Self {
        BallotError::Db(e)
    }
}
impl From<BoardError> for BallotError {
    fn from(e: BoardError) -> Self {
        BallotError::Board(e)
    }
}
impl From<std::io::Error> for BallotError {
    fn from(e: std::io::Error) -> Self {
        BallotError::Replica(e)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballot_spec::{BallotRules, BoardEntry, TokenIssuer, finalize_token, request_token};

    async fn seeded_db() -> Db {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("init schema");
        db
    }

    /// The ballot DDL lands alongside the entity tables: init_schema created the
    /// board and roster tables, so the AppView's datastore is ballot-ready.
    #[tokio::test(flavor = "current_thread")]
    async fn init_schema_creates_the_ballot_tables() {
        let db = seeded_db().await;
        let conn = db.acquire().await.expect("acquire");
        for table in [
            "board_nullifier",
            "board_body",
            "poll",
            "eligibility",
            "token_issued",
        ] {
            let mut rows = conn
                .query(
                    "SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1",
                    [table],
                )
                .await
                .expect("query");
            assert!(
                matches!(rows.next().await, Ok(Some(_))),
                "ballot table `{table}` should exist after init_schema"
            );
        }
    }

    /// End-to-end wiring: a board opened over the AppView's datastore accepts a
    /// real blind-signed cast, proving the durable core is live in-process (not
    /// just the DDL). No replica log here (empty path).
    #[tokio::test(flavor = "current_thread")]
    async fn board_over_the_appview_datastore_accepts_a_cast() {
        let db = seeded_db().await;
        let board = open_board(&db, "").await.expect("open board");
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let rules = BallotRules {
            options: 2,
            blank: false,
            min: 1,
            max: 1,
        };
        let pk = issuer.public_key();
        let req = request_token(pk).expect("request");
        let blind_sig = issuer
            .blind_sign(&req.blinding.blind_message)
            .expect("sign");
        let signature = finalize_token(pk, &req, &blind_sig).expect("finalize");
        let entry = BoardEntry {
            token: req.nullifier,
            msg_randomizer: req.blinding.msg_randomizer,
            signature,
            choices: vec![1],
        };
        board.cast(pk, &rules, entry).await.expect("cast");
        assert_eq!(board.entries().await.expect("entries").len(), 1);
    }
}
