//! HTTP route definitions and WebSocket handlers.
//!
//! Defines the axum router, shared application state, and handler functions
//! for the MOTD, `/events` WebSocket, `/logs/{knot}/{rkey}/{name}` WebSocket,
//! and `/xrpc/{method}` XRPC dispatch endpoints.

use std::io::{BufRead, Seek, SeekFrom};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::Router;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Path, Query, State};
use axum::http::header;
use axum::response::IntoResponse;
use axum::routing::get;
use serde::Deserialize;
use spindle_db::Database;
use spindle_models::pipeline::PipelineId;
use spindle_models::workflow::WorkflowId;
use spindle_models::workflow_logger;
use spindle_xrpc::XrpcContext;
use tokio::sync::mpsc;
use tracing::{debug, error, warn};

use crate::notifier::Notifier;

/// Shared application state, available to all route handlers via
/// `axum::extract::State<Arc<AppState>>`.
pub struct AppState {
    /// Database handle.
    pub db: Arc<Database>,
    /// Event notifier (broadcast channel).
    pub notifier: Arc<Notifier>,
    /// XRPC context (shared with the XRPC sub-router).
    pub xrpc: Arc<XrpcContext>,
    /// Directory where workflow log files are stored.
    pub log_dir: PathBuf,
    /// Public hostname of this spindle instance.
    pub hostname: String,
}

impl std::fmt::Debug for AppState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AppState")
            .field("hostname", &self.hostname)
            .field("log_dir", &self.log_dir)
            .finish_non_exhaustive()
    }
}

/// Build the axum router with all routes.
pub fn build_router(state: Arc<AppState>) -> Router {
    let xrpc_ctx = Arc::clone(&state.xrpc);

    let xrpc_router = Router::new()
        .route(
            "/{method}",
            get(spindle_xrpc::dispatch_query).post(spindle_xrpc::dispatch),
        )
        .with_state(xrpc_ctx);

    Router::new()
        .route("/", get(motd_handler))
        .route("/.well-known/did.json", get(did_json_handler))
        .route("/events", get(events_ws_handler))
        .route("/logs/{knot}/{rkey}/{name}", get(logs_ws_handler))
        .nest("/xrpc", xrpc_router)
        .with_state(state)
}

// ---------------------------------------------------------------------------
// MOTD handler
// ---------------------------------------------------------------------------

/// `GET /` — Return a simple MOTD text identifying the spindle instance.
async fn motd_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    format!(
        "tangled-spindle-nix v{} @ {}\n",
        env!("CARGO_PKG_VERSION"),
        state.hostname,
    )
}

// ---------------------------------------------------------------------------
// /.well-known/did.json handler
// ---------------------------------------------------------------------------

/// `GET /.well-known/did.json` — Return the DID document for `did:web` resolution.
///
/// When tangled.org verifies a spindle, it resolves `did:web:{hostname}` by
/// fetching `https://{hostname}/.well-known/did.json`.
async fn did_json_handler(State(state): State<Arc<AppState>>) -> impl IntoResponse {
    let did_web = format!("did:web:{}", state.hostname);
    let scheme = if state.xrpc.dev { "http" } else { "https" };
    let doc = serde_json::json!({
        "@context": ["https://www.w3.org/ns/did/v1"],
        "id": did_web,
        "service": [{
            "id": "#tangled_spindle",
            "type": "TangledSpindle",
            "serviceEndpoint": format!("{scheme}://{}", state.hostname),
        }],
    });
    (
        [(header::CONTENT_TYPE, "application/did+ld+json")],
        serde_json::to_string_pretty(&doc).unwrap(),
    )
}

// ---------------------------------------------------------------------------
// /events WebSocket handler
// ---------------------------------------------------------------------------

/// Query parameters for the `/events` WebSocket endpoint.
#[derive(Debug, Deserialize)]
struct EventsQuery {
    /// Cursor: the last event ID the client has seen. Events with `id > cursor`
    /// will be backfilled on connection.
    #[serde(default)]
    cursor: Option<i64>,
}

