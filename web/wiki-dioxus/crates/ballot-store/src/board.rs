//! The DURABLE public bulletin board (rewrite kickoff item 12): binds
//! `ballot_spec::Board`'s pure in-memory cast to the exact kill-9-proven atomic
//! transaction from `durability-harness` (BEGIN IMMEDIATE + a UNIQUE-token dedup
//! insert + an append-only body insert). The two halves were proven separately
//! and never joined; this joins them.
//!
//! What is pinned vs provisional: the DEDUP key (the unblinded unit token under
//! `UNIQUE`) and the monotonic position are load-bearing and named. The rest of
//! the entry (`msg_randomizer`, `signature`, `choices`) is stored as an OPAQUE
//! provisional blob, NOT named columns, because those byte encodings are marked
//! PROVISIONAL (DECISIONS.md D7) until item 9 (the board/poll record design)
//! pins them. So this store commits to the integrity-bearing shape while leaving
//! the wire encoding free.

use ballot_spec::{BallotRules, BoardEntry, CastError, IssuerPublicKey};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use turso::{Connection, Value};

/// The durable board schema, in the SQLite dialect (Turso's frontend and the
/// SQLite bridge). Two tables mirror the `durability-harness` dedup+body shape
/// so its kill-9 atomicity proof covers exactly these rows:
/// - `board_nullifier`: the spent token (UNIQUE = the double-spend rejection)
///   paired with its monotonic board position;
/// - `board_body`: the opaque provisional entry body, 1:1 with a position.
pub const BOARD_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS board_nullifier (
  token    BLOB PRIMARY KEY,          -- the unblinded unit token (dedup key)
  position INTEGER NOT NULL UNIQUE     -- monotonic board position
);
CREATE TABLE IF NOT EXISTS board_body (
  position INTEGER PRIMARY KEY,        -- pairs 1:1 with a nullifier's position
  body     BLOB NOT NULL               -- OPAQUE provisional (msg_randomizer, signature, choices)
);
"#;

/// The opaque, PROVISIONAL entry body. Serialized to bytes for `board_body`.
/// Deliberately a local struct built from the crypto types' raw bytes rather
/// than serde on `Signature`/`MessageRandomizer` themselves: the byte-level wire
/// encoding is item 9's decision (D7), so nothing here pins it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct ProvisionalBody {
    msg_randomizer: Option<Vec<u8>>,
    signature: Vec<u8>,
    choices: Vec<usize>,
}

fn encode_body(entry: &BoardEntry) -> Result<Vec<u8>, serde_json::Error> {
    let body = ProvisionalBody {
        msg_randomizer: entry.msg_randomizer.as_ref().map(|m| m.0.to_vec()),
        signature: entry.signature.0.clone(),
        choices: entry.choices.clone(),
    };
    serde_json::to_vec(&body)
}

/// The off-node replication SEAM, a boundary only (kickoff item 12): called
/// after a cast commits, so an append-only log can be shipped to an independent
/// replica (the load-bearing E2E-V integrity control). No wire format is decided
/// here; the WAL-shipping transport is the immediate next step after this crate.
pub trait ReplicationSink: Send + Sync {
    fn on_appended(&self, position: u64, token: &[u8], body: &[u8]);
}

/// A durable, single-poll board over a Turso database. `cast` runs the atomic
/// dedup+append transaction; an optional [`ReplicationSink`] observes commits.
pub struct PersistentBoard {
    conn: Connection,
    sink: Option<Arc<dyn ReplicationSink>>,
}

/// A cast failure: a domain rejection (bad signature / double spend / invalid
/// ballot, mirroring `ballot_spec::CastError`) or a store/encoding error.
#[derive(Debug)]
pub enum BoardError {
    Cast(CastError),
    Store(turso::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardError::Cast(e) => write!(f, "cast rejected: {e:?}"),
            BoardError::Store(e) => write!(f, "board store error: {e}"),
            BoardError::Json(e) => write!(f, "board body encoding error: {e}"),
        }
    }
}
impl std::error::Error for BoardError {}
impl From<turso::Error> for BoardError {
    fn from(e: turso::Error) -> Self {
        BoardError::Store(e)
    }
}
impl From<serde_json::Error> for BoardError {
    fn from(e: serde_json::Error) -> Self {
        BoardError::Json(e)
    }
}

