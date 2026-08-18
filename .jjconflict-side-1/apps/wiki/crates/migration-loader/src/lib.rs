//! The BACK of the migration pipeline (rewrite kickoff item 9): write an
//! `Extraction` (from `migration-extractor`) into a staging Turso db under the
//! generated entity schema (`wiki_schema::ENTITY_SCHEMA`). It realizes two
//! properties the schema was designed for:
//!
//! - **FK ORDER**: users and contexts are inserted before the documents /
//!   members / comments that reference them, and a document before its
//!   author-join rows. turso 0.2.2 does NOT enforce foreign keys (a recorded
//!   gap; see `crates/appview/src/db.rs`), so this ordering is the
//!   application-layer referential integrity the loader upholds regardless.
//! - **IDEMPOTENCY**: every entity is keyed by its primary key (checked before
//!   insert) and carries `legacy_id UNIQUE`, so re-running the big-bang load
//!   never duplicates a row. A document's author-join rows load only when the
//!   document itself is new, so they are idempotent as a unit.
//!
//! What this does NOT do: apply the DDL (the caller runs `ENTITY_SCHEMA` once)
//! and the voting entities (excluded from the content/membership migration).
//! Intra-table parent ordering (a child context before its parent) relies on
//! turso not enforcing FKs; when turso gains FK support, add a topological sort.

use migration_extractor::Extraction;
use turso::{Connection, Value};

/// What a load inserted (new rows only; already-present rows are skipped), per
/// table. A second load of the same `Extraction` yields all zeros.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct LoadStats {
    pub users: usize,
    pub contexts: usize,
    pub documents: usize,
    pub document_authors: usize,
    pub members: usize,
    pub comments: usize,
}

#[derive(Debug)]
pub enum LoadError {
    Turso(turso::Error),
    Json(serde_json::Error),
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Turso(e) => write!(f, "load query error: {e}"),
            LoadError::Json(e) => write!(f, "load json error: {e}"),
        }
    }
}

impl std::error::Error for LoadError {}

impl From<turso::Error> for LoadError {
    fn from(e: turso::Error) -> Self {
        LoadError::Turso(e)
    }
}
impl From<serde_json::Error> for LoadError {
    fn from(e: serde_json::Error) -> Self {
        LoadError::Json(e)
    }
}

fn text(s: &str) -> Value {
    Value::Text(s.to_string())
}

fn opt(s: &Option<String>) -> Value {
    match s {
        Some(v) => Value::Text(v.clone()),
        None => Value::Null,
    }
}

fn opt_str(s: Option<&str>) -> Value {
    match s {
        Some(v) => Value::Text(v.to_string()),
        None => Value::Null,
    }
}

fn boolv(b: bool) -> Value {
    Value::Integer(if b { 1 } else { 0 })
}

/// Append the `created_at` column + param ONLY when a source timestamp exists.
/// Omitting it lets the `NOT NULL DEFAULT (datetime('now'))` fire; passing an
/// explicit NULL would violate the NOT NULL constraint (default notwithstanding).
fn push_created_at(cols: &mut Vec<&str>, params: &mut Vec<Value>, created_at: &Option<String>) {
    if let Some(ts) = created_at {
        cols.push("created_at");
        params.push(Value::Text(ts.clone()));
    }
}

/// Serialize a `#[serde(rename_all = "snake_case")]` domain enum to its DB
/// string value (e.g. `ContextKind::Group` -> `"group"`), so the enum stays the
/// single source of truth for the CHECK-constrained column values.
fn enum_val<T: serde::Serialize>(v: &T) -> Result<Value, LoadError> {
    match serde_json::to_value(v)? {
        serde_json::Value::String(s) => Ok(Value::Text(s)),
        other => Ok(Value::Text(other.to_string())),
    }
}

async fn exists(conn: &Connection, table: &str, col: &str, key: &str) -> Result<bool, LoadError> {
    let mut rows = conn
        .query(
            &format!("SELECT 1 FROM {table} WHERE {col} = ?1 LIMIT 1"),
            [key],
        )
        .await?;
    Ok(rows.next().await?.is_some())
}

