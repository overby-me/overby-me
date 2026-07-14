//! Shared authentication + admin-GraphQL helpers used by every handler that
//! acts on the caller's behalf via the Hasura admin secret. Previously each of
//! notify.rs / vote.rs / members.rs kept byte-identical private copies of
//! `token_from` and `admin_gql`, and near-duplicate caller-identity resolvers;
//! centralising them here keeps token handling and the admin POST consistent.

use crate::oauth::Config;
use serde_json::{json, Value};

/// The caller's session JWT: prefer the `Authorization: Bearer` header, fall back
/// to a `?token=` query param (the only option for full-page redirects).
pub fn token_from(query: Option<&str>, bearer: Option<&str>) -> Option<String> {
    if let Some(b) = bearer.filter(|b| !b.is_empty()) {
        return Some(b.to_string());
    }
    crate::util::parse_query(query)
        .into_iter()
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v)
}

/// POST a GraphQL body to Hasura with the admin secret (bypasses row-level
/// permissions). Returns the parsed response or a `hasura error: ...` string.
pub async fn admin_gql(
    cfg: &Config,
    client: &reqwest::Client,
    body: Value,
) -> Result<Value, String> {
    let resp = client
        .post(&cfg.hasura_url)
        .header("x-hasura-admin-secret", &cfg.admin_secret)
        .json(&body)
        .send()
        .await
        .map_err(|e| e.to_string())?;
    let v: Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(errors) = v.get("errors") {
        return Err(format!("hasura error: {errors}"));
    }
    Ok(v)
}

/// Verify the caller's JWT and return their user id (`sub`).
pub fn caller_uid(
    cfg: &Config,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<String, String> {
    let token = token_from(query, bearer).ok_or("missing token")?;
    crate::nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)
}

/// The signed-in caller's uuid + email (email is how membership is keyed).
pub async fn caller(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(String, String), String> {
    let uid = caller_uid(cfg, query, bearer)?;
    let v = admin_gql(
        cfg,
        client,
        json!({
            "query": "query($id: uuid!) { user(id: $id) { email } }",
            "variables": { "id": uid },
        }),
    )
    .await?;
    let email = v
        .pointer("/data/user/email")
        .and_then(|e| e.as_str())
        .ok_or("no email for user")?
        .to_string();
    Ok((uid, email))
}
