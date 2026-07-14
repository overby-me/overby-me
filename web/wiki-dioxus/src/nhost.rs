use serde::{Deserialize, Serialize};

const NHOST_SUBDOMAIN: &str = "pgvhpsenoifywhuxnybq";
const NHOST_REGION: &str = "eu-central-1";

pub fn auth_url() -> String {
    format!("https://{NHOST_SUBDOMAIN}.auth.{NHOST_REGION}.nhost.run/v1")
}

pub fn graphql_url() -> String {
    format!("https://{NHOST_SUBDOMAIN}.hasura.{NHOST_REGION}.nhost.run/v1/graphql")
}

pub fn storage_url() -> String {
    format!("https://{NHOST_SUBDOMAIN}.storage.{NHOST_REGION}.nhost.run/v1")
}

/// The RadikalWiki backend (a Rust axum service on Scaleway Serverless Containers,
/// fr-par). Hosts the atproto OAuth flow for linking a Bluesky account:
/// `GET /atproto/start?handle=&token=` begins linking and redirects back with
/// `?linked=success|error`.
pub const BACKEND_URL: &str =
    "https://wikidioxusd0caa45e-wiki-backend.functions.fnc.fr-par.scw.cloud";

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

#[derive(Debug, Serialize)]
pub struct SignInRequest {
    pub email: String,
    pub password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignUpRequest {
    pub email: String,
    pub password: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub options: Option<SignUpOptions>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct SignUpOptions {
    pub display_name: String,
}

#[derive(Debug, Serialize)]
pub struct ResetPasswordRequest {
    pub email: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ChangePasswordRequest {
    pub new_password: String,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct RefreshTokenRequest {
    pub refresh_token: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AuthSession {
    pub access_token: String,
    pub refresh_token: String,
    /// Seconds until the access token expires (NHost default 900).
    #[serde(default)]
    pub access_token_expires_in: Option<i64>,
    pub user: Option<NhostUser>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct NhostUser {
    pub id: String,
    pub email: Option<String>,
    pub display_name: Option<String>,
    pub avatar_url: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct NhostSignInResponse {
    pub session: Option<AuthSession>,
    pub error: Option<NhostError>,
}

#[derive(Debug, Deserialize, Clone)]
pub struct NhostError {
    pub status: Option<u16>,
    pub error: Option<String>,
    pub message: Option<String>,
}

impl std::fmt::Display for NhostError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}",
            self.message
                .as_deref()
                .unwrap_or(self.error.as_deref().unwrap_or("Unknown error"))
        )
    }
}

pub async fn sign_in(email: &str, password: &str) -> Result<AuthSession, NhostError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/signin/email-password", auth_url()))
        .json(&SignInRequest {
            email: email.to_lowercase(),
            password: password.to_string(),
        })
        .send()
        .await
        .map_err(|e| NhostError {
            status: None,
            error: Some("network_error".to_string()),
            message: Some(e.to_string()),
        })?;

    let body: NhostSignInResponse = resp.json().await.map_err(|e| NhostError {
        status: None,
        error: Some("parse_error".to_string()),
        message: Some(e.to_string()),
    })?;

    if let Some(err) = body.error {
        return Err(err);
    }

    body.session.ok_or(NhostError {
        status: None,
        error: Some("no_session".to_string()),
        message: Some("No session returned".to_string()),
    })
}

pub async fn sign_up(email: &str, password: &str, display_name: &str) -> Result<(), NhostError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/signup/email-password", auth_url()))
        .json(&SignUpRequest {
            email: email.to_lowercase(),
            password: password.to_string(),
            options: Some(SignUpOptions {
                display_name: display_name.to_string(),
            }),
        })
        .send()
        .await
        .map_err(|e| NhostError {
            status: None,
            error: Some("network_error".to_string()),
            message: Some(e.to_string()),
        })?;

    let body: serde_json::Value = resp.json().await.map_err(|e| NhostError {
        status: None,
        error: Some("parse_error".to_string()),
        message: Some(e.to_string()),
    })?;

    if let Some(error) = body.get("error") {
        return Err(
            serde_json::from_value::<NhostError>(error.clone()).unwrap_or(NhostError {
                status: None,
                error: Some("unknown".to_string()),
                message: Some("Registration failed".to_string()),
            }),
        );
    }

    Ok(())
}

pub async fn reset_password(email: &str) -> Result<(), NhostError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/user/password/reset", auth_url()))
        .json(&ResetPasswordRequest {
            email: email.to_lowercase(),
        })
        .send()
        .await
        .map_err(|e| NhostError {
            status: None,
            error: Some("network_error".to_string()),
            message: Some(e.to_string()),
        })?;

    if !resp.status().is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(
            serde_json::from_value::<NhostError>(body).unwrap_or(NhostError {
                status: None,
                error: Some("unknown".to_string()),
                message: Some("Password reset failed".to_string()),
            }),
        );
    }

    Ok(())
}

