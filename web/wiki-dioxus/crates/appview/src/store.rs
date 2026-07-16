//! The AppView data-access seam, ported from the interim backend's
//! `backend/src/store.rs` admin-GraphQL bodies to parameterized SQL against the
//! reconciled Turso schema (`wiki_schema::ENTITY_SCHEMA` + this crate's
//! [`crate::schema::RUNTIME_DDL`]). The intent-named surface and the small typed
//! return structs are preserved so handlers stay storage-agnostic; only the
//! query bodies changed from GraphQL to SQL. This is the seam class where at
//! cutover only this module was ever meant to be rewritten.
//!
//! DELIBERATELY NOT PORTED here (see `docs/rewrite-kickoff-plan.md` item 4):
//! - `is_active_member` / `is_active_owner`: the interim keys authz on the NHost
//!   `uid`, but the target keys it on `user_did`, and 0 DIDs are linked with no
//!   uid->DID resolution yet. They are a DID-keyed rewrite that waits on the
//!   DID-binding flow, NOT a mechanical body swap.
//! - `poll_meta` and any voting query: deferred with the voting shapes.
//!
//! Schema shifts from the interim GraphQL these queries reconcile to:
//! - the universal `node` table split into `document`/`comment`/`context`, so a
//!   "node's owner + context" is now read from `document`/`comment` with the
//!   owner realized as the document's first author DID (`author_did` in the
//!   `document_author` join), free-text-only authors having no notifiable DID;
//! - `members.nodeId`/`parentId`/`accepted` became `member.user_did`/
//!   `context_id` and the folded-in active state (there is no `accepted`);
//! - `push_subscriptions` is AppView runtime infra (`RUNTIME_DDL`), keyed by
//!   endpoint, with `user_id` now `user_did`.

use crate::db::{Db, DbError};
use turso::Value;
use wiki_domain_types::{Author, Context, ContextKind, Document, DocumentKind};

