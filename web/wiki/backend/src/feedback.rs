//! User feedback / bug reports / feature requests. A member (signed in or not)
//! submits from the app; the report is shipped to BetterStack (Logtail) as a
//! structured `feedback` event, the same sink the frontend ships errors to, so
//! it lands in the existing observability inbox with no new service to run.
//! This endpoint survives into the AppView (only the sink call would change).
//!
//!   POST /feedback?kind=bug|feature|other&message=&path=&app=&ua=
//!        (Authorization: Bearer <jwt> optional; captures the sender when present)

use crate::error::AppError;
use crate::oauth::Config;
use axum::response::Response;
use http::StatusCode;
use serde_json::json;

/// Cap on the shipped message (matches the client-side maxlength), so a runaway
/// paste cannot bloat an ingest event.
const MAX_MESSAGE: usize = 4000;

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
    let ua = get("ua").unwrap_or_default();

    // Best-effort identity: captured when a valid token is present, anonymous
    // otherwise (a logged-out member hitting a bug can still report it).
    let sender = crate::auth::caller(cfg, client, query, bearer).await.ok();

    ship_feedback(cfg, client, kind, &message, &path, &app, &ua, sender).await
}

/// Ship one feedback event to BetterStack (the frontend logger's sink; no `dt`
/// field, so BetterStack stamps ingest time). A ship failure is UPSTREAM.
#[allow(clippy::too_many_arguments)]
async fn ship_feedback(
    cfg: &Config,
    client: &reqwest::Client,
    kind: &str,
    message: &str,
    path: &str,
    app: &str,
    ua: &str,
    sender: Option<(String, String)>,
) -> Result<(), AppError> {
    let (user_id, email) = sender.unwrap_or_else(|| ("anonymous".into(), String::new()));

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
