//! What has happened lately: the feed, what a person or a group has put
//! forward, and the admin's list of nodes that lost their parent.
//!
//! A feed row is light. It says what happened and what it is about, and the
//! interim's rows each carried their whole document, which is what made three
//! letters in a search box cost 1.5 MB.

use crate::AppState;
use crate::authz::{Authz, readable_comment, readable_document};
use crate::db::DbError;
use crate::search::{NodeRef, node_refs};
use crate::session::{Caller, MaybeCaller};
use crate::xrpc::{err, forbidden, invalid, write_failed};
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use turso::Value;

/// What counts as activity worth listing. The interim's list, less reactions,
/// which here only ever mirror public records and have no context to be in.
const FEED_KINDS: &str = "'document','policy','change','position','candidate','file'";

/// What someone is credited with on their profile: what they put forward.
const CONTRIBUTION_KINDS: &str = "'policy','change','candidate','question'";

const MAX_PAGE: i64 = 100;
/// Paging is by offset, which costs what it skips; nobody pages this far.
const MAX_OFFSET: i64 = 2000;

#[derive(Debug, Serialize)]
pub struct Item {
    /// `document` or `comment`.
    pub node: &'static str,
    pub id: String,
    /// A document's kind; `comment` for a comment.
    pub kind: String,
    /// A document's title, or a comment's text.
    pub text: String,
    /// A document's own path. A comment is reached through `about`.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub path: Option<String>,
    pub context_id: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_did: Option<String>,
    /// A comment by someone with no account.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub by_text: Option<String>,
    pub created_at: String,
    /// Where a document sits, or what a comment is on, if the caller may read it.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub about: Option<NodeRef>,
    #[serde(skip)]
    about_id: Option<String>,
}

/// Which rows a listing is of. Every listing is also held to what the caller may
/// read, whatever it asks for.
enum Of {
    /// One context's own content. A group's feed is not its events': each is its
    /// own context.
    Context(String),
    /// Everything in the contexts the caller belongs to.
    Mine(String),
    /// Whatever is public.
    Public,
    /// What one person put forward.
    ByPerson(String),
    /// What a group is named as the author of.
    ByGroup(String),
}

fn opt(row: &turso::Row, i: usize) -> Option<String> {
    match row.get_value(i) {
        Ok(Value::Text(s)) => Some(s),
        _ => None,
    }
}

