//! The canvas: a board of cells a room paints together, one placement per
//! person per cooldown. The interim keeps a canvas as a `canvas/canvas` node and
//! each painted cell as a hidden `canvas/pixel` child of it, with the cooldown
//! in a database trigger; here a canvas has a place in the tree (a document of
//! kind `canvas`) and its cells have a table.
//!
//! The cooldown is what makes it survivable in a hall: it bounds the whole
//! feature's write rate to one placement per person per interval, and it is
//! enforced here, under the write lock, and not by a client asking nicely.
//!
//! A change says only THAT the board moved. A listener asks for the cells painted
//! since the last it has (`getCanvas?since=`), so a repaint costs a few bytes to
//! everyone watching and not the board again.

use crate::AppState;
use crate::db::DbError;
use crate::live::Topic;
use crate::session::{Caller, MaybeCaller};
use crate::store::{NewDocument, WriteError};
use crate::xrpc::{conflict, err, invalid, member_of, owner_of, write_failed};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::BTreeSet;
use std::time::Duration;
use turso::Value;

pub const CANVAS_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS canvas (
  id       TEXT PRIMARY KEY REFERENCES document(id),
  width    INTEGER NOT NULL,
  height   INTEGER NOT NULL,
  cooldown INTEGER NOT NULL,                             -- seconds between one person's placements
  open     INTEGER NOT NULL DEFAULT 1
);
-- A repaint takes the cell over: the colour, the painter and the time are all
-- the last one's. Nothing deletes a cell, so a board cannot be quietly erased.
CREATE TABLE IF NOT EXISTS canvas_cell (
  canvas_id  TEXT NOT NULL REFERENCES canvas(id),
  x          INTEGER NOT NULL,
  y          INTEGER NOT NULL,
  colour     INTEGER NOT NULL,                           -- an index into the client's palette
  painter    TEXT REFERENCES user(did),
  painted_at TEXT NOT NULL,
  PRIMARY KEY (canvas_id, x, y)
);
CREATE INDEX IF NOT EXISTS canvas_cell_by_time ON canvas_cell(canvas_id, painted_at);
-- When each person last painted, which a cell cannot say once it is painted over.
CREATE TABLE IF NOT EXISTS canvas_painter (
  canvas_id  TEXT NOT NULL REFERENCES canvas(id),
  did        TEXT NOT NULL REFERENCES user(did),
  painted_at INTEGER NOT NULL,                           -- unix milliseconds
  PRIMARY KEY (canvas_id, did)
);
"#;

/// A cap, not a recommendation: a mistyped number must not ask for a million
/// cells. The interim's figure.
pub const MAX_SIDE: i64 = 128;
const DEFAULT_SIDE: i64 = 32;
const DEFAULT_COOLDOWN: i64 = 60;
const MAX_COOLDOWN: i64 = 24 * 60 * 60;

/// How long a painted cell waits to be announced, so a room painting at once is
/// told a few times a second and not once a cell.
const BEAT: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_millis(250)
};

/// What this module keeps between requests.
#[derive(Default)]
pub struct Shared {
    /// Canvases with an announcement on its way.
    announcing: std::sync::Mutex<BTreeSet<String>>,
}

fn now_ms() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_millis()).unwrap_or(i64::MAX))
}

struct Canvas {
    context_id: String,
    width: i64,
    height: i64,
    cooldown: i64,
    open: bool,
}

/// A canvas whose place in the tree is live and which `did` may read.
async fn load(state: &AppState, id: &str, did: Option<&str>) -> Result<Option<Canvas>, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            &format!(
                "SELECT d.context_id, c.width, c.height, c.cooldown, c.open \
                 FROM canvas c JOIN document d ON d.id = c.id \
                 WHERE c.id = ?1 AND d.deleted_at IS NULL AND {}",
                crate::authz::readable_document("d", 2)
            ),
            vec![
                Value::Text(id.to_string()),
                did.map_or(Value::Null, |did| Value::Text(did.to_string())),
            ],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(Canvas {
        context_id: row.get(0)?,
        width: row.get(1)?,
        height: row.get(2)?,
        cooldown: row.get(3)?,
        open: row.get::<i64>(4)? != 0,
    }))
}

fn no_such_canvas() -> Response {
    err(StatusCode::NOT_FOUND, "NotFound", "no such canvas")
}