/// INSERT a row from aligned `cols`/`params`. Columns are only ever included
/// when a value is provided; a `NOT NULL DEFAULT` column (e.g. `created_at`) is
/// OMITTED when its source is `None` so the DB default applies (turso, like
/// SQLite, rejects an explicit NULL on a NOT NULL column even with a default).
async fn insert(
    conn: &Connection,
    table: &str,
    cols: &[&str],
    params: Vec<Value>,
) -> Result<(), LoadError> {
    let placeholders = (1..=params.len())
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ");
    conn.execute(
        &format!(
            "INSERT INTO {table} ({}) VALUES ({placeholders})",
            cols.join(", ")
        ),
        params,
    )
    .await?;
    Ok(())
}

/// Load an `Extraction` into `conn` (which must already have `ENTITY_SCHEMA`
/// applied), in FK order and idempotently by primary key. Returns the count of
/// newly inserted rows per table.
pub async fn load(conn: &Connection, ex: &Extraction) -> Result<LoadStats, LoadError> {
    let mut stats = LoadStats::default();

    // 1. Users: the FK target every author / member / comment references.
    for u in &ex.users {
        if exists(conn, "user", "did", &u.did).await? {
            continue;
        }
        insert(
            conn,
            "user",
            &["did", "handle", "display_name", "avatar_url", "legacy_id"],
            vec![
                text(&u.did),
                opt(&u.handle),
                opt(&u.display_name),
                opt(&u.avatar_url),
                opt(&u.legacy_id),
            ],
        )
        .await?;
        stats.users += 1;
    }

    // 2. Contexts (groups/events): before the documents/members/comments in them.
    for c in &ex.contexts {
        if exists(conn, "context", "id", &c.id).await? {
            continue;
        }
        let mut cols = vec![
            "id",
            "kind",
            "name",
            "slug",
            "parent_id",
            "visibility",
            "published_uri",
            "legacy_id",
        ];
        let mut params = vec![
            text(&c.id),
            enum_val(&c.kind)?,
            text(&c.name),
            text(&c.slug),
            opt(&c.parent_id),
            enum_val(&c.visibility)?,
            opt(&c.published_uri),
            opt(&c.legacy_id),
        ];
        push_created_at(&mut cols, &mut params, &c.created_at);
        insert(conn, "context", &cols, params).await?;
        stats.contexts += 1;
    }

    // 3. Documents + their author-join rows (as an idempotent unit).
    for d in &ex.documents {
        if exists(conn, "document", "id", &d.id).await? {
            continue;
        }
        let content = match &d.content {
            Some(v) => Value::Text(serde_json::to_string(v)?),
            None => Value::Null,
        };
        let mut cols = vec![
            "id",
            "context_id",
            "parent_id",
            "kind",
            "title",
            "content",
            "visibility",
            "published_uri",
            "legacy_id",
        ];
        let mut params = vec![
            text(&d.id),
            text(&d.context_id),
            opt(&d.parent_id),
            enum_val(&d.kind)?,
            text(&d.title),
            content,
            enum_val(&d.visibility)?,
            opt(&d.published_uri),
            opt(&d.legacy_id),
        ];
        push_created_at(&mut cols, &mut params, &d.created_at);
        insert(conn, "document", &cols, params).await?;
        stats.documents += 1;
        for (ord, a) in d.authors.iter().enumerate() {
            insert(
                conn,
                "document_author",
                &["document_id", "author_did", "author_text", "ord"],
                vec![
                    text(&d.id),
                    opt_str(a.did()),
                    opt_str(a.text()),
                    Value::Integer(ord as i64),
                ],
            )
            .await?;
            stats.document_authors += 1;
        }
    }

    // 4. Members (no created_at column).
    for m in &ex.members {
        if exists(conn, "member", "id", &m.id).await? {
            continue;
        }
        insert(
            conn,
            "member",
            &[
                "id",
                "user_did",
                "context_id",
                "role",
                "active",
                "email",
                "claim_token",
                "legacy_id",
            ],
            vec![
                text(&m.id),
                opt(&m.user_did),
                text(&m.context_id),
                enum_val(&m.role)?,
                boolv(m.active),
                opt(&m.email),
                opt(&m.claim_token),
                opt(&m.legacy_id),
            ],
        )
        .await?;
        stats.members += 1;
    }

    // 5. Comments.
    for k in &ex.comments {
        if exists(conn, "comment", "id", &k.id).await? {
            continue;
        }
        let mut cols = vec![
            "id",
            "on_id",
            "context_id",
            "author_did",
            "author_text",
            "text",
            "legacy_id",
        ];
        let mut params = vec![
            text(&k.id),
            text(&k.on_id),
            text(&k.context_id),
            opt_str(k.author.did()),
            opt_str(k.author.text()),
            text(&k.text),
            opt(&k.legacy_id),
        ];
        push_created_at(&mut cols, &mut params, &k.created_at);
        insert(conn, "comment", &cols, params).await?;
        stats.comments += 1;
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiki_domain_types::{
        Author, Comment, Context, ContextKind, Document, DocumentKind, Member, Role, User,
        Visibility,
    };

    fn sample() -> Extraction {
        Extraction {
            users: vec![User {
                did: "did:plc:alice".into(),
                handle: Some("alice.test".into()),
                display_name: Some("Alice".into()),
                avatar_url: None,
                legacy_id: Some("u1".into()),
            }],
            contexts: vec![Context {
                id: "c1".into(),
                kind: ContextKind::Group,
                name: "Group One".into(),
                slug: "group-one".into(),
                parent_id: None,
                visibility: Visibility::Private,
                published_uri: None,
                created_at: None,
                legacy_id: Some("c1".into()),
            }],
            documents: vec![Document {
                id: "d1".into(),
                context_id: "c1".into(),
                parent_id: None,
                kind: DocumentKind::Document,
                title: "Doc".into(),
                content: Some(serde_json::json!({"blocks": [{"text": "hi"}]})),
                // One DID author, one free-text author (the reconciled model).
                authors: vec![
                    Author::User {
                        did: "did:plc:alice".into(),
                    },
                    Author::FreeText {
                        display: "Guest".into(),
                    },
                ],
                visibility: Visibility::Private,
                published_uri: None,
                created_at: None,
                legacy_id: Some("d1".into()),
            }],
            members: vec![Member {
                id: "m1".into(),
                user_did: Some("did:plc:alice".into()),
                context_id: "c1".into(),
                role: Role::Owner,
                active: true,
                email: Some("alice@x.dk".into()),
                claim_token: Some("tok".into()),
                legacy_id: Some("m1".into()),
            }],
            comments: vec![Comment {
                id: "k1".into(),
                on_id: "d1".into(),
                context_id: "c1".into(),
                author: Author::FreeText {
                    display: "A Guest".into(),
                },
                text: "nice".into(),
                created_at: None,
                legacy_id: Some("k1".into()),
            }],
            ..Default::default()
        }
    }

    async fn count(conn: &Connection, table: &str) -> i64 {
        let mut rows = conn
            .query(&format!("SELECT count(*) FROM {table}"), ())
            .await
            .expect("count query");
        rows.next()
            .await
            .expect("row")
            .expect("some")
            .get::<i64>(0)
            .expect("i64")
    }

    #[tokio::test(flavor = "current_thread")]
    async fn load_is_idempotent_and_fk_ordered() {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build");
        let conn = db.connect().expect("connect");
        conn.execute_batch(wiki_schema::ENTITY_SCHEMA)
            .await
            .expect("DDL");

        let ex = sample();
        let first = load(&conn, &ex).await.expect("first load");
        assert_eq!(
            first,
            LoadStats {
                users: 1,
                contexts: 1,
                documents: 1,
                document_authors: 2,
                members: 1,
                comments: 1,
            },
            "first load inserts every row"
        );

        // A second load of the same extraction is a complete no-op.
        let second = load(&conn, &ex).await.expect("second load");
        assert_eq!(
            second,
            LoadStats::default(),
            "re-running the big-bang load inserts nothing (legacy_id + PK idempotency)"
        );

        // Row counts are stable (no duplicates), including the author join.
        assert_eq!(count(&conn, "user").await, 1);
        assert_eq!(count(&conn, "context").await, 1);
        assert_eq!(count(&conn, "document").await, 1);
        assert_eq!(count(&conn, "document_author").await, 2);
        assert_eq!(count(&conn, "member").await, 1);
        assert_eq!(count(&conn, "comment").await, 1);

        // Spot-check the free-text-vs-DID authorship landed correctly.
        let mut rows = conn
            .query(
                "SELECT count(*) FROM document_author WHERE document_id = 'd1' AND author_text IS NOT NULL",
                (),
            )
            .await
            .expect("q");
        let free_text: i64 = rows
            .next()
            .await
            .expect("row")
            .expect("some")
            .get(0)
            .expect("i64");
        assert_eq!(
            free_text, 1,
            "the free-text author is stored as author_text"
        );
    }
}