/// The newest `limit` rows after `offset`, documents and comments together.
async fn list(
    state: &AppState,
    of: &Of,
    caller: Option<&str>,
    limit: i64,
    offset: i64,
) -> Result<Vec<Item>, DbError> {
    let who = caller.map_or(Value::Null, |did| Value::Text(did.to_string()));
    // ?1 is the caller, ?2 what the listing is of.
    let (documents, comments, subject) = match of {
        Of::Context(id) => ("d.context_id = ?2", Some("k.context_id = ?2"), id.clone()),
        Of::Mine(did) => (
            "d.context_id IN (SELECT m.context_id FROM member m WHERE m.user_did = ?2)",
            Some("k.context_id IN (SELECT m.context_id FROM member m WHERE m.user_did = ?2)"),
            did.clone(),
        ),
        Of::Public => ("?2 = ''", Some("?2 = ''"), String::new()),
        Of::ByPerson(did) => (
            "(d.owner_did = ?2 OR EXISTS (SELECT 1 FROM document_author a \
               WHERE a.document_id = d.id AND a.author_did = ?2))",
            Some("k.author_did = ?2"),
            did.clone(),
        ),
        Of::ByGroup(id) => (
            "EXISTS (SELECT 1 FROM document_author a \
               WHERE a.document_id = d.id AND a.author_context = ?2)",
            None,
            id.clone(),
        ),
    };
    let contributions = matches!(of, Of::ByPerson(_) | Of::ByGroup(_));
    let kinds = if contributions {
        CONTRIBUTION_KINDS
    } else {
        FEED_KINDS
    };
    // A feed lists what has been submitted; a draft is nobody's news. What
    // someone put forward is theirs while it is still a draft, too.
    let submitted = if contributions {
        ""
    } else {
        "AND d.mutable = 0"
    };
    // Its parent must still be there: a row that opens nowhere is `listOrphans`'.
    let placed = |id: &str| {
        format!(
            "(EXISTS (SELECT 1 FROM document p WHERE p.id = {id} AND p.deleted_at IS NULL) \
              OR EXISTS (SELECT 1 FROM context p WHERE p.id = {id} AND p.deleted_at IS NULL))"
        )
    };
    let take = limit + offset;
    let conn = state.db.acquire().await?;
    let mut items = Vec::new();

    let sql = format!(
        "SELECT d.id, d.kind, d.title, d.path, d.context_id, d.owner_did, d.created_at, d.parent_id \
         FROM document d \
         WHERE d.deleted_at IS NULL AND d.kind IN ({kinds}) {submitted} AND {} AND {documents} \
           AND {} \
         ORDER BY d.created_at DESC, d.id DESC LIMIT {take}",
        readable_document("d", 1),
        placed("d.parent_id"),
    );
    let mut rows = conn
        .query(&sql, vec![who.clone(), Value::Text(subject.clone())])
        .await?;
    while let Some(row) = rows.next().await? {
        items.push(Item {
            node: "document",
            id: row.get(0)?,
            kind: row.get(1)?,
            text: row.get(2)?,
            path: Some(row.get(3)?),
            context_id: row.get(4)?,
            by_did: opt(&row, 5),
            by_text: None,
            created_at: row.get(6)?,
            about: None,
            about_id: opt(&row, 7),
        });
    }
    if let Some(comments) = comments {
        let sql = format!(
            // About the document its thread is on, so a reply is news of that
            // document too. An emptied comment is nobody's news.
            "SELECT k.id, k.text, k.context_id, k.author_did, k.author_text, k.created_at, k.root_id \
             FROM comment k WHERE {} AND {comments} AND {} AND k.tombstone = 0 \
             ORDER BY k.created_at DESC, k.id DESC LIMIT {take}",
            readable_comment("k", 1),
            placed("k.root_id"),
        );
        let mut rows = conn.query(&sql, vec![who, Value::Text(subject)]).await?;
        while let Some(row) = rows.next().await? {
            items.push(Item {
                node: "comment",
                id: row.get(0)?,
                kind: "comment".to_string(),
                text: row.get(1)?,
                path: None,
                context_id: row.get(2)?,
                by_did: opt(&row, 3),
                by_text: opt(&row, 4),
                created_at: row.get(5)?,
                about: None,
                about_id: opt(&row, 6),
            });
        }
    }
    // Two sorted lists into one: each was cut at `limit + offset`, so the first
    // that many of the two together are all here.
    items.sort_by(|a, b| {
        b.created_at
            .cmp(&a.created_at)
            .then_with(|| b.id.cmp(&a.id))
    });
    let mut page: Vec<Item> = items
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize)
        .collect();
    let about: BTreeSet<String> = page.iter().filter_map(|i| i.about_id.clone()).collect();
    let about = node_refs(&conn, &about, caller).await?;
    for item in &mut page {
        item.about = item.about_id.as_ref().and_then(|id| about.get(id).cloned());
    }
    Ok(page)
}

/// A listing with the profile behind every DID in it.
async fn answer(
    state: &AppState,
    of: Of,
    caller: Option<&str>,
    page: (Option<i64>, Option<i64>),
    what: &str,
) -> Response {
    let limit = page.0.unwrap_or(20).clamp(1, MAX_PAGE);
    let offset = page.1.unwrap_or(0).clamp(0, MAX_OFFSET);
    let listed = async {
        let items = list(state, &of, caller, limit, offset).await?;
        let dids: BTreeSet<String> = items.iter().filter_map(|i| i.by_did.clone()).collect();
        let profiles = crate::Store::new(state.db.clone()).profiles(&dids).await?;
        Ok::<_, DbError>((items, profiles))
    };
    match listed.await {
        Ok((items, profiles)) => (
            StatusCode::OK,
            Json(serde_json::json!({ "items": items, "profiles": profiles })),
        )
            .into_response(),
        Err(e) => write_failed(what, e),
    }
}

