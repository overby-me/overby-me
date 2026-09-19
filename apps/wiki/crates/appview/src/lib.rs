//! The atproto AppView service (the stateful process the interim NHost/Hasura
//! backend evolves into). This is the kickoff skeleton: config, the Turso pool
//! with the FK-pragma helper, the in-process broadcast channel, and one `/ws`
//! plus `/healthz`. Every transferable module (util, statecookie, push, the
//! Store seam, XRPC handlers, the firehose consumer, the OAuth callback) lands
//! into this crate from here.
//!
//! It is a single stateful always-on process (Turso core+view, a firehose
//! connection, the broadcast channel, the WebSocket server), so it CANNOT run
//! on scale-to-zero serverless like the interim backend; see the deploy item.

pub mod ballot;
pub mod config;
pub mod db;
pub mod firehose;
pub mod http;
pub mod oauth;
pub mod schema;
pub mod session;
pub mod statecookie;
pub mod store;
pub mod util;
pub mod ws;
pub mod xrpc;

pub use config::Config;
pub use db::{Db, DbError};
pub use store::Store;

use axum::extract::State;
use axum::http::header::{AUTHORIZATION, CONTENT_TYPE};
use axum::http::{HeaderValue, Method};
use axum::routing::{get, post};
use axum::{Json, Router};
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::time::Duration;
use tokio::sync::broadcast;
use tower_http::cors::CorsLayer;

/// Shared application state handed to every handler.
#[derive(Clone)]
pub struct AppState {
    pub db: Db,
    /// Authoritative deltas broadcast to all connected clients over `/ws`.
    pub deltas: broadcast::Sender<String>,
    pub config: Config,
    /// The atproto OAuth client, present when the AppView is built with identity
    /// wired (the default in tests is `None`, so `/callback` reports 503).
    pub oauth: Option<Arc<oauth::WikiOAuth>>,
    /// Live firehose status (updated by the consumer task, read by `/healthz`).
    pub firehose: Arc<firehose::FirehoseStatus>,
}

impl AppState {
    /// Build state around an open database, with a fresh broadcast channel and
    /// no OAuth client (see [`AppState::with_oauth`]).
    pub fn new(db: Db, config: Config) -> Self {
        let (deltas, _rx) = broadcast::channel::<String>(1024);
        Self {
            db,
            deltas,
            config,
            oauth: None,
            firehose: Arc::new(firehose::FirehoseStatus::default()),
        }
    }

    /// Attach the atproto OAuth client (enables the `/callback` slice).
    pub fn with_oauth(mut self, oauth: Arc<oauth::WikiOAuth>) -> Self {
        self.oauth = Some(oauth);
        self
    }
}

/// CORS for the configured frontend origins, or none, which keeps the API
/// same-origin. No credentials mode: a session travels in a header the page
/// sets, never in a cookie.
fn cors(config: &Config) -> Option<CorsLayer> {
    let origins: Vec<HeaderValue> = config
        .frontend_origins
        .iter()
        .filter_map(|origin| origin.parse().ok())
        .collect();
    if origins.is_empty() {
        return None;
    }
    Some(
        CorsLayer::new()
            .allow_origin(origins)
            .allow_methods([Method::GET, Method::POST])
            .allow_headers([AUTHORIZATION, CONTENT_TYPE])
            .max_age(Duration::from_secs(3600)),
    )
}