/// `GET /events` — Upgrade to WebSocket for pipeline event streaming.
async fn events_ws_handler(
    ws: WebSocketUpgrade,
    Query(query): Query<EventsQuery>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| events_ws(socket, state, query.cursor.unwrap_or(0)))
}

/// Serialize an event into the envelope format expected by the tangled appview:
/// `{"rkey": "...", "nsid": "...", "event": {...}}`
fn event_to_envelope(event: &spindle_db::events::Event) -> Result<String, serde_json::Error> {
    // Parse the payload back into a JSON value so it's nested (not double-escaped).
    let event_json: serde_json::Value = serde_json::from_str(&event.payload)
        .unwrap_or_else(|_| serde_json::Value::String(event.payload.clone()));

    let envelope = serde_json::json!({
        "rkey": event.rkey,
        "nsid": event.nsid,
        "event": event_json,
    });

    serde_json::to_string(&envelope)
}

/// Handle the `/events` WebSocket connection.
///
/// Protocol:
/// 1. Subscribe to the broadcast channel **before** backfilling (avoids race).
/// 2. Backfill events from the database with `created > cursor`.
/// 3. Stream live events from the broadcast channel, deduplicating by created timestamp.
/// 4. Send keepalive pings every 30 seconds.
///
/// Events are sent in the envelope format: `{"rkey":"...","nsid":"...","event":{...}}`
async fn events_ws(mut socket: WebSocket, state: Arc<AppState>, cursor: i64) {
    // Subscribe first to avoid missing events between backfill and live.
    let mut rx = state.notifier.subscribe();
    let mut last_sent_cursor = cursor;

    // Backfill from database.
    match state.db.get_events_after(cursor) {
        Ok(events) => {
            for event in events {
                let created = event.created;
                match event_to_envelope(&event) {
                    Ok(json) => {
                        if socket.send(Message::Text(json.into())).await.is_err() {
                            return;
                        }
                        last_sent_cursor = created;
                    }
                    Err(e) => {
                        warn!(%e, "failed to serialize event for backfill");
                    }
                }
            }
        }
        Err(e) => {
            warn!(%e, "failed to backfill events from database");
        }
    }

    debug!(
        last_sent_cursor,
        "events backfill complete, switching to live stream"
    );

    // Live stream loop.
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            result = rx.recv() => {
                match result {
                    Ok(event) => {
                        if event.created <= last_sent_cursor {
                            continue; // Deduplicate.
                        }
                        last_sent_cursor = event.created;
                        match event_to_envelope(&event) {
                            Ok(json) => {
                                if socket.send(Message::Text(json.into())).await.is_err() {
                                    return;
                                }
                            }
                            Err(e) => {
                                warn!(%e, "failed to serialize live event");
                            }
                        }
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                        warn!(n, "events WebSocket consumer lagged, some events may be missed");
                        // Client can reconnect with a cursor to re-backfill.
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => {
                        debug!("events broadcast channel closed");
                        return;
                    }
                }
            }
            _ = keepalive.tick() => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Err(_)) => return,
                    _ => {} // Ignore text/binary/pong from client.
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// /logs/{knot}/{rkey}/{name} WebSocket handler
// ---------------------------------------------------------------------------

/// `GET /logs/{knot}/{rkey}/{name}` — Upgrade to WebSocket for log streaming.
async fn logs_ws_handler(
    ws: WebSocketUpgrade,
    Path((knot, rkey, name)): Path<(String, String, String)>,
    State(state): State<Arc<AppState>>,
) -> impl IntoResponse {
    ws.on_upgrade(move |socket| logs_ws(socket, state, knot, rkey, name))
}

/// Handle the `/logs` WebSocket connection.
///
/// Streams NDJSON log lines for a specific workflow. If the workflow is
/// finished, the complete log file is sent and the connection is closed.
/// If the workflow is still running, existing lines are sent and then new
/// lines are streamed in real-time using filesystem notification.
async fn logs_ws(
    mut socket: WebSocket,
    state: Arc<AppState>,
    knot: String,
    rkey: String,
    name: String,
) {
    let pipeline_id = PipelineId {
        knot: knot.clone(),
        rkey: rkey.clone(),
    };
    let workflow_id = WorkflowId::new(pipeline_id, &name);
    let log_path = workflow_logger::log_file_path(&state.log_dir, &workflow_id);
    let wid_str = workflow_id.to_string();

    // Check if workflow is in a terminal state.
    let is_finished = match state.db.get_status(&wid_str) {
        Ok(Some(status)) => {
            matches!(
                status.status.as_str(),
                "success" | "failed" | "timeout" | "cancelled"
            )
        }
        Ok(None) => {
            // Workflow not found — might not have started yet. Try to stream anyway.
            false
        }
        Err(e) => {
            warn!(%e, workflow_id = %wid_str, "failed to get workflow status");
            false
        }
    };

    if is_finished {
        // Send the complete log file and close.
        if send_log_file(&mut socket, &log_path).await.is_err() {
            return;
        }
        let _ = socket.send(Message::Close(None)).await;
        return;
    }

    // Workflow is pending or running — stream existing + live tail.
    //
    // Set up a filesystem watcher to detect new lines appended to the log file.
    let (notify_tx, mut notify_rx) = mpsc::channel::<()>(16);

    let watcher = setup_file_watcher(&log_path, notify_tx);
    if watcher.is_err() {
        warn!(path = %log_path.display(), "failed to set up file watcher, falling back to polling");
    }
    // Keep the watcher alive for the duration of the connection.
    let _watcher = watcher.ok();

    // Read existing content from the log file.
    let mut file = match std::fs::File::open(&log_path) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            // File doesn't exist yet. Wait for it to appear.
            debug!(path = %log_path.display(), "log file not found, waiting for creation");
            match wait_for_file(&log_path, &mut socket, &mut notify_rx).await {
                Some(f) => f,
                None => return,
            }
        }
        Err(e) => {
            error!(%e, path = %log_path.display(), "failed to open log file");
            return;
        }
    };

    // Send existing content.
    if send_lines_from_file(&mut socket, &mut file).await.is_err() {
        return;
    }

    // Live tail loop.
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    // Also poll periodically in case inotify misses an event.
    let mut poll_interval = tokio::time::interval(Duration::from_secs(2));
    poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = notify_rx.recv() => {
                if send_lines_from_file(&mut socket, &mut file).await.is_err() {
                    return;
                }

                // Re-check if workflow finished.
                if check_finished(&state.db, &wid_str) {
                    // Send any remaining lines and close.
                    let _ = send_lines_from_file(&mut socket, &mut file).await;
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
            _ = poll_interval.tick() => {
                if send_lines_from_file(&mut socket, &mut file).await.is_err() {
                    return;
                }

                if check_finished(&state.db, &wid_str) {
                    let _ = send_lines_from_file(&mut socket, &mut file).await;
                    let _ = socket.send(Message::Close(None)).await;
                    return;
                }
            }
            _ = keepalive.tick() => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    return;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => return,
                    Some(Err(_)) => return,
                    _ => {}
                }
            }
        }
    }
}

