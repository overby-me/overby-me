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

// -- Publication: the board of a public poll, as records in a repo of its own --
//
// A board the world can read is what lets someone outside the organization
// keep a copy and hold it to its close-out. It is also a statement to the world
// of what a vote came to, so only a poll opened as public has one: a closed
// group's counts are its members' to know, and a hidden tally is its owners'.
//
// Ballots are not published as they are cast. A record's arrival is a public
// event with a time on it, and one record a cast would pair every ballot with
// its voter's moment at the keyboard. They wait until a few can go together, in
// an order that is not the order they came in, stamped with the time they left.

pub const BOARD_PUBLICATION_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS board_publication (
  poll_id    TEXT PRIMARY KEY REFERENCES poll(id),
  poll_uri   TEXT NOT NULL,                                -- the announcement
  poll_cid   TEXT NOT NULL,
  poll_rkey  TEXT NOT NULL,
  closed_uri TEXT                                          -- the close-out, once it is out
);
CREATE TABLE IF NOT EXISTS board_published (
  poll_id  TEXT NOT NULL,
  position INTEGER NOT NULL,
  PRIMARY KEY (poll_id, position)
);
"#;

/// Fewer than this wait for more, or for the close: one ballot published alone
/// is published at the moment it was cast.
const MIN_BATCH: usize = 3;
/// What one `applyWrites` is asked to take. A PDS refuses more than 200.
const MAX_WRITES: usize = 100;

const POLL_NSID: &str = "com.example.wiki.poll";
const ENTRY_NSID: &str = "com.example.wiki.ballotEntry";
const CLOSEOUT_NSID: &str = "com.example.wiki.pollCloseOut";

/// A record key as a repo expects one to be made: 53 bits of microseconds and
/// ten of a clock id, in the sortable base32 of the atproto spec.
fn tid(micros: u64, clock: u16) -> String {
    const ALPHABET: &[u8] = b"234567abcdefghijklmnopqrstuvwxyz";
    let value = ((micros & ((1 << 53) - 1)) << 10) | u64::from(clock & 0x3ff);
    (0..13)
        .rev()
        .map(|i| ALPHABET[((value >> (5 * i)) & 31) as usize] as char)
        .collect()
}

/// A session with the board account's PDS.
pub struct BoardAccount {
    pds: String,
    did: String,
    access: String,
}

type Failure = crate::poll::Failure;

async fn xrpc(
    state: &AppState,
    url: &str,
    bearer: Option<&str>,
    body: &serde_json::Value,
) -> Result<serde_json::Value, Failure> {
    let mut request = state
        .http
        .post(url)?
        .header("content-type", "application/json")
        .body(serde_json::to_vec(body)?);
    if let Some(token) = bearer {
        request = request.bearer_auth(token);
    }
    let response = request.send().await?;
    let status = response.status();
    let bytes = response.bytes().await?;
    if !status.is_success() {
        let said = String::from_utf8_lossy(&bytes[..bytes.len().min(300)]).into_owned();
        return Err(format!("{status} from the board's PDS: {said}").into());
    }
    Ok(serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null))
}

impl BoardAccount {
    /// Sign in to the board account. `None` where none is configured.
    pub async fn sign_in(state: &AppState) -> Result<Option<BoardAccount>, Failure> {
        let config = &state.config;
        if config.board_pds.is_empty() {
            return Ok(None);
        }
        let url = format!("{}/xrpc/com.atproto.server.createSession", config.board_pds);
        let body = serde_json::json!({
            "identifier": config.board_identifier,
            "password": config.board_password.expose(),
        });
        let session = xrpc(state, &url, None, &body).await?;
        let text = |key: &str| session[key].as_str().map(str::to_string);
        match (text("did"), text("accessJwt")) {
            (Some(did), Some(access)) => Ok(Some(BoardAccount {
                pds: config.board_pds.clone(),
                did,
                access,
            })),
            _ => Err("the board's PDS answered a sign-in with no session".into()),
        }
    }

    async fn call(
        &self,
        state: &AppState,
        method: &str,
        body: &serde_json::Value,
    ) -> Result<serde_json::Value, Failure> {
        let url = format!("{}/xrpc/com.atproto.repo.{method}", self.pds);
        xrpc(state, &url, Some(&self.access), body).await
    }
}

