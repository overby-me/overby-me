//! The AppView data-access seam, ported from the interim backend's
//! `backend/src/store.rs` admin-GraphQL bodies to parameterized SQL against the
//! reconciled Turso schema (`wiki_schema::ENTITY_SCHEMA` + this crate's
//! [`crate::schema::RUNTIME_DDL`]). The intent-named surface and the small typed
//! return structs are preserved so handlers stay storage-agnostic; only the
//! query bodies changed from GraphQL to SQL. This is the seam class where at
//! cutover only this module was ever meant to be rewritten.
//!
//! Who may read or write a row is not decided here: `crate::authz` owns that
//! rule, as SQL the reads below compose in and as predicates the handlers ask.
//! Voting queries arrive with the voting procedures.
//!
//! Schema shifts from the interim GraphQL these queries reconcile to:
//! - the universal `node` table split into `document`/`comment`/`context`. The
//!   first two are the spines of one tree and share a `Place` (a stored path,
//!   the parent, the order, the bin), so a path resolves in one lookup;
//! - a "node's owner + context" for a reply notification is read from
//!   `document`/`comment` with the owner realized as the document's first
//!   author DID (`author_did` in the `document_author` join), free-text-only
//!   authors having no notifiable DID;
//! - `members.nodeId`/`parentId`/`accepted` became `member.user_did`/
//!   `context_id` and the folded-in active state (there is no `accepted`);
//! - `push_subscriptions` is AppView runtime infra (`RUNTIME_DDL`), keyed by
//!   endpoint, with `user_id` now `user_did`.

use crate::authz::{readable_comment, readable_context, readable_document};
use crate::db::{Db, DbError};
use turso::Value;
use wiki_domain_types::{
    Author, Comment, Context, ContextKind, Document, DocumentKind, Place, Reaction, User,
};

/// Parse a snake_case DB enum value (e.g. `"document"`, `"private"`) into a
/// `#[serde(rename_all = "snake_case")]` domain enum; `None` on an unknown value.
fn parse_enum<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
}

/// The tree columns `context` and `document` share, as [`place_at`] reads them.
/// They end each column list, so the same reader serves both tables.
macro_rules! place_cols {
    () => {
        "slug, path, parent_id, idx, attachable, owner_did, created_at, updated_at, deleted_at, \
         deleted_root"
    };
}
/// The `document` columns the read side selects (order matches [`doc_base`]).
const DOC_COLS: &str = concat!(
    "id, context_id, kind, title, mutable, content, data, visibility, published_uri, ",
    place_cols!()
);
/// The `context` columns the read side selects (order matches [`ctx_from_row`]).
const CTX_COLS: &str = concat!("id, kind, name, visibility, published_uri, ", place_cols!());

/// A row is live unless it is in the bin. Every read but the bin's own asks.
const LIVE: &str = "deleted_at IS NULL";

/// The node whose path is `?2` and everything under it. Not `LIKE`: a slug is
/// full of underscores, and to `LIKE` an underscore is a wildcard.
const SUBTREE: &str = "(path = ?2 OR substr(path, 1, length(?2) + 1) = ?2 || '/')";

fn place_at(row: &turso::Row, first: usize) -> Result<Place, DbError> {
    Ok(Place {
        slug: row.get::<String>(first)?,
        path: row.get::<String>(first + 1)?,
        parent_id: opt_text(row, first + 2),
        idx: row.get::<i64>(first + 3)?,
        attachable: row.get::<i64>(first + 4)? != 0,
        owner_did: opt_text(row, first + 5),
        created_at: opt_text(row, first + 6),
        updated_at: opt_text(row, first + 7),
        deleted_at: opt_text(row, first + 8),
        deleted_root: opt_text(row, first + 9),
    })
}

/// The raw `document` row fields, before authors are hydrated.
struct DocBase {
    id: String,
    context_id: String,
    kind: String,
    title: String,
    mutable: bool,
    content: Option<String>,
    data: Option<String>,
    visibility: Option<String>,
    published_uri: Option<String>,
    place: Place,
}

fn doc_base(row: &turso::Row) -> Result<DocBase, DbError> {
    Ok(DocBase {
        id: row.get::<String>(0)?,
        context_id: row.get::<String>(1)?,
        kind: row.get::<String>(2)?,
        title: row.get::<String>(3)?,
        mutable: row.get::<i64>(4)? != 0,
        content: opt_text(row, 5),
        data: opt_text(row, 6),
        visibility: opt_text(row, 7),
        published_uri: opt_text(row, 8),
        place: place_at(row, 9)?,
    })
}

fn ctx_from_row(row: &turso::Row) -> Result<Context, DbError> {
    Ok(Context {
        id: row.get::<String>(0)?,
        kind: parse_enum(&row.get::<String>(1)?).unwrap_or(ContextKind::Group),
        name: row.get::<String>(2)?,
        visibility: opt_text(row, 3)
            .and_then(|s| parse_enum(&s))
            .unwrap_or_default(),
        published_uri: opt_text(row, 4),
        place: place_at(row, 5)?,
        legacy_id: None,
    })
}

/// Read a nullable TEXT column as an `Option<String>`. turso's `FromValue` has
/// no blanket `Option` impl, so a SQL NULL is matched on the raw `Value`.
fn opt_text(row: &turso::Row, idx: usize) -> Option<String> {
    match row.get_value(idx) {
        Ok(Value::Text(s)) => Some(s),
        _ => None,
    }
}

/// The data-access seam over the Turso datastore. Cheap to clone (wraps the
/// clonable `Db` handle); a fresh connection is acquired per call.
#[derive(Clone)]
pub struct Store {
    db: Db,
}

/// A node's owner + context (whose author a reply notification should reach).
/// `owner_id` is the notifiable author DID (the document's first `author_did`,
/// or a comment's `author_did`); a free-text-only author yields `None`.
pub struct NodeOwnerContext {
    pub owner_id: Option<String>,
    pub context_id: Option<String>,
}

/// A member row located by its secret claim token. Field names preserve the
/// interim seam (`node_id` is the bound `user_did`, `parent_id` the `context_id`).
pub struct ClaimMember {
    pub id: String,
    pub node_id: Option<String>,
    pub parent_id: Option<String>,
}

/// A member's context + secret claim token (for the owner claim-link flow).
pub struct MemberClaimInfo {
    pub parent_id: Option<String>,
    pub claim_token: Option<String>,
}

/// A stored Web Push subscription (the fields the push sender needs).
pub struct Subscription {
    pub endpoint: String,
    pub p256dh: String,
    pub auth: String,
}

/// What a path names: either spine of the tree.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
#[serde(tag = "node", rename_all = "snake_case")]
pub enum Node {
    Context(Context),
    Document(Document),
}

impl Node {
    pub fn id(&self) -> &str {
        match self {
            Node::Context(c) => &c.id,
            Node::Document(d) => &d.id,
        }
    }

    /// The context it is in. A context is in itself.
    pub fn context_id(&self) -> &str {
        match self {
            Node::Context(c) => &c.id,
            Node::Document(d) => &d.context_id,
        }
    }

    pub fn place(&self) -> &Place {
        match self {
            Node::Context(c) => &c.place,
            Node::Document(d) => &d.place,
        }
    }
}

/// A child as a folder view, the drawer and a breadcrumb need it: the same few
/// fields whichever table it lives in, and no content.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Child {
    /// `context` or `document`.
    pub node: &'static str,
    pub id: String,
    pub kind: String,
    pub name: String,
    pub slug: String,
    pub path: String,
    pub idx: i64,
    pub mutable: bool,
    pub attachable: bool,
    pub owner_did: Option<String>,
    pub created_at: Option<String>,
    /// A file's id and type, a cover image: what a row needs to draw itself.
    pub data: Option<serde_json::Value>,
    /// Live children, so the drawer offers to expand only what has some.
    pub child_count: i64,
}

/// One segment of the way down to a node.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct Crumb {
    pub slug: String,
    pub path: String,
    /// Absent where the caller may not read the node the segment names. They
    /// hold the slug already, in the URL, and are told nothing more.
    #[serde(flatten)]
    pub named: Option<CrumbName>,
}

#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct CrumbName {
    pub id: String,
    pub node: &'static str,
    pub kind: String,
    pub name: String,
}

/// A row of a context's member list.
#[derive(Debug, Clone, PartialEq, serde::Serialize)]
pub struct MemberRow {
    pub id: String,
    pub user_did: Option<String>,
    pub role: String,
    /// Voting rights.
    pub active: bool,
    pub accepted: bool,
    pub hidden: bool,
    /// The roster's name for them, the only label a pending invitation has.
    pub name: Option<String>,
    pub display_name: Option<String>,
    pub handle: Option<String>,
    pub avatar_url: Option<String>,
    /// Served to owners of the context and to nobody else: see
    /// [`MemberQuery::for_owner`].
    #[serde(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
}

/// Which members of a context to list.
#[derive(Debug, Default, Clone)]
pub struct MemberQuery {
    pub owner: Option<bool>,
    pub active: Option<bool>,
    pub accepted: Option<bool>,
    pub hidden: Option<bool>,
    /// Matched against the names. Against the email too, for an owner only:
    /// for anyone else a search that matched an address would be a way to ask
    /// whether it is on the roster.
    pub search: String,
    /// The caller owns the context, so they are served addresses and the rows
    /// that are hidden from everyone else. The interim could not draw this
    /// line (a column permission is per role, and an owner is role `user` too),
    /// so any member could read most of the organisation's addresses.
    pub for_owner: bool,
    pub limit: i64,
    pub offset: i64,
}

