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

/// Parse the `choices` query param ("0,2,3") into option indices. Whitespace is
/// trimmed and non-integer / empty entries are dropped, so a malformed choice
/// silently narrows the ballot rather than rejecting it (the tally only counts
/// indices that exist).
fn parse_choices(raw: &str) -> Vec<i64> {
    raw.split(',')
        .filter_map(|s| s.trim().parse().ok())
        .collect()
}

/// A ballot node's `key`. Anonymity depends on this carrying NOTHING that
/// correlates the ballot to the per-user `has_voted` marker: no owner, no user
/// id, no timestamp — just a random suffix under the poll's `(parent_id, key)`
/// uniqueness.
fn ballot_key() -> String {
    format!("ballot-{}", crate::util::random_token(16))
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
    let choices = parse_choices(&get("choices").unwrap_or_default());
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
            "query": "query($p: uuid!) { node(id: $p) { mimeId mutable contextId createdAt } }",
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
    // The poll's own creation time, used to coarsen the ballot's timestamps below.
    let poll_created = pnode
        .get("createdAt")
        .and_then(|c| c.as_str())
        .map(str::to_string);

    // Active membership in the poll's context (the shared predicate).
    let principal = crate::auth::Principal {
        uid: uid.clone(),
        email: email.clone(),
    };
    if !crate::auth::is_active_member(cfg, client, &poll_context, &principal).await? {
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

    // Insert the ANONYMOUS vote node (no owner_id). Anonymity depends on the
    // ballot carrying NOTHING that correlates it to the per-user `has_voted`
    // marker: no owner, no time in the name/key, and its `created_at`/`updated_at`
    // are coarsened to the poll's own creation time (default `now()` would let a
    // DB/admin holder align the ballot with the marker's timestamp to recover who
    // voted how). All ballots for a poll therefore share one timestamp.
    let mut obj = json!({
        "name": "ballot",
        "key": ballot_key(),
        "mimeId": "vote/vote",
        "parentId": poll,
        "contextId": poll_context,
        "data": choices,
    });
    if let Some(ts) = &poll_created {
        obj["createdAt"] = json!(ts);
        obj["updatedAt"] = json!(ts);
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_choices_trims_and_drops_invalid() {
        assert_eq!(parse_choices("0,2,3"), vec![0, 2, 3]);
        assert_eq!(parse_choices(" 1 , 2 "), vec![1, 2]); // whitespace trimmed
        assert_eq!(parse_choices("1,x,3"), vec![1, 3]); // non-integer dropped
        assert_eq!(parse_choices("1,,2"), vec![1, 2]); // empty segment dropped
    }

    #[test]
    fn parse_choices_empty_input_is_empty() {
        assert!(parse_choices("").is_empty());
        assert!(parse_choices(" ").is_empty());
        assert!(parse_choices(",").is_empty());
    }

    #[test]
    fn ballot_key_is_anonymous_and_unique() {
        let uid = "3f2504e0-4f89-11d3-9a0c-0305e82c3301";
        let a = ballot_key();
        let b = ballot_key();
        assert!(a.starts_with("ballot-"), "keeps the ballot- prefix");
        // The suffix must be a fresh random token, never derived from the voter:
        // it carries no user id (so it can't be correlated to `has_voted`) and no
        // two ballots collide on the poll's (parent_id, key) uniqueness.
        assert!(!a.contains(uid), "key must not embed the user id");
        assert_ne!(a, b, "each ballot key is unique");
        assert_eq!(a.len(), "ballot-".len() + 22); // 16 random bytes -> 22 b64url chars
    }
}
