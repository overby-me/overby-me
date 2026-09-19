//! The two tree operations that make and destroy: copying a subtree, and
//! deleting one for good. (Moving is `Store::move_document`; the bin is
//! `bin_subtree`.)
//!
//! Both are an owner's. Both have to mind what a node points at: a file node
//! holds a blob's id, and a blob is read through its own context, so a copy gets
//! blob rows of its own (the bytes are stored by their hash, so that costs no
//! disk), and a purge takes the blobs nothing else points at with it.

use crate::AppState;
use crate::authz::{Authz, readable_document};
use crate::live::Topic;
use crate::session::Caller;
use crate::store::{WriteError, under};
use crate::xrpc::{err, forbidden, invalid, owner_of, write_failed};
use axum::Json;
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Deserialize;
use std::collections::HashMap;
use turso::{Connection, Value};

/// The most nodes one copy takes. A whole meeting's folder is a few hundred.
const MAX_COPY: usize = 2000;

/// Where in a node's `data` a blob id can be.
const BLOB_FIELDS: [&str; 2] = ["fileId", "image"];

#[derive(Debug, Deserialize)]
pub struct CopyBody {
    pub id: String,
    /// Where the copy goes. Any context the caller owns.
    pub parent_id: String,
}

struct Source {
    id: String,
    parent_id: Option<String>,
    kind: String,
    title: String,
    slug: String,
    idx: i64,
    attachable: i64,
    mutable: i64,
    content: Option<String>,
    data: Option<String>,
    created_at: String,
}

fn text(row: &turso::Row, i: usize) -> Option<String> {
    match row.get_value(i) {
        Ok(Value::Text(s)) => Some(s),
        _ => None,
    }
}

/// `data` with every blob it names swapped for a copy in `context_id`.
async fn with_own_blobs(
    conn: &Connection,
    data: Option<&str>,
    context_id: &str,
    owner: &str,
) -> Result<Option<String>, WriteError> {
    let Some(mut json) = data.and_then(|d| serde_json::from_str::<serde_json::Value>(d).ok())
    else {
        return Ok(data.map(str::to_string));
    };
    for field in BLOB_FIELDS {
        let Some(old) = json.get(field).and_then(|v| v.as_str()).map(str::to_string) else {
            continue;
        };
        let new = format!("b-{}", crate::util::random_token(16));
        let copied = conn
            .execute(
                "INSERT INTO blob (id, context_id, owner_did, sha256, size, mime, name) \
                 SELECT ?1, ?2, ?3, sha256, size, mime, name FROM blob WHERE id = ?4",
                [new.as_str(), context_id, owner, old.as_str()],
            )
            .await?;
        if copied > 0 {
            json[field] = serde_json::Value::String(new);
        }
    }
    Ok(Some(json.to_string()))
}