/// Someone to put on a context's roster. A `did` invites an account, which is
/// bound from the start and has to say yes; otherwise it is a roster row that
/// whoever holds its claim link binds.
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct Invite {
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub email: Option<String>,
    #[serde(default)]
    pub did: Option<String>,
}

/// What an import did. `skipped` counts people this context had already: a
/// roster says who belongs here, not that none of them are here yet, so they are
/// passed over rather than failing the import.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, serde::Serialize)]
pub struct InviteOutcome {
    pub inserted: usize,
    pub skipped: usize,
    /// Of those inserted, how many the roster gave no address for. On the list,
    /// but only reachable by handing them their claim link.
    pub without_email: usize,
}

/// What a change to a member may set. `None` leaves a field as it is.
#[derive(Debug, Default)]
pub struct MemberPatch<'a> {
    pub name: Option<&'a str>,
    pub email: Option<&'a str>,
    pub owner: Option<bool>,
    pub active: Option<bool>,
    pub hidden: Option<bool>,
}

/// What authorizing a change to a member row needs to know about it.
pub struct MemberMeta {
    pub context_id: String,
    pub user_did: Option<String>,
}

/// An invitation the caller has not answered.
#[derive(Debug, serde::Serialize)]
pub struct Invitation {
    pub id: String,
    pub context_id: String,
    pub context_kind: String,
    pub context_name: String,
    pub context_path: String,
}

fn normalized_email(email: Option<&str>) -> Option<String> {
    email
        .map(|e| e.trim().to_lowercase())
        .filter(|e| !e.is_empty())
}

/// A document to create. The store picks its slug and path.
pub struct NewDocument<'a> {
    pub context_id: &'a str,
    /// `None` hangs it directly off the context.
    pub parent_id: Option<&'a str>,
    pub kind: &'a str,
    pub title: &'a str,
    pub content: Option<&'a str>,
    pub data: Option<&'a str>,
    /// The creator, and the first author.
    pub author_did: &'a str,
}

/// What a change to a document may set. `None` leaves a field as it is. The
/// slug is not among them: a rename keeps the URL people have linked to.
#[derive(Debug, Default)]
pub struct DocumentPatch<'a> {
    pub title: Option<&'a str>,
    pub content: Option<&'a str>,
    pub data: Option<&'a str>,
    pub mutable: Option<bool>,
    pub attachable: Option<bool>,
    pub idx: Option<i64>,
}

/// What authorizing a change to a document needs to know about it.
pub struct DocumentMeta {
    pub context_id: String,
    pub owner_did: Option<String>,
    pub mutable: bool,
    pub path: String,
    pub parent_id: Option<String>,
    /// Whether it is in the bin.
    pub binned: bool,
}

/// A row of the bin: the root of a subtree that was deleted together.
#[derive(Debug, serde::Serialize)]
pub struct Binned {
    pub id: String,
    pub kind: String,
    pub title: String,
    pub path: String,
    pub owner_did: Option<String>,
    pub deleted_at: String,
}

/// Why a write was refused, as distinct from failing.
#[derive(Debug)]
pub enum WriteError {
    Db(DbError),
    /// The parent is missing, or in the bin.
    NoSuchParent,
    /// The parent is in another context. A member of one context could otherwise
    /// hang a document off another's tree.
    ParentElsewhere,
    /// A live node has taken the path a restore would put this one back at.
    PathTaken,
    /// The new parent is the node itself, or somewhere inside it.
    IntoItself,
    /// The change would leave a context with nobody who owns it, and then
    /// nobody could ever administer it again.
    LastOwner,
    /// Another invitation to this context already has that address.
    EmailTaken,
}

impl std::fmt::Display for WriteError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            WriteError::Db(e) => write!(f, "{e}"),
            WriteError::NoSuchParent => write!(f, "no such parent"),
            WriteError::ParentElsewhere => write!(f, "parent is not in that context"),
            WriteError::PathTaken => write!(f, "another node now has that path"),
            WriteError::IntoItself => write!(f, "a node cannot be moved into itself"),
            WriteError::LastOwner => write!(f, "a context must keep an owner"),
            WriteError::EmailTaken => write!(f, "that address is already invited here"),
        }
    }
}

impl std::error::Error for WriteError {}

impl From<DbError> for WriteError {
    fn from(e: DbError) -> Self {
        WriteError::Db(e)
    }
}

impl From<turso::Error> for WriteError {
    fn from(e: turso::Error) -> Self {
        WriteError::Db(e.into())
    }
}

/// A live node, as somewhere to hang a child or a comment.
pub struct Parent {
    pub path: String,
    /// The context it is in. A context is in itself.
    pub context_id: String,
    /// [`crate::authz::CONTEXT`] for any context, else the document's kind.
    pub kind: String,
    pub attachable: bool,
}

impl Store {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// The live context or document `id` names, as a place to hang a child.
    /// Ungated: it answers a write check, and says nothing to the caller.
    pub async fn parent_of(&self, id: &str) -> Result<Option<Parent>, DbError> {
        let conn = self.db.acquire().await?;
        self.parent(&conn, id).await
    }

    async fn parent(&self, conn: &turso::Connection, id: &str) -> Result<Option<Parent>, DbError> {
        let context = crate::authz::CONTEXT;
        for sql in [
            format!(
                "SELECT path, id, '{context}', attachable FROM context WHERE id = ?1 AND {LIVE}"
            ),
            format!(
                "SELECT path, context_id, kind, attachable FROM document WHERE id = ?1 AND {LIVE}"
            ),
        ] {
            let mut rows = conn.query(&sql, [id]).await?;
            if let Some(row) = rows.next().await? {
                return Ok(Some(Parent {
                    path: row.get::<String>(0)?,
                    context_id: row.get::<String>(1)?,
                    kind: row.get::<String>(2)?,
                    attachable: row.get::<i64>(3)? != 0,
                }));
            }
        }
        Ok(None)
    }