/// Parse a snake_case DB enum value (e.g. `"document"`, `"private"`) into a
/// `#[serde(rename_all = "snake_case")]` domain enum; `None` on an unknown value.
fn parse_enum<T: serde::de::DeserializeOwned>(s: &str) -> Option<T> {
    serde_json::from_value(serde_json::Value::String(s.to_string())).ok()
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

impl Store {
    pub fn new(db: Db) -> Self {
        Self { db }
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
    /// active in the context (surfaces as a constraint error).
    pub async fn bind_member_to_user(
        &self,
        member_id: &str,
        user_did: &str,
    ) -> Result<bool, DbError> {
        let conn = self.db.acquire().await?;
        let affected = conn
            .execute(
                "UPDATE member SET user_did = ?1 WHERE id = ?2 AND user_did IS NULL",
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
    ///
    /// FINDING (turso 0.2.2): neither `INSERT ... ON CONFLICT` nor `INSERT OR
    /// REPLACE` parse ("ON CONFLICT clause is not supported"), so the interim's
    /// single upsert statement cannot port verbatim. This does the portable
    /// two-step instead: UPDATE by the endpoint key, and INSERT only if no row
    /// matched. Revisit when turso gains upsert support.
    pub async fn upsert_push_subscription(
        &self,
        user_did: &str,
        email: &str,
        endpoint: &str,
        p256dh: &str,
        auth: &str,
    ) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        let updated = conn
            .execute(
                "UPDATE push_subscription SET user_did = ?1, email = ?2, p256dh = ?3, auth = ?4 \
                 WHERE endpoint = ?5",
                [user_did, email, p256dh, auth, endpoint],
            )
            .await?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO push_subscription \
                 (endpoint, user_did, email, p256dh, auth) VALUES (?1, ?2, ?3, ?4, ?5)",
                [endpoint, user_did, email, p256dh, auth],
            )
            .await?;
        }
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

    /// Upsert a MINIMAL user row (just the DID) so an author FK target exists for
    /// a firehose-materialized public record. No-op if the user already exists.
    /// The nullable-UNIQUE `legacy_id` is passed as an explicit NULL (turso
    /// rejects omitting it).
    pub async fn upsert_user_min(&self, did: &str) -> Result<(), DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query("SELECT 1 FROM user WHERE did = ?1 LIMIT 1", [did])
            .await?;
        if rows.next().await?.is_some() {
            return Ok(());
        }
        conn.execute("INSERT INTO user (did, legacy_id) VALUES (?1, NULL)", [did])
            .await?;
        Ok(())
    }

    /// Materialize a public `com.example.wiki.post` record into the `post` view,
    /// keyed by its at-uri (also its `published_uri` and `legacy_id`). Upsert via
    /// UPDATE-then-INSERT (turso has no `ON CONFLICT`). `group`/`reply` are the
    /// record's at-uris, stored as-is (resolving them to local ids is depth-3).
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
        let group = opt_str_val(group);
        let reply = opt_str_val(reply);
        let updated = conn
            .execute(
                "UPDATE post SET author_did = ?1, text = ?2, group_id = ?3, reply_to = ?4, \
                 visibility = 'public', published_uri = ?5, created_at = ?6 WHERE id = ?5",
                vec![
                    Value::Text(author_did.to_string()),
                    Value::Text(text.to_string()),
                    group.clone(),
                    reply.clone(),
                    Value::Text(uri.to_string()),
                    Value::Text(created_at.to_string()),
                ],
            )
            .await?;
        if updated == 0 {
            conn.execute(
                "INSERT INTO post \
                 (id, author_did, text, group_id, reply_to, visibility, published_uri, created_at, legacy_id) \
                 VALUES (?1, ?2, ?3, ?4, ?5, 'public', ?1, ?6, ?1)",
                vec![
                    Value::Text(uri.to_string()),
                    Value::Text(author_did.to_string()),
                    Value::Text(text.to_string()),
                    group,
                    reply,
                    Value::Text(created_at.to_string()),
                ],
            )
            .await?;
        }
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
    pub async fn read_document(&self, id: &str) -> Result<Option<Document>, DbError> {
        // Extract the row fields first (releasing the query) before the authors
        // sub-query on a fresh connection.
        let fields = {
            let conn = self.db.acquire().await?;
            let mut rows = conn
                .query(
                    "SELECT id, context_id, parent_id, kind, title, content, visibility, \
                     published_uri, created_at FROM document WHERE id = ?1",
                    [id],
                )
                .await?;
            match rows.next().await? {
                Some(row) => (
                    row.get::<String>(0)?,
                    row.get::<String>(1)?,
                    opt_text(&row, 2),
                    row.get::<String>(3)?,
                    row.get::<String>(4)?,
                    opt_text(&row, 5),
                    opt_text(&row, 6),
                    opt_text(&row, 7),
                    opt_text(&row, 8),
                ),
                None => return Ok(None),
            }
        };
        let (
            id,
            context_id,
            parent_id,
            kind,
            title,
            content,
            visibility,
            published_uri,
            created_at,
        ) = fields;
        let authors = self.document_authors(&id).await?;
        Ok(Some(Document {
            id,
            context_id,
            parent_id,
            kind: parse_enum(&kind).unwrap_or(DocumentKind::Document),
            title,
            content: content.and_then(|s| serde_json::from_str(&s).ok()),
            authors,
            visibility: visibility.and_then(|s| parse_enum(&s)).unwrap_or_default(),
            published_uri,
            created_at,
            legacy_id: None,
        }))
    }

    /// A context (group / event) by id. `None` if no such context.
    pub async fn read_context(&self, id: &str) -> Result<Option<Context>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT id, kind, name, slug, parent_id, visibility, published_uri, created_at \
                 FROM context WHERE id = ?1",
                [id],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(Context {
            id: row.get::<String>(0)?,
            kind: parse_enum(&row.get::<String>(1)?).unwrap_or(ContextKind::Group),
            name: row.get::<String>(2)?,
            slug: row.get::<String>(3)?,
            parent_id: opt_text(&row, 4),
            visibility: opt_text(&row, 5)
                .and_then(|s| parse_enum(&s))
                .unwrap_or_default(),
            published_uri: opt_text(&row, 6),
            created_at: opt_text(&row, 7),
            legacy_id: None,
        }))
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
             INSERT INTO context (id, kind, name, slug, legacy_id) \
               VALUES ('c1', 'group', 'Group One', 'group-one', NULL);
             INSERT INTO document (id, context_id, kind, title, legacy_id) \
               VALUES ('d1', 'c1', 'document', 'Doc', NULL);
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
