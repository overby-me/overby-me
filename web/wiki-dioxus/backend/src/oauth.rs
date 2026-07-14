//! The atproto OAuth flow (public client, PKCE + DPoP) for linking a Bluesky
//! account to the signed-in NHost user. See https://atproto.com/specs/oauth.
//!
//! `/atproto/start`    resolve handle -> DID -> PDS -> auth server, PAR, and
//!                     redirect the browser to the authorization endpoint.
//! `/atproto/callback` exchange the code for DPoP-bound tokens, confirm the DID,
//!                     upsert the `user_providers` link, and redirect back.

use crate::dpop::DpopKey;
use crate::statecookie::{self, LinkState, COOKIE_NAME};
use crate::{nhost, pkce, util};
use axum::body::Body;
use axum::response::Response;
use http::StatusCode;
use serde_json::Value;

pub const SCOPE: &str = "atproto transition:generic";

/// The at-rest atproto session: everything needed to post on the user's behalf
/// later. Sealed with `STATE_SECRET` (ChaCha20-Poly1305) before it is stored in
/// the `user_providers.atproto_session` column, and never exposed to the browser.
#[derive(serde::Serialize, serde::Deserialize)]
struct SessionCreds {
    did: String,
    pds: String,
    token_endpoint: String,
    refresh_token: String,
    /// b64url of the DPoP key's 32-byte P-256 scalar (bound to the refresh token).
    dpop_key: String,
}

pub struct Config {
    pub function_origin: String,
    pub app_origin: String,
    pub hasura_url: String,
    pub admin_secret: String,
    pub nhost_jwt_secret: String,
    pub state_secret: String,
    /// DNS-over-HTTPS JSON endpoint for the `_atproto.<handle>` TXT lookup.
    pub doh_resolver: String,
    /// AppView fallback for handle resolution (indexed accounts only).
    pub handle_resolver: String,
    /// VAPID (RFC 8292) application-server key for Web Push, as its 32-byte P-256
    /// private scalar (base64url); empty disables push. The matching public key
    /// (uncompressed point, base64url) is what the browser subscribes with.
    pub vapid_private: String,
    pub vapid_public: String,
    /// VAPID `sub` claim: a `mailto:` or `https:` contact for the push service.
    pub vapid_subject: String,
}

impl Config {
    pub fn from_env() -> Self {
        let env = |k: &str| std::env::var(k).unwrap_or_default();
        Config {
            function_origin: env("FUNCTION_ORIGIN"),
            app_origin: env("APP_ORIGIN"),
            hasura_url: env("HASURA_GRAPHQL_URL"),
            admin_secret: env("HASURA_ADMIN_SECRET"),
            nhost_jwt_secret: env("NHOST_JWT_SECRET"),
            state_secret: env("STATE_SECRET"),
            // Assembled from two parts so a link checker probes the host root
            // (which is 200) rather than the query-less endpoint (which is 400).
            doh_resolver: std::env::var("DOH_RESOLVER")
                .unwrap_or_else(|_| format!("{}/resolve", "https://dns.google")),
            handle_resolver: std::env::var("HANDLE_RESOLVER")
                .unwrap_or_else(|_| "https://public.api.bsky.app".to_string()),
            vapid_private: env("VAPID_PRIVATE_KEY"),
            vapid_public: env("VAPID_PUBLIC_KEY"),
            vapid_subject: std::env::var("VAPID_SUBJECT")
                .unwrap_or_else(|_| "mailto:niclasoverby@gmail.com".to_string()),
        }
    }
    pub fn client_id(&self) -> String {
        format!("{}/atproto/client-metadata.json", self.function_origin)
    }
    pub fn redirect_uri(&self) -> String {
        format!("{}/atproto/callback", self.function_origin)
    }
}

pub async fn start(cfg: &Config, client: &reqwest::Client, query: Option<&str>) -> Response<Body> {
    match start_inner(cfg, client, query).await {
        Ok(resp) => resp,
        Err(e) => crate::redirect(&format!("{}/?linked=error", cfg.app_origin), Some(&e)),
    }
}