/// Copy the document `id` and what the caller may read under it to under
/// `parent`. Returns the copy's id and path, and how many nodes were copied.
async fn copy_in(
    state: &AppState,
    conn: &Connection,
    id: &str,
    parent_id: &str,
    parent: &crate::store::Parent,
    caller: &str,
) -> Result<(String, String, usize), WriteError> {
    let store = crate::Store::new(state.db.clone());
    let mut rows = conn
        .query(
            "SELECT path FROM document WHERE id = ?1 AND deleted_at IS NULL",
            [id],
        )
        .await?;
    let Some(row) = rows.next().await? else {
        return Err(WriteError::NoSuchParent);
    };
    let root_path: String = row.get(0)?;
    drop(rows);
    if parent.path == root_path || parent.path.starts_with(&format!("{root_path}/")) {
        return Err(WriteError::IntoItself);
    }

    // Parents before children, so that a child finds its parent's copy. A poll
    // is a vote's record and not content, a canvas is what a room made and not a
    // template, and a context is not a document.
    let mut rows = conn
        .query(
            &format!(
                "SELECT d.id, d.parent_id, d.kind, d.title, d.slug, d.idx, d.attachable, \
                   d.mutable, d.content, d.data, d.created_at \
                 FROM document d \
                 WHERE (d.id = ?2 OR {}) AND d.deleted_at IS NULL \
                   AND d.kind NOT IN ('poll', 'canvas') AND {} \
                 ORDER BY length(d.path), d.path",
                under("d.path", 3),
                readable_document("d", 1)
            ),
            [caller, id, root_path.as_str()],
        )
        .await?;
    let mut sources = Vec::new();
    while let Some(row) = rows.next().await? {
        sources.push(Source {
            id: row.get(0)?,
            parent_id: text(&row, 1),
            kind: row.get(2)?,
            title: row.get(3)?,
            slug: row.get(4)?,
            idx: row.get(5)?,
            attachable: row.get(6)?,
            mutable: row.get(7)?,
            content: text(&row, 8),
            data: text(&row, 9),
            created_at: row.get(10)?,
        });
        if sources.len() > MAX_COPY {
            return Err(WriteError::TooMany);
        }
    }
    drop(rows);

    // old id -> (new id, new path)
    let mut copies: HashMap<String, (String, String)> = HashMap::new();
    let mut root = None;
    for source in &sources {
        let new_id = format!("d-{}", crate::util::random_token(16));
        let (new_parent, slug, path) = if source.id == id {
            // It may land beside its source, so its slug is found afresh.
            let mut taken = (String::new(), String::new());
            for candidate in crate::slug::candidates(&source.title) {
                taken = (parent.child_path(&candidate), candidate);
                if !store.path_taken(conn, &taken.0).await? {
                    break;
                }
            }
            (parent_id.to_string(), taken.1, taken.0)
        } else {
            // Under a fresh parent a slug cannot collide, so a child keeps its own.
            let Some((new_parent, parent_path)) = source
                .parent_id
                .as_ref()
                .and_then(|old| copies.get(old))
                .cloned()
            else {
                // Its parent was not copied (unreadable, or a poll): nor is it.
                continue;
            };
            let path = format!("{parent_path}/{}", source.slug);
            (new_parent, source.slug.clone(), path)
        };
        let data = with_own_blobs(conn, source.data.as_deref(), &parent.context_id, caller).await?;
        // The original's date: a copy is the same content somewhere else, and
        // dated now it would sort to the end of its folder as the newest thing.
        conn.execute(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, idx, \
               attachable, mutable, owner_did, content, data, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11, ?12, ?13, ?14)",
            vec![
                Value::Text(new_id.clone()),
                Value::Text(parent.context_id.clone()),
                Value::Text(new_parent),
                Value::Text(source.kind.clone()),
                Value::Text(source.title.clone()),
                Value::Text(slug),
                Value::Text(path.clone()),
                Value::Integer(source.idx),
                Value::Integer(source.attachable),
                Value::Integer(source.mutable),
                Value::Text(caller.to_string()),
                source.content.clone().map_or(Value::Null, Value::Text),
                data.map_or(Value::Null, Value::Text),
                Value::Text(source.created_at.clone()),
            ],
        )
        .await?;
        conn.execute(
            "INSERT INTO document_author (document_id, author_did, author_text, author_context, ord) \
             SELECT ?1, author_did, author_text, author_context, ord FROM document_author \
             WHERE document_id = ?2",
            [new_id.as_str(), source.id.as_str()],
        )
        .await?;
        crate::search::index(conn, &new_id, &source.title, source.content.as_deref()).await?;
        if source.id == id {
            root = Some((new_id.clone(), path.clone()));
        }
        copies.insert(source.id.clone(), (new_id, path));
    }
    let (new_id, path) = root.ok_or(WriteError::NoSuchParent)?;
    Ok((new_id, path, copies.len()))
}

/// `com.example.wiki.copyDocument` (procedure): copy a document, with what the
/// caller may read under it, to under another parent, in this context or any
/// other the caller owns. The copy is the caller's.
pub async fn copy_document(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<CopyBody>,
) -> Response {
    let what = "copyDocument";
    let store = crate::Store::new(state.db.clone());
    let source = match store.read_document(&body.id, Some(&did)).await {
        Ok(Some(source)) => source,
        Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such document"),
        Err(e) => return write_failed(what, e),
    };
    let kind = serde_json::to_value(&source.kind)
        .ok()
        .and_then(|k| k.as_str().map(str::to_string))
        .unwrap_or_default();
    if matches!(kind.as_str(), "poll" | "canvas") {
        return invalid("a poll and a canvas are records of what happened, and are not copied");
    }
    let parent = match store.parent_of(&body.parent_id).await {
        Ok(Some(parent)) => parent,
        Ok(None) => return invalid("no such parent"),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = owner_of(&state, &parent.context_id, &did, what).await {
        return refusal;
    }
    let membership = match Authz::new(state.db.clone())
        .membership(&parent.context_id, &did)
        .await
    {
        Ok(Some(membership)) => membership,
        Ok(None) => return forbidden("not a member of that context"),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) =
        crate::authz::may_create(&kind, &parent.kind, parent.attachable, membership)
    {
        return invalid(refusal.message());
    }

    let copied = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let copied = copy_in(&state, &conn, &body.id, &body.parent_id, &parent, &did).await;
        conn.execute(if copied.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
            .await?;
        copied
    };
    match copied.await {
        Ok((id, path, nodes)) => {
            state.publish(Topic::Context(parent.context_id.clone()), "node", &id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "id": id, "path": path, "copied": nodes })),
            )
                .into_response()
        }
        Err(WriteError::Db(e)) => write_failed(what, e),
        Err(refused) => invalid(&refused.to_string()),
    }
}