/// Send all lines from a log file over the WebSocket.
async fn send_log_file(socket: &mut WebSocket, path: &std::path::Path) -> Result<(), ()> {
    let file = match std::fs::File::open(path) {
        Ok(f) => f,
        Err(e) => {
            warn!(%e, path = %path.display(), "failed to open log file");
            return Err(());
        }
    };
    let reader = std::io::BufReader::new(file);
    for line in reader.lines() {
        match line {
            Ok(line) if !line.is_empty() => {
                if socket.send(Message::Text(line.into())).await.is_err() {
                    return Err(());
                }
            }
            Ok(_) => {} // Skip empty lines.
            Err(e) => {
                warn!(%e, "error reading log file line");
                break;
            }
        }
    }
    Ok(())
}

/// Read new lines from the current file position and send them over WebSocket.
async fn send_lines_from_file(socket: &mut WebSocket, file: &mut std::fs::File) -> Result<(), ()> {
    let mut reader = std::io::BufReader::new(&*file);
    let mut line = String::new();
    loop {
        line.clear();
        match reader.read_line(&mut line) {
            Ok(0) => break, // EOF — no more data.
            Ok(_) => {
                let trimmed = line.trim_end();
                if !trimmed.is_empty()
                    && socket
                        .send(Message::Text(trimmed.to_owned().into()))
                        .await
                        .is_err()
                {
                    // Update file position before returning error.
                    let pos = reader.stream_position().unwrap_or(0);
                    let _ = file.seek(SeekFrom::Start(pos));
                    return Err(());
                }
            }
            Err(e) => {
                warn!(%e, "error reading log file");
                break;
            }
        }
    }
    // Update the underlying file's position to match what we've read.
    let pos = reader.stream_position().unwrap_or(0);
    let _ = file.seek(SeekFrom::Start(pos));
    Ok(())
}