impl PersistentBoard {
    /// Open a board over `conn`, creating the schema if absent.
    pub async fn open(conn: Connection) -> Result<Self, BoardError> {
        conn.execute_batch(BOARD_DDL).await?;
        Ok(Self { conn, sink: None })
    }

    /// Attach an off-node replication sink (called after each committed cast).
    pub fn with_replication(mut self, sink: Arc<dyn ReplicationSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    /// Verify and durably append a cast, returning the entry's board position.
    ///
    /// Check order matches `ballot_spec::Board::cast` in intent: signature
    /// first (a forged token can never probe the spent set), then ballot
    /// validity (an invalid ballot must not burn the token), then the atomic
    /// dedup+append. Double spend is caught inside `BEGIN IMMEDIATE` (which
    /// holds the write lock, so the check is race-free), with `UNIQUE(token)`
    /// as the enforced backstop.
    pub async fn cast(
        &self,
        pk: &IssuerPublicKey,
        rules: &BallotRules,
        entry: BoardEntry,
    ) -> Result<u64, BoardError> {
        pk.verify(&entry.signature, entry.msg_randomizer, &entry.token)
            .map_err(|_| BoardError::Cast(CastError::BadSignature))?;
        rules
            .validate(&entry.choices)
            .map_err(|e| BoardError::Cast(CastError::Invalid(e)))?;

        let body = encode_body(&entry)?;
        let token = Value::Blob(entry.token.clone());

        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        // Race-free double-spend check under the held write lock.
        let spent = {
            let mut rows = self
                .conn
                .query(
                    "SELECT 1 FROM board_nullifier WHERE token = ?1 LIMIT 1",
                    vec![token.clone()],
                )
                .await?;
            rows.next().await?.is_some()
        };
        if spent {
            self.conn.execute("ROLLBACK", ()).await.ok();
            return Err(BoardError::Cast(CastError::DoubleSpend));
        }
        let position = self.next_position().await?;
        if let Err(e) = self.append_locked(&token, position, &body).await {
            self.conn.execute("ROLLBACK", ()).await.ok();
            return Err(e);
        }
        self.conn.execute("COMMIT", ()).await?;

        if let Some(sink) = &self.sink {
            sink.on_appended(position, &entry.token, &body);
        }
        Ok(position)
    }

    async fn next_position(&self) -> Result<u64, BoardError> {
        let mut rows = self
            .conn
            .query(
                "SELECT coalesce(max(position), -1) + 1 FROM board_nullifier",
                (),
            )
            .await?;
        // An aggregate always returns exactly one row.
        let row = rows.next().await?.expect("aggregate returns a row");
        Ok(row.get::<i64>(0)? as u64)
    }

    /// The two dedup+append inserts, assuming a `BEGIN IMMEDIATE` is already in
    /// effect. `UNIQUE(token)` is the backstop double-spend guard.
    async fn append_locked(
        &self,
        token: &Value,
        position: u64,
        body: &[u8],
    ) -> Result<(), BoardError> {
        self.conn
            .execute(
                "INSERT INTO board_nullifier (token, position) VALUES (?1, ?2)",
                vec![token.clone(), Value::Integer(position as i64)],
            )
            .await?;
        self.conn
            .execute(
                "INSERT INTO board_body (position, body) VALUES (?1, ?2)",
                vec![Value::Integer(position as i64), Value::Blob(body.to_vec())],
            )
            .await?;
        Ok(())
    }

    /// The number of entries on the board (the tally is a plain count: every
    /// entry weighs exactly one unit token).
    pub async fn len(&self) -> Result<u64, BoardError> {
        let mut rows = self
            .conn
            .query("SELECT count(*) FROM board_nullifier", ())
            .await?;
        let row = rows.next().await?.expect("count returns a row");
        Ok(row.get::<i64>(0)? as u64)
    }

    /// Whether the board has no entries.
    pub async fn is_empty(&self) -> Result<bool, BoardError> {
        Ok(self.len().await? == 0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ballot_spec::{BallotRules, BoardEntry, TokenIssuer, finalize_token, request_token};
    use std::sync::Mutex;

    async fn mem_board() -> PersistentBoard {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build");
        PersistentBoard::open(db.connect().expect("connect"))
            .await
            .expect("open")
    }

    /// Mint a valid, castable entry (one blind-signed unit token) for `choices`.
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

    #[tokio::test(flavor = "current_thread")]
    async fn cast_appends_and_double_spend_is_rejected() {
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let rules = BallotRules {
            options: 3,
            blank: false,
            min: 1,
            max: 1,
        };
        let board = mem_board().await;

        let entry = valid_entry(&issuer, vec![0]);
        // First cast lands at position 0.
        let pos = board.cast(issuer.public_key(), &rules, entry.clone()).await;
        assert_eq!(pos.expect("first cast"), 0);
        assert_eq!(board.len().await.expect("len"), 1);

        // Re-casting the SAME token is a double spend (the dedup key collides).
        let again = board.cast(issuer.public_key(), &rules, entry).await;
        assert!(
            matches!(again, Err(BoardError::Cast(CastError::DoubleSpend))),
            "reused token rejected as double spend, got {again:?}"
        );
        assert_eq!(board.len().await.expect("len"), 1, "no second row written");

        // A fresh distinct token lands at the next position.
        let pos2 = board
            .cast(issuer.public_key(), &rules, valid_entry(&issuer, vec![1]))
            .await;
        assert_eq!(pos2.expect("second cast"), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn forged_and_invalid_are_rejected_without_burning() {
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let other = TokenIssuer::new_for_poll(2048).expect("keypair");
        let rules = BallotRules {
            options: 3,
            blank: false,
            min: 1,
            max: 1,
        };
        let board = mem_board().await;

        // A token signed by a DIFFERENT poll's key does not verify.
        let forged = valid_entry(&other, vec![0]);
        let bad = board.cast(issuer.public_key(), &rules, forged).await;
        assert!(matches!(
            bad,
            Err(BoardError::Cast(CastError::BadSignature))
        ));
        assert!(
            board.is_empty().await.expect("empty"),
            "forged cast wrote nothing"
        );

        // A validly-signed token with out-of-range choices is Invalid and, being
        // rejected before the append, writes nothing (the token is not burned).
        let entry = valid_entry(&issuer, vec![9]);
        let invalid = board.cast(issuer.public_key(), &rules, entry).await;
        assert!(matches!(
            invalid,
            Err(BoardError::Cast(CastError::Invalid(_)))
        ));
        assert!(board.is_empty().await.expect("still empty"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn replication_sink_observes_commits() {
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let rules = BallotRules {
            options: 2,
            blank: false,
            min: 1,
            max: 1,
        };
        #[derive(Default)]
        struct Recorder {
            positions: Mutex<Vec<u64>>,
        }
        impl ReplicationSink for Recorder {
            fn on_appended(&self, position: u64, _token: &[u8], _body: &[u8]) {
                self.positions.lock().unwrap().push(position);
            }
        }
        let rec = Arc::new(Recorder::default());
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build");
        let board = PersistentBoard::open(db.connect().expect("connect"))
            .await
            .expect("open")
            .with_replication(rec.clone());

        board
            .cast(issuer.public_key(), &rules, valid_entry(&issuer, vec![0]))
            .await
            .expect("cast");
        board
            .cast(issuer.public_key(), &rules, valid_entry(&issuer, vec![1]))
            .await
            .expect("cast");
        assert_eq!(*rec.positions.lock().unwrap(), vec![0, 1]);
    }
}
