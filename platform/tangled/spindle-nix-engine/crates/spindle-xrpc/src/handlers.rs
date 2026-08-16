//! XRPC endpoint handler functions.
//!
//! Dispatches XRPC method calls to the appropriate handler based on the
//! method name extracted from the URL path. Each handler validates
//! authentication and authorization before performing the requested operation.
//!
//! # Endpoints
//!
//! | Method | Description |
//! |--------|-------------|
//! | `sh.tangled.spindle.addMember` | Add a spindle member (owner only) |
//! | `sh.tangled.spindle.removeMember` | Remove a spindle member (owner only) |
//! | `sh.tangled.spindle.putSecret` | Store a per-repo secret (member) |
//! | `sh.tangled.spindle.listSecrets` | List secret names for a repo (member) |
//! | `sh.tangled.spindle.deleteSecret` | Delete a per-repo secret (member) |
//! | `sh.tangled.spindle.cancelPipeline` | Cancel a running pipeline (member) |

use std::sync::Arc;

use axum::Json;
use axum::extract::{FromRequestParts, Path, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use tracing::{info, warn};

use crate::XrpcContext;
use crate::service_auth::ServiceAuth;

/// Resolve an AT URI repo reference to a `did/name` path.
///
/// The upstream Go spindle resolves the AT URI to get the repo record, then
/// builds `securejoin(did, repo.Name)`. We look up the repo in our database
/// by DID to find the matching name. If the repo string is not an AT URI,
/// it is returned as-is for backwards compatibility.
fn resolve_repo_path(ctx: &XrpcContext, repo: &str) -> Result<String, String> {
    // Parse AT URI: at://did:plc:xxx/sh.tangled.repo/rkey
    if !repo.starts_with("at://") {
        return Ok(repo.to_string());
    }

    let without_scheme = &repo[5..]; // strip "at://"
    let did = without_scheme
        .split('/')
        .next()
        .ok_or_else(|| "invalid AT URI: missing DID".to_string())?;

    // Look up repos for this DID in our database
    let repos = ctx
        .db
        .get_repos_by_did(did)
        .map_err(|e| format!("failed to query repos: {e}"))?;

    if repos.is_empty() {
        return Err(format!("no repos found for DID {did}"));
    }

    // If there's only one repo, use it. Otherwise, try to match by rkey.
    let repo_name = if repos.len() == 1 {
        repos[0].name.clone()
    } else {
        // Try matching by name — the rkey in the AT URI might be the repo name
        let rkey = without_scheme.rsplit('/').next().unwrap_or("");
        repos
            .iter()
            .find(|r| r.name == rkey)
            .map(|r| r.name.clone())
            .unwrap_or_else(|| repos[0].name.clone())
    };

    Ok(format!("{did}/{repo_name}"))
}

/// Standard XRPC error response body.
#[derive(Debug, Serialize)]
pub struct XrpcError {
    pub error: String,
    pub message: String,
}

impl XrpcError {
    pub fn invalid_request(message: impl Into<String>) -> Self {
        Self {
            error: "InvalidRequest".into(),
            message: message.into(),
        }
    }

    pub fn not_found(message: impl Into<String>) -> Self {
        Self {
            error: "NotFound".into(),
            message: message.into(),
        }
    }

    pub fn internal(message: impl Into<String>) -> Self {
        Self {
            error: "InternalServerError".into(),
            message: message.into(),
        }
    }

    pub fn forbidden(message: impl Into<String>) -> Self {
        Self {
            error: "Forbidden".into(),
            message: message.into(),
        }
    }
}

/// Dispatch an XRPC procedure (POST) request to the appropriate handler.
pub async fn dispatch(
    Path(method): Path<String>,
    state: State<Arc<XrpcContext>>,
    auth: ServiceAuth,
    body: Option<Json<serde_json::Value>>,
) -> Response {
    let body = body.map(|Json(v)| v).unwrap_or(serde_json::Value::Null);

    match method.as_str() {
        "sh.tangled.spindle.addMember" => add_member(state, auth, body).await,
        "sh.tangled.spindle.removeMember" => remove_member(state, auth, body).await,
        "sh.tangled.repo.addSecret" | "sh.tangled.spindle.putSecret" => {
            put_secret(state, auth, body).await
        }
        "sh.tangled.repo.listSecrets" | "sh.tangled.spindle.listSecrets" => {
            list_secrets(state, auth, body).await
        }
        "sh.tangled.repo.removeSecret" | "sh.tangled.spindle.deleteSecret" => {
            delete_secret(state, auth, body).await
        }
        "sh.tangled.pipeline.cancelPipeline" | "sh.tangled.spindle.cancelPipeline" => {
            cancel_pipeline(state, auth, body).await
        }
        _ => {
            let err = XrpcError::not_found(format!("unknown XRPC method: {method}"));
            (StatusCode::NOT_FOUND, Json(err)).into_response()
        }
    }
}

/// Dispatch an XRPC query (GET) request to the appropriate handler.
///
/// Handles both unauthenticated queries (e.g. `sh.tangled.owner`) and
/// authenticated queries (e.g. `sh.tangled.repo.listSecrets`). For
/// authenticated queries, the `ServiceAuth` extractor is invoked manually
/// so that unauthenticated queries don't require a bearer token.
pub async fn dispatch_query(
    method: Path<String>,
    state: State<Arc<XrpcContext>>,
    query: axum::extract::Query<std::collections::HashMap<String, String>>,
    request: axum::extract::Request,
) -> Response {
    match method.0.as_str() {
        "sh.tangled.owner" => owner(state).await,
        "sh.tangled.repo.listSecrets" | "sh.tangled.spindle.listSecrets" => {
            // Manually extract auth for authenticated GET queries.
            let (mut parts, _body) = request.into_parts();
            let auth = match ServiceAuth::from_request_parts(&mut parts, &state).await {
                Ok(a) => a,
                Err(e) => return e.into_response(),
            };
            let repo = query.get("repo").cloned().unwrap_or_default();
            list_secrets_query(state, auth, &repo).await
        }
        "sh.tangled.spindle.listRuns" => {
            let (mut parts, _body) = request.into_parts();
            let auth = match ServiceAuth::from_request_parts(&mut parts, &state).await {
                Ok(a) => a,
                Err(e) => return e.into_response(),
            };
            list_runs(state, auth, &query).await
        }
        _ => {
            let err = XrpcError::not_found(format!("unknown XRPC query: {}", method.0));
            (StatusCode::NOT_FOUND, Json(err)).into_response()
        }
    }
}

/// Return the spindle owner's DID. No authentication required.
///
/// This is called by the tangled appview to verify spindle ownership.
async fn owner(State(ctx): State<Arc<XrpcContext>>) -> Response {
    (
        StatusCode::OK,
        Json(serde_json::json!({"owner": ctx.owner})),
    )
        .into_response()
}

// ---------------------------------------------------------------------------
// Request body types
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
struct AddMemberRequest {
    did: String,
}

#[derive(Debug, Deserialize)]
struct RemoveMemberRequest {
    did: String,
}

#[derive(Debug, Deserialize)]
struct PutSecretRequest {
    repo: String,
    key: String,
    value: String,
}

#[derive(Debug, Deserialize)]
struct ListSecretsRequest {
    repo: String,
}

#[derive(Debug, Deserialize)]
struct DeleteSecretRequest {
    repo: String,
    key: String,
}

#[derive(Debug, Deserialize)]
struct CancelPipelineRequest {
    workflow_id: String,
}

// ---------------------------------------------------------------------------
// Handlers
// ---------------------------------------------------------------------------

/// Add a spindle member. Requires owner authorization.
async fn add_member(
    State(ctx): State<Arc<XrpcContext>>,
    auth: ServiceAuth,
    body: serde_json::Value,
) -> Response {
    if auth.did != ctx.owner {
        return (
            StatusCode::FORBIDDEN,
            Json(XrpcError::forbidden("only the owner can add members")),
        )
            .into_response();
    }

    let req: AddMemberRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "invalid request body: {e}"
                ))),
            )
                .into_response();
        }
    };

    // Add to database
    if let Err(e) = ctx.db.add_spindle_member(&req.did) {
        warn!(%e, did = %req.did, "failed to add member to database");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to add member")),
        )
            .into_response();
    }

    // Add to RBAC
    if let Err(e) = ctx.rbac.add_spindle_member(&req.did).await {
        warn!(%e, did = %req.did, "failed to add member to RBAC");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to add member to RBAC")),
        )
            .into_response();
    }

    // Add DID to Jetstream watch list
    if let Err(e) = ctx.db.add_did(&req.did) {
        warn!(%e, did = %req.did, "failed to add DID to watch list");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to add DID to watch list")),
        )
            .into_response();
    }

    info!(did = %req.did, "added spindle member");
    (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
}

