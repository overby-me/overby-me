//! Making and keeping a group or an event. Everything else in the tree is a
//! document inside one of these; a context is what people are members OF, and
//! what decides who may read what is in it.
//!
//! The interim makes one in four writes from the browser: a node in the parent's
//! context, then the node turned into its own context, then a permission
//! template, then its first owner. Here it is one transaction, and there is no
//! template: `crate::authz` holds the one rule every context had.

use crate::AppState;
use crate::db::DbError;
use crate::live::Topic;
use crate::session::Caller;
use crate::store::{Parent, WriteError};
use crate::xrpc::{err, forbidden, invalid, member_of, owner_of, owns, write_failed};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use turso::Value;

const MAX_NAME_CHARS: usize = 200;

#[derive(Debug, Deserialize)]
pub struct CreateContextBody {
    /// `group` or `event`. A site is not made this way.
    pub kind: String,
    pub name: String,
    pub parent_id: String,
}

fn named(name: &str) -> Option<String> {
    let name = name.trim();
    (!name.is_empty() && name.chars().count() <= MAX_NAME_CHARS).then(|| name.to_string())
}

/// The context, its first owner and its place in the search index, as one step:
/// a group nobody owns could never be administered, and one that exists for a
/// moment without an owner is one a crash can leave that way.
async fn create_in(
    state: &AppState,
    conn: &turso::Connection,
    body: &CreateContextBody,
    name: &str,
    parent: &Parent,
    creator: &str,
) -> Result<(String, String), WriteError> {
    let store = crate::Store::new(state.db.clone());
    let id = format!("c-{}", crate::util::random_token(16));
    let (mut slug, mut path) = (String::new(), String::new());
    for candidate in crate::slug::candidates(name) {
        path = format!("{}/{candidate}", parent.path);
        slug = candidate;
        if !store.path_taken(conn, &path).await? {
            break;
        }
    }
    conn.execute(
        "INSERT INTO context (id, kind, name, slug, path, parent_id, owner_did) \
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
        [
            id.as_str(),
            body.kind.as_str(),
            name,
            slug.as_str(),
            path.as_str(),
            body.parent_id.as_str(),
            creator,
        ],
    )
    .await?;
    conn.execute(
        "INSERT INTO member (id, user_did, context_id, role, active, accepted, name) \
         SELECT ?1, ?2, ?3, 'owner', 1, 1, display_name FROM user WHERE did = ?2",
        [
            format!("m-{}", crate::util::random_token(16)).as_str(),
            creator,
            id.as_str(),
        ],
    )
    .await?;
    crate::search::index(conn, &id, name, None).await?;
    Ok((id, path))
}

