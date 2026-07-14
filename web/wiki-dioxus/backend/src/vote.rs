//! Backend-enforced SECRET ballot. A normal cast inserts a `vote/vote` node with
//! the voter's own token, so Hasura's per-role preset stamps `owner_id` — the
//! context owner can then see who voted how. A *secret* cast routes here: the
//! backend inserts the vote node with the ADMIN secret and NO owner_id (the admin
//! role has no preset, and `owner_id` has a NULL default + no trigger — verified),
//! so the ballot is untraceable, while a separate `has_voted(poll_id, user_id)`
//! marker enforces one vote per member without linking the marker to the ballot.
//!
//! The cast is authorised server-side (the admin path bypasses row-level
//! security): the target must be an open `vote/poll` and the caller an active
//! member of that poll's context. The `context` derives from the poll, not the
//! request.
//!
//!   POST /vote/cast?poll=&choices=0,2   (Authorization: Bearer <jwt>)
//!   GET  /vote/status?poll=             -> {"voted": bool}

use crate::oauth::Config;
use axum::{body::Body, response::Response};
use http::StatusCode;
use serde_json::json;

pub async fn cast(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match cast_inner(cfg, client, query, bearer).await {
        Ok(()) => crate::json(StatusCode::OK, "{\"ok\":true}".into()),
        Err(e) if e == "already voted" => crate::json(
            StatusCode::CONFLICT,
            json!({ "ok": false, "error": e }).to_string(),
        ),
        // Authorization/state failures carry a safe, user-facing message and a
        // 403, so they reach the voter instead of being genericised by
        // `error_json` (which is reserved for internal errors).
        Err(e)
            if matches!(
                e.as_str(),
                "not a poll"
                    | "poll closed"
                    | "poll not found"
                    | "poll has no context"
                    | "not a member of this context"
            ) =>
        {
            crate::json(
                StatusCode::FORBIDDEN,
                json!({ "ok": false, "error": e }).to_string(),
            )
        }
        Err(e) => crate::error_json("vote cast", e),
    }
}

async fn cast_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(), String> {
    let params = crate::util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    let poll = get("poll").ok_or("missing poll")?;
    let choices: Vec<i64> = get("choices")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    // Identify the caller (verifies the JWT and fetches their invite email).
    let (uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    // Authorize the ballot. The admin-secret path below bypasses Hasura's
    // row-level security, so membership + poll state MUST be checked here: the
    // target must be an OPEN `vote/poll`, and the caller an active member of that
    // poll's context. The context is read from the poll itself (authoritative),
    // never trusted from the client.
    let poll_v = crate::auth::admin_gql(
        cfg,
        client,
        json!({
            "query": "query($p: uuid!) { node(id: $p) { mimeId mutable contextId } }",
            "variables": { "p": poll },
        }),
    )
    .await?;
    let pnode = poll_v
        .pointer("/data/node")
        .filter(|n| !n.is_null())
        .ok_or("poll not found")?;
    if pnode.get("mimeId").and_then(|m| m.as_str()) != Some("vote/poll") {
        return Err("not a poll".into());
    }
    if pnode.get("mutable").and_then(|m| m.as_bool()) != Some(true) {
        return Err("poll closed".into());
    }
    let poll_context = pnode
        .get("contextId")
        .and_then(|c| c.as_str())
        .filter(|c| !c.is_empty())
        .ok_or("poll has no context")?
        .to_string();

    // Active membership in the poll's context, matched by the durable node_id
    // binding or (fallback) the invite email.
    let mem_v = crate::auth::admin_gql(
        cfg,
        client,
        json!({
            "query": "query($c: uuid!, $u: uuid!, $e: String!) { members(where: {parentId: {_eq: $c}, active: {_eq: true}, _or: [{nodeId: {_eq: $u}}, {email: {_eq: $e}}]}, limit: 1) { id } }",
            "variables": { "c": poll_context, "u": uid, "e": email },
        }),
    )
    .await?;
    if mem_v.pointer("/data/members/0").is_none() {
        return Err("not a member of this context".into());
    }

    // Dedup first: insert the has_voted marker. A conflict (0 rows) = already voted.
    let marker = json!({
        "query": "mutation($p: uuid!, $u: uuid!) { insert_has_voted(objects: [{poll_id: $p, user_id: $u}], on_conflict: {constraint: has_voted_pkey, update_columns: []}) { affected_rows } }",
        "variables": { "p": poll, "u": uid },
    });
    let v = crate::auth::admin_gql(cfg, client, marker).await?;
    let inserted = v
        .pointer("/data/insert_has_voted/affected_rows")
        .and_then(|n| n.as_i64())
        .unwrap_or(0)
        > 0;
    if !inserted {
        return Err("already voted".into());
    }

    // Insert the ANONYMOUS vote node (no owner_id).
    let secs = crate::util::now_secs();
    let obj = json!({
        "name": format!("vote-{secs}"),
        "key": format!("vote-{secs}-{}", crate::util::random_token(6)),
        "mimeId": "vote/vote",
        "parentId": poll,
        "contextId": poll_context,
        "data": choices,
    });
    let insert = json!({
        "query": "mutation($obj: nodes_insert_input!) { insertNode(object: $obj) { id } }",
        "variables": { "obj": obj },
    });
    crate::auth::admin_gql(cfg, client, insert).await?;
    Ok(())
}

pub async fn status(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match status_inner(cfg, client, query, bearer).await {
        Ok(voted) => crate::json(StatusCode::OK, json!({ "voted": voted }).to_string()),
        Err(e) => {
            eprintln!("vote status error: {e}");
            crate::json(StatusCode::OK, "{\"voted\":false}".into())
        }
    }
}

async fn status_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<bool, String> {
    let params = crate::util::parse_query(query);
    let poll = params
        .iter()
        .find(|(k, _)| k == "poll")
        .map(|(_, v)| v.clone())
        .ok_or("missing poll")?;
    let token = crate::auth::token_from(query, bearer).ok_or("missing token")?;
    let uid = crate::nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)?;
    let q = json!({
        "query": "query($p: uuid!, $u: uuid!) { has_voted(where: {poll_id: {_eq: $p}, user_id: {_eq: $u}}, limit: 1) { poll_id } }",
        "variables": { "p": poll, "u": uid },
    });
    let v = crate::auth::admin_gql(cfg, client, q).await?;
    Ok(v.pointer("/data/has_voted")
        .and_then(|a| a.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false))
}
