//! Frontend-owned domain types: the anti-corruption seam between the Dioxus UI
//! and whatever backend serves it. Components read and write these plain serde
//! types; `graphql.rs` maps its cynic wire types (the `*Row` structs bound to
//! `graphql/schema.graphql`) to and from these at the query/mutation boundary.
//!
//! Nothing here carries a cynic derive or a schema binding, so swapping the
//! Hasura/cynic data layer for the atproto AppView is contained to that mapping
//! layer instead of rippling through every component. The field shapes mirror
//! today's GraphQL selection for a zero-behaviour-change port; they are expected
//! to be re-derived from lexicons at the rewrite, so treat them as the interim
//! frontend view, not a canonical shared model.

use serde::{Deserialize, Serialize};

// --- Scalars (plain newtypes; deliberately NOT cynic scalars) ---

/// A UUID as its canonical string. `.0` is the raw id, as everywhere in the UI.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Uuid(pub String);

/// A `timestamptz` as its ISO-8601 string.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timestamptz(pub String);

/// A `jsonb` blob (Slate document JSON, poll options, file metadata, …).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jsonb(pub serde_json::Value);

// --- Nodes ---

/// A node with its children, members and computed permission flags: the core
/// read model behind every `?app=` screen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeWithChildren {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub context_id: Option<Uuid>,
    pub owner_id: Option<Uuid>,
    pub mutable: bool,
    pub index: i32,
    pub data: Option<Jsonb>,
    pub mime: Option<MimeFields>,
    pub parent: Option<Box<ParentNodeFields>>,
    pub children: Vec<ChildNodeFields>,
    pub members: Vec<MemberFields>,
    /// `is_owner` = the session user owns this node; `is_context_owner` = they
    /// own its context. Drive owner-only UI gating.
    pub is_owner: Option<bool>,
    pub is_context_owner: Option<bool>,
    /// Whether children may be added (the folder "lock"; owner-toggleable).
    pub attachable: bool,
    pub created_at: Option<Timestamptz>,
    /// The creating user (fallback author label when no explicit author chip).
    pub owner: Option<UserRef>,
}

/// A node's basic fields: no children, used by search and the appbar.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub context_id: Option<Uuid>,
    pub owner_id: Option<Uuid>,
    pub mutable: bool,
    pub index: i32,
    /// Computed ordinal among same-type siblings (1-based); drives A/B/C and
    /// 1/2/3 avatar labels for policies / change proposals.
    pub get_index: Option<i32>,
    pub data: Option<Jsonb>,
    pub mime: Option<MimeFields>,
    pub is_owner: Option<bool>,
    pub is_context_owner: Option<bool>,
    pub created_at: Option<Timestamptz>,
    /// The parent node, for the search-result secondary line ("in <parent>").
    pub parent: Option<ParentNodeFields>,
}

/// A child node row (folder view, "Newest" list, export).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ChildNodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    pub mutable: bool,
    pub index: i32,
    pub created_at: Option<Timestamptz>,
    pub owner_id: Option<Uuid>,
    pub data: Option<Jsonb>,
    pub mime: Option<MimeFields>,
    pub is_owner: Option<bool>,
    pub is_context_owner: Option<bool>,
    /// Creating user (fallback label for questions/candidates/comments/amendments).
    pub owner: Option<UserRef>,
    /// The parent node, for the "Newest" list's secondary line ("in <parent>").
    pub parent: Option<ParentNodeFields>,
}

/// The lean child shape for the drawer tree: carries a `child_count` (flattened
/// from the GraphQL `children_aggregate`) instead of the children themselves.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DrawerChildFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    pub mutable: bool,
    pub data: Option<Jsonb>,
    /// Visible-child count, so the drawer expander only shows when it has some.
    pub child_count: i32,
}

impl DrawerChildFields {
    /// Whether this node has any (visible) children to expand.
    pub fn has_children(&self) -> bool {
        self.child_count > 0
    }
}

