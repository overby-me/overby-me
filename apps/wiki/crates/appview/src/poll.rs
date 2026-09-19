//! Voting. A chair opens a poll on a motion, the room votes, the chair closes
//! it. This replaces the interim's `vote/poll` and `vote/vote` nodes and its
//! `/vote` sidecar.
//!
//! A poll has a place in the tree (a `document` of kind `poll`, so it has a URL,
//! moves with its motion and goes to the bin with it) and a `poll` row of the
//! same id holding what is voted on and how it went. Who may vote is FROZEN when
//! it opens: every member of its context who holds voting rights at that moment,
//! with the weight the roster resolves (`ballot_store::freeze_at_open`).
//!
//! A SECRET poll is the decided scheme (`crates/ballot-spec`): the voter asks,
//! signed in, for blind signatures on tokens only they can see, and casts each
//! token WITHOUT a session. Nothing here can join the two halves: issuance
//! records that a voter was served, never what they were given, and a cast
//! carries a token and no voter. An OPEN poll is the plain thing it sounds
//! like: a ballot names its voter.
//!
//! Not built, and waiting on the owner's custody call
//! (`docs/ballot-board-custody.md`): publishing the board as atproto records,
//! and signed inclusion receipts. The board is served from here until then.

use crate::AppState;
use crate::authz::{Authz, readable_document};
use crate::config::Config;
use crate::db::DbError;
use crate::live::Topic;
use crate::session::{Caller, MaybeCaller};
use crate::store::{NewDocument, WriteError};
use crate::xrpc::{conflict, err, forbidden, invalid, owner_of, owns, write_failed};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use ballot_spec::provisional::{ProvisionalEntry, decode_bytes, encode_entry};
use ballot_spec::{
    BallotRules, BoardEntry, CastError, IssuerPublicKey, MessageRandomizer, Outcome, Signature,
    TOKEN_NULLIFIER_LEN, TokenIssuer,
};
use ballot_store::BoardError;
use chacha20poly1305::aead::{Aead, KeyInit, Payload};
use chacha20poly1305::{Key, XChaCha20Poly1305, XNonce};
use hkdf::Hkdf;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;
use turso::Value;

/// What a poll may be opened on: the kinds that put something to a vote.
pub const POLLABLE: &[&str] = &["policy", "position", "change", "question"];

const MAX_OPTIONS: usize = 100;
const MAX_TEXT_CHARS: usize = 300;

/// RFC 9474's minimum modulus (`ballot-spec` DECISIONS.md D6).
const ISSUER_BITS: usize = 2048;

/// How long a change to a tally waits to be announced, so that a room voting at
/// once is told once a beat and not once a ballot. It also keeps an announcement
/// from marking the moment of any one cast.
const TALLY_BEAT: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(1)
};

type Failure = Box<dyn std::error::Error + Send + Sync>;

/// What this module keeps between requests.
#[derive(Default)]
pub struct Shared {
    /// The issuers of open secret polls, so a key is unsealed once per poll and
    /// not once per voter.
    issuers: tokio::sync::Mutex<HashMap<String, Arc<TokenIssuer>>>,
    /// Polls with a tally announcement on its way.
    announcing: std::sync::Mutex<HashSet<String>>,
    /// The last tally of each running poll, with the ballot count it was taken
    /// at: every listener refetches on an announcement, and they all ask for the
    /// same sum.
    tallies: std::sync::Mutex<HashMap<String, Tally>>,
}

#[derive(Debug, Clone)]
struct PollRow {
    id: String,
    context_id: String,
    question: String,
    options: Vec<String>,
    min: usize,
    max: usize,
    blank: bool,
    open: bool,
    secret: bool,
    hide_tally: bool,
    issuer_pubkey: Option<String>,
    issuer_secret: Option<String>,
    result: Option<Tally>,
    created_at: String,
    closed_at: Option<String>,
    path: String,
    parent_id: Option<String>,
}

impl PollRow {
    fn rules(&self) -> BallotRules {
        BallotRules {
            options: self.options.len(),
            min: self.min,
            max: self.max,
            blank: self.blank,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
struct Tally {
    counts: Vec<u64>,
    ballots: u64,
}

const POLL_COLS: &str = "p.id, p.context_id, p.question, p.options, p.min_choices, \
    p.max_choices, p.blank, p.open, p.secret, p.hide_tally, p.issuer_pubkey, p.issuer_secret, \
    p.counts, p.ballots, p.created_at, p.closed_at, d.path, d.parent_id";

fn text(row: &turso::Row, i: usize) -> Option<String> {
    match row.get_value(i) {
        Ok(Value::Text(s)) => Some(s),
        _ => None,
    }
}

fn natural(row: &turso::Row, i: usize) -> Option<u64> {
    match row.get_value(i) {
        Ok(Value::Integer(n)) => u64::try_from(n).ok(),
        _ => None,
    }
}

fn poll_row(row: &turso::Row) -> Result<PollRow, DbError> {
    let flag = |i: usize| natural(row, i).unwrap_or(0) != 0;
    let counts: Option<Vec<u64>> = text(row, 12).and_then(|json| serde_json::from_str(&json).ok());
    Ok(PollRow {
        id: row.get::<String>(0)?,
        context_id: row.get::<String>(1)?,
        question: row.get::<String>(2)?,
        options: serde_json::from_str(&row.get::<String>(3)?).unwrap_or_default(),
        min: natural(row, 4).unwrap_or(1) as usize,
        max: natural(row, 5).unwrap_or(1) as usize,
        blank: flag(6),
        open: flag(7),
        secret: flag(8),
        hide_tally: flag(9),
        issuer_pubkey: text(row, 10),
        issuer_secret: text(row, 11),
        result: counts
            .zip(natural(row, 13))
            .map(|(counts, ballots)| Tally { counts, ballots }),
        created_at: row.get::<String>(14)?,
        closed_at: text(row, 15),
        path: row.get::<String>(16)?,
        parent_id: text(row, 17),
    })
}

/// Who a poll is being loaded for.
enum Reader<'a> {
    /// Whoever asks: a cast, which is authorized by its token and carries no
    /// voter to ask about.
    Anyone,
    /// A caller, or nobody signed in, who must be able to read the poll.
    Caller(Option<&'a str>),
}

/// A poll whose place in the tree is live, if `reader` may have it.
async fn load(state: &AppState, id: &str, reader: Reader<'_>) -> Result<Option<PollRow>, DbError> {
    let conn = state.db.acquire().await?;
    let live = "FROM poll p JOIN document d ON d.id = p.id \
                WHERE p.id = ?1 AND d.deleted_at IS NULL";
    let mut rows = match reader {
        Reader::Anyone => {
            conn.query(&format!("SELECT {POLL_COLS} {live}"), [id])
                .await?
        }
        Reader::Caller(did) => {
            conn.query(
                &format!(
                    "SELECT {POLL_COLS} {live} AND {}",
                    readable_document("d", 2)
                ),
                vec![
                    Value::Text(id.to_string()),
                    did.map_or(Value::Null, |did| Value::Text(did.to_string())),
                ],
            )
            .await?
        }
    };
    match rows.next().await? {
        Some(row) => Ok(Some(poll_row(&row)?)),
        None => Ok(None),
    }
}

fn no_such_poll() -> Response {
    err(StatusCode::NOT_FOUND, "NotFound", "no such poll")
}

fn poll_closed() -> Response {
    conflict("PollClosed", "the poll is closed")
}

/// Whoever holds an open poll's issuer key can mint ballots for it, so the
/// database holds it only sealed: a copy of the database without
/// `APPVIEW_SECRET` forges nothing. The poll id is bound in, so a sealed key
/// cannot be moved to another poll's row.
fn cipher(config: &Config) -> XChaCha20Poly1305 {
    let mut key = [0u8; 32];
    Hkdf::<Sha256>::new(None, config.secret.as_bytes())
        .expand(b"wiki-appview poll issuer key v1", &mut key)
        .expect("32 is a valid HKDF-SHA256 output length");
    XChaCha20Poly1305::new(Key::from_slice(&key))
}

fn seal(config: &Config, poll_id: &str, der: &[u8]) -> Option<String> {
    let mut out = crate::util::random_bytes(24);
    let sealed = cipher(config)
        .encrypt(
            XNonce::from_slice(&out),
            Payload {
                msg: der,
                aad: poll_id.as_bytes(),
            },
        )
        .ok()?;
    out.extend_from_slice(&sealed);
    Some(crate::util::b64url(&out))
}

fn unseal(config: &Config, poll_id: &str, sealed: &str) -> Option<Vec<u8>> {
    let raw = crate::util::b64url_decode(sealed).ok()?;
    let (nonce, sealed) = raw.split_at_checked(24)?;
    cipher(config)
        .decrypt(
            XNonce::from_slice(nonce),
            Payload {
                msg: sealed,
                aad: poll_id.as_bytes(),
            },
        )
        .ok()
}

async fn issuer_of(state: &AppState, poll: &PollRow) -> Result<Arc<TokenIssuer>, Failure> {
    let mut issuers = state.polls.issuers.lock().await;
    if let Some(issuer) = issuers.get(&poll.id) {
        return Ok(issuer.clone());
    }
    let sealed = poll
        .issuer_secret
        .as_deref()
        .ok_or("the poll has no issuer key")?;
    let der = unseal(&state.config, &poll.id, sealed)
        .ok_or("the poll's issuer key does not open under this APPVIEW_SECRET")?;
    let issuer = Arc::new(TokenIssuer::from_secret_der(&der)?);
    issuers.insert(poll.id.clone(), issuer.clone());
    Ok(issuer)
}

/// The tally as it stands: the stored result of a closed poll, else a count of
/// the board or of the open ballots. Always a count, never a running total.
async fn tally(state: &AppState, poll: &PollRow) -> Result<Tally, Failure> {
    if let Some(result) = &poll.result {
        return Ok(result.clone());
    }
    let cached = |ballots: u64| {
        let tallies = state.polls.tallies.lock().expect("tallies");
        tallies
            .get(&poll.id)
            .filter(|t| t.ballots == ballots)
            .cloned()
    };
    let fresh = if poll.secret {
        let board = crate::ballot::board(state, &poll.id).await?;
        if let Some(tally) = cached(board.len().await?) {
            return Ok(tally);
        }
        let entries = board.ballots().await?;
        Tally {
            counts: ballot_spec::tally(&entries, &poll.rules()),
            ballots: entries.len() as u64,
        }
    } else {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT choices, weight FROM open_ballot WHERE poll_id = ?1",
                [poll.id.as_str()],
            )
            .await?;
        let mut fresh = Tally {
            counts: vec![0; poll.options.len()],
            ballots: 0,
        };
        while let Some(row) = rows.next().await? {
            let choices: Vec<usize> =
                serde_json::from_str(&row.get::<String>(0)?).unwrap_or_default();
            let weight = natural(&row, 1).unwrap_or(0);
            fresh.ballots += weight;
            for choice in choices {
                if let Some(count) = fresh.counts.get_mut(choice) {
                    *count += weight;
                }
            }
        }
        fresh
    };
    state
        .polls
        .tallies
        .lock()
        .expect("tallies")
        .insert(poll.id.clone(), fresh.clone());
    Ok(fresh)
}

/// The sum of the frozen weights: what a turnout is out of.
async fn eligible_weight(state: &AppState, poll_id: &str) -> Result<u64, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            "SELECT coalesce(sum(resolved_weight), 0) FROM eligibility WHERE poll_id = ?1",
            [poll_id],
        )
        .await?;
    Ok(rows
        .next()
        .await?
        .and_then(|row| natural(&row, 0))
        .unwrap_or(0))
}

