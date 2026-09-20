//! The board's custody: what the AppView signs about a secret poll.
//!
//! Every ballot passes through here, which is what lets it be anonymous and
//! what would let one be dropped or rewritten unseen. So each cast is answered
//! with a signed receipt, and each close with a signed close-out: a board
//! without a receipted entry, or a count that is not the board's, contradicts
//! this AppView's own signature (`ballot_spec::custody`,
//! `docs/ballot-board-custody.md`).
//!
//! The key is kept for nothing else, and is derived from `APPVIEW_SECRET`, so
//! it survives a restart and a restore without being stored anywhere a copy of
//! the database would carry it.

use crate::AppState;
use crate::config::Config;
use crate::db::DbError;
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ballot_spec::custody::{CloseOut, Custodian, Receipt, entry_digest};
use ballot_spec::provisional::ProvisionalEntry;
use hkdf::Hkdf;
use serde::Serialize;
use sha2::Sha256;
use turso::{Connection, Value};

pub const BOARD_CUSTODY_DDL: &str = r#"
-- What a board was when it closed and what it came to, as it was signed. Kept
-- as signed and never rebuilt: a later key, or a later bug, must not re-sign it.
CREATE TABLE IF NOT EXISTS poll_closeout (
  poll_id      TEXT PRIMARY KEY REFERENCES poll(id),
  entries      INTEGER NOT NULL,
  issued       INTEGER NOT NULL,
  counts       TEXT NOT NULL,                              -- JSON array
  board_digest TEXT NOT NULL,
  closed_at    TEXT NOT NULL,
  key          TEXT NOT NULL,                              -- the did:key that signed
  sig          TEXT NOT NULL
);
"#;

/// The custody key. One in 2^128 derivations is no valid scalar, so a counter
/// is bound in and the next one is tried.
pub fn custodian(config: &Config) -> Custodian {
    (0u8..=255)
        .find_map(|attempt| {
            let mut seed = [0u8; 32];
            Hkdf::<Sha256>::new(Some(&[attempt]), config.secret.as_bytes())
                .expand(b"wiki-appview board custody key v1", &mut seed)
                .ok()?;
            Custodian::from_seed(&seed)
        })
        .expect("one of 256 derivations is a valid P-256 scalar")
}

/// A receipt or a close-out with who signed it and the signature.
#[derive(Debug, Serialize)]
pub struct Signed<T: Serialize> {
    #[serde(flatten)]
    pub body: T,
    /// The custody key, as a `did:key`.
    pub key: String,
    /// ECDSA P-256 over SHA-256 of the payload, `r || s`, base64url.
    pub sig: String,
}

#[derive(Debug, Serialize)]
pub struct ReceiptView {
    pub poll: String,
    pub position: u64,
    pub token: String,
    pub choices: Vec<usize>,
    pub entry_digest: String,
    pub at: String,
}

/// The custodian's word that `entry` is at `position` on `poll`'s board.
pub fn receipt(
    config: &Config,
    poll: &str,
    position: u64,
    entry: &ProvisionalEntry,
) -> Signed<ReceiptView> {
    // To the minute: a receipt shown in a dispute must not date its cast finely
    // enough to be paired with anybody's log.
    let now = crate::util::now_stamp();
    let at = format!("{}Z", &now[..now.len().min(16)]);
    let body = Receipt {
        poll: poll.to_string(),
        position,
        token: entry.token.clone(),
        choices: entry.choices.clone(),
        entry_digest: entry_digest(entry),
        at,
    };
    let custodian = custodian(config);
    Signed {
        sig: custodian.sign(&body.payload()),
        key: custodian.did_key(),
        body: ReceiptView {
            poll: body.poll,
            position: body.position,
            token: body.token,
            choices: body.choices,
            entry_digest: body.entry_digest,
            at: body.at,
        },
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CloseOutView {
    pub entries: u64,
    pub issued: u64,
    pub counts: Vec<u64>,
    pub board_digest: String,
    pub closed_at: String,
    pub key: String,
    pub sig: String,
}

/// Sign and keep what `poll`'s board came to. Inside the transaction that
/// closes it, so that a closed secret poll is never without one.
pub async fn close_out(
    conn: &Connection,
    config: &Config,
    close: &CloseOut,
) -> Result<(), crate::poll::Failure> {
    let custodian = custodian(config);
    conn.execute(
        "INSERT OR IGNORE INTO poll_closeout \
           (poll_id, entries, issued, counts, board_digest, closed_at, key, sig) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
        vec![
            Value::Text(close.poll.clone()),
            Value::Integer(close.entries as i64),
            Value::Integer(close.issued as i64),
            Value::Text(serde_json::to_string(&close.counts)?),
            Value::Text(close.board_digest.clone()),
            Value::Text(close.closed_at.clone()),
            Value::Text(custodian.did_key()),
            Value::Text(custodian.sign(&close.payload())),
        ],
    )
    .await?;
    Ok(())
}

/// A closed poll's close-out, as it was signed.
pub async fn close_out_of(
    state: &AppState,
    poll_id: &str,
) -> Result<Option<CloseOutView>, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            "SELECT entries, issued, counts, board_digest, closed_at, key, sig \
             FROM poll_closeout WHERE poll_id = ?1",
            [poll_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    let natural = |i: usize| row.get::<i64>(i).map(|n| n.max(0) as u64);
    Ok(Some(CloseOutView {
        entries: natural(0)?,
        issued: natural(1)?,
        counts: serde_json::from_str(&row.get::<String>(2)?).unwrap_or_default(),
        board_digest: row.get(3)?,
        closed_at: row.get(4)?,
        key: row.get(5)?,
        sig: row.get(6)?,
    }))
}

/// `com.example.wiki.getBoardKey`: the key this AppView signs receipts and
/// close-outs with. For anyone: a receipt names its key too, and this is what
/// it is checked against.
pub async fn get_board_key(State(state): State<AppState>) -> Response {
    let key = custodian(&state.config).did_key();
    (StatusCode::OK, Json(serde_json::json!({ "key": key }))).into_response()
}