    /// Whether a live node of either kind already has this path. The unique
    /// indexes each cover one table, so the other is asked here.
    async fn path_taken(&self, conn: &turso::Connection, path: &str) -> Result<bool, DbError> {
        for table in ["context", "document"] {
            let mut rows = conn
                .query(
                    &format!("SELECT 1 FROM {table} WHERE path = ?1 AND {LIVE}"),
                    [path],
                )
                .await?;
            if rows.next().await?.is_some() {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Fetch a node's owner DID + context. Reads `document` first (owner = its
    /// first author with a DID), then `comment`; `None` if neither exists.
    pub async fn node_owner_and_context(
        &self,
        node_id: &str,
    ) -> Result<Option<NodeOwnerContext>, DbError> {
        let conn = self.db.acquire().await?;
        // A document: context from the row, owner from the author join.
        let mut rows = conn
            .query("SELECT context_id FROM document WHERE id = ?1", [node_id])
            .await?;
        if let Some(row) = rows.next().await? {
            let context_id = opt_text(&row, 0);
            let mut a = conn
                .query(
                    "SELECT author_did FROM document_author \
                     WHERE document_id = ?1 AND author_did IS NOT NULL \
                     ORDER BY ord LIMIT 1",
                    [node_id],
                )
                .await?;
            let owner_id = match a.next().await? {
                Some(ar) => opt_text(&ar, 0),
                None => None,
            };
            return Ok(Some(NodeOwnerContext {
                owner_id,
                context_id,
            }));
        }
        // A comment: author + context are both on the row.
        let mut rows = conn
            .query(
                "SELECT author_did, context_id FROM comment WHERE id = ?1",
                [node_id],
            )
            .await?;
        if let Some(row) = rows.next().await? {
            return Ok(Some(NodeOwnerContext {
                owner_id: opt_text(&row, 0),
                context_id: opt_text(&row, 1),
            }));
        }
        Ok(None)
    }

    /// The context and kind of a node (a document, or a comment as `"comment"`)
    /// that `caller` may read. `None` covers both "no such node" and "not theirs
    /// to read".
    pub async fn readable_subject(
        &self,
        node_id: &str,
        caller: Option<&str>,
    ) -> Result<Option<(String, String)>, DbError> {
        let conn = self.db.acquire().await?;
        let params = || vec![Value::Text(node_id.to_string()), opt_str_val(caller)];
        for sql in [
            format!(
                "SELECT d.context_id, d.kind FROM document d \
                 WHERE d.id = ?1 AND d.{LIVE} AND {}",
                readable_document("d", 2)
            ),
            format!(
                "SELECT k.context_id, 'comment' FROM comment k WHERE k.id = ?1 AND {}",
                readable_comment("k", 2)
            ),
        ] {
            let mut rows = conn.query(&sql, params()).await?;
            if let Some(row) = rows.next().await? {
                return Ok(Some((row.get::<String>(0)?, row.get::<String>(1)?)));
            }
        }
        Ok(None)
    }

    /// Look up the member a `?claim=<token>` link points at.
    pub async fn member_by_claim_token(
        &self,
        claim_token: &str,
    ) -> Result<Option<ClaimMember>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT id, user_did, context_id FROM member WHERE claim_token = ?1 LIMIT 1",
                [claim_token],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        let Some(id) = opt_text(&row, 0) else {
            return Ok(None);
        };
        Ok(Some(ClaimMember {
            id,
            node_id: opt_text(&row, 1),
            parent_id: opt_text(&row, 2),
        }))
    }

    /// Bind a pending member row to a user, guarded on `user_did` still NULL so a
    /// race cannot double-claim. Returns whether a row was actually bound. The
    /// `member_bound` partial unique additionally rejects binding a DID already
    /// active in the context (surfaces as a constraint error). Claiming an
    /// invitation is saying yes to it, so the row is accepted too.
    pub async fn bind_member_to_user(
        &self,
        member_id: &str,
        user_did: &str,
    ) -> Result<bool, DbError> {
        let conn = self.db.acquire().await?;
        let affected = conn
            .execute(
                "UPDATE member SET user_did = ?1, accepted = 1 \
                 WHERE id = ?2 AND user_did IS NULL",
                [user_did, member_id],
            )
            .await?;
        Ok(affected > 0)
    }

    /// Fetch a member's context id + claim token by member id.
    pub async fn member_claim_token(
        &self,
        member_id: &str,
    ) -> Result<Option<MemberClaimInfo>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT context_id, claim_token FROM member WHERE id = ?1 LIMIT 1",
                [member_id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(MemberClaimInfo {
            parent_id: opt_text(&row, 0),
            claim_token: opt_text(&row, 1),
        }))
    }

    /// A page of a context's members, by name with the nameless last, and how
    /// many match in all.
    pub async fn list_members(
        &self,
        context_id: &str,
        q: &MemberQuery,
    ) -> Result<(Vec<MemberRow>, i64), DbError> {
        let mut wheres = vec!["m.context_id = ?1".to_string()];
        let mut params = vec![Value::Text(context_id.to_string())];
        let flag = |b: bool| Value::Integer(i64::from(b));
        if let Some(owner) = q.owner {
            params.push(Value::Text(
                if owner { "owner" } else { "member" }.to_string(),
            ));
            wheres.push(format!("m.role = ?{}", params.len()));
        }
        for (column, wanted) in [
            ("active", q.active),
            ("accepted", q.accepted),
            ("hidden", q.hidden),
        ] {
            if let Some(wanted) = wanted {
                params.push(flag(wanted));
                wheres.push(format!("m.{column} = ?{}", params.len()));
            }
        }
        if !q.for_owner {
            wheres.push("m.hidden = 0".to_string());
        }
        let search = q.search.trim();
        if !search.is_empty() {
            params.push(Value::Text(format!("%{search}%")));
            let n = params.len();
            let email = if q.for_owner {
                format!(" OR m.email LIKE ?{n}")
            } else {
                String::new()
            };
            wheres.push(format!(
                "(m.name LIKE ?{n} OR u.display_name LIKE ?{n} OR u.handle LIKE ?{n}{email})"
            ));
        }
        let from = format!(
            "FROM member m LEFT JOIN user u ON u.did = m.user_did WHERE {}",
            wheres.join(" AND ")
        );

        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(&format!("SELECT count(*) {from}"), params.clone())
            .await?;
        let total = match rows.next().await? {
            Some(row) => row.get::<i64>(0)?,
            None => 0,
        };
        drop(rows);

        params.push(Value::Integer(q.limit));
        params.push(Value::Integer(q.offset));
        let (limit, offset) = (params.len() - 1, params.len());
        let mut rows = conn
            .query(
                &format!(
                    "SELECT m.id, m.user_did, m.role, m.active, m.accepted, m.hidden, m.name, \
                            u.display_name, u.handle, u.avatar_url, m.email \
                     {from} \
                     ORDER BY coalesce(m.name, u.display_name, u.handle) IS NULL, \
                              lower(coalesce(m.name, u.display_name, u.handle)), m.id \
                     LIMIT ?{limit} OFFSET ?{offset}"
                ),
                params,
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(MemberRow {
                id: row.get::<String>(0)?,
                user_did: opt_text(&row, 1),
                role: row.get::<String>(2)?,
                active: row.get::<i64>(3)? != 0,
                accepted: row.get::<i64>(4)? != 0,
                hidden: row.get::<i64>(5)? != 0,
                name: opt_text(&row, 6),
                display_name: opt_text(&row, 7),
                handle: opt_text(&row, 8),
                avatar_url: opt_text(&row, 9),
                email: opt_text(&row, 10).filter(|_| q.for_owner),
            });
        }
        Ok((out, total))
    }

    /// Put people on a context's roster. Anyone the context already has, by
    /// address or by account, is skipped, and so is an address the batch itself
    /// repeats. One write transaction, so two imports cannot interleave.
    pub async fn invite(
        &self,
        context_id: &str,
        invites: &[Invite],
    ) -> Result<InviteOutcome, DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let outcome = self.invite_in(&conn, context_id, invites).await;
        conn.execute(
            if outcome.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            },
            (),
        )
        .await?;
        outcome
    }

    async fn invite_in(
        &self,
        conn: &turso::Connection,
        context_id: &str,
        invites: &[Invite],
    ) -> Result<InviteOutcome, DbError> {
        let mut outcome = InviteOutcome::default();
        for invite in invites {
            let name = invite
                .name
                .as_deref()
                .map(str::trim)
                .filter(|n| !n.is_empty());
            let email = normalized_email(invite.email.as_deref());
            let did = invite
                .did
                .as_deref()
                .map(str::trim)
                .filter(|d| !d.is_empty());
            if name.is_none() && email.is_none() && did.is_none() {
                continue;
            }
            // Only an address or an account can say "the same person". Two rows
            // sharing a name and nothing else are two people.
            let had = match (did, email.as_deref()) {
                (Some(did), _) => {
                    let mut rows = conn
                        .query(
                            "SELECT 1 FROM member WHERE context_id = ?1 AND user_did = ?2",
                            [context_id, did],
                        )
                        .await?;
                    rows.next().await?.is_some()
                }
                (None, Some(email)) => {
                    let mut rows = conn
                        .query(
                            "SELECT 1 FROM member WHERE context_id = ?1 AND email = ?2",
                            [context_id, email],
                        )
                        .await?;
                    rows.next().await?.is_some()
                }
                (None, None) => false,
            };
            if had {
                outcome.skipped += 1;
                continue;
            }
            if let Some(did) = did {
                conn.execute("INSERT OR IGNORE INTO user (did) VALUES (?1)", [did])
                    .await?;
            }
            conn.execute(
                "INSERT INTO member (id, context_id, user_did, name, email, claim_token) \
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                vec![
                    Value::Text(format!("m-{}", crate::util::random_token(16))),
                    Value::Text(context_id.to_string()),
                    opt_str_val(did),
                    opt_str_val(name),
                    opt_str_val(email.as_deref()),
                    // An account is bound already, so it has nothing to claim.
                    match did {
                        Some(_) => Value::Null,
                        None => Value::Text(crate::util::random_token(24)),
                    },
                ],
            )
            .await?;
            outcome.inserted += 1;
            if did.is_none() && email.is_none() {
                outcome.without_email += 1;
            }
        }
        Ok(outcome)
    }

    /// A member row's authorization facts.
    pub async fn member_meta(&self, id: &str) -> Result<Option<MemberMeta>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT context_id, user_did FROM member WHERE id = ?1",
                [id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(MemberMeta {
            context_id: row.get::<String>(0)?,
            user_did: opt_text(&row, 1),
        }))
    }

    /// Whether taking the owner role from member `id` would leave its context
    /// with no owner who can sign in. An unclaimed invitation cannot administer
    /// anything, so it does not count.
    async fn is_last_owner(&self, conn: &turso::Connection, id: &str) -> Result<bool, DbError> {
        let mut rows = conn
            .query(
                "SELECT m.role = 'owner' AND m.user_did IS NOT NULL AND NOT EXISTS ( \
                   SELECT 1 FROM member o WHERE o.context_id = m.context_id AND o.id <> m.id \
                     AND o.role = 'owner' AND o.user_did IS NOT NULL) \
                 FROM member m WHERE m.id = ?1",
                [id],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<i64>(0)? != 0),
            None => Ok(false),
        }
    }

    /// Apply `patch` to a member row. Returns whether there was one.
    pub async fn update_member(
        &self,
        id: &str,
        patch: &MemberPatch<'_>,
    ) -> Result<bool, WriteError> {
        let conn = self.db.acquire().await?;
        if patch.owner == Some(false) && self.is_last_owner(&conn, id).await? {
            return Err(WriteError::LastOwner);
        }
        let mut sets = Vec::new();
        let mut params = Vec::new();
        let mut set = |column: &'static str, value: Value| {
            params.push(value);
            sets.push(format!("{column} = ?{}", params.len()));
        };
        let flag = |b: bool| Value::Integer(i64::from(b));
        if let Some(name) = patch.name {
            set(
                "name",
                opt_str_val(Some(name.trim()).filter(|n| !n.is_empty())),
            );
        }
        if let Some(email) = patch.email {
            set(
                "email",
                opt_str_val(normalized_email(Some(email)).as_deref()),
            );
        }
        if let Some(owner) = patch.owner {
            set(
                "role",
                Value::Text(if owner { "owner" } else { "member" }.to_string()),
            );
        }
        if let Some(active) = patch.active {
            set("active", flag(active));
        }
        if let Some(hidden) = patch.hidden {
            set("hidden", flag(hidden));
        }
        if sets.is_empty() {
            return Ok(self.member_meta(id).await?.is_some());
        }
        params.push(Value::Text(id.to_string()));
        let changed = conn
            .execute(
                &format!(
                    "UPDATE member SET {} WHERE id = ?{}",
                    sets.join(", "),
                    params.len()
                ),
                params,
            )
            .await;
        match changed {
            Ok(n) => Ok(n > 0),
            // `member_pending`: one unclaimed invitation per address per context.
            Err(turso::Error::Constraint(_)) => Err(WriteError::EmailTaken),
            Err(e) => Err(e.into()),
        }
    }

    /// Take someone off a roster: a removal, a leaving, or a declined invitation.
    pub async fn remove_member(&self, id: &str) -> Result<bool, WriteError> {
        let conn = self.db.acquire().await?;
        if self.is_last_owner(&conn, id).await? {
            return Err(WriteError::LastOwner);
        }
        let removed = conn
            .execute("DELETE FROM member WHERE id = ?1", [id])
            .await?;
        Ok(removed > 0)
    }

    /// The invitations `did` has not answered, newest first.
    pub async fn list_invitations(&self, did: &str) -> Result<Vec<Invitation>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT m.id, c.id, c.kind, c.name, c.path \
                     FROM member m JOIN context c ON c.id = m.context_id \
                     WHERE m.user_did = ?1 AND m.accepted = 0 AND c.{LIVE} \
                     ORDER BY m.created_at DESC, m.id"
                ),
                [did],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(Invitation {
                id: row.get::<String>(0)?,
                context_id: row.get::<String>(1)?,
                context_kind: row.get::<String>(2)?,
                context_name: row.get::<String>(3)?,
                context_path: row.get::<String>(4)?,
            });
        }
        Ok(out)
    }

    /// Say yes to an invitation. The row is named by its id AND its account, so
    /// nobody accepts on another's behalf, and accepting touches that row alone:
    /// the interim's accept once asked "is there a membership here?" with a
    /// question the invitation itself answered, and deleted it as a duplicate.
    pub async fn accept_invitation(&self, id: &str, did: &str) -> Result<bool, DbError> {
        let conn = self.db.acquire().await?;
        let accepted = conn
            .execute(
                "UPDATE member SET accepted = 1 WHERE id = ?1 AND user_did = ?2",
                [id, did],
            )
            .await?;
        Ok(accepted > 0)
    }

    /// How many members of a context hold voting rights: a poll's turnout is
    /// out of this.
    pub async fn count_voters(&self, context_id: &str) -> Result<i64, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT count(*) FROM member WHERE context_id = ?1 AND active = 1",
                [context_id],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(row.get::<i64>(0)?),
            None => Ok(0),
        }
    }

    /// The emails of a context's active members (push fan-out targets). The
    /// interim `accepted` flag has no target column (it folded into the active/
    /// bind state), so active membership is `active = 1`.
    pub async fn active_member_emails(&self, context: &str) -> Result<Vec<String>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT email FROM member \
                 WHERE context_id = ?1 AND active = 1 AND email IS NOT NULL",
                [context],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            if let Some(email) = opt_text(&row, 0) {
                out.push(email);
            }
        }
        Ok(out)
    }

    /// Upsert a device's Web Push subscription by endpoint (a device
    /// re-subscribing keeps one row with fresh keys).
    pub async fn upsert_push_subscription(
        &self,
        user_did: &str,
        email: &str,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT INTO push_subscription (endpoint, user_did, email, p256dh, auth) \
             VALUES (?1, ?2, ?3, ?4, ?5) \
             ON CONFLICT(endpoint) DO UPDATE SET user_did = excluded.user_did, \
               email = excluded.email, p256dh = excluded.p256dh, auth = excluded.auth",
            [endpoint, user_did, email, p256dh, auth],
        )
        .await?;
        Ok(())
    }

    /// Delete push subscriptions by endpoint (unsubscribe, or pruning gone ones).
    pub async fn delete_subscriptions_by_endpoint(
        &self,
        endpoints: &[String],
    ) -> Result<(), DbError> {
        if endpoints.is_empty() {
            return Ok(());
        }
        let conn = self.db.acquire().await?;
        let placeholders = in_placeholders(endpoints.len());
        conn.execute(
            &format!("DELETE FROM push_subscription WHERE endpoint IN ({placeholders})"),
            turso::params_from_iter(endpoints.to_vec()),
        )
        .await?;
        Ok(())
    }

    /// The stored Web Push subscriptions for a set of member emails.
    pub async fn subscriptions_for_emails(
        &self,
        emails: &[String],
    ) -> Result<Vec<Subscription>, DbError> {
        if emails.is_empty() {
            return Ok(Vec::new());
        }
        let conn = self.db.acquire().await?;
        let placeholders = in_placeholders(emails.len());
        let mut rows = conn
            .query(
                &format!(
                    "SELECT endpoint, p256dh, auth FROM push_subscription \
                     WHERE email IN ({placeholders})"
                ),
                turso::params_from_iter(emails.to_vec()),
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let (Some(endpoint), Some(p256dh), Some(auth)) =
                (opt_text(&row, 0), opt_text(&row, 1), opt_text(&row, 2))
            else {
                continue;
            };
            out.push(Subscription {
                endpoint,
                p256dh,
                auth,
            });
        }
        Ok(out)
    }

    // -- Firehose materialization (public records mirrored into the view) --

    /// A user row for `did` if there is none yet, so the foreign keys that point
    /// at a person have something to point at. Leaves an existing profile alone.
    pub async fn upsert_user_min(&self, did: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("INSERT OR IGNORE INTO user (did) VALUES (?1)", [did])
            .await?;
        Ok(())
    }

    /// Materialize a public `com.example.wiki.post` record into the `post` view,
    /// keyed by its at-uri (also its `published_uri` and `legacy_id`). `group` and
    /// `reply` are at-uris and foreign keys, so the caller must have checked that
    /// both are in the view.
    pub async fn upsert_public_post(
        &self,
        uri: &str,
        author_did: &str,
        text: &str,
        group: Option<&str>,
        reply: Option<&str>,
        created_at: &str,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT INTO post \
             (id, author_did, text, group_id, reply_to, visibility, published_uri, created_at, legacy_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, 'public', ?1, ?6, ?1) \
             ON CONFLICT(id) DO UPDATE SET author_did = excluded.author_did, \
               text = excluded.text, group_id = excluded.group_id, \
               reply_to = excluded.reply_to, created_at = excluded.created_at",
            vec![
                Value::Text(uri.to_string()),
                Value::Text(author_did.to_string()),
                Value::Text(text.to_string()),
                opt_str_val(group),
                opt_str_val(reply),
                Value::Text(created_at.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    /// Delete a public post from the view by its at-uri (a firehose delete op).
    pub async fn delete_public_post(&self, uri: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("DELETE FROM post WHERE id = ?1", [uri])
            .await?;
        Ok(())
    }

    /// Materialize a public `com.example.wiki.reaction` record into the `reaction`
    /// view, keyed by its at-uri. Idempotent: updates the row if the at-uri is
    /// already present, and skips if the same `(subject, reactor, emoji)` triple
    /// already exists under a different record (a redundant double-react).
    pub async fn upsert_reaction(
        &self,
        uri: &str,
        subject_uri: &str,
        reactor_did: &str,
        emoji: &str,
        created_at: &str,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        let updated = conn
            .execute(
                "UPDATE reaction SET subject_uri = ?1, reactor_did = ?2, emoji = ?3, created_at = ?4 \
                 WHERE id = ?5",
                [subject_uri, reactor_did, emoji, created_at, uri],
            )
            .await?;
        if updated > 0 {
            return Ok(());
        }
        // The unique (subject, reactor, emoji) already reacted (a different
        // record with the same triple): idempotent no-op.
        let mut rows = conn
            .query(
                "SELECT 1 FROM reaction WHERE subject_uri = ?1 AND reactor_did = ?2 AND emoji = ?3 LIMIT 1",
                [subject_uri, reactor_did, emoji],
            )
            .await?;
        if rows.next().await?.is_some() {
            return Ok(());
        }
        conn.execute(
            "INSERT INTO reaction (id, subject_uri, reactor_did, emoji, created_at, legacy_id) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?1)",
            [uri, subject_uri, reactor_did, emoji, created_at],
        )
        .await?;
        Ok(())
    }

    /// Delete a reaction from the view by its at-uri (a firehose delete op).
    pub async fn delete_reaction(&self, uri: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("DELETE FROM reaction WHERE id = ?1", [uri])
            .await?;
        Ok(())
    }

    // -- Read side of the native serving layer (returns the canonical domain
    //    types, which the XRPC handlers serve as JSON). Identity-free reads. --

    /// The authors of a document (from the `document_author` join, in `ord`),
    /// each a DID (an account) or a free-text display name.
    async fn document_authors(&self, document_id: &str) -> Result<Vec<Author>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT author_did, author_text FROM document_author \
                 WHERE document_id = ?1 ORDER BY ord",
                [document_id],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            if let Some(did) = opt_text(&row, 0) {
                out.push(Author::User { did });
            } else if let Some(text) = opt_text(&row, 1) {
                out.push(Author::FreeText { display: text });
            }
        }
        Ok(out)
    }

    /// A content node (document / folder / file / proposal) by id, with its
    /// authors. `None` if no such document.
    /// Hydrate a `DocBase` into a full `Document` (attaches its authors).
    async fn hydrate_document(&self, b: DocBase) -> Result<Document, DbError> {
        let authors = self.document_authors(&b.id).await?;
        Ok(Document {
            id: b.id,
            context_id: b.context_id,
            kind: parse_enum(&b.kind).unwrap_or(DocumentKind::Document),
            title: b.title,
            place: b.place,
            mutable: b.mutable,
            content: b.content.and_then(|s| serde_json::from_str(&s).ok()),
            data: b.data.and_then(|s| serde_json::from_str(&s).ok()),
            authors,
            visibility: b
                .visibility
                .and_then(|s| parse_enum(&s))
                .unwrap_or_default(),
            published_uri: b.published_uri,
            legacy_id: None,
        })
    }

    /// A content node (document / folder / file / proposal) by id, with its
    /// authors. `None` if there is no such document, or none `caller` may read:
    /// the two are not told apart, so a private document's existence is not
    /// revealed.
    pub async fn read_document(
        &self,
        id: &str,
        caller: Option<&str>,
    ) -> Result<Option<Document>, DbError> {
        let base = {
            let conn = self.db.acquire().await?;
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {DOC_COLS} FROM document d WHERE d.id = ?1 AND d.{LIVE} AND {}",
                        readable_document("d", 2)
                    ),
                    vec![Value::Text(id.to_string()), opt_str_val(caller)],
                )
                .await?;
            match rows.next().await? {
                Some(row) => doc_base(&row)?,
                None => return Ok(None),
            }
        };
        Ok(Some(self.hydrate_document(base).await?))
    }

    pub async fn read_user(&self, did: &str) -> Result<Option<User>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT did, handle, display_name, avatar_url FROM user WHERE did = ?1",
                [did],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(User {
            did: row.get::<String>(0)?,
            handle: opt_text(&row, 1),
            display_name: opt_text(&row, 2),
            avatar_url: opt_text(&row, 3),
            legacy_id: None,
        }))
    }

    /// A context (group / event) by id. `None` if there is none `caller` may read.
    pub async fn read_context(
        &self,
        id: &str,
        caller: Option<&str>,
    ) -> Result<Option<Context>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {CTX_COLS} FROM context c WHERE c.id = ?1 AND c.{LIVE} AND {}",
                    readable_context("c", 2)
                ),
                vec![Value::Text(id.to_string()), opt_str_val(caller)],
            )
            .await?;
        match rows.next().await? {
            Some(row) => Ok(Some(ctx_from_row(&row)?)),
            None => Ok(None),
        }
    }

    /// The live node at `path` that `caller` may read. One lookup at any depth,
    /// because the path is stored; it also means nothing above the node has to be
    /// readable, so a member of an event reaches it through a group they do not
    /// belong to.
    pub async fn resolve_path(
        &self,
        path: &str,
        caller: Option<&str>,
    ) -> Result<Option<Node>, DbError> {
        let found = {
            let conn = self.db.acquire().await?;
            let params = || vec![Value::Text(path.to_string()), opt_str_val(caller)];
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {CTX_COLS} FROM context c \
                         WHERE c.path = ?1 AND c.{LIVE} AND {}",
                        readable_context("c", 2)
                    ),
                    params(),
                )
                .await?;
            if let Some(row) = rows.next().await? {
                return Ok(Some(Node::Context(ctx_from_row(&row)?)));
            }
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {DOC_COLS} FROM document d \
                         WHERE d.path = ?1 AND d.{LIVE} AND {}",
                        readable_document("d", 2)
                    ),
                    params(),
                )
                .await?;
            match rows.next().await? {
                Some(row) => doc_base(&row)?,
                None => return Ok(None),
            }
        };
        Ok(Some(Node::Document(self.hydrate_document(found).await?)))
    }

    /// Everything directly under `parent_id` that `caller` may read, of either
    /// kind, in the manual order and then by age.
    pub async fn children(
        &self,
        parent_id: &str,
        caller: Option<&str>,
    ) -> Result<Vec<Child>, DbError> {
        // A context is locked from the day it is made, as in the interim.
        let counted = |alias: &str| {
            format!(
                "(SELECT count(*) FROM document x WHERE x.parent_id = {alias}.id AND x.{LIVE}) + \
                 (SELECT count(*) FROM context y WHERE y.parent_id = {alias}.id AND y.{LIVE})"
            )
        };
        let queries = [
            (
                "context",
                format!(
                    "SELECT c.id, c.kind, c.name, c.slug, c.path, c.idx, 0, c.attachable, \
                            c.owner_did, c.created_at, NULL, {} \
                     FROM context c WHERE c.parent_id = ?1 AND c.{LIVE} AND {}",
                    counted("c"),
                    readable_context("c", 2)
                ),
            ),
            (
                "document",
                format!(
                    "SELECT d.id, d.kind, d.title, d.slug, d.path, d.idx, d.mutable, d.attachable, \
                            d.owner_did, d.created_at, d.data, {} \
                     FROM document d WHERE d.parent_id = ?1 AND d.{LIVE} AND {}",
                    counted("d"),
                    readable_document("d", 2)
                ),
            ),
        ];
        let conn = self.db.acquire().await?;
        let mut out = Vec::new();
        for (node, sql) in queries {
            let mut rows = conn
                .query(
                    &sql,
                    vec![Value::Text(parent_id.to_string()), opt_str_val(caller)],
                )
                .await?;
            while let Some(row) = rows.next().await? {
                out.push(Child {
                    node,
                    id: row.get::<String>(0)?,
                    kind: row.get::<String>(1)?,
                    name: row.get::<String>(2)?,
                    slug: row.get::<String>(3)?,
                    path: row.get::<String>(4)?,
                    idx: row.get::<i64>(5)?,
                    mutable: row.get::<i64>(6)? != 0,
                    attachable: row.get::<i64>(7)? != 0,
                    owner_did: opt_text(&row, 8),
                    created_at: opt_text(&row, 9),
                    data: opt_text(&row, 10).and_then(|s| serde_json::from_str(&s).ok()),
                    child_count: row.get::<i64>(11)?,
                });
            }
        }
        out.sort_by(|a, b| (a.idx, &a.created_at, &a.id).cmp(&(b.idx, &b.created_at, &b.id)));
        Ok(out)
    }

    /// The way down to `path`, a crumb per segment. A segment the caller may not
    /// read keeps its slug and loses its name.
    pub async fn crumbs(&self, path: &str, caller: Option<&str>) -> Result<Vec<Crumb>, DbError> {
        let conn = self.db.acquire().await?;
        let mut out = Vec::new();
        let mut prefix = String::new();
        for slug in path.split('/').filter(|s| !s.is_empty()) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(slug);
            let params = || vec![Value::Text(prefix.clone()), opt_str_val(caller)];
            let mut named = None;
            for (node, sql) in [
                (
                    "context",
                    format!(
                        "SELECT c.id, c.kind, c.name FROM context c \
                         WHERE c.path = ?1 AND c.{LIVE} AND {}",
                        readable_context("c", 2)
                    ),
                ),
                (
                    "document",
                    format!(
                        "SELECT d.id, d.kind, d.title FROM document d \
                         WHERE d.path = ?1 AND d.{LIVE} AND {}",
                        readable_document("d", 2)
                    ),
                ),
            ] {
                let mut rows = conn.query(&sql, params()).await?;
                if let Some(row) = rows.next().await? {
                    named = Some(CrumbName {
                        id: row.get::<String>(0)?,
                        node,
                        kind: row.get::<String>(1)?,
                        name: row.get::<String>(2)?,
                    });
                    break;
                }
            }
            out.push(Crumb {
                slug: slug.to_string(),
                path: prefix.clone(),
                named,
            });
        }
        Ok(out)
    }

    /// The live node `id` names that `caller` may read, of either kind.
    pub async fn read_node(&self, id: &str, caller: Option<&str>) -> Result<Option<Node>, DbError> {
        if let Some(ctx) = self.read_context(id, caller).await? {
            return Ok(Some(Node::Context(ctx)));
        }
        Ok(self.read_document(id, caller).await?.map(Node::Document))
    }

    /// The child content nodes directly under `parent_id` (a context or folder)
    /// that `caller` may read, oldest first, each with its authors.
    pub async fn list_children(
        &self,
        parent_id: &str,
        caller: Option<&str>,
    ) -> Result<Vec<Document>, DbError> {
        let bases = {
            let conn = self.db.acquire().await?;
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {DOC_COLS} FROM document d \
                         WHERE d.parent_id = ?1 AND d.{LIVE} AND {} \
                         ORDER BY d.idx, d.created_at",
                        readable_document("d", 2)
                    ),
                    vec![Value::Text(parent_id.to_string()), opt_str_val(caller)],
                )
                .await?;
            let mut v = Vec::new();
            while let Some(row) = rows.next().await? {
                v.push(doc_base(&row)?);
            }
            v
        };
        let mut out = Vec::with_capacity(bases.len());
        for b in bases {
            out.push(self.hydrate_document(b).await?);
        }
        Ok(out)
    }

    /// The top-level contexts (groups/events with no parent) `caller` may read,
    /// by name.
    pub async fn list_root_contexts(&self, caller: Option<&str>) -> Result<Vec<Context>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT {CTX_COLS} FROM context c \
                     WHERE c.parent_id IS NULL AND c.{LIVE} AND {} ORDER BY c.name",
                    readable_context("c", 1)
                ),
                vec![opt_str_val(caller)],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(ctx_from_row(&row)?);
        }
        Ok(out)
    }

    /// Documents `caller` may read whose title or content matches `query` (a
    /// case-insensitive substring), most recent first, capped.
    pub async fn search_documents(
        &self,
        query: &str,
        caller: Option<&str>,
    ) -> Result<Vec<Document>, DbError> {
        let like = format!("%{query}%");
        let bases = {
            let conn = self.db.acquire().await?;
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {DOC_COLS} FROM document d \
                         WHERE (d.title LIKE ?1 OR d.content LIKE ?1) AND d.{LIVE} AND {} \
                         ORDER BY d.created_at DESC LIMIT 50",
                        readable_document("d", 2)
                    ),
                    vec![Value::Text(like), opt_str_val(caller)],
                )
                .await?;
            let mut v = Vec::new();
            while let Some(row) = rows.next().await? {
                v.push(doc_base(&row)?);
            }
            v
        };
        let mut out = Vec::with_capacity(bases.len());
        for b in bases {
            out.push(self.hydrate_document(b).await?);
        }
        Ok(out)
    }

    /// The most recently created documents `caller` may read, across all
    /// contexts (the "newest" feed).
    pub async fn list_recent(
        &self,
        limit: i64,
        caller: Option<&str>,
    ) -> Result<Vec<Document>, DbError> {
        let bases = {
            let conn = self.db.acquire().await?;
            let mut rows = conn
                .query(
                    &format!(
                        "SELECT {DOC_COLS} FROM document d WHERE d.{LIVE} AND {} \
                         ORDER BY d.created_at DESC LIMIT ?2",
                        readable_document("d", 1)
                    ),
                    vec![opt_str_val(caller), Value::Integer(limit)],
                )
                .await?;
            let mut v = Vec::new();
            while let Some(row) = rows.next().await? {
                v.push(doc_base(&row)?);
            }
            v
        };
        let mut out = Vec::with_capacity(bases.len());
        for b in bases {
            out.push(self.hydrate_document(b).await?);
        }
        Ok(out)
    }

    /// The comments on a node (those whose `on_id` is the node) that `caller`
    /// may read, oldest first. Each carries a DID or free-text author.
    pub async fn get_comments(
        &self,
        on_id: &str,
        caller: Option<&str>,
    ) -> Result<Vec<Comment>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                &format!(
                    "SELECT k.id, k.on_id, k.context_id, k.author_did, k.author_text, k.text, \
                            k.created_at \
                     FROM comment k WHERE k.on_id = ?1 AND {} ORDER BY k.created_at",
                    readable_comment("k", 2)
                ),
                vec![Value::Text(on_id.to_string()), opt_str_val(caller)],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            let author = match (opt_text(&row, 3), opt_text(&row, 4)) {
                (Some(did), _) => Author::User { did },
                (None, Some(display)) => Author::FreeText { display },
                // The CHECK guarantees one is present; skip a malformed row.
                (None, None) => continue,
            };
            out.push(Comment {
                id: row.get::<String>(0)?,
                on_id: row.get::<String>(1)?,
                context_id: row.get::<String>(2)?,
                author,
                text: row.get::<String>(5)?,
                created_at: opt_text(&row, 6),
                legacy_id: None,
            });
        }
        Ok(out)
    }

    /// The reactions on a subject (by its at-uri), oldest first.
    pub async fn get_reactions(&self, subject_uri: &str) -> Result<Vec<Reaction>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT id, subject_uri, reactor_did, emoji, created_at \
                 FROM reaction WHERE subject_uri = ?1 ORDER BY created_at",
                [subject_uri],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(Reaction {
                id: row.get::<String>(0)?,
                subject_uri: row.get::<String>(1)?,
                reactor_did: opt_text(&row, 2),
                emoji: row.get::<String>(3)?,
                created_at: opt_text(&row, 4),
                legacy_id: None,
            });
        }
        Ok(out)
    }

    // -- Write side (Phase 1): a freshly-authenticated DID authors its own
    //    content. Membership/authz gating (is_active_member) is deferred with the
    //    DID-binding flow; these inserts are unconditional given a caller DID. --

    /// Create a document, giving it the cleanest slug that is free under its
    /// parent. Returns the new document's id.
    ///
    /// The slug is found inside one write transaction, so two members naming a
    /// document the same thing at once get `name` and `name-2`, not an error.
    pub async fn create_document(&self, new: &NewDocument<'_>) -> Result<String, WriteError> {
        let id = format!("d-{}", crate::util::random_token(16));
        let parent_id = new.parent_id.unwrap_or(new.context_id);
        let conn = self.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let written = self.insert_document(&conn, &id, parent_id, new).await;
        conn.execute(
            if written.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            },
            (),
        )
        .await?;
        written.map(|()| id)
    }

    async fn insert_document(
        &self,
        conn: &turso::Connection,
        id: &str,
        parent_id: &str,
        new: &NewDocument<'_>,
    ) -> Result<(), WriteError> {
        let parent = self
            .parent(conn, parent_id)
            .await?
            .ok_or(WriteError::NoSuchParent)?;
        if parent.context_id != new.context_id {
            return Err(WriteError::ParentElsewhere);
        }
        let mut slug = String::new();
        let mut path = String::new();
        for candidate in crate::slug::candidates(new.title) {
            path = format!("{}/{candidate}", parent.path);
            slug = candidate;
            if !self.path_taken(conn, &path).await? {
                break;
            }
        }
        conn.execute(
            "INSERT INTO document \
             (id, context_id, parent_id, kind, title, slug, path, owner_did, content, data) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            vec![
                Value::Text(id.to_string()),
                Value::Text(new.context_id.to_string()),
                Value::Text(parent_id.to_string()),
                Value::Text(new.kind.to_string()),
                Value::Text(new.title.to_string()),
                Value::Text(slug),
                Value::Text(path),
                Value::Text(new.author_did.to_string()),
                opt_str_val(new.content),
                opt_str_val(new.data),
            ],
        )
        .await?;
        conn.execute(
            "INSERT INTO document_author (document_id, author_did, ord) VALUES (?1, ?2, 0)",
            [id, new.author_did],
        )
        .await?;
        Ok(())
    }

    /// Move a live document, and everything under it, to a new parent in the
    /// same context. Returns its new path.
    ///
    /// It keeps its slug if that is free there and takes the next one if not.
    /// Every path in the subtree is rewritten, the binned ones too: a node that
    /// is restored later must come back under where its parent now is.
    pub async fn move_document(&self, id: &str, new_parent_id: &str) -> Result<String, WriteError> {
        let conn = self.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let moved = self.move_in(&conn, id, new_parent_id).await;
        conn.execute(if moved.is_ok() { "COMMIT" } else { "ROLLBACK" }, ())
            .await?;
        moved
    }

    async fn move_in(
        &self,
        conn: &turso::Connection,
        id: &str,
        new_parent_id: &str,
    ) -> Result<String, WriteError> {
        let mut rows = conn
            .query(
                &format!("SELECT path, slug, context_id FROM document WHERE id = ?1 AND {LIVE}"),
                [id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Err(WriteError::NoSuchParent);
        };
        let (old_path, slug, context_id) = (
            row.get::<String>(0)?,
            row.get::<String>(1)?,
            row.get::<String>(2)?,
        );
        drop(rows);
        let parent = self
            .parent(conn, new_parent_id)
            .await?
            .ok_or(WriteError::NoSuchParent)?;
        if parent.context_id != context_id {
            return Err(WriteError::ParentElsewhere);
        }
        if parent.path == old_path || parent.path.starts_with(&format!("{old_path}/")) {
            return Err(WriteError::IntoItself);
        }

        let mut new_slug = slug.clone();
        let mut new_path = format!("{}/{new_slug}", parent.path);
        for candidate in std::iter::once(slug.clone()).chain(crate::slug::candidates(&slug).skip(1))
        {
            new_path = format!("{}/{candidate}", parent.path);
            new_slug = candidate;
            if new_path == old_path || !self.path_taken(conn, &new_path).await? {
                break;
            }
        }

        // Descendants first, while the old prefix still identifies them.
        for table in ["document", "context"] {
            conn.execute(
                &format!(
                    "UPDATE {table} SET path = ?1 || substr(path, length(?2) + 1), \
                       updated_at = datetime('now') \
                     WHERE substr(path, 1, length(?2) + 1) = ?2 || '/'"
                ),
                [new_path.as_str(), old_path.as_str()],
            )
            .await?;
        }
        conn.execute(
            "UPDATE document SET parent_id = ?1, slug = ?2, path = ?3, \
               updated_at = datetime('now') WHERE id = ?4",
            [new_parent_id, new_slug.as_str(), new_path.as_str(), id],
        )
        .await?;
        Ok(new_path)
    }

    /// A document's authorization facts, whether or not it is in the bin.
    /// Ungated: it answers a write check, and says nothing to the caller.
    pub async fn document_meta(&self, id: &str) -> Result<Option<DocumentMeta>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT context_id, owner_did, mutable, path, parent_id, deleted_at IS NOT NULL \
                 FROM document WHERE id = ?1",
                [id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(DocumentMeta {
            context_id: row.get::<String>(0)?,
            owner_did: opt_text(&row, 1),
            mutable: row.get::<i64>(2)? != 0,
            path: row.get::<String>(3)?,
            parent_id: opt_text(&row, 4),
            binned: row.get::<i64>(5)? != 0,
        }))
    }

    /// The bin of a context: each document that was deleted, but not the ones
    /// that only went along with a parent, which come back with it. `owner`
    /// narrows it to what one person created.
    pub async fn list_binned(
        &self,
        context_id: &str,
        owner: Option<&str>,
    ) -> Result<Vec<Binned>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT d.id, d.kind, d.title, d.path, d.owner_did, d.deleted_at \
                 FROM document d \
                 WHERE d.context_id = ?1 AND d.deleted_at IS NOT NULL \
                   AND (?2 IS NULL OR d.owner_did = ?2) \
                   AND d.deleted_root = d.id \
                 ORDER BY d.deleted_at DESC",
                vec![Value::Text(context_id.to_string()), opt_str_val(owner)],
            )
            .await?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().await? {
            out.push(Binned {
                id: row.get::<String>(0)?,
                kind: row.get::<String>(1)?,
                title: row.get::<String>(2)?,
                path: row.get::<String>(3)?,
                owner_did: opt_text(&row, 4),
                deleted_at: row.get::<String>(5)?,
            });
        }
        Ok(out)
    }

    /// Apply `patch` to a live document. Returns whether there was one.
    pub async fn update_document(
        &self,
        id: &str,
        patch: &DocumentPatch<'_>,
    ) -> Result<bool, DbError> {
        let mut sets = Vec::new();
        let mut params = Vec::new();
        let mut set = |column: &'static str, value: Value| {
            params.push(value);
            sets.push(format!("{column} = ?{}", params.len()));
        };
        let flag = |b: bool| Value::Integer(i64::from(b));
        if let Some(title) = patch.title {
            set("title", Value::Text(title.to_string()));
        }
        if let Some(content) = patch.content {
            set("content", Value::Text(content.to_string()));
        }
        if let Some(data) = patch.data {
            set("data", Value::Text(data.to_string()));
        }
        if let Some(mutable) = patch.mutable {
            set("mutable", flag(mutable));
        }
        if let Some(attachable) = patch.attachable {
            set("attachable", flag(attachable));
        }
        if let Some(idx) = patch.idx {
            set("idx", Value::Integer(idx));
        }
        params.push(Value::Text(id.to_string()));
        let conn = self.db.acquire().await?;
        let changed = conn
            .execute(
                &format!(
                    "UPDATE document SET {}updated_at = datetime('now') \
                     WHERE id = ?{} AND {LIVE}",
                    sets.iter().map(|s| format!("{s}, ")).collect::<String>(),
                    params.len()
                ),
                params,
            )
            .await?;
        Ok(changed > 0)
    }

    /// Put the node `id` at `path`, and everything under it, in the bin, contexts
    /// included. Each row is marked with `id` as the root it went with, so
    /// [`Store::restore_subtree`] brings back exactly those and nothing that was
    /// deleted from inside the subtree before.
    pub async fn bin_subtree(&self, id: &str, path: &str) -> Result<u64, DbError> {
        let conn = self.db.acquire().await?;
        let mut binned = 0;
        for table in ["document", "context"] {
            binned += conn
                .execute(
                    &format!(
                        "UPDATE {table} SET deleted_at = datetime('now'), deleted_root = ?1, \
                           updated_at = datetime('now') \
                         WHERE {LIVE} AND {SUBTREE}"
                    ),
                    [id, path],
                )
                .await?;
        }
        Ok(binned)
    }

    /// Bring back what went to the bin with the node `id`, whose path is `path`.
    /// Refused if a live node has since taken that path; a path deeper in the
    /// subtree cannot be taken while its root's is free.
    pub async fn restore_subtree(&self, id: &str, path: &str) -> Result<u64, WriteError> {
        let conn = self.db.acquire().await?;
        conn.execute("BEGIN IMMEDIATE", ()).await?;
        let restored = self.restore_in(&conn, id, path).await;
        conn.execute(
            if restored.is_ok() {
                "COMMIT"
            } else {
                "ROLLBACK"
            },
            (),
        )
        .await?;
        restored
    }

    async fn restore_in(
        &self,
        conn: &turso::Connection,
        id: &str,
        path: &str,
    ) -> Result<u64, WriteError> {
        if self.path_taken(conn, path).await? {
            return Err(WriteError::PathTaken);
        }
        let mut restored = 0;
        for table in ["document", "context"] {
            restored += conn
                .execute(
                    &format!(
                        "UPDATE {table} SET deleted_at = NULL, deleted_root = NULL, \
                           updated_at = datetime('now') \
                         WHERE deleted_root = ?1"
                    ),
                    [id],
                )
                .await?;
        }
        Ok(restored)
    }

    /// Create a comment on `on_id` authored by `author_did`. Returns its id.
    pub async fn create_comment(
        &self,
        on_id: &str,
        context_id: &str,
        author_did: &str,
        text: &str,
    ) -> Result<String, DbError> {
        let id = format!("k-{}", crate::util::random_token(16));
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT INTO comment (id, on_id, context_id, author_did, text) \
             VALUES (?1, ?2, ?3, ?4, ?5)",
            [id.as_str(), on_id, context_id, author_did, text],
        )
        .await?;
        Ok(id)
    }

    /// Add `reactor_did`'s `emoji` reaction to `subject_uri` (idempotent via
    /// `upsert_reaction`'s unique triple). Returns the reaction id.
    pub async fn create_reaction(
        &self,
        subject_uri: &str,
        reactor_did: &str,
        emoji: &str,
    ) -> Result<String, DbError> {
        let id = format!("r-{}", crate::util::random_token(16));
        let now = crate::util::rfc3339_utc(crate::util::now_secs());
        self.upsert_reaction(&id, subject_uri, reactor_did, emoji, &now)
            .await?;
        Ok(id)
    }

    /// Remove `reactor_did`'s `emoji` reaction from `subject_uri` (toggle off).
    pub async fn remove_reaction(
        &self,
        subject_uri: &str,
        reactor_did: &str,
        emoji: &str,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute(
            "DELETE FROM reaction WHERE subject_uri = ?1 AND reactor_did = ?2 AND emoji = ?3",
            [subject_uri, reactor_did, emoji],
        )
        .await?;
        Ok(())
    }

    // -- Firehose depth-2 materialization: public container contexts (group/
    //    event), comments, and resolutions, keyed by the record's at-uri. --

    /// Materialize a public `group`/`event` record into the `context` view.
    /// `parent_uri` is a foreign key, so the caller must have checked it is in
    /// the view.
    pub async fn upsert_public_context(
        &self,
        uri: &str,
        kind: &str,
        name: &str,
        slug: &str,
        parent_uri: Option<&str>,
        created_at: &str,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        let path = match parent_uri {
            Some(parent) => match self.parent(&conn, parent).await? {
                Some(parent) => format!("{}/{slug}", parent.path),
                None => return Ok(()),
            },
            None => slug.to_string(),
        };
        conn.execute(
            "INSERT INTO context \
             (id, kind, name, slug, path, parent_id, visibility, published_uri, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'public', ?1, ?7) \
             ON CONFLICT(id) DO UPDATE SET kind = excluded.kind, name = excluded.name, \
               slug = excluded.slug, path = excluded.path, parent_id = excluded.parent_id, \
               created_at = excluded.created_at, updated_at = datetime('now')",
            vec![
                Value::Text(uri.to_string()),
                Value::Text(kind.to_string()),
                Value::Text(name.to_string()),
                Value::Text(slug.to_string()),
                Value::Text(path),
                opt_str_val(parent_uri),
                Value::Text(created_at.to_string()),
            ],
        )
        .await?;
        Ok(())
    }

    /// Delete a public context from the view by its at-uri.
    pub async fn delete_public_context(&self, uri: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("DELETE FROM context WHERE id = ?1", [uri])
            .await?;
        Ok(())
    }

    /// Whether `id` is a row of `table`. `table` is one of this crate's own table
    /// names, never caller input.
    async fn exists(&self, table: &'static str, id: &str) -> Result<bool, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(&format!("SELECT 1 FROM {table} WHERE id = ?1"), [id])
            .await?;
        Ok(rows.next().await?.is_some())
    }

    pub async fn context_exists(&self, id: &str) -> Result<bool, DbError> {
        self.exists("context", id).await
    }

    pub async fn post_exists(&self, id: &str) -> Result<bool, DbError> {
        self.exists("post", id).await
    }

    /// The context a subject at-uri belongs to, resolved through the view (a
    /// materialized document, comment, or post). `None` if the subject is not (yet)
    /// materialized, in which case a comment on it stays broadcast-only.
    pub async fn resolve_subject_context(
        &self,
        subject_uri: &str,
    ) -> Result<Option<String>, DbError> {
        let conn = self.db.acquire().await?;
        for sql in [
            "SELECT context_id FROM document WHERE id = ?1 OR published_uri = ?1 LIMIT 1",
            "SELECT context_id FROM comment WHERE id = ?1 LIMIT 1",
            "SELECT group_id FROM post WHERE id = ?1 LIMIT 1",
        ] {
            let mut rows = conn.query(sql, [subject_uri]).await?;
            if let Some(row) = rows.next().await?
                && let Some(ctx) = opt_text(&row, 0)
            {
                return Ok(Some(ctx));
            }
        }
        Ok(None)
    }

    /// Materialize a public `comment` record into the `comment` view.
    pub async fn upsert_public_comment(
        &self,
        uri: &str,
        on_id: &str,
        context_id: &str,
        author_did: &str,
        text: &str,
        created_at: &str,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute(
            "INSERT INTO comment (id, on_id, context_id, author_did, text, created_at) \
             VALUES (?1, ?2, ?3, ?4, ?5, ?6) \
             ON CONFLICT(id) DO UPDATE SET on_id = excluded.on_id, \
               context_id = excluded.context_id, author_did = excluded.author_did, \
               text = excluded.text, created_at = excluded.created_at",
            [uri, on_id, context_id, author_did, text, created_at],
        )
        .await?;
        Ok(())
    }

    /// Delete a public comment from the view by its at-uri.
    pub async fn delete_public_comment(&self, uri: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("DELETE FROM comment WHERE id = ?1", [uri])
            .await?;
        Ok(())
    }

    /// Materialize a public `resolution` record as a `document` (kind
    /// `resolution`), its body + status folded into the content JSON, authored by
    /// the org DID. `context_id` is the resolution's context at-uri and a foreign
    /// key, so the caller must have checked it is in the view.
    #[allow(clippy::too_many_arguments)]
    pub async fn upsert_public_resolution(
        &self,
        uri: &str,
        context_id: &str,
        title: &str,
        body: Option<&str>,
        status: &str,
        author_did: &str,
        created_at: &str,
    ) -> Result<(), DbError> {
        let content = serde_json::json!({ "body": body, "status": status }).to_string();
        let conn = self.db.acquire().await?;
        let Some(parent) = self.parent(&conn, context_id).await? else {
            return Ok(());
        };
        // The record key, not the title: a mirrored record's place must not
        // depend on what else happens to be in the view when it arrives.
        let slug = uri.rsplit('/').next().unwrap_or(uri);
        let path = format!("{}/{slug}", parent.path);
        conn.execute(
            "INSERT INTO document \
             (id, context_id, parent_id, kind, title, slug, path, owner_did, content, \
              visibility, published_uri, created_at) \
             VALUES (?1, ?2, ?2, 'resolution', ?3, ?4, ?5, ?6, ?7, 'public', ?1, ?8) \
             ON CONFLICT(id) DO UPDATE SET context_id = excluded.context_id, \
               parent_id = excluded.parent_id, title = excluded.title, path = excluded.path, \
               content = excluded.content, created_at = excluded.created_at, \
               updated_at = datetime('now')",
            [
                uri,
                context_id,
                title,
                slug,
                path.as_str(),
                author_did,
                content.as_str(),
                created_at,
            ],
        )
        .await?;
        conn.execute("DELETE FROM document_author WHERE document_id = ?1", [uri])
            .await?;
        conn.execute(
            "INSERT INTO document_author (document_id, author_did, ord) VALUES (?1, ?2, 0)",
            [uri, author_did],
        )
        .await?;
        Ok(())
    }

    /// Delete a public resolution (document + its author rows) by its at-uri.
    pub async fn delete_public_resolution(&self, uri: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        conn.execute("DELETE FROM document_author WHERE document_id = ?1", [uri])
            .await?;
        conn.execute("DELETE FROM document WHERE id = ?1", [uri])
            .await?;
        Ok(())
    }
}

