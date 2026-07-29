//! User feedback / bug reports / feature requests. A member (signed in or not)
//! submits from the app; the report becomes a `wiki/feedback` node under the root
//! node — the same node the in-app feedback dialog creates — so it shows up in
//! the feedback app beside every other report rather than in a log only an
//! operator reads.
//!
//! Only the crash overlay posts here (`src/crash.rs`): the dialog runs inside a
//! live app and inserts the node itself, but a panic leaves the wasm instance
//! trapped, so its Report button can do nothing but `fetch` a URL. Doing the
//! insert server-side is also what lets the stack be symbolicated on the way in
//! (see [`crate::symbolicate`]) — the reader's browser has no DWARF to resolve
//! against.
//!
//! BetterStack (Logtail) remains the fallback sink, for when the node cannot be
//! written: a report is worth keeping even somewhere less convenient.
//!
//!   POST /feedback?kind=bug|feature|other&message=&path=&app=&ua=
//!        (Authorization: Bearer <jwt> optional; captures the sender when present)

use crate::error::AppError;
use crate::oauth::Config;
use axum::response::Response;
use http::StatusCode;
use serde_json::{json, Value};

/// Cap on the stored message (matches the client-side maxlength), so a runaway
/// paste cannot bloat a node or an ingest event.
const MAX_MESSAGE: usize = 4000;

/// Cap on the node's name, which is the message's first line in the feedback
/// app's list. Matches the frontend's `insert_feedback`.
const MAX_NAME: usize = 80;

pub async fn submit(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response {
    match submit_inner(cfg, client, query, bearer).await {
        Ok(()) => crate::json(StatusCode::OK, "{\"ok\":true}".into()),
        Err(e) => e.respond("feedback"),
    }
}

async fn submit_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(), AppError> {
    let params = crate::util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };

    let mut message = get("message")
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .ok_or(AppError::BadRequest("missing message".into()))?;
    // A crash report carries the panic's stack in its message, so it gets the
    // same treatment as a shipped log entry — otherwise the one report a reader
    // deliberately chose to send would be the least readable thing in the sink.
    // Ordinary feedback has no wasm frames and passes through untouched.
    message = crate::symbolicate::resolve_stack(client, &cfg.app_origin, &message).await;
    message.truncate(MAX_MESSAGE);
    let kind = match get("kind").as_deref() {
        Some("bug") => "bug",
        Some("feature") => "feature",
        _ => "other",
    };
    let path = get("path").unwrap_or_default();
    let app = get("app").unwrap_or_default();
    // The build the report came from, which is what ties it to code — `app` is
    // the crate version and reads the same for every build ever made.
    let commit = get("commit").unwrap_or_default();
    let ua = get("ua").unwrap_or_default();

    // Best-effort identity: captured when a valid token is present, anonymous
    // otherwise (a logged-out member hitting a bug can still report it).
    let sender = crate::auth::caller(cfg, client, query, bearer).await.ok();
    let owner_id = sender.as_ref().map(|(id, _)| id.as_str());

    let report = Report {
        kind,
        message: &message,
        path: &path,
        app: &app,
        commit: &commit,
        ua: &ua,
    };
    match insert_feedback_node(cfg, client, &report, owner_id).await {
        Ok(()) => Ok(()),
        // Losing the report would be worse than filing it somewhere awkward, so
        // fall back to the log sink rather than failing the request.
        Err(e) => {
            tracing::warn!("feedback node insert failed ({e}); shipping to the log sink instead");
            ship_feedback(cfg, client, &report, sender).await
        }
    }
}

/// One submission, as it arrived. Grouped because both sinks take all of it and
/// threading six strings through two signatures obscured which was which.
struct Report<'a> {
    kind: &'a str,
    message: &'a str,
    path: &'a str,
    /// The crate version.
    app: &'a str,
    /// The commit the bundle was built from, or empty from a build too old to
    /// report one.
    commit: &'a str,
    ua: &'a str,
}

