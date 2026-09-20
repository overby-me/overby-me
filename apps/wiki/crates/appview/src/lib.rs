//! The atproto AppView: the backend that replaces NHost, Hasura and the
//! serverless sidecar. `docs/appview-roadmap.md` says what it does today and
//! what is left.
//!
//! It is a single stateful always-on process (Turso core+view, a firehose
//! connection, the broadcast channel, the WebSocket server), so it CANNOT run
//! on scale-to-zero serverless like the interim backend.

pub mod authz;
pub mod ballot;
pub mod blob;
pub mod board;
pub mod canvas;
pub mod comment;
pub mod config;
pub mod context;
pub mod db;
pub mod delegation;
pub mod feed;
pub mod feedback;
pub mod firehose;
pub mod http;
pub mod import;
pub mod legacy;
pub mod live;
pub mod logs;
pub mod mail;
pub mod metafile;
pub mod metafile_svg;
pub mod oauth;
pub mod people;
pub mod poll;
pub mod profile;
pub mod projector;
pub mod push;
pub mod roster;
pub mod schema;
pub mod search;
pub mod session;
pub mod share;
pub mod slug;
pub mod spaces;
pub mod speak;
pub mod statecookie;
pub mod store;
pub mod symbolicate;
pub mod tree;
pub mod util;
pub mod verify;
pub mod xrpc;

pub use config::Config;
pub use db::{Db, DbError};
pub use store::Store;

use axum::extract::State;
use axum::http::header::{ACCEPT_RANGES, AUTHORIZATION, CONTENT_RANGE, CONTENT_TYPE, RANGE};
use axum::http::{HeaderName, HeaderValue, Method};
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
    /// What changed, for the `/ws` listeners allowed to hear it (`crate::live`).
    pub changes: broadcast::Sender<live::Change>,
    pub config: Config,
    /// The atproto OAuth client, present when the AppView is built with identity
    /// wired (the default in tests is `None`, so `/callback` reports 503).
    pub oauth: Option<Arc<oauth::WikiOAuth>>,
    /// Live firehose status (updated by the consumer task, read by `/healthz`).
    pub firehose: Arc<firehose::FirehoseStatus>,
    /// The shared outbound client, for the crate's own calls to other hosts.
    pub http: http::RustlsHttpClient,
    /// The off-node ballot replica log, when one is configured (`crate::ballot`).
    pub replica: Option<Arc<ballot_store::ReplicaLog>>,
    /// What `crate::poll` keeps between requests.
    pub polls: Arc<poll::Shared>,
    /// What `crate::feedback` keeps between requests.
    pub feedback: Arc<feedback::Shared>,
    /// What `crate::canvas` keeps between requests.
    pub canvases: Arc<canvas::Shared>,
    /// Where invitations are mailed from, when the site mails any (`crate::mail`).
    pub mailer: Option<Arc<mail::Mailer>>,
    /// The organization's account, when content is mirrored into atproto spaces
    /// (`crate::spaces`).
    pub spaces: Option<Arc<spaces::Spaces>>,
}

impl AppState {
    /// Build state around an open database, with a fresh broadcast channel and
    /// no OAuth client (see [`AppState::with_oauth`]).
    pub fn new(db: Db, config: Config) -> Self {
        let (changes, _rx) = broadcast::channel(1024);
        // As for the OAuth client: a deployed AppView reaches public hosts only.
        let reach = if config.public_url.is_empty() {
            http::Reach::Any
        } else {
            http::Reach::PublicOnly
        };
        Self {
            http: http::RustlsHttpClient::new(reach).expect("the rustls client builds"),
            db,
            changes,
            config,
            oauth: None,
            firehose: Arc::new(firehose::FirehoseStatus::default()),
            replica: None,
            polls: Arc::default(),
            feedback: Arc::default(),
            canvases: Arc::default(),
            mailer: None,
            spaces: None,
        }
    }

    /// Tell the listeners something changed. Nobody listening is not an error.
    pub fn publish(&self, topic: live::Topic, kind: &'static str, id: &str) {
        self.publish_row(topic, kind, id, None);
    }

