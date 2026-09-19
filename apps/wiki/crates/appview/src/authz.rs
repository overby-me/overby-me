//! Who may read and write what, keyed on the DID.
//!
//! One module, because the interim backend wrote this predicate four times and
//! the four disagreed (`docs/pre-rewrite-plan.md`, round 1 item 1).
//!
//! The read rule is the interim's row permissions (`docs/read-permissions.md`)
//! carried over, less the two clauses that matched a member by email: a DID
//! login asserts no email, so an invitation is claimed (`Store::bind_member_to_user`)
//! rather than matched. A row is readable when it is public, when its context
//! is public, when the caller is a member of its context, or when the caller
//! wrote it. Membership does not inherit: a member of a group is not thereby a
//! member of an event inside it, as today.
//!
//! `member.active` is NOT part of that rule. It is the voting-rights flag an
//! owner sets, and reading was always by membership alone
//! (`migrations/0024-read-by-context-membership-again.sql`, which records what
//! it cost to get that wrong once). Only [`Authz::is_active_member`] and
//! [`Authz::is_active_owner`] ask about it, for what the interim backend's
//! predicates of the same names gate: voting, notifying, administering.
//!
//! The rule is SQL rather than a check after the fact so that a list query
//! filters before its `LIMIT`. Each fragment takes the index of the positional
//! parameter holding the caller's DID; bind NULL for an anonymous caller and
//! every clause about the caller is simply false.

use crate::db::{Db, DbError};
use wiki_domain_types::Role;

const MEMBER_OF: &str = "SELECT 1 FROM member m WHERE m.user_did";

/// Holds for the rows of context `alias` the caller may read.
pub fn readable_context(alias: &str, caller: usize) -> String {
    format!(
        "({alias}.visibility = 'public' \
         OR EXISTS ({MEMBER_OF} = ?{caller} AND m.context_id = {alias}.id))"
    )
}

/// Holds for the rows of document `alias` the caller may read.
pub fn readable_document(alias: &str, caller: usize) -> String {
    format!(
        "({alias}.visibility = 'public' \
         OR EXISTS (SELECT 1 FROM context c \
                    WHERE c.id = {alias}.context_id AND c.visibility = 'public') \
         OR EXISTS ({MEMBER_OF} = ?{caller} AND m.context_id = {alias}.context_id) \
         OR EXISTS (SELECT 1 FROM document_author a \
                    WHERE a.document_id = {alias}.id AND a.author_did = ?{caller}))"
    )
}

/// Holds for the rows of comment `alias` the caller may read.
pub fn readable_comment(alias: &str, caller: usize) -> String {
    format!(
        "({alias}.author_did = ?{caller} \
         OR EXISTS (SELECT 1 FROM context c \
                    WHERE c.id = {alias}.context_id AND c.visibility = 'public') \
         OR EXISTS ({MEMBER_OF} = ?{caller} AND m.context_id = {alias}.context_id))"
    )
}

// -- The write model: who may create which kind under which parent. --
//
// The interim keeps this per context, as `permissions` rows seeded from one
// template (`context_permission_objects` in the frontend's `graphql/nodes.rs`).
// That template is the rule in practice, so it is carried here as one table.
// It says who may CREATE. It says nothing about reading, and must never be made
// to: that is the mistake `migrations/0024` records.

/// The parent kind every context kind (group, event, site) answers to.
pub const CONTEXT: &str = "context";

const CONTAINERS: &[&str] = &[CONTEXT, "folder"];

/// (kind, the role it takes, the parent kinds it may be created under).
const CREATE_RULES: &[(&str, Role, &[&str])] = &[
    ("folder", Role::Owner, CONTAINERS),
    ("document", Role::Owner, CONTAINERS),
    ("file", Role::Owner, CONTAINERS),
    ("position", Role::Owner, &["folder"]),
    ("policy", Role::Member, &["folder"]),
    ("candidate", Role::Member, &["position"]),
    ("change", Role::Member, &["policy", "change", "file"]),
    ("question", Role::Member, &["position", "file"]),
];

/// What a comment may be written on: content, never a container.
pub const COMMENTABLE: &[&str] = &[
    "policy",
    "change",
    "document",
    "file",
    "position",
    "candidate",
];

/// Discussion is not an addition to what was locked: closing a resolution to new
/// amendments must not stop people asking about it.
const EXEMPT_FROM_LOCK: &[&str] = &["question"];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Refusal {
    /// Not a kind anyone creates this way.
    UnknownKind,
    /// That kind does not go under that parent.
    WrongParent,
    /// That kind is an owner's to create.
    NeedsOwner,
    /// The parent is locked against new children.
    Locked,
}