#[derive(Debug, Deserialize)]
pub struct RecentParams {
    /// One context's own feed. Absent: everything in the contexts the caller
    /// belongs to, or whatever is public for someone not signed in.
    #[serde(default)]
    pub context: Option<String>,
    // Not a flattened struct: through `flatten` a query string's numbers arrive
    // as strings, and are refused.
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// `com.example.wiki.listRecent`: the feed. Submitted content and comments,
/// newest first, each with what it is about.
pub async fn list_recent(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<RecentParams>,
) -> Response {
    let of = match (p.context, caller.did()) {
        (Some(context), _) => Of::Context(context),
        (None, Some(did)) => Of::Mine(did.to_string()),
        (None, None) => Of::Public,
    };
    answer(&state, of, caller.did(), (p.limit, p.offset), "listRecent").await
}

#[derive(Debug, Deserialize)]
pub struct ContributionParams {
    #[serde(default)]
    pub did: Option<String>,
    /// A group or an event, for what it is named as the author of.
    #[serde(default)]
    pub context: Option<String>,
    #[serde(default)]
    pub limit: Option<i64>,
    #[serde(default)]
    pub offset: Option<i64>,
}

/// `com.example.wiki.listContributions`: what a person, or a group, has put
/// forward, as far as the caller may read it.
pub async fn list_contributions(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<ContributionParams>,
) -> Response {
    let of = match (p.did, p.context) {
        (Some(did), None) => Of::ByPerson(did),
        (None, Some(context)) => Of::ByGroup(context),
        _ => return invalid("give a did or a context, and not both"),
    };
    answer(
        &state,
        of,
        caller.did(),
        (p.limit, p.offset),
        "listContributions",
    )
    .await
}

#[derive(Debug, Serialize)]
pub struct Orphan {
    /// `document`, `context` or `comment`.
    pub node: &'static str,
    pub id: String,
    pub kind: String,
    pub text: String,
    /// The parent that is not there.
    pub parent_id: String,
}

/// `com.example.wiki.listOrphans`: the nodes whose parent no longer exists, for
/// an owner of the site to put right. A parent is a plain column, since it may
/// be of either kind, so nothing but this notices one going missing.
pub async fn list_orphans(State(state): State<AppState>, Caller { did }: Caller) -> Response {
    let what = "listOrphans";
    match Authz::new(state.db.clone()).owns_a_site(&did).await {
        Ok(true) => {}
        Ok(false) => return forbidden("only an owner of the site sees what has gone astray"),
        Err(e) => return write_failed(what, e),
    }
    let nowhere = |id: &str| {
        format!(
            "{id} IS NOT NULL \
             AND NOT EXISTS (SELECT 1 FROM document p WHERE p.id = {id}) \
             AND NOT EXISTS (SELECT 1 FROM context p WHERE p.id = {id})"
        )
    };
    let found = async {
        let conn = state.db.acquire().await?;
        let mut orphans = Vec::new();
        for (node, sql) in [
            (
                "document",
                format!(
                    "SELECT d.id, d.kind, d.title, d.parent_id FROM document d WHERE {}",
                    nowhere("d.parent_id")
                ),
            ),
            (
                "context",
                format!(
                    "SELECT c.id, c.kind, c.name, c.parent_id FROM context c WHERE {}",
                    nowhere("c.parent_id")
                ),
            ),
            // By what its thread is on: an answer's own parent is a comment,
            // which is no node, and every answer would be listed as astray.
            (
                "comment",
                format!(
                    "SELECT k.id, 'comment', k.text, k.root_id FROM comment k WHERE {} \
                       AND NOT EXISTS (SELECT 1 FROM post p WHERE p.id = k.root_id)",
                    nowhere("k.root_id")
                ),
            ),
        ] {
            let mut rows = conn.query(&sql, ()).await?;
            while let Some(row) = rows.next().await? {
                orphans.push(Orphan {
                    node,
                    id: row.get(0)?,
                    kind: row.get(1)?,
                    text: row.get(2)?,
                    parent_id: row.get(3)?,
                });
            }
        }
        Ok::<_, DbError>(orphans)
    };
    match found.await {
        Ok(orphans) => (
            StatusCode::OK,
            Json(serde_json::json!({ "orphans": orphans })),
        )
            .into_response(),
        Err(e) => {
            tracing::error!("{what} failed: {e}");
            err(StatusCode::BAD_GATEWAY, "InternalError", "read failed")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{get, get_as, join_as, post, seeded_state, token_for};
    use serde_json::json;

    /// The seeded state with something submitted in both groups: `d1` (public
    /// group c1) and `s1` (closed group c9), and a comment on each.
    async fn state() -> AppState {
        let state = seeded_state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "UPDATE document SET mutable = 0, created_at = '2026-01-01T10:00:00.000Z' WHERE id = 'd1';
             UPDATE document SET mutable = 0, created_at = '2026-01-02T10:00:00.000Z', \
               owner_did = 'did:plc:alice' WHERE id = 's1';
             UPDATE comment SET created_at = '2026-01-03T10:00:00.000Z' WHERE id = 'ks';
             UPDATE comment SET created_at = '2026-01-01T09:00:00.000Z' WHERE id = 'k1';",
        )
        .await
        .expect("seed");
        state
    }

    fn ids(v: &serde_json::Value) -> Vec<&str> {
        v["items"]
            .as_array()
            .expect("items")
            .iter()
            .map(|i| i["id"].as_str().expect("id"))
            .collect()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_feed_is_what_was_submitted_where_the_caller_belongs() {
        let state = state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let recent = "/xrpc/com.example.wiki.listRecent";

        let (_, v) = get_as(router(state.clone()), recent, &bob).await;
        assert_eq!(
            ids(&v),
            ["ks", "s1"],
            "bob belongs to the closed group only: {v}"
        );
        assert_eq!(v["items"][0]["about"]["title"], "Secret Minutes", "{v}");
        assert_eq!(v["items"][1]["about"]["title"], "Closed Group");
        assert!(v["profiles"]["did:plc:alice"].is_object(), "{v}");
        assert!(
            v["items"][1].get("content").is_none(),
            "a feed row carries its whole document"
        );

        // Signed out: what is public, and no draft (d2 was never submitted).
        let (_, v) = get(router(state.clone()), recent).await;
        assert_eq!(ids(&v), ["d1", "k1"], "{v}");

        // One context's own feed, to whoever may read it.
        let of_closed = format!("{recent}?context=c9");
        let (_, v) = get_as(router(state.clone()), &of_closed, &bob).await;
        assert_eq!(ids(&v), ["ks", "s1"]);
        let (_, v) = get(router(state.clone()), &of_closed).await;
        assert_eq!(
            ids(&v),
            Vec::<&str>::new(),
            "a closed group's feed was served outside it"
        );

        let (_, v) = get_as(
            router(state.clone()),
            &format!("{recent}?limit=1&offset=1"),
            &bob,
        )
        .await;
        assert_eq!(ids(&v), ["s1"], "the second page of one");

        // An answer is news of the document its thread is on, as the comment it
        // answers is. It used to be news of nothing, and was left out.
        let answer = json!({"on_id": "ks", "text": "Enig"});
        let said = "/xrpc/com.example.wiki.postComment";
        let (status, said) = post(router(state.clone()), said, Some(&bob), answer).await;
        assert_eq!(status, StatusCode::OK, "{said}");
        let (_, v) = get_as(router(state.clone()), &of_closed, &bob).await;
        assert_eq!(v["items"][0]["id"], said["id"], "{v}");
        assert_eq!(v["items"][0]["about"]["title"], "Secret Minutes", "{v}");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_went_to_the_bin_with_its_parent_is_not_news() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let (status, _) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.deleteDocument",
            Some(&alice),
            json!({"id": "s1"}),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (_, v) = get_as(
            router(state.clone()),
            "/xrpc/com.example.wiki.listRecent",
            &alice,
        )
        .await;
        assert_eq!(
            ids(&v),
            Vec::<&str>::new(),
            "a comment on a binned document: {v}"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_person_and_a_group_are_credited_with_what_they_put_forward() {
        let state = state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let bob = token_for(&state, "did:plc:bob").await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO document (id, context_id, parent_id, kind, title, slug, path, owner_did) \
               VALUES ('mo', 'c9', 'c9', 'policy', 'Forslag', 'forslag', 'closed/forslag', 'did:plc:bob');
             INSERT INTO document_author (document_id, author_context, ord) VALUES ('mo', 'c1', 0);",
        )
        .await
        .expect("motion");
        let by = "/xrpc/com.example.wiki.listContributions";

        let (_, v) = get_as(
            router(state.clone()),
            &format!("{by}?did=did:plc:bob"),
            &alice,
        )
        .await;
        assert_eq!(ids(&v), ["mo"], "a draft motion is still bob's: {v}");
        // Alice is an author of d1 (a chip) and wrote both comments.
        let (_, v) = get_as(
            router(state.clone()),
            &format!("{by}?did=did:plc:alice"),
            &bob,
        )
        .await;
        assert_eq!(ids(&v), ["ks", "d1", "k1"], "{v}");
        let (_, v) = get(router(state.clone()), &format!("{by}?did=did:plc:alice")).await;
        assert_eq!(ids(&v), ["d1", "k1"], "signed out: the public half");

        let (_, v) = get_as(router(state.clone()), &format!("{by}?context=c1"), &bob).await;
        assert_eq!(
            ids(&v),
            ["mo"],
            "what Group One is named as the author of: {v}"
        );
        let (_, v) = get(router(state.clone()), &format!("{by}?context=c1")).await;
        assert_eq!(
            ids(&v),
            Vec::<&str>::new(),
            "a closed group's motion, to the signed out"
        );
        let (status, _) = get(router(state.clone()), by).await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn what_lost_its_parent_is_listed_for_the_site_owner() {
        let state = state().await;
        let conn = state.db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO context (id, kind, name, slug, path) VALUES ('home', 'site', 'Home', 'home', 'home');
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('lost', 'c1', 'gone', 'document', 'Lost Page', 'lost', 'group-one/lost');
             INSERT INTO comment (id, on_id, root_id, context_id, author_did, text) \
               VALUES ('kl', 'gone-too', 'gone-too', 'c1', 'did:plc:alice', 'On nothing');
             INSERT INTO comment (id, on_id, root_id, context_id, author_did, text) \
               VALUES ('ka', 'k1', 'd1', 'c1', 'did:plc:alice', 'An answer, which is in its place');",
        )
        .await
        .expect("seed");
        let carol = token_for(&state, "did:plc:carol").await;
        join_as(&state, "did:plc:carol", "home", "owner").await;
        let uri = "/xrpc/com.example.wiki.listOrphans";

        let (status, v) = get_as(router(state.clone()), uri, &carol).await;
        assert_eq!(status, StatusCode::OK, "{v}");
        let found: Vec<&str> = v["orphans"]
            .as_array()
            .expect("orphans")
            .iter()
            .map(|o| o["id"].as_str().expect("id"))
            .collect();
        assert_eq!(found, ["lost", "kl"], "{v}");

        let bob = token_for(&state, "did:plc:bob").await;
        assert_eq!(
            get_as(router(state.clone()), uri, &bob).await.0,
            StatusCode::FORBIDDEN
        );
        // And an orphan is not news: it would open nowhere.
        let (_, v) = get(router(state.clone()), "/xrpc/com.example.wiki.listRecent").await;
        assert!(!ids(&v).contains(&"kl"), "{v}");
    }
}
