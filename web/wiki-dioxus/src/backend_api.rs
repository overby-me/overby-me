//! Client for the RadikalWiki backend API: the endpoints that SURVIVE the atproto
//! rewrite (the axum backend already serves these exact paths and evolves into the
//! AppView). Split out of `nhost.rs` so the cutover deletion boundary is exact:
//! `nhost.rs` (NHost auth + storage glue) is deleted wholesale, this module is kept
//! and repointed. Also home of the Bluesky public-AppView typeahead and the
//! atproto link types, which likewise outlive NHost.

use serde::Deserialize;

/// The RadikalWiki backend (a Rust axum service on Scaleway Serverless Containers,
/// fr-par). Hosts the atproto OAuth flow for linking a Bluesky account:
/// `GET /atproto/start?handle=&token=` begins linking and redirects back with
/// `?linked=success|error`.
///
/// Overridable at compile time with `WIKI_BACKEND_URL` (same pattern as
/// `logging.rs`), so a dev/staging build can point at a local backend or a future
/// AppView without editing code; unset, the constant is bit-identical to before.
pub const BACKEND_URL: &str = match option_env!("WIKI_BACKEND_URL") {
    Some(url) => url,
    None => "https://wikidioxusd0caa45e-wiki-backend.functions.fnc.fr-par.scw.cloud",
};

/// The URL that starts the atproto (Bluesky) account-linking flow for `handle`,
/// authenticated by the session JWT (both are URL-safe: a domain and base64url).
/// The backend redirects back with `?linked=success|error`.
pub fn atproto_start_url(handle: &str, token: &str) -> String {
    format!("{BACKEND_URL}/atproto/start?handle={handle}&token={token}")
}

/// The URL to fetch a stored file's bytes, with the session JWT in the `?token=`
/// query (the backend accepts it there for both `<img src>` and `fetch`). This
/// is the ONE place file-blob URLs are built, so the NHost-Storage -> AppView
/// blob swap at cutover is a one-line change to this function's body rather than
/// a scavenger hunt across component call sites. It still routes through
/// `nhost::storage_url()` today; that reference is the exact seam the cutover
/// repoints (NHost Storage dies, the AppView serves blobs from a different path).
pub fn file_url(file_id: &str, token: &str) -> String {
    format!(
        "{}/files/{file_id}?token={token}",
        crate::nhost::storage_url()
    )
}

/// The caller's Bluesky (atproto) link status, from the backend `/atproto/status`
/// endpoint. Defaults to "not linked" so a failed lookup just shows the link form.
#[derive(Deserialize, Clone, PartialEq, Debug, Default)]
pub struct AtprotoLink {
    #[serde(default)]
    pub linked: bool,
    #[serde(default)]
    pub handle: String,
    #[serde(default)]
    pub did: String,
}

/// Ask the backend whether the caller has a linked Bluesky account (and its
/// handle). The session JWT goes in the `Authorization` header (not the URL).
/// Returns "not linked" on any error.
pub async fn atproto_status(token: &str) -> AtprotoLink {
    let url = format!("{BACKEND_URL}/atproto/status");
    match reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
    {
        Ok(resp) => resp.json::<AtprotoLink>().await.unwrap_or_default(),
        Err(_) => AtprotoLink::default(),
    }
}

/// A Bluesky account suggestion for the link-handle typeahead.
#[derive(Clone, PartialEq, Deserialize, Default)]
pub struct BskyActor {
    pub handle: String,
    #[serde(rename = "displayName", default)]
    pub display_name: String,
    #[serde(default)]
    pub avatar: String,
}

