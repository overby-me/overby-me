//! Membership claiming: bind a rostered member row to the signed-in account.
//!
//! Roster invites arrive as an email spreadsheet, so a member is normally
//! discovered by email (`email == my_email`) and the accept flow stamps
//! `node_id = <user id>` (the durable binding). Someone who signs up with a
//! DIFFERENT email than the roster can never discover their invite. A per-member
//! secret `claim_token` (never exposed to the `user` GraphQL role) lets an owner
//! hand out a `?claim=<token>` link that binds the row to whoever claims it,
//! setting the same `node_id` the accept flow sets.
//!
//!   POST /members/claim?token=        bind the token's member to the caller
//!   POST /members/claim-link?member=  (owner) fetch a member's claim token

use crate::oauth::Config;
use axum::{body::Body, response::Response};
use http::StatusCode;
use serde_json::{json, Value};

fn token_from(query: Option<&str>, bearer: Option<&str>) -> Option<String> {
    if let Some(b) = bearer.filter(|b| !b.is_empty()) {
        return Some(b.to_string());
    }
    crate::util::parse_query(query)
        .into_iter()
        .find(|(k, _)| k == "token")
        .map(|(_, v)| v)
}

async fn admin_gql(cfg: &Config, client: &reqwest::Client, body: Value) -> Result<Value, String> {
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

fn caller_uid(cfg: &Config, query: Option<&str>, bearer: Option<&str>) -> Result<String, String> {
    let token = token_from(query, bearer).ok_or("missing token")?;
    crate::nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)
}

// --- claim ------------------------------------------------------------------

pub async fn claim(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match claim_inner(cfg, client, query, bearer).await {
        Ok(context) => crate::json(
            StatusCode::OK,
            json!({ "ok": true, "context": context }).to_string(),
        ),
        Err(e) => {
            eprintln!("member claim error: {e}");
            crate::json(
                StatusCode::BAD_REQUEST,
                json!({ "ok": false, "error": e }).to_string(),
            )
        }
    }
}

/// Bind the member identified by `?claim=<token>` to the caller. Returns the
/// context (parent) id so the app can navigate there.
async fn claim_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<String, String> {
    let params = crate::util::parse_query(query);
    let claim_token = params
        .iter()
        .find(|(k, _)| k == "claim" || k == "member_token")
        .map(|(_, v)| v.clone())
        .filter(|s| !s.is_empty())
        .ok_or("missing claim token")?;
    let uid = caller_uid(cfg, query, bearer)?;

    let m = admin_gql(cfg, client, json!({
        "query": "query($t: String!) { members(where: {claim_token: {_eq: $t}}, limit: 1) { id nodeId parentId } }",
        "variables": { "t": claim_token },
    })).await?;
    let member = m
        .pointer("/data/members/0")
        .ok_or("invalid or expired claim link")?;
    let member_id = member
        .get("id")
        .and_then(|v| v.as_str())
        .ok_or("no member id")?;
    let context = member
        .get("parentId")
        .and_then(|v| v.as_str())
        .ok_or("member has no context")?
        .to_string();

    // Already claimed? Idempotent for the same user; refuse for anyone else.
    if let Some(existing) = member.get("nodeId").and_then(|v| v.as_str()) {
        if existing == uid {
            return Ok(context);
        }
        return Err("this invitation has already been claimed".into());
    }

    // Bind (guarded on nodeId still null, so a race can't double-claim).
    let updated = admin_gql(cfg, client, json!({
        "query": "mutation($id: uuid!, $u: uuid!) { updateMembers(where: {id: {_eq: $id}, nodeId: {_is_null: true}}, _set: {nodeId: $u, accepted: true}) { affected_rows } }",
        "variables": { "id": member_id, "u": uid },
    })).await?;
    let rows = updated
        .pointer("/data/updateMembers/affected_rows")
        .and_then(|n| n.as_i64())
        .unwrap_or(0);
    if rows == 0 {
        return Err("this invitation has already been claimed".into());
    }
    Ok(context)
}

// --- claim-link (owner only) ------------------------------------------------

pub async fn claim_link(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match claim_link_inner(cfg, client, query, bearer).await {
        Ok(token) => crate::json(
            StatusCode::OK,
            json!({ "ok": true, "token": token }).to_string(),
        ),
        Err(e) => {
            eprintln!("member claim-link error: {e}");
            crate::json(
                StatusCode::BAD_REQUEST,
                json!({ "ok": false, "error": e }).to_string(),
            )
        }
    }
}

/// Return a member's secret claim token, but only to an owner of that member's
/// context (so an owner can build/share the `?claim=` link).
async fn claim_link_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<String, String> {
    let params = crate::util::parse_query(query);
    let member_id = params
        .iter()
        .find(|(k, _)| k == "member")
        .map(|(_, v)| v.clone())
        .filter(|s| !s.is_empty())
        .ok_or("missing member id")?;
    let uid = caller_uid(cfg, query, bearer)?;

    let m = admin_gql(cfg, client, json!({
        "query": "query($id: uuid!) { members(where: {id: {_eq: $id}}, limit: 1) { parentId claim_token } }",
        "variables": { "id": member_id },
    })).await?;
    let member = m.pointer("/data/members/0").ok_or("member not found")?;
    let context = member
        .get("parentId")
        .and_then(|v| v.as_str())
        .ok_or("member has no context")?;
    let claim_token = member
        .get("claim_token")
        .and_then(|v| v.as_str())
        .ok_or("member has no claim token")?
        .to_string();

    // Owner = a member of the context flagged owner, or the context node's owner.
    let owner = admin_gql(cfg, client, json!({
        "query": "query($c: uuid!, $u: uuid!) { members(where: {parentId: {_eq: $c}, nodeId: {_eq: $u}, owner: {_eq: true}}, limit: 1) { id } node(id: $c) { ownerId } }",
        "variables": { "c": context, "u": uid },
    })).await?;
    let is_member_owner = owner
        .pointer("/data/members")
        .and_then(|a| a.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    let is_node_owner =
        owner.pointer("/data/node/ownerId").and_then(|v| v.as_str()) == Some(uid.as_str());
    if !is_member_owner && !is_node_owner {
        return Err("not a context owner".into());
    }
    Ok(claim_token)
}
