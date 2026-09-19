//! Speaker lists: who has asked for the floor, and in what order they get it.
//!
//! A context holds one or more lists. A member joins one with a kind of
//! contribution, the chair serves the queue, and the room follows it on the
//! projector. It is coordination state, not content: nothing here has a path or
//! a place in the tree, nothing is published, and it is not migrated, because
//! a queue from a meeting that has ended is of no use to the next one.
//!
//! The queue is served in the order the frontend has always sorted it
//! (`src/components/speak.rs`): the chair's override first, then the kind of
//! contribution with a point of order ahead of a speech, then arrival.

use crate::AppState;
use crate::db::{Db, DbError};
use crate::live::Topic;
use crate::session::{Caller, MaybeCaller};
use crate::xrpc::{err, forbidden, invalid, member_of, owner_of, owns, write_failed};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use turso::Value;

/// The tables, applied with the runtime DDL. `kind` is 0 a speech, 1 a question,
/// 2 a clarification, 3 "I was misunderstood", 4 a point of order: the numbers
/// the frontend already stores, where a higher one jumps the queue.
pub const SPEAK_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS speaker_list (
  id              TEXT PRIMARY KEY,
  context_id      TEXT NOT NULL REFERENCES context(id),
  name            TEXT NOT NULL,
  open            INTEGER NOT NULL DEFAULT 1,
  turn_secs       INTEGER NOT NULL DEFAULT 0,
  turn_started_at TEXT,
  created_at      TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
CREATE INDEX IF NOT EXISTS speaker_list_by_context ON speaker_list(context_id);

CREATE TABLE IF NOT EXISTS speaker_entry (
  id          TEXT PRIMARY KEY,
  list_id     TEXT NOT NULL REFERENCES speaker_list(id),
  speaker_did TEXT NOT NULL REFERENCES user(did),
  kind        INTEGER NOT NULL DEFAULT 0 CHECK (kind BETWEEN 0 AND 4),
  idx         INTEGER NOT NULL DEFAULT 0,
  created_at  TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
-- One place in a queue per person per kind: a second tap is the first again.
CREATE UNIQUE INDEX IF NOT EXISTS speaker_entry_once ON speaker_entry(list_id, speaker_did, kind);
"#;

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

/// The order a queue is served in.
const QUEUE_ORDER: &str = "e.idx, e.kind DESC, e.created_at, e.id";

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpeakerList {
    pub id: String,
    pub context_id: String,
    pub name: String,
    /// Whether members may join.
    pub open: bool,
    /// The limit on a turn in seconds. 0 runs no clock.
    pub turn_secs: i64,
    /// When the current turn began, which the clock counts from.
    pub turn_started_at: Option<String>,
    /// First is who has the floor.
    pub queue: Vec<SpeakerEntry>,
}

#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct SpeakerEntry {
    pub id: String,
    pub speaker_did: String,
    pub kind: i64,
    pub created_at: String,
}

/// What authorizing a change to a list or an entry needs to know.
struct Scope {
    context_id: String,
    list_id: String,
    open: bool,
    /// Whose entry it is, when the scope is an entry's.
    speaker_did: Option<String>,
}

#[derive(Clone)]
struct Speak {
    db: Db,
}

impl Speak {
    async fn lists(&self, context_id: &str) -> Result<Vec<SpeakerList>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT id, name, open, turn_secs, turn_started_at FROM speaker_list \
                 WHERE context_id = ?1 ORDER BY created_at, id",
                [context_id],
            )
            .await?;
        let mut lists = Vec::new();
        while let Some(row) = rows.next().await? {
            lists.push(SpeakerList {
                id: row.get::<String>(0)?,
                context_id: context_id.to_string(),
                name: row.get::<String>(1)?,
                open: row.get::<i64>(2)? != 0,
                turn_secs: row.get::<i64>(3)?,
                turn_started_at: match row.get_value(4)? {
                    Value::Text(at) => Some(at),
                    _ => None,
                },
                queue: Vec::new(),
            });
        }
        drop(rows);
        for list in &mut lists {
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT e.id, e.speaker_did, e.kind, e.created_at FROM speaker_entry e \
                         WHERE e.list_id = ?1 ORDER BY {QUEUE_ORDER}"
                    ),
                    [list.id.as_str()],
                )
                .await?;
            while let Some(row) = rows.next().await? {
                list.queue.push(SpeakerEntry {
                    id: row.get::<String>(0)?,
                    speaker_did: row.get::<String>(1)?,
                    kind: row.get::<i64>(2)?,
                    created_at: row.get::<String>(3)?,
                });
            }
        }
        Ok(lists)
    }

    async fn list_scope(&self, list_id: &str) -> Result<Option<Scope>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT context_id, open FROM speaker_list WHERE id = ?1",
                [list_id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(Scope {
            context_id: row.get::<String>(0)?,
            list_id: list_id.to_string(),
            open: row.get::<i64>(1)? != 0,
            speaker_did: None,
        }))
    }

    async fn entry_scope(&self, entry_id: &str) -> Result<Option<Scope>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT l.context_id, l.id, l.open, e.speaker_did \
                 FROM speaker_entry e JOIN speaker_list l ON l.id = e.list_id WHERE e.id = ?1",
                [entry_id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(Scope {
            context_id: row.get::<String>(0)?,
            list_id: row.get::<String>(1)?,
            open: row.get::<i64>(2)? != 0,
            speaker_did: Some(row.get::<String>(3)?),
        }))
    }

    async fn create(&self, context_id: &str, name: &str) -> Result<String, DbError> {
        let id = format!("sl-{}", crate::util::random_token(16));
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT INTO speaker_list (id, context_id, name) VALUES (?1, ?2, ?3)",
            [id.as_str(), context_id, name],
        )
        .await?;
        Ok(id)
    }

    async fn update(
        &self,
        id: &str,
        name: Option<&str>,
        open: Option<bool>,
        turn_secs: Option<i64>,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        if let Some(name) = name {
            conn.execute(
                "UPDATE speaker_list SET name = ?1 WHERE id = ?2",
                [name, id],
            )
            .await?;
        }
        if let Some(open) = open {
            conn.execute(
                "UPDATE speaker_list SET open = ?1 WHERE id = ?2",
                vec![Value::Integer(i64::from(open)), Value::Text(id.to_string())],
            )
            .await?;
        }
        if let Some(secs) = turn_secs {
            // A new limit starts a new turn, or the clock would show the old
            // turn's elapsed time against it.
            conn.execute(
                &format!(
                    "UPDATE speaker_list SET turn_secs = ?1, turn_started_at = {NOW} WHERE id = ?2"
                ),
                vec![Value::Integer(secs), Value::Text(id.to_string())],
            )
            .await?;
        }
        Ok(())
    }

    async fn delete(&self, id: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("DELETE FROM speaker_entry WHERE list_id = ?1", [id])
            .await?;
        conn.execute("DELETE FROM speaker_list WHERE id = ?1", [id])
            .await?;
        Ok(())
    }

    async fn clear(&self, id: &str) -> Result<u64, DbError> {
        let conn = self.db.acquire().await?;
        Ok(conn
            .execute("DELETE FROM speaker_entry WHERE list_id = ?1", [id])
            .await?)
    }

    /// Join a queue. Asking twice for the same kind is asking once: the entry
    /// already there is returned, so a double tap cannot take two places.
    async fn join(&self, list_id: &str, did: &str, kind: i64) -> Result<String, DbError> {
        let id = format!("se-{}", crate::util::random_token(16));
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT OR IGNORE INTO speaker_entry (id, list_id, speaker_did, kind) \
             VALUES (?1, ?2, ?3, ?4)",
            vec![
                Value::Text(id),
                Value::Text(list_id.to_string()),
                Value::Text(did.to_string()),
                Value::Integer(kind),
            ],
        )
        .await?;
        let mut rows = conn
            .query(
                "SELECT id FROM speaker_entry WHERE list_id = ?1 AND speaker_did = ?2 AND kind = ?3",
                vec![
                    Value::Text(list_id.to_string()),
                    Value::Text(did.to_string()),
                    Value::Integer(kind),
                ],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<String>(0)?),
            None => Err(DbError::Turso(turso::Error::QueryReturnedNoRows)),
        }
    }

    async fn remove_entry(&self, entry_id: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("DELETE FROM speaker_entry WHERE id = ?1", [entry_id])
            .await?;
        Ok(())
    }

    /// The floor passes on: whoever had it leaves the queue and the next turn's
    /// clock starts. Returns the entry that was served, if the queue had one.
    async fn next(&self, list_id: &str) -> Result<Option<String>, DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let served = async {
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT e.id FROM speaker_entry e WHERE e.list_id = ?1 \
                         ORDER BY {QUEUE_ORDER} LIMIT 1"
                    ),
                    [list_id],
                )
                .await?;
            let current = match rows.next().await? {
                Some(row) => Some(row.get::<String>(0)?),
                None => None,
            };
            drop(rows);
            if let Some(id) = &current {
                conn.execute("DELETE FROM speaker_entry WHERE id = ?1", [id.as_str()])
                    .await?;
            }
            conn.execute(
                &format!("UPDATE speaker_list SET turn_started_at = {NOW} WHERE id = ?1"),
                [list_id],
            )
            .await?;
            Ok::<_, DbError>(current)
        }
        .await;
        conn.execute(if served.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
            .await?;
        served
    }

    /// The chair's override: ahead of everyone, or behind them.
    async fn move_entry(
        &self,
        entry_id: &str,
        list_id: &str,
        to_front: bool,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        let sql = if to_front {
            "UPDATE speaker_entry SET idx = \
               (SELECT coalesce(min(idx), 0) - 1 FROM speaker_entry WHERE list_id = ?1) \
             WHERE id = ?2"
        } else {
            "UPDATE speaker_entry SET idx = \
               (SELECT coalesce(max(idx), 0) + 1 FROM speaker_entry WHERE list_id = ?1) \
             WHERE id = ?2"
        };
        conn.execute(sql, [list_id, entry_id]).await?;
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// The XRPC methods.
// ---------------------------------------------------------------------------

fn ok() -> Response {
    (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
}

fn no_such(what: &str) -> Response {
    err(StatusCode::NOT_FOUND, "NotFound", what)
}

/// A scope's context, for an owner of it; else the response to send.
async fn as_owner(
    state: &AppState,
    scope: Result<Option<Scope>, DbError>,
    did: &str,
    what: &str,
) -> Result<Scope, Response> {
    let scope = match scope {
        Ok(Some(scope)) => scope,
        Ok(None) => return Err(no_such("no such speaker list")),
        Err(e) => return Err(write_failed(what, e)),
    };
    owner_of(state, &scope.context_id, did, what).await?;
    Ok(scope)
}

fn changed(state: &AppState, scope: &Scope) {
    state.publish(
        Topic::Context(scope.context_id.clone()),
        "speak",
        &scope.list_id,
    );
}

#[derive(Debug, Deserialize)]
pub struct ContextParam {
    pub context: String,
}

/// `com.example.wiki.listSpeakerLists`: a context's lists with their queues, for
/// whoever may read the context: the room follows it on the projector. `now` is
/// the server's clock, so a countdown does not depend on the viewer's.
pub async fn list_speaker_lists(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<ContextParam>,
) -> Response {
    let what = "listSpeakerLists";
    match crate::Store::new(state.db.clone())
        .read_context(&p.context, caller.did())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return no_such("no such context"),
        Err(e) => return write_failed(what, e),
    }
    let lists = match (Speak {
        db: state.db.clone(),
    })
    .lists(&p.context)
    .await
    {
        Ok(lists) => lists,
        Err(e) => return write_failed(what, e),
    };
    let dids = lists
        .iter()
        .flat_map(|l| l.queue.iter().map(|e| e.speaker_did.clone()))
        .collect();
    let profiles = match crate::Store::new(state.db.clone()).profiles(&dids).await {
        Ok(profiles) => profiles,
        Err(e) => return write_failed(what, e),
    };
    let now = crate::util::rfc3339_utc(crate::util::now_secs());
    (
        StatusCode::OK,
        Json(serde_json::json!({ "lists": lists, "profiles": profiles, "now": now })),
    )
        .into_response()
}

#[derive(Debug, Deserialize)]
pub struct CreateListBody {
    pub context_id: String,
    pub name: String,
}

/// `com.example.wiki.createSpeakerList` (procedure): an owner's.
pub async fn create_speaker_list(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CreateListBody>,
) -> Response {
    let what = "createSpeakerList";
    if let Err(refusal) = owner_of(&state, &body.context_id, &did, what).await {
        return refusal;
    }
    let name = body.name.trim();
    if name.is_empty() {
        return invalid("a speaker list needs a name");
    }
    match (Speak {
        db: state.db.clone(),
    })
    .create(&body.context_id, name)
    .await
    {
        Ok(id) => {
            state.publish(Topic::Context(body.context_id), "speak", &id);
            (StatusCode::OK, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateListBody {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub open: Option<bool>,
    #[serde(default)]
    pub turn_secs: Option<i64>,
}

/// `com.example.wiki.updateSpeakerList` (procedure): rename a list, open or
/// close it to new speakers, or set the limit on a turn, which starts the clock
/// afresh.
pub async fn update_speaker_list(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<UpdateListBody>,
) -> Response {
    let what = "updateSpeakerList";
    let speak = Speak {
        db: state.db.clone(),
    };
    let scope = match as_owner(&state, speak.list_scope(&body.id).await, &did, what).await {
        Ok(scope) => scope,
        Err(refusal) => return refusal,
    };
    if body
        .turn_secs
        .is_some_and(|secs| !(0..=86_400).contains(&secs))
    {
        return invalid("a turn is between 0 seconds and a day");
    }
    let name = body
        .name
        .as_deref()
        .map(str::trim)
        .filter(|n| !n.is_empty());
    match speak
        .update(&body.id, name, body.open, body.turn_secs)
        .await
    {
        Ok(()) => {
            changed(&state, &scope);
            ok()
        }
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ListIdBody {
    pub list_id: String,
}

/// `com.example.wiki.deleteSpeakerList` (procedure): the list and its queue.
pub async fn delete_speaker_list(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ListIdBody>,
) -> Response {
    let what = "deleteSpeakerList";
    let speak = Speak {
        db: state.db.clone(),
    };
    let scope = match as_owner(&state, speak.list_scope(&body.list_id).await, &did, what).await {
        Ok(scope) => scope,
        Err(refusal) => return refusal,
    };
    match speak.delete(&body.list_id).await {
        Ok(()) => {
            changed(&state, &scope);
            ok()
        }
        Err(e) => write_failed(what, e),
    }
}

/// `com.example.wiki.clearSpeakerList` (procedure): empty the queue.
pub async fn clear_speaker_list(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ListIdBody>,
) -> Response {
    let what = "clearSpeakerList";
    let speak = Speak {
        db: state.db.clone(),
    };
    let scope = match as_owner(&state, speak.list_scope(&body.list_id).await, &did, what).await {
        Ok(scope) => scope,
        Err(refusal) => return refusal,
    };
    match speak.clear(&body.list_id).await {
        Ok(cleared) => {
            changed(&state, &scope);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "cleared": cleared })),
            )
                .into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

/// `com.example.wiki.nextSpeaker` (procedure): the floor passes on.
pub async fn next_speaker(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ListIdBody>,
) -> Response {
    let what = "nextSpeaker";
    let speak = Speak {
        db: state.db.clone(),
    };
    let scope = match as_owner(&state, speak.list_scope(&body.list_id).await, &did, what).await {
        Ok(scope) => scope,
        Err(refusal) => return refusal,
    };
    match speak.next(&body.list_id).await {
        Ok(served) => {
            changed(&state, &scope);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "served": served })),
            )
                .into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct JoinBody {
    pub list_id: String,
    #[serde(default)]
    pub kind: i64,
}

/// `com.example.wiki.joinSpeakerList` (procedure): ask for the floor. A member
/// of the context may while the list is open; an owner may regardless, to enter
/// someone who cannot.
pub async fn join_speaker_list(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<JoinBody>,
) -> Response {
    let what = "joinSpeakerList";
    let speak = Speak {
        db: state.db.clone(),
    };
    let scope = match speak.list_scope(&body.list_id).await {
        Ok(Some(scope)) => scope,
        Ok(None) => return no_such("no such speaker list"),
        Err(e) => return write_failed(what, e),
    };
    let membership = match member_of(&state, &scope.context_id, &did, what).await {
        Ok(membership) => membership,
        Err(refusal) => return refusal,
    };
    if !scope.open && !owns(membership) {
        return forbidden("the speaker list is closed");
    }
    if !(0..=4).contains(&body.kind) {
        return invalid("kind is 0 to 4");
    }
    match speak.join(&body.list_id, &did, body.kind).await {
        Ok(id) => {
            changed(&state, &scope);
            (StatusCode::OK, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct EntryIdBody {
    pub entry_id: String,
}

/// An entry's scope for whoever may act on it: the speaker, if `or_speaker`, or
/// an owner of the context. To anyone outside the context it is not there.
async fn entry_for(
    state: &AppState,
    entry_id: &str,
    did: &str,
    or_speaker: bool,
    what: &str,
) -> Result<Scope, Response> {
    let scope = match (Speak {
        db: state.db.clone(),
    })
    .entry_scope(entry_id)
    .await
    {
        Ok(Some(scope)) => scope,
        Ok(None) => return Err(no_such("no such entry")),
        Err(e) => return Err(write_failed(what, e)),
    };
    let membership = member_of(state, &scope.context_id, did, what).await?;
    let own = or_speaker && scope.speaker_did.as_deref() == Some(did);
    if own || owns(membership) {
        Ok(scope)
    } else {
        Err(forbidden(
            "only the speaker or an owner of the context may do that",
        ))
    }
}

/// `com.example.wiki.leaveSpeakerList` (procedure): withdraw, or be removed by
/// an owner.
pub async fn leave_speaker_list(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<EntryIdBody>,
) -> Response {
    let what = "leaveSpeakerList";
    let scope = match entry_for(&state, &body.entry_id, &did, true, what).await {
        Ok(scope) => scope,
        Err(refusal) => return refusal,
    };
    match (Speak {
        db: state.db.clone(),
    })
    .remove_entry(&body.entry_id)
    .await
    {
        Ok(()) => {
            changed(&state, &scope);
            ok()
        }
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct MoveBody {
    pub entry_id: String,
    /// `front` or `back`.
    pub to: String,
}

/// `com.example.wiki.moveSpeaker` (procedure): the chair puts someone ahead of
/// the queue or behind it.
pub async fn move_speaker(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<MoveBody>,
) -> Response {
    let what = "moveSpeaker";
    let to_front = match body.to.as_str() {
        "front" => true,
        "back" => false,
        _ => return invalid("to is front or back"),
    };
    let scope = match entry_for(&state, &body.entry_id, &did, false, what).await {
        Ok(scope) => scope,
        Err(refusal) => return refusal,
    };
    match (Speak {
        db: state.db.clone(),
    })
    .move_entry(&body.entry_id, &scope.list_id, to_front)
    .await
    {
        Ok(()) => {
            changed(&state, &scope);
            ok()
        }
        Err(e) => write_failed(what, e),
    }
}

#[cfg(test)]
mod tests {
    use crate::router;
    use crate::xrpc::tests::{get, get_as, join, post, seeded_state, token_for};
    use axum::http::StatusCode;

    /// The closed group c9: alice chairs it, bob and carol are members.
    struct Room {
        state: crate::AppState,
        alice: String,
        bob: String,
        carol: String,
        list: String,
    }

    async fn call(
        state: &crate::AppState,
        method: &str,
        who: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        post(
            router(state.clone()),
            &format!("/xrpc/com.example.wiki.{method}"),
            Some(who),
            body,
        )
        .await
    }

    async fn room() -> Room {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let carol = token_for(&state, "did:plc:carol").await;
        join(&state, "did:plc:carol", "c9").await;
        let (status, v) = call(
            &state,
            "createSpeakerList",
            &alice,
            serde_json::json!({"context_id": "c9", "name": "Talerliste"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let list = v["id"].as_str().expect("id").to_string();
        Room {
            state,
            alice,
            bob,
            carol,
            list,
        }
    }

    impl Room {
        async fn join(&self, who: &str, kind: i64) -> (StatusCode, serde_json::Value) {
            call(
                &self.state,
                "joinSpeakerList",
                who,
                serde_json::json!({"list_id": self.list, "kind": kind}),
            )
            .await
        }

        /// The queue, as the DIDs in the order they get the floor.
        async fn queue(&self) -> Vec<String> {
            let (_, v) = get_as(
                router(self.state.clone()),
                "/xrpc/com.example.wiki.listSpeakerLists?context=c9",
                &self.bob,
            )
            .await;
            v["lists"][0]["queue"]
                .as_array()
                .expect("queue")
                .iter()
                .map(|e| e["speaker_did"].as_str().unwrap().replace("did:plc:", ""))
                .collect()
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_point_of_order_goes_ahead_of_a_speech_and_the_rest_by_arrival() {
        let r = room().await;
        assert_eq!(r.join(&r.bob, 0).await.0, StatusCode::OK);
        assert_eq!(r.join(&r.carol, 0).await.0, StatusCode::OK);
        assert_eq!(r.join(&r.alice, 4).await.0, StatusCode::OK);
        assert_eq!(r.queue().await, ["alice", "bob", "carol"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_second_tap_takes_no_second_place() {
        let r = room().await;
        let (_, first) = r.join(&r.bob, 0).await;
        let (status, second) = r.join(&r.bob, 0).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(first["id"], second["id"]);
        assert_eq!(r.queue().await, ["bob"]);
        // A question beside a speech is a second, different, place.
        assert_eq!(r.join(&r.bob, 1).await.0, StatusCode::OK);
        assert_eq!(r.queue().await.len(), 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_chair_serves_the_queue_and_each_turn_starts_the_clock() {
        let r = room().await;
        r.join(&r.bob, 0).await;
        r.join(&r.carol, 0).await;
        let list = serde_json::json!({"list_id": r.list});
        assert_eq!(
            call(&r.state, "nextSpeaker", &r.bob, list.clone()).await.0,
            StatusCode::FORBIDDEN,
            "a member served the queue"
        );
        let (status, v) = call(&r.state, "nextSpeaker", &r.alice, list.clone()).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert!(v["served"].is_string());
        assert_eq!(r.queue().await, ["carol"]);

        let (_, lists) = get_as(
            router(r.state.clone()),
            "/xrpc/com.example.wiki.listSpeakerLists?context=c9",
            &r.bob,
        )
        .await;
        assert!(lists["lists"][0]["turn_started_at"].is_string(), "{lists}");
        assert!(
            lists["now"].is_string(),
            "the server's clock, for the countdown"
        );

        call(&r.state, "nextSpeaker", &r.alice, list.clone()).await;
        let (status, v) = call(&r.state, "nextSpeaker", &r.alice, list).await;
        assert_eq!(status, StatusCode::OK, "an empty queue is not an error");
        assert!(v["served"].is_null());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_chair_moves_a_speaker_and_a_member_moves_nobody() {
        let r = room().await;
        r.join(&r.bob, 0).await;
        let (_, carol) = r.join(&r.carol, 0).await;
        let to_front = serde_json::json!({"entry_id": carol["id"], "to": "front"});
        assert_eq!(
            call(&r.state, "moveSpeaker", &r.carol, to_front.clone())
                .await
                .0,
            StatusCode::FORBIDDEN,
            "a speaker jumped the queue"
        );
        assert_eq!(
            call(&r.state, "moveSpeaker", &r.alice, to_front).await.0,
            StatusCode::OK
        );
        assert_eq!(r.queue().await, ["carol", "bob"]);
        let to_back = serde_json::json!({"entry_id": carol["id"], "to": "back"});
        assert_eq!(
            call(&r.state, "moveSpeaker", &r.alice, to_back).await.0,
            StatusCode::OK
        );
        assert_eq!(r.queue().await, ["bob", "carol"]);
        // The override outranks even a point of order.
        r.join(&r.alice, 4).await;
        assert_eq!(r.queue().await, ["alice", "bob", "carol"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_list_takes_nobody_but_whom_the_chair_enters() {
        let r = room().await;
        let close = serde_json::json!({"id": r.list, "open": false});
        assert_eq!(
            call(&r.state, "updateSpeakerList", &r.bob, close.clone())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&r.state, "updateSpeakerList", &r.alice, close).await.0,
            StatusCode::OK
        );
        assert_eq!(r.join(&r.bob, 0).await.0, StatusCode::FORBIDDEN);
        assert_eq!(r.join(&r.alice, 0).await.0, StatusCode::OK);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_speaker_withdraws_and_only_the_chair_removes_another() {
        let r = room().await;
        let (_, bob) = r.join(&r.bob, 0).await;
        let (_, carol) = r.join(&r.carol, 0).await;
        let entry = |v: &serde_json::Value| serde_json::json!({"entry_id": v["id"]});
        assert_eq!(
            call(&r.state, "leaveSpeakerList", &r.bob, entry(&carol))
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&r.state, "leaveSpeakerList", &r.bob, entry(&bob))
                .await
                .0,
            StatusCode::OK
        );
        assert_eq!(
            call(&r.state, "leaveSpeakerList", &r.alice, entry(&carol))
                .await
                .0,
            StatusCode::OK
        );
        assert!(r.queue().await.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_groups_queue_is_its_members_business() {
        let r = room().await;
        r.join(&r.bob, 0).await;
        let lists = "/xrpc/com.example.wiki.listSpeakerLists?context=c9";
        assert_eq!(
            get(router(r.state.clone()), lists).await.0,
            StatusCode::NOT_FOUND
        );
        let mallory = token_for(&r.state, "did:plc:mallory").await;
        assert_eq!(
            get_as(router(r.state.clone()), lists, &mallory).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(r.join(&mallory, 0).await.0, StatusCode::NOT_FOUND);
        let (status, _) = call(
            &r.state,
            "createSpeakerList",
            &r.bob,
            serde_json::json!({"context_id": "c9", "name": "Min egen"}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "a member made a speaker list"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn clearing_and_deleting_are_the_chairs() {
        let r = room().await;
        r.join(&r.bob, 0).await;
        r.join(&r.carol, 1).await;
        let list = serde_json::json!({"list_id": r.list});
        let (status, v) = call(&r.state, "clearSpeakerList", &r.alice, list.clone()).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["cleared"], 2);
        r.join(&r.bob, 0).await;
        assert_eq!(
            call(&r.state, "deleteSpeakerList", &r.bob, list.clone())
                .await
                .0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            call(&r.state, "deleteSpeakerList", &r.alice, list).await.0,
            StatusCode::OK
        );
        let (_, v) = get_as(
            router(r.state.clone()),
            "/xrpc/com.example.wiki.listSpeakerLists?context=c9",
            &r.bob,
        )
        .await;
        assert_eq!(
            v["lists"].as_array().map(Vec::len),
            Some(0),
            "with a queue still in it"
        );
    }
}
