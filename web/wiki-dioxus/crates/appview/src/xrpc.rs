//! The native XRPC serving layer (read side). Methods live at `/xrpc/{nsid}`
//! following the atproto convention (queries are GET with query-string params)
//! and return the canonical domain types as JSON, so there is no premature
//! frontend-shape decision: the AppView serves its real, reconciled entities
//! (`document`, `context`, ...) and the frontend seam that consumes them is a
//! separate, deferred change (nothing here touches the frontend).
//!
//! These are the IDENTITY-FREE reads (public content lookups, no auth). The
//! membership/authz-gated reads and the write procedures wait on the DID-binding
//! flow (see `store.rs`); this is the buildable-now slice of the serving layer.

use crate::AppState;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

/// The `?id=` query param shared by the by-id lookups.
#[derive(Debug, Deserialize)]
pub struct IdParam {
    pub id: String,
}

/// An XRPC error body (`{ "error": ..., "message": ... }`, the atproto shape).
fn err(status: StatusCode, error: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": error, "message": message })),
    )
        .into_response()
}

/// `com.example.wiki.getDocument` — a content node (document/folder/file/
/// proposal) by id, with its authors. Identity-free.
pub async fn get_document(State(state): State<AppState>, Query(p): Query<IdParam>) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.read_document(&p.id).await {
        Ok(Some(doc)) => (StatusCode::OK, Json(doc)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "NotFound", "no such document"),
        Err(e) => {
            tracing::error!("getDocument failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `com.example.wiki.getContext` — a group/event context by id. Identity-free.
pub async fn get_context(State(state): State<AppState>, Query(p): Query<IdParam>) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.read_context(&p.id).await {
        Ok(Some(ctx)) => (StatusCode::OK, Json(ctx)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "NotFound", "no such context"),
        Err(e) => {
            tracing::error!("getContext failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?path=a/b/c` — a slash-separated context slug path.
#[derive(Debug, Deserialize)]
pub struct PathParam {
    pub path: String,
}

/// `com.example.wiki.resolveNode` — resolve a slug path to a context.
pub async fn resolve_node(State(state): State<AppState>, Query(p): Query<PathParam>) -> Response {
    let slugs: Vec<String> = p
        .path
        .split('/')
        .filter(|s| !s.is_empty())
        .map(str::to_string)
        .collect();
    let store = crate::Store::new(state.db.clone());
    match store.resolve_context(&slugs).await {
        Ok(Some(ctx)) => (StatusCode::OK, Json(ctx)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "NotFound", "no node at that path"),
        Err(e) => {
            tracing::error!("resolveNode failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?parent=<id>`.
#[derive(Debug, Deserialize)]
pub struct ParentParam {
    pub parent: String,
}

/// `com.example.wiki.listChildren` — the child documents under a node.
pub async fn list_children(
    State(state): State<AppState>,
    Query(p): Query<ParentParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.list_children(&p.parent).await {
        Ok(docs) => (StatusCode::OK, Json(docs)).into_response(),
        Err(e) => {
            tracing::error!("listChildren failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `com.example.wiki.listContexts` — the top-level groups/events.
pub async fn list_contexts(State(state): State<AppState>) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.list_root_contexts().await {
        Ok(ctxs) => (StatusCode::OK, Json(ctxs)).into_response(),
        Err(e) => {
            tracing::error!("listContexts failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?limit=<n>` (default 20).
#[derive(Debug, Deserialize)]
pub struct RecentParam {
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `com.example.wiki.listRecent` — the newest documents across contexts.
pub async fn list_recent(State(state): State<AppState>, Query(p): Query<RecentParam>) -> Response {
    let limit = p.limit.unwrap_or(20).clamp(1, 200);
    let store = crate::Store::new(state.db.clone());
    match store.list_recent(limit).await {
        Ok(docs) => (StatusCode::OK, Json(docs)).into_response(),
        Err(e) => {
            tracing::error!("listRecent failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?q=<query>`.
#[derive(Debug, Deserialize)]
pub struct SearchParam {
    pub q: String,
}

/// `com.example.wiki.search` — documents matching a title/content substring.
pub async fn search(State(state): State<AppState>, Query(p): Query<SearchParam>) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.search_documents(&p.q).await {
        Ok(docs) => (StatusCode::OK, Json(docs)).into_response(),
        Err(e) => {
            tracing::error!("search failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?on=<id>`.
#[derive(Debug, Deserialize)]
pub struct OnParam {
    pub on: String,
}

/// `com.example.wiki.getComments` — the comment thread on a node.
pub async fn get_comments(State(state): State<AppState>, Query(p): Query<OnParam>) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.get_comments(&p.on).await {
        Ok(comments) => (StatusCode::OK, Json(comments)).into_response(),
        Err(e) => {
            tracing::error!("getComments failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?subject=<at-uri>`.
#[derive(Debug, Deserialize)]
pub struct SubjectParam {
    pub subject: String,
}

/// `com.example.wiki.getReactions` — the reactions on a subject (by at-uri).
pub async fn get_reactions(
    State(state): State<AppState>,
    Query(p): Query<SubjectParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.get_reactions(&p.subject).await {
        Ok(reactions) => (StatusCode::OK, Json(reactions)).into_response(),
        Err(e) => {
            tracing::error!("getReactions failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::{AppState, Config, Db, router};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    async fn seeded_router() -> axum::Router {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let conn = db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO user (did, handle, display_name, legacy_id) \
               VALUES ('did:plc:alice', 'alice.test', 'Alice', NULL);
             INSERT INTO context (id, kind, name, slug, legacy_id) \
               VALUES ('c1', 'group', 'Group One', 'group-one', NULL);
             INSERT INTO context (id, kind, name, slug, parent_id, legacy_id) \
               VALUES ('c2', 'event', 'Sub Event', 'sub', 'c1', NULL);
             INSERT INTO document (id, context_id, kind, title, content, legacy_id) \
               VALUES ('d1', 'c1', 'policy', 'Motion', '{\"blocks\":[{\"text\":\"hi\"}]}', NULL);
             INSERT INTO document (id, context_id, parent_id, kind, title, legacy_id) \
               VALUES ('d2', 'c1', 'c1', 'document', 'Child Doc', NULL);
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', 'did:plc:alice', NULL, 0);
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', NULL, 'Guest', 1);
             INSERT INTO comment (id, on_id, context_id, author_did, text, legacy_id) \
               VALUES ('k1', 'd1', 'c1', 'did:plc:alice', 'Nice motion', NULL);
             INSERT INTO reaction (id, subject_uri, reactor_did, emoji, legacy_id) \
               VALUES ('at://did:plc:bob/com.example.wiki.reaction/r1', \
                       'at://did:plc:alice/com.example.wiki.post/p1', 'did:plc:bob', '👍', NULL);",
        )
        .await
        .expect("seed");
        router(AppState::new(db, Config::default()))
    }

    async fn get(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .expect("request");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("body");
        let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_document_returns_the_document_with_authors() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getDocument?id=d1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["title"], "Motion");
        assert_eq!(v["kind"], "policy");
        assert_eq!(v["context_id"], "c1");
        // Authorship survives: one DID account + one free-text author, in order.
        let authors = v["authors"].as_array().expect("authors array");
        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0]["kind"], "user");
        assert_eq!(authors[0]["did"], "did:plc:alice");
        assert_eq!(authors[1]["kind"], "free_text");
        assert_eq!(authors[1]["display"], "Guest");
        // The Slate JSON round-trips through the TEXT column.
        assert_eq!(v["content"]["blocks"][0]["text"], "hi");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_context_returns_the_context() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getContext?id=c1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["name"], "Group One");
        assert_eq!(v["kind"], "group");
        assert_eq!(v["slug"], "group-one");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_id_is_a_404_xrpc_error() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getDocument?id=nope",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "NotFound");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_node_walks_the_slug_path() {
        let app = seeded_router().await;
        let (status, v) = get(app, "/xrpc/com.example.wiki.resolveNode?path=group-one/sub").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["slug"], "sub");
        assert_eq!(v["name"], "Sub Event");
        // A broken path is a 404.
        let (status, _) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.resolveNode?path=group-one/nope",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_children_search_recent_return_documents() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.listChildren?parent=c1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let docs = v.as_array().expect("array");
        assert!(docs.iter().any(|d| d["id"] == "d2"));

        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.search?q=Motion",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(v.as_array().unwrap().iter().any(|d| d["id"] == "d1"));

        let (status, v) = get(seeded_router().await, "/xrpc/com.example.wiki.listRecent").await;
        assert_eq!(status, StatusCode::OK);
        assert!(v.as_array().unwrap().len() >= 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_contexts_returns_only_roots() {
        let (status, v) = get(seeded_router().await, "/xrpc/com.example.wiki.listContexts").await;
        assert_eq!(status, StatusCode::OK);
        let ctxs = v.as_array().expect("array");
        // c1 is a root; c2 has a parent and is excluded.
        assert!(ctxs.iter().any(|c| c["id"] == "c1"));
        assert!(!ctxs.iter().any(|c| c["id"] == "c2"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_comments_and_reactions() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getComments?on=d1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let comments = v.as_array().expect("array");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0]["text"], "Nice motion");
        assert_eq!(comments[0]["author"]["did"], "did:plc:alice");

        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getReactions?subject=at://did:plc:alice/com.example.wiki.post/p1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let reactions = v.as_array().expect("array");
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0]["emoji"], "👍");
        assert_eq!(reactions[0]["reactor_did"], "did:plc:bob");
    }
}