/// `com.example.wiki.createContext` (procedure): make a group or an event under
/// a context, or a folder in one, that the caller owns. The caller is its first
/// owner, with voting rights. It starts closed to everyone else.
pub async fn create_context(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CreateContextBody>,
) -> Response {
    let what = "createContext";
    if !matches!(body.kind.as_str(), "group" | "event") {
        return invalid("a context is a group or an event");
    }
    let Some(name) = named(&body.name) else {
        return invalid("a context needs a name, of at most 200 characters");
    };
    let parent = match crate::Store::new(state.db.clone())
        .parent_of(&body.parent_id)
        .await
    {
        Ok(Some(parent)) => parent,
        Ok(None) => return invalid("no such parent"),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = owner_of(&state, &parent.context_id, &did, what).await {
        return refusal;
    }
    if !crate::authz::PLACES.contains(&parent.kind.as_str()) {
        return invalid("a group or an event sits in a context, or in a folder");
    }
    let created = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let created = create_in(&state, &conn, &body, &name, &parent, &did).await;
        conn.execute(
            if created.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            },
            (),
        )
        .await?;
        created
    };
    match created.await {
        Ok((id, path)) => {
            state.publish(Topic::Context(parent.context_id.clone()), "node", &id);
            state.publish(Topic::User(did), "membership", &id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "id": id, "path": path })),
            )
                .into_response()
        }
        Err(WriteError::Db(e)) => write_failed(what, e),
        Err(refused) => invalid(&refused.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct UpdateContextBody {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    /// `public` opens it, and what is in it, to everyone, signed in or not.
    #[serde(default)]
    pub visibility: Option<String>,
    /// Whether members may add to it directly.
    #[serde(default)]
    pub attachable: Option<bool>,
}

/// `com.example.wiki.updateContext` (procedure): an owner renames a context,
/// opens or closes it to the public, or locks it. A rename keeps the address:
/// the slug is what links point at.
pub async fn update_context(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<UpdateContextBody>,
) -> Response {
    let what = "updateContext";
    if let Err(refusal) = owner_of(&state, &body.id, &did, what).await {
        return refusal;
    }
    let name = match body.name.as_deref().map(named) {
        Some(None) => return invalid("a context needs a name, of at most 200 characters"),
        Some(name) => name,
        None => None,
    };
    if body
        .visibility
        .as_deref()
        .is_some_and(|v| !matches!(v, "public" | "private"))
    {
        return invalid("visibility is public or private");
    }
    let changed = async {
        let conn = state.db.acquire().await?;
        let text = |v: &Option<String>| v.clone().map_or(Value::Null, Value::Text);
        let changed = conn
            .execute(
                "UPDATE context SET name = coalesce(?2, name), \
                   visibility = coalesce(?3, visibility), attachable = coalesce(?4, attachable), \
                   updated_at = strftime('%Y-%m-%dT%H:%M:%fZ','now') \
                 WHERE id = ?1 AND deleted_at IS NULL",
                vec![
                    Value::Text(body.id.clone()),
                    text(&name),
                    text(&body.visibility),
                    body.attachable
                        .map_or(Value::Null, |b| Value::Integer(i64::from(b))),
                ],
            )
            .await?;
        if let Some(name) = &name {
            crate::search::index(&conn, &body.id, name, None).await?;
        }
        Ok::<_, DbError>(changed)
    };
    match changed.await {
        Ok(0) => err(StatusCode::NOT_FOUND, "NotFound", "no such context"),
        Ok(_) => {
            state.publish(Topic::Context(body.id.clone()), "node", &body.id);
            (StatusCode::OK, Json(serde_json::json!({ "ok": true }))).into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

struct ContextMeta {
    kind: String,
    path: String,
    parent_id: Option<String>,
    binned: bool,
    /// Whether it went to the bin itself, and not along with something else.
    bin_entry: bool,
}

async fn meta(state: &AppState, id: &str) -> Result<Option<ContextMeta>, DbError> {
    let conn = state.db.acquire().await?;
    let mut rows = conn
        .query(
            "SELECT kind, path, parent_id, deleted_at IS NOT NULL, coalesce(deleted_root = id, 0) \
             FROM context WHERE id = ?1",
            [id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(None);
    };
    Ok(Some(ContextMeta {
        kind: row.get(0)?,
        path: row.get(1)?,
        parent_id: match row.get_value(2)? {
            Value::Text(parent) => Some(parent),
            _ => None,
        },
        binned: row.get::<i64>(3)? != 0,
        bin_entry: row.get::<i64>(4)? != 0,
    }))
}

/// Whether `did` may bin or restore the context: an owner of it, or an owner of
/// the context it sits in, who answers for everything under them.
async fn may_remove(
    state: &AppState,
    id: &str,
    meta: &ContextMeta,
    did: &str,
) -> Result<bool, DbError> {
    let authz = crate::authz::Authz::new(state.db.clone());
    if authz.membership(id, did).await?.is_some_and(owns) {
        return Ok(true);
    }
    let Some(parent_id) = &meta.parent_id else {
        return Ok(false);
    };
    let store = crate::Store::new(state.db.clone());
    let Some(parent) = store.parent_of(parent_id).await? else {
        return Ok(false);
    };
    Ok(authz
        .membership(&parent.context_id, did)
        .await?
        .is_some_and(owns))
}

#[derive(Debug, Deserialize)]
pub struct ContextIdBody {
    pub id: String,
}

/// `com.example.wiki.deleteContext` (procedure): put a group or an event, and
/// everything in it, in the bin. Its members keep their rows, so restoring it
/// brings back who was in it too.
pub async fn delete_context(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ContextIdBody>,
) -> Response {
    let what = "deleteContext";
    // Asked as a member of it first, so that a stranger hears "no such context"
    // for a closed one, as they do everywhere else.
    if let Err(refusal) = member_or_parent_owner(&state, &body.id, &did, what).await {
        return refusal;
    }
    let meta = match meta(&state, &body.id).await {
        Ok(Some(meta)) if !meta.binned => meta,
        Ok(_) => return err(StatusCode::NOT_FOUND, "NotFound", "no such context"),
        Err(e) => return write_failed(what, e),
    };
    if meta.kind == "site" {
        return invalid("a site is not deleted from inside it");
    }
    match may_remove(&state, &body.id, &meta, &did).await {
        Ok(true) => {}
        Ok(false) => return forbidden("only an owner may delete a context"),
        Err(e) => return write_failed(what, e),
    }
    match crate::Store::new(state.db.clone())
        .bin_subtree(&body.id, &meta.path)
        .await
    {
        Ok(binned) => {
            state.publish(Topic::Context(body.id.clone()), "node", &body.id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "binned": binned })),
            )
                .into_response()
        }
        Err(e) => write_failed(what, e),
    }
}

/// The caller as a member of the context, or as an owner of where it sits.
/// Answers as a missing context does to anyone who is neither.
async fn member_or_parent_owner(
    state: &AppState,
    id: &str,
    did: &str,
    what: &str,
) -> Result<(), Response> {
    let missing = || err(StatusCode::NOT_FOUND, "NotFound", "no such context");
    let found = meta(state, id)
        .await
        .map_err(|e| write_failed(what, e))?
        .ok_or_else(missing)?;
    match may_remove(state, id, &found, did).await {
        Ok(true) => Ok(()),
        Ok(false) if found.binned => Err(missing()),
        Ok(false) => member_of(state, id, did, what).await.map(|_| ()),
        Err(e) => Err(write_failed(what, e)),
    }
}

/// `com.example.wiki.restoreContext` (procedure): bring a context back from the
/// bin, with everything that went with it.
pub async fn restore_context(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<ContextIdBody>,
) -> Response {
    let what = "restoreContext";
    let meta = match meta(&state, &body.id).await {
        Ok(Some(meta)) if meta.binned => meta,
        Ok(_) => {
            return err(
                StatusCode::NOT_FOUND,
                "NotFound",
                "no such context in the bin",
            );
        }
        Err(e) => return write_failed(what, e),
    };
    match may_remove(&state, &body.id, &meta, &did).await {
        Ok(true) => {}
        Ok(false) => {
            return err(
                StatusCode::NOT_FOUND,
                "NotFound",
                "no such context in the bin",
            );
        }
        Err(e) => return write_failed(what, e),
    }
    if !meta.bin_entry {
        return invalid("it went to the bin along with what it is in; restore that");
    }
    match crate::Store::new(state.db.clone())
        .restore_subtree(&body.id, &meta.path)
        .await
    {
        Ok(restored) => {
            state.publish(Topic::Context(body.id.clone()), "node", &body.id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "restored": restored })),
            )
                .into_response()
        }
        Err(WriteError::Db(e)) => write_failed(what, e),
        Err(refused) => crate::xrpc::conflict("PathTaken", &refused.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use crate::router;
    use crate::xrpc::tests::{get, get_as, post, seeded_state, token_for};
    use axum::http::StatusCode;
    use serde_json::json;

    const CREATE: &str = "/xrpc/com.example.wiki.createContext";
    const UPDATE: &str = "/xrpc/com.example.wiki.updateContext";

    #[tokio::test(flavor = "current_thread")]
    async fn an_owner_makes_an_event_in_their_group_and_owns_it() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let body = json!({"kind": "event", "name": "Årsmøde 2027", "parent_id": "c9"});

        let (status, _) = post(router(state.clone()), CREATE, Some(&bob), body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a member made an event");
        let (status, v) = post(router(state.clone()), CREATE, Some(&alice), body.clone()).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            v["path"], "closed/årsmøde_2027",
            "the frontend's slug rule keeps the letters"
        );
        let id = v["id"].as_str().expect("id");

        let node = format!("/xrpc/com.example.wiki.getNode?id={id}");
        let (_, seen) = get_as(router(state.clone()), &node, &alice).await;
        assert_eq!(seen["node"]["kind"], "event", "{seen}");
        assert_eq!(seen["viewer"]["is_context_owner"], true);
        assert_eq!(seen["viewer"]["can_vote"], true);
        assert_eq!(
            get_as(router(state.clone()), &node, &bob).await.0,
            StatusCode::NOT_FOUND,
            "an event is its own context: the group's members are not in it"
        );
        let members = format!("/xrpc/com.example.wiki.listMembers?context={id}");
        let (_, roster) = get_as(router(state.clone()), &members, &alice).await;
        assert_eq!(roster["members"][0]["display_name"], "Alice", "{roster}");

        // The same name again lands beside it.
        let (_, again) = post(router(state.clone()), CREATE, Some(&alice), body).await;
        assert_eq!(again["path"], "closed/årsmøde_2027-2");
        let (_, found) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.search?q=årsmøde",
            &alice,
        )
        .await;
        assert_eq!(found["hits"].as_array().expect("hits").len(), 2, "{found}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_is_not_a_context_or_has_nowhere_to_sit_is_refused() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        for (why, body) in [
            (
                "a site",
                json!({"kind": "site", "name": "x", "parent_id": "c9"}),
            ),
            (
                "no name",
                json!({"kind": "group", "name": "  ", "parent_id": "c9"}),
            ),
            (
                "under a document",
                json!({"kind": "group", "name": "x", "parent_id": "s1"}),
            ),
            (
                "under nothing",
                json!({"kind": "group", "name": "x", "parent_id": "nowhere"}),
            ),
        ] {
            let (status, v) = post(router(state.clone()), CREATE, Some(&alice), body).await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{why}: {v}");
        }
        let mallory = token_for(&state, "did:plc:mallory").await;
        let body = json!({"kind": "group", "name": "x", "parent_id": "c9"});
        let (status, _) = post(router(state.clone()), CREATE, Some(&mallory), body).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a stranger learned of a closed group"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn an_owner_opens_a_group_to_the_public_and_closes_it_again() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let page = "/xrpc/com.example.wiki.getNode?path=closed";
        assert_eq!(
            get(router(state.clone()), page).await.0,
            StatusCode::NOT_FOUND
        );

        let open = json!({"id": "c9", "visibility": "public", "name": "Open Group"});
        let (status, _) = post(router(state.clone()), UPDATE, Some(&bob), open.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN);
        let (status, v) = post(router(state.clone()), UPDATE, Some(&alice), open).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (status, seen) = get(router(state.clone()), page).await;
        assert_eq!(status, StatusCode::OK, "a rename moved the address: {seen}");
        assert_eq!(seen["node"]["name"], "Open Group");

        let shut = json!({"id": "c9", "visibility": "private"});
        post(router(state.clone()), UPDATE, Some(&alice), shut).await;
        assert_eq!(
            get(router(state.clone()), page).await.0,
            StatusCode::NOT_FOUND
        );
        let (status, v) = post(
            router(state.clone()),
            UPDATE,
            Some(&alice),
            json!({"id": "c9", "visibility": "friends"}),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_group_goes_to_the_bin_whole_and_comes_back_with_its_members() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let delete = "/xrpc/com.example.wiki.deleteContext";
        let restore = "/xrpc/com.example.wiki.restoreContext";
        let page = "/xrpc/com.example.wiki.getNode?path=closed/secret_minutes";

        let (status, _) = post(
            router(state.clone()),
            delete,
            Some(&bob),
            json!({"id": "c9"}),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "a member binned the group");
        let (status, v) = post(
            router(state.clone()),
            delete,
            Some(&alice),
            json!({"id": "c9"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert!(
            v["binned"].as_u64().expect("binned") >= 3,
            "the meeting and the minutes go along: {v}"
        );
        assert_eq!(
            get_as(router(state.clone()), page, &bob).await.0,
            StatusCode::NOT_FOUND
        );

        // What went along is not an entry of its own in any bin.
        let bin_of_group = "/xrpc/com.example.wiki.listDeleted?context=c9";
        let (_, bin) = get_as(router(state.clone()), bin_of_group, &alice).await;
        assert_eq!(bin["deleted"], json!([]), "{bin}");

        // The meeting inside went with it, so it is the group that is restored.
        let (status, v) = post(
            router(state.clone()),
            restore,
            Some(&alice),
            json!({"id": "c10"}),
        )
        .await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "alice is not in the meeting: {v}"
        );
        let (status, v) = post(
            router(state.clone()),
            restore,
            Some(&alice),
            json!({"id": "c9"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            get_as(router(state.clone()), page, &bob).await.0,
            StatusCode::OK,
            "bob lost his seat"
        );

        // Binned by itself, the meeting is an entry in the bin of where it sat,
        // and an owner of the group may remove what sits in it.
        let meeting = json!({"id": "c10"});
        let (status, v) = post(router(state.clone()), delete, Some(&alice), meeting.clone()).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let (_, bin) = get_as(router(state.clone()), bin_of_group, &alice).await;
        assert_eq!(bin["deleted"][0]["id"], "c10", "{bin}");
        assert_eq!(bin["deleted"][0]["node"], "context");
        let (status, _) = post(router(state.clone()), restore, Some(&alice), meeting).await;
        assert_eq!(status, StatusCode::OK);
    }
}
