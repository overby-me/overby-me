//! Backend-enforced SECRET ballot. A normal cast inserts a `vote/vote` node with
//! the voter's own token, so Hasura's per-role preset stamps `owner_id` — the
//! context owner can then see who voted how. A *secret* cast routes here: the
//! backend inserts the vote node with the ADMIN secret and NO owner_id (the admin
//! role has no preset, and `owner_id` has a NULL default + no trigger — verified),
//! so the ballot is untraceable, while a separate `has_voted(poll_id, user_id)`
//! marker enforces one vote per member without linking the marker to the ballot.
//!
//!   POST /vote/cast?poll=&context=&choices=0,2   (Authorization: Bearer <jwt>)
//!   GET  /vote/status?poll=                       -> {"voted": bool}

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
        Err(e) => {
            eprintln!("vote cast error: {e}");
            crate::json(
                StatusCode::BAD_REQUEST,
                json!({ "ok": false, "error": e }).to_string(),
            )
        }
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
    let token = token_from(query, bearer).ok_or("missing token")?;
    let poll = get("poll").ok_or("missing poll")?;
    let context = get("context").filter(|c| !c.is_empty());
    let choices: Vec<i64> = get("choices")
        .unwrap_or_default()
        .split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect();
    let uid = crate::nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)?;

    // Dedup first: insert the has_voted marker. A conflict (0 rows) = already voted.
    let marker = json!({
        "query": "mutation($p: uuid!, $u: uuid!) { insert_has_voted(objects: [{poll_id: $p, user_id: $u}], on_conflict: {constraint: has_voted_pkey, update_columns: []}) { affected_rows } }",
        "variables": { "p": poll, "u": uid },
    });
    let v = admin_gql(cfg, client, marker).await?;
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
        "contextId": context,
        "data": choices,
    });
    let insert = json!({
        "query": "mutation($obj: nodes_insert_input!) { insertNode(object: $obj) { id } }",
        "variables": { "obj": obj },
    });
    admin_gql(cfg, client, insert).await?;
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
    let token = token_from(query, bearer).ok_or("missing token")?;
    let uid = crate::nhost::verify_access_token(&token, &cfg.nhost_jwt_secret)?;
    let q = json!({
        "query": "query($p: uuid!, $u: uuid!) { has_voted(where: {poll_id: {_eq: $p}, user_id: {_eq: $u}}, limit: 1) { poll_id } }",
        "variables": { "p": poll, "u": uid },
    });
    let v = admin_gql(cfg, client, q).await?;
    Ok(v.pointer("/data/has_voted")
        .and_then(|a| a.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false))
}