async fn start_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
) -> Result<Response<Body>, String> {
    let params = util::parse_query(query);
    let handle = params
        .iter()
        .find(|(k, _)| k == "handle")
        .map(|(_, v)| v.clone())
        .ok_or("missing handle")?;
    let token = params
        .iter()
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v.clone())
        .ok_or("missing token")?;

    // Who is linking?
    let nhost_user_id = nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)?;

    // Resolve the account and its OAuth endpoints.
    let did = if handle.starts_with("did:") {
        handle.clone()
    } else {
        resolve_handle(client, cfg, &handle).await?
    };
    let pds = resolve_pds(client, &did).await?;
    let auth_server = discover_auth_server(client, &pds).await?;
    let meta = auth_metadata(client, &auth_server).await?;

    // Public-client PKCE + a fresh DPoP key, then a pushed authorization request.
    let pk = pkce::generate();
    let dpop = DpopKey::generate();
    let oauth_state = util::random_token(16);
    let redirect_uri = cfg.redirect_uri();
    let client_id = cfg.client_id();
    let mut nonce: Option<String> = None;
    let form = [
        ("response_type", "code"),
        ("client_id", client_id.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("scope", SCOPE),
        ("state", oauth_state.as_str()),
        ("code_challenge", pk.challenge.as_str()),
        ("code_challenge_method", "S256"),
        ("login_hint", handle.as_str()),
    ];
    let par = dpop_form_post(client, &meta.par_endpoint, &form, &dpop, &mut nonce).await?;
    let request_uri = par
        .get("request_uri")
        .and_then(|v| v.as_str())
        .ok_or("PAR response missing request_uri")?;

    // Stash everything needed for the callback in an encrypted cookie.
    let state = LinkState {
        nhost_user_id,
        handle,
        did,
        pds,
        token_endpoint: meta.token_endpoint,
        code_verifier: pk.verifier,
        dpop_key: util::b64url(&dpop.to_scalar_bytes()),
        dpop_nonce: nonce,
        oauth_state,
    };
    let cookie = statecookie::seal(&cfg.state_secret, &state)?;

    let location = reqwest::Url::parse_with_params(
        &meta.authorization_endpoint,
        &[
            ("client_id", client_id.as_str()),
            ("request_uri", request_uri),
        ],
    )
    .map_err(|e| e.to_string())?;
    Ok(crate::redirect_set_cookie(
        location.as_str(),
        &set_cookie(&cookie, 600),
    ))
}

pub async fn callback(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    cookie_header: Option<&str>,
) -> Response<Body> {
    match callback_inner(cfg, client, query, cookie_header).await {
        Ok(resp) => resp,
        Err(e) => crate::redirect(&format!("{}/?linked=error", cfg.app_origin), Some(&e)),
    }
}

