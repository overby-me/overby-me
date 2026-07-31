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
        // A stack arrives either as one newline-joined string (older bundles) or
        // as one frame per array element (newer ones, which read far better in
        // Better Stack). Reading only the string form silently stopped resolving
        // EVERY report the moment the frontend switched, which is how a build
        // shipped with raw `wasm-function[6719]` frames and no sign of why.
        let Some((joined, as_frames)) = entry.get("stack").and_then(stack_input) else {
            continue;
        };
        let resolved = crate::symbolicate::resolve_stack(client, &cfg.app_origin, &joined).await;
        if let Some(obj) = entry.as_object_mut() {
            obj.insert("stack".into(), stack_output(resolved, as_frames));
        }
    }
}

/// The stack to resolve, and whether the answer should be a list of frames.
///
/// `None` when there is nothing to resolve, so an entry without a stack (most of
/// them) costs a map lookup and no work.
fn stack_input(stack: &serde_json::Value) -> Option<(String, bool)> {
    let (joined, as_frames) = match stack {
        serde_json::Value::String(s) => (s.clone(), false),
        serde_json::Value::Array(items) => (
            items
                .iter()
                .filter_map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join("\n"),
            true,
        ),
        _ => return None,
    };
    (!joined.trim().is_empty()).then_some((joined, as_frames))
}

/// Answer in the shape the stack arrived in. Resolving expands a frame into the
/// calls inlined into it, so a list comes back longer than it went in.
fn stack_output(resolved: String, as_frames: bool) -> serde_json::Value {
    if as_frames {
        serde_json::Value::Array(
            resolved
                .lines()
                .map(|l| serde_json::Value::String(l.to_string()))
                .collect(),
        )
    } else {
        serde_json::Value::String(resolved)
    }
}

#[cfg(test)]
mod stack_shape_tests {
    use super::*;
    use serde_json::json;

    /// A stack sent as a list of frames must still be symbolicated.
    ///
    /// This is the regression: the reader took only the string form, so when the
    /// frontend started sending one frame per element — which reads far better in
    /// Better Stack — every report silently stopped being resolved and shipped
    /// raw `wasm-function[6719]:0x2a31ba` frames instead.
    #[test]
    fn a_stack_sent_as_frames_is_still_resolved() {
        let (joined, as_frames) = stack_input(&json!([
            "at foo (a.rs:1)",
            "@https://x/assets/app_bg-dxhabc.wasm:wasm-function[42]:0x1",
        ]))
        .expect("a list of frames is resolvable");
        assert!(as_frames);
        assert_eq!(
            joined,
            "at foo (a.rs:1)\n@https://x/assets/app_bg-dxhabc.wasm:wasm-function[42]:0x1",
            "frames are joined for the resolver, which works on whole stacks"
        );
        // ...and it comes back as a list, not as one line.
        assert_eq!(
            stack_output("at foo (a.rs:1)\nat bar (b.rs:2)".into(), as_frames),
            json!(["at foo (a.rs:1)", "at bar (b.rs:2)"])
        );
    }

    /// The older string form still works: tabs left open on a previous bundle
    /// keep sending it, and their reports matter most (they are the ones from
    /// people who have not reloaded).
    #[test]
    fn a_stack_sent_as_one_string_round_trips_as_a_string() {
        let (joined, as_frames) = stack_input(&json!("at foo (a.rs:1)\nat bar (b.rs:2)")).unwrap();
        assert!(!as_frames);
        assert_eq!(joined, "at foo (a.rs:1)\nat bar (b.rs:2)");
        assert_eq!(
            stack_output(joined, as_frames),
            json!("at foo (a.rs:1)\nat bar (b.rs:2)")
        );
    }

    /// Nothing to resolve costs nothing.
    #[test]
    fn an_entry_without_a_usable_stack_is_skipped() {
        assert!(stack_input(&serde_json::Value::Null).is_none());
        assert!(stack_input(&json!("")).is_none());
        assert!(stack_input(&json!("   \n ")).is_none());
        assert!(stack_input(&json!([])).is_none());
        assert!(stack_input(&json!(["", "  "])).is_none());
        assert!(stack_input(&json!({"frames": []})).is_none());
    }
}