struct PublicPoll {
    id: String,
    question: String,
    options: Vec<String>,
    min: i64,
    max: i64,
    blank: bool,
    open: bool,
    issuer_pubkey: String,
    created_at: String,
}

async fn public_polls(conn: &Connection) -> Result<Vec<PublicPoll>, DbError> {
    // Those with anything left to say: not yet closed out in public.
    let mut rows = conn
        .query(
            "SELECT p.id, p.question, p.options, p.min_choices, p.max_choices, p.blank, p.open, \
                    p.issuer_pubkey, p.created_at \
             FROM poll p LEFT JOIN board_publication b ON b.poll_id = p.id \
             WHERE p.public_board = 1 AND p.secret = 1 AND p.issuer_pubkey IS NOT NULL \
               AND b.closed_uri IS NULL \
             ORDER BY p.created_at",
            (),
        )
        .await?;
    let mut out = Vec::new();
    while let Some(row) = rows.next().await? {
        out.push(PublicPoll {
            id: row.get(0)?,
            question: row.get(1)?,
            options: serde_json::from_str(&row.get::<String>(2)?).unwrap_or_default(),
            min: row.get(3)?,
            max: row.get(4)?,
            blank: row.get::<i64>(5)? != 0,
            open: row.get::<i64>(6)? != 0,
            issuer_pubkey: row.get(7)?,
            created_at: row.get(8)?,
        });
    }
    Ok(out)
}

/// What one pass put out.
#[derive(Debug, Default, PartialEq)]
pub struct Published {
    pub announced: usize,
    pub entries: usize,
    pub closed: usize,
}

/// Publish what is due: each public poll's announcement, the ballots that have
/// waited long enough, and the close-out of one that has closed.
pub async fn publish_due(state: &AppState, account: &BoardAccount) -> Result<Published, Failure> {
    use rand::seq::SliceRandom;
    let conn = state.db.acquire().await?;
    let mut done = Published::default();
    for poll in public_polls(&conn).await? {
        let announced = announcement(state, account, &conn, &poll, &mut done).await?;
        let (poll_uri, poll_cid, poll_rkey) = &announced;

        let mut waiting = Vec::new();
        let board = crate::ballot::board(state, &poll.id).await?;
        let published = positions_out(&conn, &poll.id).await?;
        for (position, entry) in board.ballots().await?.iter().enumerate() {
            if !published.contains(&(position as u64)) {
                waiting.push((
                    position as u64,
                    ballot_spec::provisional::encode_entry(entry),
                ));
            }
        }
        if waiting.len() >= MIN_BATCH || (!poll.open && !waiting.is_empty()) {
            waiting.shuffle(&mut rand::thread_rng());
            for batch in waiting.chunks(MAX_WRITES) {
                let left_at = crate::util::now_stamp();
                let micros = crate::util::now_millis().max(0) as u64 * 1000;
                let writes: Vec<serde_json::Value> = batch
                    .iter()
                    .enumerate()
                    .map(|(i, (_, entry))| {
                        serde_json::json!({
                            "$type": "com.atproto.repo.applyWrites#create",
                            "collection": ENTRY_NSID,
                            "rkey": tid(micros + i as u64, rand::random::<u16>()),
                            "value": {
                                "$type": ENTRY_NSID,
                                "pollRef": { "uri": poll_uri, "cid": poll_cid },
                                "token": entry.token,
                                "msgRandomizer": entry.msg_randomizer,
                                "signature": entry.signature,
                                "choices": entry.choices,
                                "createdAt": left_at,
                            },
                        })
                    })
                    .collect();
                let body = serde_json::json!({ "repo": account.did, "writes": writes });
                account.call(state, "applyWrites", &body).await?;
                for (position, _) in batch {
                    conn.execute(
                        "INSERT OR IGNORE INTO board_published (poll_id, position) VALUES (?1, ?2)",
                        vec![
                            Value::Text(poll.id.clone()),
                            Value::Integer(*position as i64),
                        ],
                    )
                    .await?;
                }
                done.entries += batch.len();
            }
        }

        if poll.open {
            continue;
        }
        let Some(close) = close_out_of(state, &poll.id).await? else {
            continue;
        };
        let record = poll_record(state, account, &poll, "closed");
        let body = serde_json::json!({
            "repo": account.did, "collection": POLL_NSID, "rkey": poll_rkey, "record": record,
        });
        account.call(state, "putRecord", &body).await?;
        let body = serde_json::json!({
            "repo": account.did,
            "collection": CLOSEOUT_NSID,
            "rkey": tid(crate::util::now_millis().max(0) as u64 * 1000, rand::random::<u16>()),
            "record": {
                "$type": CLOSEOUT_NSID,
                // By its address alone: closing the announcement changed its CID.
                "poll": poll_uri,
                "pollId": poll.id,
                "entries": close.entries,
                "issued": close.issued,
                "counts": close.counts,
                "boardDigest": close.board_digest,
                "closedAt": close.closed_at,
                "key": close.key,
                "sig": close.sig,
                "createdAt": crate::util::now_stamp(),
            },
        });
        let out = account.call(state, "createRecord", &body).await?;
        conn.execute(
            "UPDATE board_publication SET closed_uri = ?2 WHERE poll_id = ?1",
            [poll.id.as_str(), out["uri"].as_str().unwrap_or_default()],
        )
        .await?;
        done.closed += 1;
    }
    Ok(done)
}

