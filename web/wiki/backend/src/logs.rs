//! Client-side log ingestion proxy. The frontend error/panic logger POSTs its
//! batched JSON entries here and we forward them to Better Stack server-side.
//!
//! Two reasons this goes through the backend rather than the browser shipping
//! straight to Better Stack:
//!   1. CORS — Better Stack answers preflight with `Access-Control-Allow-Headers:
//!      *`, which per the Fetch spec does NOT cover `Authorization`, so browsers
//!      are starting to block a direct cross-origin ship with a Bearer token.
//!   2. Secrecy — the write-only ingest token stays on the server, out of the
//!      shipped wasm bundle.
//!
//!   POST /log   body: a JSON array of log entries (no auth required; best-effort)

use crate::error::AppError;
use crate::oauth::Config;
use axum::{body::Body, extract::Request, response::Response};
use http::StatusCode;

/// Cap on the forwarded batch, so a runaway client cannot bloat an ingest call.
const MAX_BODY: usize = 256 * 1024;

pub async fn ingest(cfg: &Config, client: &reqwest::Client, req: Request<Body>) -> Response {
    match ingest_inner(cfg, client, req).await {
        Ok(()) => crate::json(StatusCode::OK, "{\"ok\":true}".into()),
        Err(e) => e.respond("log"),
    }
}

async fn ingest_inner(
    cfg: &Config,
    client: &reqwest::Client,
    req: Request<Body>,
) -> Result<(), AppError> {
    // No sink configured: accept and drop (the frontend logger is best-effort,
    // and a 200 keeps it from retrying a batch it can never deliver).
    if cfg.betterstack_token.is_empty() {
        return Ok(());
    }
    let bytes = axum::body::to_bytes(req.into_body(), MAX_BODY)
        .await
        .map_err(|_| AppError::BadRequest("log body too large".into()))?;
    // Validate it parses as JSON before forwarding, so this proxy can't be used
    // to relay arbitrary bytes to the ingest host.
    let mut batch: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|_| AppError::BadRequest("log body not JSON".into()))?;

    // Resolve the wasm frames while the entry is passing through: a stack of
    // `wasm-function[4231]:0x1d4c0` is unreadable in Better Stack, and the sink
    // has no way to make sense of it later.
    symbolicate_batch(cfg, client, &mut batch).await;

    let url = format!("https://{}/", cfg.betterstack_host);
    client
        .post(&url)
        .bearer_auth(&cfg.betterstack_token)
        .json(&batch)
        .send()
        .await
        .map_err(|e| AppError::Upstream(format!("log ship failed: {e}")))?;
    Ok(())
}

/// Rewrite the `stack` of every entry in a batch, in place.
///
/// The frontend sends either one entry or an array of them, and only entries
/// that actually carry a wasm stack cost anything — `resolve_stack` returns the
/// input untouched when it finds no bundle hash, so ordinary JS stacks and
/// stackless entries pass straight through.
async fn symbolicate_batch(cfg: &Config, client: &reqwest::Client, batch: &mut serde_json::Value) {
    let entries: Vec<&mut serde_json::Value> = match batch {
        serde_json::Value::Array(items) => items.iter_mut().collect(),
        single => vec![single],
    };
    for entry in entries {
        let Some(stack) = entry.get("stack").and_then(|s| s.as_str()) else {
            continue;
        };
        let resolved = crate::symbolicate::resolve_stack(client, &cfg.app_origin, stack).await;
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("stack".into(), serde_json::Value::String(resolved));
        }
    }
}