/// Create the `wiki/feedback` node, mirroring the frontend's `insert_feedback`
/// exactly — same mime, same parent, same `data` keys — so the feedback app
/// renders a crash report and a typed one identically.
///
/// Admin-secret, because the caller may be anonymous and because a crash report
/// arrives with a token this endpoint has already validated. That bypasses the
/// column preset that would otherwise stamp the owner from the JWT, so `ownerId`
/// is set explicitly; null for an anonymous report, which the select rule leaves
/// visible to home-context owners.
async fn insert_feedback_node(
    cfg: &Config,
    client: &reqwest::Client,
    report: &Report<'_>,
    owner_id: Option<&str>,
) -> Result<(), String> {
    let root_id = root_node_id(cfg, client).await?;

    let name: String = report.message.trim().chars().take(MAX_NAME).collect();
    let name = if name.is_empty() {
        report.kind.to_string()
    } else {
        name
    };

    let mut object = json!({
        "name": name,
        "key": feedback_key(),
        "mimeId": "wiki/feedback",
        "parentId": root_id,
        "contextId": root_id,
        "mutable": false,
        "data": {
            "kind": report.kind,
            "message": report.message,
            "path": report.path,
            "appVersion": report.app,
            "commit": report.commit,
            "userAgent": report.ua,
        },
    });
    if let Some(uid) = owner_id {
        object["ownerId"] = Value::from(uid);
    }

    let body = json!({
        "query": "mutation($object: nodes_insert_input!) { \
                  insertNode(object: $object) { id } }",
        "variables": { "object": object },
    });
    let value = hasura(cfg, client, &body).await?;
    value
        .get("data")
        .and_then(|d| d.get("insertNode"))
        .filter(|n| !n.is_null())
        .map(|_| ())
        .ok_or_else(|| "insertNode returned no row".to_string())
}

/// The parent-less root node, which owns the feedback collection. Looked up per
/// report: reports are rare, and caching it would only add a way to go stale.
async fn root_node_id(cfg: &Config, client: &reqwest::Client) -> Result<String, String> {
    let body = json!({
        "query": "query { nodes(where: { parentId: { _is_null: true } }, limit: 1) { id } }",
    });
    let value = hasura(cfg, client, &body).await?;
    value
        .get("data")
        .and_then(|d| d.get("nodes"))
        .and_then(|n| n.as_array())
        .and_then(|a| a.first())
        .and_then(|n| n.get("id"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "root node not found".to_string())
}

/// POST a GraphQL body as admin, surfacing a GraphQL-level `errors` array as an
/// error — Hasura answers 200 for those, so the status alone proves nothing.
async fn hasura(cfg: &Config, client: &reqwest::Client, body: &Value) -> Result<Value, String> {
    let resp = client
        .post(&cfg.hasura_url)
        .header("x-hasura-admin-secret", &cfg.admin_secret)
        .json(body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let value: Value = resp.json().await.map_err(|e| e.to_string())?;
    match value.get("errors") {
        Some(errors) => Err(format!("hasura error: {errors}")),
        None => Ok(value),
    }
}

/// A unique `key` for the node. The frontend uses time + a random number; here a
/// process-local counter does the same job without a source of randomness, since
/// two reports in the same millisecond would have to come from one process.
fn feedback_key() -> String {
    use std::sync::atomic::{AtomicU64, Ordering};
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let millis = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis())
        .unwrap_or(0);
    format!("feedback-{millis}-{}", SEQ.fetch_add(1, Ordering::Relaxed))
}

/// Fallback sink: ship one feedback event to BetterStack (the frontend logger's
/// sink; no `dt` field, so BetterStack stamps ingest time). Reached only when the
/// node could not be written. A ship failure is UPSTREAM.
async fn ship_feedback(
    cfg: &Config,
    client: &reqwest::Client,
    report: &Report<'_>,
    sender: Option<(String, String)>,
) -> Result<(), AppError> {
    let (user_id, email) = sender.unwrap_or_else(|| ("anonymous".into(), String::new()));
    let Report {
        kind,
        message,
        path,
        app,
        commit,
        ua,
    } = *report;

    if cfg.betterstack_token.is_empty() {
        // No sink configured: log server-side so the report is not silently
        // dropped (the container's stdout is captured).
        tracing::warn!("feedback [{kind}] from {user_id}: {message} (path={path})");
        return Ok(());
    }

    let entry = json!({
        "level": "info",
        "source": "feedback",
        "message": format!("feedback [{kind}]: {message}"),
        "feedback": {
            "kind": kind,
            "message": message,
            "path": path,
            "app_version": app,
            "commit": commit,
            "user_agent": ua,
            "user_id": user_id,
            "email": email,
        },
    });
    let url = format!("https://{}/", cfg.betterstack_host);
    client
        .post(&url)
        .bearer_auth(&cfg.betterstack_token)
        .json(&json!([entry]))
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("feedback ship failed: {e}")))?;
    Ok(())
}