/// The caller's frozen weight in a poll: `None` off the roster, and `Some(0)`
/// for someone whose weight went to a delegate.
async fn weight_of(state: &AppState, poll_id: &str, did: &str) -> Result<Option<u64>, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            "SELECT resolved_weight FROM eligibility WHERE poll_id = ?1 AND did = ?2",
            [poll_id, did],
        )
        .await?;
    Ok(rows.next().await?.map(|row| natural(&row, 0).unwrap_or(0)))
}

/// Tell the poll's listeners its tally moved: soon, and once for every ballot
/// that lands within the beat.
fn announce_tally(state: &AppState, poll: &PollRow) {
    let first = state
        .polls
        .announcing
        .lock()
        .expect("announcing")
        .insert(poll.id.clone());
    if !first {
        return;
    }
    let (state, context_id, poll_id) = (state.clone(), poll.context_id.clone(), poll.id.clone());
    tokio::spawn(async move {
        tokio::time::sleep(TALLY_BEAT).await;
        // Cleared BEFORE it is sent: a ballot landing in between starts the next
        // beat rather than being announced by nobody.
        state
            .polls
            .announcing
            .lock()
            .expect("announcing")
            .remove(&poll_id);
        state.publish(Topic::Context(context_id), "tally", &poll_id);
    });
}

#[derive(Debug, Serialize)]
pub struct OutcomeView {
    /// `winner`, `tie` or `none`.
    pub kind: &'static str,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub option: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct ViewerView {
    /// Ballots the caller has in this poll. 0 when their weight went to a
    /// delegate.
    pub weight: u64,
    /// A secret poll: the caller has collected their tokens. Whether they then
    /// cast them is not something the server can know.
    pub issued: bool,
    /// An open poll: the caller's own ballot, once cast.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub choices: Option<Vec<usize>>,
}

#[derive(Debug, Serialize)]
pub struct PollView {
    pub id: String,
    pub context_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<String>,
    pub path: String,
    pub question: String,
    pub options: Vec<String>,
    pub min: usize,
    pub max: usize,
    pub blank: bool,
    pub secret: bool,
    pub hide_tally: bool,
    pub open: bool,
    pub created_at: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub closed_at: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub issuer_pubkey: Option<String>,
    /// Ballots cast. Shown to everyone, also where the counts are not: a room
    /// reads its turnout off this.
    pub ballots: u64,
    pub eligible: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counts: Option<Vec<u64>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub outcome: Option<OutcomeView>,
    /// Absent for a caller who is not on the poll's roster.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub viewer: Option<ViewerView>,
}

/// Whether `did` sees a poll's counts and its board. The interim's rule: a poll
/// that hides its tally shows it to the owners of its context, open or closed.
async fn sees_counts(state: &AppState, poll: &PollRow, did: Option<&str>) -> Result<bool, DbError> {
    if !poll.hide_tally {
        return Ok(true);
    }
    let Some(did) = did else {
        return Ok(false);
    };
    let membership = Authz::new(state.db.clone())
        .membership(&poll.context_id, did)
        .await?;
    Ok(membership.is_some_and(owns))
}

async fn viewer(
    state: &AppState,
    poll: &PollRow,
    did: &str,
) -> Result<Option<ViewerView>, DbError> {
    let Some(weight) = weight_of(state, &poll.id, did).await? else {
        return Ok(None);
    };
    let conn = state.db.acquire().await?;
    let (issued, choices) = if poll.secret {
        let mut rows = conn
            .query(
                "SELECT 1 FROM token_issued WHERE poll_id = ?1 AND did = ?2",
                [poll.id.as_str(), did],
            )
            .await?;
        (rows.next().await?.is_some(), None)
    } else {
        let mut rows = conn
            .query(
                "SELECT choices FROM open_ballot WHERE poll_id = ?1 AND did = ?2",
                [poll.id.as_str(), did],
            )
            .await?;
        let own = rows.next().await?.and_then(|row| text(&row, 0));
        (false, own.and_then(|json| serde_json::from_str(&json).ok()))
    };
    Ok(Some(ViewerView {
        weight,
        issued,
        choices,
    }))
}

