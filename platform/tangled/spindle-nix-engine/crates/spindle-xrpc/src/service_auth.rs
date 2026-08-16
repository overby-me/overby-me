//! Service authentication extractor for XRPC requests.
//!
//! Validates the `Authorization: Bearer <token>` header. Supports two modes:
//!
//! 1. **AT Protocol JWT** — The bearer token is a JWT signed by the caller.
//!    The JWT's `aud` claim must match this spindle's `did:web`, and the
//!    signature is verified against the caller's public key resolved from the
//!    PLC directory. This is what tangled.org (appview) sends.
//!
//! 2. **Plain bearer token** — Falls back to a direct string comparison
//!    against the configured spindle token. Used for local/internal API calls.

use std::time::{SystemTime, UNIX_EPOCH};

use axum::extract::FromRequestParts;
use axum::http::StatusCode;
use axum::http::request::Parts;
use axum::response::{IntoResponse, Response};
use base64::Engine as _;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use k256::ecdsa::signature::Verifier as _;
use serde::Deserialize;
use tracing::warn;

use crate::XrpcContext;

/// Extracted service authentication information.
///
/// If this extractor succeeds, the request has been authenticated.
#[derive(Debug, Clone)]
pub struct ServiceAuth {
    /// The authenticated caller's DID.
    pub did: String,
}

/// Error returned when authentication fails.
#[derive(Debug)]
pub struct AuthError {
    pub status: StatusCode,
    pub message: String,
}

impl IntoResponse for AuthError {
    fn into_response(self) -> Response {
        let body = serde_json::json!({
            "error": "AuthenticationRequired",
            "message": self.message,
        });
        (self.status, axum::Json(body)).into_response()
    }
}

impl FromRequestParts<std::sync::Arc<XrpcContext>> for ServiceAuth {
    type Rejection = AuthError;

    async fn from_request_parts(
        parts: &mut Parts,
        state: &std::sync::Arc<XrpcContext>,
    ) -> Result<Self, Self::Rejection> {
        let auth_header = parts
            .headers
            .get("authorization")
            .and_then(|v| v.to_str().ok());

        let token = match auth_header {
            Some(header) if header.starts_with("Bearer ") => &header[7..],
            _ => {
                return Err(AuthError {
                    status: StatusCode::UNAUTHORIZED,
                    message: "missing or invalid Authorization header".into(),
                });
            }
        };

        // Try JWT verification if the token looks like a JWT (has 2 dots).
        if token.matches('.').count() == 2 {
            match verify_jwt(token, state).await {
                Ok(auth) => return Ok(auth),
                Err(e) => {
                    warn!(error = %e.message, "JWT service auth verification failed");
                    return Err(e);
                }
            }
        }

        // Fall back to plain bearer token comparison.
        if token == state.token {
            return Ok(ServiceAuth {
                did: state.owner.clone(),
            });
        }

        Err(AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "invalid bearer token".into(),
        })
    }
}

// ---------------------------------------------------------------------------
// JWT verification
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct JwtHeader {
    alg: String,
}

#[derive(Debug, Deserialize)]
struct JwtPayload {
    iss: String,
    aud: String,
    exp: u64,
}