fn poll_record(
    state: &AppState,
    account: &BoardAccount,
    poll: &PublicPoll,
    poll_state: &str,
) -> serde_json::Value {
    serde_json::json!({
        "$type": POLL_NSID,
        "pollId": poll.id,
        "question": poll.question,
        "options": poll.options,
        "state": poll_state,
        "closingAuthority": account.did,
        "issuerPubkey": poll.issuer_pubkey,
        "custodyKey": custodian(&state.config).did_key(),
        "minVote": poll.min,
        "maxVote": poll.max,
        "blank": poll.blank,
        "openAt": poll.created_at,
        "createdAt": poll.created_at,
    })
}

/// The poll's announcement, published now if it has not been: where it is, and
/// the CID every ballot names to say which poll it was cast in.
async fn announcement(
    state: &AppState,
    account: &BoardAccount,
    conn: &Connection,
    poll: &PublicPoll,
    done: &mut Published,
) -> Result<(String, String, String), Failure> {
    let mut rows = conn
        .query(
            "SELECT poll_uri, poll_cid, poll_rkey FROM board_publication WHERE poll_id = ?1",
            [poll.id.as_str()],
        )
        .await?;
    if let Some(row) = rows.next().await? {
        return Ok((row.get(0)?, row.get(1)?, row.get(2)?));
    }
    drop(rows);
    let rkey = tid(
        crate::util::now_millis().max(0) as u64 * 1000,
        rand::random::<u16>(),
    );
    let body = serde_json::json!({
        "repo": account.did,
        "collection": POLL_NSID,
        "rkey": rkey,
        "record": poll_record(state, account, poll, "open"),
    });
    let out = account.call(state, "createRecord", &body).await?;
    let (Some(uri), Some(cid)) = (out["uri"].as_str(), out["cid"].as_str()) else {
        return Err("the board's PDS made a record and did not say where".into());
    };
    conn.execute(
        "INSERT INTO board_publication (poll_id, poll_uri, poll_cid, poll_rkey) \
         VALUES (?1, ?2, ?3, ?4)",
        [poll.id.as_str(), uri, cid, rkey.as_str()],
    )
    .await?;
    done.announced += 1;
    Ok((uri.to_string(), cid.to_string(), rkey))
}

async fn positions_out(
    conn: &Connection,
    poll_id: &str,
) -> Result<std::collections::BTreeSet<u64>, DbError> {
    let mut rows = conn
        .query(
            "SELECT position FROM board_published WHERE poll_id = ?1",
            [poll_id],
        )
        .await?;
    let mut out = std::collections::BTreeSet::new();
    while let Some(row) = rows.next().await? {
        out.insert(row.get::<i64>(0)?.max(0) as u64);
    }
    Ok(out)
}