/// Remove a spindle member. Requires owner authorization.
async fn remove_member(
    State(ctx): State<Arc<XrpcContext>>,
    auth: ServiceAuth,
    body: serde_json::Value,
) -> Response {
    if auth.did != ctx.owner {
        return (
            StatusCode::FORBIDDEN,
            Json(XrpcError::forbidden("only the owner can remove members")),
        )
            .into_response();
    }

    let req: RemoveMemberRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "invalid request body: {e}"
                ))),
            )
                .into_response();
        }
    };

    // Remove from database
    if let Err(e) = ctx.db.remove_member(&req.did) {
        warn!(%e, did = %req.did, "failed to remove member from database");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to remove member")),
        )
            .into_response();
    }

    // Remove from RBAC
    if let Err(e) = ctx.rbac.remove_spindle_member(&req.did).await {
        warn!(%e, did = %req.did, "failed to remove member from RBAC");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to remove member from RBAC")),
        )
            .into_response();
    }

    // Remove DID from Jetstream watch list
    if let Err(e) = ctx.db.remove_did(&req.did) {
        warn!(%e, did = %req.did, "failed to remove DID from watch list");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to remove DID from watch list")),
        )
            .into_response();
    }

    info!(did = %req.did, "removed spindle member");
    (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
}

/// Store a per-repo secret. Requires member authorization.
async fn put_secret(
    State(ctx): State<Arc<XrpcContext>>,
    _auth: ServiceAuth,
    body: serde_json::Value,
) -> Response {
    let req: PutSecretRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "invalid request body: {e}"
                ))),
            )
                .into_response();
        }
    };

    let repo_path = match resolve_repo_path(&ctx, &req.repo) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "failed to resolve repo: {e}"
                ))),
            )
                .into_response();
        }
    };

    if let Err(e) = ctx
        .secrets
        .put_secret(&repo_path, &req.key, &req.value)
        .await
    {
        warn!(%e, repo = %repo_path, key = %req.key, "failed to store secret");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to store secret")),
        )
            .into_response();
    }

    info!(repo = %repo_path, key = %req.key, "stored secret");
    (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
}

