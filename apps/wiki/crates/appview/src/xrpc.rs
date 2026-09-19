//! The native XRPC serving layer (read side). Methods live at `/xrpc/{nsid}`
//! following the atproto convention (queries are GET with query-string params)
//! and return the canonical domain types as JSON, so there is no premature
//! frontend-shape decision: the AppView serves its real, reconciled entities
//! (`document`, `context`, ...) and the frontend seam that consumes them is a
//! separate, deferred change (nothing here touches the frontend).
//!
//! Single-item reads return the entity object directly; LIST reads wrap the
//! array in a named field (`{ documents: [...] }`, `{ contexts: [...] }`, ...) so
//! the output has an atproto-expressible object schema (a bare top-level array is
//! not a valid lexicon `output.schema`) and leaves room for a future cursor. The
//! method lexicons in `lexicons/com/example/wiki/` are the contract for exactly
//! these shapes.
//!
//! Every read serves only what the caller may read (`crate::authz`): a row they
//! may not read is indistinguishable from one that is not there. Every procedure
//! takes the caller from a session (`crate::session::Caller`), never from the
//! request body.

use crate::AppState;
use crate::authz::Authz;
use crate::session::{Caller, MaybeCaller, Sessions};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::{HeaderMap, StatusCode};
use axum::response::{IntoResponse, Response};
use serde::Deserialize;

/// The `?id=` query param shared by the by-id lookups.
#[derive(Debug, Deserialize)]
pub struct IdParam {
    pub id: String,
}

/// An XRPC error body (`{ "error": ..., "message": ... }`, the atproto shape).
pub(crate) fn err(status: StatusCode, error: &str, message: &str) -> Response {
    (
        status,
        Json(serde_json::json!({ "error": error, "message": message })),
    )
        .into_response()
}

