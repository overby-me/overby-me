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

use crate::error::AppError;
use crate::oauth::Config;
use axum::{body::Body, response::Response};
use http::StatusCode;
use serde_json::json;

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
        Err(e) => e.respond("member claim"),
    }
}

/// Bind the member identified by `?claim=<token>` to the caller. Returns the
/// context (parent) id so the app can navigate there.
async fn claim_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<String, AppError> {
    let params = crate::util::parse_query(query);
    let claim_token = params
        .iter()
        .find(|(k, _)| k == "claim" || k == "member_token")
        .map(|(_, v)| v.clone())
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing claim token".into()))?;
    let uid = crate::auth::caller_uid(cfg, query, bearer)?;

    let member = crate::store::member_by_claim_token(cfg, client, &claim_token)
        .await?
        .ok_or(AppError::BadRequest("invalid or expired claim link".into()))?;
    let context = member
        .parent_id
        .ok_or(AppError::BadRequest("member has no context".into()))?;

    // Already claimed? Idempotent for the same user; refuse for anyone else.
    if let Some(existing) = &member.node_id {
        if *existing == uid {
            return Ok(context);
        }
        return Err(AppError::Conflict(
            "this invitation has already been claimed".into(),
        ));
    }

    // Bind (guarded on nodeId still null, so a race can't double-claim).
    if !crate::store::bind_member_to_user(cfg, client, &member.id, &uid).await? {
        return Err(AppError::Conflict(
            "this invitation has already been claimed".into(),
        ));
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
        Err(e) => e.respond("member claim-link"),
    }
}

/// Return a member's secret claim token, but only to an owner of that member's
/// context (so an owner can build/share the `?claim=` link).
async fn claim_link_inner(
    cfg: &Config,
    client: &reqwest::Client,
    query: Option<&str>,
    bearer: Option<&str>,
) -> Result<String, AppError> {
    let params = crate::util::parse_query(query);
    let member_id = params
        .iter()
        .find(|(k, _)| k == "member")
        .map(|(_, v)| v.clone())
        .filter(|s| !s.is_empty())
        .ok_or(AppError::BadRequest("missing member id".into()))?;
    let (uid, email) = crate::auth::caller(cfg, client, query, bearer).await?;

    let member = crate::store::member_claim_token(cfg, client, &member_id)
        .await?
        .ok_or(AppError::BadRequest("member not found".into()))?;
    let context = member
        .parent_id
        .ok_or(AppError::BadRequest("member has no context".into()))?;
    let claim_token = member
        .claim_token
        .ok_or(AppError::BadRequest("member has no claim token".into()))?;

    // Owner check via the shared predicate: an active owner (by node_id OR email) or
    // the context node's own owner. Now also requires the owner member to be active.
    let principal = crate::auth::Principal { uid, email };
    if !crate::auth::is_active_owner(cfg, client, &context, &principal).await? {
        return Err(AppError::Forbidden("not a context owner".into()));
    }
    Ok(claim_token)
}
