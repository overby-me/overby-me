//! NHost / Hasura glue: verify the caller's NHost access token (HS256, since the
//! project's JWKS is empty) to learn which user is linking, and upsert the
//! atproto identity into the `user_providers` table with the admin secret.

use crate::util;
use hmac::{Hmac, Mac};
use serde_json::{json, Value};
use sha2::Sha256;

type HmacSha256 = Hmac<Sha256>;

/// Verify an NHost access token (HS256) with the shared JWT secret and return
/// the user id (`sub`). Rejects a wrong/`none` alg, a bad signature, or expiry.
pub fn verify_access_token(token: &str, secret: &str) -> Result<String, String> {
    let parts: Vec<&str> = token.split('.').collect();
    if parts.len() != 3 {
        return Err("malformed jwt".into());
    }
    let header: Value =
        serde_json::from_slice(&util::b64url_decode(parts[0])?).map_err(|e| e.to_string())?;
    if header.get("alg").and_then(|a| a.as_str()) != Some("HS256") {
        return Err("unexpected jwt alg".into());
    }
    let signing_input = format!("{}.{}", parts[0], parts[1]);
    let sig = util::b64url_decode(parts[2])?;
    let mut mac = HmacSha256::new_from_slice(secret.as_bytes()).map_err(|e| e.to_string())?;
    mac.update(signing_input.as_bytes());
    mac.verify_slice(&sig)
        .map_err(|_| "bad jwt signature".to_string())?;

    let claims: Value =
        serde_json::from_slice(&util::b64url_decode(parts[1])?).map_err(|e| e.to_string())?;
    if let Some(exp) = claims.get("exp").and_then(|v| v.as_u64()) {
        if util::now_secs() >= exp {
            return Err("token expired".into());
        }
    }
    claims
        .get("sub")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .ok_or_else(|| "jwt has no sub".into())
}

/// Upsert the atproto link (`provider = atproto`) for a user, keyed on the
/// unique (provider, provider_id) pair so re-linking updates the handle/owner.
pub async fn upsert_atproto_link(
    client: &reqwest::Client,
    graphql_url: &str,
    admin_secret: &str,
    user_id: &str,
    did: &str,
    handle: &str,
) -> Result<(), String> {
    let query = "mutation($obj: user_providers_insert_input!) { \
        insert_user_providers_one(object: $obj, on_conflict: { \
            constraint: user_providers_provider_provider_id_key, \
            update_columns: [handle, user_id] }) { id } }";
    let body = json!({
        "query": query,
        "variables": { "obj": {
            "user_id": user_id,
            "provider": "atproto",
            "provider_id": did,
            "handle": handle,
        }},
    });
    let resp = client
        .post(graphql_url)
        .header("x-hasura-admin-secret", admin_secret)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let value: Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(errors) = value.get("errors") {
        return Err(format!("hasura error: {errors}"));
    }
    Ok(())
}