/// Typeahead search for Bluesky accounts matching `query`, via the public AppView
/// (no auth needed). Powers the preview of matching handles in the link-account
/// field. Returns an empty list on any error or a too-short query.
pub async fn search_bsky_actors(query: &str) -> Vec<BskyActor> {
    let q = query.trim();
    if q.len() < 2 {
        return Vec::new();
    }
    let url = "https://public.api.bsky.app/xrpc/app.bsky.actor.searchActorsTypeahead";
    match reqwest::Client::new()
        .get(url)
        .query(&[("q", q), ("limit", "6")])
        .send()
        .await
    {
        Ok(resp) => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| serde_json::from_value(v.get("actors").cloned().unwrap_or_default()).ok())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

/// Unlink the caller's Bluesky account via the backend. Returns true on success.
pub async fn atproto_unlink(token: &str) -> bool {
    let url = format!("{BACKEND_URL}/atproto/unlink");
    reqwest::Client::new()
        .get(url)
        .bearer_auth(token)
        .send()
        .await
        .map(|r| r.status().is_success())
        .unwrap_or(false)
}

/// Post `text` to the caller's linked Bluesky account via the backend (which holds
/// the encrypted session). `link`/`title` become a tappable facet + link card when
/// non-empty. Ok on success; Err carries the backend's message (e.g. `no linked
/// Bluesky account`) for the UI to surface.
pub async fn atproto_post(token: &str, text: &str, link: &str, title: &str) -> Result<(), String> {
    let url = format!("{BACKEND_URL}/atproto/post");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&[("text", text), ("url", link), ("title", title)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("post failed")
            .to_string())
    }
}

#[derive(Deserialize)]
struct RosterRow {
    name: String,
    email: String,
}

/// Parse a bulk-import roster (.xlsx) via the backend, which keeps calamine/zip
/// out of the wasm bundle. Returns (name, email) pairs; empty on any error.
pub async fn parse_roster(token: Option<&str>, bytes: Vec<u8>) -> Vec<(String, String)> {
    let url = format!("{BACKEND_URL}/roster/parse");
    let mut req = reqwest::Client::new().post(&url).body(bytes);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let Ok(resp) = req.send().await else {
        return vec![];
    };
    match resp.json::<Vec<RosterRow>>().await {
        Ok(rows) => rows.into_iter().map(|r| (r.name, r.email)).collect(),
        Err(_) => vec![],
    }
}

/// Cast an anonymous ballot on a SECRET poll via the backend: the vote node is
/// inserted with no owner_id, and a has-voted marker enforces one vote/member.
/// Ok on success; Err("already voted") or a message otherwise.
pub async fn vote_cast_secret(
    token: &str,
    poll: &str,
    context: Option<&str>,
    choices: &[usize],
) -> Result<(), String> {
    let url = format!("{BACKEND_URL}/vote/cast");
    let choices_str = choices
        .iter()
        .map(|c| c.to_string())
        .collect::<Vec<_>>()
        .join(",");
    let mut params = vec![("poll", poll.to_string()), ("choices", choices_str)];
    if let Some(c) = context {
        params.push(("context", c.to_string()));
    }
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&params)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if body.get("ok").and_then(|v| v.as_bool()) == Some(true) {
        Ok(())
    } else {
        Err(body
            .get("error")
            .and_then(|v| v.as_str())
            .unwrap_or("cast failed")
            .to_string())
    }
}

/// Whether the caller has already voted on a secret poll (the anonymous vote
/// nodes carry no owner_id, so the has-voted marker lives backend-side).
pub async fn vote_status(token: &str, poll: &str) -> bool {
    let url = format!("{BACKEND_URL}/vote/status");
    match reqwest::Client::new()
        .get(&url)
        .bearer_auth(token)
        .query(&[("poll", poll)])
        .send()
        .await
    {
        Ok(resp) => resp
            .json::<serde_json::Value>()
            .await
            .ok()
            .and_then(|v| v.get("voted").and_then(|b| b.as_bool()))
            .unwrap_or(false),
        Err(_) => false,
    }
}

/// Register this browser's Web Push subscription with the backend (keyed to the
/// caller's user + email so a context owner can notify its members).
pub async fn push_subscribe(
    token: &str,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
) -> Result<(), String> {
    let url = format!("{BACKEND_URL}/push/subscribe");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&[("endpoint", endpoint), ("p256dh", p256dh), ("auth", auth)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("subscribe failed: {}", resp.status()))
    }
}