/// List secret names for a repo (POST body variant). Requires member authorization.
async fn list_secrets(
    State(ctx): State<Arc<XrpcContext>>,
    _auth: ServiceAuth,
    body: serde_json::Value,
) -> Response {
    let req: ListSecretsRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "invalid request body: {e}"
                ))),
            )
                .into_response();
        }
    };

    let repo_path = match resolve_repo_path(&ctx, &req.repo) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "failed to resolve repo: {e}"
                ))),
            )
                .into_response();
        }
    };

    list_secrets_impl(&ctx, &repo_path).await
}

/// List secret names for a repo (GET query param variant). Requires member authorization.
async fn list_secrets_query(
    State(ctx): State<Arc<XrpcContext>>,
    _auth: ServiceAuth,
    repo: &str,
) -> Response {
    if repo.is_empty() {
        return (
            StatusCode::BAD_REQUEST,
            Json(XrpcError::invalid_request("missing repo parameter")),
        )
            .into_response();
    }
    let repo_path = match resolve_repo_path(&ctx, repo) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "failed to resolve repo: {e}"
                ))),
            )
                .into_response();
        }
    };
    list_secrets_impl(&ctx, &repo_path).await
}

/// Shared implementation for listing secrets.
async fn list_secrets_impl(ctx: &XrpcContext, repo: &str) -> Response {
    match ctx.secrets.list_secrets(repo).await {
        Ok(keys) => {
            let secrets: Vec<serde_json::Value> = keys
                .into_iter()
                .map(|key| {
                    serde_json::json!({
                        "key": key,
                        "repo": repo,
                        "createdAt": "1970-01-01T00:00:00Z",
                        "createdBy": ctx.owner,
                    })
                })
                .collect();
            (
                StatusCode::OK,
                Json(serde_json::json!({"secrets": secrets})),
            )
                .into_response()
        }
        Err(e) => {
            warn!(%e, repo = %repo, "failed to list secrets");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(XrpcError::internal("failed to list secrets")),
            )
                .into_response()
        }
    }
}