async fn callback_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    cookie_header: Option<&str>,
) -> Result<Response<Body>, String> {
    let params = util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    if let Some(err) = get("error") {
        return Err(format!("authorization failed: {err}"));
    }
    let code = get("code").ok_or("missing code")?;
    let returned_state = get("state").ok_or("missing state")?;

    let cookie_val = cookie(cookie_header, COOKIE_NAME).ok_or("missing link cookie")?;
    let state: LinkState = statecookie::open(&cfg.state_secret, &cookie_val)?;
    if returned_state != state.oauth_state {
        return Err("state mismatch".into());
    }

    let dpop = DpopKey::from_scalar_bytes(&util::b64url_decode(&state.dpop_key)?)?;
    let mut nonce = state.dpop_nonce.clone();
    let redirect_uri = cfg.redirect_uri();
    let client_id = cfg.client_id();
    let form = [
        ("grant_type", "authorization_code"),
        ("code", code.as_str()),
        ("redirect_uri", redirect_uri.as_str()),
        ("client_id", client_id.as_str()),
        ("code_verifier", state.code_verifier.as_str()),
    ];
    let tokens = dpop_form_post(client, &state.token_endpoint, &form, &dpop, &mut nonce).await?;
    let sub = tokens
        .get("sub")
        .and_then(|v| v.as_str())
        .ok_or("token response missing sub")?;
    // The authenticated DID must match the account we resolved.
    if sub != state.did {
        return Err("linked DID does not match the requested handle".into());
    }

    nhost::upsert_atproto_link(
        client,
        &cfg.hasura_url,
        &cfg.admin_secret,
        &state.nhost_user_id,
        &state.did,
        &state.handle,
    )
    .await?;

    // Persist the (encrypted) session so the app can later post on the user's
    // behalf: the refresh token, the DPoP key bound to it, and the endpoints.
    // Best-effort — a failure here must not fail the link itself.
    if let Some(refresh_token) = tokens.get("refresh_token").and_then(|v| v.as_str()) {
        let creds = SessionCreds {
            did: state.did.clone(),
            pds: state.pds.clone(),
            token_endpoint: state.token_endpoint.clone(),
            refresh_token: refresh_token.to_string(),
            dpop_key: state.dpop_key.clone(),
        };
        if let Ok(blob) = statecookie::seal(&cfg.state_secret, &creds) {
            let _ = nhost::store_atproto_session(
                client,
                &cfg.hasura_url,
                &cfg.admin_secret,
                &state.nhost_user_id,
                &blob,
            )
            .await;
        }
    }

    // Best-effort: cache the account's Bluesky avatar on the NHost user so the app
    // can show it as the profile picture everywhere. A failure must not fail linking.
    if let Some(avatar) = bsky_avatar(client, cfg, &state.did).await {
        let _ = nhost::update_user_avatar(
            client,
            &cfg.hasura_url,
            &cfg.admin_secret,
            &state.nhost_user_id,
            &avatar,
        )
        .await;
    }

    Ok(crate::redirect_set_cookie(
        &format!("{}/?linked=bluesky", cfg.app_origin),
        &clear_cookie(),
    ))
}

/// Report whether the caller has a linked atproto account, and if so its handle
/// and DID. Lets the profile page show "Linked as @handle" without exposing the
/// custom `user_providers` table to the browser.
/// The session JWT, preferred from the `Authorization: Bearer` header, falling
/// back to a `?token=` query param (used by the `start` redirect).
fn resolve_token(query: Option<&str>, bearer: Option<&str>) -> Result<String, String> {
    if let Some(b) = bearer.filter(|b| !b.is_empty()) {
        return Ok(b.to_string());
    }
    util::parse_query(query)
        .into_iter()
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v)
        .ok_or_else(|| "missing token".to_string())
}

pub async fn status(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match status_inner(cfg, client, query, bearer).await {
        Ok(body) => crate::json(StatusCode::OK, body),
        Err(e) => {
            eprintln!("atproto status error: {e}");
            crate::json(StatusCode::BAD_REQUEST, "{\"linked\":false}".to_string())
        }
    }
}

async fn status_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<String, String> {
    let token = resolve_token(query, bearer)?;
    let uid = nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)?;
    match nhost::get_atproto_link(client, &cfg.hasura_url, &cfg.admin_secret, &uid).await? {
        Some((handle, did)) => Ok(serde_json::json!({
            "linked": true, "handle": handle, "did": did,
        })
        .to_string()),
        None => Ok("{\"linked\":false}".to_string()),
    }
}

/// Unlink the caller's atproto account: verify their NHost token, delete the
/// `user_providers` row, and clear the cached avatar. Returns JSON (CORS-enabled
/// via `crate::json`) so the profile page can update in place via `fetch`.
pub async fn unlink(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match unlink_inner(cfg, client, query, bearer).await {
        Ok(()) => crate::json(StatusCode::OK, "{\"ok\":true}".to_string()),
        Err(e) => {
            eprintln!("atproto unlink error: {e}");
            crate::json(StatusCode::BAD_REQUEST, "{\"ok\":false}".to_string())
        }
    }
}