/// Publish what is due, every `board_batch_secs`, for the life of the process.
/// Does nothing where no board account is configured. A pass that fails is
/// logged and tried again: what is waiting stays waiting, and nothing is lost.
pub async fn run_publisher(state: AppState) {
    if state.config.board_pds.is_empty() {
        return;
    }
    let every = std::time::Duration::from_secs(state.config.board_batch_secs.max(1));
    let mut account: Option<BoardAccount> = None;
    loop {
        tokio::time::sleep(every).await;
        if account.is_none() {
            match BoardAccount::sign_in(&state).await {
                Ok(signed_in) => account = signed_in,
                Err(e) => tracing::warn!("could not sign in to the board account: {e}"),
            }
        }
        let Some(session) = &account else { continue };
        match publish_due(&state, session).await {
            Ok(done) if done != Published::default() => tracing::info!(
                "board: announced {}, published {} ballots, closed out {}",
                done.announced,
                done.entries,
                done.closed
            ),
            Ok(_) => {}
            Err(e) => {
                tracing::warn!("publishing the board failed, to be tried again: {e}");
                // A session lapses, and asking for a new one costs nothing.
                account = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::poll::tests::{CLOSE, OPEN, for_against, open, state, vote};
    use crate::router;
    use crate::xrpc::tests::{join, post, token_for};
    use ballot_spec::custody::{CloseOut, verify};
    use fake_pds::FakePds;
    use serde_json::json;

    const PASSWORD: &str = "an-app-password";

    /// The closed group of the poll tests, with a board account to publish to
    /// and two more voters, so that a batch can fill.
    async fn publishing() -> (AppState, FakePds, Vec<String>) {
        let pds = FakePds::start(PASSWORD).await;
        let mut state = state().await;
        state.config.board_pds = pds.url.clone();
        state.config.board_identifier = "board.test".to_string();
        state.config.board_password = crate::config::Secret::new(PASSWORD);
        let mut voters = Vec::new();
        for did in [
            "did:plc:alice",
            "did:plc:bob",
            "did:plc:carol",
            "did:plc:dave",
        ] {
            voters.push(token_for(&state, did).await);
        }
        join(&state, "did:plc:carol", "c9").await;
        join(&state, "did:plc:dave", "c9").await;
        (state, pds, voters)
    }

    fn in_public(mut poll: serde_json::Value) -> serde_json::Value {
        poll["public_board"] = json!(true);
        poll
    }

    #[test]
    fn a_record_key_sorts_by_when_it_was_made() {
        let (early, late) = (tid(1_700_000_000_000_000, 7), tid(1_700_000_000_000_001, 0));
        assert_eq!((early.len(), late.len()), (13, 13));
        assert!(early < late, "{early} {late}");
        assert!(
            early
                .bytes()
                .all(|b| b"234567abcdefghijklmnopqrstuvwxyz".contains(&b))
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn ballots_go_out_together_and_a_close_is_signed_for_in_public() {
        let (state, pds, voters) = publishing().await;
        let account = BoardAccount::sign_in(&state)
            .await
            .expect("a session")
            .expect("configured");
        let poll = open(&state, &voters[0], in_public(for_against(true))).await;
        assert_eq!(poll["public_board"], true);
        let id = poll["id"].as_str().expect("id");

        vote(&state, &poll, &voters[0], &[0]).await;
        vote(&state, &poll, &voters[1], &[1]).await;
        let first = publish_due(&state, &account).await.expect("a pass");
        assert_eq!(
            first,
            Published {
                announced: 1,
                entries: 0,
                closed: 0
            },
            "two ballots wait: either alone would go out when it was cast"
        );
        let announced = pds.records(POLL_NSID);
        assert_eq!(announced[0].value["state"], "open");
        assert_eq!(announced[0].value["issuerPubkey"], poll["issuer_pubkey"]);
        assert_eq!(
            announced[0].value["custodyKey"],
            custodian(&state.config).did_key().as_str()
        );

        vote(&state, &poll, &voters[2], &[0]).await;
        let second = publish_due(&state, &account).await.expect("a pass");
        assert_eq!(second.entries, 3);
        let out = pds.records(ENTRY_NSID);
        let batches: Vec<_> = pds
            .calls()
            .into_iter()
            .filter(|c| c.0 == "applyWrites")
            .collect();
        assert_eq!(batches, [("applyWrites".to_string(), 3)], "in one write");
        let left_at: std::collections::BTreeSet<_> = out
            .iter()
            .map(|r| r.value["createdAt"].to_string())
            .collect();
        assert_eq!(
            left_at.len(),
            1,
            "stamped with when they left, not when each came"
        );
        assert!(
            out.iter()
                .all(|r| r.value["pollRef"]["uri"] == announced[0].uri().as_str())
        );
        assert!(
            out.iter()
                .all(|r| r.value.get("did").is_none() && r.value.get("voter").is_none())
        );

        // One more, which waits, and goes with the close.
        vote(&state, &poll, &voters[3], &[1]).await;
        assert_eq!(
            publish_due(&state, &account).await.expect("a pass").entries,
            0
        );
        let (_, closed) = post(
            router(state.clone()),
            CLOSE,
            Some(&voters[0]),
            json!({"id": id}),
        )
        .await;
        let last = publish_due(&state, &account).await.expect("a pass");
        assert_eq!(
            last,
            Published {
                announced: 0,
                entries: 1,
                closed: 1
            }
        );
        assert_eq!(pds.records(ENTRY_NSID).len(), 4);
        assert_eq!(pds.records(POLL_NSID)[0].value["state"], "closed");

        let signed_for = &pds.records(CLOSEOUT_NSID)[0].value;
        let text = |key: &str| signed_for[key].as_str().expect(key).to_string();
        let close = CloseOut {
            poll: id.to_string(),
            entries: 4,
            issued: 4,
            counts: vec![2, 2, 0],
            board_digest: text("boardDigest"),
            closed_at: closed["closed_at"].as_str().expect("closed_at").to_string(),
        };
        assert!(
            verify(&text("key"), &close.payload(), &text("sig")),
            "{signed_for}"
        );
        assert_eq!(text("key"), custodian(&state.config).did_key());
        assert_eq!(
            publish_due(&state, &account).await.expect("a pass"),
            Published::default(),
            "said once"
        );
    }

    /// A public poll of four ballots, published and closed out. Returns the
    /// custody key.
    async fn a_closed_public_poll(state: &AppState, voters: &[String]) -> String {
        let account = BoardAccount::sign_in(state)
            .await
            .expect("a session")
            .expect("configured");
        let poll = open(state, &voters[0], in_public(for_against(true))).await;
        for (voter, choice) in voters.iter().zip([0, 1, 0, 0]) {
            vote(state, &poll, voter, &[choice]).await;
        }
        publish_due(state, &account).await.expect("a pass");
        let id = poll["id"].as_str().expect("id");
        post(
            router(state.clone()),
            CLOSE,
            Some(&voters[0]),
            json!({"id": id}),
        )
        .await;
        publish_due(state, &account).await.expect("a pass");
        custodian(&state.config).did_key()
    }

    fn mirror_dir() -> std::path::PathBuf {
        std::env::temp_dir().join(format!("board-mirror-{}", crate::util::random_token(8)))
    }

    /// The whole chain, end to end: the AppView publishes, someone else keeps a
    /// copy, and the copy is what the custodian is held to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_mirror_counts_the_board_for_itself_and_notices_what_is_taken_back() {
        let (state, pds, voters) = publishing().await;
        let key = a_closed_public_poll(&state, &voters).await;
        let dir = mirror_dir();

        let seen = board_mirror::follow_once(&pds.url, fake_pds::DID, &dir)
            .await
            .expect("the repo");
        assert_eq!(
            (seen.new, seen.alarms.len()),
            (6, 0),
            "a poll, four ballots, a close-out"
        );
        let counted = board_mirror::check(&dir, Some(&key));
        assert_eq!(counted.len(), 1);
        assert_eq!(counted[0].problems, Vec::<String>::new());
        assert_eq!(counted[0].counts, Some(vec![3, 1, 0]));
        assert!(
            board_mirror::check(&dir, Some("did:key:zSomeoneElse"))[0]
                .problems
                .iter()
                .any(|p| p.contains("not the key given")),
            "signed by a key the checker was not told to trust"
        );

        // A ballot unpublished, and another rewritten where it stands: both are
        // noticed, and the mirror's own copy still counts as it did.
        pds.tamper(|records| {
            let at = records
                .iter()
                .position(|r| r.collection == ENTRY_NSID)
                .expect("an entry");
            records.remove(at);
            let next = records
                .iter_mut()
                .find(|r| r.collection == ENTRY_NSID)
                .expect("another");
            next.value["choices"] = json!([1]);
        });
        let after = board_mirror::follow_once(&pds.url, fake_pds::DID, &dir)
            .await
            .expect("the repo");
        let said: Vec<&str> = after.alarms.iter().map(|a| a.what.as_str()).collect();
        assert_eq!(after.alarms.len(), 2, "{said:?}");
        assert!(said.contains(&"gone") && said.iter().any(|s| s.starts_with("rewritten")));
        assert_eq!(
            board_mirror::check(&dir, Some(&key))[0].counts,
            Some(vec![3, 1, 0])
        );
        let quiet = board_mirror::follow_once(&pds.url, fake_pds::DID, &dir)
            .await
            .expect("the repo");
        assert!(quiet.alarms.iter().all(|a| a.what != "gone"), "said once");

        // Someone who only starts looking now sees the board as it was left: it
        // is not the board that was signed for, and it does not add up to it.
        let late = mirror_dir();
        board_mirror::follow_once(&pds.url, fake_pds::DID, &late)
            .await
            .expect("the repo");
        let disputed = &board_mirror::check(&late, Some(&key))[0];
        assert!(
            disputed
                .problems
                .iter()
                .any(|p| p.contains("not the board that was signed for")),
            "{:?}",
            disputed.problems
        );
        assert!(
            disputed
                .problems
                .iter()
                .any(|p| p.contains("was announced"))
        );
        for dir in [dir, late] {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// The other lie a custodian could tell: the ballots all there, and a
    /// result they do not add up to.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn an_announced_result_the_board_does_not_add_up_to_is_disputed() {
        let (state, pds, voters) = publishing().await;
        let key = a_closed_public_poll(&state, &voters).await;
        pds.tamper(|records| {
            let close = records
                .iter_mut()
                .find(|r| r.collection == CLOSEOUT_NSID)
                .expect("a close-out");
            close.value["counts"] = json!([1, 3, 0]);
        });
        let dir = mirror_dir();
        board_mirror::follow_once(&pds.url, fake_pds::DID, &dir)
            .await
            .expect("the repo");
        let disputed = &board_mirror::check(&dir, Some(&key))[0];
        assert!(
            disputed
                .problems
                .iter()
                .any(|p| p.contains("signature does not verify")),
            "{:?}",
            disputed.problems
        );
        assert!(
            disputed
                .problems
                .iter()
                .any(|p| p.contains("counted [3, 1, 0]"))
        );
        let _ = std::fs::remove_dir_all(dir);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_closed_groups_board_is_its_members_unless_the_chair_says_otherwise() {
        let (state, pds, voters) = publishing().await;
        let account = BoardAccount::sign_in(&state)
            .await
            .expect("a session")
            .expect("configured");
        let private = open(&state, &voters[0], for_against(true)).await;
        assert_eq!(private["public_board"], false, "c9 is a closed group");
        for (voter, choice) in voters.iter().zip([0, 1, 0, 1]) {
            vote(&state, &private, voter, &[choice]).await;
        }
        assert_eq!(
            publish_due(&state, &account).await.expect("a pass"),
            Published::default()
        );
        assert!(pds.records(POLL_NSID).is_empty() && pds.records(ENTRY_NSID).is_empty());

        // A board tells everyone the tally, so a hidden one cannot have one, and
        // a poll that names its voters has no board at all.
        let mut hidden = in_public(for_against(true));
        hidden["hide_tally"] = json!(true);
        for refused in [hidden, in_public(for_against(false))] {
            let (status, v) = post(router(state.clone()), OPEN, Some(&voters[0]), refused).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        }
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_board_account_that_does_not_let_us_in_is_said_so() {
        let (mut state, _pds, _) = publishing().await;
        state.config.board_password = crate::config::Secret::new("not-it");
        assert!(BoardAccount::sign_in(&state).await.is_err());
        state.config.board_pds = String::new();
        assert!(
            BoardAccount::sign_in(&state)
                .await
                .expect("unconfigured")
                .is_none()
        );
    }
}