/// The id + key returned by a node insert (create node / context / poll).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InsertedNode {
    pub id: Uuid,
    pub key: String,
}

/// A context node (group / event) for the home list and orphan admin.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ContextNodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub created_at: Option<Timestamptz>,
    /// A file's content `type`, so orphan file nodes show a format-specific icon.
    pub data: Option<Jsonb>,
}

/// A referenced parent node (the "in <parent>" secondary line, breadcrumbs).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParentNodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
}

// --- Mime types ---

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MimeFields {
    pub id: String,
    pub icon: String,
    pub hidden: bool,
    pub context: bool,
}

// --- Members / users ---

/// A membership row on a node: the author chips on documents and the member
/// list of a context.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberFields {
    pub id: Uuid,
    pub name: Option<String>,
    pub email: Option<String>,
    pub accepted: bool,
    pub active: bool,
    pub owner: bool,
    pub hidden: bool,
    pub node_id: Option<Uuid>,
    pub user: Option<UserRef>,
    pub node: Option<MemberNodeRef>,
}

impl MemberFields {
    /// The display label for a member: their explicit name, else the linked
    /// user's display name, else empty.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| self.user.as_ref().map(|u| u.display_name.clone()))
            .unwrap_or_default()
    }
}

/// The node a membership hangs on (only its mime, for gating).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemberNodeRef {
    pub mime_id: Option<String>,
}

/// A user reference: id + display name + avatar (gravatar, or Bluesky once linked).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserRef {
    pub id: Uuid,
    pub display_name: String,
    pub avatar_url: String,
}

/// A user row from the profile / user-search queries.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct UserSearchFields {
    pub id: Uuid,
    pub display_name: String,
    pub avatar_url: String,
}

/// A pending invitation (a member row with a resolvable parent context).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvitationFields {
    pub id: Uuid,
    pub parent: Option<ParentNodeFields>,
}

// --- Permissions ---

/// A per-context permission rule (the insert/select/delete gate template).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PermissionFields {
    pub id: Uuid,
    pub mime_id: Option<String>,
    pub role: String,
    pub insert: bool,
    pub select: bool,
    pub delete: bool,
    pub active: bool,
}

// --- Polls ---

/// A poll summary row for the admin poll list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PollSummaryFields {
    pub id: Uuid,
    pub name: String,
    pub data: Option<Jsonb>,
    pub created_at: Option<Timestamptz>,
    /// Whether the poll is still open (mutable); drives the open/closed badge.
    pub mutable: bool,
}

// NOTE: `Author`, `Crumb` and `BallotRules` are already plain cynic-free serde
// structs and stay in `graphql.rs` (the plan's existing precedent); they are not
// part of this seam because they never carried a cynic derive.

// --- Write-side inputs (mirrors of the GraphQL `*_set_input` / `*_insert_input`
// objects; `graphql.rs` maps these to its cynic input types). Unset `None`
// fields are omitted from the mutation, never sent as explicit null. ---

/// Fields to set when creating a node.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodesInsertInput {
    pub name: Option<String>,
    pub key: Option<String>,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub context_id: Option<Uuid>,
    pub data: Option<Jsonb>,
    pub mutable: Option<bool>,
    pub index: Option<i32>,
}

/// Fields to update on a node.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct NodesSetInput {
    pub name: Option<String>,
    pub data: Option<Jsonb>,
    pub mutable: Option<bool>,
    pub index: Option<i32>,
    pub attachable: Option<bool>,
    pub context_id: Option<Uuid>,
    pub owner_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
    pub created_at: Option<Timestamptz>,
}

/// Fields to update on a member.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct MembersSetInput {
    pub accepted: Option<bool>,
    pub active: Option<bool>,
    pub email: Option<String>,
    pub name: Option<String>,
    pub owner: Option<bool>,
    pub hidden: Option<bool>,
    pub node_id: Option<Uuid>,
    pub parent_id: Option<Uuid>,
}
