//! Web Push subscription storage and fan-out.
//!
//!   POST /push/subscribe?endpoint=&p256dh=&auth=   store the caller's subscription
//!   POST /push/unsubscribe?endpoint=               drop it
//!   POST /push/notify?context=&title=&body=&url=   push to a context's members
//!
//! Membership in this app is by email, so a subscription is stored with the user's
//! email and the fan-out joins on it: a context owner may notify the active members
//! of that context. Encryption + VAPID live in [`crate::push`].

use crate::oauth::Config;
use crate::push::{self, Subscription};
use axum::{body::Body, response::Response};
use http::StatusCode;
use serde_json::json;

// --- subscribe / unsubscribe -----------------------------------------------

pub async fn subscribe(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match subscribe_inner(cfg, client, query, bearer).await {
        Ok(()) => crate::json(StatusCode::OK, "{\"ok\":true}".into()),
        Err(e) => crate::error_json("push subscribe", e),
    }
}

async fn subscribe_inner(
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
    let endpoint = get("endpoint")
        .filter(|s| !s.is_empty())
        .ok_or("missing endpoint")?;
    // SSRF guard: never store an endpoint the backend must not POST to.
    if !push::endpoint_allowed(&endpoint) {
        return Err("disallowed push endpoint".into());
    }
    let p256dh = get("p256dh")
        .filter(|s| !s.is_empty())
        .ok_or("missing p256dh")?;
    let auth = get("auth")
        .filter(|s| !s.is_empty())
        .ok_or("missing auth")?;
    let (uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    // Upsert by endpoint: a device re-subscribing keeps one row with fresh keys.
    crate::auth::admin_gql(cfg, client, json!({
        "query": "mutation($o: [push_subscriptions_insert_input!]!) { insert_push_subscriptions(objects: $o, on_conflict: {constraint: push_subscriptions_endpoint_key, update_columns: [user_id, email, p256dh, auth]}) { affected_rows } }",
        "variables": { "o": [{
            "user_id": uid, "email": email, "endpoint": endpoint,
            "p256dh": p256dh, "auth": auth,
        }] },
    })).await?;
    Ok(())
}

pub async fn unsubscribe(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match unsubscribe_inner(cfg, client, query, bearer).await {
        Ok(()) => crate::json(StatusCode::OK, "{\"ok\":true}".into()),
        Err(e) => crate::error_json("push unsubscribe", e),
    }
}

async fn unsubscribe_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(), String> {
    let params = crate::util::parse_query(query);
    let endpoint = params
        .iter()
        .find(|(k, _)| k == "endpoint")
        .map(|(_, v)| v.clone())
        .filter(|s| !s.is_empty())
        .ok_or("missing endpoint")?;
    // Must be signed in, but a subscription is device-owned: drop it by endpoint.
    let _ = crate::auth::caller(cfg, client, query, bearer).await?;
    delete_by_endpoints(cfg, client, &[endpoint]).await?;
    Ok(())
}

async fn delete_by_endpoints(
    cfg: &Config,
    client: &reqwest::Client,
    endpoints: &[String],
) -> Result<(), String> {
    if endpoints.is_empty() {
        return Ok(());
    }
    crate::auth::admin_gql(cfg, client, json!({
        "query": "mutation($e: [String!]!) { delete_push_subscriptions(where: {endpoint: {_in: $e}}) { affected_rows } }",
        "variables": { "e": endpoints },
    })).await?;
    Ok(())
}

/// Fetch the subscriptions for a set of member emails, push `payload` to each,
/// and prune any that report gone (404/410). Returns (recipients, sent).
async fn push_to_emails(
    cfg: &Config,
    client: &reqwest::Client,
    emails: &[String],
    payload: &str,
) -> Result<(usize, usize), String> {
    if emails.is_empty() {
        return Ok((0, 0));
    }
    let subs = crate::auth::admin_gql(cfg, client, json!({
        "query": "query($e: [String!]!) { push_subscriptions(where: {email: {_in: $e}}) { endpoint p256dh auth } }",
        "variables": { "e": emails },
    })).await?;
    let subs: Vec<Subscription> = subs
        .pointer("/data/push_subscriptions")
        .and_then(|a| a.as_array())
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
        .unwrap_or_default();
    let recipients = subs.len();
    let mut sent = 0usize;
    let mut stale: Vec<String> = Vec::new();
    for sub in &subs {
        match push::send(cfg, client, sub, payload.as_bytes()).await {
            Ok(status) if (200..300).contains(&status) => sent += 1,
            Ok(404) | Ok(410) => stale.push(sub.endpoint.clone()),
            // Log only the endpoint origin: its path segment is a per-user secret.
            Ok(status) => eprintln!(
                "push send -> {status} ({})",
                sub.endpoint
                    .split('/')
                    .take(3)
                    .collect::<Vec<_>>()
                    .join("/")
            ),
            Err(e) => eprintln!("push send error: {e}"),
        }
    }
    let _ = delete_by_endpoints(cfg, client, &stale).await;
    Ok((recipients, sent))
}

// --- reply: push to the author of the node being commented on ----------------

pub async fn reply(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match reply_inner(cfg, client, query, bearer).await {
        Ok((recipients, sent)) => crate::json(
            StatusCode::OK,
            json!({ "ok": true, "recipients": recipients, "sent": sent }).to_string(),
        ),
        Err(e) => crate::error_json("push reply", e),
    }
}

async fn reply_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(usize, usize), String> {
    let params = crate::util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    let parent = get("parent")
        .filter(|s| !s.is_empty())
        .ok_or("missing parent")?;
    let title = get("title").unwrap_or_else(|| "RadikalWiki".into());
    let body = get("body").unwrap_or_default();
    let url = get("url").unwrap_or_default();

    let (uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    // The node being commented on: whose author should hear about the reply.
    let node = crate::auth::admin_gql(
        cfg,
        client,
        json!({
            "query": "query($id: uuid!) { node(id: $id) { ownerId contextId } }",
            "variables": { "id": parent },
        }),
    )
    .await?;
    let owner_id = match node.pointer("/data/node/ownerId").and_then(|v| v.as_str()) {
        Some(o) => o.to_string(),
        None => return Ok((0, 0)), // anonymous / ownerless node: nobody to notify
    };
    if owner_id == uid {
        return Ok((0, 0)); // don't notify yourself about your own comment
    }
    let ctx = node
        .pointer("/data/node/contextId")
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| parent.clone());

    // Anti-abuse: only an active member of the node's context may ping its author.
    let member = crate::auth::admin_gql(cfg, client, json!({
        "query": "query($c: uuid!, $e: String!) { members(where: {parentId: {_eq: $c}, email: {_eq: $e}, active: {_eq: true}}, limit: 1) { id } }",
        "variables": { "c": ctx, "e": email },
    })).await?;
    if member
        .pointer("/data/members")
        .and_then(|a| a.as_array())
        .map(|a| a.is_empty())
        .unwrap_or(true)
    {
        return Err("not a member of this context".into());
    }

    let owner = crate::auth::admin_gql(
        cfg,
        client,
        json!({
            "query": "query($id: uuid!) { user(id: $id) { email } }",
            "variables": { "id": owner_id },
        }),
    )
    .await?;
    let owner_email = match owner.pointer("/data/user/email").and_then(|v| v.as_str()) {
        Some(e) if e != email => e.to_string(),
        _ => return Ok((0, 0)),
    };

    let payload = json!({ "title": title, "body": body, "url": url }).to_string();
    push_to_emails(cfg, client, &[owner_email], &payload).await
}

// --- notify -----------------------------------------------------------------

pub async fn notify(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Response<Body> {
    match notify_inner(cfg, client, query, bearer).await {
        Ok((recipients, sent)) => crate::json(
            StatusCode::OK,
            json!({ "ok": true, "recipients": recipients, "sent": sent }).to_string(),
        ),
        Err(e) => crate::error_json("push notify", e),
    }
}

async fn notify_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(usize, usize), String> {
    let params = crate::util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    let context = get("context")
        .filter(|s| !s.is_empty())
        .ok_or("missing context")?;
    let title = get("title").unwrap_or_else(|| "RadikalWiki".into());
    let body = get("body").unwrap_or_default();
    let url = get("url").unwrap_or_default();

    let (_uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    // Only a context owner may notify its members (anti-spam).
    let owner = crate::auth::admin_gql(cfg, client, json!({
        "query": "query($c: uuid!, $e: String!) { members(where: {parentId: {_eq: $c}, email: {_eq: $e}, owner: {_eq: true}}, limit: 1) { id } }",
        "variables": { "c": context, "e": email },
    })).await?;
    let is_owner = owner
        .pointer("/data/members")
        .and_then(|a| a.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);
    if !is_owner {
        return Err("not a context owner".into());
    }

    // Recipients = active, accepted members other than the sender.
    let members = crate::auth::admin_gql(cfg, client, json!({
        "query": "query($c: uuid!) { members(where: {parentId: {_eq: $c}, active: {_eq: true}, accepted: {_eq: true}}) { email } }",
        "variables": { "c": context },
    })).await?;
    let emails: Vec<String> = members
        .pointer("/data/members")
        .and_then(|a| a.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|m| m.get("email").and_then(|e| e.as_str()))
                .filter(|e| *e != email)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();
    let payload = json!({ "title": title, "body": body, "url": url }).to_string();
    push_to_emails(cfg, client, &emails, &payload).await
}