fn announce(state: &AppState, context_id: &str, canvas_id: &str) {
    let first = state
        .canvases
        .announcing
        .lock()
        .expect("announcing")
        .insert(canvas_id.to_string());
    if !first {
        return;
    }
    let (state, context_id, canvas_id) =
        (state.clone(), context_id.to_string(), canvas_id.to_string());
    tokio::spawn(async move {
        tokio::time::sleep(BEAT).await;
        // Cleared before it is sent, so a cell painted in between starts the
        // next beat rather than being announced by nobody.
        state
            .canvases
            .announcing
            .lock()
            .expect("announcing")
            .remove(&canvas_id);
        state.publish(Topic::Context(context_id), "canvas", &canvas_id);
    });
}

#[derive(Debug, Deserialize)]
pub struct CreateCanvasBody {
    pub parent_id: String,
    pub name: String,
    #[serde(default)]
    pub width: Option<i64>,
    #[serde(default)]
    pub height: Option<i64>,
    /// Seconds between one person's placements.
    #[serde(default)]
    pub cooldown: Option<i64>,
}

/// `com.example.wiki.createCanvas` (procedure): an owner puts a canvas in a
/// context, or a folder in one. It starts open.
pub async fn create_canvas(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CreateCanvasBody>,
) -> Response {
    let what = "createCanvas";
    let store = crate::Store::new(state.db.clone());
    let parent = match store.parent_of(&body.parent_id).await {
        Ok(Some(parent)) => parent,
        Ok(None) => return invalid("no such parent"),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = owner_of(&state, &parent.context_id, &did, what).await {
        return refusal;
    }
    if !crate::authz::PLACES.contains(&parent.kind.as_str()) {
        return invalid("a canvas sits in a context, or in a folder");
    }
    let name = body.name.trim();
    if name.is_empty() || name.chars().count() > 200 {
        return invalid("a canvas needs a name, of at most 200 characters");
    }
    let side = |asked: Option<i64>| asked.unwrap_or(DEFAULT_SIDE).clamp(1, MAX_SIDE);
    let cooldown = body
        .cooldown
        .unwrap_or(DEFAULT_COOLDOWN)
        .clamp(0, MAX_COOLDOWN);

    let id = format!("d-{}", crate::util::random_token(16));
    let created = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let written: Result<(), WriteError> = async {
            let new = NewDocument {
                context_id: &parent.context_id,
                parent_id: Some(&body.parent_id),
                kind: "canvas",
                title: name,
                content: None,
                data: None,
                author_did: &did,
                credited: false,
            };
            store
                .insert_document(&conn, &id, &body.parent_id, &new)
                .await?;
            conn.execute(
                "INSERT INTO canvas (id, width, height, cooldown) VALUES (?1, ?2, ?3, ?4)",
                vec![
                    Value::Text(id.clone()),
                    Value::Integer(side(body.width)),
                    Value::Integer(side(body.height)),
                    Value::Integer(cooldown),
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
        written
    };
    match created.await {
        Ok(()) => {
            state.publish(Topic::Context(parent.context_id.clone()), "node", &id);
            (StatusCode::OK, Json(serde_json::json!({ "id": id }))).into_response()
        }
        Err(WriteError::Db(e)) => write_failed(what, e),
        Err(refused) => invalid(&refused.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct GetCanvasParams {
    pub id: String,
    /// Only the cells painted at or after this, as `now` of an earlier answer
    /// gave it. "At": a cell painted in that same millisecond may come twice,
    /// which repaints it the same colour, and must never come not at all.
    #[serde(default)]
    pub since: Option<String>,
}

/// `com.example.wiki.getCanvas`: a canvas and its painted cells, or with `since`
/// only those painted after it. Cells are rows of `[x, y, colour, painter]`,
/// where `painter` indexes `painters`: a thousand cells are a handful of people.
pub async fn get_canvas(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<GetCanvasParams>,
) -> Response {
    let what = "getCanvas";
    let canvas = match load(&state, &p.id, caller.did()).await {
        Ok(Some(canvas)) => canvas,
        Ok(None) => return no_such_canvas(),
        Err(e) => return write_failed(what, e),
    };
    let read = async {
        let conn = state.db.acquire().await?;
        // Taken BEFORE the cells are read: a cell painted while they are being
        // read is then in this answer or the next, and never in neither.
        let mut rows = conn
            .query("SELECT strftime('%Y-%m-%dT%H:%M:%fZ','now')", ())
            .await?;
        let now: String = match rows.next().await? {
            Some(row) => row.get(0)?,
            None => String::new(),
        };
        let mut rows = conn
            .query(
                "SELECT x, y, colour, painter, painted_at FROM canvas_cell \
                 WHERE canvas_id = ?1 AND (?2 IS NULL OR painted_at >= ?2) ORDER BY painted_at",
                vec![
                    Value::Text(p.id.clone()),
                    p.since.clone().map_or(Value::Null, Value::Text),
                ],
            )
            .await?;
        let mut painters: Vec<String> = Vec::new();
        let mut cells = Vec::new();
        while let Some(row) = rows.next().await? {
            let painter = match row.get_value(3)? {
                Value::Text(did) => Some(match painters.iter().position(|p| *p == did) {
                    Some(at) => at,
                    None => {
                        painters.push(did);
                        painters.len() - 1
                    }
                }),
                _ => None,
            };
            cells.push(serde_json::json!([
                row.get::<i64>(0)?,
                row.get::<i64>(1)?,
                row.get::<i64>(2)?,
                painter,
                row.get::<String>(4)?,
            ]));
        }
        let next_paint_at = match caller.did() {
            Some(did) => {
                let mut rows = conn
                    .query(
                        "SELECT painted_at FROM canvas_painter WHERE canvas_id = ?1 AND did = ?2",
                        [p.id.as_str(), did],
                    )
                    .await?;
                match rows.next().await? {
                    Some(row) => Some(row.get::<i64>(0)? + canvas.cooldown * 1000),
                    None => None,
                }
            }
            None => None,
        };
        let dids: BTreeSet<String> = painters.iter().cloned().collect();
        let profiles = crate::Store::new(state.db.clone()).profiles(&dids).await?;
        Ok::<_, DbError>(serde_json::json!({
            "id": p.id, "context_id": canvas.context_id,
            "width": canvas.width, "height": canvas.height,
            "cooldown": canvas.cooldown, "open": canvas.open,
            "cells": cells, "painters": painters, "profiles": profiles,
            "now": now, "now_ms": now_ms(), "next_paint_at": next_paint_at,
        }))
    };
    match read.await {
        Ok(view) => (StatusCode::OK, Json(view)).into_response(),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PaintBody {
    pub canvas: String,
    pub x: i64,
    pub y: i64,
    pub colour: i64,
}

enum Painted {
    Done { next_paint_at: i64 },
    TooSoon { retry_after_ms: i64 },
    Closed,
}

/// `com.example.wiki.paintCell` (procedure): a member of the canvas's context
/// paints one cell, and then waits out the cooldown. The cell becomes theirs.
pub async fn paint_cell(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<PaintBody>,
) -> Response {
    let what = "paintCell";
    let canvas = match load(&state, &body.canvas, Some(&did)).await {
        Ok(Some(canvas)) => canvas,
        Ok(None) => return no_such_canvas(),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = member_of(&state, &canvas.context_id, &did, what).await {
        return refusal;
    }
    let inside = (0..canvas.width).contains(&body.x) && (0..canvas.height).contains(&body.y);
    if !inside || !(0..=255).contains(&body.colour) {
        return invalid("that cell, or that colour, is not on this canvas");
    }
    let painted = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let outcome: Result<Painted, DbError> = async {
            // Both inside the transaction, which nothing can overlap: a canvas
            // closed, or a second tap, a moment ago is seen here.
            let mut rows = conn
                .query(
                    "SELECT open FROM canvas WHERE id = ?1",
                    [body.canvas.as_str()],
                )
                .await?;
            let open = matches!(rows.next().await?, Some(row) if row.get::<i64>(0)? != 0);
            drop(rows);
            if !open {
                return Ok(Painted::Closed);
            }
            let now = now_ms();
            let mut rows = conn
                .query(
                    "SELECT painted_at FROM canvas_painter WHERE canvas_id = ?1 AND did = ?2",
                    [body.canvas.as_str(), did.as_str()],
                )
                .await?;
            let last = match rows.next().await? {
                Some(row) => Some(row.get::<i64>(0)?),
                None => None,
            };
            drop(rows);
            let free_at = last.map_or(0, |last| last + canvas.cooldown * 1000);
            if now < free_at {
                return Ok(Painted::TooSoon {
                    retry_after_ms: free_at - now,
                });
            }
            conn.execute(
                "INSERT INTO canvas_cell (canvas_id, x, y, colour, painter, painted_at) \
                 VALUES (?1, ?2, ?3, ?4, ?5, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
                 ON CONFLICT(canvas_id, x, y) DO UPDATE SET colour = excluded.colour, \
                   painter = excluded.painter, painted_at = excluded.painted_at",
                vec![
                    Value::Text(body.canvas.clone()),
                    Value::Integer(body.x),
                    Value::Integer(body.y),
                    Value::Integer(body.colour),
                    Value::Text(did.clone()),
                ],
            )
            .await?;
            conn.execute(
                "INSERT INTO canvas_painter (canvas_id, did, painted_at) VALUES (?1, ?2, ?3) \
                 ON CONFLICT(canvas_id, did) DO UPDATE SET painted_at = excluded.painted_at",
                vec![
                    Value::Text(body.canvas.clone()),
                    Value::Text(did.clone()),
                    Value::Integer(now),
                ],
            )
            .await?;
            Ok(Painted::Done {
                next_paint_at: now + canvas.cooldown * 1000,
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
    match painted.await {
        Ok(Painted::Done { next_paint_at }) => {
            announce(&state, &canvas.context_id, &body.canvas);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "next_paint_at": next_paint_at })),
            )
                .into_response()
        }
        Ok(Painted::TooSoon { retry_after_ms }) => (
            StatusCode::TOO_MANY_REQUESTS,
            Json(serde_json::json!({
                "error": "TooSoon", "message": "wait out the cooldown",
                "retry_after_ms": retry_after_ms,
            })),
        )
            .into_response(),
        Ok(Painted::Closed) => conflict("CanvasClosed", "the canvas takes no more paint"),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct SetOpenBody {
    pub id: String,
    pub open: bool,
}

/// `com.example.wiki.setCanvasOpen` (procedure): an owner opens a canvas to the
/// room, or locks it so that it takes no more paint.
pub async fn set_canvas_open(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<SetOpenBody>,
) -> Response {
    let what = "setCanvasOpen";
    let canvas = match load(&state, &body.id, Some(&did)).await {
        Ok(Some(canvas)) => canvas,
        Ok(None) => return no_such_canvas(),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = owner_of(&state, &canvas.context_id, &did, what).await {
        return refusal;
    }
    let set = async {
        let conn = state.db.acquire().await?;
        conn.execute(
            "UPDATE canvas SET open = ?2 WHERE id = ?1",
            vec![
                Value::Text(body.id.clone()),
                Value::Integer(i64::from(body.open)),
            ],
        )
        .await?;
        Ok::<_, DbError>(())
    };
    match set.await {
        Ok(()) => {
            state.publish(Topic::Context(canvas.context_id), "canvas", &body.id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "open": body.open })),
            )
                .into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{get, get_as, post, seeded_state, token_for};
    use serde_json::json;

    const PAINT: &str = "/xrpc/com.example.wiki.paintCell";

    async fn canvas(state: &AppState, owner: &str, cooldown: i64) -> String {
        let body = json!({"parent_id": "c9", "name": "Tavlen", "width": 4, "height": 3, "cooldown": cooldown});
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createCanvas",
            Some(owner),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        v["id"].as_str().expect("id").to_string()
    }

    async fn paint(
        state: &AppState,
        who: &str,
        id: &str,
        at: (i64, i64),
        colour: i64,
    ) -> (StatusCode, serde_json::Value) {
        let body = json!({"canvas": id, "x": at.0, "y": at.1, "colour": colour});
        post(router(state.clone()), PAINT, Some(who), body).await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_room_paints_a_board_and_a_repaint_takes_the_cell_over() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let id = canvas(&state, &alice, 0).await;
        let board = format!("/xrpc/com.example.wiki.getCanvas?id={id}");

        assert_eq!(paint(&state, &bob, &id, (1, 2), 5).await.0, StatusCode::OK);
        assert_eq!(
            paint(&state, &alice, &id, (0, 0), 7).await.0,
            StatusCode::OK
        );
        tokio::time::sleep(Duration::from_millis(5)).await;
        let (_, v) = get_as(router(state.clone()), &board, &bob).await;
        assert_eq!(
            (v["width"].as_i64(), v["height"].as_i64()),
            (Some(4), Some(3))
        );
        assert_eq!(v["cells"].as_array().expect("cells").len(), 2, "{v}");
        let since = v["now"].as_str().expect("now").to_string();
        tokio::time::sleep(Duration::from_millis(5)).await;

        // Alice paints over bob's cell: one cell, hers now.
        assert_eq!(
            paint(&state, &alice, &id, (1, 2), 9).await.0,
            StatusCode::OK
        );
        let (_, v) = get_as(
            router(state.clone()),
            &format!("{board}&since={since}"),
            &bob,
        )
        .await;
        let cells = v["cells"].as_array().expect("cells");
        assert_eq!(cells.len(), 1, "only what was painted since: {v}");
        assert_eq!(
            (
                cells[0][0].as_i64(),
                cells[0][1].as_i64(),
                cells[0][2].as_i64()
            ),
            (Some(1), Some(2), Some(9))
        );
        let painter = cells[0][3].as_u64().expect("painter") as usize;
        assert_eq!(v["painters"][painter], "did:plc:alice");
        let (_, whole) = get_as(router(state.clone()), &board, &bob).await;
        assert_eq!(
            whole["cells"].as_array().expect("cells").len(),
            2,
            "a repaint added a cell"
        );

        for (why, at, colour) in [
            ("off the board", (4, 0), 1),
            ("above it", (0, -1), 1),
            ("no such colour", (0, 0), 256),
        ] {
            assert_eq!(
                paint(&state, &bob, &id, at, colour).await.0,
                StatusCode::BAD_REQUEST,
                "{why}"
            );
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn one_placement_a_person_a_cooldown_whatever_the_client_does() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let id = canvas(&state, &alice, 60).await;

        let (status, first) = paint(&state, &bob, &id, (0, 0), 1).await;
        assert_eq!(status, StatusCode::OK);
        let (status, v) = paint(&state, &bob, &id, (1, 1), 2).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{v}");
        assert_eq!(v["error"], "TooSoon");
        let wait = v["retry_after_ms"].as_i64().expect("retry_after_ms");
        assert!((1..=60_000).contains(&wait), "{wait}");
        // Alice's clock is her own.
        assert_eq!(
            paint(&state, &alice, &id, (1, 1), 2).await.0,
            StatusCode::OK
        );

        let (_, seen) = get_as(
            router(state.clone()),
            &format!("/xrpc/com.example.wiki.getCanvas?id={id}"),
            &bob,
        )
        .await;
        assert_eq!(
            seen["next_paint_at"], first["next_paint_at"],
            "the countdown survives a reload"
        );
        assert_eq!(
            seen["cells"].as_array().expect("cells").len(),
            2,
            "a refused placement landed"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_canvas_is_its_groups_and_a_closed_one_takes_no_paint() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let mallory = token_for(&state, "did:plc:mallory").await;
        let id = canvas(&state, &alice, 0).await;
        let board = format!("/xrpc/com.example.wiki.getCanvas?id={id}");

        assert_eq!(
            get(router(state.clone()), &board).await.0,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            paint(&state, &mallory, &id, (0, 0), 1).await.0,
            StatusCode::NOT_FOUND
        );
        let make = json!({"parent_id": "c9", "name": "Bobs"});
        let (status, _) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createCanvas",
            Some(&bob),
            make,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a member made a canvas");

        let shut = "/xrpc/com.example.wiki.setCanvasOpen";
        let (status, _) = post(
            router(state.clone()),
            shut,
            Some(&bob),
            json!({"id": id, "open": false}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, _) = post(
            router(state.clone()),
            shut,
            Some(&alice),
            json!({"id": id, "open": false}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, v) = paint(&state, &bob, &id, (0, 0), 1).await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "CanvasClosed");

        // It has a place in the tree, like anything else.
        let (_, node) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getNode?path=closed/tavlen",
            &bob,
        )
        .await;
        assert_eq!(node["node"]["kind"], "canvas", "{node}");
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
    async fn a_room_painting_at_once_is_all_painted_and_told_a_few_times() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let id = canvas(&state, &alice, 0).await;
        let mut painters = Vec::new();
        for n in 0..12 {
            let did = format!("did:plc:painter{n}");
            painters.push(token_for(&state, &did).await);
            crate::xrpc::tests::join(&state, &did, "c9").await;
        }
        let mut changes = state.changes.subscribe();
        let strokes = painters.into_iter().enumerate().map(|(n, who)| {
            let (state, id) = (state.clone(), id.clone());
            tokio::spawn(async move {
                paint(&state, &who, &id, (n as i64 % 4, n as i64 / 4), 3)
                    .await
                    .0
            })
        });
        for stroke in strokes.collect::<Vec<_>>() {
            assert_eq!(stroke.await.expect("join"), StatusCode::OK);
        }
        tokio::time::sleep(BEAT * 4).await;
        let mut told = 0;
        while let Ok(change) = changes.try_recv() {
            told += usize::from(change.kind == "canvas");
        }
        assert!(
            (1..12).contains(&told),
            "{told} announcements for twelve cells"
        );
        let (_, v) = get_as(
            router(state.clone()),
            &format!("/xrpc/com.example.wiki.getCanvas?id={id}"),
            &alice,
        )
        .await;
        assert_eq!(v["cells"].as_array().expect("cells").len(), 12);
    }
}