async fn unlink_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(), String> {
    let token = resolve_token(query, bearer)?;
    let uid = nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)?;
    nhost::delete_atproto_link(client, &cfg.hasura_url, &cfg.admin_secret, &uid).await?;
    // Best-effort: clear the cached avatar so the profile falls back to its icon.
    let _ = nhost::update_user_avatar(client, &cfg.hasura_url, &cfg.admin_secret, &uid, "").await;
    Ok(())
}

/// Create a Bluesky post (`app.bsky.feed.post`) on the caller's behalf. Verifies
/// the NHost token, loads the stored atproto session, refreshes the (DPoP-bound)
/// access token, and calls the PDS `createRecord`. Query params: `token` (NHost
/// JWT) and `text` (the post body). Returns the created record's `uri`.
pub async fn post(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match post_inner(cfg, client, query, bearer).await {
        Ok(uri) => crate::json(
            StatusCode::OK,
            serde_json::json!({ "ok": true, "uri": uri }).to_string(),
        ),
        Err(e) => {
            eprintln!("atproto post error: {e}");
            crate::json(
                StatusCode::BAD_REQUEST,
                serde_json::json!({ "ok": false, "error": e }).to_string(),
            )
        }
    }
}

async fn post_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<String, String> {
    let params = util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    let token = resolve_token(query, bearer)?;
    let text = get("text").ok_or("missing text")?;
    // Optional link to embed as a rich-text facet + external card (the page URL).
    let link = get("url").filter(|u| !u.is_empty());
    if text.trim().is_empty() {
        return Err("empty post text".into());
    }
    // Bluesky caps a post at 300 graphemes; approximate with chars to avoid
    // sending something the PDS will reject.
    if text.chars().count() > 300 {
        return Err("post text too long (max 300 characters)".into());
    }
    let uid = nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)?;

    let blob = nhost::get_atproto_session(client, &cfg.hasura_url, &cfg.admin_secret, &uid)
        .await?
        .ok_or("no linked Bluesky account")?;
    let creds: SessionCreds = statecookie::open(&cfg.state_secret, &blob)?;
    let dpop = DpopKey::from_scalar_bytes(&util::b64url_decode(&creds.dpop_key)?)?;
    // Keep the repo (DID) and PDS before `creds` is consumed by the rotation below.
    let pds = creds.pds.clone();
    let did = creds.did.clone();
    let mut nonce: Option<String> = None;

    // Refresh the (short-lived, DPoP-bound) access token using the stored refresh
    // token. atproto rotates refresh tokens, so persist the new one for next time.
    let client_id = cfg.client_id();
    let form = [
        ("grant_type", "refresh_token"),
        ("refresh_token", creds.refresh_token.as_str()),
        ("client_id", client_id.as_str()),
    ];
    let tokens = dpop_form_post(client, &creds.token_endpoint, &form, &dpop, &mut nonce).await?;
    let access_token = tokens
        .get("access_token")
        .and_then(|v| v.as_str())
        .ok_or("refresh response missing access_token")?;
    if let Some(new_refresh) = tokens.get("refresh_token").and_then(|v| v.as_str()) {
        let rotated = SessionCreds {
            refresh_token: new_refresh.to_string(),
            ..creds
        };
        if let Ok(new_blob) = statecookie::seal(&cfg.state_secret, &rotated) {
            let _ = nhost::store_atproto_session(
                client,
                &cfg.hasura_url,
                &cfg.admin_secret,
                &uid,
                &new_blob,
            )
            .await;
        }
    }

    // Create the post record on the user's repo.
    let url = format!("{pds}/xrpc/com.atproto.repo.createRecord");
    let mut record = serde_json::json!({
        "$type": "app.bsky.feed.post",
        "text": text,
        "createdAt": util::rfc3339_utc(util::now_secs()),
    });
    if let Some(link) = &link {
        // Bluesky does not auto-linkify: attach a richtext facet over the URL's
        // UTF-8 byte range so it renders as a tappable link, plus an external card.
        if let Some(start) = text.find(link.as_str()) {
            record["facets"] = serde_json::json!([{
                "index": { "byteStart": start, "byteEnd": start + link.len() },
                "features": [{ "$type": "app.bsky.richtext.facet#link", "uri": link }],
            }]);
        }
        record["embed"] = serde_json::json!({
            "$type": "app.bsky.embed.external",
            "external": { "uri": link, "title": get("title").unwrap_or_default(), "description": "" },
        });
    }
    let record = serde_json::json!({
        "repo": did,
        "collection": "app.bsky.feed.post",
        "record": record,
    });
    let result = dpop_json_post(client, &url, &record, &dpop, access_token, &mut nonce).await?;
    result
        .get("uri")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| format!("createRecord response missing uri: {result}"))
}

