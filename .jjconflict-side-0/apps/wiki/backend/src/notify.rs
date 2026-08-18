//! Web Push subscription storage and fan-out.
//!
//!   POST /push/subscribe?endpoint=&p256dh=&auth=   store the caller's subscription
//!   POST /push/unsubscribe?endpoint=               drop it
//!   POST /push/notify?context=&title=&body=&url=   push to a context's members
//!
//! Membership in this app is by email, so a subscription is stored with the user's
//! email and the fan-out joins on it: a context owner may notify the active members
//! of that context. Encryption + VAPID live in [`crate::push`].

use crate::error::AppError;
use crate::oauth::Config;
use crate::push;
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
        Err(e) => e.respond("push subscribe"),
    }
}

async fn subscribe_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(), AppError> {
    let params = crate::util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    let endpoint = get("endpoint")
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing endpoint".into()))?;
    // SSRF guard: never store an endpoint the backend must not POST to.
    if !push::endpoint_allowed(&endpoint) {
        return Err(AppError::BadRequest("disallowed push endpoint".into()));
    }
    let p256dh = get("p256dh")
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing p256dh".into()))?;
    let auth = get("auth")
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing auth".into()))?;
    let (uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    crate::store::upsert_push_subscription(cfg, client, &uid, &email, &endpoint, &p256dh, &auth)
        .await?;
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
        Err(e) => e.respond("push unsubscribe"),
    }
}

async fn unsubscribe_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(), AppError> {
    let params = crate::util::parse_query(query);
    let endpoint = params
        .iter()
        .find(|(k, _)| k == "endpoint")
        .map(|(_, v)| v.clone())
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing endpoint".into()))?;
    // Must be signed in, but a subscription is device-owned: drop it by endpoint.
    let _ = crate::auth::caller(cfg, client, query, bearer).await?;
    crate::store::delete_subscriptions_by_endpoint(cfg, client, &[endpoint]).await?;
    Ok(())
}

/// Fetch the subscriptions for a set of member emails, push `payload` to each,
/// and prune any that report gone (404/410). Returns (recipients, sent).
async fn push_to_emails(
    cfg: &Config,
    client: &reqwest::Client,
    emails: &[String],
    payload: &str,
) -> Result<(usize, usize), AppError> {
    if emails.is_empty() {
        return Ok((0, 0));
    }
    let subs = crate::store::subscriptions_for_emails(cfg, client, emails).await?;
    let recipients = subs.len();
    let bytes = payload.as_bytes();
    // Deliver concurrently instead of one blocking HTTPS round-trip at a time: a
    // context with N members took N × ~200ms sequentially. Member counts are small
    // (dozens), so unbounded join is fine; reqwest's pool caps per-host sockets.
    let outcomes: Vec<(String, Result<u16, String>)> =
        futures::future::join_all(subs.iter().map(|sub| async move {
            (
                sub.endpoint.clone(),
                push::send(cfg, client, sub, bytes).await,
            )
        }))
        .await;

    let mut sent = 0usize;
    let mut stale: Vec<String> = Vec::new();
    for (endpoint, res) in outcomes {
        match res {
            Ok(status) if (200..300).contains(&status) => sent += 1,
            Ok(404) | Ok(410) => stale.push(endpoint),
            // Log only the endpoint origin: its path segment is a per-user secret.
            Ok(status) => tracing::error!(
                "push send -> {status} ({})",
                endpoint.split('/').take(3).collect::<Vec<_>>().join("/")
            ),
            Err(e) => tracing::error!("push send error: {e}"),
        }
    }
    let _ = crate::store::delete_subscriptions_by_endpoint(cfg, client, &stale).await;
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
        Err(e) => e.respond("push reply"),
    }
}

async fn reply_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(usize, usize), AppError> {
    let params = crate::util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    let parent = get("parent")
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing parent".into()))?;
    let title = get("title").unwrap_or_else(|| "RadikalWiki".into());
    let body = get("body").unwrap_or_default();
    let url = get("url").unwrap_or_default();

    let (uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    // The node being commented on: whose author should hear about the reply.
    let node = crate::store::node_owner_and_context(cfg, client, &parent)
        .await?
        .unwrap_or(crate::store::NodeOwnerContext {
            owner_id: None,
            context_id: None,
        });
    let owner_id = match node.owner_id {
        Some(o) => o,
        None => return Ok((0, 0)), // anonymous / ownerless node: nobody to notify
    };
    if owner_id == uid {
        return Ok((0, 0)); // don't notify yourself about your own comment
    }
    let ctx = node.context_id.unwrap_or_else(|| parent.clone());

    // Anti-abuse: only an active member of the node's context may ping its author.
    // Shared predicate: now honours the durable node_id binding, not just the email.
    let principal = crate::auth::Principal {
        uid: uid.clone(),
        email: email.clone(),
    };
    if !crate::auth::is_active_member(cfg, client, &ctx, &principal).await? {
        return Err(AppError::Forbidden("not a member of this context".into()));
    }

    // INTERIM query, deliberately inline (not in store.rs): looks up the NHost
    // users table by id for an email, which dies with NHost identity at cutover.
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
        Err(e) => e.respond("push notify"),
    }
}

async fn notify_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<(usize, usize), AppError> {
    let params = crate::util::parse_query(query);
    let get = |k: &str| {
        params
            .iter()
            .find(|(pk, _)| pk == k)
            .map(|(_, v)| v.clone())
    };
    let context = get("context")
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing context".into()))?;
    let title = get("title").unwrap_or_else(|| "RadikalWiki".into());
    let body = get("body").unwrap_or_default();
    let url = get("url").unwrap_or_default();

    let (uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    // Only an active context owner may notify its members (anti-spam) — shared predicate.
    let principal = crate::auth::Principal {
        uid,
        email: email.clone(),
    };
    if !crate::auth::is_active_owner(cfg, client, &context, &principal).await? {
        return Err(AppError::Forbidden("not a context owner".into()));
    }

    // Recipients = active, accepted members other than the sender.
    let emails: Vec<String> = crate::store::active_member_emails(cfg, client, &context)
        .await?
        .into_iter()
        .filter(|e| *e != email)
        .collect();
    let payload = json!({ "title": title, "body": body, "url": url }).to_string();
    push_to_emails(cfg, client, &emails, &payload).await
}
