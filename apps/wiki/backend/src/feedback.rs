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
//!   POST /feedback?kind=bug|feature|other|crash&message=&path=&app=&commit=&ua=
//!        (Authorization: Bearer <jwt> optional; captures the sender when present)

use crate::error::AppError;
use crate::oauth::Config;
use axum::response::Response;
use http::StatusCode;
use serde_json::{json, Value};

/// Cap on the message as it ARRIVES (matches the client-side maxlength), so a
/// runaway paste cannot bloat a node or an ingest event.
const MAX_MESSAGE: usize = 4000;

/// Cap on the message as STORED, which has to be far larger, because
/// symbolication multiplies it: one wasm frame becomes every function inlined
/// into it, so a stack that arrives as twenty lines can leave as a hundred.
/// Capping the result at [`MAX_MESSAGE`] cut the resolved stack off partway
/// through — losing precisely the outer frames that name the component.
const MAX_STORED: usize = 32_000;

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

    let message = get("message")
        .map(|m| m.trim().to_string())
        .filter(|m| !m.is_empty())
        .ok_or(AppError::BadRequest("missing message".into()))?;
    // Bound what arrives, BEFORE resolving it: the cap exists to stop a runaway
    // paste, and applying it afterwards would instead have trimmed the work.
    let message = clamp(message, MAX_MESSAGE);
    // A crash report carries the panic's stack in its message, so it gets the
    // same treatment as a shipped log entry — otherwise the one report a reader
    // deliberately chose to send would be the least readable thing in the sink.
    // Ordinary feedback has no wasm frames and passes through untouched.
    let message = crate::symbolicate::resolve_stack(client, &cfg.app_origin, &message).await;
    let message = clamp(message, MAX_STORED);
    let kind = match get("kind").as_deref() {
        Some("bug") => "bug",
        Some("feature") => "feature",
        // The app reporting its own death, which the dialog never sends: it
        // carries a stack rather than an account of what happened, and reads
        // differently in the feedback app for that reason.
        Some("crash") => "crash",
        // A failure the app noticed and could only describe to the user as
        // "something went wrong". Distinct from a crash: the app survived, and
        // distinct from `bug`, which is a person's account of what they saw.
        Some("error") => "error",
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
            tracing::error!("feedback node insert failed ({e}); shipping to the log sink instead");
            ship_feedback(cfg, client, &report, sender, &e).await
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

    // A crash that has been seen before becomes a count on the row that is
    // already there, rather than another row. The node is the durable record —
    // the log sink keeps three days — so "how often" and "how many people" have
    // to survive here or not at all.
    // Automatic reports group; a person's account of what happened does not. The
    // stored field stays `crashDigest` so rows filed before this keep matching.
    let digest = matches!(report.kind, "crash" | "error").then(|| crash_digest(report.message));
    if let Some(digest) = &digest {
        if let Some((id, data)) = find_crash(cfg, client, &root_id, digest).await? {
            return bump_crash(cfg, client, &id, data, owner_id).await;
        }
    }

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
    // First sighting of this crash. `seen` and `reporters` are what later
    // sightings add to; `updatedAt` (a database trigger keeps it) is when it was
    // last seen, so nothing here has to carry a clock.
    if let Some(digest) = digest {
        object["data"]["crashDigest"] = Value::from(digest);
        object["data"]["seen"] = Value::from(1);
        object["data"]["reporters"] = json!([owner_id.unwrap_or(ANONYMOUS)]);
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

/// Stands in for a reporter with no account, so anonymous sightings count once
/// rather than once each.
const ANONYMOUS: &str = "anonymous";

/// Identify a crash by WHERE it happened, not by the exact bytes of its report.
///
/// Built from the panic text plus the source locations of the resolved frames,
/// deliberately ignoring wasm offsets, function indices and asset URLs. Those
/// change with every build, so hashing the whole message would start a fresh row
/// at each deploy and the count would reset exactly when a recurring crash
/// becomes most interesting.
///
/// A report with nothing resolved falls back to the whole message, which at
/// least groups identical unresolved reports together.
fn crash_digest(message: &str) -> String {
    let mut material = String::new();
    let mut resolved = 0usize;
    for line in message.lines() {
        let trimmed = line.trim_start();
        if let Some(rest) = trimmed.strip_prefix("at ") {
            // Keep only the trailing `(file:line)`; the function name carries
            // monomorphised type parameters that shift between builds.
            if let Some(open) = rest.rfind(" (") {
                if rest.ends_with(')') {
                    material.push_str(&rest[open + 2..rest.len() - 1]);
                    material.push('\n');
                    resolved += 1;
                }
            }
        } else if !trimmed.contains("wasm-function[") && !trimmed.contains("://") {
            // The panic itself and its message; everything else on these lines
            // is an engine artefact.
            material.push_str(trimmed);
            material.push('\n');
        }
    }
    if resolved == 0 {
        material = message.to_string();
    }
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in material.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// The existing node for this crash, if there is one.
async fn find_crash(
    cfg: &Config,
    client: &reqwest::Client,
    root_id: &str,
    digest: &str,
) -> Result<Option<(String, Value)>, String> {
    let body = json!({
        "query": "query($p: uuid!, $d: jsonb!) { \
                  nodes(where: { parentId: { _eq: $p }, mimeId: { _eq: \"wiki/feedback\" }, \
                                 data: { _contains: $d } }, limit: 1) { id data } }",
        "variables": { "p": root_id, "d": { "crashDigest": digest } },
    });
    let value = hasura(cfg, client, &body).await?;
    let Some(node) = value
        .get("data")
        .and_then(|d| d.get("nodes"))
        .and_then(|n| n.as_array())
        .and_then(|a| a.first())
    else {
        return Ok(None);
    };
    let id = node
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("crash node has no id")?
        .to_string();
    Ok(Some((id, node.get("data").cloned().unwrap_or(json!({})))))
}

/// Record another sighting: one more occurrence, and one more reporter if this
/// one is new. `updatedAt` moves on its own (a database trigger), which is what
/// "last seen" reads from.
async fn bump_crash(
    cfg: &Config,
    client: &reqwest::Client,
    node_id: &str,
    mut data: Value,
    owner_id: Option<&str>,
) -> Result<(), String> {
    let seen = data.get("seen").and_then(|s| s.as_u64()).unwrap_or(1) + 1;
    data["seen"] = Value::from(seen);

    let who = owner_id.unwrap_or(ANONYMOUS);
    let mut reporters: Vec<String> = data
        .get("reporters")
        .and_then(|r| r.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect()
        })
        .unwrap_or_default();
    if !reporters.iter().any(|r| r == who) {
        reporters.push(who.to_string());
    }
    data["reporters"] = json!(reporters);

    let body = json!({
        "query": "mutation($id: uuid!, $data: jsonb!) { \
                  updateNode(pk_columns: { id: $id }, _set: { data: $data }) { id } }",
        "variables": { "id": node_id, "data": data },
    });
    let value = hasura(cfg, client, &body).await?;
    value
        .get("data")
        .and_then(|d| d.get("updateNode"))
        .filter(|n| !n.is_null())
        .map(|_| ())
        .ok_or_else(|| "updateNode returned no row".to_string())
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

/// Cut `text` to at most `max` bytes without splitting a character.
///
/// `String::truncate` panics when the index lands mid-character, and a Danish
/// message reaches that on any æ, ø or å sitting across the limit — so the cap
/// that exists to keep a report small could instead have taken the request down
/// with it.
fn clamp(mut text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text
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
/// sink; no `dt` field, so BetterStack stamps ingest time). A ship failure is
/// UPSTREAM.
///
/// This is only ever reached when the `wiki/feedback` node could NOT be written,
/// and it says so loudly — `level: error`, its own source, and `why` carrying the
/// reason. It used to arrive as an ordinary `feedback` event at `info`, which
/// made the one case worth noticing indistinguishable from the routine one: the
/// reader was told "Reported", the report existed, and it was nowhere in the
/// feedback app with nothing to say why.
async fn ship_feedback(
    cfg: &Config,
    client: &reqwest::Client,
    report: &Report<'_>,
    sender: Option<(String, String)>,
    why: &str,
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
        tracing::error!(
            "feedback [{kind}] from {user_id} NOT FILED ({why}): {message} (path={path})"
        );
        return Ok(());
    }

    let entry = json!({
        "level": "error",
        "source": "feedback-not-filed",
        "message": format!("feedback [{kind}] NOT FILED as a wiki/feedback node ({why}): {message}"),
        "feedback": {
            "kind": kind,
            "message": message,
            "path": path,
            "app_version": app,
            "commit": commit,
            "user_agent": ua,
            "user_id": user_id,
            "email": email,
            // What to search for when a report is missing from the feedback app.
            "filed": false,
            "insert_error": why,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_same_crash_digests_the_same_across_builds() {
        // The same crash, reported from two builds: different wasm offsets,
        // different asset hashes, same source locations. It must fold into one
        // row, or the count resets at every deploy — exactly when a recurring
        // crash becomes worth counting.
        let from_build_a = "panicked at src/components/error.rs:20:5:\n\
             Triggered test panic\n\
             @https://radikal.wiki/assets/wiki-dxhAAA.js:1:42944\n\
             @…_bg-dxhAAA.wasm:wasm-function[6175]:0x376d02\n    \
             at ZoomableImage (src/components/widgets/image.rs:11)\n";
        let from_build_b = "panicked at src/components/error.rs:20:5:\n\
             Triggered test panic\n\
             @https://radikal.wiki/assets/wiki-dxhBBB.js:1:51001\n\
             6175@wasm-function[6175]\n    \
             at ZoomableImage (src/components/widgets/image.rs:11)\n";
        assert_eq!(crash_digest(from_build_a), crash_digest(from_build_b));
    }

    #[test]
    fn a_different_crash_digests_differently() {
        let one = "panicked at a.rs:1:1: boom\n    at Foo (src/one.rs:5)\n";
        let other = "panicked at a.rs:1:1: boom\n    at Foo (src/two.rs:5)\n";
        assert_ne!(crash_digest(one), crash_digest(other));
        // And the panic text itself distinguishes two crashes in the same place.
        let third = "panicked at a.rs:1:1: different message\n    at Foo (src/one.rs:5)\n";
        assert_ne!(crash_digest(one), crash_digest(third));
    }

    #[test]
    fn an_unresolved_report_still_groups_with_its_twin() {
        // Nothing resolved, so there are no source locations to key on. Falling
        // back to the whole message at least folds identical reports.
        let raw = "panicked at a.rs:1:1: boom\n6175@wasm-function[6175]\n";
        assert_eq!(crash_digest(raw), crash_digest(raw));
        assert_ne!(
            crash_digest(raw),
            crash_digest("panicked at b.rs:2:2: boom\n")
        );
    }

    #[test]
    fn short_messages_are_left_alone() {
        assert_eq!(clamp("hej".to_string(), 4000), "hej");
    }

    #[test]
    fn clamping_never_splits_a_character() {
        // "ø" is two bytes, so a limit of 2 lands inside it. Truncating there is
        // a panic, not a short string.
        assert_eq!(clamp("aøb".to_string(), 2), "a");
        // And a limit on the boundary keeps the whole character.
        assert_eq!(clamp("aøb".to_string(), 3), "aø");
    }

    #[test]
    fn a_resolved_stack_survives_the_stored_cap() {
        // Roughly what symbolication produces from a full crash report: one wasm
        // frame becomes every function inlined into it. This used to be cut off
        // partway through, losing the outer frames that name the component.
        let resolved = "    at drop_in_place<dioxus_primitives::switch::SwitchPropsWithOwner> \
                        (core/src/ptr/mod.rs:805)\n"
            .repeat(120);
        assert!(resolved.len() > MAX_MESSAGE, "test stack is not big enough");
        assert_eq!(clamp(resolved.clone(), MAX_STORED), resolved);
    }
}