#[derive(Debug, Deserialize)]
pub struct PurgeBody {
    pub id: String,
}

/// Delete for good everything one bin entry holds. Returns how many documents
/// went, and the blobs they pointed at.
async fn purge_in(conn: &Connection, id: &str) -> Result<(u64, Vec<String>), WriteError> {
    let mut rows = conn
        .query(
            "SELECT 1 FROM document WHERE id = ?1 AND deleted_at IS NOT NULL AND deleted_root = ?1",
            [id],
        )
        .await?;
    if rows.next().await?.is_none() {
        return Err(WriteError::NotBinned);
    }
    for (refusal, sql) in [
        (
            WriteError::ContextInside,
            "SELECT 1 FROM context WHERE deleted_root = ?1 LIMIT 1",
        ),
        (
            WriteError::PollInside,
            "SELECT 1 FROM document WHERE deleted_root = ?1 AND kind = 'poll' LIMIT 1",
        ),
    ] {
        let mut rows = conn.query(sql, [id]).await?;
        if rows.next().await?.is_some() {
            return Err(refusal);
        }
    }
    let mut blobs = Vec::new();
    for field in BLOB_FIELDS {
        let mut rows = conn
            .query(
                &format!(
                    "SELECT json_extract(data, '$.{field}') FROM document \
                     WHERE deleted_root = ?1 AND json_extract(data, '$.{field}') IS NOT NULL"
                ),
                [id],
            )
            .await?;
        while let Some(row) = rows.next().await? {
            blobs.extend(text(&row, 0));
        }
    }
    let going = "SELECT id FROM document WHERE deleted_root = ?1";
    // By root: the replies go with what they answer, and not only the comments
    // that sit on a document directly.
    let threads = format!("SELECT k.id FROM comment k WHERE k.root_id IN ({going})");
    let mut rows = conn
        .query(
            &format!(
                "SELECT k.image FROM comment k \
                 WHERE k.root_id IN ({going}) AND k.image IS NOT NULL"
            ),
            [id],
        )
        .await?;
    while let Some(row) = rows.next().await? {
        blobs.extend(text(&row, 0));
    }
    drop(rows);
    for sql in [
        format!("DELETE FROM canvas_cell WHERE canvas_id IN ({going})"),
        format!("DELETE FROM canvas_painter WHERE canvas_id IN ({going})"),
        format!("DELETE FROM canvas WHERE id IN ({going})"),
        format!("DELETE FROM document_author WHERE document_id IN ({going})"),
        format!(
            "DELETE FROM reaction WHERE subject_uri IN ({going}) OR subject_uri IN ({threads})"
        ),
        format!("DELETE FROM comment WHERE root_id IN ({going})"),
        format!("DELETE FROM search_index WHERE node_id IN ({going})"),
    ] {
        conn.execute(&sql, [id]).await?;
    }
    let purged = conn
        .execute("DELETE FROM document WHERE deleted_root = ?1", [id])
        .await?;
    Ok((purged, blobs))
}

/// `com.example.wiki.purgeDocument` (procedure): the way out of the bin that
/// restoring is not. An owner of the context deletes for good what one bin entry
/// holds: the documents, their author chips, the threads on them with their
/// reactions and pictures, and the files nothing else points at. A vote's record, and a group, are nobody's to purge.
pub async fn purge_document(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<PurgeBody>,
) -> Response {
    let what = "purgeDocument";
    let meta = match crate::Store::new(state.db.clone())
        .document_meta(&body.id)
        .await
    {
        Ok(Some(meta)) => meta,
        Ok(None) => return err(StatusCode::NOT_FOUND, "NotFound", "no such document"),
        Err(e) => return write_failed(what, e),
    };
    if let Err(refusal) = owner_of(&state, &meta.context_id, &did, what).await {
        return refusal;
    }
    let purged = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let purged = purge_in(&conn, &body.id).await;
        conn.execute(if purged.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
            .await?;
        purged
    };
    match purged.await {
        Ok((documents, blobs)) => {
            // After the rows are gone, so that "nothing points at it" is true
            // of what this took too. A failure here leaves a file, not a hole.
            for blob in blobs {
                if let Err(e) = crate::blob::forget_if_unreferenced(&state, &blob).await {
                    tracing::warn!("purge left blob {blob} behind: {e}");
                }
            }
            state.publish(Topic::Context(meta.context_id), "node", &body.id);
            (
                StatusCode::OK,
                Json(serde_json::json!({ "purged": documents })),
            )
                .into_response()
        }
        Err(WriteError::Db(e)) => write_failed(what, e),
        Err(refused @ (WriteError::PollInside | WriteError::ContextInside)) => {
            crate::xrpc::conflict("NotPurgeable", &refused.to_string())
        }
        Err(refused) => invalid(&refused.to_string()),
    }
}