    /// [`Self::publish`] for a change that made a row of its own on `id`.
    pub fn publish_row(&self, topic: live::Topic, kind: &'static str, id: &str, row: Option<&str>) {
        let _ = self.changes.send(live::Change {
            topic,
            kind,
            id: id.to_string(),
            row: row.map(str::to_string),
        });
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
            .allow_headers([AUTHORIZATION, CONTENT_TYPE, RANGE])
            // A player seeking through a video reads the first two off a ranged
            // reply; the frontend sets its clock by the third.
            .expose_headers([ACCEPT_RANGES, CONTENT_RANGE, SERVER_TIME])
            .max_age(Duration::from_secs(3600)),
    )
}

/// This server's clock on every answer, in unix milliseconds. A countdown or a
/// cooldown is reckoned against rows stamped here, and a phone's clock can be
/// minutes out, so the frontend measures the difference off whatever it asks.
pub const SERVER_TIME: HeaderName = HeaderName::from_static("x-server-time");

async fn stamped(mut response: axum::response::Response) -> axum::response::Response {
    response
        .headers_mut()
        .insert(SERVER_TIME, HeaderValue::from(util::now_millis()));
    response
}

/// The AppView router: liveness, the multiplexed client WebSocket, the atproto
/// OAuth login, and the XRPC surface.
pub fn router(state: AppState) -> Router {
    let cors = cors(&state.config);
    let router = build_router(state).layer(axum::middleware::map_response(stamped));
    match cors {
        Some(layer) => router.layer(layer),
        None => router,
    }
}

