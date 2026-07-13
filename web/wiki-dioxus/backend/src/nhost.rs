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

/// Look up the caller's atproto link (handle + DID), if any. Admin query, so the
/// custom `user_providers` table stays entirely backend-side.
pub async fn get_atproto_link(
    client: &reqwest::Client,
    graphql_url: &str,
    admin_secret: &str,
    user_id: &str,
) -> Result<Option<(String, String)>, String> {
    let query = "query($uid: uuid!) { user_providers(where: { \
        user_id: { _eq: $uid }, provider: { _eq: \"atproto\" } }, limit: 1) \
        { handle provider_id } }";
    let body = json!({ "query": query, "variables": { "uid": user_id } });
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
    let row = value
        .get("data")
        .and_then(|d| d.get("user_providers"))
        .and_then(|a| a.as_array())
        .and_then(|a| a.first());
    Ok(row.map(|r| {
        let handle = r
            .get("handle")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        let did = r
            .get("provider_id")
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string();
        (handle, did)
    }))
}

/// Store the encrypted atproto session blob (refresh token + DPoP key + endpoints)
/// on the user's link row, so the app can later post on their behalf. Admin-only;
/// the `atproto_session` column is never exposed to the `user` role.
pub async fn store_atproto_session(
    client: &reqwest::Client,
    graphql_url: &str,
    admin_secret: &str,
    user_id: &str,
    session_blob: &str,
) -> Result<(), String> {
    let query = "mutation($uid: uuid!, $s: String!) { \
        update_user_providers(where: { user_id: { _eq: $uid }, \
            provider: { _eq: \"atproto\" } }, _set: { atproto_session: $s }) \
        { affected_rows } }";
    let body = json!({ "query": query, "variables": { "uid": user_id, "s": session_blob } });
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

/// Load the encrypted atproto session blob for a user, if present. Admin query.
pub async fn get_atproto_session(
    client: &reqwest::Client,
    graphql_url: &str,
    admin_secret: &str,
    user_id: &str,
) -> Result<Option<String>, String> {
    let query = "query($uid: uuid!) { user_providers(where: { \
        user_id: { _eq: $uid }, provider: { _eq: \"atproto\" } }, limit: 1) \
        { atproto_session } }";
    let body = json!({ "query": query, "variables": { "uid": user_id } });
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
    Ok(value
        .get("data")
        .and_then(|d| d.get("user_providers"))
        .and_then(|a| a.as_array())
        .and_then(|a| a.first())
        .and_then(|r| r.get("atproto_session"))
        .and_then(|v| v.as_str())
        .map(str::to_string))
}

/// Remove the atproto link (`provider = atproto`) for a user. Used by the unlink
/// endpoint; the caller separately clears the cached avatar (best-effort).
pub async fn delete_atproto_link(
    client: &reqwest::Client,
    graphql_url: &str,
    admin_secret: &str,
    user_id: &str,
) -> Result<(), String> {
    let query = "mutation($uid: uuid!) { \
        delete_user_providers(where: { user_id: { _eq: $uid }, \
            provider: { _eq: \"atproto\" } }) { affected_rows } }";
    let body = json!({ "query": query, "variables": { "uid": user_id } });
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

/// Set the NHost user's `avatarUrl` so the app can show the linked Bluesky avatar
/// as the profile picture. Uses the admin secret (nhost's `updateUser` by pk).
pub async fn update_user_avatar(
    client: &reqwest::Client,
    graphql_url: &str,
    admin_secret: &str,
    user_id: &str,
    avatar_url: &str,
) -> Result<(), String> {
    let query = "mutation($id: uuid!, $url: String!) { \
        updateUser(pk_columns: { id: $id }, _set: { avatarUrl: $url }) { id } }";
    let body = json!({
        "query": query,
        "variables": { "id": user_id, "url": avatar_url },
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
