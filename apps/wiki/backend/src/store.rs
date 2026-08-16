//! The backend's data-access seam: every SURVIVING-intent Hasura query that a
//! handler needs, wrapped in an intent-named function returning a small typed
//! struct. Handlers stay Hasura-free; at cutover only this module is rewritten
//! against Turso (the same seam class `auth.rs` established for the authz
//! predicates). Interim-protocol queries that die wholesale at cutover (the
//! secret-ballot marker/insert/status in `vote.rs`, the NHost email lookup in
//! `notify.rs`) deliberately stay inline at their call sites, marked as such.

use crate::auth::admin_gql;
use crate::error::AppError;
use crate::oauth::Config;
use crate::push::Subscription;
use serde_json::{json, Value};

/// The poll fields the cast authorization reads (from the poll node itself,
/// never trusted from the client).
pub struct PollMeta {
    pub mime_id: Option<String>,
    pub mutable: Option<bool>,
    pub context_id: Option<String>,
    pub created_at: Option<String>,
}

/// The poll node's kind/state/context, or None if no such node exists.
pub async fn poll_meta(
    cfg: &Config,
    client: &reqwest::Client,
    poll: &str,
) -> Result<Option<PollMeta>, AppError> {
    let v = admin_gql(
        cfg,
        client,
        json!({
            "query": "query($p: uuid!) { node(id: $p) { mimeId mutable contextId createdAt } }",
            "variables": { "p": poll },
        }),
    )
    .await?;
    let Some(node) = v.pointer("/data/node").filter(|n| !n.is_null()) else {
        return Ok(None);
    };
    let s = |k: &str| node.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(Some(PollMeta {
        mime_id: s("mimeId"),
        mutable: node.get("mutable").and_then(Value::as_bool),
        context_id: s("contextId"),
        created_at: s("createdAt"),
    }))
}

/// A member row located by its secret claim token.
pub struct ClaimMember {
    pub id: String,
    pub node_id: Option<String>,
    pub parent_id: Option<String>,
}

/// Look up the member a `?claim=<token>` link points at.
pub async fn member_by_claim_token(
    cfg: &Config,
    client: &reqwest::Client,
    claim_token: &str,
) -> Result<Option<ClaimMember>, AppError> {
    let v = admin_gql(cfg, client, json!({
        "query": "query($t: String!) { members(where: {claim_token: {_eq: $t}}, limit: 1) { id nodeId parentId } }",
        "variables": { "t": claim_token },
    })).await?;
    let Some(m) = v.pointer("/data/members/0") else {
        return Ok(None);
    };
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    let Some(id) = s("id") else { return Ok(None) };
    Ok(Some(ClaimMember {
        id,
        node_id: s("nodeId"),
        parent_id: s("parentId"),
    }))
}

/// Bind a pending member row to a user, guarded on `nodeId` still null so a
/// race cannot double-claim. Returns whether a row was actually bound.
pub async fn bind_member_to_user(
    cfg: &Config,
    client: &reqwest::Client,
    member_id: &str,
    uid: &str,
) -> Result<bool, AppError> {
    let v = admin_gql(cfg, client, json!({
        "query": "mutation($id: uuid!, $u: uuid!) { updateMembers(where: {id: {_eq: $id}, nodeId: {_is_null: true}}, _set: {nodeId: $u, accepted: true}) { affected_rows } }",
        "variables": { "id": member_id, "u": uid },
    })).await?;
    Ok(v.pointer("/data/updateMembers/affected_rows")
        .and_then(Value::as_i64)
        .unwrap_or(0)
        > 0)
}

/// A member's context and secret claim token (for the owner claim-link flow).
pub struct MemberClaimInfo {
    pub parent_id: Option<String>,
    pub claim_token: Option<String>,
}

/// Fetch a member's context id + claim token by member id.
pub async fn member_claim_token(
    cfg: &Config,
    client: &reqwest::Client,
    member_id: &str,
) -> Result<Option<MemberClaimInfo>, AppError> {
    let v = admin_gql(cfg, client, json!({
        "query": "query($id: uuid!) { members(where: {id: {_eq: $id}}, limit: 1) { parentId claim_token } }",
        "variables": { "id": member_id },
    })).await?;
    let Some(m) = v.pointer("/data/members/0") else {
        return Ok(None);
    };
    let s = |k: &str| m.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(Some(MemberClaimInfo {
        parent_id: s("parentId"),
        claim_token: s("claim_token"),
    }))
}

