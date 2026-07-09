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