impl Refusal {
    pub fn message(self) -> &'static str {
        match self {
            Refusal::UnknownKind => "not a kind that can be created",
            Refusal::WrongParent => "that kind cannot be created under that parent",
            Refusal::NeedsOwner => "only an owner of the context may create that",
            Refusal::Locked => "the parent is locked against new content",
        }
    }
}

/// Whether a member may create a `kind` under a parent of `parent_kind`.
///
/// An owner may add to a locked parent: the lock is theirs, and a chair who
/// closes candidature and then has to enter a late one by hand should not have
/// to unlock the position to do it.
pub fn may_create(
    kind: &str,
    parent_kind: &str,
    parent_attachable: bool,
    membership: Membership,
) -> Result<(), Refusal> {
    let (_, role, parents) = CREATE_RULES
        .iter()
        .find(|(k, _, _)| *k == kind)
        .ok_or(Refusal::UnknownKind)?;
    if !parents.contains(&parent_kind) {
        return Err(Refusal::WrongParent);
    }
    let is_owner = membership.role == Role::Owner;
    if *role == Role::Owner && !is_owner {
        return Err(Refusal::NeedsOwner);
    }
    if !parent_attachable && !is_owner && !EXEMPT_FROM_LOCK.contains(&kind) {
        return Err(Refusal::Locked);
    }
    Ok(())
}

/// What a group, an event or a canvas may sit in: a context, or a folder in one.
pub const PLACES: &[&str] = CONTAINERS;

/// What a poll may be opened on: the interim's rule for `vote/poll`.
pub const POLLABLE: &[&str] = &["policy", "change", "position"];

/// Everything `membership` may make under a node of `parent_kind`, so that a
/// screen offers what the server will accept and nothing it will refuse. Beside
/// the document kinds: `comment`, and the things an owner makes by their own
/// procedure (`group`, `event`, `canvas`, `poll`).
pub fn creatable(
    parent_kind: &str,
    parent_attachable: bool,
    membership: Membership,
) -> Vec<&'static str> {
    let mut kinds: Vec<&'static str> = CREATE_RULES
        .iter()
        .map(|(kind, _, _)| *kind)
        .filter(|kind| may_create(kind, parent_kind, parent_attachable, membership).is_ok())
        .collect();
    if COMMENTABLE.contains(&parent_kind) {
        kinds.push("comment");
    }
    if membership.role == Role::Owner {
        if PLACES.contains(&parent_kind) {
            kinds.extend(["group", "event", "canvas"]);
        }
        if POLLABLE.contains(&parent_kind) {
            kinds.push("poll");
        }
    }
    kinds
}

/// A caller's standing towards one node.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Standing {
    /// They created it.
    pub owns_node: bool,
    /// They hold the owner role in its context.
    pub owns_context: bool,
}

impl Standing {
    /// Two different powers, deliberately not the same. A node's owner may edit
    /// it while it is a draft: submitting makes it immutable, which is the point,
    /// since the room is about to vote on it and its author is exactly who might
    /// change it. A context owner may edit regardless, because they answer for
    /// the whole meeting and have a typo in a submitted motion to correct.
    pub fn may_edit(self, mutable: bool) -> bool {
        self.owns_context || (self.owns_node && mutable)
    }

    /// The order of siblings, the folder lock, and reopening what was submitted
    /// are the meeting's business, not an author's.
    pub fn may_arrange(self) -> bool {
        self.owns_context
    }

    pub fn may_delete(self) -> bool {
        self.owns_node || self.owns_context
    }
}

/// A person's standing in a context.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Membership {
    pub role: Role,
    /// Voting rights, set by an owner. Says nothing about reading or writing.
    pub active: bool,
}

#[derive(Clone)]
pub struct Authz {
    db: Db,
}

impl Authz {
    pub fn new(db: Db) -> Self {
        Self { db }
    }

