//! Finding things by what they are called and what they say.
//!
//! A document's content is Slate JSON, so a `LIKE` over it matches the JSON as
//! much as the text: every document "contains" `children`, `type` and `bold`.
//! And SQLite folds case for ASCII only, which leaves `Årsmøde` unfindable as
//! `årsmøde`. So the words are taken out of the JSON and lowercased HERE, into
//! an index of their own.
//!
//! The index is derived and nothing else: it is rebuilt from `document` and
//! `context` at every start, so the migration loader never has to know of it
//! and it cannot drift for longer than a process lives. Writes keep it fresh in
//! between.

use crate::AppState;
use crate::authz::{readable_context, readable_document};
use crate::db::{Db, DbError};
use crate::session::MaybeCaller;
use crate::xrpc::write_failed;
use axum::Json;
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::{Deserialize, Serialize};
use turso::{Connection, Value};

pub const SEARCH_DDL: &str = r#"
CREATE TABLE IF NOT EXISTS search_index (
  node_id TEXT PRIMARY KEY,
  title   TEXT NOT NULL,            -- folded
  body    TEXT NOT NULL             -- folded; empty for a context
);
"#;

/// More terms than this and the rest are dropped: a query is a few words, and
/// each term is another pair of scans.
const MAX_TERMS: usize = 8;

/// A search box shows a page of hits, not every hit. Unbounded, the interim
/// answered 407 rows and 1.5 MB for three letters.
const MAX_HITS: i64 = 30;

/// How text is compared: lowercased, by Unicode's rules and not ASCII's.
pub fn fold(text: &str) -> String {
    text.to_lowercase()
}

/// The words of a Slate document. Leaves of one block are runs of one line, so
/// they join with nothing between them (a word half in bold is still a word);
/// blocks join with a newline.
pub fn plain_text(content: &serde_json::Value) -> String {
    fn walk(value: &serde_json::Value, out: &mut String) {
        match value {
            serde_json::Value::Array(items) => items.iter().for_each(|item| walk(item, out)),
            serde_json::Value::Object(map) => {
                if let Some(serde_json::Value::String(text)) = map.get("text") {
                    out.push_str(text);
                }
                let before = out.len();
                for (key, inner) in map {
                    if key != "text" {
                        walk(inner, out);
                    }
                }
                if out.len() > before && !out.ends_with('\n') {
                    out.push('\n');
                }
            }
            _ => {}
        }
    }
    let mut out = String::new();
    walk(content, &mut out);
    out.trim_end().to_string()
}

/// Index a node, replacing what was there. `content` is a document's JSON as
/// stored; a context has none.
pub async fn index(
    conn: &Connection,
    node_id: &str,
    title: &str,
    content: Option<&str>,
) -> Result<(), turso::Error> {
    let body = content
        .and_then(|json| serde_json::from_str::<serde_json::Value>(json).ok())
        .map(|json| fold(&plain_text(&json)))
        .unwrap_or_default();
    conn.execute(
        "INSERT INTO search_index (node_id, title, body) VALUES (?1, ?2, ?3) \
         ON CONFLICT(node_id) DO UPDATE SET title = excluded.title, body = excluded.body",
        [node_id, fold(title).as_str(), body.as_str()],
    )
    .await?;
    Ok(())
}

/// Index a document from its row, as it now stands.
pub async fn index_document(conn: &Connection, id: &str) -> Result<(), turso::Error> {
    let mut rows = conn
        .query("SELECT title, content FROM document WHERE id = ?1", [id])
        .await?;
    let Some(row) = rows.next().await? else {
        return Ok(());
    };
    let title: String = row.get(0)?;
    let content = match row.get_value(1)? {
        Value::Text(json) => Some(json),
        _ => None,
    };
    drop(rows);
    index(conn, id, &title, content.as_deref()).await
}