/// The most comments one orphaned thread is followed through.
const MAX_THREAD: usize = 10_000;

/// Delete the comment `id` and every answer under it, with the reactions to
/// them. Level by level: the engine has no recursive query. Returns how many
/// comments went, and the pictures they held.
async fn purge_thread(conn: &Connection, id: &str) -> Result<(u64, Vec<String>), WriteError> {
    let mut all: Vec<String> = Vec::new();
    let mut level = vec![id.to_string()];
    while !level.is_empty() && all.len() < MAX_THREAD {
        let marks = (1..=level.len())
            .map(|i| format!("?{i}"))
            .collect::<Vec<_>>()
            .join(", ");
        let mut rows = conn
            .query(
                &format!("SELECT id FROM comment WHERE on_id IN ({marks})"),
                level.iter().cloned().map(Value::Text).collect::<Vec<_>>(),
            )
            .await?;
        let mut next = Vec::new();
        while let Some(row) = rows.next().await? {
            next.push(row.get::<String>(0)?);
        }
        all.append(&mut level);
        level = next;
    }
    let (mut purged, mut images) = (0, Vec::new());
    for comment in &all {
        let mut rows = conn
            .query(
                "SELECT image FROM comment WHERE id = ?1",
                [comment.as_str()],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            images.extend(text(&row, 0));
        }
        drop(rows);
        conn.execute(
            "DELETE FROM reaction WHERE subject_uri = ?1",
            [comment.as_str()],
        )
        .await?;
        purged += conn
            .execute("DELETE FROM comment WHERE id = ?1", [comment.as_str()])
            .await?;
    }
    Ok((purged, images))
}

