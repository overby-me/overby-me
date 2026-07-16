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
             INSERT INTO document (id, context_id, kind, title, content, legacy_id) \
               VALUES ('d1', 'c1', 'policy', 'Motion', '{\"blocks\":[{\"text\":\"hi\"}]}', NULL);
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', 'did:plc:alice', NULL, 0);
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', NULL, 'Guest', 1);",
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
}