/// Upsert a device's Web Push subscription by endpoint (a device
/// re-subscribing keeps one row with fresh keys).
pub async fn upsert_push_subscription(
    cfg: &Config,
    client: &reqwest::Client,
    uid: &str,
    email: &str,
    endpoint: &str,
    p256dh: &str,
    auth: &str,
) -> Result<(), AppError> {
    admin_gql(cfg, client, json!({
        "query": "mutation($o: [push_subscriptions_insert_input!]!) { insert_push_subscriptions(objects: $o, on_conflict: {constraint: push_subscriptions_endpoint_key, update_columns: [user_id, email, p256dh, auth]}) { affected_rows } }",
        "variables": { "o": [{
            "user_id": uid, "email": email, "endpoint": endpoint,
            "p256dh": p256dh, "auth": auth,
        }] },
    })).await?;
    Ok(())
}

/// Delete push subscriptions by endpoint (unsubscribe, or pruning gone ones).
pub async fn delete_subscriptions_by_endpoint(
    cfg: &Config,
    client: &reqwest::Client,
    endpoints: &[String],
) -> Result<(), AppError> {
    if endpoints.is_empty() {
        return Ok(());
    }
    admin_gql(cfg, client, json!({
        "query": "mutation($e: [String!]!) { delete_push_subscriptions(where: {endpoint: {_in: $e}}) { affected_rows } }",
        "variables": { "e": endpoints },
    })).await?;
    Ok(())
}

/// The stored Web Push subscriptions for a set of member emails.
pub async fn subscriptions_for_emails(
    cfg: &Config,
    client: &reqwest::Client,
    emails: &[String],
) -> Result<Vec<Subscription>, AppError> {
    let v = admin_gql(cfg, client, json!({
        "query": "query($e: [String!]!) { push_subscriptions(where: {email: {_in: $e}}) { endpoint p256dh auth } }",
        "variables": { "e": emails },
    })).await?;
    Ok(v.pointer("/data/push_subscriptions")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|s| {
                    Some(Subscription {
                        endpoint: s.get("endpoint")?.as_str()?.to_string(),
                        p256dh: s.get("p256dh")?.as_str()?.to_string(),
                        auth: s.get("auth")?.as_str()?.to_string(),
                    })
                })
                .collect()
        })
        .unwrap_or_default())
}

/// A node's owner + context (whose author a reply notification should reach).
pub struct NodeOwnerContext {
    pub owner_id: Option<String>,
    pub context_id: Option<String>,
}

/// Fetch a node's `ownerId` and `contextId`, or None if no such node.
pub async fn node_owner_and_context(
    cfg: &Config,
    client: &reqwest::Client,
    node_id: &str,
) -> Result<Option<NodeOwnerContext>, AppError> {
    let v = admin_gql(
        cfg,
        client,
        json!({
            "query": "query($id: uuid!) { node(id: $id) { ownerId contextId } }",
            "variables": { "id": node_id },
        }),
    )
    .await?;
    let Some(node) = v.pointer("/data/node").filter(|n| !n.is_null()) else {
        return Ok(None);
    };
    let s = |k: &str| node.get(k).and_then(Value::as_str).map(str::to_string);
    Ok(Some(NodeOwnerContext {
        owner_id: s("ownerId"),
        context_id: s("contextId"),
    }))
}

/// The emails of a context's active, accepted members (push fan-out targets).
pub async fn active_member_emails(
    cfg: &Config,
    client: &reqwest::Client,
    context: &str,
) -> Result<Vec<String>, AppError> {
    let v = admin_gql(cfg, client, json!({
        "query": "query($c: uuid!) { members(where: {parentId: {_eq: $c}, active: {_eq: true}, accepted: {_eq: true}}) { email } }",
        "variables": { "c": context },
    })).await?;
    Ok(v.pointer("/data/members")
        .and_then(Value::as_array)
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("email").and_then(Value::as_str))
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default())
}
