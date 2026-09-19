//! The frontend's log shipping. Its error and panic logger POSTs batched JSON
//! entries here, and they are forwarded to Better Stack. Carried over from the
//! interim backend (`backend/src/logs.rs`).
//!
//! Through the backend rather than straight from the browser, for two reasons:
//!   1. CORS. Better Stack answers preflight with `Access-Control-Allow-Headers:
//!      *`, which per the Fetch spec does NOT cover `Authorization`, so browsers
//!      are starting to block a direct cross-origin ship with a Bearer token.
//!   2. Secrecy. The write-only ingest token stays on the server, out of the
//!      shipped wasm bundle.
//!
//! `POST /log`, a JSON entry or an array of them. It takes NO session: what is
//! most worth shipping happens to people who could not sign in.

use crate::AppState;
use crate::xrpc::{err, invalid};
use axum::Json;
use axum::body::Body;
use axum::extract::State;
use axum::http::StatusCode;
use axum::http::header::CONTENT_TYPE;
use axum::response::{IntoResponse, Response};

/// Cap on the forwarded batch, so a runaway client cannot bloat an ingest call.
const MAX_BODY: usize = 256 * 1024;

pub async fn ingest(State(state): State<AppState>, body: Body) -> Response {
    let ok = || (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response();
    // No sink configured: accept and drop. The frontend logger is best-effort,
    // and a 200 keeps it from retrying a batch it can never deliver.
    if state.config.betterstack_token.is_empty() {
        return ok();
    }
    let Ok(bytes) = axum::body::to_bytes(body, MAX_BODY).await else {
        return err(
            StatusCode::PAYLOAD_TOO_LARGE,
            "LogTooLarge",
            "the batch is too large",
        );
    };
    // Parsed before it is forwarded, so that this cannot be used to relay
    // arbitrary bytes to the ingest host.
    let Ok(mut batch) = serde_json::from_slice::<serde_json::Value>(&bytes) else {
        return invalid("the batch is not JSON");
    };

    // Resolve the wasm frames while the entry is passing through: a stack of
    // `wasm-function[4231]:0x1d4c0` is unreadable in Better Stack, and the sink
    // has no way to make sense of it later.
    symbolicate_batch(&state, &mut batch).await;

    let host = &state.config.betterstack_host;
    let url = if host.contains("://") {
        host.clone()
    } else {
        format!("https://{host}/")
    };
    let shipped = async {
        state
            .http
            .post(&url)?
            .bearer_auth(state.config.betterstack_token.expose())
            .header(CONTENT_TYPE, "application/json")
            .body(serde_json::to_vec(&batch)?)
            .send()
            .await?
            .error_for_status()?;
        Ok::<_, Box<dyn std::error::Error + Send + Sync>>(())
    };
    match shipped.await {
        Ok(()) => ok(),
        Err(e) => {
            tracing::warn!("log ship failed: {e}");
            err(
                StatusCode::BAD_GATEWAY,
                "UpstreamError",
                "the log sink refused the batch",
            )
        }
    }
}

/// Rewrite the `stack` of every entry in a batch, in place.
///
/// The frontend sends either one entry or an array of them, and only entries
/// that actually carry a wasm stack cost anything: `resolve_stack` returns the
/// input untouched when it finds no bundle hash, so ordinary JS stacks and
/// stackless entries pass straight through.
async fn symbolicate_batch(state: &AppState, batch: &mut serde_json::Value) {
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
        let resolved =
            crate::symbolicate::resolve_stack(&state.http, &state.config.app_origin, &joined).await;
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
mod tests {
    use super::*;
    use crate::config::Secret;
    use crate::router;
    use crate::xrpc::tests::seeded_state;
    use axum::http::{HeaderMap, Request};
    use serde_json::json;
    use std::sync::{Arc, Mutex};
    use tower::ServiceExt;

    type Received = Arc<Mutex<Vec<(String, serde_json::Value)>>>;

    /// A stand-in for the ingest host: keeps the credential and the batch of
    /// every call it gets.
    async fn sink() -> (String, Received) {
        let received = Received::default();
        let keep = received.clone();
        let app = axum::Router::new().fallback(
            move |headers: HeaderMap, Json(batch): Json<serde_json::Value>| async move {
                let credential = headers["authorization"]
                    .to_str()
                    .expect("ascii")
                    .to_string();
                keep.lock().expect("received").push((credential, batch));
                StatusCode::ACCEPTED
            },
        );
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind");
        let url = format!("http://{}/", listener.local_addr().expect("addr"));
        tokio::spawn(async move {
            let _ = axum::serve(listener, app).await;
        });
        (url, received)
    }

    async fn ship(state: &AppState, body: Vec<u8>) -> StatusCode {
        let req = Request::builder()
            .method("POST")
            .uri("/log")
            .body(Body::from(body))
            .expect("request");
        router(state.clone())
            .oneshot(req)
            .await
            .expect("response")
            .status()
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn a_batch_is_forwarded_with_the_servers_token_and_no_session() {
        let (url, received) = sink().await;
        let mut state = seeded_state().await;
        state.config.betterstack_host = url;
        state.config.betterstack_token = Secret::new("ingest-token");

        let batch = json!([
            {"level": "error", "message": "boom", "stack": ["at plain.js:1:2"]},
            {"level": "warn", "message": "no stack at all"},
        ]);
        assert_eq!(
            ship(&state, batch.to_string().into_bytes()).await,
            StatusCode::OK
        );
        let got = received.lock().expect("received").clone();
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].0, "Bearer ingest-token");
        assert_eq!(
            got[0].1, batch,
            "a stack with no wasm frame passes through untouched"
        );

        assert_eq!(
            ship(&state, b"not json".to_vec()).await,
            StatusCode::BAD_REQUEST
        );
        let huge = format!("[\"{}\"]", "x".repeat(MAX_BODY));
        assert_eq!(
            ship(&state, huge.into_bytes()).await,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            received.lock().expect("received").len(),
            1,
            "a refused batch was relayed"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn with_no_sink_a_batch_is_accepted_and_dropped() {
        let state = seeded_state().await;
        assert_eq!(
            ship(&state, b"[{\"message\":\"x\"}]".to_vec()).await,
            StatusCode::OK
        );
    }

    /// A stack sent as a list of frames must still be symbolicated.
    ///
    /// This is the regression: the reader took only the string form, so when the
    /// frontend started sending one frame per element every report silently
    /// stopped being resolved and shipped raw `wasm-function[6719]:0x2a31ba`
    /// frames instead.
    #[test]
    fn a_stack_sent_as_frames_is_still_resolved() {
        let (joined, as_frames) = stack_input(&json!([
            "at foo (a.rs:1)",
            "@/assets/wiki_bg-dxhabc.wasm:wasm-function[42]:0x1",
        ]))
        .expect("a list of frames is resolvable");
        assert!(as_frames);
        assert_eq!(
            joined, "at foo (a.rs:1)\n@/assets/wiki_bg-dxhabc.wasm:wasm-function[42]:0x1",
            "frames are joined for the resolver, which works on whole stacks"
        );
        assert_eq!(
            stack_output("at foo (a.rs:1)\nat bar (b.rs:2)".into(), as_frames),
            json!(["at foo (a.rs:1)", "at bar (b.rs:2)"]),
            "and it comes back as a list, not as one line"
        );
    }

    /// The older string form still works: tabs left open on a previous bundle
    /// keep sending it, and their reports matter most (they are the ones from
    /// people who have not reloaded).
    #[test]
    fn a_stack_sent_as_one_string_round_trips_as_a_string() {
        let (joined, as_frames) =
            stack_input(&json!("at foo (a.rs:1)\nat bar (b.rs:2)")).expect("a stack");
        assert!(!as_frames);
        assert_eq!(
            stack_output(joined, as_frames),
            json!("at foo (a.rs:1)\nat bar (b.rs:2)")
        );
    }

    #[test]
    fn an_entry_without_a_usable_stack_is_skipped() {
        for nothing in [
            serde_json::Value::Null,
            json!(""),
            json!("   \n "),
            json!([]),
            json!(["", "  "]),
            json!({"frames": []}),
        ] {
            assert!(stack_input(&nothing).is_none(), "{nothing}");
        }
    }
}