    /// The caller's membership of a context. A pending invitation has no DID
    /// bound yet, so it is nobody's membership.
    /// Whether `did` owns a site: who the feedback and the admin views are for.
    pub async fn owns_a_site(&self, did: &str) -> Result<bool, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT 1 FROM member m JOIN context c ON c.id = m.context_id \
                 WHERE c.kind = 'site' AND c.deleted_at IS NULL \
                   AND m.user_did = ?1 AND m.role = 'owner' LIMIT 1",
                [did],
            )
            .await?;
        Ok(rows.next().await?.is_some())
    }

    pub async fn membership(
        &self,
        context_id: &str,
        did: &str,
    ) -> Result<Option<Membership>, DbError> {
        let conn = self.db.acquire().await?;
        let mut rows = conn
            .query(
                "SELECT role, active FROM member WHERE context_id = ?1 AND user_did = ?2",
                [context_id, did],
            )
            .await?;
        let Some(row) = rows.next().await? else {
            return Ok(None);
        };
        Ok(Some(Membership {
            role: match row.get::<String>(0)?.as_str() {
                "owner" => Role::Owner,
                _ => Role::Member,
            },
            active: row.get::<i64>(1)? != 0,
        }))
    }

    /// The caller's standing towards a node created by `node_owner` in
    /// `context_id`. Ownership of a context is the role alone, as the interim's
    /// `isContextOwner` reads it.
    pub async fn standing(
        &self,
        context_id: &str,
        node_owner: Option<&str>,
        did: &str,
    ) -> Result<Standing, DbError> {
        let membership = self.membership(context_id, did).await?;
        Ok(Standing {
            owns_node: node_owner == Some(did),
            owns_context: membership.is_some_and(|m| m.role == Role::Owner),
        })
    }

    /// May read the context and write content into it.
    pub async fn is_member(&self, context_id: &str, did: &str) -> Result<bool, DbError> {
        Ok(self.membership(context_id, did).await?.is_some())
    }

    /// Holds voting rights in the context.
    pub async fn is_active_member(&self, context_id: &str, did: &str) -> Result<bool, DbError> {
        Ok(self
            .membership(context_id, did)
            .await?
            .is_some_and(|m| m.active))
    }

    /// May administer the context.
    pub async fn is_active_owner(&self, context_id: &str, did: &str) -> Result<bool, DbError> {
        Ok(self
            .membership(context_id, did)
            .await?
            .is_some_and(|m| m.active && m.role == Role::Owner))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn seeded() -> Authz {
        let db = Db::open(":memory:").await.expect("open");
        db.init_schema().await.expect("schema");
        let conn = db.acquire().await.expect("conn");
        conn.execute_batch(
            "INSERT INTO user (did) VALUES ('did:plc:owner');
             INSERT INTO user (did) VALUES ('did:plc:member');
             INSERT INTO user (did) VALUES ('did:plc:gone');
             INSERT INTO context (id, kind, name, slug, path) VALUES ('c1', 'group', 'G', 'g', 'g');
             INSERT INTO context (id, kind, name, slug, path, parent_id) VALUES ('c2', 'event', 'E', 'e', 'g/e', 'c1');
             INSERT INTO member (id, user_did, context_id, role, active) VALUES ('m1', 'did:plc:owner', 'c1', 'owner', 1);
             INSERT INTO member (id, user_did, context_id, role, active) VALUES ('m2', 'did:plc:member', 'c1', 'member', 1);
             INSERT INTO member (id, user_did, context_id, role, active) VALUES ('m3', 'did:plc:gone', 'c1', 'owner', 0);
             INSERT INTO member (id, user_did, context_id, role, active, email) VALUES ('m4', NULL, 'c1', 'owner', 1, 'invited@x.dk');",
        )
        .await
        .expect("seed");
        Authz::new(db)
    }

    #[tokio::test(flavor = "current_thread")]
    async fn a_membership_needs_a_bound_row() {
        let a = seeded().await;
        let owner = a.membership("c1", "did:plc:owner").await.expect("q");
        assert_eq!(
            owner,
            Some(Membership {
                role: Role::Owner,
                active: true
            })
        );
        assert_eq!(
            a.membership("c1", "did:plc:stranger").await.expect("q"),
            None
        );
    }

    /// `active` is voting rights. Losing it must not shut a member out of the
    /// pages of their own group.
    #[tokio::test(flavor = "current_thread")]
    async fn an_inactive_member_still_belongs_but_holds_no_rights() {
        let a = seeded().await;
        assert!(a.is_member("c1", "did:plc:gone").await.expect("q"));
        assert!(!a.is_active_member("c1", "did:plc:gone").await.expect("q"));
        assert!(
            !a.is_active_owner("c1", "did:plc:gone").await.expect("q"),
            "a deactivated owner could still administer"
        );
    }

    #[tokio::test(flavor = "current_thread")]
    async fn owner_is_a_member_but_a_member_is_not_an_owner() {
        let a = seeded().await;
        assert!(a.is_active_member("c1", "did:plc:owner").await.expect("q"));
        assert!(a.is_active_owner("c1", "did:plc:owner").await.expect("q"));
        assert!(a.is_active_member("c1", "did:plc:member").await.expect("q"));
        assert!(!a.is_active_owner("c1", "did:plc:member").await.expect("q"));
    }

    const MEMBER: Membership = Membership {
        role: Role::Member,
        active: true,
    };
    const OWNER: Membership = Membership {
        role: Role::Owner,
        active: true,
    };

    #[test]
    fn an_author_edits_a_draft_and_a_chair_edits_anything() {
        let author = Standing {
            owns_node: true,
            owns_context: false,
        };
        let chair = Standing {
            owns_node: false,
            owns_context: true,
        };
        let stranger = Standing {
            owns_node: false,
            owns_context: false,
        };
        assert!(author.may_edit(true), "a draft is theirs");
        assert!(
            !author.may_edit(false),
            "the room votes on what was submitted"
        );
        assert!(chair.may_edit(false));
        assert!(!stranger.may_edit(true));

        assert!(
            !author.may_arrange(),
            "the lock and the order are the chair's"
        );
        assert!(chair.may_arrange());
        assert!(author.may_delete());
        assert!(chair.may_delete());
        assert!(!stranger.may_delete());
    }

    #[test]
    fn members_write_motions_and_owners_make_the_structure() {
        assert_eq!(may_create("policy", "folder", true, MEMBER), Ok(()));
        assert_eq!(may_create("candidate", "position", true, MEMBER), Ok(()));
        assert_eq!(may_create("change", "policy", true, MEMBER), Ok(()));
        for structural in ["folder", "document", "file"] {
            assert_eq!(
                may_create(structural, CONTEXT, true, MEMBER),
                Err(Refusal::NeedsOwner),
                "{structural}"
            );
            assert_eq!(may_create(structural, CONTEXT, true, OWNER), Ok(()));
        }
        assert_eq!(
            may_create("position", "folder", true, MEMBER),
            Err(Refusal::NeedsOwner)
        );
    }

    #[test]
    fn a_screen_is_told_what_the_server_will_accept() {
        let member = Membership {
            role: Role::Member,
            active: false,
        };
        assert_eq!(creatable("folder", true, member), ["policy"]);
        assert_eq!(
            creatable("folder", false, member),
            Vec::<&str>::new(),
            "locked"
        );
        assert_eq!(creatable("policy", true, member), ["change", "comment"]);
        assert_eq!(
            creatable("position", false, member),
            ["question", "comment"],
            "a lock leaves questions"
        );
        assert_eq!(
            creatable("folder", false, OWNER),
            [
                "folder", "document", "file", "position", "policy", "group", "event", "canvas"
            ]
        );
        assert_eq!(
            creatable("policy", true, OWNER),
            ["change", "comment", "poll"]
        );
        assert_eq!(
            creatable(CONTEXT, true, OWNER),
            ["folder", "document", "file", "group", "event", "canvas"]
        );
    }

    #[test]
    fn a_kind_goes_only_where_it_belongs() {
        assert_eq!(
            may_create("policy", CONTEXT, true, OWNER),
            Err(Refusal::WrongParent),
            "a motion belongs in a folder, even for an owner"
        );
        assert_eq!(
            may_create("candidate", "folder", true, OWNER),
            Err(Refusal::WrongParent)
        );
        assert_eq!(
            may_create("poll", "policy", true, OWNER),
            Err(Refusal::UnknownKind),
            "a poll is opened, not created as a document"
        );
    }

    #[test]
    fn a_lock_stops_members_but_not_the_owner_or_a_question() {
        assert_eq!(
            may_create("candidate", "position", false, MEMBER),
            Err(Refusal::Locked)
        );
        assert_eq!(may_create("candidate", "position", false, OWNER), Ok(()));
        assert_eq!(
            may_create("question", "position", false, MEMBER),
            Ok(()),
            "closing candidature must not stop people asking about it"
        );
    }

    #[test]
    fn voting_rights_have_no_say_in_what_a_member_may_write() {
        let inactive = Membership {
            active: false,
            ..MEMBER
        };
        assert_eq!(may_create("policy", "folder", true, inactive), Ok(()));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn membership_does_not_reach_into_a_nested_context() {
        let a = seeded().await;
        assert!(
            !a.is_member("c2", "did:plc:owner").await.expect("q"),
            "owning a group made someone a member of the event inside it"
        );
    }
}