/// Rebuild the whole index from the rows it is derived from. Returns how many
/// nodes it now holds.
pub async fn rebuild(db: &Db) -> Result<usize, DbError> {
    let conn = db.acquire().await?;
    let mut nodes: Vec<(String, String, Option<String>)> = Vec::new();
    let mut rows = conn.query("SELECT id, name FROM context", ()).await?;
    while let Some(row) = rows.next().await? {
        nodes.push((row.get(0)?, row.get(1)?, None));
    }
    let mut rows = conn
        .query("SELECT id, title, content FROM document", ())
        .await?;
    while let Some(row) = rows.next().await? {
        let content = match row.get_value(2)? {
            Value::Text(json) => Some(json),
            _ => None,
        };
        nodes.push((row.get(0)?, row.get(1)?, content));
    }
    drop(rows);
    let _turn = db.write_turn().await;
    conn.execute("BEGIN IMMEDIATE", ()).await?;
    let written: Result<(), turso::Error> = async {
        conn.execute("DELETE FROM search_index", ()).await?;
        for (id, title, content) in &nodes {
            index(&conn, id, title, content.as_deref()).await?;
        }
        Ok(())
    }
    .await;
    conn.execute(
        if written.is_ok() {
            "COMMIT"
        } else {
            "ROLLBACK"
        },
        (),
    )
    .await?;
    written?;
    Ok(nodes.len())
}

/// What a row is about, or sits in: enough to say "in <parent>" and link there.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct NodeRef {
    /// `document` or `context`.
    pub node: &'static str,
    pub id: String,
    pub kind: String,
    pub title: String,
    pub path: String,
}

/// The live nodes among `ids` that `caller` may read, by id.
pub async fn node_refs(
    conn: &Connection,
    ids: &std::collections::BTreeSet<String>,
    caller: Option<&str>,
) -> Result<std::collections::BTreeMap<String, NodeRef>, DbError> {
    let mut out = std::collections::BTreeMap::new();
    if ids.is_empty() {
        return Ok(out);
    }
    let who = caller.map_or(Value::Null, |did| Value::Text(did.to_string()));
    let places: String = (0..ids.len())
        .map(|i| format!("?{}", i + 2))
        .collect::<Vec<_>>()
        .join(", ");
    for (node, sql) in [
        (
            "document",
            format!(
                "SELECT d.id, d.kind, d.title, d.path FROM document d \
                 WHERE d.id IN ({places}) AND d.deleted_at IS NULL AND {}",
                readable_document("d", 1)
            ),
        ),
        (
            "context",
            format!(
                "SELECT c.id, c.kind, c.name, c.path FROM context c \
                 WHERE c.id IN ({places}) AND c.deleted_at IS NULL AND {}",
                readable_context("c", 1)
            ),
        ),
    ] {
        let params: Vec<Value> = std::iter::once(who.clone())
            .chain(ids.iter().map(|id| Value::Text(id.clone())))
            .collect();
        let mut rows = conn.query(&sql, params).await?;
        while let Some(row) = rows.next().await? {
            let id: String = row.get(0)?;
            out.insert(
                id.clone(),
                NodeRef {
                    node,
                    id,
                    kind: row.get(1)?,
                    title: row.get(2)?,
                    path: row.get(3)?,
                },
            );
        }
    }
    Ok(out)
}

#[derive(Debug, Serialize)]
pub struct Hit {
    #[serde(flatten)]
    pub found: NodeRef,
    pub context_id: String,
    pub created_at: String,
    /// `title` when every term is in what it is called, else `text`.
    pub matched: &'static str,
    /// Where it sits, if the caller may read that too.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent: Option<NodeRef>,
}

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: String,
    /// Look inside one context only.
    #[serde(default)]
    pub context: Option<String>,
}

