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

/// The URL of a stored file's bytes, with no credentials in it.
///
/// The storage service authenticates by `Authorization` header only — a `?token=`
/// in the query is decorative, and the URL resolves as an ANONYMOUS request. That
/// went unnoticed while every file was world-readable; once files are readable
/// only through the node that references them, such a request is refused.
///
/// So this is for FETCHES, which can set the header (see
/// [`crate::components::loader::use_file_object_url`]). An `<img>` or `<iframe>`
/// cannot set one and must use [`presigned_file_url`] instead.
///
/// This is the ONE place file URLs are built, so the NHost-Storage -> AppView
/// swap at cutover is a change here rather than a scavenger hunt across call
/// sites; `nhost::storage_url()` is the exact seam that repoints.
pub fn file_url(file_id: &str) -> String {
    format!("{}/files/{file_id}", crate::nhost::storage_url())
}

/// The bytes of a stored file, fetched with the session token in the header.
///
/// For the readers that parse a file in Rust rather than hand a URL to an
/// element: they can send an `Authorization` header, so they should, and then
/// there is no URL to expire and no token in the DOM.
///
/// Deliberately NOT a presigned URL. One of those lasts thirty seconds, which is
/// fine for a fetch that happens now and wrong for anything that might happen
/// again — a reader remounted after a detour through another viewer would find
/// its link already dead.
pub async fn file_bytes(file_id: &str, token: &str) -> Result<Vec<u8>, String> {
    if file_id.is_empty() {
        return Err("no file".into());
    }
    let resp = reqwest::Client::new()
        .get(file_url(file_id))
        .bearer_auth(token)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("storage said {}", resp.status()));
    }
    resp.bytes()
        .await
        .map(|b| b.to_vec())
        .map_err(|e| e.to_string())
}

/// Render one Windows metafile (EMF/EMF+/WMF) to PNG.
///
/// The bytes go up because the caller already has them: it opened the package
/// to find the picture. The backend does the drawing because the renderer is
/// ~400 KB gzipped and this wasm bundle is already the heaviest thing a
/// delegate downloads.
pub async fn render_metafile(bytes: &[u8], token: &str) -> Result<(Vec<u8>, String), String> {
    let resp = reqwest::Client::new()
        .post(format!("{BACKEND_URL}/office/metafile"))
        .bearer_auth(token)
        .header("content-type", "application/octet-stream")
        .body(bytes.to_vec())
        .send()
        .await
        .map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("backend said {}", resp.status()));
    }
    // SVG when the backend could draw the records, PNG when only its rasteriser
    // could; the caller just needs to label the data url correctly.
    let mime = resp
        .headers()
        .get("content-type")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("image/png")
        .split(';')
        .next()
        .unwrap_or("image/png")
        .trim()
        .to_string();
    resp.bytes()
        .await
        .map(|b| (b.to_vec(), mime))
        .map_err(|e| e.to_string())
}

/// A backend-hosted URL for a document, for the Microsoft Office web viewer.
///
/// That viewer fetches the document from MICROSOFT'S servers, so neither a
/// header nor a 30-second presigned URL is any use to it: it needs a link that
/// is reachable without a session and stays reachable. The backend mints one,
/// having first checked with the caller's own token that they may read the file,
/// and serves the bytes on it from storage.
///
/// The link is a capability — whoever holds it reads that document until it
/// expires — and using the viewer at all means sending the document to
/// Microsoft. Both are inherent to this viewer, not to this function.
pub async fn office_embed_url(file_id: &str, token: &str) -> Option<String> {
    let resp = reqwest::Client::new()
        .get(format!("{BACKEND_URL}/office/sign?fileId={file_id}"))
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("url")?.as_str().map(str::to_string)
}

/// A time-limited URL for a stored file that carries its own authorization, for
/// the element `src`s that cannot send a header.
///
/// The presign REQUEST is authenticated with the session token; the URL it hands
/// back is not, and it expires. That is the shape `<img>`, `<iframe>`, `<video>`
/// and a download `href` need: no header, and no standing public read on the
/// bucket. Streaming media keeps working too, since the browser fetches it
/// directly and can issue range requests (a blob URL would have to buffer the
/// whole file first).
pub async fn presigned_file_url(file_id: &str, token: &str) -> Option<String> {
    let resp = reqwest::Client::new()
        .get(format!(
            "{}/files/{file_id}/presignedurl",
            crate::nhost::storage_url()
        ))
        .bearer_auth(token)
        .send()
        .await
        .ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let body: serde_json::Value = resp.json().await.ok()?;
    body.get("url")?.as_str().map(str::to_string)
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

thread_local! {
    /// Reports already filed by this tab, so a failure that repeats on every
    /// render files once.
    static REPORTED: std::cell::RefCell<std::collections::HashSet<String>> =
        std::cell::RefCell::new(std::collections::HashSet::new());
}

/// The most automatic reports one tab may file, however many things go wrong.
///
/// The server merges by digest, so repeats of ONE failure are already a count on
/// one row rather than many rows. This is the other axis: a build that is broken
/// in twenty different ways should not have every device narrating all twenty.
const MAX_AUTO_REPORTS: usize = 5;

/// File a failure the app could only describe to the user as "something went
/// wrong", so it reaches the feedback app instead of only the log sink.
///
/// Deliberately quiet about its own failure: this runs on the error path, and an
/// error report that reports its own failure to report is a loop, not a feature.
pub async fn report_error(access_token: Option<&str>, message: &str, path: &str) {
    let fresh = REPORTED.with(|seen| {
        let mut seen = seen.borrow_mut();
        seen.len() < MAX_AUTO_REPORTS && seen.insert(message.to_string())
    });
    if !fresh {
        return;
    }
    let ua = web_sys::window()
        .map(|w| w.navigator().user_agent().unwrap_or_default())
        .unwrap_or_default();
    let url = format!(
        "{BACKEND_URL}/feedback?kind=error&message={}&path={}&app={}&commit={}&ua={}",
        js_sys::encode_uri_component(message),
        js_sys::encode_uri_component(path),
        js_sys::encode_uri_component(env!("CARGO_PKG_VERSION")),
        js_sys::encode_uri_component(crate::build_info::COMMIT),
        js_sys::encode_uri_component(&ua),
    );
    let client = reqwest::Client::new();
    let mut req = client.post(&url);
    if let Some(token) = access_token {
        req = req.bearer_auth(token);
    }
    let _ = req.send().await;
}
