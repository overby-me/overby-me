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

    #[tokio::test(flavor = "current_thread")]
    async fn membership_does_not_reach_into_a_nested_context() {
        let a = seeded().await;
        assert!(
            !a.is_member("c2", "did:plc:owner").await.expect("q"),
            "owning a group made someone a member of the event inside it"
        );
    }
}