/// The AppView router: liveness, the multiplexed client WebSocket, the atproto
/// OAuth login, and the XRPC surface.
pub fn router(state: AppState) -> Router {
    let cors = cors(&state.config);
    let router = build_router(state);
    match cors {
        Some(layer) => router.layer(layer),
        None => router,
    }
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(ws::ws_handler))
        .route("/login", get(oauth::login_handler))
        .route("/callback", get(oauth::callback_handler))
        .route(
            oauth::CLIENT_METADATA_PATH,
            get(oauth::client_metadata_handler),
        )
        // The native XRPC read surface (identity-free content lookups).
        .route(
            "/xrpc/com.example.wiki.getDocument",
            get(xrpc::get_document),
        )
        .route("/xrpc/com.example.wiki.getContext", get(xrpc::get_context))
        .route(
            "/xrpc/com.example.wiki.resolveNode",
            get(xrpc::resolve_node),
        )
        .route(
            "/xrpc/com.example.wiki.listChildren",
            get(xrpc::list_children),
        )
        .route(
            "/xrpc/com.example.wiki.listContexts",
            get(xrpc::list_contexts),
        )
        .route("/xrpc/com.example.wiki.listRecent", get(xrpc::list_recent))
        .route("/xrpc/com.example.wiki.search", get(xrpc::search))
        .route(
            "/xrpc/com.example.wiki.getComments",
            get(xrpc::get_comments),
        )
        .route(
            "/xrpc/com.example.wiki.getReactions",
            get(xrpc::get_reactions),
        )
        .route(
            "/xrpc/com.example.wiki.createSession",
            post(xrpc::create_session),
        )
        .route("/xrpc/com.example.wiki.getSession", get(xrpc::get_session))
        .route(
            "/xrpc/com.example.wiki.deleteSession",
            post(xrpc::delete_session),
        )
        // The write procedures (the session's DID authors content).
        .route(
            "/xrpc/com.example.wiki.createDocument",
            post(xrpc::create_document),
        )
        .route(
            "/xrpc/com.example.wiki.postComment",
            post(xrpc::post_comment),
        )
        .route(
            "/xrpc/com.example.wiki.addReaction",
            post(xrpc::add_reaction),
        )
        .route(
            "/xrpc/com.example.wiki.removeReaction",
            post(xrpc::remove_reaction),
        )
        .with_state(state)
}

/// Liveness + readiness: confirms the database is reachable (a real connection
/// with the FK pragma verified) and reports firehose configuration. A stalled
/// firehose or wedged DB is otherwise invisible until users complain, so this
/// is the signal the deploy unit and any uptime check watch.
async fn healthz(State(state): State<AppState>) -> Json<serde_json::Value> {
    let db_ok = state.db.acquire().await.is_ok();
    Json(serde_json::json!({
        "ok": db_ok,
        "db": db_ok,
        "firehose_configured": !state.config.firehose_url.is_empty(),
        // Real liveness from the consumer task: connected + events seen (a stalled
        // firehose is connected-but-not-advancing).
        "firehose_connected": state.firehose.connected.load(Ordering::Relaxed),
        "firehose_events": state.firehose.events_seen.load(Ordering::Relaxed),
        "clients": state.deltas.receiver_count(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    #[tokio::test(flavor = "current_thread")]
    async fn healthz_reports_db_reachable() {
        let db = Db::open(":memory:").await.expect("open");
        let app = router(AppState::new(db, Config::default()));
        let resp = app
            .oneshot(
                Request::builder()
                    .uri("/healthz")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("body");
        let v: serde_json::Value = serde_json::from_slice(&bytes).expect("json");
        assert_eq!(v["ok"], true);
        assert_eq!(v["db"], true);
    }

    async fn preflight(app: Router, origin: &str) -> axum::http::HeaderMap {
        app.oneshot(
            Request::builder()
                .method("OPTIONS")
                .uri("/xrpc/com.example.wiki.createSession")
                .header("origin", origin)
                .header("access-control-request-method", "POST")
                .header(
                    "access-control-request-headers",
                    "authorization,content-type",
                )
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .expect("request")
        .headers()
        .clone()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn cors_answers_the_configured_frontend_and_nobody_else() {
        let db = Db::open(":memory:").await.expect("open");
        let config = Config {
            frontend_origins: vec!["https://wiki.example".to_string()],
            ..Config::default()
        };
        let app = router(AppState::new(db, config));

        let ours = preflight(app.clone(), "https://wiki.example").await;
        assert_eq!(
            ours.get("access-control-allow-origin")
                .and_then(|v| v.to_str().ok()),
            Some("https://wiki.example")
        );
        let allowed = ours
            .get("access-control-allow-headers")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(allowed.contains("authorization"), "{allowed}");
        assert!(
            ours.get("access-control-allow-credentials").is_none(),
            "sessions are header-borne; cookies must not be invited"
        );

        let theirs = preflight(app, "https://evil.example").await;
        assert!(
            theirs.get("access-control-allow-origin").is_none(),
            "a foreign origin was granted CORS"
        );
    }
}