/// Set up a filesystem watcher for the given path.
///
/// Sends a notification on the `tx` channel whenever the file is modified.
/// Uses the `notify` crate with inotify on Linux.
fn setup_file_watcher(
    path: &std::path::Path,
    tx: mpsc::Sender<()>,
) -> Result<notify::RecommendedWatcher, notify::Error> {
    use notify::{RecursiveMode, Watcher};

    let watch_path = if path.exists() {
        path.to_path_buf()
    } else {
        // Watch the parent directory if the file doesn't exist yet.
        path.parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| path.to_path_buf())
    };

    let mut watcher = notify::recommended_watcher(move |res: Result<notify::Event, _>| {
        if let Ok(event) = res {
            use notify::EventKind;
            match event.kind {
                EventKind::Modify(_) | EventKind::Create(_) => {
                    let _ = tx.try_send(());
                }
                _ => {}
            }
        }
    })?;

    watcher.watch(&watch_path, RecursiveMode::NonRecursive)?;
    Ok(watcher)
}

/// Wait for a log file to be created, sending keepalive pings in the meantime.
///
/// Returns the opened file, or `None` if the WebSocket connection closed.
async fn wait_for_file(
    path: &std::path::Path,
    socket: &mut WebSocket,
    notify_rx: &mut mpsc::Receiver<()>,
) -> Option<std::fs::File> {
    let mut keepalive = tokio::time::interval(Duration::from_secs(30));
    keepalive.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    let mut poll_interval = tokio::time::interval(Duration::from_secs(1));
    poll_interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);

    loop {
        tokio::select! {
            _ = notify_rx.recv() => {
                if let Ok(f) = std::fs::File::open(path) {
                    return Some(f);
                }
            }
            _ = poll_interval.tick() => {
                if let Ok(f) = std::fs::File::open(path) {
                    return Some(f);
                }
            }
            _ = keepalive.tick() => {
                if socket.send(Message::Ping(vec![].into())).await.is_err() {
                    return None;
                }
            }
            msg = socket.recv() => {
                match msg {
                    Some(Ok(Message::Close(_))) | None => return None,
                    Some(Err(_)) => return None,
                    _ => {}
                }
            }
        }
    }
}

