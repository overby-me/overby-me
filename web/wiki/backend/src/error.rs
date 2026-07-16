//! Typed backend errors with correct HTTP status codes, replacing the
//! stringly-typed `Result<_, String>` chain whose failures all collapsed to a
//! 400 in `error_json`. The enum and its one to-Response mapping are backbone
//! code the AppView keeps wholesale; only the message TEXTS are interim.

use axum::{body::Body, response::Response};
use http::StatusCode;
use serde_json::json;

#[derive(Debug, Clone, PartialEq)]
pub enum AppError {
    /// 401: missing or invalid session token.
    Unauthorized(String),
    /// 403: authenticated but not allowed (membership/ownership/poll state).
    Forbidden(String),
    /// 409: a uniqueness rule rejected the action (already voted / claimed).
    Conflict(String),
    /// 400: malformed or missing request input.
    BadRequest(String),
    /// 502: an upstream dependency (Hasura, the push service) failed.
    Upstream(String),
}

impl AppError {
    pub fn status(&self) -> StatusCode {
        match self {
            AppError::Unauthorized(_) => StatusCode::UNAUTHORIZED,
            AppError::Forbidden(_) => StatusCode::FORBIDDEN,
            AppError::Conflict(_) => StatusCode::CONFLICT,
            AppError::BadRequest(_) => StatusCode::BAD_REQUEST,
            AppError::Upstream(_) => StatusCode::BAD_GATEWAY,
        }
    }

    /// The user-facing message. Upstream failures are already genericised at
    /// their source (`admin_gql` logs the Hasura detail server-side), so every
    /// variant's message is safe to return to the client.
    pub fn message(&self) -> &str {
        match self {
            AppError::Unauthorized(m)
            | AppError::Forbidden(m)
            | AppError::Conflict(m)
            | AppError::BadRequest(m)
            | AppError::Upstream(m) => m,
        }
    }

    /// The standard `{ok:false,error}` failure response for an endpoint (the
    /// same body shape `error_json` produced), now with the variant's status
    /// code, logging the detail under `ctx` as a tracing event.
    pub fn respond(&self, ctx: &str) -> Response<Body> {
        tracing::error!("{ctx} error ({}): {}", self.status(), self.message());
        crate::json(
            self.status(),
            json!({ "ok": false, "error": self.message() }).to_string(),
        )
    }
}