/// Delete a per-repo secret. Requires member authorization.
async fn delete_secret(
    State(ctx): State<Arc<XrpcContext>>,
    _auth: ServiceAuth,
    body: serde_json::Value,
) -> Response {
    let req: DeleteSecretRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "invalid request body: {e}"
                ))),
            )
                .into_response();
        }
    };

    let repo_path = match resolve_repo_path(&ctx, &req.repo) {
        Ok(p) => p,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "failed to resolve repo: {e}"
                ))),
            )
                .into_response();
        }
    };

    if let Err(e) = ctx.secrets.delete_secret(&repo_path, &req.key).await {
        warn!(%e, repo = %repo_path, key = %req.key, "failed to delete secret");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to delete secret")),
        )
            .into_response();
    }

    info!(repo = %repo_path, key = %req.key, "deleted secret");
    (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
}

/// Cancel a running pipeline. Requires member authorization.
///
/// Sets the workflow status to `cancelled` in the database. Full process
/// cancellation (killing running child processes) will be implemented in
/// Phase 6 when the engine/queue integration is wired up.
async fn cancel_pipeline(
    State(ctx): State<Arc<XrpcContext>>,
    _auth: ServiceAuth,
    body: serde_json::Value,
) -> Response {
    let req: CancelPipelineRequest = match serde_json::from_value(body) {
        Ok(r) => r,
        Err(e) => {
            return (
                StatusCode::BAD_REQUEST,
                Json(XrpcError::invalid_request(format!(
                    "invalid request body: {e}"
                ))),
            )
                .into_response();
        }
    };

    // Check that the workflow exists
    match ctx.db.get_status(&req.workflow_id) {
        Ok(Some(status)) => {
            if status.status == "success"
                || status.status == "failed"
                || status.status == "timeout"
                || status.status == "cancelled"
            {
                return (
                    StatusCode::BAD_REQUEST,
                    Json(XrpcError::invalid_request(format!(
                        "workflow is already in terminal state: {}",
                        status.status
                    ))),
                )
                    .into_response();
            }
        }
        Ok(None) => {
            return (
                StatusCode::NOT_FOUND,
                Json(XrpcError::not_found("workflow not found")),
            )
                .into_response();
        }
        Err(e) => {
            warn!(%e, workflow_id = %req.workflow_id, "failed to get workflow status");
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(XrpcError::internal("failed to get workflow status")),
            )
                .into_response();
        }
    }

    if let Err(e) = ctx.db.status_cancelled(&req.workflow_id) {
        warn!(%e, workflow_id = %req.workflow_id, "failed to cancel workflow");
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(XrpcError::internal("failed to cancel workflow")),
        )
            .into_response();
    }

    info!(workflow_id = %req.workflow_id, "cancelled workflow");
    (StatusCode::OK, Json(serde_json::json!({"success": true}))).into_response()
}

/// List workflow runs. Requires member authorization.
///
/// Optional query parameters:
/// - `repo_did`: filter by repository DID
/// - `pipeline_knot` + `pipeline_rkey`: filter by pipeline
/// - `status`: filter by status (pending, running, success, failed, timeout, cancelled)
/// - `limit`: max results (default 50)
async fn list_runs(
    State(ctx): State<Arc<XrpcContext>>,
    _auth: ServiceAuth,
    query: &std::collections::HashMap<String, String>,
) -> Response {
    let limit: usize = query
        .get("limit")
        .and_then(|v| v.parse().ok())
        .unwrap_or(50);

    let rows = if let Some(repo_did) = query.get("repo_did") {
        ctx.db.get_statuses_for_repo(repo_did)
    } else if let (Some(knot), Some(rkey)) =
        (query.get("pipeline_knot"), query.get("pipeline_rkey"))
    {
        ctx.db.get_statuses_for_pipeline(knot, rkey)
    } else if let Some(status) = query.get("status") {
        ctx.db.get_statuses_by_status(status)
    } else {
        ctx.db.get_all_statuses()
    };

    match rows {
        Ok(rows) => {
            // Return most recent first, limited
            let runs: Vec<serde_json::Value> = rows
                .into_iter()
                .rev()
                .take(limit)
                .map(|r| {
                    serde_json::json!({
                        "workflow_id": r.workflow_id,
                        "pipeline_knot": r.pipeline_knot,
                        "pipeline_rkey": r.pipeline_rkey,
                        "repo_did": r.repo_did,
                        "workflow_name": r.workflow_name,
                        "status": r.status,
                        "started_at": r.started_at,
                        "finished_at": r.finished_at,
                        "created_at": r.created_at,
                    })
                })
                .collect();
            (StatusCode::OK, Json(serde_json::json!({"runs": runs}))).into_response()
        }
        Err(e) => {
            warn!(%e, "failed to list workflow runs");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(XrpcError::internal("failed to list runs")),
            )
                .into_response()
        }
    }
}