/// `com.example.wiki.purgeOrphan` (procedure): whoever runs the site deletes for
/// good a node `listOrphans` lists, and what is under it. A page goes as a purge
/// from the bin takes it, a comment with the answers to it. A group does not go
/// this way: it holds people's seats and a record of its own, and it still opens
/// by its path.
pub async fn purge_orphan(
    State(state): State<AppState>,
    Caller { did }: Caller,
    Json(body): Json<PurgeBody>,
) -> Response {
    let what = "purgeOrphan";
    match Authz::new(state.db.clone()).owns_the_site(&did).await {
        Ok(true) => {}
        Ok(false) => {
            return forbidden("only an owner of the site clears away what has gone astray");
        }
        Err(e) => return write_failed(what, e),
    }
    let purged = async {
        let _turn = state.db.write_turn().await;
        let conn = state.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let purged = purge_orphan_in(&conn, &body.id).await;
        conn.execute(if purged.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
            .await?;
        purged
    };
    match purged.await {
        Ok((nodes, blobs)) => {
            for blob in blobs {
                if let Err(e) = crate::blob::forget_if_unreferenced(&state, &blob).await {
                    tracing::warn!("purging an orphan left blob {blob} behind: {e}");
                }
            }
            (StatusCode::OK, Json(serde_json::json!({ "purged": nodes }))).into_response()
        }
        Err(WriteError::Db(e)) => write_failed(what, e),
        Err(WriteError::NotBinned) => err(StatusCode::NOT_FOUND, "NotFound", "no such orphan"),
        Err(refused) => crate::xrpc::conflict("NotPurgeable", &refused.to_string()),
    }
}

async fn purge_orphan_in(conn: &Connection, id: &str) -> Result<(u64, Vec<String>), WriteError> {
    let mut rows = conn
        .query(
            &format!(
                "SELECT d.path FROM document d WHERE d.id = ?1 AND {}",
                crate::feed::nowhere("d.parent_id")
            ),
            [id],
        )
        .await?;
    if let Some(row) = rows.next().await? {
        let path: String = row.get(0)?;
        drop(rows);
        // In the bin under itself, whatever bin it was in: what a purge takes.
        conn.execute(
            "UPDATE document SET deleted_root = ?1, \
               deleted_at = coalesce(deleted_at, strftime('%Y-%m-%dT%H:%M:%fZ','now')) \
             WHERE id = ?1 OR substr(path, 1, length(?2) + 1) = ?2 || '/'",
            [id, path.as_str()],
        )
        .await?;
        conn.execute(
            "UPDATE context SET deleted_root = ?1 \
             WHERE substr(path, 1, length(?2) + 1) = ?2 || '/'",
            [id, path.as_str()],
        )
        .await?;
        return purge_in(conn, id).await;
    }
    drop(rows);
    let mut rows = conn
        .query(
            &format!(
                "SELECT 1 FROM comment k WHERE k.id = ?1 AND {} \
                   AND NOT EXISTS (SELECT 1 FROM post p WHERE p.id = k.root_id)",
                crate::feed::nowhere("k.root_id")
            ),
            [id],
        )
        .await?;
    if rows.next().await?.is_none() {
        return Err(WriteError::NotBinned);
    }
    drop(rows);
    purge_thread(conn, id).await
}

#[cfg(test)]
mod tests {
    use crate::AppState;
    use crate::blob::tests::{fetch, state as blob_state, upload};
    use crate::router;
    use crate::xrpc::tests::{get_as, join, join_as, post, token_for};
    use axum::http::StatusCode;
    use serde_json::json;

    const COPY: &str = "/xrpc/com.example.wiki.copyDocument";
    const PURGE: &str = "/xrpc/com.example.wiki.purgeDocument";
    const MOVE: &str = "/xrpc/com.example.wiki.moveDocument";
    const BIN: &str = "/xrpc/com.example.wiki.deleteDocument";

    async fn count(state: &AppState, sql: &str) -> i64 {
        let conn = state.db.acquire().await.expect("conn");
        let mut rows = conn.query(sql, ()).await.expect("query");
        let row = rows.next().await.expect("next").expect("row");
        row.get(0).expect("count")
    }

    /// The closed group c9 with a folder in it: a page, a file with its bytes
    /// uploaded, and on the page a comment, an answer to it that shows a
    /// picture, and a reaction to the answer. Alice also owns a second closed
    /// group, c11, which carol alone is a member of.
    async fn state() -> (AppState, String) {
        let state = blob_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        token_for(&state, "did:plc:carol").await;
        let (status, up) =
            upload(&state, &alice, "c9", "application/pdf", b"%PDF the agenda").await;
        assert_eq!(status, StatusCode::OK, "{up}");
        let blob = up["id"].as_str().expect("id").to_string();
        let (status, up) = upload(&state, &alice, "c9", "image/png", b"a picture").await;
        assert_eq!(status, StatusCode::OK, "{up}");
        let picture = up["id"].as_str().expect("id").to_string();
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(&format!(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, created_at) \
               VALUES ('fo', 'c9', 'c9', 'folder', 'Bilag', 'bilag', 'closed/bilag', '2026-01-05T08:00:00.000Z');
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, content) \
               VALUES ('pg', 'c9', 'fo', 'document', 'Dagsorden', 'dagsorden', 'closed/bilag/dagsorden', \
                       '[{{\"children\":[{{\"text\":\"Valg af dirigent\"}}]}}]');
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, data) \
               VALUES ('fi', 'c9', 'fo', 'file', 'Agenda.pdf', 'agenda_pdf', 'closed/bilag/agenda_pdf', \
                       '{{\"fileId\":\"{blob}\",\"type\":\"application/pdf\"}}');
             INSERT INTO document_author (document_id, author_text, ord) VALUES ('pg', 'Sekretariatet', 0);
             INSERT INTO comment (id, on_id, root_id, context_id, author_did, text) \
               VALUES ('kp', 'pg', 'pg', 'c9', 'did:plc:bob', 'Punkt 3 mangler');
             INSERT INTO comment (id, on_id, root_id, context_id, author_did, text, image) \
               VALUES ('kr', 'kp', 'pg', 'c9', 'did:plc:alice', 'Rettet', '{picture}');
             INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
               VALUES ('rr', 'kr', 'did:plc:bob', '👍');
             INSERT INTO context (id, kind, name, slug, path) \
               VALUES ('c11', 'group', 'Other Group', 'other', 'other');"
        ))
        .await
        .expect("seed");
        join_as(&state, "did:plc:alice", "c11", "owner").await;
        join(&state, "did:plc:carol", "c11").await;
        crate::search::rebuild(&state.db).await.expect("index");
        (state, blob)
    }

    async fn file_of(state: &AppState, path: &str, who: &str) -> String {
        let uri = format!("/xrpc/com.example.wiki.getNode?path={path}");
        let (status, v) = get_as(router(state.clone()), &uri, who).await;
        assert_eq!(status, StatusCode::OK, "{path}: {v}");
        v["node"]["data"]["fileId"]
            .as_str()
            .expect("fileId")
            .to_string()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_copy_is_the_same_content_somewhere_else_with_files_of_its_own() {
        let (state, blob) = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let (status, v) = post(
            router(state.clone()),
            COPY,
            Some(&alice),
            json!({"id": "fo", "parent_id": "c9"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            v["path"], "closed/bilag-2",
            "beside its source, so under the next name"
        );
        assert_eq!(v["copied"], 3);

        let (_, page) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getNode?path=closed/bilag-2/dagsorden",
            &alice,
        )
        .await;
        assert_eq!(
            page["node"]["authors"][0]["display"], "Sekretariatet",
            "{page}"
        );
        assert_eq!(
            page["node"]["owner_did"], "did:plc:alice",
            "a copy is the copier's"
        );
        let (_, folder) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getNode?path=closed/bilag-2",
            &alice,
        )
        .await;
        assert_eq!(
            folder["node"]["created_at"], "2026-01-05T08:00:00.000Z",
            "dated now"
        );
        assert_eq!(
            count(
                &state,
                "SELECT count(*) FROM comment WHERE on_id <> 'pg' AND text = 'Punkt 3 mangler'"
            )
            .await,
            0,
            "the discussion was copied along with what it was about"
        );

        // A file of its own, over the same bytes: deleting one leaves the other.
        let copied_blob = file_of(&state, "closed/bilag-2/agenda_pdf", &alice).await;
        assert_ne!(copied_blob, blob);
        let (_, _, bytes) =
            fetch(&state, &format!("/blob/{copied_blob}"), Some(&alice), None).await;
        assert_eq!(bytes, b"%PDF the agenda");
        let agendas = "SELECT count(DISTINCT sha256) FROM blob WHERE mime = 'application/pdf'";
        assert_eq!(count(&state, agendas).await, 1);

        let (_, found) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.search?q=dirigent",
            &alice,
        )
        .await;
        assert_eq!(
            found["hits"].as_array().expect("hits").len(),
            2,
            "the copy is not found: {found}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_copy_into_another_group_is_readable_there_and_the_original_is_not() {
        let (state, blob) = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let carol = token_for(&state, "did:plc:carol").await;
        let (status, v) = post(
            router(state.clone()),
            COPY,
            Some(&alice),
            json!({"id": "fo", "parent_id": "c11"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["path"], "other/bilag");

        let copied_blob = file_of(&state, "other/bilag/agenda_pdf", &carol).await;
        let (status, _, bytes) =
            fetch(&state, &format!("/blob/{copied_blob}"), Some(&carol), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "the copy's file stayed in the group it came from"
        );
        assert_eq!(bytes, b"%PDF the agenda");
        let (status, _, _) = fetch(&state, &format!("/blob/{blob}"), Some(&carol), None).await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "and the original opened to the other group"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn copying_is_an_owners_and_never_into_itself() {
        let (state, _) = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let carol = token_for(&state, "did:plc:carol").await;
        let copy = |who: String, id: &'static str, parent: &'static str| {
            let state = state.clone();
            async move {
                post(
                    router(state),
                    COPY,
                    Some(&who),
                    json!({"id": id, "parent_id": parent}),
                )
                .await
            }
        };
        assert_eq!(copy(bob, "fo", "c9").await.0, StatusCode::FORBIDDEN);
        assert_eq!(
            copy(carol, "fo", "c11").await.0,
            StatusCode::NOT_FOUND,
            "copying out of a group one cannot read"
        );
        let (status, v) = copy(alice.clone(), "fo", "pg").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "into its own subtree: {v}");
        let (status, v) = copy(alice, "fo", "nowhere").await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        assert_eq!(
            count(
                &state,
                "SELECT count(*) FROM document WHERE title = 'Bilag'"
            )
            .await,
            1
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_purge_takes_the_documents_and_what_only_they_held() {
        let (state, blob) = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let purge = |who: String| {
            let state = state.clone();
            async move { post(router(state), PURGE, Some(&who), json!({"id": "fo"})).await }
        };
        let (status, v) = purge(alice.clone()).await;
        assert_eq!(
            status,
            StatusCode::BAD_REQUEST,
            "purged straight out of the tree: {v}"
        );

        post(
            router(state.clone()),
            BIN,
            Some(&alice),
            json!({"id": "fo"}),
        )
        .await;
        assert_eq!(purge(bob).await.0, StatusCode::FORBIDDEN);
        let (status, v) = purge(alice.clone()).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["purged"], 3);

        for (what, sql) in [
            (
                "documents",
                "SELECT count(*) FROM document WHERE id IN ('fo', 'pg', 'fi')",
            ),
            (
                "author chips",
                "SELECT count(*) FROM document_author WHERE document_id = 'pg'",
            ),
            (
                "comments, the answers to them too",
                "SELECT count(*) FROM comment WHERE root_id = 'pg'",
            ),
            (
                "reactions",
                "SELECT count(*) FROM reaction WHERE subject_uri IN ('kp', 'kr')",
            ),
            (
                "index rows",
                "SELECT count(*) FROM search_index WHERE node_id IN ('fo', 'pg', 'fi')",
            ),
            ("blob rows", "SELECT count(*) FROM blob"),
        ] {
            assert_eq!(count(&state, sql).await, 0, "{what} outlived the purge");
        }
        let (status, _, _) = fetch(&state, &format!("/blob/{blob}"), Some(&alice), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND);
        assert_eq!(purge(alice).await.0, StatusCode::NOT_FOUND, "twice");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_purge_leaves_a_file_something_else_points_at_and_a_votes_record() {
        let (state, blob) = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, data) \
             VALUES ('cover', 'c9', 'c9', 'document', 'Forside', 'forside', 'closed/forside', ?1)",
            [json!({"image": blob}).to_string()],
        )
        .await
        .expect("cover");
        post(
            router(state.clone()),
            BIN,
            Some(&alice),
            json!({"id": "fo"}),
        )
        .await;
        let (status, _) = post(
            router(state.clone()),
            PURGE,
            Some(&alice),
            json!({"id": "fo"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _, _) = fetch(&state, &format!("/blob/{blob}"), Some(&alice), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "a cover image went with a file node that shared it"
        );

        // A motion with a poll on it goes to the bin whole, and stays there.
        conn.execute(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
             VALUES ('mo', 'c9', 'c9', 'policy', 'Forslag', 'forslag', 'closed/forslag')",
            (),
        )
        .await
        .expect("motion");
        let (status, poll) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.openPoll",
            Some(&alice),
            json!({"parent_id": "mo", "title": "Forslag", "options": ["for", "against", "blank"]}),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{poll}");
        post(
            router(state.clone()),
            BIN,
            Some(&alice),
            json!({"id": "mo"}),
        )
        .await;
        let (status, v) = post(
            router(state.clone()),
            PURGE,
            Some(&alice),
            json!({"id": "mo"}),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{v}");
        assert_eq!(v["error"], "NotPurgeable");
        assert_eq!(
            count(&state, "SELECT count(*) FROM document WHERE id = 'mo'").await,
            1
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_move_into_another_group_takes_the_comments_and_the_files_along() {
        let (state, blob) = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let carol = token_for(&state, "did:plc:carol").await;

        // Owning where it is, is not owning where it would go.
        join_as(&state, "did:plc:bob", "c11", "member").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "UPDATE member SET role = 'owner' WHERE user_did = 'did:plc:bob' AND context_id = 'c9'",
            (),
        )
        .await
        .expect("bob owns c9");
        let body = json!({"id": "fo", "parent_id": "c11"});
        let (status, v) = post(router(state.clone()), MOVE, Some(&bob), body.clone()).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{v}");

        let (status, v) = post(router(state.clone()), MOVE, Some(&alice), body).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(v["path"], "other/bilag");
        for (what, sql) in [
            (
                "documents",
                "SELECT count(*) FROM document WHERE id IN ('fo','pg','fi') AND context_id <> 'c11'",
            ),
            (
                "comments, the answers to them too",
                "SELECT count(*) FROM comment WHERE root_id = 'pg' AND context_id <> 'c11'",
            ),
            (
                "files",
                "SELECT count(*) FROM blob WHERE context_id <> 'c11'",
            ),
        ] {
            assert_eq!(count(&state, sql).await, 0, "{what} stayed behind");
        }
        // Carol, of the new group only, reads all of it; the old group's bob,
        // were he not also there, would read none.
        assert_eq!(
            file_of(&state, "other/bilag/agenda_pdf", &carol).await,
            blob
        );
        let (status, _, _) = fetch(&state, &format!("/blob/{blob}"), Some(&carol), None).await;
        assert_eq!(status, StatusCode::OK);
        let (_, comments) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getComments?on=pg",
            &carol,
        )
        .await;
        assert_eq!(
            comments["comments"].as_array().expect("comments").len(),
            1,
            "{comments}"
        );
        // An answer used to stay in the old group: read there, and not here.
        let (_, answers) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.getComments?on=kp",
            &carol,
        )
        .await;
        let answers = answers["comments"].as_array().expect("answers");
        assert_eq!(answers.len(), 1, "{answers:?}");
        let picture = answers[0]["image"].as_str().expect("its picture");
        let (status, _, _) = fetch(&state, &format!("/blob/{picture}"), Some(&carol), None).await;
        assert_eq!(status, StatusCode::OK, "the picture in it went along");
    }

    /// The view that lists what has gone astray is there so that it can be
    /// cleared away, which the AppView could list and not do.
    #[tokio::test(flavor = "current_thread")]
    async fn whoever_runs_the_site_clears_away_what_has_gone_astray() {
        let (mut state, _) = state().await;
        state.config.site_owner = Some("did:plc:carol".to_string());
        crate::context::ensure_home(&state.db, &state.config)
            .await
            .expect("home");
        let alice = token_for(&state, "did:plc:alice").await;
        let carol = token_for(&state, "did:plc:carol").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('lost', 'c9', 'gone', 'folder', 'Lost', 'lost', 'closed/gone/lost');
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('lost-page', 'c9', 'lost', 'document', 'Side', 'side', 'closed/gone/lost/side');
             INSERT INTO comment (id, on_id, root_id, context_id, author_did, text) \
               VALUES ('k-lost', 'lost-page', 'lost-page', 'c9', 'did:plc:bob', 'Hvor blev den af?');
             INSERT INTO comment (id, on_id, root_id, context_id, author_did, text) \
               VALUES ('k-astray', 'nothing', 'nothing', 'c9', 'did:plc:bob', 'On nothing');
             INSERT INTO comment (id, on_id, root_id, context_id, author_did, text) \
               VALUES ('k-answer', 'k-astray', 'nothing', 'c9', 'did:plc:alice', 'An answer to it');
             INSERT INTO reaction (id, subject_uri, reactor_did, emoji) \
               VALUES ('r-astray', 'k-answer', 'did:plc:bob', '👍');",
        )
        .await
        .expect("seed");
        let purge = |who: &str, id: &'static str| {
            let (state, who) = (state.clone(), who.to_string());
            async move {
                let uri = "/xrpc/com.example.wiki.purgeOrphan";
                post(router(state), uri, Some(&who), json!({"id": id})).await
            }
        };

        let (status, v) = purge(&alice, "lost").await;
        assert_eq!(
            status,
            StatusCode::FORBIDDEN,
            "she runs a group, not the site: {v}"
        );
        let (status, v) = purge(&carol, "pg").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "a page in its place is no orphan: {v}"
        );
        let (status, v) = purge(&carol, "kp").await;
        assert_eq!(
            status,
            StatusCode::NOT_FOUND,
            "nor is a comment on one: {v}"
        );

        let (status, v) = purge(&carol, "lost").await;
        assert_eq!((status, &v["purged"]), (StatusCode::OK, &json!(2)), "{v}");
        let (status, v) = purge(&carol, "k-astray").await;
        assert_eq!((status, &v["purged"]), (StatusCode::OK, &json!(2)), "{v}");
        for (what, sql) in [
            (
                "pages",
                "SELECT count(*) FROM document WHERE id IN ('lost', 'lost-page')",
            ),
            (
                "comments",
                "SELECT count(*) FROM comment WHERE id IN ('k-lost', 'k-astray', 'k-answer')",
            ),
            (
                "reactions",
                "SELECT count(*) FROM reaction WHERE id = 'r-astray'",
            ),
        ] {
            assert_eq!(count(&state, sql).await, 0, "{what} outlived the purge");
        }
        assert_eq!(
            count(
                &state,
                "SELECT count(*) FROM comment WHERE id IN ('kp', 'kr')"
            )
            .await,
            2,
            "what is in its place was left alone"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_running_poll_does_not_change_groups() {
        let (state, _) = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
             VALUES ('mo', 'c9', 'fo', 'policy', 'Forslag', 'forslag', 'closed/bilag/forslag')",
            (),
        )
        .await
        .expect("motion");
        let (_, poll) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.openPoll",
            Some(&alice),
            json!({"parent_id": "mo", "title": "Forslag", "options": ["for", "against", "blank"]}),
        )
        .await;
        let id = poll["id"].as_str().expect("id");
        let body = json!({"id": "fo", "parent_id": "c11"});
        let (status, v) = post(router(state.clone()), MOVE, Some(&alice), body.clone()).await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{v}");
        assert_eq!(
            count(
                &state,
                "SELECT count(*) FROM document WHERE context_id = 'c11'"
            )
            .await,
            0
        );

        post(
            router(state.clone()),
            "/xrpc/com.example.wiki.closePoll",
            Some(&alice),
            json!({"id": id}),
        )
        .await;
        let (status, v) = post(router(state.clone()), MOVE, Some(&alice), body).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(
            count(&state, "SELECT count(*) FROM poll WHERE context_id = 'c11'").await,
            1,
            "a closed poll's record stayed with the group it left"
        );
    }
}
