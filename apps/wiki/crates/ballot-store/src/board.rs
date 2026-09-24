//! The DURABLE public bulletin board (rewrite kickoff item 12): binds
//! `ballot_spec::Board`'s pure in-memory cast to the exact kill-9-proven atomic
//! transaction from `durability-harness` (BEGIN IMMEDIATE + a UNIQUE-token dedup
//! insert + an append-only body insert). The two halves were proven separately
//! and never joined; this joins them.
//!
//! One store holds every poll's board: each row carries its `poll_id`, and a
//! [`PersistentBoard`] is a view of one poll's rows. Positions are monotonic
//! within a poll. A token is deduplicated within its poll, which is all the
//! scheme needs: a token signed for one poll never verifies under another's key.
//!
//! What is pinned vs provisional: the DEDUP key (the unblinded unit token under
//! `UNIQUE`) and the monotonic position are load-bearing and named. The rest of
//! the entry (`msg_randomizer`, `signature`, `choices`) is stored as an OPAQUE
//! provisional blob, NOT named columns, because those byte encodings are marked
//! PROVISIONAL (DECISIONS.md D7) until item 9 (the board/poll record design)
//! pins them. So this store commits to the integrity-bearing shape while leaving
//! the wire encoding free.

use ballot_spec::{
    BallotRules, BoardEntry, CastError, IssuerPublicKey, MessageRandomizer, Signature,
};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use turso::{Connection, Value};

