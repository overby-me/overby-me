//! The BACK of the migration pipeline (rewrite kickoff item 9): write an
//! `Extraction` (from `migration-extractor`) into a staging Turso db under the
//! generated entity schema (`wiki_schema::ENTITY_SCHEMA`). It realizes two
//! properties the schema was designed for:
//!
//! - **FK ORDER**: users and contexts are inserted before the documents /
//!   members / comments that reference them, and a document before its
//!   author-join rows. The load turns foreign-key enforcement on and checks that
//!   it took, so a dump that points at a row it does not contain fails here, at
//!   the rehearsal, rather than loading quietly and surfacing as a page with a
//!   piece missing.
//! - **THE TREE**: `parent_id` is not a foreign key, because a parent may be a
//!   context or a document. The load checks every one itself, before it writes
//!   anything.
//! - **IDEMPOTENCY**: every entity is keyed by its primary key (checked before
//!   insert) and carries `legacy_id UNIQUE`, so re-running the big-bang load
//!   never duplicates a row. A document's author-join rows load only when the
//!   document itself is new, so they are idempotent as a unit.
//!
//! What this does NOT do: apply the DDL (the caller runs `ENTITY_SCHEMA` once),
//! or load what lives in the AppView's own tables (a poll's result, a canvas's
//! cells, feedback). `appview import` does both, and calls this for the rest.

use migration_extractor::Extraction;
use std::collections::BTreeSet;
use turso::{Connection, Value};
use wiki_domain_types::Place;

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
    pub reactions: usize,
}

#[derive(Debug)]
pub enum LoadError {
    Turso(turso::Error),
    Json(serde_json::Error),
    /// The engine would not turn foreign-key enforcement on, so the load would
    /// have checked nothing.
    ForeignKeysOff,
    /// A node names a parent that is neither in the extraction nor already
    /// loaded, as a context or as a document.
    DanglingParent {
        node: String,
        parent: String,
    },
}

impl std::fmt::Display for LoadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LoadError::Turso(e) => write!(f, "load query error: {e}"),
            LoadError::Json(e) => write!(f, "load json error: {e}"),
            LoadError::ForeignKeysOff => write!(f, "foreign keys are not enforced"),
            LoadError::DanglingParent { node, parent } => {
                write!(f, "node {node} has no loadable parent {parent}")
            }
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
/// Omitting it lets the column's `NOT NULL DEFAULT` fire; passing an
/// explicit NULL would violate the NOT NULL constraint (default notwithstanding).
fn push_created_at(
    cols: &mut Vec<&'static str>,
    params: &mut Vec<Value>,
    created_at: &Option<String>,
) {
    push_timestamp(cols, params, "created_at", created_at);
}

fn push_timestamp(
    cols: &mut Vec<&'static str>,
    params: &mut Vec<Value>,
    col: &'static str,
    at: &Option<String>,
) {
    if let Some(ts) = at {
        cols.push(col);
        params.push(Value::Text(ts.clone()));
    }
}