/// DID document returned by the PLC directory.
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DidDocument {
    verification_method: Vec<VerificationMethod>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct VerificationMethod {
    id: String,
    public_key_multibase: Option<String>,
}

/// Multicodec prefixes for key types.
const MULTICODEC_SECP256K1: [u8; 2] = [0xe7, 0x01];
const MULTICODEC_P256: [u8; 2] = [0x80, 0x24];

/// Verify an AT Protocol service auth JWT.
async fn verify_jwt(token: &str, state: &XrpcContext) -> Result<ServiceAuth, AuthError> {
    let parts: Vec<&str> = token.splitn(3, '.').collect();
    if parts.len() != 3 {
        return Err(AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "malformed JWT".into(),
        });
    }

    let header_bytes = URL_SAFE_NO_PAD.decode(parts[0]).map_err(|_| AuthError {
        status: StatusCode::UNAUTHORIZED,
        message: "invalid JWT header encoding".into(),
    })?;

    let payload_bytes = URL_SAFE_NO_PAD.decode(parts[1]).map_err(|_| AuthError {
        status: StatusCode::UNAUTHORIZED,
        message: "invalid JWT payload encoding".into(),
    })?;

    let signature_bytes = URL_SAFE_NO_PAD.decode(parts[2]).map_err(|_| AuthError {
        status: StatusCode::UNAUTHORIZED,
        message: "invalid JWT signature encoding".into(),
    })?;

    let header: JwtHeader = serde_json::from_slice(&header_bytes).map_err(|_| AuthError {
        status: StatusCode::UNAUTHORIZED,
        message: "invalid JWT header".into(),
    })?;

    let payload: JwtPayload = serde_json::from_slice(&payload_bytes).map_err(|_| AuthError {
        status: StatusCode::UNAUTHORIZED,
        message: "invalid JWT payload".into(),
    })?;

    // Validate audience matches this spindle's DID.
    if payload.aud != state.did_web {
        return Err(AuthError {
            status: StatusCode::FORBIDDEN,
            message: format!(
                "JWT audience mismatch: expected {}, got {}",
                state.did_web, payload.aud
            ),
        });
    }

    // Validate expiration (with 30s grace for clock skew).
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();
    if payload.exp + 30 < now {
        return Err(AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "JWT has expired".into(),
        });
    }

    // Validate issuer is a DID.
    if !payload.iss.starts_with("did:") {
        return Err(AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "JWT issuer is not a DID".into(),
        });
    }

    // Resolve the issuer's DID document from PLC directory.
    let did_url = format!("{}/{}", state.plc_url, payload.iss);
    let did_doc: DidDocument = state
        .http_client
        .get(&did_url)
        .send()
        .await
        .map_err(|e| AuthError {
            status: StatusCode::BAD_GATEWAY,
            message: format!("failed to resolve DID {}: {e}", payload.iss),
        })?
        .json()
        .await
        .map_err(|e| AuthError {
            status: StatusCode::BAD_GATEWAY,
            message: format!("invalid DID document for {}: {e}", payload.iss),
        })?;

    // Find the #atproto verification method.
    let vm = did_doc
        .verification_method
        .iter()
        .find(|vm| vm.id.ends_with("#atproto"))
        .ok_or_else(|| AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: format!(
                "no #atproto verification method in DID document for {}",
                payload.iss
            ),
        })?;

    let multibase_key = vm
        .public_key_multibase
        .as_deref()
        .ok_or_else(|| AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "verification method missing publicKeyMultibase".into(),
        })?;

    // Decode the multibase key (base58btc, 'z' prefix).
    if !multibase_key.starts_with('z') {
        return Err(AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "unsupported multibase encoding (expected base58btc 'z' prefix)".into(),
        });
    }

    let key_bytes = bs58::decode(&multibase_key[1..])
        .into_vec()
        .map_err(|_| AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "invalid base58btc key encoding".into(),
        })?;

    if key_bytes.len() < 3 {
        return Err(AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "key too short".into(),
        });
    }

    // The signing input is "header.payload" (the first two JWT segments).
    let signing_input = format!("{}.{}", parts[0], parts[1]);

    // Verify signature based on multicodec prefix and JWT algorithm.
    let prefix = [key_bytes[0], key_bytes[1]];
    let pubkey_bytes = &key_bytes[2..];

    match (prefix, header.alg.as_str()) {
        (MULTICODEC_SECP256K1, "ES256K") => {
            let vk = k256::ecdsa::VerifyingKey::from_sec1_bytes(pubkey_bytes).map_err(|_| {
                AuthError {
                    status: StatusCode::UNAUTHORIZED,
                    message: "invalid secp256k1 public key".into(),
                }
            })?;
            let sig =
                k256::ecdsa::Signature::from_slice(&signature_bytes).map_err(|_| AuthError {
                    status: StatusCode::UNAUTHORIZED,
                    message: "invalid ES256K signature".into(),
                })?;
            vk.verify(signing_input.as_bytes(), &sig)
                .map_err(|_| AuthError {
                    status: StatusCode::UNAUTHORIZED,
                    message: "ES256K signature verification failed".into(),
                })?;
        }
        (MULTICODEC_P256, "ES256") => {
            let vk = p256::ecdsa::VerifyingKey::from_sec1_bytes(pubkey_bytes).map_err(|_| {
                AuthError {
                    status: StatusCode::UNAUTHORIZED,
                    message: "invalid P-256 public key".into(),
                }
            })?;
            let sig =
                p256::ecdsa::Signature::from_slice(&signature_bytes).map_err(|_| AuthError {
                    status: StatusCode::UNAUTHORIZED,
                    message: "invalid ES256 signature".into(),
                })?;
            vk.verify(signing_input.as_bytes(), &sig)
                .map_err(|_| AuthError {
                    status: StatusCode::UNAUTHORIZED,
                    message: "ES256 signature verification failed".into(),
                })?;
        }
        _ => {
            return Err(AuthError {
                status: StatusCode::UNAUTHORIZED,
                message: format!(
                    "unsupported key type / algorithm combination: prefix={prefix:02x?}, alg={}",
                    header.alg
                ),
            });
        }
    }

    Ok(ServiceAuth { did: payload.iss })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_error_response_format() {
        let err = AuthError {
            status: StatusCode::UNAUTHORIZED,
            message: "test error".into(),
        };
        let response = err.into_response();
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }
}
