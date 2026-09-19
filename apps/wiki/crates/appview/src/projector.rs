//! What a context's projector shows: the node on screen, where in it to look,
//! and whether its comments and the feed are shown with it.
//!
//! The interim keeps these as rows of a generic `relations` table, one per
//! setting, with the focus anchor packed into a relation's NAME (`focus:<anchor>`)
//! because that table has no free-text column. Here it is one row per context.
//! Like the speaker lists it is coordination state: not content, not published,
//! not migrated.

use crate::AppState;
use crate::db::DbError;
use crate::live::Topic;
use crate::session::{Caller, MaybeCaller};
use crate::xrpc::{err, owner_of, write_failed};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Deserializer, Serialize};
use turso::Value;

pub const PROJECTOR_DDL: &str = r#"
-- active_id names a node of any kind (a document today, a poll later), so it is
-- not a foreign key.
CREATE TABLE IF NOT EXISTS projector (
  context_id    TEXT PRIMARY KEY REFERENCES context(id),
  active_id     TEXT,
  focus         TEXT,
  canvas_id     TEXT,                                      -- the board the context's canvas app shows
  show_comments INTEGER NOT NULL DEFAULT 0,
  show_feed     INTEGER NOT NULL DEFAULT 0,
  updated_at    TEXT NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ','now'))
);
"#;

const NOW: &str = "strftime('%Y-%m-%dT%H:%M:%fZ','now')";

#[derive(Debug, Clone, Default, PartialEq, Serialize)]
pub struct Projector {
    /// The node on screen. During a vote it is the open poll.
    pub active_id: Option<String>,
    /// A heading anchor within it, for a document too long to show whole.
    pub focus: Option<String>,
    /// The canvas the context shows where it shows one: the room's board. Its
    /// own setting, since a board is up beside whatever is on the screen.
    pub canvas_id: Option<String>,
    pub show_comments: bool,
    pub show_feed: bool,
}

/// Tell an absent field from an explicit `null`: absent leaves a setting as it
/// is, `null` clears it.
fn set_or_clear<'de, D: Deserializer<'de>>(de: D) -> Result<Option<Option<String>>, D::Error> {
    Option::<String>::deserialize(de).map(Some)
}

#[derive(Debug, Deserialize)]
pub struct SetProjectorBody {
    pub context_id: String,
    #[serde(default, deserialize_with = "set_or_clear")]
    pub active_id: Option<Option<String>>,
    #[serde(default, deserialize_with = "set_or_clear")]
    pub focus: Option<Option<String>>,
    #[serde(default, deserialize_with = "set_or_clear")]
    pub canvas_id: Option<Option<String>>,
    #[serde(default)]
    pub show_comments: Option<bool>,
    #[serde(default)]
    pub show_feed: Option<bool>,
}

async fn read(state: &AppState, context_id: &str) -> Result<Projector, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            "SELECT active_id, focus, show_comments, show_feed, canvas_id FROM projector \
             WHERE context_id = ?1",
            [context_id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(Projector::default());
    };
    let text = |i: usize| match row.get_value(i) {
        Ok(Value::Text(s)) => Some(s),
        _ => None,
    };
    Ok(Projector {
        active_id: text(0),
        focus: text(1),
        canvas_id: text(4),
        show_comments: row.get::<i64>(2)? != 0,
        show_feed: row.get::<i64>(3)? != 0,
    })
}

async fn write(state: &AppState, context_id: &str, p: &Projector) -> Result<(), DbError> {
    let text = |s: &Option<String>| match s {
        Some(s) => Value::Text(s.clone()),
        None => Value::Null,
    };
    let conn = state.db.acquire().await?;
    conn.execute(
        &format!(
            "INSERT INTO projector \
               (context_id, active_id, focus, show_comments, show_feed, canvas_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(context_id) DO UPDATE SET active_id = excluded.active_id, \
               focus = excluded.focus, show_comments = excluded.show_comments, \
               show_feed = excluded.show_feed, canvas_id = excluded.canvas_id, \
               updated_at = {NOW}"
        ),
        vec![
            Value::Text(context_id.to_string()),
            text(&p.active_id),
            text(&p.focus),
            Value::Integer(i64::from(p.show_comments)),
            Value::Integer(i64::from(p.show_feed)),
            text(&p.canvas_id),
        ],
    )
    .await?;
    Ok(())
}

#[derive(Debug, Deserialize)]
pub struct ContextParam {
    pub context: String,
}

/// `com.example.wiki.getProjector`: what a context's projector shows, for
/// whoever may read the context. A context nobody has set one for shows nothing.
pub async fn get_projector(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<ContextParam>,
) -> Response {
    let what = "getProjector";
    match crate::Store::new(state.db.clone())
        .read_context(&p.context, caller.did())
        .await
    {
        Ok(Some(_)) => {}
        Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such context"),
        Err(e) => return write_failed(what, e),
    }
    match read(&state, &p.context).await {
        Ok(projector) => (StatusCode::OK, Json(projector)).into_response(),
        Err(e) => write_failed(what, e),
    }
}