/// The durable board schema, in the SQLite dialect (Turso's frontend and the
/// SQLite bridge). Two tables mirror the `durability-harness` dedup+body shape
/// so its kill-9 atomicity proof covers exactly these rows:
/// - `board_nullifier`: the spent token (UNIQUE within its poll = the
///   double-spend rejection) paired with its monotonic board position;
/// - `board_body`: the opaque provisional entry body, 1:1 with a position.
///
/// `board_closed` seals a board: a row here and `cast` appends nothing more.
pub const BOARD_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS board_nullifier (
  poll_id  TEXT NOT NULL,
  token    BLOB NOT NULL,             -- the unblinded unit token (dedup key)
  position INTEGER NOT NULL,          -- monotonic within the poll
  PRIMARY KEY (poll_id, token),
  UNIQUE (poll_id, position)
);
CREATE TABLE IF NOT EXISTS board_body (
  poll_id  TEXT NOT NULL,
  position INTEGER NOT NULL,          -- pairs 1:1 with a nullifier's position
  body     BLOB NOT NULL,             -- OPAQUE provisional (msg_randomizer, signature, choices)
  PRIMARY KEY (poll_id, position)
);
CREATE TABLE IF NOT EXISTS board_closed (
  poll_id   TEXT PRIMARY KEY,
  entries   INTEGER NOT NULL,         -- the board's length when it was sealed
  closed_at TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
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

fn decode_entry(token: Vec<u8>, body: &[u8]) -> Result<BoardEntry, BoardError> {
    let body: ProvisionalBody = serde_json::from_slice(body)?;
    let msg_randomizer = match body.msg_randomizer {
        Some(bytes) => Some(MessageRandomizer(
            bytes.try_into().map_err(|_| BoardError::Corrupt)?,
        )),
        None => None,
    };
    Ok(BoardEntry {
        token,
        msg_randomizer,
        signature: Signature(body.signature),
        choices: body.choices,
    })
}

fn blob(value: Value) -> Vec<u8> {
    match value {
        Value::Blob(bytes) => bytes,
        _ => Vec::new(),
    }
}

/// The off-node replication SEAM, a boundary only (kickoff item 12): called
/// after a cast commits, so an append-only log can be shipped to an independent
/// replica (the load-bearing E2E-V integrity control). No wire format is decided
/// here; the WAL-shipping transport is the immediate next step after this crate.
pub trait ReplicationSink: Send + Sync {
    fn on_appended(&self, poll_id: &str, position: u64, token: &[u8], body: &[u8]);
}

/// One poll's durable board over a Turso database. `cast` runs the atomic
/// dedup+append transaction; an optional [`ReplicationSink`] observes commits.
pub struct PersistentBoard {
    conn: Connection,
    poll_id: String,
    sink: Option<Arc<dyn ReplicationSink>>,
}

/// A cast failure: a domain rejection (bad signature / double spend / invalid
/// ballot, mirroring `ballot_spec::CastError`), a sealed board, or a
/// store/encoding error.
#[derive(Debug)]
pub enum BoardError {
    Cast(CastError),
    /// The board was sealed by [`PersistentBoard::close`].
    Closed,
    /// A stored body does not decode into an entry.
    Corrupt,
    Store(turso::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for BoardError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            BoardError::Cast(e) => write!(f, "cast rejected: {e:?}"),
            BoardError::Closed => write!(f, "the board is closed"),
            BoardError::Corrupt => write!(f, "a board entry does not decode"),
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
    /// Open the board of `poll_id` over `conn`, creating the schema if absent.
    pub async fn open(conn: Connection, poll_id: &str) -> Result<Self, BoardError> {
        conn.execute_batch(BOARD_DDL).await?;
        Ok(Self::attach(conn, poll_id))
    }

    /// The board of `poll_id` in a store that already has the schema: what a
    /// server uses per request, where re-running the DDL for every ballot of a
    /// room voting at once is work under the write lock for nothing.
    pub fn attach(conn: Connection, poll_id: &str) -> Self {
        Self {
            conn,
            poll_id: poll_id.to_string(),
            sink: None,
        }
    }

    /// Attach an off-node replication sink (called after each committed cast).
    pub fn with_replication(mut self, sink: Arc<dyn ReplicationSink>) -> Self {
        self.sink = Some(sink);
        self
    }

    fn poll(&self) -> Value {
        Value::Text(self.poll_id.clone())
    }

    /// Verify and durably append a cast, returning the entry's board position.
    ///
    /// Check order matches `ballot_spec::Board::cast` in intent: signature
    /// first (a forged token can never probe the spent set), then ballot
    /// validity (an invalid ballot must not burn the token), then the atomic
    /// dedup+append. The seal and the double spend are both checked inside
    /// `BEGIN IMMEDIATE` (which holds the write lock, so neither check can
    /// race a `close` or another cast), with `UNIQUE(poll_id, token)` as the
    /// enforced backstop.
    pub async fn cast(
        &self,
        pk: &IssuerPublicKey,
        rules: &BallotRules,
        entry: BoardEntry,
    ) -> Result<u64, BoardError> {
        pk.verify(&entry.signature, entry.msg_randomizer, &entry.token)
            .map_err(|_sig_err| BoardError::Cast(CastError::BadSignature))?;
        rules
            .validate(&entry.choices)
            .map_err(|e| BoardError::Cast(CastError::Invalid(e)))?;

        let body = encode_body(&entry)?;
        let token = Value::Blob(entry.token.clone());

        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        let appended = self.cast_locked(&token, &body).await;
        let end = if appended.is_ok() {
            "COMMIT"
        } else {
            "ROLLBACK"
        };
        self.conn.execute(end, ()).await?;
        let position = appended?;

        if let Some(sink) = &self.sink {
            sink.on_appended(&self.poll_id, position, &entry.token, &body);
        }
        Ok(position)
    }

    /// The checks and the append that must be one step, assuming a
    /// `BEGIN IMMEDIATE` is already in effect.
    async fn cast_locked(&self, token: &Value, body: &[u8]) -> Result<u64, BoardError> {
        if self.is_closed().await? {
            return Err(BoardError::Closed);
        }
        let spent = {
            let mut rows = self
                .conn
                .query(
                    "SELECT 1 FROM board_nullifier WHERE poll_id = ?1 AND token = ?2 LIMIT 1",
                    vec![self.poll(), token.clone()],
                )
                .await?;
            rows.next().await?.is_some()
        };
        if spent {
            return Err(BoardError::Cast(CastError::DoubleSpend));
        }
        let position = self.next_position().await?;
        self.append_locked(token, position, body).await?;
        Ok(position)
    }

    /// One past the highest position, not the count: a board rebuilt from a
    /// replica log with a record missing must not hand a position out twice.
    async fn next_position(&self) -> Result<u64, BoardError> {
        let mut rows = self
            .conn
            .query(
                "SELECT coalesce(max(position), -1) + 1 FROM board_nullifier WHERE poll_id = ?1",
                vec![self.poll()],
            )
            .await?;
        let row = rows.next().await?.expect("an aggregate returns a row");
        Ok(row.get::<i64>(0)? as u64)
    }

    /// Seal the board: no cast is appended after this returns. Returns the
    /// board's final length. Sealing twice changes nothing and answers the same.
    ///
    /// Taken under the same write lock as a cast, so a cast either landed
    /// before the seal and is counted, or finds the seal and is refused. There
    /// is no cast that a tally taken after this can miss.
    pub async fn close(&self) -> Result<u64, BoardError> {
        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        let sealed = self.close_locked().await;
        let end = if sealed.is_ok() { "COMMIT" } else { "ROLLBACK" };
        self.conn.execute(end, ()).await?;
        sealed
    }

    async fn close_locked(&self) -> Result<u64, BoardError> {
        let entries = self.len().await?;
        self.conn
            .execute(
                "INSERT INTO board_closed (poll_id, entries) VALUES (?1, ?2) \
                 ON CONFLICT(poll_id) DO NOTHING",
                vec![self.poll(), Value::Integer(entries as i64)],
            )
            .await?;
        Ok(entries)
    }

    pub async fn is_closed(&self) -> Result<bool, BoardError> {
        let mut rows = self
            .conn
            .query(
                "SELECT 1 FROM board_closed WHERE poll_id = ?1",
                vec![self.poll()],
            )
            .await?;
        Ok(rows.next().await?.is_some())
    }

    /// Restore a replicated entry at its ORIGINAL board position. Used by
    /// rebuild-from-replica (`crate::replica`): the entry is an already-verified,
    /// already-committed cast shipped from a trusted replica of THIS board, so
    /// restoration re-materializes the dedup + body rows WITHOUT re-checking the
    /// signature or ballot validity (those were checked when it was first cast).
    /// The atomic dedup+append shape is the same as `cast`, so the UNIQUE token
    /// still rejects a duplicate (an idempotent re-run of a rebuild is safe).
    pub async fn restore_entry(
        &self,
        position: u64,
        token: &[u8],
        body: &[u8],
    ) -> Result<(), BoardError> {
        let token = Value::Blob(token.to_vec());
        self.conn.execute("BEGIN IMMEDIATE", ()).await?;
        if let Err(e) = self.append_locked(&token, position, body).await {
            self.conn.execute("ROLLBACK", ()).await.ok();
            return Err(e);
        }
        self.conn.execute("COMMIT", ()).await?;
        Ok(())
    }

    /// The board's entries as `(position, token)` in board order (the audit
    /// read; see [`Self::ballots`] for the bodies).
    pub async fn entries(&self) -> Result<Vec<(u64, Vec<u8>)>, BoardError> {
        let mut rows = self
            .conn
            .query(
                "SELECT position, token FROM board_nullifier WHERE poll_id = ?1 \
                 ORDER BY position",
                vec![self.poll()],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push((row.get::<i64>(0)? as u64, blob(row.get_value(1)?)));
        }
        Ok(out)
    }

    /// Every entry decoded, ordered by TOKEN and not by position. A token is
    /// random, so this order says nothing; board order is the order people
    /// voted in, which a reader who watched the room could match to faces.
    pub async fn ballots(&self) -> Result<Vec<BoardEntry>, BoardError> {
        let mut rows = self
            .conn
            .query(
                "SELECT n.token, b.body FROM board_nullifier n \
                 JOIN board_body b ON b.poll_id = n.poll_id AND b.position = n.position \
                 WHERE n.poll_id = ?1 ORDER BY n.token",
                vec![self.poll()],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(decode_entry(
                blob(row.get_value(0)?),
                &blob(row.get_value(1)?),
            )?);
        }
        Ok(out)
    }

    /// The entry that spent `token`, with its position: how a voter finds their
    /// own ballot.
    pub async fn find(&self, token: &[u8]) -> Result<Option<(u64, BoardEntry)>, BoardError> {
        let mut rows = self
            .conn
            .query(
                "SELECT n.position, b.body FROM board_nullifier n \
                 JOIN board_body b ON b.poll_id = n.poll_id AND b.position = n.position \
                 WHERE n.poll_id = ?1 AND n.token = ?2",
                vec![self.poll(), Value::Blob(token.to_vec())],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let position = row.get::<i64>(0)? as u64;
        let entry = decode_entry(token.to_vec(), &blob(row.get_value(1)?))?;
        Ok(Some((position, entry)))
    }

    /// The two dedup+append inserts, assuming a `BEGIN IMMEDIATE` is already in
    /// effect. `UNIQUE(poll_id, token)` is the backstop double-spend guard.
    async fn append_locked(
        &self,
        token: &Value,
        position: u64,
        body: &[u8],
    ) -> Result<(), BoardError> {
        self.conn
            .execute(
                "INSERT INTO board_nullifier (poll_id, token, position) VALUES (?1, ?2, ?3)",
                vec![self.poll(), token.clone(), Value::Integer(position as i64)],
            )
            .await?;
        self.conn
            .execute(
                "INSERT INTO board_body (poll_id, position, body) VALUES (?1, ?2, ?3)",
                vec![
                    self.poll(),
                    Value::Integer(position as i64),
                    Value::Blob(body.to_vec()),
                ],
            )
            .await?;
        Ok(())
    }

    /// The number of entries on the board (the tally is a plain count: every
    /// entry weighs exactly one unit token).
    pub async fn len(&self) -> Result<u64, BoardError> {
        let mut rows = self
            .conn
            .query(
                "SELECT count(*) FROM board_nullifier WHERE poll_id = ?1",
                vec![self.poll()],
            )
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

    async fn mem_db() -> turso::Database {
        turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build")
    }

    async fn board_of(db: &turso::Database, poll_id: &str) -> PersistentBoard {
        PersistentBoard::open(db.connect().expect("connect"), poll_id)
            .await
            .expect("open")
    }

    async fn mem_board() -> PersistentBoard {
        board_of(&mem_db().await, "p1").await
    }

    fn one_of_three() -> BallotRules {
        BallotRules {
            options: 3,
            blank: false,
            min: 1,
            max: 1,
        }
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
            fn on_appended(&self, poll_id: &str, position: u64, _token: &[u8], _body: &[u8]) {
                assert_eq!(poll_id, "p1");
                self.positions.lock().unwrap().push(position);
            }
        }
        let rec = Arc::new(Recorder::default());
        let board = mem_board().await.with_replication(rec.clone());

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

    /// The store-level property. In use the polls also have different issuer
    /// keys; one key here shows it is the board that keeps them apart.
    #[tokio::test(flavor = "current_thread")]
    async fn two_polls_share_a_store_and_nothing_else() {
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let pk = issuer.public_key();
        let db = mem_db().await;
        let (first, second) = (board_of(&db, "p1").await, board_of(&db, "p2").await);

        let entry = valid_entry(&issuer, vec![0]);
        assert_eq!(
            first
                .cast(pk, &one_of_three(), entry.clone())
                .await
                .expect("p1"),
            0
        );
        assert_eq!(
            first
                .cast(pk, &one_of_three(), valid_entry(&issuer, vec![1]))
                .await
                .expect("p1"),
            1
        );
        assert_eq!(
            second.cast(pk, &one_of_three(), entry).await.expect("p2"),
            0,
            "a second poll counts its positions from its own start"
        );
        assert_eq!(first.len().await.expect("len"), 2);
        assert_eq!(second.len().await.expect("len"), 1);
        assert_eq!(second.entries().await.expect("entries").len(), 1);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_board_takes_no_more_entries() {
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let pk = issuer.public_key();
        let db = mem_db().await;
        let (board, other) = (board_of(&db, "p1").await, board_of(&db, "p2").await);
        board
            .cast(pk, &one_of_three(), valid_entry(&issuer, vec![2]))
            .await
            .expect("cast");

        assert!(!board.is_closed().await.expect("open"));
        assert_eq!(board.close().await.expect("close"), 1);
        assert!(board.is_closed().await.expect("closed"));
        let late = board
            .cast(pk, &one_of_three(), valid_entry(&issuer, vec![0]))
            .await;
        assert!(matches!(late, Err(BoardError::Closed)), "got {late:?}");
        assert_eq!(
            board.len().await.expect("len"),
            1,
            "a late cast was written"
        );
        assert_eq!(board.close().await.expect("again"), 1, "sealing twice");

        other
            .cast(pk, &one_of_three(), valid_entry(&issuer, vec![0]))
            .await
            .expect("another poll's board is still open");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn ballots_come_back_whole_and_in_no_telling_order() {
        let issuer = TokenIssuer::new_for_poll(2048).expect("keypair");
        let pk = issuer.public_key();
        let board = mem_board().await;
        let mut cast = Vec::new();
        for choice in [0usize, 1, 2, 1] {
            let entry = valid_entry(&issuer, vec![choice]);
            board
                .cast(pk, &one_of_three(), entry.clone())
                .await
                .expect("cast");
            cast.push(entry);
        }

        let ballots = board.ballots().await.expect("ballots");
        assert_eq!(ballots.len(), 4);
        assert!(
            ballots.windows(2).all(|w| w[0].token < w[1].token),
            "ordered by token, which is random, and not by when it was cast"
        );
        for ballot in &ballots {
            assert!(cast.contains(ballot), "an entry did not survive the store");
            assert!(
                pk.verify(&ballot.signature, ballot.msg_randomizer, &ballot.token)
                    .is_ok()
            );
        }

        let (position, found) = board
            .find(&cast[2].token)
            .await
            .expect("find")
            .expect("it is there");
        assert_eq!(position, 2);
        assert_eq!(found, cast[2]);
        assert!(board.find(b"never spent").await.expect("find").is_none());
    }
}