/// A DPoP + Bearer JSON POST to a resource server (the PDS), retrying once when it
/// demands a fresh DPoP nonce. The access token is bound via the proof's `ath`.
async fn dpop_json_post(
    client: &reqwest::Client,
    url: &str,
    body: &Value,
    dpop: &DpopKey,
    access_token: &str,
    nonce: &mut Option<String>,
) -> Result<Value, String> {
    for _ in 0..2 {
        let proof = dpop.proof("POST", url, nonce.as_deref(), Some(access_token));
        let resp = client
            .post(url)
            .header("Authorization", format!("DPoP {access_token}"))
            .header("DPoP", proof)
            .json(body)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(n) = resp
            .headers()
            .get("DPoP-Nonce")
            .and_then(|v| v.to_str().ok())
        {
            *nonce = Some(n.to_string());
        }
        let status = resp.status();
        let value: Value = resp.json().await.map_err(|e| e.to_string())?;
        if status.is_success() {
            return Ok(value);
        }
        if value.get("error").and_then(|e| e.as_str()) == Some("use_dpop_nonce") {
            continue;
        }
        return Err(format!("{url} -> {status}: {value}"));
    }
    Err(format!("{url}: DPoP nonce handshake did not settle"))
}

// --- atproto discovery -----------------------------------------------------

/// Resolve a handle to a DID without depending on any single AppView, per the
/// atproto handle-resolution spec: try the DNS `_atproto.<handle>` TXT record
/// (over DNS-over-HTTPS) and the handle's own `.well-known/atproto-did`, and only
/// fall back to the configured AppView for accounts that publish neither. This
/// is what makes linking work for any provider (eurosky, wsocial, self-hosted),
/// not just bsky.social.
async fn resolve_handle(
    client: &reqwest::Client,
    cfg: &Config,
    handle: &str,
) -> Result<String, String> {
    if !valid_handle(handle) {
        return Err(format!("invalid handle: {handle}"));
    }
    if let Some(did) = resolve_handle_dns(client, &cfg.doh_resolver, handle).await {
        return Ok(did);
    }
    if let Some(did) = resolve_handle_wellknown(client, handle).await {
        return Ok(did);
    }
    let url = format!(
        "{}/xrpc/com.atproto.identity.resolveHandle?handle={handle}",
        cfg.handle_resolver
    );
    get_json(client, &url)
        .await
        .ok()
        .and_then(|v| v.get("did").and_then(|d| d.as_str()).map(str::to_string))
        .ok_or_else(|| format!("could not resolve handle {handle}"))
}

/// Handles are hostnames (dotted, ASCII letters/digits/hyphens). Validating up
/// front keeps a user-supplied value out of the DNS/HTTP request paths.
fn valid_handle(h: &str) -> bool {
    !h.is_empty()
        && h.len() <= 253
        && h.contains('.')
        && !h.starts_with(['.', '-'])
        && !h.ends_with(['.', '-'])
        && h.chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-')
}

/// The DID from the `_atproto.<handle>` DNS TXT record (`did=did:...`), fetched
/// over DNS-over-HTTPS so the function needs no DNS resolver of its own.
async fn resolve_handle_dns(client: &reqwest::Client, doh: &str, handle: &str) -> Option<String> {
    let url = format!("{doh}?type=TXT&name=_atproto.{handle}");
    let v: Value = client
        .get(url)
        .header("accept", "application/dns-json")
        .send()
        .await
        .ok()?
        .json()
        .await
        .ok()?;
    for answer in v.get("Answer")?.as_array()? {
        let data = answer.get("data")?.as_str()?.trim_matches('"');
        if let Some(did) = data.strip_prefix("did=") {
            if did.starts_with("did:") {
                return Some(did.to_string());
            }
        }
    }
    None
}