/// `com.example.wiki.setProjector` (procedure): an owner changes what the room
/// sees. Absent fields stay as they are; `null` clears one.
pub async fn set_projector(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<SetProjectorBody>,
) -> Response {
    let what = "setProjector";
    if let Err(refusal) = owner_of(&state, &body.context_id, &did, what).await {
        return refusal;
    }
    let mut projector = match read(&state, &body.context_id).await {
        Ok(projector) => projector,
        Err(e) => return write_failed(what, e),
    };
    if let Some(active_id) = body.active_id {
        // An anchor points into the node that was on screen, and means nothing
        // in the next one.
        if active_id != projector.active_id {
            projector.focus = None;
        }
        projector.active_id = active_id;
    }
    if let Some(focus) = body.focus {
        projector.focus = focus;
    }
    if let Some(canvas_id) = body.canvas_id {
        projector.canvas_id = canvas_id;
    }
    if let Some(show) = body.show_comments {
        projector.show_comments = show;
    }
    if let Some(show) = body.show_feed {
        projector.show_feed = show;
    }
    match write(&state, &body.context_id, &projector).await {
        Ok(()) => {
            state.publish(
                Topic::Context(body.context_id.clone()),
                "screen",
                &body.context_id,
            );
            (StatusCode::OK, Json(projector)).into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

#[cfg(test)]
mod tests {
    use crate::router;
    use crate::xrpc::tests::{get, get_as, post, seeded_state, token_for};
    use axum::http::StatusCode;

    const SCREEN: &str = "/xrpc/com.example.wiki.getProjector?context=c9";

    async fn set(
        state: &crate::AppState,
        who: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        post(
            router(state.clone()),
            "/xrpc/com.example.wiki.setProjector",
            Some(who),
            body,
        )
        .await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_chair_decides_what_the_room_sees() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;

        let (_, blank) = get_as(router(state.clone()), SCREEN, &bob).await;
        assert_eq!(
            blank,
            serde_json::json!({
                "active_id": null, "focus": null, "canvas_id": null,
                "show_comments": false, "show_feed": false
            }),
            "a projector nobody has set shows nothing"
        );

        // The room's board is a setting of its own: up beside what is on screen.
        let board = serde_json::json!({"context_id": "c9", "canvas_id": "cv"});
        let (_, v) = set(&state, &alice, board).await;
        assert_eq!(
            (&v["canvas_id"], &v["active_id"]),
            (&serde_json::json!("cv"), &serde_json::Value::Null)
        );
        let no_board = serde_json::json!({"context_id": "c9", "canvas_id": null});
        let (_, v) = set(&state, &alice, no_board).await;
        assert!(v["canvas_id"].is_null(), "{v}");

        let show =
            serde_json::json!({"context_id": "c9", "active_id": "s1", "show_comments": true});
        assert_eq!(
            set(&state, &bob, show.clone()).await.0,
            StatusCode::FORBIDDEN
        );
        assert_eq!(set(&state, &alice, show).await.0, StatusCode::OK);
        let (_, v) = get_as(router(state.clone()), SCREEN, &bob).await;
        assert_eq!(v["active_id"], "s1");
        assert_eq!(v["show_comments"], true);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_absent_field_stays_and_a_null_one_clears() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let c9 = |rest: serde_json::Value| {
            let mut body = serde_json::json!({"context_id": "c9"});
            body.as_object_mut()
                .expect("object")
                .extend(rest.as_object().expect("object").clone());
            body
        };
        set(
            &state,
            &alice,
            c9(serde_json::json!({"active_id": "s1", "focus": "stk-3"})),
        )
        .await;
        let (_, v) = set(&state, &alice, c9(serde_json::json!({"show_feed": true}))).await;
        assert_eq!(v["active_id"], "s1", "an absent field was cleared");
        assert_eq!(v["focus"], "stk-3");

        let (_, v) = set(&state, &alice, c9(serde_json::json!({"focus": null}))).await;
        assert!(v["focus"].is_null());
        assert_eq!(v["active_id"], "s1");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_anchor_does_not_follow_the_screen_to_another_node() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let first = serde_json::json!({"context_id": "c9", "active_id": "s1", "focus": "stk-3"});
        set(&state, &alice, first).await;
        let next = serde_json::json!({"context_id": "c9", "active_id": "another"});
        let (_, v) = set(&state, &alice, next).await;
        assert!(
            v["focus"].is_null(),
            "the next document was scrolled to a heading of the last one"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_closed_groups_screen_is_not_shown_outside_it() {
        let state = seeded_state().await;
        assert_eq!(
            get(router(state.clone()), SCREEN).await.0,
            StatusCode::NOT_FOUND
        );
        let mallory = token_for(&state, "did:plc:mallory").await;
        assert_eq!(
            get_as(router(state.clone()), SCREEN, &mallory).await.0,
            StatusCode::NOT_FOUND
        );
        let (status, _) = set(
            &state,
            &mallory,
            serde_json::json!({"context_id": "c9", "active_id": "s1"}),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }
}