/// A nullable TEXT param: `Value::Text` or SQL NULL.
fn opt_str_val(s: Option<&str>) -> Value {
    match s {
        Some(v) => Value::Text(v.to_string()),
        None => Value::Null,
    }
}

/// `?1, ?2, ..., ?n` for an `IN (...)` clause of `n` positional params.
fn in_placeholders(n: usize) -> String {
    (1..=n)
        .map(|i| format!("?{i}"))
        .collect::<Vec<_>>()
        .join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A schema-initialized in-memory db seeded with one context, a document
    /// with two authors (one DID, one free-text), a comment, and members.
    async fn seeded() -> Db {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let conn = db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO user (did, handle, display_name, legacy_id) \
               VALUES ('did:plc:alice', 'alice.test', 'Alice', NULL);
             INSERT INTO context (id, kind, name, slug, path) \
               VALUES ('c1', 'group', 'Group One', 'group-one', 'group-one');
             INSERT INTO document (id, context_id, parent_id, kind, title, slug, path) \
               VALUES ('d1', 'c1', 'c1', 'document', 'Doc', 'doc', 'group-one/doc');
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', 'did:plc:alice', NULL, 0);
             INSERT INTO document_author (document_id, author_did, author_text, ord) \
               VALUES ('d1', NULL, 'Guest', 1);
             INSERT INTO comment (id, on_id, context_id, author_did, author_text, text, legacy_id) \
               VALUES ('k1', 'd1', 'c1', 'did:plc:alice', NULL, 'nice', NULL);
             INSERT INTO member (id, user_did, context_id, role, active, email, claim_token, legacy_id) \
               VALUES ('m1', 'did:plc:alice', 'c1', 'owner', 1, 'alice@x.dk', 'tok-a', NULL);
             INSERT INTO member (id, user_did, context_id, role, active, email, claim_token, legacy_id) \
               VALUES ('m2', NULL, 'c1', 'member', 1, 'bob@x.dk', 'tok-b', NULL);
             INSERT INTO member (id, user_did, context_id, role, active, email, claim_token, legacy_id) \
               VALUES ('m3', NULL, 'c1', 'member', 0, 'gone@x.dk', 'tok-c', NULL);",
        )
        .await
        .expect("seed");
        db
    }

    #[tokio::test(flavor = "current_thread")]
    async fn node_owner_and_context_reads_document_and_comment() {
        let store = Store::new(seeded().await);
        // A document: owner is the first author WITH a DID (ord 0), not the
        // free-text author at ord 1.
        let doc = store
            .node_owner_and_context("d1")
            .await
            .expect("query")
            .expect("some");
        assert_eq!(doc.owner_id.as_deref(), Some("did:plc:alice"));
        assert_eq!(doc.context_id.as_deref(), Some("c1"));
        // A comment: author + context off the row.
        let com = store
            .node_owner_and_context("k1")
            .await
            .expect("query")
            .expect("some");
        assert_eq!(com.owner_id.as_deref(), Some("did:plc:alice"));
        assert_eq!(com.context_id.as_deref(), Some("c1"));
        // A missing node.
        assert!(
            store
                .node_owner_and_context("nope")
                .await
                .expect("query")
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn member_by_claim_token_maps_seam_fields() {
        let store = Store::new(seeded().await);
        let m = store
            .member_by_claim_token("tok-b")
            .await
            .expect("query")
            .expect("some");
        assert_eq!(m.id, "m2");
        assert_eq!(m.node_id, None, "pending invite has no bound user_did");
        assert_eq!(m.parent_id.as_deref(), Some("c1"));
        assert!(
            store
                .member_by_claim_token("absent")
                .await
                .expect("query")
                .is_none()
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn bind_member_to_user_is_guarded_and_idempotent() {
        let store = Store::new(seeded().await);
        // Binding follows a login, which has already written the user row.
        store.upsert_user_min("did:plc:bob").await.expect("user");
        // First bind of the pending invite succeeds.
        assert!(
            store
                .bind_member_to_user("m2", "did:plc:bob")
                .await
                .expect("bind")
        );
        // A second bind is a no-op (the guard `user_did IS NULL` matches nothing).
        assert!(
            !store
                .bind_member_to_user("m2", "did:plc:bob")
                .await
                .expect("rebind")
        );
        // The claim seam now sees the bound DID.
        let m = store
            .member_by_claim_token("tok-b")
            .await
            .expect("query")
            .expect("some");
        assert_eq!(m.node_id.as_deref(), Some("did:plc:bob"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn member_claim_token_reads_context_and_token() {
        let store = Store::new(seeded().await);
        let info = store
            .member_claim_token("m1")
            .await
            .expect("query")
            .expect("some");
        assert_eq!(info.parent_id.as_deref(), Some("c1"));
        assert_eq!(info.claim_token.as_deref(), Some("tok-a"));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn active_member_emails_excludes_inactive() {
        let store = Store::new(seeded().await);
        let mut emails = store.active_member_emails("c1").await.expect("query");
        emails.sort();
        // m1 + m2 are active; m3 is inactive and excluded.
        assert_eq!(emails, vec!["alice@x.dk", "bob@x.dk"]);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn push_subscriptions_upsert_fetch_and_delete() {
        let store = Store::new(seeded().await);
        store
            .upsert_push_subscription("did:plc:alice", "alice@x.dk", "https://ep/1", "k1", "a1")
            .await
            .expect("insert");
        // Re-subscribing the same endpoint updates keys, not duplicates.
        store
            .upsert_push_subscription("did:plc:alice", "alice@x.dk", "https://ep/1", "k2", "a2")
            .await
            .expect("update");
        store
            .upsert_push_subscription("did:plc:bob", "bob@x.dk", "https://ep/2", "k3", "a3")
            .await
            .expect("insert 2");

        let subs = store
            .subscriptions_for_emails(&["alice@x.dk".into(), "bob@x.dk".into()])
            .await
            .expect("fetch");
        assert_eq!(
            subs.len(),
            2,
            "one row per endpoint (no duplicate on upsert)"
        );
        let alice = subs.iter().find(|s| s.endpoint == "https://ep/1").unwrap();
        assert_eq!(alice.p256dh, "k2", "upsert refreshed the key");

        // Empty inputs short-circuit.
        assert!(
            store
                .subscriptions_for_emails(&[])
                .await
                .expect("empty")
                .is_empty()
        );

        store
            .delete_subscriptions_by_endpoint(&["https://ep/1".into()])
            .await
            .expect("delete");
        let after = store
            .subscriptions_for_emails(&["alice@x.dk".into(), "bob@x.dk".into()])
            .await
            .expect("fetch after delete");
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].endpoint, "https://ep/2");
    }
}