/// `com.example.wiki.getDocument` — a content node (document/folder/file/
/// proposal) by id, with its authors.
pub async fn get_document(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<IdParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.read_document(&p.id, caller.did()).await {
        Ok(Some(doc)) => (StatusCode::OK, Json(doc)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "NotFound", "no such document"),
        Err(e) => {
            tracing::error!("getDocument failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `com.example.wiki.getContext` — a group/event context by id.
pub async fn get_context(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<IdParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.read_context(&p.id, caller.did()).await {
        Ok(Some(ctx)) => (StatusCode::OK, Json(ctx)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "NotFound", "no such context"),
        Err(e) => {
            tracing::error!("getContext failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?path=a/b/c` — the slugs from the root, as a URL carries them.
#[derive(Debug, Deserialize)]
pub struct PathParam {
    pub path: String,
}

/// `com.example.wiki.resolveNode` — the context or document a path names.
pub async fn resolve_node(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<PathParam>,
) -> Response {
    // Empty segments are dropped, so `/a//b/` names what `a/b` names.
    let path = p
        .path
        .split('/')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    let store = crate::Store::new(state.db.clone());
    match store.resolve_path(&path, caller.did()).await {
        Ok(Some(node)) => (StatusCode::OK, Json(node)).into_response(),
        Ok(None) => err(StatusCode::NOT_FOUND, "NotFound", "no node at that path"),
        Err(e) => {
            tracing::error!("resolveNode failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?path=a/b/c` or `?id=<id>`.
#[derive(Debug, Deserialize)]
pub struct NodeParam {
    #[serde(default)]
    pub path: Option<String>,
    #[serde(default)]
    pub id: Option<String>,
}

/// `com.example.wiki.getNode` — a node with what every screen draws around it:
/// its children of either kind, the way down to it, and what the caller may do
/// here. One call where the interim makes several.
pub async fn get_node(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<NodeParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    let failed = |e: crate::DbError| {
        tracing::error!("getNode failed: {e}");
        err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
    };
    let found = match (&p.path, &p.id) {
        (Some(path), None) => {
            let path = path
                .split('/')
                .filter(|s| !s.is_empty())
                .collect::<Vec<_>>()
                .join("/");
            store.resolve_path(&path, caller.did()).await
        }
        (None, Some(id)) => store.read_node(id, caller.did()).await,
        _ => return invalid("give a path or an id, not both"),
    };
    let node = match found {
        Ok(Some(node)) => node,
        Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such node"),
        Err(e) => return failed(e),
    };
    let children = match store.children(node.id(), caller.did()).await {
        Ok(children) => children,
        Err(e) => return failed(e),
    };
    let crumbs = match store.crumbs(&node.place().path, caller.did()).await {
        Ok(crumbs) => crumbs,
        Err(e) => return failed(e),
    };
    let membership = match caller.did() {
        Some(did) => match Authz::new(state.db.clone())
            .membership(node.context_id(), did)
            .await
        {
            Ok(membership) => membership,
            Err(e) => return failed(e),
        },
        None => None,
    };
    let viewer = serde_json::json!({
        "is_owner": caller.did().is_some() && caller.did() == node.place().owner_did.as_deref(),
        "is_member": membership.is_some(),
        "is_context_owner": membership.is_some_and(|m| m.role == wiki_domain_types::Role::Owner),
        "can_vote": membership.is_some_and(|m| m.active),
    });
    (
        StatusCode::OK,
        Json(serde_json::json!({
            "node": node, "children": children, "crumbs": crumbs, "viewer": viewer
        })),
    )
        .into_response()
}

/// `?parent=<id>`.
#[derive(Debug, Deserialize)]
pub struct ParentParam {
    pub parent: String,
}

/// `com.example.wiki.listChildren` — the child documents under a node.
pub async fn list_children(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<ParentParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.list_children(&p.parent, caller.did()).await {
        Ok(docs) => (
            StatusCode::OK,
            Json(serde_json::json!({ "documents": docs })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("listChildren failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `com.example.wiki.listContexts` — the top-level groups/events.
pub async fn list_contexts(State(state): State<AppState>, caller: MaybeCaller) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.list_root_contexts(caller.did()).await {
        Ok(ctxs) => (
            StatusCode::OK,
            Json(serde_json::json!({ "contexts": ctxs })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("listContexts failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?limit=<n>` (default 20).
#[derive(Debug, Deserialize)]
pub struct RecentParam {
    #[serde(default)]
    pub limit: Option<i64>,
}

/// `com.example.wiki.listRecent` — the newest documents across contexts.
pub async fn list_recent(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<RecentParam>,
) -> Response {
    let limit = p.limit.unwrap_or(20).clamp(1, 200);
    let store = crate::Store::new(state.db.clone());
    match store.list_recent(limit, caller.did()).await {
        Ok(docs) => (
            StatusCode::OK,
            Json(serde_json::json!({ "documents": docs })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("listRecent failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?q=<query>`.
#[derive(Debug, Deserialize)]
pub struct SearchParam {
    pub q: String,
}

/// `com.example.wiki.search` — documents matching a title/content substring.
pub async fn search(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<SearchParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.search_documents(&p.q, caller.did()).await {
        Ok(docs) => (
            StatusCode::OK,
            Json(serde_json::json!({ "documents": docs })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("search failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?on=<id>`.
#[derive(Debug, Deserialize)]
pub struct OnParam {
    pub on: String,
}

/// `com.example.wiki.getComments` — the comment thread on a node.
pub async fn get_comments(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<OnParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.get_comments(&p.on, caller.did()).await {
        Ok(comments) => (
            StatusCode::OK,
            Json(serde_json::json!({ "comments": comments })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("getComments failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `?subject=<at-uri>`.
#[derive(Debug, Deserialize)]
pub struct SubjectParam {
    pub subject: String,
}

/// `com.example.wiki.getReactions` — the reactions on a subject (by at-uri).
pub async fn get_reactions(
    State(state): State<AppState>,
    Query(p): Query<SubjectParam>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.get_reactions(&p.subject).await {
        Ok(reactions) => (
            StatusCode::OK,
            Json(serde_json::json!({ "reactions": reactions })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("getReactions failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

// ---------------------------------------------------------------------------
// The session itself.
// ---------------------------------------------------------------------------

#[derive(Debug, Deserialize)]
pub struct CreateSessionBody {
    pub code: String,
}

/// `com.example.wiki.createSession` (procedure) — redeem the one-time code
/// `/callback` handed the browser for a session. The only unauthenticated
/// procedure: the code is the credential.
pub async fn create_session(
    State(state): State<AppState>,
    Json(body): Json<CreateSessionBody>,
) -> Response {
    let sessions = Sessions::new(state.db.clone());
    let did = match sessions.redeem_code(&body.code).await {
        Ok(Some(did)) => did,
        Ok(None) => {
            return err(
                StatusCode::BAD_REQUEST,
                "InvalidCode",
                "the login code is unknown, expired, or already used",
            );
        }
        Err(e) => return write_failed("createSession", e),
    };
    match sessions.create(&did).await {
        Ok(session) => (
            StatusCode::OK,
            Json(serde_json::json!({ "session": session, "did": did })),
        )
            .into_response(),
        Err(e) => write_failed("createSession", e),
    }
}

/// `com.example.wiki.getSession` — who the presented session belongs to.
pub async fn get_session(State(state): State<AppState>, caller: Caller) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store.read_user(&caller.did).await {
        Ok(Some(user)) => (StatusCode::OK, Json(user)).into_response(),
        // Login writes the user row before the session, so the user was deleted since.
        Ok(None) => err(StatusCode::UNAUTHORIZED, "InvalidToken", "no such user"),
        Err(e) => {
            tracing::error!("getSession failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

/// `com.example.wiki.deleteSession` (procedure) — sign out: ends the presented
/// session and no other, so a phone stays signed in when a laptop signs out.
pub async fn delete_session(
    State(state): State<AppState>,
    _caller: Caller,
    headers: HeaderMap,
) -> Response {
    let Some(token) = crate::session::bearer(&headers) else {
        return err(StatusCode::UNAUTHORIZED, "AuthRequired", "no session");
    };
    match Sessions::new(state.db.clone()).revoke(token).await {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => write_failed("deleteSession", e),
    }
}

// ---------------------------------------------------------------------------
// Write side: the caller authors content in a context they are a member of.
// ---------------------------------------------------------------------------

fn forbidden(message: &str) -> Response {
    err(StatusCode::FORBIDDEN, "Forbidden", message)
}

fn invalid(message: &str) -> Response {
    err(StatusCode::BAD_REQUEST, "InvalidRequest", message)
}

/// `Ok` if `did` is a member of `context_id`, else the response to send.
async fn require_member(state: &AppState, context_id: &str, did: &str) -> Result<(), Response> {
    match Authz::new(state.db.clone())
        .is_member(context_id, did)
        .await
    {
        Ok(true) => Ok(()),
        Ok(false) => Err(forbidden("not a member of that context")),
        Err(e) => Err(write_failed("membership check", e)),
    }
}

fn wrote(id: String) -> Response {
    (StatusCode::OK, Json(serde_json::json!({ "id": id }))).into_response()
}

fn write_failed(what: &str, e: impl std::fmt::Display) -> Response {
    tracing::error!("{what} failed: {e}");
    err(StatusCode::BAD_GATEWAY, "InternalError", "write failed")
}

fn conflict(error: &str, message: &str) -> Response {
    err(StatusCode::CONFLICT, error, message)
}

#[derive(Debug, Deserialize)]
pub struct ClaimMembershipBody {
    pub token: String,
}

/// `com.example.wiki.claimMembership` (procedure) — bind the invitation a
/// `?claim=<token>` link names to the caller. This is how a rostered member,
/// known only by an email address, becomes a DID.
///
/// It binds and nothing more. `active` is an owner's to grant, so a claim must
/// leave it as it found it, or an invitation would mint its own voting rights.
pub async fn claim_membership(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ClaimMembershipBody>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    let member = match store.member_by_claim_token(&body.token).await {
        Ok(Some(member)) => member,
        Ok(None) => {
            return err(
                StatusCode::BAD_REQUEST,
                "InvalidClaim",
                "the claim link is unknown or no longer valid",
            );
        }
        Err(e) => return write_failed("claimMembership", e),
    };
    let Some(context_id) = member.parent_id else {
        return err(StatusCode::BAD_REQUEST, "InvalidClaim", "no context");
    };
    let claimed = || {
        (
            StatusCode::OK,
            Json(serde_json::json!({ "context_id": context_id })),
        )
            .into_response()
    };
    match member.node_id.as_deref() {
        Some(bound) if bound == did => return claimed(),
        Some(_) => return conflict("AlreadyClaimed", "this invitation has been claimed"),
        None => {}
    }
    // One bound row per person per context (the `member_bound` index), so a
    // second invitation cannot be stacked on a membership the caller holds.
    match Authz::new(state.db.clone())
        .is_member(&context_id, &did)
        .await
    {
        Ok(false) => {}
        Ok(true) => return conflict("AlreadyMember", "already a member of that context"),
        Err(e) => return write_failed("claimMembership", e),
    }
    match store.bind_member_to_user(&member.id, &did).await {
        Ok(true) => claimed(),
        // Guarded on the row still being unbound, so a racing claim lost.
        Ok(false) => conflict("AlreadyClaimed", "this invitation has been claimed"),
        Err(e) => write_failed("claimMembership", e),
    }
}

/// `?member=<member id>`.
#[derive(Debug, Deserialize)]
pub struct MemberParam {
    pub member: String,
}

/// `com.example.wiki.getMemberClaimLink` — a member's claim token, for an owner
/// of that member's context to hand out. An unknown member answers as a
/// forbidden one does, so the method is no oracle for member ids.
pub async fn get_member_claim_link(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Query(p): Query<MemberParam>,
) -> Response {
    let refused = || forbidden("not an owner of that member's context");
    let store = crate::Store::new(state.db.clone());
    let info = match store.member_claim_token(&p.member).await {
        Ok(Some(info)) => info,
        Ok(None) => return refused(),
        Err(e) => return write_failed("getMemberClaimLink", e),
    };
    let (Some(context_id), Some(token)) = (info.parent_id, info.claim_token) else {
        return refused();
    };
    match Authz::new(state.db.clone())
        .is_active_owner(&context_id, &did)
        .await
    {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({ "token": token }))).into_response(),
        Ok(false) => refused(),
        Err(e) => write_failed("getMemberClaimLink", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct CreateDocumentBody {
    pub context_id: String,
    #[serde(default)]
    pub parent_id: Option<String>,
    pub kind: String,
    pub title: String,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    /// What the node holds beside its text: a file's id and type, a cover image.
    #[serde(default)]
    pub data: Option<serde_json::Value>,
}

/// `com.example.wiki.createDocument` (procedure) — the caller authors a document.
pub async fn create_document(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CreateDocumentBody>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    let parent_id = body.parent_id.as_deref().unwrap_or(&body.context_id);
    let parent = match store.parent_of(parent_id).await {
        Ok(Some(parent)) if parent.context_id == body.context_id => parent,
        Ok(Some(_)) => return invalid("parent is not in that context"),
        Ok(None) => return invalid("no such parent"),
        Err(e) => return write_failed("createDocument", e),
    };
    let membership = match Authz::new(state.db.clone())
        .membership(&body.context_id, &did)
        .await
    {
        Ok(Some(membership)) => membership,
        Ok(None) => return forbidden("not a member of that context"),
        Err(e) => return write_failed("createDocument", e),
    };
    if let Err(refusal) =
        crate::authz::may_create(&body.kind, &parent.kind, parent.attachable, membership)
    {
        use crate::authz::Refusal;
        return match refusal {
            Refusal::UnknownKind | Refusal::WrongParent => invalid(refusal.message()),
            Refusal::NeedsOwner | Refusal::Locked => forbidden(refusal.message()),
        };
    }
    let content = body.content.as_ref().map(|v| v.to_string());
    let data = body.data.as_ref().map(|v| v.to_string());
    let new = crate::store::NewDocument {
        context_id: &body.context_id,
        parent_id: body.parent_id.as_deref(),
        kind: &body.kind,
        title: &body.title,
        content: content.as_deref(),
        data: data.as_deref(),
        author_did: &did,
    };
    match store.create_document(&new).await {
        Ok(id) => wrote(id),
        Err(crate::store::WriteError::Db(e)) => write_failed("createDocument", e),
        Err(refused) => err(
            StatusCode::BAD_REQUEST,
            "InvalidRequest",
            &refused.to_string(),
        ),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateDocumentBody {
    pub id: String,
    #[serde(default)]
    pub title: Option<String>,
    #[serde(default)]
    pub content: Option<serde_json::Value>,
    #[serde(default)]
    pub data: Option<serde_json::Value>,
    #[serde(default)]
    pub mutable: Option<bool>,
    #[serde(default)]
    pub attachable: Option<bool>,
    #[serde(default)]
    pub idx: Option<i64>,
}

/// A live document the caller may read, with their standing towards it. A
/// document they may not read answers as missing, as it does to a read.
async fn standing_towards(
    state: &AppState,
    id: &str,
    did: &str,
    what: &str,
) -> Result<(crate::store::DocumentMeta, crate::authz::Standing), Response> {
    let store = crate::Store::new(state.db.clone());
    let missing = || err(StatusCode::NOT_FOUND, "NotFound", "no such document");
    match store.read_document(id, Some(did)).await {
        Ok(Some(_)) => {}
        Ok(None) => return Err(missing()),
        Err(e) => return Err(write_failed(what, e)),
    }
    let meta = match store.document_meta(id).await {
        Ok(Some(meta)) => meta,
        Ok(None) => return Err(missing()),
        Err(e) => return Err(write_failed(what, e)),
    };
    let standing = Authz::new(state.db.clone())
        .standing(&meta.context_id, meta.owner_did.as_deref(), did)
        .await
        .map_err(|e| write_failed(what, e))?;
    Ok((meta, standing))
}

/// `com.example.wiki.updateDocument` (procedure) — change a document. The slug
/// stays: a rename keeps the URL people have linked to.
pub async fn update_document(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<UpdateDocumentBody>,
) -> Response {
    let (meta, standing) = match standing_towards(&state, &body.id, &did, "updateDocument").await {
        Ok(found) => found,
        Err(refusal) => return refusal,
    };
    let arranges = body.attachable.is_some()
        || body.idx.is_some()
        || (body.mutable == Some(true) && !meta.mutable);
    if arranges && !standing.may_arrange() {
        return forbidden("only an owner of the context may reorder, lock or reopen");
    }
    if !standing.may_edit(meta.mutable) {
        return forbidden(if meta.mutable {
            "not yours to edit"
        } else {
            "it has been submitted and can no longer be edited"
        });
    }
    let content = body.content.as_ref().map(|v| v.to_string());
    let data = body.data.as_ref().map(|v| v.to_string());
    let patch = crate::store::DocumentPatch {
        title: body.title.as_deref(),
        content: content.as_deref(),
        data: data.as_deref(),
        mutable: body.mutable,
        attachable: body.attachable,
        idx: body.idx,
    };
    match crate::Store::new(state.db.clone())
        .update_document(&body.id, &patch)
        .await
    {
        Ok(true) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Ok(false) => err(StatusCode::NOT_FOUND, "NotFound", "no such document"),
        Err(e) => write_failed("updateDocument", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct DocumentIdBody {
    pub id: String,
}

/// `com.example.wiki.deleteDocument` (procedure) — put a document, and
/// everything under it, in the bin.
pub async fn delete_document(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<DocumentIdBody>,
) -> Response {
    let (meta, standing) = match standing_towards(&state, &body.id, &did, "deleteDocument").await {
        Ok(found) => found,
        Err(refusal) => return refusal,
    };
    if !standing.may_delete() {
        return forbidden("not yours to delete");
    }
    match crate::Store::new(state.db.clone())
        .bin_subtree(&body.id, &meta.path)
        .await
    {
        Ok(binned) => (
            StatusCode::OK,
            Json(serde_json::json!({ "binned": binned })),
        )
            .into_response(),
        Err(e) => write_failed("deleteDocument", e),
    }
}

/// `com.example.wiki.restoreDocument` (procedure) — bring a document back from
/// the bin, with everything that went there with it.
pub async fn restore_document(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<DocumentIdBody>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    // No read reaches the bin, so there is no reader's view to defer to: a
    // caller with no standing towards the document is told it is not there.
    let missing = || {
        err(
            StatusCode::NOT_FOUND,
            "NotFound",
            "nothing in the bin by that id",
        )
    };
    let meta = match store.document_meta(&body.id).await {
        Ok(Some(meta)) => meta,
        Ok(None) => return missing(),
        Err(e) => return write_failed("restoreDocument", e),
    };
    if !meta.binned {
        return missing();
    }
    match Authz::new(state.db.clone())
        .standing(&meta.context_id, meta.owner_did.as_deref(), &did)
        .await
    {
        Ok(standing) if standing.may_delete() => {}
        Ok(_) => return missing(),
        Err(e) => return write_failed("restoreDocument", e),
    }
    let parent_id = meta.parent_id.as_deref().unwrap_or(&meta.context_id);
    match store.parent_of(parent_id).await {
        Ok(Some(_)) => {}
        Ok(None) => return conflict("ParentInBin", "restore what it was in first"),
        Err(e) => return write_failed("restoreDocument", e),
    }
    match store.restore_subtree(&body.id, &meta.path).await {
        Ok(restored) => (
            StatusCode::OK,
            Json(serde_json::json!({ "restored": restored })),
        )
            .into_response(),
        Err(crate::store::WriteError::Db(e)) => write_failed("restoreDocument", e),
        Err(refused) => conflict("PathTaken", &refused.to_string()),
    }
}

/// `?context=<id>`.
#[derive(Debug, Deserialize)]
pub struct ContextParam {
    pub context: String,
}

/// `com.example.wiki.listDeleted` — the bin of a context. An owner of the
/// context sees all of it; anyone else sees what they created and deleted.
pub async fn list_deleted(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Query(p): Query<ContextParam>,
) -> Response {
    let membership = match Authz::new(state.db.clone())
        .membership(&p.context, &did)
        .await
    {
        Ok(membership) => membership,
        Err(e) => return write_failed("listDeleted", e),
    };
    let own_only = !membership.is_some_and(|m| m.role == wiki_domain_types::Role::Owner);
    match crate::Store::new(state.db.clone())
        .list_binned(&p.context, own_only.then_some(did.as_str()))
        .await
    {
        Ok(deleted) => (
            StatusCode::OK,
            Json(serde_json::json!({ "deleted": deleted })),
        )
            .into_response(),
        Err(e) => write_failed("listDeleted", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct PostCommentBody {
    pub on_id: String,
    /// Optional, and only ever checked: the comment's context is the one its
    /// subject is in, whatever the caller says.
    #[serde(default)]
    pub context_id: Option<String>,
    pub text: String,
}

/// `com.example.wiki.postComment` (procedure) — the caller comments on a node.
pub async fn post_comment(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<PostCommentBody>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    // A node the caller may not read answers as missing, as it does to a read.
    let (context_id, kind) = match store.readable_subject(&body.on_id, Some(&did)).await {
        Ok(Some(subject)) => subject,
        Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such node"),
        Err(e) => return write_failed("postComment", e),
    };
    // Content and replies, never a container: a folder has no thread to show it.
    if kind != "comment" && !crate::authz::COMMENTABLE.contains(&kind.as_str()) {
        return invalid("that kind of node takes no comments");
    }
    if body.context_id.as_ref().is_some_and(|c| *c != context_id) {
        return err(
            StatusCode::BAD_REQUEST,
            "InvalidRequest",
            "the node is not in that context",
        );
    }
    if let Err(refusal) = require_member(&state, &context_id, &did).await {
        return refusal;
    }
    match store
        .create_comment(&body.on_id, &context_id, &did, &body.text)
        .await
    {
        Ok(id) => wrote(id),
        Err(e) => write_failed("postComment", e),
    }
}

#[derive(Debug, Deserialize)]
pub struct ReactionBody {
    pub subject: String,
    pub emoji: String,
}

/// `com.example.wiki.addReaction` (procedure) — the caller reacts to a subject.
pub async fn add_reaction(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ReactionBody>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store
        .create_reaction(&body.subject, &did, &body.emoji)
        .await
    {
        Ok(id) => wrote(id),
        Err(e) => write_failed("addReaction", e),
    }
}

/// `com.example.wiki.removeReaction` (procedure) — the caller un-reacts.
pub async fn remove_reaction(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ReactionBody>,
) -> Response {
    let store = crate::Store::new(state.db.clone());
    match store
        .remove_reaction(&body.subject, &did, &body.emoji)
        .await
    {
        Ok(()) => (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response(),
        Err(e) => write_failed("removeReaction", e),
    }
}

#[cfg(test)]
mod tests {
    use crate::{AppState, Config, Db, router};
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    // Split, or the repository's link checker tries to parse an at-uri as a URL.
    const POST_URI: &str = concat!("at:", "//did:plc:alice/com.example.wiki.post/p1");
    const REACTION_URI: &str = concat!("at:", "//did:plc:bob/com.example.wiki.reaction/r1");

    async fn seeded_router() -> axum::Router {
        router(seeded_state().await)
    }

    async fn seeded_state() -> AppState {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let conn = db.acquire().await.expect("conn");
        // c1 is a public group, which anyone may read. c9 is a private one that
        // alice owns and bob is a member of: what the gate is tested against.
        conn.execute_batch(
            "INSERT INTO user (did, handle, display_name) \
               VALUES ('did:plc:alice', 'alice.test', 'Alice');
             INSERT INTO user (did) VALUES ('did:plc:bob');
             INSERT INTO user (did) VALUES ('did:plc:zoe');
             INSERT INTO context (id, kind, name, slug, path, visibility) \
               VALUES ('c1', 'group', 'Group One', 'group-one', 'group-one', 'public');
             INSERT INTO context (id, kind, name, slug, path, parent_id, visibility) \
               VALUES ('c2', 'event', 'Sub Event', 'sub', 'group-one/sub', 'c1', 'public');
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, content) \
               VALUES ('d1', 'c1', 'c1', 'policy', 'Motion', 'motion', 'group-one/motion', \
                       '{\"blocks\":[{\"text\":\"hi\"}]}');
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('d2', 'c1', 'c1', 'document', 'Child Doc', 'child_doc', \
                       'group-one/child_doc');
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', 'did:plc:alice', NULL, 0);
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', NULL, 'Guest', 1);
             INSERT INTO comment (id, on_id, context_id, author_did, text) \
               VALUES ('k1', 'd1', 'c1', 'did:plc:alice', 'Nice motion');
             INSERT INTO context (id, kind, name, slug, path) \
               VALUES ('c9', 'group', 'Closed Group', 'closed', 'closed');
             INSERT INTO context (id, kind, name, slug, path, parent_id) \
               VALUES ('c10', 'event', 'Closed Meeting', 'meeting', 'closed/meeting', 'c9');
             INSERT INTO member (id, user_did, context_id, role, active) \
               VALUES ('m-alice', 'did:plc:alice', 'c9', 'owner', 1);
             INSERT INTO member (id, user_did, context_id, role, active) \
               VALUES ('m-bob', 'did:plc:bob', 'c9', 'member', 1);
             INSERT INTO member (id, user_did, context_id, role, active) \
               VALUES ('m-zoe', 'did:plc:zoe', 'c10', 'member', 1);
             INSERT INTO user (did) VALUES ('did:plc:ivan');
             INSERT INTO member (id, user_did, context_id, role, active) \
               VALUES ('m-ivan', 'did:plc:ivan', 'c9', 'member', 0);
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('s1', 'c9', 'c9', 'document', 'Secret Minutes', 'secret_minutes', \
                       'closed/secret_minutes');
             INSERT INTO comment (id, on_id, context_id, author_did, text) \
               VALUES ('ks', 's1', 'c9', 'did:plc:alice', 'Secret remark');",
        )
        .await
        .expect("seed");
        conn.execute(
            "INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
             VALUES (?1, ?2, 'did:plc:bob', '👍')",
            [REACTION_URI, POST_URI],
        )
        .await
        .expect("seed reaction");
        AppState::new(db, Config::default())
    }

    async fn get(app: axum::Router, uri: &str) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
            .await
            .expect("request");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("body");
        let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    /// Sign `did` in the way `/callback` does: a user row and a session.
    async fn token_for(state: &AppState, did: &str) -> String {
        crate::Store::new(state.db.clone())
            .upsert_user_min(did)
            .await
            .expect("user");
        crate::session::Sessions::new(state.db.clone())
            .create(did)
            .await
            .expect("session")
    }

    /// Make `did` (who must have a user row) an active member of `context`.
    async fn join(state: &AppState, did: &str, context: &str) {
        join_as(state, did, context, "member").await;
    }

    async fn join_as(state: &AppState, did: &str, context: &str, role: &str) {
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO member (id, user_did, context_id, role, active) \
             VALUES (?1, ?2, ?3, ?4, 1)",
            [format!("m-{did}-{context}").as_str(), did, context, role],
        )
        .await
        .expect("join");
    }

    async fn post(
        app: axum::Router,
        uri: &str,
        token: Option<&str>,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let mut b = Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json");
        if let Some(t) = token {
            b = b.header("authorization", format!("Bearer {t}"));
        }
        let resp = app
            .oneshot(b.body(Body::from(body.to_string())).unwrap())
            .await
            .expect("request");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("body");
        let v = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, v)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_document_returns_the_document_with_authors() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getDocument?id=d1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["title"], "Motion");
        assert_eq!(v["kind"], "policy");
        assert_eq!(v["context_id"], "c1");
        // Authorship survives: one DID account + one free-text author, in order.
        let authors = v["authors"].as_array().expect("authors array");
        assert_eq!(authors.len(), 2);
        assert_eq!(authors[0]["kind"], "user");
        assert_eq!(authors[0]["did"], "did:plc:alice");
        assert_eq!(authors[1]["kind"], "free_text");
        assert_eq!(authors[1]["display"], "Guest");
        // The Slate JSON round-trips through the TEXT column.
        assert_eq!(v["content"]["blocks"][0]["text"], "hi");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_context_returns_the_context() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getContext?id=c1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["name"], "Group One");
        assert_eq!(v["kind"], "group");
        assert_eq!(v["slug"], "group-one");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn missing_id_is_a_404_xrpc_error() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getDocument?id=nope",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(v["error"], "NotFound");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn resolve_node_walks_the_slug_path() {
        let app = seeded_router().await;
        let (status, v) = get(app, "/xrpc/com.example.wiki.resolveNode?path=group-one/sub").await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["slug"], "sub");
        assert_eq!(v["name"], "Sub Event");
        // A broken path is a 404.
        let (status, _) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.resolveNode?path=group-one/nope",
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_children_search_recent_return_documents() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.listChildren?parent=c1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let docs = v["documents"].as_array().expect("array");
        assert!(docs.iter().any(|d| d["id"] == "d2"));

        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.search?q=Motion",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            v["documents"]
                .as_array()
                .unwrap()
                .iter()
                .any(|d| d["id"] == "d1")
        );

        let (status, v) = get(seeded_router().await, "/xrpc/com.example.wiki.listRecent").await;
        assert_eq!(status, StatusCode::OK);
        assert!(v["documents"].as_array().unwrap().len() >= 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn list_contexts_returns_only_roots() {
        let (status, v) = get(seeded_router().await, "/xrpc/com.example.wiki.listContexts").await;
        assert_eq!(status, StatusCode::OK);
        let ctxs = v["contexts"].as_array().expect("array");
        // c1 is a root; c2 has a parent and is excluded.
        assert!(ctxs.iter().any(|c| c["id"] == "c1"));
        assert!(!ctxs.iter().any(|c| c["id"] == "c2"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_comments_and_reactions() {
        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getComments?on=d1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let comments = v["comments"].as_array().expect("array");
        assert_eq!(comments.len(), 1);
        assert_eq!(comments[0]["text"], "Nice motion");
        assert_eq!(comments[0]["author"]["did"], "did:plc:alice");

        let (status, v) = get(
            seeded_router().await,
            "/xrpc/com.example.wiki.getReactions?subject=at://did:plc:alice/com.example.wiki.post/p1",
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let reactions = v["reactions"].as_array().expect("array");
        assert_eq!(reactions.len(), 1);
        assert_eq!(reactions[0]["emoji"], "👍");
        assert_eq!(reactions[0]["reactor_did"], "did:plc:bob");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn create_document_by_a_did_then_read_it_back() {
        let state = seeded_state().await;
        let carol = token_for(&state, "did:plc:carol").await;
        join_as(&state, "did:plc:carol", "c1", "owner").await;
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createDocument",
            Some(&carol),
            serde_json::json!({
                "context_id": "c1",
                "kind": "document",
                "title": "Carol's Doc",
                "content": {"blocks": [{"text": "hej"}]}
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let id = v["id"].as_str().expect("id").to_string();

        // Read it back through getDocument on the SAME db: title, author, content.
        let (status, doc) = get(
            router(state.clone()),
            &format!("/xrpc/com.example.wiki.getDocument?id={id}"),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(doc["title"], "Carol's Doc");
        assert_eq!(doc["authors"][0]["did"], "did:plc:carol");
        assert_eq!(doc["content"]["blocks"][0]["text"], "hej");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_write_without_a_bearer_is_401() {
        let (status, v) = post(
            seeded_router().await,
            "/xrpc/com.example.wiki.postComment",
            None,
            serde_json::json!({"on_id": "d1", "context_id": "c1", "text": "hi"}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(v["error"], "AuthRequired");
    }

    /// The placeholder this replaced read the DID out of the header, so anyone
    /// could write as anyone by naming them.
    #[tokio::test(flavor = "current_thread")]
    async fn naming_a_did_is_not_a_credential() {
        let state = seeded_state().await;
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.postComment",
            Some("did:plc:alice"),
            serde_json::json!({"on_id": "d1", "context_id": "c1", "text": "forged"}),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(v["error"], "InvalidToken");
        let (_, v) = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.getComments?on=d1",
        )
        .await;
        assert!(
            !v["comments"]
                .as_array()
                .expect("array")
                .iter()
                .any(|c| c["text"] == "forged"),
            "a forged write landed"
        );
    }

    async fn get_as(app: axum::Router, uri: &str, token: &str) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .uri(uri)
                    .header("authorization", format!("Bearer {token}"))
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .expect("request");
        let status = resp.status();
        let bytes = axum::body::to_bytes(resp.into_body(), 64 * 1024)
            .await
            .expect("body");
        (
            status,
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null),
        )
    }

    #[tokio::test(flavor = "current_thread")]
    async fn get_session_names_the_caller_and_sign_out_ends_it() {
        let state = seeded_state().await;
        let token = token_for(&state, "did:plc:alice").await;
        let session = "/xrpc/com.example.wiki.getSession";

        let (status, v) = get_as(router(state.clone()), session, &token).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["did"], "did:plc:alice");
        assert_eq!(v["handle"], "alice.test");

        let (status, _) = get(router(state.clone()), session).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "getSession is not public");

        let (status, _) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.deleteSession",
            Some(&token),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, v) = get_as(router(state.clone()), session, &token).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_eq!(v["error"], "InvalidToken");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_login_code_buys_one_session() {
        let state = seeded_state().await;
        crate::Store::new(state.db.clone())
            .upsert_user_min("did:plc:frank")
            .await
            .expect("user");
        let code = crate::session::Sessions::new(state.db.clone())
            .issue_code("did:plc:frank")
            .await
            .expect("code");
        let create = "/xrpc/com.example.wiki.createSession";

        let (status, v) = post(
            router(state.clone()),
            create,
            None,
            serde_json::json!({ "code": code }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["did"], "did:plc:frank");
        let session = v["session"].as_str().expect("session").to_string();
        let (status, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getSession",
            &session,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(v["did"], "did:plc:frank");

        let (status, v) = post(
            router(state.clone()),
            create,
            None,
            serde_json::json!({ "code": code }),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a code must not work twice"
        );
        assert_eq!(v["error"], "InvalidCode");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn signing_out_one_device_leaves_the_other_signed_in() {
        let state = seeded_state().await;
        let laptop = token_for(&state, "did:plc:alice").await;
        let phone = token_for(&state, "did:plc:alice").await;
        let (status, _) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.deleteSession",
            Some(&laptop),
            serde_json::json!({}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getSession",
            &phone,
        )
        .await;
        assert_eq!(status, StatusCode::OK);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn post_comment_lands_in_the_thread() {
        let state = seeded_state().await;
        let dave = token_for(&state, "did:plc:dave").await;
        join(&state, "did:plc:dave", "c1").await;
        let (status, _) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.postComment",
            Some(&dave),
            serde_json::json!({"on_id": "d1", "context_id": "c1", "text": "Seconded"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, v) = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.getComments?on=d1",
        )
        .await;
        let comments = v["comments"].as_array().expect("array");
        // The seed comment plus the new one.
        assert!(comments.iter().any(|c| c["text"] == "Seconded"));
        assert!(
            comments
                .iter()
                .any(|c| c["author"]["did"] == "did:plc:dave")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn add_then_remove_reaction_toggles() {
        let state = seeded_state().await;
        let subject = "at://did:plc:x/com.example.wiki.comment/z";
        let eve = token_for(&state, "did:plc:eve").await;
        let add = |emoji: &'static str| {
            let s = state.clone();
            let eve = eve.clone();
            async move {
                post(
                    router(s),
                    "/xrpc/com.example.wiki.addReaction",
                    Some(&eve),
                    serde_json::json!({"subject": subject, "emoji": emoji}),
                )
                .await
            }
        };
        assert_eq!(add("🎉").await.0, StatusCode::OK);
        // Re-adding the same emoji is idempotent (unique triple), still one row.
        assert_eq!(add("🎉").await.0, StatusCode::OK);
        let (_, v) = get(
            router(state.clone()),
            &format!("/xrpc/com.example.wiki.getReactions?subject={subject}"),
        )
        .await;
        assert_eq!(v["reactions"].as_array().unwrap().len(), 1);

        // Remove it.
        let (status, _) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.removeReaction",
            Some(&eve),
            serde_json::json!({"subject": subject, "emoji": "🎉"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, v) = get(
            router(state.clone()),
            &format!("/xrpc/com.example.wiki.getReactions?subject={subject}"),
        )
        .await;
        assert_eq!(v["reactions"].as_array().unwrap().len(), 0);
    }

    // -- The read gate. `s1` and `ks` are in the private group c9 (alice owns it,
    //    bob is a member); `did:plc:zoe` belongs only to the meeting inside it. --

    const SECRET_READS: [&str; 6] = [
        "/xrpc/com.example.wiki.getDocument?id=s1",
        "/xrpc/com.example.wiki.getContext?id=c9",
        "/xrpc/com.example.wiki.resolveNode?path=closed",
        "/xrpc/com.example.wiki.listChildren?parent=c9",
        "/xrpc/com.example.wiki.search?q=Secret",
        "/xrpc/com.example.wiki.getComments?on=s1",
    ];

    /// Whether a response gave away anything from the private group.
    fn leaks(status: StatusCode, body: &serde_json::Value) -> bool {
        let text = body.to_string();
        status == StatusCode::OK && (text.contains("Secret") || text.contains("Closed"))
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_private_context_is_invisible_to_the_signed_out() {
        let state = seeded_state().await;
        for uri in SECRET_READS {
            let (status, v) = get(router(state.clone()), uri).await;
            assert!(
                !leaks(status, &v),
                "{uri} leaked to an anonymous reader: {v}"
            );
        }
        let (_, v) = get(router(state.clone()), "/xrpc/com.example.wiki.listRecent").await;
        assert!(!v.to_string().contains("Secret"), "listRecent leaked: {v}");
        let (_, v) = get(router(state.clone()), "/xrpc/com.example.wiki.listContexts").await;
        assert!(
            !v.to_string().contains("Closed"),
            "listContexts leaked: {v}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_private_context_is_invisible_to_a_signed_in_stranger() {
        let state = seeded_state().await;
        let mallory = token_for(&state, "did:plc:mallory").await;
        for uri in SECRET_READS {
            let (status, v) = get_as(router(state.clone()), uri, &mallory).await;
            assert!(!leaks(status, &v), "{uri} leaked to a stranger: {v}");
        }
    }

    /// A missing document and a forbidden one must look the same, or the 403
    /// itself tells a stranger the document exists.
    #[tokio::test(flavor = "current_thread")]
    async fn a_forbidden_document_answers_exactly_as_a_missing_one() {
        let state = seeded_state().await;
        let forbidden = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.getDocument?id=s1",
        )
        .await;
        let missing = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.getDocument?id=nope",
        )
        .await;
        assert_eq!(forbidden, missing);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_member_reads_their_private_context() {
        let state = seeded_state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        for uri in SECRET_READS {
            let (status, v) = get_as(router(state.clone()), uri, &bob).await;
            assert!(
                leaks(status, &v),
                "{uri} hid the group from its own member: {v}"
            );
        }
        let (_, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.listRecent",
            &bob,
        )
        .await;
        assert!(v.to_string().contains("Secret Minutes"), "{v}");
        let (_, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.listContexts",
            &bob,
        )
        .await;
        assert!(v.to_string().contains("Closed Group"), "{v}");
    }

    /// `active` is voting rights (`crate::authz`). A member without them still
    /// reads the group they belong to.
    #[tokio::test(flavor = "current_thread")]
    async fn a_member_without_voting_rights_still_reads() {
        let state = seeded_state().await;
        let ivan = token_for(&state, "did:plc:ivan").await;
        for uri in SECRET_READS {
            let (status, v) = get_as(router(state.clone()), uri, &ivan).await;
            assert!(leaks(status, &v), "{uri} shut out an inactive member: {v}");
        }
    }

    /// The meeting sits inside a group its member does not belong to. The path
    /// still has to resolve, and the group still has to stay shut.
    #[tokio::test(flavor = "current_thread")]
    async fn a_path_resolves_through_a_context_the_caller_cannot_read() {
        let state = seeded_state().await;
        let zoe = token_for(&state, "did:plc:zoe").await;
        let (status, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.resolveNode?path=closed/meeting",
            &zoe,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["id"], "c10");
        let (status, _) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.resolveNode?path=closed",
            &zoe,
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_author_still_reads_what_they_wrote_after_leaving() {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO user (did) VALUES ('did:plc:erin');
             INSERT INTO document_author (document_id, author_did, ord) \
               VALUES ('s1', 'did:plc:erin', 0);",
        )
        .await
        .expect("seed");
        let erin = token_for(&state, "did:plc:erin").await;
        let (status, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getDocument?id=s1",
            &erin,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, _) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getContext?id=c9",
            &erin,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "authorship of one page opened the whole group"
        );
    }

    // -- The write gate. --

    #[tokio::test(flavor = "current_thread")]
    async fn only_a_member_may_write_into_a_context() {
        let state = seeded_state().await;
        let mallory = token_for(&state, "did:plc:mallory").await;
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createDocument",
            Some(&mallory),
            serde_json::json!({"context_id": "c9", "kind": "document", "title": "Planted"}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        assert_eq!(v["error"], "Forbidden");
        let bob = token_for(&state, "did:plc:bob").await;
        let (_, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.listChildren?parent=c9",
            &bob,
        )
        .await;
        assert!(!v.to_string().contains("Planted"), "a refused write landed");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_member_cannot_hang_a_document_off_another_contexts_tree() {
        let state = seeded_state().await;
        let carol = token_for(&state, "did:plc:carol").await;
        join(&state, "did:plc:carol", "c1").await;
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createDocument",
            Some(&carol),
            serde_json::json!({
                "context_id": "c1", "parent_id": "s1", "kind": "document", "title": "Graft"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        assert_eq!(v["error"], "InvalidRequest");
    }

    /// The comment's context comes from its subject. Were the body believed, a
    /// member of any group could file a comment under it on someone else's page.
    #[tokio::test(flavor = "current_thread")]
    async fn a_comment_cannot_name_its_own_context() {
        let state = seeded_state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.postComment",
            Some(&bob),
            serde_json::json!({"on_id": "d1", "context_id": "c9", "text": "smuggled"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn commenting_needs_membership_and_hides_what_cannot_be_read() {
        let state = seeded_state().await;
        let mallory = token_for(&state, "did:plc:mallory").await;
        let comment = |on: &'static str| {
            let (state, mallory) = (state.clone(), mallory.clone());
            async move {
                post(
                    router(state),
                    "/xrpc/com.example.wiki.postComment",
                    Some(&mallory),
                    serde_json::json!({"on_id": on, "text": "hello"}),
                )
                .await
            }
        };
        let (status, _) = comment("d1").await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "readable, but not hers to write"
        );
        let (status, _) = comment("s1").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a private page must not be confirmed"
        );
    }

    // -- Claiming an invitation. `inv` is a pending row in the private group. --

    async fn invite(state: &AppState, id: &str, token: &str, active: i64) {
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO member (id, context_id, role, active, email, claim_token) \
             VALUES (?1, 'c9', 'member', ?2, ?3, ?4)",
            vec![
                turso::Value::Text(id.to_string()),
                turso::Value::Integer(active),
                turso::Value::Text(format!("{id}@x.dk")),
                turso::Value::Text(token.to_string()),
            ],
        )
        .await
        .expect("invite");
    }

    async fn claim(state: &AppState, who: &str, token: &str) -> (StatusCode, serde_json::Value) {
        post(
            router(state.clone()),
            "/xrpc/com.example.wiki.claimMembership",
            Some(who),
            serde_json::json!({ "token": token }),
        )
        .await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn claiming_an_invitation_opens_the_context_to_the_claimant_only() {
        let state = seeded_state().await;
        invite(&state, "inv", "tok-inv", 1).await;
        let nina = token_for(&state, "did:plc:nina").await;
        let secret = "/xrpc/com.example.wiki.getDocument?id=s1";
        let (before, _) = get_as(router(state.clone()), secret, &nina).await;
        assert_eq!(before, StatusCode::NOT_FOUND);

        let (status, v) = claim(&state, &nina, "tok-inv").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["context_id"], "c9");
        let (after, _) = get_as(router(state.clone()), secret, &nina).await;
        assert_eq!(after, StatusCode::OK);

        let (again, _) = claim(&state, &nina, "tok-inv").await;
        assert_eq!(again, StatusCode::OK, "a repeated claim is not an error");

        let otto = token_for(&state, "did:plc:otto").await;
        let (status, v) = claim(&state, &otto, "tok-inv").await;
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(v["error"], "AlreadyClaimed");
        let (status, _) = get_as(router(state.clone()), secret, &otto).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a refused claim let someone in"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_claim_mints_no_voting_rights() {
        let state = seeded_state().await;
        invite(&state, "inv", "tok-inv", 0).await;
        let nina = token_for(&state, "did:plc:nina").await;
        let (status, _) = claim(&state, &nina, "tok-inv").await;
        assert_eq!(status, StatusCode::OK);
        let authz = crate::authz::Authz::new(state.db.clone());
        assert!(authz.is_member("c9", "did:plc:nina").await.expect("q"));
        assert!(
            !authz
                .is_active_member("c9", "did:plc:nina")
                .await
                .expect("q"),
            "claiming an invitation granted the rights an owner had withheld"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_bad_token_and_a_second_membership_are_both_refused() {
        let state = seeded_state().await;
        invite(&state, "inv", "tok-inv", 1).await;
        let bob = token_for(&state, "did:plc:bob").await;
        let (status, v) = claim(&state, &bob, "no-such-token").await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(v["error"], "InvalidClaim");
        let (status, v) = claim(&state, &bob, "tok-inv").await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "AlreadyMember");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn only_an_active_owner_is_given_a_claim_link() {
        let state = seeded_state().await;
        invite(&state, "inv", "tok-inv", 1).await;
        let link = "/xrpc/com.example.wiki.getMemberClaimLink?member=inv";
        let alice = token_for(&state, "did:plc:alice").await;
        let (status, v) = get_as(router(state.clone()), link, &alice).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["token"], "tok-inv");

        let bob = token_for(&state, "did:plc:bob").await;
        let refused = get_as(router(state.clone()), link, &bob).await;
        assert_eq!(refused.0, StatusCode::FORBIDDEN);
        let unknown = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getMemberClaimLink?member=nobody",
            &bob,
        )
        .await;
        assert_eq!(
            refused, unknown,
            "an unknown member must not be tellable apart"
        );
    }

    // -- The tree: stored paths, server-picked slugs, the bin, sibling order. --

    async fn create(state: &AppState, token: &str, body: serde_json::Value) -> serde_json::Value {
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createDocument",
            Some(token),
            body,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let id = v["id"].as_str().expect("id");
        let (status, doc) = get_as(
            router(state.clone()),
            &format!("/xrpc/com.example.wiki.getDocument?id={id}"),
            token,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{doc}");
        doc
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_path_names_a_document_as_well_as_a_context() {
        let state = seeded_state().await;
        let resolve = "/xrpc/com.example.wiki.resolveNode?path=";
        let (status, v) = get(router(state.clone()), &format!("{resolve}group-one/motion")).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["node"], "document");
        assert_eq!(v["id"], "d1");
        let (_, v) = get(router(state.clone()), &format!("{resolve}/group-one//sub/")).await;
        assert_eq!(v["node"], "context");
        assert_eq!(
            v["id"], "c2",
            "empty segments must not change what a path names"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_server_gives_a_new_document_the_cleanest_free_key() {
        let state = seeded_state().await;
        let carol = token_for(&state, "did:plc:carol").await;
        join_as(&state, "did:plc:carol", "c1", "owner").await;
        fn named(title: &str, parent: Option<&str>) -> serde_json::Value {
            serde_json::json!({
                "context_id": "c1", "parent_id": parent, "kind": "folder", "title": title
            })
        }

        let first = create(&state, &carol, named("Landsmøde 2026", None)).await;
        assert_eq!(first["slug"], "landsmøde_2026");
        assert_eq!(first["path"], "group-one/landsmøde_2026");
        assert_eq!(first["parent_id"], "c1");
        assert_eq!(first["owner_did"], "did:plc:carol");

        let second = create(&state, &carol, named("Landsmøde 2026", None)).await;
        assert_eq!(
            second["slug"], "landsmøde_2026-2",
            "a taken key counts up from 2"
        );

        // `sub` is a CONTEXT under c1. The two tables share one namespace.
        let beside_a_context = create(&state, &carol, named("Sub", None)).await;
        assert_eq!(beside_a_context["path"], "group-one/sub-2");

        let folder = first["id"].as_str().expect("id");
        let nested = create(&state, &carol, named("Dagsorden", Some(folder))).await;
        assert_eq!(nested["path"], "group-one/landsmøde_2026/dagsorden");
        let (status, v) = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.resolveNode?path=group-one/landsmøde_2026/dagsorden",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["id"], nested["id"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_document_needs_a_parent_that_is_there() {
        let state = seeded_state().await;
        let carol = token_for(&state, "did:plc:carol").await;
        join(&state, "did:plc:carol", "c1").await;
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createDocument",
            Some(&carol),
            serde_json::json!({
                "context_id": "c1", "parent_id": "nowhere", "kind": "document", "title": "Lost"
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        assert_eq!(v["error"], "InvalidRequest");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_binned_document_is_gone_from_every_read_and_gives_up_its_path() {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "UPDATE document SET deleted_at = datetime('now') WHERE id = 'd1'",
            (),
        )
        .await
        .expect("bin");
        for uri in [
            "/xrpc/com.example.wiki.getDocument?id=d1",
            "/xrpc/com.example.wiki.resolveNode?path=group-one/motion",
        ] {
            let (status, _) = get(router(state.clone()), uri).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{uri}");
        }
        for uri in [
            "/xrpc/com.example.wiki.listChildren?parent=c1",
            "/xrpc/com.example.wiki.search?q=Motion",
            "/xrpc/com.example.wiki.listRecent",
        ] {
            let (_, v) = get(router(state.clone()), uri).await;
            assert!(
                !v.to_string().contains("Motion"),
                "{uri} served the bin: {v}"
            );
        }

        let carol = token_for(&state, "did:plc:carol").await;
        join_as(&state, "did:plc:carol", "c1", "owner").await;
        let again = create(
            &state,
            &carol,
            serde_json::json!({"context_id": "c1", "kind": "document", "title": "Motion"}),
        )
        .await;
        assert_eq!(
            again["path"], "group-one/motion",
            "a node in the bin held its URL hostage"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn children_come_in_their_manual_order() {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute("UPDATE document SET idx = 5 WHERE id = 'd1'", ())
            .await
            .expect("reorder");
        conn.execute("UPDATE document SET idx = 1 WHERE id = 'd2'", ())
            .await
            .expect("reorder");
        let (_, v) = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.listChildren?parent=c1",
        )
        .await;
        let ids: Vec<&str> = v["documents"]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|d| d["id"].as_str())
            .collect();
        assert_eq!(ids, ["d2", "d1"]);
    }

    // -- The write model, end to end. `fold` is a folder in the public group. --

    async fn with_a_folder(attachable: i64) -> AppState {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, attachable) \
             VALUES ('fold', 'c1', 'c1', 'folder', 'Resolutioner', 'resolutioner', \
                     'group-one/resolutioner', ?1)",
            [attachable],
        )
        .await
        .expect("folder");
        state
    }

    async fn try_create(
        state: &AppState,
        token: &str,
        kind: &str,
        parent: &str,
    ) -> (StatusCode, serde_json::Value) {
        post(
            router(state.clone()),
            "/xrpc/com.example.wiki.createDocument",
            Some(token),
            serde_json::json!({
                "context_id": "c1", "parent_id": parent, "kind": kind, "title": "Forslag"
            }),
        )
        .await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_member_files_a_motion_but_does_not_make_the_folder_for_it() {
        let state = with_a_folder(1).await;
        let dave = token_for(&state, "did:plc:dave").await;
        join(&state, "did:plc:dave", "c1").await;

        let (status, v) = try_create(&state, &dave, "policy", "fold").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, v) = try_create(&state, &dave, "folder", "c1").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");
        let (status, v) = try_create(&state, &dave, "policy", "c1").await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "a motion belongs in a folder: {v}"
        );
        let (status, v) = try_create(&state, &dave, "poll", "fold").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_locked_folder_takes_the_owners_motion_and_nobody_elses() {
        let state = with_a_folder(0).await;
        let dave = token_for(&state, "did:plc:dave").await;
        join(&state, "did:plc:dave", "c1").await;
        let (status, v) = try_create(&state, &dave, "policy", "fold").await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");

        let chair = token_for(&state, "did:plc:chair").await;
        join_as(&state, "did:plc:chair", "c1", "owner").await;
        let (status, v) = try_create(&state, &chair, "policy", "fold").await;
        assert_eq!(status, StatusCode::OK, "{v}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_comment_goes_on_content_or_a_comment_never_on_a_container() {
        let state = with_a_folder(1).await;
        let dave = token_for(&state, "did:plc:dave").await;
        join(&state, "did:plc:dave", "c1").await;
        let comment = |on: &'static str| {
            let (state, dave) = (state.clone(), dave.clone());
            async move {
                post(
                    router(state),
                    "/xrpc/com.example.wiki.postComment",
                    Some(&dave),
                    serde_json::json!({"on_id": on, "text": "hej"}),
                )
                .await
            }
        };
        assert_eq!(comment("d1").await.0, StatusCode::OK, "a motion");
        assert_eq!(
            comment("k1").await.0,
            StatusCode::OK,
            "a reply to a comment"
        );
        let (status, v) = comment("fold").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "a folder: {v}");
    }

    // -- Changing and deleting. Dave writes a motion in `fold`; the chair owns c1. --

    struct Meeting {
        state: AppState,
        dave: String,
        chair: String,
        motion: String,
    }

    async fn meeting() -> Meeting {
        let state = with_a_folder(1).await;
        let dave = token_for(&state, "did:plc:dave").await;
        join(&state, "did:plc:dave", "c1").await;
        let chair = token_for(&state, "did:plc:chair").await;
        join_as(&state, "did:plc:chair", "c1", "owner").await;
        let (status, v) = try_create(&state, &dave, "policy", "fold").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let motion = v["id"].as_str().expect("id").to_string();
        Meeting {
            state,
            dave,
            chair,
            motion,
        }
    }

    impl Meeting {
        async fn call(&self, method: &str, who: &str, body: serde_json::Value) -> StatusCode {
            post(
                router(self.state.clone()),
                &format!("/xrpc/com.example.wiki.{method}"),
                Some(who),
                body,
            )
            .await
            .0
        }

        async fn read(&self, who: &str) -> (StatusCode, serde_json::Value) {
            get_as(
                router(self.state.clone()),
                &format!("/xrpc/com.example.wiki.getDocument?id={}", self.motion),
                who,
            )
            .await
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_author_edits_a_draft_until_it_is_submitted() {
        let m = meeting().await;
        let id = &m.motion;
        let edit = |title: &'static str| serde_json::json!({"id": id, "title": title});

        assert_eq!(
            m.call("updateDocument", &m.dave, edit("Bedre titel")).await,
            StatusCode::OK
        );
        let (_, doc) = m.read(&m.dave).await;
        assert_eq!(doc["title"], "Bedre titel");
        assert_eq!(doc["slug"], "forslag", "a rename must keep the URL");

        let submit = serde_json::json!({"id": id, "mutable": false});
        assert_eq!(
            m.call("updateDocument", &m.dave, submit).await,
            StatusCode::OK
        );
        assert_eq!(
            m.call("updateDocument", &m.dave, edit("Fortrudt")).await,
            StatusCode::FORBIDDEN,
            "the room votes on what was submitted"
        );
        let reopen = serde_json::json!({"id": id, "mutable": true});
        assert_eq!(
            m.call("updateDocument", &m.dave, reopen.clone()).await,
            StatusCode::FORBIDDEN,
            "an author reopened their own submitted motion"
        );

        assert_eq!(
            m.call("updateDocument", &m.chair, edit("Rettet")).await,
            StatusCode::OK
        );
        assert_eq!(
            m.call("updateDocument", &m.chair, reopen).await,
            StatusCode::OK
        );
        let (_, doc) = m.read(&m.dave).await;
        assert_eq!(doc["title"], "Rettet");
        assert_eq!(doc["mutable"], true);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_order_and_the_lock_are_the_chairs() {
        let m = meeting().await;
        for arrange in [
            serde_json::json!({"id": m.motion, "idx": 3}),
            serde_json::json!({"id": m.motion, "attachable": false}),
        ] {
            assert_eq!(
                m.call("updateDocument", &m.dave, arrange.clone()).await,
                StatusCode::FORBIDDEN,
                "{arrange}"
            );
            assert_eq!(
                m.call("updateDocument", &m.chair, arrange).await,
                StatusCode::OK
            );
        }
        let (_, doc) = m.read(&m.dave).await;
        assert_eq!(doc["idx"], 3);
        assert_eq!(doc["attachable"], false);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_stranger_changes_nothing_and_is_told_nothing() {
        let m = meeting().await;
        let carol = token_for(&m.state, "did:plc:carol").await;
        join(&m.state, "did:plc:carol", "c1").await;
        let mallory = token_for(&m.state, "did:plc:mallory").await;
        let touch = |id: &str| serde_json::json!({"id": id, "title": "Overtaget"});

        // A fellow member reads it, so is told no. c1 is public, so is Mallory.
        assert_eq!(
            m.call("updateDocument", &carol, touch(&m.motion)).await,
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            m.call(
                "deleteDocument",
                &carol,
                serde_json::json!({"id": m.motion})
            )
            .await,
            StatusCode::FORBIDDEN
        );
        // What she cannot read, she is not told exists.
        assert_eq!(
            m.call("updateDocument", &mallory, touch("s1")).await,
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            m.call("deleteDocument", &mallory, serde_json::json!({"id": "s1"}))
                .await,
            StatusCode::NOT_FOUND
        );
        let (_, doc) = m.read(&m.dave).await;
        assert_eq!(doc["title"], "Forslag");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn deleting_a_folder_bins_what_is_in_it_and_restore_brings_it_all_back() {
        let m = meeting().await;
        let fold = serde_json::json!({"id": "fold"});
        let motion_path = "/xrpc/com.example.wiki.resolveNode?path=group-one/resolutioner/forslag";

        // An older deletion inside the folder, which must stay deleted.
        let (_, v) = try_create(&m.state, &m.dave, "policy", "fold").await;
        let earlier = v["id"].as_str().expect("id").to_string();
        assert_eq!(
            m.call(
                "deleteDocument",
                &m.dave,
                serde_json::json!({"id": earlier})
            )
            .await,
            StatusCode::OK
        );
        assert_eq!(
            m.call("deleteDocument", &m.dave, fold.clone()).await,
            StatusCode::FORBIDDEN,
            "a member deleted the chair's folder"
        );
        assert_eq!(
            m.call("deleteDocument", &m.chair, fold.clone()).await,
            StatusCode::OK
        );
        assert_eq!(m.read(&m.dave).await.0, StatusCode::NOT_FOUND);
        let (status, _) = get(router(m.state.clone()), motion_path).await;
        assert_eq!(status, StatusCode::NOT_FOUND);

        // The bin lists the folder, not the motion that went along with it.
        let (_, bin) = get_as(
            router(m.state.clone()),
            "/xrpc/com.example.wiki.listDeleted?context=c1",
            &m.chair,
        )
        .await;
        let ids: Vec<&str> = bin["deleted"]
            .as_array()
            .expect("array")
            .iter()
            .filter_map(|d| d["id"].as_str())
            .collect();
        assert!(ids.contains(&"fold"), "{bin}");
        assert!(!ids.contains(&m.motion.as_str()), "{bin}");
        assert!(ids.contains(&earlier.as_str()), "{bin}");

        assert_eq!(
            m.call("restoreDocument", &m.chair, fold).await,
            StatusCode::OK
        );
        assert_eq!(m.read(&m.dave).await.0, StatusCode::OK);
        let (status, _) = get(router(m.state.clone()), motion_path).await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = get_as(
            router(m.state.clone()),
            &format!("/xrpc/com.example.wiki.getDocument?id={earlier}"),
            &m.dave,
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "restoring the folder dug up what had been deleted before it"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_restore_is_refused_when_the_path_has_been_taken() {
        let m = meeting().await;
        let motion = serde_json::json!({"id": m.motion});
        assert_eq!(
            m.call("deleteDocument", &m.dave, motion.clone()).await,
            StatusCode::OK
        );
        // Its URL is free again, and someone takes it.
        let (status, v) = try_create(&m.state, &m.dave, "policy", "fold").await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, v) = post(
            router(m.state.clone()),
            "/xrpc/com.example.wiki.restoreDocument",
            Some(&m.dave),
            motion,
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "PathTaken");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_bin_shows_a_member_only_their_own() {
        let m = meeting().await;
        let (_, v) = try_create(&m.state, &m.chair, "policy", "fold").await;
        let chairs = v["id"].as_str().expect("id").to_string();
        for (who, id) in [(&m.dave, &m.motion), (&m.chair, &chairs)] {
            assert_eq!(
                m.call("deleteDocument", who, serde_json::json!({"id": id}))
                    .await,
                StatusCode::OK
            );
        }
        let bin = |who: String| {
            let state = m.state.clone();
            async move {
                get_as(
                    router(state),
                    "/xrpc/com.example.wiki.listDeleted?context=c1",
                    &who,
                )
                .await
                .1["deleted"]
                    .as_array()
                    .expect("array")
                    .len()
            }
        };
        assert_eq!(bin(m.dave.clone()).await, 1);
        assert_eq!(bin(m.chair.clone()).await, 2);
        // Dave cannot restore what is not his, and is not told it is there.
        assert_eq!(
            m.call(
                "restoreDocument",
                &m.dave,
                serde_json::json!({"id": chairs})
            )
            .await,
            StatusCode::NOT_FOUND
        );
    }

    // -- getNode: what a screen draws, in one call. --

    #[tokio::test(flavor = "current_thread")]
    async fn a_node_comes_with_its_children_of_both_kinds_in_order() {
        let state = with_a_folder(1).await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "UPDATE context SET idx = 2 WHERE id = 'c2';
             UPDATE document SET idx = 1 WHERE id = 'fold';
             UPDATE document SET idx = 3, data = '{\"image\":\"f1\"}' WHERE id = 'd1';
             UPDATE document SET idx = 4 WHERE id = 'd2';
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('in-fold', 'c1', 'fold', 'policy', 'Inde', 'inde', \
                       'group-one/resolutioner/inde');",
        )
        .await
        .expect("arrange");

        let (status, v) = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.getNode?path=group-one",
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["node"]["node"], "context");
        assert_eq!(v["node"]["id"], "c1");
        let children = v["children"].as_array().expect("children");
        let seen: Vec<(&str, &str)> = children
            .iter()
            .map(|c| (c["id"].as_str().unwrap(), c["node"].as_str().unwrap()))
            .collect();
        assert_eq!(
            seen,
            [
                ("fold", "document"),
                ("c2", "context"),
                ("d1", "document"),
                ("d2", "document")
            ],
            "one list, in the manual order, whichever table a child is in"
        );
        assert_eq!(
            children[0]["child_count"], 1,
            "the folder has something in it"
        );
        assert_eq!(children[1]["child_count"], 0);
        assert_eq!(children[2]["data"]["image"], "f1");
        assert!(
            children[2].get("content").is_none(),
            "a listing must not carry every child's whole text"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_node_is_found_by_id_as_well_as_by_path() {
        let state = seeded_state().await;
        let by = |query: &'static str| {
            let state = state.clone();
            async move {
                get(
                    router(state),
                    &format!("/xrpc/com.example.wiki.getNode?{query}"),
                )
                .await
            }
        };
        let (_, by_path) = by("path=group-one/motion").await;
        let (_, by_id) = by("id=d1").await;
        assert_eq!(by_path, by_id);
        assert_eq!(by_id["node"]["node"], "document");
        assert_eq!(by("id=c2").await.1["node"]["node"], "context");
        assert_eq!(by("id=nope").await.0, StatusCode::NOT_FOUND);
        assert_eq!(by("path=a&id=b").await.0, StatusCode::BAD_REQUEST);
        assert_eq!(by("").await.0, StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn crumbs_name_what_the_caller_may_read_and_only_that() {
        let state = seeded_state().await;
        let zoe = token_for(&state, "did:plc:zoe").await;
        let (status, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getNode?path=closed/meeting",
            &zoe,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let crumbs = v["crumbs"].as_array().expect("crumbs");
        assert_eq!(crumbs.len(), 2);
        // Zoe belongs to the meeting, not to the closed group it sits in.
        assert_eq!(crumbs[0]["slug"], "closed");
        assert_eq!(crumbs[0]["path"], "closed");
        assert!(
            crumbs[0].get("name").is_none() && crumbs[0].get("id").is_none(),
            "a crumb named a group its reader may not see: {}",
            crumbs[0]
        );
        assert_eq!(crumbs[1]["name"], "Closed Meeting");
        assert_eq!(crumbs[1]["id"], "c10");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_viewer_is_told_what_they_may_do_here() {
        let state = seeded_state().await;
        let node = "/xrpc/com.example.wiki.getNode?path=closed/secret_minutes";
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "UPDATE document SET owner_did = 'did:plc:bob' WHERE id = 's1'",
            (),
        )
        .await
        .expect("owner");

        let viewer = |did: &'static str| {
            let state = state.clone();
            async move {
                let token = token_for(&state, did).await;
                get_as(router(state), node, &token).await.1["viewer"].clone()
            }
        };
        let alice = viewer("did:plc:alice").await;
        assert_eq!(alice["is_context_owner"], true);
        assert_eq!(alice["is_owner"], false);
        let bob = viewer("did:plc:bob").await;
        assert_eq!(bob["is_owner"], true);
        assert_eq!(bob["is_context_owner"], false);
        assert_eq!(bob["can_vote"], true);
        let ivan = viewer("did:plc:ivan").await;
        assert_eq!(ivan["is_member"], true);
        assert_eq!(ivan["can_vote"], false, "ivan holds no voting rights");

        let (_, open) = get(
            router(state.clone()),
            "/xrpc/com.example.wiki.getNode?path=group-one",
        )
        .await;
        assert_eq!(
            open["viewer"],
            serde_json::json!({
                "is_owner": false, "is_member": false,
                "is_context_owner": false, "can_vote": false
            }),
            "a signed-out reader"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn children_the_caller_may_not_read_are_not_listed() {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        // A private group placed INSIDE the public one.
        conn.execute(
            "UPDATE context SET parent_id = 'c1', path = 'group-one/closed' WHERE id = 'c9'",
            (),
        )
        .await
        .expect("nest");
        let ids = |v: &serde_json::Value| -> Vec<String> {
            v["children"]
                .as_array()
                .expect("children")
                .iter()
                .map(|c| c["id"].as_str().unwrap().to_string())
                .collect()
        };
        let node = "/xrpc/com.example.wiki.getNode?path=group-one";
        let (_, anonymous) = get(router(state.clone()), node).await;
        assert!(!ids(&anonymous).contains(&"c9".to_string()), "{anonymous}");
        let bob = token_for(&state, "did:plc:bob").await;
        let (_, member) = get_as(router(state.clone()), node, &bob).await;
        assert!(ids(&member).contains(&"c9".to_string()), "{member}");
    }
}