/// Re-send the sign-up verification email for an unverified account. Used from
/// the login screen when a sign-in fails with `unverified-user`.
pub async fn send_verification_email(email: &str) -> Result<(), NhostError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/user/email/send-verification-email", auth_url()))
        .json(&ResetPasswordRequest {
            email: email.to_lowercase(),
        })
        .send()
        .await
        .map_err(|e| NhostError {
            status: None,
            error: Some("network_error".to_string()),
            message: Some(e.to_string()),
        })?;

    if !resp.status().is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(
            serde_json::from_value::<NhostError>(body).unwrap_or(NhostError {
                status: None,
                error: Some("unknown".to_string()),
                message: Some("Failed to send verification email".to_string()),
            }),
        );
    }

    Ok(())
}

pub async fn change_password(access_token: &str, new_password: &str) -> Result<(), NhostError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/user/password", auth_url()))
        .bearer_auth(access_token)
        .json(&ChangePasswordRequest {
            new_password: new_password.to_string(),
        })
        .send()
        .await
        .map_err(|e| NhostError {
            status: None,
            error: Some("network_error".to_string()),
            message: Some(e.to_string()),
        })?;

    if !resp.status().is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(
            serde_json::from_value::<NhostError>(body).unwrap_or(NhostError {
                status: None,
                error: Some("unknown".to_string()),
                message: Some("Password change failed".to_string()),
            }),
        );
    }

    Ok(())
}

pub async fn refresh_session(refresh_token: &str) -> Result<AuthSession, NhostError> {
    let client = reqwest::Client::new();
    let resp = client
        .post(format!("{}/token", auth_url()))
        .json(&RefreshTokenRequest {
            refresh_token: refresh_token.to_string(),
        })
        .send()
        .await
        .map_err(|e| NhostError {
            status: None,
            error: Some("network_error".to_string()),
            message: Some(e.to_string()),
        })?;

    // A non-2xx response means the refresh token is invalid/expired; surface it
    // as an auth error (not a parse error) so callers can sign the user out.
    let status = resp.status();
    if !status.is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(
            serde_json::from_value::<NhostError>(body).unwrap_or(NhostError {
                status: Some(status.as_u16()),
                error: Some("invalid-refresh-token".to_string()),
                message: Some("Session expired".to_string()),
            }),
        );
    }

    resp.json().await.map_err(|e| NhostError {
        status: None,
        error: Some("parse_error".to_string()),
        message: Some(e.to_string()),
    })
}

/// Whether a refresh error means the session is unrecoverable (bad/expired
/// refresh token) rather than a transient network blip or a request bug. Only
/// an explicit rejection (401/403, or the `refresh-token` error code) counts.
/// A 404/400 signals a client mistake and must not force the user to log out.
pub fn is_auth_error(err: &NhostError) -> bool {
    matches!(err.status, Some(401) | Some(403))
        || err
            .error
            .as_deref()
            .map(|e| e.contains("refresh-token"))
            .unwrap_or(false)
}

pub fn sign_out() {
    // Simply clear the session locally; NHost tokens will expire naturally
}

/// One entry of the storage `POST /files` response (`processedFiles[]`). The
/// server sniffs and records the real `mimeType`, so it is authoritative for
/// the `type` we persist on the `wiki/file` node's data.
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct UploadedFile {
    pub id: String,
    pub mime_type: Option<String>,
}

#[derive(Debug, Deserialize)]
struct UploadResponse {
    #[serde(rename = "processedFiles")]
    processed_files: Vec<UploadedFile>,
}

/// Upload a file to NHost storage (`POST /files`, multipart field `file[]`,
/// mirroring `nhost.storage.upload`). Returns the created file's metadata; the
/// caller stores `{ fileId: id, type: mimeType }` on a `wiki/file` node.
pub async fn upload_file(
    access_token: Option<&str>,
    bytes: Vec<u8>,
    file_name: &str,
    content_type: &str,
) -> Result<UploadedFile, NhostError> {
    // An empty browser File.type would make `mime_str` reject the part; fall
    // back to octet-stream and let the server sniff the real type.
    let ctype = if content_type.trim().is_empty() {
        "application/octet-stream"
    } else {
        content_type
    };
    let part = reqwest::multipart::Part::bytes(bytes)
        .file_name(file_name.to_string())
        .mime_str(ctype)
        .map_err(|e| NhostError {
            status: None,
            error: Some("bad_mime".to_string()),
            message: Some(e.to_string()),
        })?;
    let form = reqwest::multipart::Form::new().part("file[]", part);

    let client = reqwest::Client::new();
    let mut req = client
        .post(format!("{}/files", storage_url()))
        .multipart(form);
    if let Some(token) = access_token {
        req = req.bearer_auth(token);
    }
    let resp = req.send().await.map_err(|e| NhostError {
        status: None,
        error: Some("network_error".to_string()),
        message: Some(e.to_string()),
    })?;

    let status = resp.status();
    if !status.is_success() {
        let body: serde_json::Value = resp.json().await.unwrap_or_default();
        return Err(
            serde_json::from_value::<NhostError>(body).unwrap_or(NhostError {
                status: Some(status.as_u16()),
                error: Some("upload_failed".to_string()),
                message: Some("File upload failed".to_string()),
            }),
        );
    }

    let body: UploadResponse = resp.json().await.map_err(|e| NhostError {
        status: None,
        error: Some("parse_error".to_string()),
        message: Some(e.to_string()),
    })?;
    body.processed_files.into_iter().next().ok_or(NhostError {
        status: None,
        error: Some("no_file".to_string()),
        message: Some("Upload returned no file".to_string()),
    })
}