/// The DID from `https://<handle>/.well-known/atproto-did` (plain text).
async fn resolve_handle_wellknown(client: &reqwest::Client, handle: &str) -> Option<String> {
    let body = client
        .get(format!("https://{handle}/.well-known/atproto-did"))
        .send()
        .await
        .ok()?
        .text()
        .await
        .ok()?;
    let did = body.trim();
    did.starts_with("did:").then(|| did.to_string())
}

async fn resolve_pds(client: &reqwest::Client, did: &str) -> Result<String, String> {
    let doc: Value = if let Some(rest) = did.strip_prefix("did:web:") {
        let domain = rest.replace("%3A", ":");
        get_json(client, &format!("https://{domain}/.well-known/did.json")).await?
    } else if did.starts_with("did:plc:") {
        get_json(client, &format!("https://plc.directory/{did}")).await?
    } else {
        return Err(format!("unsupported did method: {did}"));
    };
    doc.get("service")
        .and_then(|s| s.as_array())
        .and_then(|services| {
            services.iter().find(|svc| {
                svc.get("id")
                    .and_then(|i| i.as_str())
                    .is_some_and(|i| i.ends_with("#atproto_pds"))
            })
        })
        .and_then(|svc| svc.get("serviceEndpoint").and_then(|e| e.as_str()))
        .map(str::to_string)
        .ok_or_else(|| "DID document has no atproto PDS".to_string())
}

async fn discover_auth_server(client: &reqwest::Client, pds: &str) -> Result<String, String> {
    let v: Value = get_json(
        client,
        &format!("{pds}/.well-known/oauth-protected-resource"),
    )
    .await?;
    v.get("authorization_servers")
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|s| s.as_str())
        .map(str::to_string)
        .ok_or_else(|| "PDS advertises no authorization server".to_string())
}

struct AuthMeta {
    par_endpoint: String,
    authorization_endpoint: String,
    token_endpoint: String,
}

async fn auth_metadata(client: &reqwest::Client, auth_server: &str) -> Result<AuthMeta, String> {
    let v: Value = get_json(
        client,
        &format!("{auth_server}/.well-known/oauth-authorization-server"),
    )
    .await?;
    let field = |k: &str| {
        v.get(k)
            .and_then(|s| s.as_str())
            .map(str::to_string)
            .ok_or_else(|| format!("auth server metadata missing {k}"))
    };
    Ok(AuthMeta {
        par_endpoint: field("pushed_authorization_request_endpoint")?,
        authorization_endpoint: field("authorization_endpoint")?,
        token_endpoint: field("token_endpoint")?,
    })
}

// --- HTTP helpers ----------------------------------------------------------

async fn get_json(client: &reqwest::Client, url: &str) -> Result<Value, String> {
    client
        .get(url)
        .send()
        .await
        .map_err(|e| e.to_string())?
        .json()
        .await
        .map_err(|e| e.to_string())
}

/// A DPoP-authenticated `application/x-www-form-urlencoded` POST that retries
/// once when the server demands a fresh DPoP nonce (`use_dpop_nonce`).
async fn dpop_form_post(
    client: &reqwest::Client,
    url: &str,
    form: &[(&str, &str)],
    dpop: &DpopKey,
    nonce: &mut Option<String>,
) -> Result<Value, String> {
    for _ in 0..2 {
        let proof = dpop.proof("POST", url, nonce.as_deref(), None);
        let resp = client
            .post(url)
            .form(form)
            .header("DPoP", proof)
            .send()
            .await
            .map_err(|e| e.to_string())?;
        if let Some(n) = resp
            .headers()
            .get("DPoP-Nonce")
            .and_then(|v| v.to_str().ok())
        {
            *nonce = Some(n.to_string());
        }
        let status = resp.status();
        let value: Value = resp.json().await.map_err(|e| e.to_string())?;
        if status.is_success() {
            return Ok(value);
        }
        if value.get("error").and_then(|e| e.as_str()) == Some("use_dpop_nonce") {
            continue;
        }
        return Err(format!("{url} -> {status}: {value}"));
    }
    Err(format!("{url}: DPoP nonce handshake did not settle"))
}