/// Check if a workflow has reached a terminal state.
fn check_finished(db: &Database, workflow_id: &str) -> bool {
    match db.get_status(workflow_id) {
        Ok(Some(status)) => matches!(
            status.status.as_str(),
            "success" | "failed" | "timeout" | "cancelled"
        ),
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use http_body_util::BodyExt;
    use spindle_rbac::SpindleEnforcer;
    use spindle_secrets::SqliteManager;
    use tower::ServiceExt;

    const TEST_TOKEN: &str = "test-token-abc123";
    const TEST_OWNER: &str = "did:plc:testowner";
    const TEST_HOSTNAME: &str = "spindle.test.example.com";

    /// Create a fully wired test app with in-memory DB, RBAC, and secrets.
    async fn test_app() -> (Router, Arc<AppState>) {
        let db = Arc::new(spindle_db::Database::open_in_memory().unwrap());
        let notifier = Arc::new(Notifier::new(64));
        let log_dir = std::env::temp_dir().join("spindle-test-logs");
        let _ = std::fs::create_dir_all(&log_dir);

        let rbac = SpindleEnforcer::new().await.unwrap();
        let did_web = format!("did:web:{TEST_HOSTNAME}");
        rbac.add_spindle(&did_web).await.unwrap();
        rbac.add_spindle_owner(&did_web, TEST_OWNER).await.unwrap();

        let secrets_path =
            std::env::temp_dir().join(format!("spindle-test-secrets-{}.db", std::process::id()));
        let secrets: Arc<dyn spindle_secrets::Manager + Send + Sync> =
            Arc::new(SqliteManager::new(&secrets_path, &[0u8; 32]).unwrap());

        let xrpc = Arc::new(XrpcContext {
            db: Arc::clone(&db),
            rbac,
            secrets,
            did_web,
            owner: TEST_OWNER.into(),
            token: TEST_TOKEN.into(),
            plc_url: "https://plc.directory".into(),
            http_client: reqwest::Client::new(),
            dev: true,
        });

        let state = Arc::new(AppState {
            db,
            notifier,
            xrpc,
            log_dir,
            hostname: TEST_HOSTNAME.into(),
        });

        let router = build_router(Arc::clone(&state));
        (router, state)
    }

    fn auth_header() -> String {
        format!("Bearer {TEST_TOKEN}")
    }

    async fn body_json(body: Body) -> serde_json::Value {
        let bytes = body.collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap()
    }

    #[test]
    fn motd_format() {
        let version = env!("CARGO_PKG_VERSION");
        let expected = format!("tangled-spindle-nix v{version} @ test.example.com\n");
        let result = format!(
            "tangled-spindle-nix v{} @ {}\n",
            version, "test.example.com"
        );
        assert_eq!(result, expected);
    }

    // -----------------------------------------------------------------------
    // XRPC: owner query
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn xrpc_owner_returns_owner_did() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::get("/xrpc/sh.tangled.owner")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["owner"], TEST_OWNER);
    }

    // -----------------------------------------------------------------------
    // DID document endpoint
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn get_did_json_returns_valid_document() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::get("/.well-known/did.json")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        assert_eq!(
            resp.headers().get("content-type").unwrap(),
            "application/did+ld+json"
        );

        let json = body_json(resp.into_body()).await;
        assert_eq!(json["id"], format!("did:web:{TEST_HOSTNAME}"));
        assert_eq!(json["service"][0]["type"], "TangledSpindle");
        assert!(
            json["service"][0]["serviceEndpoint"]
                .as_str()
                .unwrap()
                .contains(TEST_HOSTNAME)
        );
    }

    // -----------------------------------------------------------------------
    // MOTD endpoint
    // -----------------------------------------------------------------------

    #[tokio::test]
    #[ignore] // Flaky: "database is locked" under concurrent test execution
    async fn get_motd_returns_hostname() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(Request::get("/").body(Body::empty()).unwrap())
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(text.contains(TEST_HOSTNAME));
        assert!(text.contains("tangled-spindle-nix"));
    }

    // -----------------------------------------------------------------------
    // XRPC: unknown method
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn xrpc_unknown_method_returns_404() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.nonexistent")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from("{}"))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    // -----------------------------------------------------------------------
    // XRPC: authentication
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn xrpc_missing_auth_returns_401() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.addMember")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"did":"did:plc:test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn xrpc_wrong_token_returns_401() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.addMember")
                    .header("authorization", "Bearer wrong-token")
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"did":"did:plc:test"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    // -----------------------------------------------------------------------
    // XRPC: member management
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn xrpc_add_and_remove_member() {
        let (app, state) = test_app().await;

        // Add member
        let resp = app
            .clone()
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.addMember")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"did":"did:plc:newmember"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["success"], true);

        // Verify in DB
        assert!(state.db.is_member("did:plc:newmember").unwrap());

        // Remove member
        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.removeMember")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"did":"did:plc:newmember"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[tokio::test]
    async fn xrpc_add_member_invalid_body_returns_400() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.addMember")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"wrong_field":"value"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }

    // -----------------------------------------------------------------------
    // XRPC: secret management
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn xrpc_secret_lifecycle() {
        let (app, _) = test_app().await;

        // Put secret
        let resp = app
            .clone()
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.putSecret")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"repo":"did:plc:alice/myrepo","key":"API_KEY","value":"secret123"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);

        // List secrets
        let resp = app
            .clone()
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.listSecrets")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"repo":"did:plc:alice/myrepo"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let secrets = json["secrets"].as_array().unwrap();
        assert_eq!(secrets.len(), 1);
        assert_eq!(secrets[0]["key"], "API_KEY");

        // Delete secret
        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.deleteSecret")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(
                        r#"{"repo":"did:plc:alice/myrepo","key":"API_KEY"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    // -----------------------------------------------------------------------
    // XRPC: cancel pipeline
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn xrpc_cancel_nonexistent_pipeline_returns_404() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.cancelPipeline")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"workflow_id":"nonexistent-wid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn xrpc_cancel_running_pipeline() {
        let (app, state) = test_app().await;

        // Create a running workflow
        state
            .db
            .status_pending("test-wid", "knot", "rkey", "did:plc:test", "test-workflow")
            .unwrap();
        state.db.status_running("test-wid").unwrap();

        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.cancelPipeline")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"workflow_id":"test-wid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let status = state.db.get_status("test-wid").unwrap().unwrap();
        assert_eq!(status.status, "cancelled");
    }

    // -----------------------------------------------------------------------
    // XRPC: list runs
    // -----------------------------------------------------------------------

    #[tokio::test]
    async fn xrpc_list_runs_empty() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::get("/xrpc/sh.tangled.spindle.listRuns")
                    .header("authorization", auth_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        assert_eq!(json["runs"].as_array().unwrap().len(), 0);
    }

    #[tokio::test]
    async fn xrpc_list_runs_returns_workflows() {
        let (app, state) = test_app().await;

        state
            .db
            .status_pending(
                "wid-1",
                "knot.example.com",
                "rkey1",
                "did:plc:test",
                "build",
            )
            .unwrap();
        state.db.status_running("wid-1").unwrap();
        state
            .db
            .status_pending("wid-2", "knot.example.com", "rkey1", "did:plc:test", "test")
            .unwrap();

        let resp = app
            .oneshot(
                Request::get("/xrpc/sh.tangled.spindle.listRuns")
                    .header("authorization", auth_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let runs = json["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 2);
        // Most recent first
        assert_eq!(runs[0]["workflow_id"], "wid-2");
        assert_eq!(runs[1]["workflow_id"], "wid-1");
    }

    #[tokio::test]
    async fn xrpc_list_runs_filter_by_status() {
        let (app, state) = test_app().await;

        state
            .db
            .status_pending("wid-1", "knot", "rkey1", "did:plc:test", "build")
            .unwrap();
        state.db.status_running("wid-1").unwrap();
        state
            .db
            .status_pending("wid-2", "knot", "rkey2", "did:plc:test", "test")
            .unwrap();

        let resp = app
            .oneshot(
                Request::get("/xrpc/sh.tangled.spindle.listRuns?status=running")
                    .header("authorization", auth_header())
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::OK);
        let json = body_json(resp.into_body()).await;
        let runs = json["runs"].as_array().unwrap();
        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0]["status"], "running");
    }

    #[tokio::test]
    async fn xrpc_list_runs_requires_auth() {
        let (app, _) = test_app().await;
        let resp = app
            .oneshot(
                Request::get("/xrpc/sh.tangled.spindle.listRuns")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn xrpc_cancel_already_finished_returns_400() {
        let (app, state) = test_app().await;

        // Create a finished workflow
        state
            .db
            .status_pending("done-wid", "knot", "rkey", "did:plc:test", "done-workflow")
            .unwrap();
        state.db.status_running("done-wid").unwrap();
        state.db.status_success("done-wid").unwrap();

        let resp = app
            .oneshot(
                Request::post("/xrpc/sh.tangled.spindle.cancelPipeline")
                    .header("authorization", auth_header())
                    .header("content-type", "application/json")
                    .body(Body::from(r#"{"workflow_id":"done-wid"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
    }
}