async fn view(state: &AppState, poll: PollRow, did: Option<&str>) -> Result<PollView, Failure> {
    let tally = tally(state, &poll).await?;
    let shown = sees_counts(state, &poll, did).await?;
    let outcome = shown.then(
        || match ballot_spec::outcome(&tally.counts, &poll.rules()) {
            Outcome::Winner(option) => OutcomeView {
                kind: "winner",
                option: Some(option),
            },
            Outcome::Tie => OutcomeView {
                kind: "tie",
                option: None,
            },
            Outcome::NoVotes => OutcomeView {
                kind: "none",
                option: None,
            },
        },
    );
    let viewer = match did {
        Some(did) => viewer(state, &poll, did).await?,
        None => None,
    };
    Ok(PollView {
        eligible: eligible_weight(state, &poll.id).await?,
        ballots: tally.ballots,
        counts: shown.then_some(tally.counts),
        outcome,
        viewer,
        id: poll.id,
        context_id: poll.context_id,
        parent_id: poll.parent_id,
        path: poll.path,
        question: poll.question,
        options: poll.options,
        min: poll.min,
        max: poll.max,
        blank: poll.blank,
        secret: poll.secret,
        hide_tally: poll.hide_tally,
        open: poll.open,
        created_at: poll.created_at,
        closed_at: poll.closed_at,
        issuer_pubkey: poll.issuer_pubkey,
    })
}

async fn answer(state: &AppState, poll: PollRow, did: Option<&str>, what: &str) -> Response {
    match view(state, poll, did).await {
        Ok(view) => (StatusCode::OK, Json(view)).into_response(),
        Err(e) => write_failed(what, e),
    }
}

fn yes() -> bool {
    true
}

fn one() -> usize {
    1
}

#[derive(Debug, Deserialize)]
pub struct OpenPollBody {
    /// The motion, amendment, question or position put to the vote.
    pub parent_id: String,
    pub title: String,
    /// The wording voted on, fixed for the life of the poll. The title if absent.
    #[serde(default)]
    pub question: Option<String>,
    pub options: Vec<String>,
    /// Whether the LAST option is the abstention, which can only be chosen alone
    /// and never wins.
    #[serde(default = "yes")]
    pub blank: bool,
    #[serde(default = "one")]
    pub min: usize,
    #[serde(default = "one")]
    pub max: usize,
    #[serde(default)]
    pub secret: bool,
    #[serde(default)]
    pub hide_tally: bool,
}

/// The ballot rules a request describes, or what is wrong with them.
fn rules_of(body: &OpenPollBody) -> Result<(Vec<String>, String, String), &'static str> {
    let clean = |s: &str| s.trim().to_string();
    let title = clean(&body.title);
    let question = body.question.as_deref().map_or(title.clone(), clean);
    let options: Vec<String> = body.options.iter().map(|o| clean(o)).collect();
    let too_long = |s: &String| s.chars().count() > MAX_TEXT_CHARS;
    if title.is_empty() || question.is_empty() || too_long(&title) || too_long(&question) {
        return Err("a poll needs a title, and a question of reasonable length");
    }
    if options.len() < 2 || options.len() > MAX_OPTIONS {
        return Err("a poll needs between 2 and 100 options");
    }
    if options.iter().any(|o| o.is_empty() || too_long(o)) {
        return Err("an option is empty or too long");
    }
    if options.iter().collect::<HashSet<_>>().len() != options.len() {
        return Err("two options read the same");
    }
    let choosable = options.len() - usize::from(body.blank);
    if body.min < 1 || body.min > body.max || body.max > choosable {
        return Err("min and max must satisfy 1 <= min <= max <= the options that can win");
    }
    Ok((options, title, question))
}

enum OpenError {
    NoVoters,
    Write(WriteError),
}

impl<E: Into<WriteError>> From<E> for OpenError {
    fn from(e: E) -> Self {
        OpenError::Write(e.into())
    }
}