/// Drop this browser's push subscription from the backend.
pub async fn push_unsubscribe(token: &str, endpoint: &str) -> Result<(), String> {
    let url = format!("{BACKEND_URL}/push/unsubscribe");
    reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&[("endpoint", endpoint)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    Ok(())
}

/// Ask the backend to push a notification to the active members of `context`
/// (only a context owner may). Returns (recipients, sent). Best-effort: errors
/// are surfaced but never block the action that triggered the notification.
pub async fn push_notify(
    token: &str,
    context: &str,
    title: &str,
    body: &str,
    link: &str,
) -> Result<(u64, u64), String> {
    let url = format!("{BACKEND_URL}/push/notify");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&[
            ("context", context),
            ("title", title),
            ("body", body),
            ("url", link),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
        Ok((
            v.get("recipients").and_then(|n| n.as_u64()).unwrap_or(0),
            v.get("sent").and_then(|n| n.as_u64()).unwrap_or(0),
        ))
    } else {
        Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("notify failed")
            .to_string())
    }
}

/// Ask the backend to push a "someone commented on your content" notification to
/// the author of `parent` (the node being commented on). The backend gates this
/// on the caller being an active member of the node's context. Best-effort.
pub async fn push_reply(
    token: &str,
    parent: &str,
    title: &str,
    body: &str,
    link: &str,
) -> Result<(u64, u64), String> {
    let url = format!("{BACKEND_URL}/push/reply");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&[
            ("parent", parent),
            ("title", title),
            ("body", body),
            ("url", link),
        ])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
        Ok((
            v.get("recipients").and_then(|n| n.as_u64()).unwrap_or(0),
            v.get("sent").and_then(|n| n.as_u64()).unwrap_or(0),
        ))
    } else {
        Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("reply notify failed")
            .to_string())
    }
}

/// Claim a rostered membership via its secret token (from a `?claim=` link),
/// binding it to the caller's account regardless of the roster email. Returns the
/// context (group/event) id so the app can navigate there.
pub async fn claim_membership(token: &str, claim_token: &str) -> Result<String, String> {
    let url = format!("{BACKEND_URL}/members/claim");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&[("claim", claim_token)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
        Ok(v.get("context")
            .and_then(|c| c.as_str())
            .unwrap_or_default()
            .to_string())
    } else {
        Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("claim failed")
            .to_string())
    }
}

/// Submit a feedback / bug report / feature request. Ships to the backend
/// `/feedback` endpoint, which forwards it to the team's observability sink.
/// `token` is the session JWT when signed in (so the backend can capture the
/// sender), None when anonymous. `kind` is bug/feature/other; `path`,
/// `app_version` and `user_agent` are auto-captured context.
pub async fn submit_feedback(
    token: Option<&str>,
    kind: &str,
    message: &str,
    path: &str,
    app_version: &str,
    user_agent: &str,
) -> Result<(), String> {
    let url = format!("{BACKEND_URL}/feedback");
    let mut req = reqwest::Client::new().post(&url).query(&[
        ("kind", kind),
        ("message", message),
        ("path", path),
        ("app", app_version),
        ("ua", user_agent),
    ]);
    if let Some(t) = token {
        req = req.bearer_auth(t);
    }
    let resp = req.send().await.map_err(|e| e.to_string())?;
    if resp.status().is_success() {
        Ok(())
    } else {
        Err(format!("feedback failed: {}", resp.status()))
    }
}

/// Owner-only: fetch a member's secret claim token so the owner can share a
/// `?claim=<token>` link with the rostered person (whose email may not match).
pub async fn member_claim_link(token: &str, member_id: &str) -> Result<String, String> {
    let url = format!("{BACKEND_URL}/members/claim-link");
    let resp = reqwest::Client::new()
        .post(&url)
        .bearer_auth(token)
        .query(&[("member", member_id)])
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if v.get("ok").and_then(|b| b.as_bool()) == Some(true) {
        Ok(v.get("token")
            .and_then(|t| t.as_str())
            .unwrap_or_default()
            .to_string())
    } else {
        Err(v
            .get("error")
            .and_then(|e| e.as_str())
            .unwrap_or("claim link failed")
            .to_string())
    }
}