/// A `LIKE` pattern that finds `term` anywhere, with the wildcards a person may
/// have typed taken literally.
fn pattern(term: &str) -> String {
    let mut escaped = String::with_capacity(term.len() + 2);
    escaped.push('%');
    for c in term.chars() {
        if matches!(c, '%' | '_' | '\\') {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped.push('%');
    escaped
}

/// `com.example.wiki.search`: documents and contexts the caller may read, by
/// what they are called and what they say. Every word of the query has to be
/// found, in any order. What is CALLED the query comes before what mentions it,
/// and that is decided before the cut, so a title match is never crowded out.
pub async fn search(
    State(state): State<AppState>,
    caller: MaybeCaller,
    Query(p): Query<SearchParams>,
) -> Response {
    let whole = fold(p.q.trim());
    let terms: Vec<String> = whole
        .split_whitespace()
        .take(MAX_TERMS)
        .map(pattern)
        .collect();
    if terms.is_empty() {
        return (StatusCode::OK, Json(serde_json::json!({ "hits": [] }))).into_response();
    }
    // ?1 caller, ?2 the whole query, ?3 the whole query as a prefix, ?4 a
    // context or NULL, then the terms.
    let term_at = |i: usize| format!("?{}", i + 5);
    let every = |column: &str| {
        (0..terms.len())
            .map(|i| format!("{column} LIKE {} ESCAPE '\\'", term_at(i)))
            .collect::<Vec<_>>()
            .join(" AND ")
    };
    let anywhere = (0..terms.len())
        .map(|i| {
            let t = term_at(i);
            format!("(s.title LIKE {t} ESCAPE '\\' OR s.body LIKE {t} ESCAPE '\\')")
        })
        .collect::<Vec<_>>()
        .join(" AND ");
    let rank = format!(
        "CASE WHEN s.title = ?2 THEN 0 WHEN s.title LIKE ?3 ESCAPE '\\' THEN 1 \
              WHEN {} THEN 2 ELSE 3 END",
        every("s.title")
    );
    let documents = format!(
        "SELECT d.id, d.kind, d.title, d.path, d.context_id, d.parent_id, d.created_at, {rank} \
         FROM search_index s JOIN document d ON d.id = s.node_id \
         WHERE d.deleted_at IS NULL AND d.kind <> 'poll' AND {} AND {anywhere} \
           AND (?4 IS NULL OR d.context_id = ?4) \
         ORDER BY 8, d.created_at DESC LIMIT {MAX_HITS}",
        readable_document("d", 1)
    );
    // A context is found by its name, and only when nothing narrows the search
    // to the inside of one.
    let contexts = format!(
        "SELECT c.id, c.kind, c.name, c.path, c.id, c.parent_id, c.created_at, {rank} \
         FROM search_index s JOIN context c ON c.id = s.node_id \
         WHERE c.deleted_at IS NULL AND {} AND {} AND ?4 IS NULL \
         ORDER BY 8, c.created_at DESC LIMIT {MAX_HITS}",
        readable_context("c", 1),
        every("s.title")
    );
    let mut prefix = pattern(&whole);
    prefix.remove(0);
    let params: Vec<Value> = [
        caller
            .did()
            .map_or(Value::Null, |did| Value::Text(did.to_string())),
        Value::Text(whole.clone()),
        Value::Text(prefix),
        p.context.clone().map_or(Value::Null, Value::Text),
    ]
    .into_iter()
    .chain(terms.iter().cloned().map(Value::Text))
    .collect();

    let found = async {
        let conn = state.db.acquire().await?;
        let mut ranked: Vec<(i64, Hit, Option<String>)> = Vec::new();
        for (node, sql) in [("document", &documents), ("context", &contexts)] {
            let mut rows = conn.query(sql, params.clone()).await?;
            while let Some(row) = rows.next().await? {
                let rank: i64 = row.get(7)?;
                let parent = match row.get_value(5)? {
                    Value::Text(parent) => Some(parent),
                    _ => None,
                };
                let hit = Hit {
                    found: NodeRef {
                        node,
                        id: row.get(0)?,
                        kind: row.get(1)?,
                        title: row.get(2)?,
                        path: row.get(3)?,
                    },
                    context_id: row.get(4)?,
                    created_at: row.get(6)?,
                    matched: if rank < 3 { "title" } else { "text" },
                    parent: None,
                };
                ranked.push((rank, hit, parent));
            }
        }
        ranked.sort_by(|a, b| {
            a.0.cmp(&b.0)
                .then_with(|| b.1.created_at.cmp(&a.1.created_at))
        });
        ranked.truncate(MAX_HITS as usize);
        let parents = ranked.iter().filter_map(|r| r.2.clone()).collect();
        let parents = node_refs(&conn, &parents, caller.did()).await?;
        Ok::<_, DbError>(
            ranked
                .into_iter()
                .map(|(_, mut hit, parent)| {
                    hit.parent = parent.and_then(|id| parents.get(&id).cloned());
                    hit
                })
                .collect::<Vec<_>>(),
        )
    };
    match found.await {
        Ok(hits) => (StatusCode::OK, Json(serde_json::json!({ "hits": hits }))).into_response(),
        Err(e) => write_failed("search", e),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::router;
    use crate::xrpc::tests::{get, get_as, post, seeded_state, token_for};
    use serde_json::json;

    #[test]
    fn the_words_come_out_of_the_markup() {
        let slate = json!([
            {"type": "heading", "children": [{"text": "Klima"}]},
            {"type": "paragraph", "children": [
                {"text": "Vi vil "}, {"text": "halv", "bold": true}, {"text": "ere udledningen."}
            ]},
            {"type": "list", "children": [
                {"type": "item", "children": [{"text": "Punkt et"}]},
                {"type": "item", "children": [{"text": "Punkt to"}]}
            ]}
        ]);
        assert_eq!(
            plain_text(&slate),
            "Klima\nVi vil halvere udledningen.\nPunkt et\nPunkt to"
        );
        assert_eq!(plain_text(&json!({"blocks": [{"text": "hi"}]})), "hi");
        assert_eq!(plain_text(&json!(null)), "");
        assert_eq!(fold("ÅRSMØDE"), "årsmøde");
        assert_eq!(pattern("50%_a"), "%50\\%\\_a%");
    }

    async fn hits(state: &AppState, query: &str, who: Option<&str>) -> Vec<serde_json::Value> {
        let uri = format!("/xrpc/com.example.wiki.search?{query}");
        let (status, v) = match who {
            Some(who) => get_as(router(state.clone()), &uri, who).await,
            None => get(router(state.clone()), &uri).await,
        };
        assert_eq!(status, StatusCode::OK, "{v}");
        v["hits"].as_array().expect("hits").clone()
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_title_comes_before_a_mention_and_markup_is_not_text() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let create = |title: &str, text: &str| {
            let body = json!({
                "context_id": "c9", "kind": "document", "title": title,
                "content": [{"type": "paragraph", "children": [{"text": text}]}],
            });
            let state = state.clone();
            let alice = alice.clone();
            async move {
                post(
                    router(state),
                    "/xrpc/com.example.wiki.createDocument",
                    Some(&alice),
                    body,
                )
                .await
            }
        };
        create("Budget 2027", "Kontingentet hæves.").await;
        create("Referat", "Vi talte om klima og BUDGET for Årsmødet.").await;

        let found = hits(&state, "q=budget", Some(&alice)).await;
        let titles: Vec<&str> = found
            .iter()
            .map(|h| h["title"].as_str().expect("title"))
            .collect();
        assert_eq!(titles, ["Budget 2027", "Referat"], "{found:?}");
        assert_eq!(found[0]["matched"], "title");
        assert_eq!(found[1]["matched"], "text");
        assert_eq!(found[0]["parent"]["title"], "Closed Group", "{found:?}");

        // Every word, in any order, and Danish letters whatever their case.
        assert_eq!(
            hits(&state, "q=klima%20ÅRSMØDET", Some(&alice)).await.len(),
            1
        );
        assert!(
            hits(&state, "q=klima%20skat", Some(&alice))
                .await
                .is_empty()
        );
        // The JSON around the words is not among them.
        for markup in ["paragraph", "children", "type"] {
            assert!(
                hits(&state, &format!("q={markup}"), Some(&alice))
                    .await
                    .is_empty(),
                "{markup}"
            );
        }
        assert!(
            hits(&state, "q=%25", Some(&alice)).await.is_empty(),
            "a bare wildcard"
        );
        assert!(hits(&state, "q=%20", Some(&alice)).await.is_empty());
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_search_finds_groups_by_name_and_only_what_the_caller_may_read() {
        let state = seeded_state().await;
        let bob = token_for(&state, "did:plc:bob").await;
        let found = hits(&state, "q=closed", Some(&bob)).await;
        assert!(
            found
                .iter()
                .any(|h| h["node"] == "context" && h["id"] == "c9"),
            "{found:?}"
        );
        assert!(
            hits(&state, "q=closed", None).await.is_empty(),
            "a private group was found"
        );
        assert!(hits(&state, "q=secret", None).await.is_empty());
        assert_eq!(hits(&state, "q=secret", Some(&bob)).await.len(), 1);

        // Inside one context: its documents, and no contexts.
        let inside = hits(&state, "q=o&context=c1", Some(&bob)).await;
        assert!(!inside.is_empty());
        assert!(
            inside
                .iter()
                .all(|h| h["node"] == "document" && h["context_id"] == "c1")
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_rename_and_an_edit_are_found_at_once() {
        let state = seeded_state().await;
        let alice = token_for(&state, "did:plc:alice").await;
        let edit = json!({
            "id": "s1", "title": "Hemmeligt referat",
            "content": [{"children": [{"text": "Kassereren aflagde beretning."}]}],
        });
        let (status, v) = post(
            router(state.clone()),
            "/xrpc/com.example.wiki.updateDocument",
            Some(&alice),
            edit,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{v}");
        assert_eq!(hits(&state, "q=hemmeligt", Some(&alice)).await.len(), 1);
        assert_eq!(hits(&state, "q=kassereren", Some(&alice)).await.len(), 1);
        assert!(
            hits(&state, "q=secret", Some(&alice)).await.is_empty(),
            "the old name still finds it"
        );
    }
}
