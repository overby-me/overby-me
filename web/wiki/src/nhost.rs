use serde::{Deserialize, Serialize};

const NHOST_SUBDOMAIN: &str = "pgvhpsenoifywhuxnybq";
const NHOST_REGION: &str = "eu-central-1";

pub fn auth_url() -> String {
    format!("https://{NHOST_SUBDOMAIN}.auth.{NHOST_REGION}.nhost.run/v1")
}

/// The Hasura GraphQL endpoint. Overridable at compile time with
/// `WIKI_GRAPHQL_URL` so a dev/staging build can point at a local or staging
/// backend; unset, it is the NHost project URL as before.
pub fn graphql_url() -> String {
    match option_env!("WIKI_GRAPHQL_URL") {
        Some(url) => url.to_string(),
        None => format!("https://{NHOST_SUBDOMAIN}.hasura.{NHOST_REGION}.nhost.run/v1/graphql"),
    }
}

pub fn storage_url() -> String {
    format!("https://{NHOST_SUBDOMAIN}.storage.{NHOST_REGION}.nhost.run/v1")
}

// The backend-API client (endpoints that survive the rewrite) lives in
// `backend_api.rs`; this module keeps only the NHost auth + storage glue that
// dies with NHost at cutover.

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

/// The error carried by an auth response, in whichever of the two shapes the
/// service used.
///
/// Hasura Auth reports a failure FLAT: `{"status":409,"error":"email-already-in
/// -use","message":"Email already in use"}`. Sign-in and sign-up instead read it
/// as a nested `{"error":{...}}`, so every code they were matching on
/// (`email-already-in-use`, `unverified-user`) arrived as "unknown" or as a
/// serde parse failure, and the screen fell back to its catch-all: a wrong
/// password and an address already registered read the same. The three other
/// endpoints here already parse the flat form, which is what the service sends.
///
/// Both are accepted, so this cannot break whichever one any endpoint returns.
fn error_from_body(body: &serde_json::Value) -> Option<NhostError> {
    match body.get("error") {
        // Nested: {"error": {"status": …, "error": "…", "message": "…"}}
        Some(nested) if nested.is_object() => serde_json::from_value(nested.clone()).ok(),
        // Flat: the code sits directly under `error`, its detail alongside.
        Some(serde_json::Value::String(code)) => Some(NhostError {
            status: body
                .get("status")
                .and_then(|s| s.as_u64())
                .map(|s| s as u16),
            error: Some(code.clone()),
            message: body
                .get("message")
                .and_then(|m| m.as_str())
                .map(str::to_string),
        }),
        _ => None,
    }
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

    let body: serde_json::Value = resp.json().await.map_err(|e| NhostError {
        status: None,
        error: Some("parse_error".to_string()),
        message: Some(e.to_string()),
    })?;

    if let Some(err) = error_from_body(&body) {
        return Err(err);
    }

    let session = body.get("session").cloned().unwrap_or_default();
    serde_json::from_value::<AuthSession>(session).map_err(|_| NhostError {
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

    if let Some(err) = error_from_body(&body) {
        return Err(err);
    }
    // An `error` key in a shape neither reader understands still means failure.
    if body.get("error").is_some() {
        return Err(NhostError {
            status: None,
            error: Some("unknown".to_string()),
            message: Some("Registration failed".to_string()),
        });
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

#[cfg(test)]
mod tests {
    use super::*;

    /// Hasura Auth reports failures flat. Reading them as a nested object left
    /// sign-in and sign-up unable to see the code they matched on, so an address
    /// already in use and an unverified account both fell through to the screen's
    /// catch-all instead of their own message.
    #[test]
    fn flat_auth_error_is_read() {
        let body = serde_json::json!({
            "status": 409,
            "message": "Email already in use",
            "error": "email-already-in-use",
        });
        let err = error_from_body(&body).expect("flat error should be read");
        assert_eq!(err.error.as_deref(), Some("email-already-in-use"));
        assert_eq!(err.message.as_deref(), Some("Email already in use"));
        assert_eq!(err.status, Some(409));
    }

    /// The nested shape the client originally assumed still parses, so accepting
    /// the flat one cannot regress an endpoint that answers this way.
    #[test]
    fn nested_auth_error_is_read() {
        let body = serde_json::json!({
            "error": { "status": 401, "message": "Incorrect email or password",
                       "error": "invalid-email-password" },
        });
        let err = error_from_body(&body).expect("nested error should be read");
        assert_eq!(err.error.as_deref(), Some("invalid-email-password"));
        assert_eq!(err.status, Some(401));
    }

    /// A successful sign-in carries a session and no error: it must not be
    /// mistaken for a failure.
    #[test]
    fn success_body_has_no_error() {
        let body = serde_json::json!({
            "session": { "accessToken": "t", "refreshToken": "r" },
            "mfa": null,
        });
        assert!(error_from_body(&body).is_none());
        assert!(error_from_body(&serde_json::json!({ "error": null })).is_none());
    }
}
