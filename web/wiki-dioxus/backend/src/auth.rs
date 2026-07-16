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
        // Log the Hasura detail server-side, but return a generic error so query
        // internals (schema, constraint names) never reach the client via
        // `error_json`. Handler-level domain errors are unaffected.
        eprintln!("hasura error: {errors}");
        return Err("backend query failed".into());
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

/// The authenticated caller as an authorization principal: the durable `node_id`
/// binding (`uid`) plus the invite/roster `email` fallback. This is the one shape
/// every authz check consumes; the atproto rewrite swaps its innards for a DID while
/// [`is_active_member`] / [`is_active_owner`] keep their signatures.
pub struct Principal {
    pub uid: String,
    pub email: String,
}

impl From<(String, String)> for Principal {
    fn from((uid, email): (String, String)) -> Self {
        Self { uid, email }
    }
}

/// Is `p` an ACTIVE member of `context`? Matches the durable `node_id` binding OR
/// (fallback) the invite email, so a mismatched-email roster entry that has been
/// claim-bound by node_id still authorizes. THE membership predicate: every handler
/// must call this rather than inlining a divergent members `where` clause (which is
/// how the vote and notify paths silently disagreed before).
pub async fn is_active_member(
    cfg: &Config,
    client: &reqwest::Client,
    context: &str,
    p: &Principal,
) -> Result<bool, String> {
    let v = admin_gql(cfg, client, json!({
        "query": "query($c: uuid!, $u: uuid!, $e: String!) { members(where: {parentId: {_eq: $c}, active: {_eq: true}, _or: [{nodeId: {_eq: $u}}, {email: {_eq: $e}}]}, limit: 1) { id } }",
        "variables": { "c": context, "u": p.uid, "e": p.email },
    })).await?;
    Ok(v.pointer("/data/members/0").is_some())
}

/// Is `p` an ACTIVE OWNER of `context`? An active member flagged `owner` (matched by
/// node_id OR email), or the owner of the context node itself (its `ownerId`). THE
/// owner predicate; unifies the previously-divergent notify (email-only) and members
/// (node_id-only) owner checks and adds the active requirement.
pub async fn is_active_owner(
    cfg: &Config,
    client: &reqwest::Client,
    context: &str,
    p: &Principal,
) -> Result<bool, String> {
    let v = admin_gql(cfg, client, json!({
        "query": "query($c: uuid!, $u: uuid!, $e: String!) { members(where: {parentId: {_eq: $c}, active: {_eq: true}, owner: {_eq: true}, _or: [{nodeId: {_eq: $u}}, {email: {_eq: $e}}]}, limit: 1) { id } node(id: $c) { ownerId } }",
        "variables": { "c": context, "u": p.uid, "e": p.email },
    })).await?;
    let member_owner = v.pointer("/data/members/0").is_some();
    let node_owner =
        v.pointer("/data/node/ownerId").and_then(Value::as_str) == Some(p.uid.as_str());
    Ok(member_owner || node_owner)
}

#[cfg(test)]
mod tests {
    use super::token_from;

    #[test]
    fn prefers_bearer_over_query() {
        assert_eq!(
            token_from(Some("token=from-query"), Some("from-header")),
            Some("from-header".into())
        );
    }

    #[test]
    fn falls_back_to_query_token() {
        // An empty Bearer is ignored (treated as absent), so the query wins.
        assert_eq!(
            token_from(Some("a=1&token=abc&b=2"), Some("")),
            Some("abc".into())
        );
        assert_eq!(token_from(Some("token=abc"), None), Some("abc".into()));
    }

    #[test]
    fn none_when_neither_present() {
        assert_eq!(token_from(Some("a=1&b=2"), None), None);
        assert_eq!(token_from(None, None), None);
    }
}