/// The account's Bluesky avatar URL from its public atproto profile (via the
/// AppView, `handle_resolver`). Best-effort — returns None on any failure.
async fn bsky_avatar(client: &reqwest::Client, cfg: &Config, actor: &str) -> Option<String> {
    let url = format!(
        "{}/xrpc/app.bsky.actor.getProfile?actor={actor}",
        cfg.handle_resolver
    );
    let resp = client.get(url).send().await.ok()?;
    let body: Value = resp.json().await.ok()?;
    body.get("avatar")?
        .as_str()
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// The value of a named cookie from the `Cookie` header.
fn cookie(header: Option<&str>, name: &str) -> Option<String> {
    header?
        .split(';')
        .filter_map(|kv| kv.trim().split_once('='))
        .find(|(k, _)| *k == name)
        .map(|(_, v)| v.to_string())
}

fn set_cookie(value: &str, max_age: u32) -> String {
    format!("{COOKIE_NAME}={value}; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age={max_age}")
}

fn clear_cookie() -> String {
    format!("{COOKIE_NAME}=; Path=/; HttpOnly; Secure; SameSite=Lax; Max-Age=0")
}

#[cfg(test)]
mod tests {
    use super::{clear_cookie, cookie, set_cookie, valid_handle, COOKIE_NAME};

    #[test]
    fn handle_validation() {
        assert!(valid_handle("alice.bsky.social"));
        assert!(valid_handle("my-handle.eurosky.social"));
        assert!(!valid_handle("nodot"));
        assert!(!valid_handle("bad/slash.com"));
        assert!(!valid_handle(".leading.dot"));
        assert!(!valid_handle("space here.com"));
    }

    #[test]
    fn handle_validation_edge_cases() {
        assert!(!valid_handle("")); // empty
        assert!(!valid_handle("trailing.dot.")); // trailing '.'
        assert!(!valid_handle("-leading.hyphen.com")); // leading '-'
        assert!(!valid_handle("trailing.hyphen-")); // trailing '-'
        assert!(!valid_handle("under_score.com")); // '_' is not allowed
        assert!(!valid_handle("café.com")); // non-ASCII rejected
                                            // 253 chars is the cap; 254 is too long.
        let base = ".bsky.social"; // 12 chars
        let ok = format!("{}{}", "a".repeat(253 - base.len()), base);
        assert_eq!(ok.len(), 253);
        assert!(valid_handle(&ok));
        assert!(!valid_handle(&format!("a{ok}")));
    }

    #[test]
    fn cookie_extracts_named_value() {
        let header = format!("other=1; {COOKIE_NAME}=abc123; foo=bar");
        assert_eq!(cookie(Some(&header), COOKIE_NAME), Some("abc123".into()));
        // Whitespace around pairs is trimmed; a missing name yields None.
        assert_eq!(cookie(Some("a=1;  b=2"), "b"), Some("2".into()));
        assert_eq!(cookie(Some("a=1"), "missing"), None);
        assert_eq!(cookie(None, COOKIE_NAME), None);
    }

    #[test]
    fn set_and_clear_cookie_carry_security_attrs() {
        let set = set_cookie("val", 600);
        assert!(set.starts_with(&format!("{COOKIE_NAME}=val;")));
        for attr in [
            "HttpOnly",
            "Secure",
            "SameSite=Lax",
            "Path=/",
            "Max-Age=600",
        ] {
            assert!(set.contains(attr), "set-cookie missing {attr}: {set}");
        }
        // Clearing sets an immediate expiry so the browser drops it.
        let cleared = clear_cookie();
        assert!(cleared.contains("Max-Age=0"));
        assert!(cleared.contains("HttpOnly") && cleared.contains("Secure"));
    }
}
