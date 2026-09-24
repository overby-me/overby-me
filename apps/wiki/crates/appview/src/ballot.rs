//! The AppView's ballot-service seam: bind `ballot-store`'s durable board +
//! roster to THIS crate's Turso datastore. The pure scheme (`ballot-spec`) and
//! its persistent layer (`ballot-store`) were built and property-tested
//! standalone; this module is where they become part of the running AppView.
//! The procedures over it (open a poll, issue tokens, cast, tally) are
//! `crate::poll`.
//!
//! - [`init_ballot_schema`] applies the board + roster DDL alongside the entity
//!   and runtime tables (called from [`crate::db::Db::init_schema`]). Both DDLs
//!   are `CREATE TABLE IF NOT EXISTS`, so this is idempotent on a persistent file.
//! - [`open_replica`] opens the off-node replica log, once per process.
//! - [`board`] hands out one poll's [`PersistentBoard`] over a fresh connection,
//!   with that replica attached, so every committed cast is shipped to an
//!   independent node: the load-bearing E2E-V integrity control.

use crate::AppState;
use crate::db::{Db, DbError};
use ballot_store::{PersistentBoard, ReplicaLog};
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

/// The replica log at `path`, or none for an empty path (dev and tests).
///
/// Opened ONCE and shared: its appends are serialized by a lock inside it, so a
/// second handle on the same file would let two records interleave.
pub fn open_replica(path: &str) -> std::io::Result<Option<Arc<ReplicaLog>>> {
    if path.is_empty() {
        return Ok(None);
    }
    Ok(Some(Arc::new(ReplicaLog::open(path)?)))
}

/// The board of `poll_id`, over a connection of its own.
pub async fn board(state: &AppState, poll_id: &str) -> Result<PersistentBoard, DbError> {
    let board = PersistentBoard::attach(state.db.acquire().await?, poll_id);
    Ok(match &state.replica {
        Some(replica) => board.with_replication(replica.clone()),
        None => board,
    })
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

    /// End-to-end wiring: a board over the AppView's datastore accepts a real
    /// blind-signed cast and ships it to the configured replica, proving the
    /// durable core is live in-process (not just the DDL).
    #[tokio::test(flavor = "current_thread")]
    async fn board_over_the_appview_datastore_accepts_a_cast() {
        let log = std::env::temp_dir().join(format!(
            "appview-replica-{}.jsonl",
            crate::util::random_token(8)
        ));
        let mut state = AppState::new(seeded_db().await, crate::Config::default());
        state.replica = open_replica(&log.to_string_lossy()).expect("replica");
        let board = board(&state, "p1").await.expect("board");
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
        let shipped = std::fs::read_to_string(&log).expect("replica log");
        assert_eq!(shipped.lines().count(), 1);
        assert!(shipped.contains("\"poll\":\"p1\""), "{shipped}");
        assert!(open_replica("").expect("none").is_none());
    }
}