fn build_router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(healthz))
        .route("/ws", get(live::ws_handler))
        .route("/log", post(logs::ingest))
        .route("/login", get(oauth::login_handler))
        .route("/callback", get(oauth::callback_handler))
        .route(
            oauth::CLIENT_METADATA_PATH,
            get(oauth::client_metadata_handler),
        )
        // What the organization's PDS asks and tells, where the wiki is
        // mirrored into atproto spaces (`crate::spaces`).
        .route("/.well-known/did.json", get(spaces::did_document))
        .route(
            "/xrpc/com.atproto.simplespace.checkUserAccess",
            get(spaces::check_user_access),
        )
        .route(
            "/xrpc/com.atproto.space.notifyWrite",
            post(spaces::notify_write),
        )
        .route(
            "/xrpc/com.atproto.space.notifySpaceDeleted",
            post(spaces::notify_space_deleted),
        )
        // The native XRPC read surface (identity-free content lookups).
        .route("/xrpc/wiki.radikal.getDocument", get(xrpc::get_document))
        .route("/xrpc/wiki.radikal.getContext", get(xrpc::get_context))
        .route("/xrpc/wiki.radikal.getNode", get(xrpc::get_node))
        .route("/xrpc/wiki.radikal.resolveNode", get(xrpc::resolve_node))
        .route("/xrpc/wiki.radikal.listChildren", get(xrpc::list_children))
        .route("/xrpc/wiki.radikal.listContexts", get(xrpc::list_contexts))
        .route("/xrpc/wiki.radikal.listRecent", get(feed::list_recent))
        .route(
            "/xrpc/wiki.radikal.listContributions",
            get(feed::list_contributions),
        )
        .route("/xrpc/wiki.radikal.listOrphans", get(feed::list_orphans))
        .route("/xrpc/wiki.radikal.purgeOrphan", post(tree::purge_orphan))
        .route("/xrpc/wiki.radikal.getProfile", get(people::get_profile))
        .route(
            "/xrpc/wiki.radikal.searchPeople",
            get(people::search_people),
        )
        .route("/xrpc/wiki.radikal.search", get(search::search))
        .route("/xrpc/wiki.radikal.getComments", get(xrpc::get_comments))
        .route("/xrpc/wiki.radikal.getReactions", get(xrpc::get_reactions))
        .route(
            "/xrpc/wiki.radikal.createSession",
            post(xrpc::create_session),
        )
        .route("/xrpc/wiki.radikal.getSession", get(xrpc::get_session))
        .route(
            "/xrpc/wiki.radikal.deleteSession",
            post(xrpc::delete_session),
        )
        .route("/xrpc/wiki.radikal.listMembers", get(xrpc::list_members))
        .route(
            "/xrpc/wiki.radikal.getVoterCount",
            get(xrpc::get_voter_count),
        )
        .route(
            "/xrpc/wiki.radikal.inviteMembers",
            post(xrpc::invite_members),
        )
        .route(
            "/xrpc/wiki.radikal.sendInvitation",
            post(mail::send_invitation),
        )
        .route("/xrpc/wiki.radikal.getBoardKey", get(board::get_board_key))
        .route(
            "/xrpc/wiki.radikal.setDelegation",
            post(delegation::set_delegation),
        )
        .route(
            "/xrpc/wiki.radikal.listDelegations",
            get(delegation::list_delegations),
        )
        .route("/xrpc/wiki.radikal.updateMember", post(xrpc::update_member))
        .route("/xrpc/wiki.radikal.removeMember", post(xrpc::remove_member))
        .route(
            "/xrpc/wiki.radikal.listInvitations",
            get(xrpc::list_invitations),
        )
        .route(
            "/xrpc/wiki.radikal.acceptInvitation",
            post(xrpc::accept_invitation),
        )
        .route(
            "/xrpc/wiki.radikal.getProjector",
            get(projector::get_projector),
        )
        .route(
            "/xrpc/wiki.radikal.setProjector",
            post(projector::set_projector),
        )
        .route(
            "/xrpc/wiki.radikal.listSpeakerLists",
            get(speak::list_speaker_lists),
        )
        .route(
            "/xrpc/wiki.radikal.createSpeakerList",
            post(speak::create_speaker_list),
        )
        .route(
            "/xrpc/wiki.radikal.updateSpeakerList",
            post(speak::update_speaker_list),
        )
        .route(
            "/xrpc/wiki.radikal.deleteSpeakerList",
            post(speak::delete_speaker_list),
        )
        .route(
            "/xrpc/wiki.radikal.clearSpeakerList",
            post(speak::clear_speaker_list),
        )
        .route("/xrpc/wiki.radikal.nextSpeaker", post(speak::next_speaker))
        .route(
            "/xrpc/wiki.radikal.joinSpeakerList",
            post(speak::join_speaker_list),
        )
        .route(
            "/xrpc/wiki.radikal.leaveSpeakerList",
            post(speak::leave_speaker_list),
        )
        .route("/xrpc/wiki.radikal.moveSpeaker", post(speak::move_speaker))
        .route(
            "/xrpc/wiki.radikal.claimMembership",
            post(xrpc::claim_membership),
        )
        .route(
            "/xrpc/wiki.radikal.getMemberClaimLink",
            get(xrpc::get_member_claim_link),
        )
        // The write procedures (the session's DID authors content).
        .route(
            "/xrpc/wiki.radikal.createDocument",
            post(xrpc::create_document),
        )
        .route(
            "/xrpc/wiki.radikal.updateDocument",
            post(xrpc::update_document),
        )
        .route(
            "/xrpc/wiki.radikal.setDocumentAuthors",
            post(xrpc::set_document_authors),
        )
        .route("/xrpc/wiki.radikal.moveDocument", post(xrpc::move_document))
        .route(
            "/xrpc/wiki.radikal.createCanvas",
            post(canvas::create_canvas),
        )
        .route("/xrpc/wiki.radikal.getCanvas", get(canvas::get_canvas))
        .route("/xrpc/wiki.radikal.paintCell", post(canvas::paint_cell))
        .route(
            "/xrpc/wiki.radikal.setCanvasOpen",
            post(canvas::set_canvas_open),
        )
        .route(
            "/xrpc/wiki.radikal.createContext",
            post(context::create_context),
        )
        .route(
            "/xrpc/wiki.radikal.updateContext",
            post(context::update_context),
        )
        .route(
            "/xrpc/wiki.radikal.deleteContext",
            post(context::delete_context),
        )
        .route(
            "/xrpc/wiki.radikal.restoreContext",
            post(context::restore_context),
        )
        .route("/xrpc/wiki.radikal.copyDocument", post(tree::copy_document))
        .route(
            "/xrpc/wiki.radikal.purgeDocument",
            post(tree::purge_document),
        )
        .route(
            "/xrpc/wiki.radikal.deleteDocument",
            post(xrpc::delete_document),
        )
        .route(
            "/xrpc/wiki.radikal.restoreDocument",
            post(xrpc::restore_document),
        )
        .route("/xrpc/wiki.radikal.listDeleted", get(xrpc::list_deleted))
        .route("/xrpc/wiki.radikal.postComment", post(xrpc::post_comment))
        .route(
            "/xrpc/wiki.radikal.deleteComment",
            post(comment::delete_comment),
        )
        .route(
            "/xrpc/wiki.radikal.restoreComment",
            post(comment::restore_comment),
        )
        .route(
            "/xrpc/wiki.radikal.purgeComment",
            post(comment::purge_comment),
        )
        .route("/xrpc/wiki.radikal.addReaction", post(xrpc::add_reaction))
        .route(
            "/xrpc/wiki.radikal.removeReaction",
            post(xrpc::remove_reaction),
        )
        .route("/xrpc/wiki.radikal.openPoll", post(poll::open_poll))
        .route("/xrpc/wiki.radikal.closePoll", post(poll::close_poll))
        .route("/xrpc/wiki.radikal.getPoll", get(poll::get_poll))
        .route("/xrpc/wiki.radikal.listPolls", get(poll::list_polls))
        .route("/xrpc/wiki.radikal.getBoard", get(poll::get_board))
        .route(
            "/xrpc/wiki.radikal.getBoardEntry",
            get(poll::get_board_entry),
        )
        .route(
            "/xrpc/wiki.radikal.issueBallotTokens",
            post(poll::issue_ballot_tokens),
        )
        .route("/xrpc/wiki.radikal.castBallot", post(poll::cast_ballot))
        .route(
            "/xrpc/wiki.radikal.castOpenBallot",
            post(poll::cast_open_ballot),
        )
        .route(
            "/xrpc/wiki.radikal.submitFeedback",
            post(feedback::submit_feedback),
        )
        .route(
            "/xrpc/wiki.radikal.listFeedback",
            get(feedback::list_feedback),
        )
        .route(
            "/xrpc/wiki.radikal.deleteFeedback",
            post(feedback::delete_feedback),
        )
        .route(
            "/xrpc/wiki.radikal.subscribePush",
            post(push::subscribe_push),
        )
        .route(
            "/xrpc/wiki.radikal.unsubscribePush",
            post(push::unsubscribe_push),
        )
        .route(
            "/xrpc/wiki.radikal.notifyContext",
            post(push::notify_context),
        )
        .route("/xrpc/wiki.radikal.notifyReply", post(push::notify_reply))
        .route(
            "/xrpc/wiki.radikal.shareToBluesky",
            post(share::share_to_bluesky),
        )
        .route(
            "/xrpc/wiki.radikal.renderMetafile",
            post(metafile::render_metafile),
        )
        .route("/xrpc/wiki.radikal.parseRoster", post(roster::parse_roster))
        .route("/blob/{id}", get(blob::serve_blob))
        .route("/xrpc/wiki.radikal.uploadBlob", post(blob::upload_blob))
        .route("/xrpc/wiki.radikal.getBlobLink", get(blob::get_blob_link))
        .route("/xrpc/wiki.radikal.deleteBlob", post(blob::delete_blob))
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
        "clients": state.changes.receiver_count(),
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
        let stamp: i64 = resp
            .headers()
            .get(SERVER_TIME)
            .and_then(|v| v.to_str().ok())
            .and_then(|v| v.parse().ok())
            .expect("every answer says what time it is here");
        assert!((util::now_millis() - stamp).abs() < 60_000, "{stamp}");
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
                .uri("/xrpc/wiki.radikal.createSession")
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

        let theirs = preflight(app.clone(), "https://evil.example").await;
        assert!(
            theirs.get("access-control-allow-origin").is_none(),
            "a foreign origin was granted CORS"
        );

        // A page on another origin reads only the headers it is shown.
        let asked = Request::builder()
            .uri("/healthz")
            .header("origin", "https://wiki.example")
            .body(Body::empty())
            .unwrap();
        let answer = app.oneshot(asked).await.expect("request");
        let shown = answer
            .headers()
            .get("access-control-expose-headers")
            .and_then(|v| v.to_str().ok())
            .unwrap_or_default()
            .to_ascii_lowercase();
        assert!(shown.contains("x-server-time"), "{shown}");
    }
}