/// The tree columns `context` and `document` share.
fn push_place(cols: &mut Vec<&'static str>, params: &mut Vec<Value>, place: &Place) {
    cols.extend([
        "slug",
        "path",
        "parent_id",
        "idx",
        "attachable",
        "owner_did",
        "deleted_at",
        "deleted_root",
    ]);
    params.extend([
        text(&place.slug),
        text(&place.path),
        opt(&place.parent_id),
        Value::Integer(place.idx),
        boolv(place.attachable),
        opt(&place.owner_did),
        opt(&place.deleted_at),
        opt(&place.deleted_root),
    ]);
    push_timestamp(cols, params, "created_at", &place.created_at);
    push_timestamp(cols, params, "updated_at", &place.updated_at);
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

async fn enforce_foreign_keys(conn: &Connection) -> Result<(), LoadError> {
    conn.execute("PRAGMA foreign_keys=ON", ()).await?;
    let mut rows = conn.query("PRAGMA foreign_keys", ()).await?;
    match rows.next().await? {
        Some(row) if row.get::<i64>(0)? == 1 => Ok(()),
        _ => Err(LoadError::ForeignKeysOff),
    }
}

/// Refuse an extraction in which some node's parent is nowhere: not in the
/// extraction, and not already loaded. The schema cannot say this (see the
/// module doc), and a node with a missing parent is unreachable by any path.
async fn check_parents(conn: &Connection, ex: &Extraction) -> Result<(), LoadError> {
    let arriving: BTreeSet<&str> = ex
        .contexts
        .iter()
        .map(|c| c.id.as_str())
        .chain(ex.documents.iter().map(|d| d.id.as_str()))
        .collect();
    let places = ex
        .contexts
        .iter()
        .map(|c| (&c.id, &c.place))
        .chain(ex.documents.iter().map(|d| (&d.id, &d.place)));
    for (id, place) in places {
        let Some(parent) = place.parent_id.as_deref() else {
            continue;
        };
        if arriving.contains(parent)
            || exists(conn, "context", "id", parent).await?
            || exists(conn, "document", "id", parent).await?
        {
            continue;
        }
        return Err(LoadError::DanglingParent {
            node: id.clone(),
            parent: parent.to_string(),
        });
    }
    Ok(())
}

/// Load an `Extraction` into `conn` (which must already have `ENTITY_SCHEMA`
/// applied), in FK order and idempotently by primary key. Returns the count of
/// newly inserted rows per table.
pub async fn load(conn: &Connection, ex: &Extraction) -> Result<LoadStats, LoadError> {
    enforce_foreign_keys(conn).await?;
    check_parents(conn, ex).await?;
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
            "visibility",
            "published_uri",
            "legacy_id",
        ];
        let mut params = vec![
            text(&c.id),
            enum_val(&c.kind)?,
            text(&c.name),
            enum_val(&c.visibility)?,
            opt(&c.published_uri),
            opt(&c.legacy_id),
        ];
        push_place(&mut cols, &mut params, &c.place);
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
        let data = match &d.data {
            Some(v) => Value::Text(serde_json::to_string(v)?),
            None => Value::Null,
        };
        let mut cols = vec![
            "id",
            "context_id",
            "kind",
            "title",
            "mutable",
            "content",
            "data",
            "visibility",
            "published_uri",
            "legacy_id",
        ];
        let mut params = vec![
            text(&d.id),
            text(&d.context_id),
            enum_val(&d.kind)?,
            text(&d.title),
            boolv(d.mutable),
            content,
            data,
            enum_val(&d.visibility)?,
            opt(&d.published_uri),
            opt(&d.legacy_id),
        ];
        push_place(&mut cols, &mut params, &d.place);
        insert(conn, "document", &cols, params).await?;
        stats.documents += 1;
        for (ord, a) in d.authors.iter().enumerate() {
            insert(
                conn,
                "document_author",
                &[
                    "document_id",
                    "author_did",
                    "author_text",
                    "author_context",
                    "ord",
                ],
                vec![
                    text(&d.id),
                    opt_str(a.did()),
                    opt_str(a.text()),
                    opt_str(a.context()),
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
                "name",
                "hidden",
                "accepted",
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
                opt(&m.name),
                boolv(m.hidden),
                boolv(m.accepted),
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
            "root_id",
            "context_id",
            "author_did",
            "author_text",
            "text",
            "image",
            "tombstone",
            "deleted_at",
            "deleted_root",
            "legacy_id",
        ];
        // An extraction from before threads had a root: top-level is the guess
        // that is right for most, and wrong for a reply only until it is moved.
        let root = if k.root_id.is_empty() {
            &k.on_id
        } else {
            &k.root_id
        };
        let mut params = vec![
            text(&k.id),
            text(&k.on_id),
            text(root),
            text(&k.context_id),
            opt_str(k.author.did()),
            opt_str(k.author.text()),
            text(&k.text),
            opt(&k.image),
            boolv(k.tombstone),
            opt(&k.deleted_at),
            opt(&k.deleted_root),
            opt(&k.legacy_id),
        ];
        push_created_at(&mut cols, &mut params, &k.created_at);
        insert(conn, "comment", &cols, params).await?;
        stats.comments += 1;
    }

    // 6. Reactions. One by an account the dump does not hold is kept without
    //    its reactor, which the column allows and the foreign key would not.
    for r in &ex.reactions {
        if exists(conn, "reaction", "id", &r.id).await? {
            continue;
        }
        let reactor = match r.reactor_did.as_deref() {
            Some(did) if exists(conn, "user", "did", did).await? => Some(did),
            _ => None,
        };
        let mut cols = vec!["id", "subject_uri", "reactor_did", "emoji", "legacy_id"];
        let mut params = vec![
            text(&r.id),
            text(&r.subject_uri),
            opt_str(reactor),
            text(&r.emoji),
            opt(&r.legacy_id),
        ];
        push_created_at(&mut cols, &mut params, &r.created_at);
        insert(conn, "reaction", &cols, params).await?;
        stats.reactions += 1;
    }

    Ok(stats)
}

#[cfg(test)]
mod tests {
    use super::*;
    use wiki_domain_types::{
        Author, Comment, Context, ContextKind, Document, DocumentKind, Member, Reaction, Role,
        User, Visibility,
    };

    fn place(slug: &str, path: &str, parent: Option<&str>) -> Place {
        Place {
            slug: slug.into(),
            path: path.into(),
            parent_id: parent.map(str::to_string),
            attachable: true,
            ..Place::default()
        }
    }

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
                place: place("group-one", "group-one", None),
                visibility: Visibility::Private,
                published_uri: None,
                legacy_id: Some("c1".into()),
            }],
            documents: vec![Document {
                id: "d1".into(),
                context_id: "c1".into(),
                kind: DocumentKind::Document,
                title: "Doc".into(),
                place: Place {
                    idx: 4,
                    attachable: false,
                    owner_did: Some("did:plc:alice".into()),
                    updated_at: Some("2026-02-02 00:00:00".into()),
                    deleted_at: Some("2026-03-03 00:00:00".into()),
                    ..place("doc", "group-one/doc", Some("c1"))
                },
                mutable: false,
                content: Some(serde_json::json!({"blocks": [{"text": "hi"}]})),
                data: Some(serde_json::json!({"image": "file-1"})),
                // One DID author, one free-text author (the reconciled model).
                authors: vec![
                    Author::User {
                        did: "did:plc:alice".into(),
                    },
                    Author::FreeText {
                        display: "Guest".into(),
                    },
                    Author::Context {
                        context_id: "c1".into(),
                        name: None,
                        path: None,
                    },
                ],
                visibility: Visibility::Private,
                published_uri: None,
                legacy_id: Some("d1".into()),
            }],
            members: vec![Member {
                id: "m1".into(),
                user_did: Some("did:plc:alice".into()),
                context_id: "c1".into(),
                role: Role::Owner,
                active: true,
                name: Some("Alice A.".into()),
                hidden: true,
                accepted: true,
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
                image: Some("file-9".into()),
                tombstone: false,
                root_id: "d1".into(),
                created_at: None,
                deleted_at: Some("2026-05-05 00:00:00".into()),
                deleted_root: Some("k1".into()),
                legacy_id: Some("k1".into()),
            }],
            reactions: vec![
                reaction("r1", Some("did:plc:alice")),
                reaction("r2", Some("did:plc:gone")),
            ],
            ..Default::default()
        }
    }

    fn reaction(id: &str, reactor: Option<&str>) -> Reaction {
        Reaction {
            id: id.into(),
            subject_uri: "d1".into(),
            reactor_did: reactor.map(str::to_string),
            emoji: "🎉".into(),
            created_at: Some("2026-04-04 00:00:00".into()),
            legacy_id: Some(id.into()),
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
                document_authors: 3,
                members: 1,
                comments: 1,
                reactions: 2,
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
        assert_eq!(count(&conn, "document_author").await, 3);
        assert_eq!(count(&conn, "member").await, 1);
        assert_eq!(count(&conn, "comment").await, 1);
        assert_eq!(count(&conn, "reaction").await, 2);

        let mut rows = conn
            .query(
                "SELECT root_id || '|' || image || '|' || tombstone || '|' || deleted_root \
                 FROM comment WHERE id = 'k1' AND deleted_at IS NOT NULL",
                (),
            )
            .await
            .expect("q");
        let comment: String = rows
            .next()
            .await
            .expect("row")
            .expect("the comment, in the bin")
            .get(0)
            .expect("text");
        assert_eq!(comment, "d1|file-9|0|k1");

        let mut rows = conn
            .query("SELECT reactor_did FROM reaction WHERE id = 'r2'", ())
            .await
            .expect("q");
        let reactor = rows.next().await.expect("row").expect("some");
        assert!(
            matches!(reactor.get_value(0).expect("value"), Value::Null),
            "a reaction by an account the dump does not hold keeps no reactor"
        );

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
        let mut rows = conn
            .query(
                "SELECT author_context FROM document_author WHERE document_id = 'd1' AND ord = 2",
                (),
            )
            .await
            .expect("q");
        let group: String = rows
            .next()
            .await
            .expect("row")
            .expect("some")
            .get(0)
            .expect("text");
        assert_eq!(group, "c1", "the group named as an author is stored as one");
    }

    async fn staging() -> Connection {
        let db = turso::Builder::new_local(":memory:")
            .build()
            .await
            .expect("build");
        let conn = db.connect().expect("connect");
        conn.execute_batch(wiki_schema::ENTITY_SCHEMA)
            .await
            .expect("DDL");
        conn
    }

    fn context(id: &str, parent: Option<&str>) -> Context {
        Context {
            id: id.into(),
            kind: ContextKind::Event,
            name: id.into(),
            place: place(id, id, parent),
            visibility: Visibility::Private,
            published_uri: None,
            legacy_id: Some(id.into()),
        }
    }

    /// A dump lists rows in whatever order the database returned them.
    #[tokio::test(flavor = "current_thread")]
    async fn a_child_context_listed_before_its_parent_still_loads() {
        let conn = staging().await;
        let ex = Extraction {
            contexts: vec![
                context("grandchild", Some("child")),
                context("child", Some("root")),
                context("root", None),
            ],
            ..Default::default()
        };
        let stats = load(&conn, &ex).await.expect("load");
        assert_eq!(stats.contexts, 3);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_context_whose_parent_is_nowhere_stops_the_load_by_name() {
        let conn = staging().await;
        let ex = Extraction {
            contexts: vec![context("root", None), context("lost", Some("gone"))],
            ..Default::default()
        };
        match load(&conn, &ex).await {
            Err(LoadError::DanglingParent { node, parent }) => {
                assert_eq!(node, "lost");
                assert_eq!(parent, "gone");
            }
            other => panic!("expected a dangling-parent error, got {other:?}"),
        }
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_refused_extraction_writes_nothing() {
        let conn = staging().await;
        let ex = Extraction {
            contexts: vec![context("root", None), context("lost", Some("gone"))],
            ..Default::default()
        };
        assert!(load(&conn, &ex).await.is_err());
        assert_eq!(count(&conn, "context").await, 0, "half a tree was loaded");
    }

    /// The interim lets a group or an event sit in a folder.
    #[tokio::test(flavor = "current_thread")]
    async fn a_context_may_hang_off_a_document() {
        let conn = staging().await;
        let mut ex = sample();
        ex.contexts.push(context("nested", Some("d1")));
        load(&conn, &ex).await.expect("load");
        assert_eq!(count(&conn, "context").await, 2);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn the_tree_columns_survive_the_load() {
        let conn = staging().await;
        load(&conn, &sample()).await.expect("load");
        let mut rows = conn
            .query(
                "SELECT slug, path, parent_id, idx, mutable, attachable, owner_did, data, \
                        updated_at, deleted_at FROM document WHERE id = 'd1'",
                (),
            )
            .await
            .expect("query");
        let row = rows.next().await.expect("next").expect("row");
        let text = |i: usize| row.get::<String>(i).expect("text");
        assert_eq!(text(0), "doc");
        assert_eq!(text(1), "group-one/doc");
        assert_eq!(text(2), "c1");
        assert_eq!(row.get::<i64>(3).expect("idx"), 4);
        assert_eq!(row.get::<i64>(4).expect("mutable"), 0);
        assert_eq!(row.get::<i64>(5).expect("attachable"), 0);
        assert_eq!(text(6), "did:plc:alice");
        assert_eq!(text(7), r#"{"image":"file-1"}"#);
        assert_eq!(text(8), "2026-02-02 00:00:00");
        assert_eq!(text(9), "2026-03-03 00:00:00");
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_members_name_and_flags_survive_the_load() {
        let conn = staging().await;
        load(&conn, &sample()).await.expect("load");
        let mut rows = conn
            .query(
                "SELECT name, hidden, accepted FROM member WHERE id = 'm1'",
                (),
            )
            .await
            .expect("query");
        let row = rows.next().await.expect("next").expect("row");
        assert_eq!(row.get::<String>(0).expect("name"), "Alice A.");
        assert_eq!(row.get::<i64>(1).expect("hidden"), 1);
        assert_eq!(row.get::<i64>(2).expect("accepted"), 1);
    }

    /// The load is the rehearsal's integrity check, so it must be checking.
    #[tokio::test(flavor = "current_thread")]
    async fn a_row_pointing_at_nothing_fails_the_load() {
        let conn = staging().await;
        let mut ex = sample();
        ex.documents[0].context_id = "no-such-context".into();
        assert!(
            load(&conn, &ex).await.is_err(),
            "a document in a context the dump does not contain was loaded"
        );
    }
}