/// `com.example.wiki.openPoll` (procedure): an owner of the context puts
/// something to the vote. The roster is frozen here, and for a secret poll the
/// issuer key is made here and published with the poll, before any ballot.
pub async fn open_poll(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<OpenPollBody>,
) -> Response {
    let what = "openPoll";
    let store = crate::Store::new(state.db.clone());
    let parent = match store.parent_of(&body.parent_id).await {
        Ok(Some(parent)) => parent,
        Ok(None) => return invalid("no such parent"),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = owner_of(&state, &parent.context_id, &did, what).await {
        return refusal;
    }
    if !POLLABLE.contains(&parent.kind.as_str()) {
        return invalid("a poll is opened on a motion, an amendment, a question or a position");
    }
    let (options, title, question) = match rules_of(&body) {
        Ok(rules) => rules,
        Err(why) => return invalid(why),
    };

    let id = format!("d-{}", crate::util::random_token(16));
    let keys = if body.secret {
        // Keygen is a second or so of arithmetic: off the async threads.
        let made = tokio::task::spawn_blocking(|| TokenIssuer::new_for_poll(ISSUER_BITS)).await;
        let sealed = made.ok().and_then(Result::ok).and_then(|issuer| {
            let public = issuer.public_key().to_der().ok()?;
            let sealed = seal(&state.config, &id, &issuer.secret_der().ok()?)?;
            Some((crate::util::b64url(&public), sealed, Arc::new(issuer)))
        });
        match sealed {
            Some(keys) => Some(keys),
            None => return write_failed(what, "could not make the poll's issuer key"),
        }
    } else {
        None
    };

    let opened = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let written: Result<(), OpenError> = async {
            let new = NewDocument {
                context_id: &parent.context_id,
                parent_id: Some(&body.parent_id),
                kind: "poll",
                title: &title,
                content: None,
                data: None,
                author_did: &did,
                credited: false,
            };
            store
                .insert_document(&conn, &id, &body.parent_id, &new)
                .await?;
            let key = |part: fn(&(String, String, Arc<TokenIssuer>)) -> &String| {
                keys.as_ref()
                    .map_or(Value::Null, |keys| Value::Text(part(keys).clone()))
            };
            conn.execute(
                "INSERT INTO poll (id, context_id, question, options, min_choices, max_choices, \
                   blank, secret, hide_tally, issuer_pubkey, issuer_secret) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
                vec![
                    Value::Text(id.clone()),
                    Value::Text(parent.context_id.clone()),
                    Value::Text(question.clone()),
                    Value::Text(serde_json::to_string(&options).unwrap_or_default()),
                    Value::Integer(body.min as i64),
                    Value::Integer(body.max as i64),
                    Value::Integer(i64::from(body.blank)),
                    Value::Integer(i64::from(body.secret)),
                    Value::Integer(i64::from(body.hide_tally)),
                    key(|keys| &keys.0),
                    key(|keys| &keys.1),
                ],
            )
            .await?;
            // The roster: whoever holds voting rights in the context right now.
            let voters = conn
                .execute(
                    "INSERT OR IGNORE INTO eligibility (poll_id, did, base_weight) \
                     SELECT ?1, m.user_did, 1 FROM member m \
                     WHERE m.context_id = ?2 AND m.active = 1 AND m.user_did IS NOT NULL",
                    [id.as_str(), parent.context_id.as_str()],
                )
                .await?;
            if voters == 0 {
                return Err(OpenError::NoVoters);
            }
            ballot_store::freeze_at_open(&conn, &id).await?;
            Ok(())
        }
        .await;
        conn.execute(
            if written.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            },
            (),
        )
        .await?;
        written
    }
    .await;
    match opened {
        Ok(()) => {}
        Err(OpenError::NoVoters) => {
            return conflict(
                "NoVoters",
                "nobody in this context holds voting rights, so nobody could vote",
            );
        }
        Err(OpenError::Write(WriteError::Db(e))) => return write_failed(what, e),
        Err(OpenError::Write(refused)) => return invalid(&refused.to_string()),
    }
    if let Some((_, _, issuer)) = keys {
        state.polls.issuers.lock().await.insert(id.clone(), issuer);
    }
    state.publish(Topic::Context(parent.context_id.clone()), "poll", &id);
    match load(&state, &id, Reader::Caller(Some(&did))).await {
        Ok(Some(poll)) => answer(&state, poll, Some(&did), what).await,
        Ok(None) => write_failed(what, "the poll is not there after being opened"),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PollIdBody {
    pub id: String,
}

/// `com.example.wiki.closePoll` (procedure): an owner ends the vote. The result
/// is counted once, here, and stored, and a secret poll's issuer key is
/// destroyed. Closing a closed poll answers with the same result.
pub async fn close_poll(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<PollIdBody>,
) -> Response {
    let what = "closePoll";
    let poll = match load(&state, &body.id, Reader::Caller(Some(&did))).await {
        Ok(Some(poll)) => poll,
        Ok(None) => return no_such_poll(),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = owner_of(&state, &poll.context_id, &did, what).await {
        return refusal;
    }
    if poll.open
        && let Err(e) = close(&state, &poll).await
    {
        return write_failed(what, e);
    }
    state.publish(Topic::Context(poll.context_id.clone()), "poll", &poll.id);
    match load(&state, &body.id, Reader::Caller(Some(&did))).await {
        Ok(Some(poll)) => answer(&state, poll, Some(&did), what).await,
        Ok(None) => no_such_poll(),
        Err(e) => write_failed(what, e),
    }
}

async fn close(state: &AppState, poll: &PollRow) -> Result<(), Failure> {
    let _turn = state.db.write_turn().await;
    // First, so that nothing can land while the result is counted: the board
    // refuses a cast from here on, and an open ballot checks `poll.open` inside
    // its own transaction, which this one excludes.
    if poll.secret {
        crate::ballot::board(state, &poll.id).await?.close().await?;
    }
    let conn = state.db.acquire().await?;
    conn.execute("BEGIN IMMEDIATE", ()).await?;
    let written: Result<(), Failure> = async {
        conn.execute("UPDATE poll SET open = 0 WHERE id = ?1", [poll.id.as_str()])
            .await?;
        state
            .polls
            .tallies
            .lock()
            .expect("tallies")
            .remove(&poll.id);
        let result = tally(state, poll).await?;
        let mut rows = conn
            .query(
                "SELECT coalesce(sum(e.resolved_weight), 0) FROM token_issued t \
                 JOIN eligibility e ON e.poll_id = t.poll_id AND e.did = t.did \
                 WHERE t.poll_id = ?1",
                [poll.id.as_str()],
            )
            .await?;
        let issued = rows
            .next()
            .await?
            .and_then(|row| natural(&row, 0))
            .unwrap_or(0);
        drop(rows);
        if poll.secret && result.ballots > issued {
            tracing::error!(
                "poll {} closed with {} ballots on its board and {issued} tokens issued",
                poll.id,
                result.ballots
            );
        }
        conn.execute(
            "UPDATE poll SET counts = ?2, ballots = ?3, issued = ?4, issuer_secret = NULL, \
               closed_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
             WHERE id = ?1",
            vec![
                Value::Text(poll.id.clone()),
                Value::Text(serde_json::to_string(&result.counts)?),
                Value::Integer(result.ballots as i64),
                Value::Integer(issued as i64),
            ],
        )
        .await?;
        Ok(())
    }
    .await;
    conn.execute(
        if written.is_ok() {
            "COMMIT"
        } else {
            "ROLLBACK"
        },
        (),
    )
    .await?;
    written?;
    state.polls.issuers.lock().await.remove(&poll.id);
    state
        .polls
        .tallies
        .lock()
        .expect("tallies")
        .remove(&poll.id);
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct IdParam {
    pub id: String,
}

/// `com.example.wiki.getPoll`: a poll as the caller may see it.
pub async fn get_poll(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<IdParam>,
) -> Response {
    match load(&state, &p.id, Reader::Caller(caller.did())).await {
        Ok(Some(poll)) => answer(&state, poll, caller.did(), "getPoll").await,
        Ok(None) => no_such_poll(),
        Err(e) => write_failed("getPoll", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ParentParam {
    pub parent: String,
}

/// `com.example.wiki.listPolls`: the polls opened on a node, newest first.
pub async fn list_polls(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<ParentParam>,
) -> Response {
    let what = "listPolls";
    let ids = async {
        let conn = state.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT p.id FROM poll p JOIN document d ON d.id = p.id \
                 WHERE d.parent_id = ?1 AND d.deleted_at IS NULL \
                 ORDER BY p.created_at DESC, p.id",
                [p.parent.as_str()],
            )
            .await?;
        let mut ids = Vec::new();
        while let Some(row) = rows.next().await? {
            ids.push(row.get::<String>(0)?);
        }
        Ok::<_, DbError>(ids)
    };
    let ids = match ids.await {
        Ok(ids) => ids,
        Err(e) => return write_failed(what, e),
    };
    let mut polls = Vec::new();
    for id in ids {
        // Through the gate one by one: a poll the caller may not read is left
        // out, as it would be missing to `getPoll`.
        let poll = match load(&state, &id, Reader::Caller(caller.did())).await {
            Ok(Some(poll)) => poll,
            Ok(None) => continue,
            Err(e) => return write_failed(what, e),
        };
        match view(&state, poll, caller.did()).await {
            Ok(view) => polls.push(view),
            Err(e) => return write_failed(what, e),
        }
    }
    (StatusCode::OK, Json(serde_json::json!({ "polls": polls }))).into_response()
}

#[derive(Debug, Deserialize)]
pub struct BoardParam {
    pub poll: String,
}

/// `com.example.wiki.getBoard`: every ballot of a secret poll, for whoever may
/// see its counts, so that they can count for themselves. In token order, which
/// says nothing, and not in the order the room voted in.
pub async fn get_board(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<BoardParam>,
) -> Response {
    let what = "getBoard";
    let poll = match load(&state, &p.poll, Reader::Caller(caller.did())).await {
        Ok(Some(poll)) if poll.secret => poll,
        Ok(Some(_)) => return invalid("only a secret poll has a board"),
        Ok(None) => return no_such_poll(),
        Err(e) => return write_failed(what, e),
    };
    match sees_counts(&state, &poll, caller.did()).await {
        Ok(true) => {}
        Ok(false) => return forbidden("this poll shows its ballots to the owners of its context"),
        Err(e) => return write_failed(what, e),
    }
    let entries = async {
        let board = crate::ballot::board(&state, &poll.id).await?;
        Ok::<_, Failure>(board.ballots().await?)
    };
    match entries.await {
        Ok(entries) => {
            let entries: Vec<ProvisionalEntry> = entries.iter().map(encode_entry).collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({
                    "issuer_pubkey": poll.issuer_pubkey,
                    "open": poll.open,
                    "entries": entries,
                })),
            )
                .into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct BoardEntryParams {
    pub poll: String,
    /// base64url, as it was cast.
    pub token: String,
}

/// `com.example.wiki.getBoardEntry`: the ballot a token was spent on. How a
/// voter checks that theirs is on the board and says what they said, which
/// works where the counts are hidden too: knowing a token is having cast it.
pub async fn get_board_entry(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<BoardEntryParams>,
) -> Response {
    let what = "getBoardEntry";
    let poll = match load(&state, &p.poll, Reader::Caller(caller.did())).await {
        Ok(Some(poll)) if poll.secret => poll,
        Ok(Some(_)) => return invalid("only a secret poll has a board"),
        Ok(None) => return no_such_poll(),
        Err(e) => return write_failed(what, e),
    };
    let Ok(token) = crate::util::b64url_decode(&p.token) else {
        return invalid("the token is not base64url");
    };
    let found = async {
        let board = crate::ballot::board(&state, &poll.id).await?;
        Ok::<_, Failure>(board.find(&token).await?)
    };
    match found.await {
        Ok(Some((position, entry))) => (
            StatusCode::OK,
            Json(serde_json::json!({ "position": position, "entry": encode_entry(&entry) })),
        )
            .into_response(),
        Ok(None) => err(
            StatusCode::NOT_FOUND,
            "NotFound",
            "no ballot spent that token",
        ),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct IssueBody {
    pub poll: String,
    /// One blinded token per unit of the caller's weight, base64url.
    pub blinded: Vec<String>,
}

/// `com.example.wiki.issueBallotTokens` (procedure): blind-sign the caller's
/// tokens, once. The server signs what it cannot read, and records only that
/// this voter was served.
///
/// Asking again with the SAME blinded tokens answers with the same signatures:
/// a reply lost on the way must not cost a vote, and signing the same thing
/// twice mints nothing. Different tokens are refused.
pub async fn issue_ballot_tokens(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<IssueBody>,
) -> Response {
    let what = "issueBallotTokens";
    let poll = match load(&state, &body.poll, Reader::Caller(Some(&did))).await {
        Ok(Some(poll)) if poll.secret => poll,
        Ok(Some(_)) => return invalid("an open poll takes a ballot, not tokens"),
        Ok(None) => return no_such_poll(),
        Err(e) => return write_failed(what, e),
    };
    if !poll.open {
        return poll_closed();
    }
    let weight = match weight_of(&state, &poll.id, &did).await {
        Ok(Some(weight)) if weight > 0 => weight,
        Ok(Some(_)) => return forbidden("your vote in this poll is with your delegate"),
        Ok(None) => {
            return err(
                StatusCode::FORBIDDEN,
                "NotEligible",
                "you did not hold voting rights when this poll opened",
            );
        }
        Err(e) => return write_failed(what, e),
    };
    if body.blinded.len() as u64 != weight {
        return invalid(&format!("send exactly {weight} blinded tokens"));
    }
    let Ok(blinded) = body
        .blinded
        .iter()
        .map(|b| crate::util::b64url_decode(b))
        .collect::<Result<Vec<_>, _>>()
    else {
        return invalid("a blinded token is not base64url");
    };
    let mut digest = Sha256::new();
    for message in &blinded {
        digest.update((message.len() as u64).to_be_bytes());
        digest.update(message);
    }
    let request_hash: String = digest
        .finalize()
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect();

    let issuer = match issuer_of(&state, &poll).await {
        Ok(issuer) => issuer,
        Err(e) => return write_failed(what, e),
    };
    // Signed BEFORE the marker is written and released only after it: a token
    // that does not sign must not use up the voter's one issuance.
    let signing = tokio::task::spawn_blocking(move || {
        blinded
            .iter()
            .map(|message| {
                issuer
                    .blind_sign(message)
                    .map(|sig| crate::util::b64url(&sig.0))
            })
            .collect::<Result<Vec<_>, _>>()
    });
    let signatures = match signing.await {
        Ok(Ok(signatures)) => signatures,
        Ok(Err(_)) => return invalid("a blinded token is not one this poll's key can sign"),
        Err(e) => return write_failed(what, e),
    };

    let served = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        // Neither the order of these rows nor a clock on them may survive: set
        // beside the board's positions, either would pair voters with ballots.
        // So a random rowid, and the poll's opening time for every voter.
        let rowid = i64::from_be_bytes(
            crate::util::random_bytes(8)
                .try_into()
                .expect("eight bytes"),
        ) & i64::MAX;
        conn.execute(
            "INSERT INTO token_issued (rowid, poll_id, did, request_hash, issued_at) \
             VALUES (?1, ?2, ?3, ?4, ?5) ON CONFLICT(poll_id, did) DO NOTHING",
            vec![
                Value::Integer(rowid),
                Value::Text(poll.id.clone()),
                Value::Text(did.clone()),
                Value::Text(request_hash.clone()),
                Value::Text(poll.created_at.clone()),
            ],
        )
        .await?;
        let mut rows = conn
            .query(
                "SELECT request_hash FROM token_issued WHERE poll_id = ?1 AND did = ?2",
                [poll.id.as_str(), did.as_str()],
            )
            .await?;
        Ok::<_, DbError>(rows.next().await?.and_then(|row| text(&row, 0)))
    };
    match served.await {
        Ok(Some(recorded)) if recorded == request_hash => (
            StatusCode::OK,
            Json(serde_json::json!({ "signatures": signatures })),
        )
            .into_response(),
        Ok(_) => conflict(
            "AlreadyIssued",
            "your tokens for this poll were already issued, to the device that asked first",
        ),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CastBody {
    pub poll: String,
    #[serde(flatten)]
    pub entry: ProvisionalEntry,
}

/// `com.example.wiki.castBallot` (procedure): spend one token on one ballot.
///
/// Takes NO session, on purpose, and reads none if one is sent: the token is
/// the right to vote, and a ballot that arrived with its voter's name on the
/// request would undo what blinding the token bought.
pub async fn cast_ballot(State(state): State<AppState>, Json(body): Json<CastBody>) -> Response {
    let what = "castBallot";
    let poll = match load(&state, &body.poll, Reader::Anyone).await {
        Ok(Some(poll)) if poll.secret => poll,
        Ok(Some(_)) => return invalid("an open poll takes a signed-in ballot"),
        Ok(None) => return no_such_poll(),
        Err(e) => return write_failed(what, e),
    };
    if !poll.open {
        return poll_closed();
    }
    let bad_token = || {
        err(
            StatusCode::FORBIDDEN,
            "BadToken",
            "the token is not one this poll issued",
        )
    };
    let Ok((token, randomizer, signature)) = decode_bytes(&body.entry) else {
        return invalid("the ballot is not base64url");
    };
    if token.len() != TOKEN_NULLIFIER_LEN {
        return bad_token();
    }
    let msg_randomizer = match randomizer.map(<[u8; 32]>::try_from) {
        Some(Ok(bytes)) => Some(MessageRandomizer(bytes)),
        Some(Err(_)) => return bad_token(),
        None => None,
    };
    let public = poll
        .issuer_pubkey
        .as_deref()
        .and_then(|b64| crate::util::b64url_decode(b64).ok())
        .and_then(|der| IssuerPublicKey::from_der(&der).ok());
    let Some(public) = public else {
        return write_failed(what, "the poll has no readable issuer key");
    };
    let entry = BoardEntry {
        token,
        msg_randomizer,
        signature: Signature(signature),
        choices: body.entry.choices.clone(),
    };
    let cast = async {
        let board = crate::ballot::board(&state, &poll.id).await?;
        let _turn = state.db.write_turn().await;
        Ok::<_, DbError>(board.cast(&public, &poll.rules(), entry).await)
    };
    match cast.await {
        Ok(Ok(position)) => {
            announce_tally(&state, &poll);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "position": position })),
            )
                .into_response()
        }
        Ok(Err(BoardError::Cast(CastError::BadSignature))) => bad_token(),
        Ok(Err(BoardError::Cast(CastError::DoubleSpend))) => conflict(
            "AlreadySpent",
            "that token has already been spent on a ballot",
        ),
        Ok(Err(BoardError::Cast(CastError::Invalid(why)))) => err(
            StatusCode::BAD_REQUEST,
            "InvalidBallot",
            &format!("the ballot breaks the poll's rules: {why:?}"),
        ),
        Ok(Err(BoardError::Closed)) => poll_closed(),
        Ok(Err(e)) => write_failed(what, e),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct OpenBallotBody {
    pub poll: String,
    pub choices: Vec<usize>,
}

enum OpenBallot {
    Cast,
    AlreadyVoted,
    Closed,
}

/// `com.example.wiki.castOpenBallot` (procedure): vote in a poll that is not
/// secret. One ballot per voter, counted at their frozen weight; the first
/// stands.
pub async fn cast_open_ballot(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<OpenBallotBody>,
) -> Response {
    let what = "castOpenBallot";
    let poll = match load(&state, &body.poll, Reader::Caller(Some(&did))).await {
        Ok(Some(poll)) if !poll.secret => poll,
        Ok(Some(_)) => return invalid("a secret poll takes tokens, not a signed-in ballot"),
        Ok(None) => return no_such_poll(),
        Err(e) => return write_failed(what, e),
    };
    if !poll.open {
        return poll_closed();
    }
    let weight = match weight_of(&state, &poll.id, &did).await {
        Ok(Some(weight)) if weight > 0 => weight,
        Ok(Some(_)) => return forbidden("your vote in this poll is with your delegate"),
        Ok(None) => {
            return err(
                StatusCode::FORBIDDEN,
                "NotEligible",
                "you did not hold voting rights when this poll opened",
            );
        }
        Err(e) => return write_failed(what, e),
    };
    if let Err(why) = poll.rules().validate(&body.choices) {
        return err(
            StatusCode::BAD_REQUEST,
            "InvalidBallot",
            &format!("the ballot breaks the poll's rules: {why:?}"),
        );
    }
    let cast = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let outcome: Result<OpenBallot, DbError> = async {
            // Inside the transaction, which a close cannot overlap: a ballot is
            // either in before the count or refused.
            let mut rows = conn
                .query("SELECT open FROM poll WHERE id = ?1", [poll.id.as_str()])
                .await?;
            let open = rows.next().await?.and_then(|row| natural(&row, 0)) == Some(1);
            drop(rows);
            if !open {
                return Ok(OpenBallot::Closed);
            }
            let written = conn
                .execute(
                    "INSERT INTO open_ballot (poll_id, did, weight, choices) \
                     VALUES (?1, ?2, ?3, ?4) ON CONFLICT(poll_id, did) DO NOTHING",
                    vec![
                        Value::Text(poll.id.clone()),
                        Value::Text(did.clone()),
                        Value::Integer(weight as i64),
                        Value::Text(serde_json::to_string(&body.choices).unwrap_or_default()),
                    ],
                )
                .await?;
            Ok(if written == 0 {
                OpenBallot::AlreadyVoted
            } else {
                OpenBallot::Cast
            })
        }
        .await;
        conn.execute(
            if outcome.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            },
            (),
        )
        .await?;
        outcome
    };
    match cast.await {
        Ok(OpenBallot::Cast) => {
            announce_tally(&state, &poll);
            (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Ok(OpenBallot::AlreadyVoted) => {
            conflict("AlreadyVoted", "you have already voted in this poll")
        }
        Ok(OpenBallot::Closed) => poll_closed(),
        Err(e) => write_failed(what, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{get, get_as, join, post, seeded_state, token_for};
    use ballot_spec::{BlindSignature, TokenRequest, finalize_token, request_token};
    use serde_json::json;

    const OPEN: &str = "/xrpc/com.example.wiki.openPoll";
    const CLOSE: &str = "/xrpc/com.example.wiki.closePoll";
    const ISSUE: &str = "/xrpc/com.example.wiki.issueBallotTokens";
    const CAST: &str = "/xrpc/com.example.wiki.castBallot";
    const CAST_OPEN: &str = "/xrpc/com.example.wiki.castOpenBallot";

    /// The seeded closed group (alice owns it, bob votes in it, ivan is a member
    /// without voting rights) with a motion to vote on.
    async fn state() -> AppState {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
             VALUES ('mo1', 'c9', 'c9', 'policy', 'Motion One', 'motion-one', 'closed/motion-one')",
            (),
        )
        .await
        .expect("motion");
        state
    }

    fn for_against(secret: bool) -> serde_json::Value {
        json!({
            "parent_id": "mo1", "title": "Motion One",
            "options": ["for", "against", "blank"], "secret": secret,
        })
    }

    async fn open(state: &AppState, who: &str, body: serde_json::Value) -> serde_json::Value {
        let (status, v) = post(router(state.clone()), OPEN, Some(who), body).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        v
    }

    async fn poll_as(state: &AppState, id: &str, who: &str) -> serde_json::Value {
        let uri = format!("/xrpc/com.example.wiki.getPoll?id={id}");
        let (status, v) = get_as(router(state.clone()), &uri, who).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        v
    }

    fn issuer_key(poll: &serde_json::Value) -> IssuerPublicKey {
        let der = crate::util::b64url_decode(poll["issuer_pubkey"].as_str().expect("pubkey"))
            .expect("base64url");
        IssuerPublicKey::from_der(&der).expect("der")
    }

    /// The voter's side of issuance, as the frontend will run it: `n` tokens
    /// blinded, signed by the server, unblinded into ballots ready to cast.
    struct Wallet {
        key: IssuerPublicKey,
        requests: Vec<TokenRequest>,
    }

    impl Wallet {
        fn new(poll: &serde_json::Value, n: usize) -> Self {
            let key = issuer_key(poll);
            let requests = (0..n)
                .map(|_| request_token(&key).expect("blind"))
                .collect();
            Wallet { key, requests }
        }

        fn blinded(&self) -> Vec<String> {
            self.requests
                .iter()
                .map(|r| crate::util::b64url(&r.blinding.blind_message.0))
                .collect()
        }

        /// The ballot the `i`th token makes, given the server's signatures.
        fn ballot(
            &self,
            signed: &serde_json::Value,
            i: usize,
            choices: &[usize],
        ) -> serde_json::Value {
            let blind = crate::util::b64url_decode(signed["signatures"][i].as_str().expect("sig"))
                .expect("base64url");
            let request = &self.requests[i];
            let signature =
                finalize_token(&self.key, request, &BlindSignature(blind)).expect("unblind");
            serde_json::to_value(encode_entry(&BoardEntry {
                token: request.nullifier.clone(),
                msg_randomizer: request.blinding.msg_randomizer,
                signature,
                choices: choices.to_vec(),
            }))
            .expect("json")
        }
    }

    fn with_poll(mut ballot: serde_json::Value, poll_id: &str) -> serde_json::Value {
        ballot["poll"] = json!(poll_id);
        ballot
    }

    /// Issue and cast one ballot for `who`. The cast carries no session.
    async fn vote(
        state: &AppState,
        poll: &serde_json::Value,
        who: &str,
        choices: &[usize],
    ) -> serde_json::Value {
        let id = poll["id"].as_str().expect("id");
        let wallet = Wallet::new(poll, 1);
        let (status, signed) = post(
            router(state.clone()),
            ISSUE,
            Some(who),
            json!({"poll": id, "blinded": wallet.blinded()}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{signed}");
        let ballot = with_poll(wallet.ballot(&signed, 0, choices), id);
        let (status, v) = post(router(state.clone()), CAST, None, ballot.clone()).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        ballot
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_secret_vote_from_opening_to_result() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(true)).await;
        let id = poll["id"].as_str().expect("id");
        assert_eq!(poll["path"], "closed/motion-one/motion_one");
        assert_eq!(poll["open"], true);
        assert_eq!(poll["eligible"], 2, "ivan holds no voting rights: {poll}");

        let seen = poll_as(&state, id, &bob).await;
        assert_eq!(seen["viewer"], json!({"weight": 1, "issued": false}));
        vote(&state, &seen, &bob, &[0]).await;
        vote(&state, &seen, &alice, &[0]).await;

        let running = poll_as(&state, id, &bob).await;
        assert_eq!(running["ballots"], 2);
        assert_eq!(running["counts"], json!([2, 0, 0]));
        assert_eq!(running["outcome"], json!({"kind": "winner", "option": 0}));
        assert_eq!(running["viewer"]["issued"], true);

        let (status, _) = post(router(state.clone()), CLOSE, Some(&bob), json!({"id": id})).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a member closed the poll");
        let (status, closed) = post(
            router(state.clone()),
            CLOSE,
            Some(&alice),
            json!({"id": id}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{closed}");
        assert_eq!(closed["open"], false);
        assert_eq!(closed["counts"], json!([2, 0, 0]));
        assert!(closed["closed_at"].is_string());
        let (_, again) = post(
            router(state.clone()),
            CLOSE,
            Some(&alice),
            json!({"id": id}),
        )
        .await;
        assert_eq!(again["closed_at"], closed["closed_at"], "closing twice");

        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query(
                "SELECT issuer_secret IS NULL, issued, ballots FROM poll WHERE id = ?1",
                [id],
            )
            .await
            .expect("q");
        let row = rows.next().await.expect("next").expect("row");
        assert_eq!(
            row.get::<i64>(0).expect("null"),
            1,
            "the key outlived the poll"
        );
        assert_eq!(row.get::<i64>(1).expect("issued"), 2);
        assert_eq!(row.get::<i64>(2).expect("ballots"), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn nothing_is_taken_after_the_close() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(true)).await;
        let id = poll["id"].as_str().expect("id");

        // Bob holds a good token when the chair closes the poll.
        let wallet = Wallet::new(&poll, 1);
        let (_, signed) = post(
            router(state.clone()),
            ISSUE,
            Some(&bob),
            json!({"poll": id, "blinded": wallet.blinded()}),
        )
        .await;
        post(
            router(state.clone()),
            CLOSE,
            Some(&alice),
            json!({"id": id}),
        )
        .await;

        let late = with_poll(wallet.ballot(&signed, 0, &[1]), id);
        let (status, v) = post(router(state.clone()), CAST, None, late.clone()).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "PollClosed");
        // And not by the board either, were the handler's own check raced past.
        let board = crate::ballot::board(&state, id).await.expect("board");
        assert!(board.is_closed().await.expect("sealed"));

        let (status, v) = post(
            router(state.clone()),
            ISSUE,
            Some(&alice),
            json!({"poll": id, "blinded": Wallet::new(&poll, 1).blinded()}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(poll_as(&state, id, &bob).await["ballots"], 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn tokens_are_issued_once_and_a_lost_reply_can_be_asked_for_again() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(true)).await;
        let id = poll["id"].as_str().expect("id");
        let issue = |who: String, blinded: Vec<String>| {
            let state = state.clone();
            async move {
                let body = json!({"poll": id, "blinded": blinded});
                post(router(state), ISSUE, Some(&who), body).await
            }
        };

        let wallet = Wallet::new(&poll, 1);
        let (status, v) = issue(bob.clone(), Wallet::new(&poll, 2).blinded()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "two tokens for a weight of one: {v}"
        );
        let (status, first) = issue(bob.clone(), wallet.blinded()).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a refused request used up the issuance: {first}"
        );

        let (status, again) = issue(bob.clone(), wallet.blinded()).await;
        assert_eq!(status, StatusCode::OK, "{again}");
        assert_eq!(again, first, "the same request is answered the same");
        let (status, v) = issue(bob.clone(), Wallet::new(&poll, 1).blinded()).await;
        assert_eq!(status, StatusCode::CONFLICT, "a second set of tokens: {v}");
        assert_eq!(v["error"], "AlreadyIssued");

        // What is kept about who was served says nothing of when or in what order.
        let (status, _) = issue(alice.clone(), Wallet::new(&poll, 1).blinded()).await;
        assert_eq!(status, StatusCode::OK);
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn
            .query("SELECT rowid, issued_at FROM token_issued", ())
            .await
            .expect("q");
        while let Some(row) = rows.next().await.expect("next") {
            assert!(
                row.get::<i64>(0).expect("rowid") > 1 << 20,
                "rows numbered as they came"
            );
            assert_eq!(row.get::<String>(1).expect("at"), poll["created_at"]);
        }

        let ivan = token_for(&state, "did:plc:ivan").await;
        let (status, v) = issue(ivan, Wallet::new(&poll, 1).blinded()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        assert_eq!(v["error"], "NotEligible");
        let mallory = token_for(&state, "did:plc:mallory").await;
        let (status, _) = issue(mallory, Wallet::new(&poll, 1).blinded()).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a stranger learned the poll exists"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_token_is_spent_once_and_only_in_its_own_poll() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(true)).await;
        let other = open(&state, &alice, for_against(true)).await;
        let id = poll["id"].as_str().expect("id");
        assert_ne!(
            poll["path"], other["path"],
            "two polls on one motion share a URL"
        );

        let wallet = Wallet::new(&poll, 1);
        let (_, signed) = post(
            router(state.clone()),
            ISSUE,
            Some(&bob),
            json!({"poll": id, "blinded": wallet.blinded()}),
        )
        .await;

        // D8: a ballot that breaks the rules does not use the token up.
        let spoiled = with_poll(wallet.ballot(&signed, 0, &[0, 2]), id);
        let (status, v) = post(router(state.clone()), CAST, None, spoiled).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        assert_eq!(v["error"], "InvalidBallot");

        let elsewhere = with_poll(
            wallet.ballot(&signed, 0, &[0]),
            other["id"].as_str().expect("id"),
        );
        let (status, v) = post(router(state.clone()), CAST, None, elsewhere).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        assert_eq!(v["error"], "BadToken");

        let ballot = with_poll(wallet.ballot(&signed, 0, &[0]), id);
        let (status, v) = post(router(state.clone()), CAST, None, ballot.clone()).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["position"], 0);
        // D4: the first stands, whatever the second says.
        let changed = with_poll(wallet.ballot(&signed, 0, &[1]), id);
        let (status, v) = post(router(state.clone()), CAST, None, changed).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "AlreadySpent");
        assert_eq!(poll_as(&state, id, &bob).await["counts"], json!([1, 0, 0]));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_heavier_voter_gets_that_many_tokens() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(true)).await;
        let id = poll["id"].as_str().expect("id");
        // As the roster would have frozen it, had alice delegated to bob.
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "UPDATE eligibility SET resolved_weight = 2 WHERE did = 'did:plc:bob';
             UPDATE eligibility SET resolved_weight = 0 WHERE did = 'did:plc:alice';",
        )
        .await
        .expect("weights");

        let wallet = Wallet::new(&poll, 2);
        let (status, signed) = post(
            router(state.clone()),
            ISSUE,
            Some(&bob),
            json!({"poll": id, "blinded": wallet.blinded()}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{signed}");
        for (i, choice) in [0usize, 1].into_iter().enumerate() {
            let ballot = with_poll(wallet.ballot(&signed, i, &[choice]), id);
            let (status, v) = post(router(state.clone()), CAST, None, ballot).await;
            assert_eq!(status, StatusCode::OK, "{v}");
        }
        let (status, v) = post(
            router(state.clone()),
            ISSUE,
            Some(&alice),
            json!({"poll": id, "blinded": Vec::<String>::new()}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a delegator was served: {v}");

        let seen = poll_as(&state, id, &bob).await;
        assert_eq!(seen["eligible"], 2, "weight is moved, never made");
        assert_eq!(seen["counts"], json!([1, 1, 0]));
        assert_eq!(seen["outcome"], json!({"kind": "tie"}));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_hidden_tally_is_the_owners_but_a_voter_still_finds_their_ballot() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let mut body = for_against(true);
        body["hide_tally"] = json!(true);
        let poll = open(&state, &alice, body).await;
        let id = poll["id"].as_str().expect("id");
        let ballot = vote(&state, &poll, &bob, &[1]).await;

        let bobs = poll_as(&state, id, &bob).await;
        assert_eq!(bobs["ballots"], 1, "the turnout is not the secret");
        assert!(
            bobs.get("counts").is_none() && bobs.get("outcome").is_none(),
            "{bobs}"
        );
        assert_eq!(
            poll_as(&state, id, &alice).await["counts"],
            json!([0, 1, 0])
        );

        let board = format!("/xrpc/com.example.wiki.getBoard?poll={id}");
        assert_eq!(
            get_as(router(state.clone()), &board, &bob).await.0,
            StatusCode::FORBIDDEN,
            "the board is the counts"
        );
        let (status, v) = get_as(router(state.clone()), &board, &alice).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["entries"].as_array().expect("entries").len(), 1);
        assert_eq!(v["entries"][0]["token"], ballot["token"]);

        let mine = format!(
            "/xrpc/com.example.wiki.getBoardEntry?poll={id}&token={}",
            ballot["token"].as_str().expect("token")
        );
        let (status, v) = get_as(router(state.clone()), &mine, &bob).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["entry"]["choices"], json!([1]));
        assert_eq!(v["position"], 0);
        let nobodys = format!("/xrpc/com.example.wiki.getBoardEntry?poll={id}&token=AAAA");
        assert_eq!(
            get_as(router(state.clone()), &nobodys, &bob).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            get(router(state.clone()), &mine).await.0,
            StatusCode::NOT_FOUND
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_open_poll_counts_named_ballots_at_their_weight() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(false)).await;
        let id = poll["id"].as_str().expect("id");
        assert!(
            poll.get("issuer_pubkey").is_none(),
            "an open poll has no tokens to sign"
        );
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "UPDATE eligibility SET resolved_weight = 3 WHERE did = 'did:plc:bob'",
            (),
        )
        .await
        .expect("weight");
        let cast = |who: String, choices: serde_json::Value| {
            let state = state.clone();
            async move {
                let body = json!({"poll": id, "choices": choices});
                post(router(state), CAST_OPEN, Some(&who), body).await
            }
        };

        let (status, v) = cast(bob.clone(), json!([0, 1])).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "two choices where one is allowed: {v}"
        );
        assert_eq!(cast(bob.clone(), json!([0])).await.0, StatusCode::OK);
        let (status, v) = cast(bob.clone(), json!([1])).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "AlreadyVoted");
        assert_eq!(cast(alice.clone(), json!([2])).await.0, StatusCode::OK);

        let seen = poll_as(&state, id, &bob).await;
        assert_eq!(seen["counts"], json!([3, 0, 1]));
        assert_eq!(seen["ballots"], 4);
        assert_eq!(
            seen["viewer"],
            json!({"weight": 3, "issued": false, "choices": [0]})
        );

        let ivan = token_for(&state, "did:plc:ivan").await;
        assert_eq!(
            cast(ivan.clone(), json!([0])).await.0,
            StatusCode::FORBIDDEN
        );
        let (status, _) = post(
            router(state.clone()),
            CAST,
            None,
            json!({
                "poll": id, "token": "AAAA", "signature": "AAAA", "choices": [0]
            }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a token was taken by an open poll"
        );

        let (_, closed) = post(
            router(state.clone()),
            CLOSE,
            Some(&alice),
            json!({"id": id}),
        )
        .await;
        assert_eq!(closed["counts"], json!([3, 0, 1]));
        assert_eq!(closed["outcome"], json!({"kind": "winner", "option": 0}));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_an_owner_opens_a_poll_and_only_on_something_votable() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let mallory = token_for(&state, "did:plc:mallory").await;
        let try_open = |who: String, body: serde_json::Value| {
            let state = state.clone();
            async move { post(router(state), OPEN, Some(&who), body).await }
        };
        assert_eq!(
            try_open(bob, for_against(false)).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            try_open(mallory, for_against(false)).await.0,
            StatusCode::NOT_FOUND
        );

        let edit = |key: &str, value: serde_json::Value| {
            let mut body = for_against(false);
            body[key] = value;
            body
        };
        for (why, body) in [
            ("minutes are not a motion", edit("parent_id", json!("s1"))),
            ("one option is no choice", edit("options", json!(["for"]))),
            (
                "two options that read the same",
                edit("options", json!(["a", "a", "blank"])),
            ),
            (
                "an empty option",
                edit("options", json!(["a", " ", "blank"])),
            ),
            ("more choices than can win", edit("max", json!(3))),
            ("a minimum over the maximum", edit("min", json!(2))),
            ("no title", edit("title", json!("  "))),
        ] {
            let (status, v) = try_open(alice.clone(), body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {v}");
        }

        // A context whose only member holds no voting rights.
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO context (id, kind, name, slug, path) \
               VALUES ('c11', 'group', 'Empty Hall', 'hall', 'hall');
             INSERT INTO member (id, user_did, context_id, role, active) \
               VALUES ('m-alice-11', 'did:plc:alice', 'c11', 'owner', 0);
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('mo2', 'c11', 'c11', 'policy', 'Motion Two', 'motion-two', 'hall/motion-two');",
        )
        .await
        .expect("seed");
        let (status, v) = try_open(alice.clone(), edit("parent_id", json!("mo2"))).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "NoVoters");
        let mut rows = conn
            .query("SELECT count(*) FROM document WHERE kind = 'poll'", ())
            .await
            .expect("q");
        let left: i64 = rows
            .next()
            .await
            .expect("next")
            .expect("row")
            .get(0)
            .expect("n");
        assert_eq!(left, 0, "a refused poll left its place in the tree behind");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_poll_has_a_place_in_the_tree_and_leaves_with_it() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(false)).await;
        let id = poll["id"].as_str().expect("id");

        let (_, node) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getNode?path=closed/motion-one/motion_one",
            &bob,
        )
        .await;
        assert_eq!(node["node"]["id"], id, "{node}");
        assert_eq!(node["node"]["kind"], "poll");
        assert_eq!(
            node["node"]["authors"],
            json!([]),
            "the chair did not write the motion"
        );

        let list = "/xrpc/com.example.wiki.listPolls?parent=mo1";
        let (_, v) = get_as(router(state.clone()), list, &bob).await;
        assert_eq!(v["polls"].as_array().expect("polls").len(), 1);
        assert_eq!(v["polls"][0]["id"], id);
        let mallory = token_for(&state, "did:plc:mallory").await;
        let (_, v) = get_as(router(state.clone()), list, &mallory).await;
        assert_eq!(
            v["polls"],
            json!([]),
            "a stranger was shown a closed group's poll"
        );

        // The motion goes to the bin, and the poll with it.
        let (status, _) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.deleteDocument",
            Some(&alice),
            json!({"id": "mo1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let uri = format!("/xrpc/com.example.wiki.getPoll?id={id}");
        assert_eq!(
            get_as(router(state.clone()), &uri, &bob).await.0,
            StatusCode::NOT_FOUND
        );
        let (status, _) = post(
            router(state.clone()),
            CAST_OPEN,
            Some(&bob),
            json!({"poll": id, "choices": [0]}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "a binned poll took a ballot");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_room_voting_at_once_is_told_once_a_beat() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let poll = open(&state, &alice, for_against(false)).await;
        let row = load(&state, poll["id"].as_str().expect("id"), Reader::Anyone)
            .await
            .expect("load")
            .expect("poll");
        let mut changes = state.changes.subscribe();
        for _ in 0..50 {
            announce_tally(&state, &row);
        }
        tokio::time::sleep(TALLY_BEAT * 3).await;
        let mut told = 0;
        while let Ok(change) = changes.try_recv() {
            assert_eq!(
                (change.kind, change.id.as_str()),
                ("tally", row.id.as_str())
            );
            told += 1;
        }
        assert_eq!(told, 1, "fifty ballots in one beat");

        announce_tally(&state, &row);
        tokio::time::sleep(TALLY_BEAT * 3).await;
        assert!(
            changes.try_recv().is_ok(),
            "the ballot after the beat was never announced"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_issuer_key_is_sealed_and_survives_a_restart() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let poll = open(&state, &alice, for_against(true)).await;
        let id = poll["id"].as_str().expect("id");
        let row = load(&state, id, Reader::Anyone)
            .await
            .expect("load")
            .expect("poll");
        let sealed = row.issuer_secret.clone().expect("sealed key");

        assert!(unseal(&state.config, id, &sealed).is_some());
        assert!(
            unseal(&state.config, "another-poll", &sealed).is_none(),
            "moved to another row"
        );
        let mut stolen = state.config.clone();
        stolen.secret = crate::config::Secret::new("not the secret");
        assert!(
            unseal(&stolen, id, &sealed).is_none(),
            "opened without the secret"
        );

        // A restart: the same database and secret, nothing kept in memory.
        let mut restarted = AppState::new(state.db.clone(), state.config.clone());
        vote(&restarted, &poll, &bob, &[0]).await;

        // The same, with the secret lost: what was issued can still be cast, and
        // nothing more can be issued.
        restarted = AppState::new(state.db.clone(), stolen);
        let (status, v) = post(
            router(restarted.clone()),
            ISSUE,
            Some(&alice),
            json!({"poll": id, "blinded": Wallet::new(&poll, 1).blinded()}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_GATEWAY, "{v}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_room_voting_at_once_is_all_counted() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let mut voters = Vec::new();
        for n in 0..24 {
            let did = format!("did:plc:voter{n}");
            voters.push(token_for(&state, &did).await);
            join(&state, &did, "c9").await;
        }
        let poll = open(&state, &alice, for_against(false)).await;
        let id = poll["id"].as_str().expect("id").to_string();

        let casts = voters.into_iter().enumerate().map(|(n, who)| {
            let (state, id) = (state.clone(), id.clone());
            tokio::spawn(async move {
                let body = json!({"poll": id, "choices": [n % 2]});
                post(router(state), CAST_OPEN, Some(&who), body).await.0
            })
        });
        for cast in casts.collect::<Vec<_>>() {
            assert_eq!(cast.await.expect("join"), StatusCode::OK);
        }
        let seen = poll_as(&state, &id, &alice).await;
        assert_eq!(seen["counts"], json!([12, 12, 0]));
        assert_eq!(seen["eligible"], 26);
    }
}
