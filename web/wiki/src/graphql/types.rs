//! The schema, as Rust: the fragments and input objects the queries are built
//! from. No operations live here — this is the shape of the data, and the
//! modules beside it are what the app asks for.

use super::*;

// Custom scalar types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
pub struct Uuid(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timestamptz(pub String);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jsonb(pub serde_json::Value);

// --- Node fields (basic — no children, no data) ---

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct SearchNodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub path: Option<String>,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub context_id: Option<Uuid>,
    pub owner_id: Option<Uuid>,
    pub mutable: bool,
    pub index: i32,
    pub get_index: Option<i32>,
    /// The one thing a result row needs out of the document: a file's content
    /// `type`, which picks its icon.
    ///
    /// `data(path: "type")` rather than `data` — Hasura will select INSIDE a
    /// jsonb column, and the difference is the whole point of this fragment. A
    /// search for three letters was answering 1.5 MB because thirty rows each
    /// carried a complete document to be thrown away except for one string.
    /// With the path it is 23 KB, and the icons still work.
    #[arguments(path: "type")]
    pub data: Option<Jsonb>,
    pub mime: Option<MimeFields>,
    pub is_owner: Option<bool>,
    pub is_context_owner: Option<bool>,
    pub created_at: Option<Timestamptz>,
    /// The lean parent: a result prints its name and nothing else.
    pub parent: Option<ParentNameRef>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct NodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    /// Slash-joined keys from the root (`ru/lm2026/dagsorden`), maintained by a
    /// database trigger. Null for a node whose parent row is missing.
    pub path: Option<String>,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub context_id: Option<Uuid>,
    pub owner_id: Option<Uuid>,
    pub mutable: bool,
    pub index: i32,
    // Computed ordinal among same-type siblings (1-based) — drives the A/B/C and
    // 1/2/3 avatar labels for policies / change proposals.
    pub get_index: Option<i32>,
    // The node's data blob; for files it carries the content `type`, used to pick
    // a format-specific icon in the drawer tree.
    pub data: Option<Jsonb>,
    pub mime: Option<MimeFields>,
    pub is_owner: Option<bool>,
    pub is_context_owner: Option<bool>,
    pub created_at: Option<Timestamptz>,
    // The parent node, for the search-result secondary line ("in <parent>").
    pub parent: Option<ParentNodeFields>,
}

/// A parent reduced to what a LIST needs: the name of the thing this sits under.
///
/// `ParentNodeFields` below carries the parent's whole document and its author's
/// avatar, which the feed needs because its rows are ABOUT their parent. A
/// search result only prints the parent's name, and paying a document per hit
/// for it is how thirty results became 147 KB.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct ParentNameRef {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct ParentNodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    // What the parent says, for a feed row that is ABOUT its parent: a reaction
    // shows the comment it is on, a reply the comment it answers.
    pub data: Option<Jsonb>,
    // And who wrote it. The computed field, not the `owner` relationship, so the
    // face still appears for a comment in a context the reader is not in.
    pub author_avatar: Option<String>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
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
    // Creating user (fallback label for questions/candidates/comments/amendments).
    pub owner: Option<UserRef>,
    // The author's name and avatar as computed fields, which see past the row
    // rule on `users` that `owner` above is subject to. That rule only reveals
    // someone you share a context with, so on the home page — which lists
    // content from public contexts you may not belong to — `owner` is null and
    // the post looked anonymous. These carry the name and picture only; the
    // email stays behind the users rule.
    pub author_name: Option<String>,
    pub author_avatar: Option<String>,
    // The parent node, for the "Newest" list's secondary line ("in <parent>"),
    // matching how search results show their context.
    pub parent: Option<ParentNodeFields>,
}

// --- Mime type ---
// Schema: context: Boolean!, hidden: Boolean!, icon: String!, id: String!, unique: Boolean!

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "mimes")]
pub struct MimeFields {
    pub id: String,
    pub icon: String,
    pub hidden: bool,
    pub context: bool,
}

// --- Context nodes (groups / events) for the home list ---

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct ContextNodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub created_at: Option<Timestamptz>,
    // A file's content `type`, so orphan file nodes (missing-parents app) show a
    // format-specific icon instead of the generic file glyph.
    pub data: Option<Jsonb>,
}

// --- Ordering (for the drawer child tree) ---

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "order_by",
    rename_all = "snake_case"
)]
pub enum OrderBy {
    Asc,
    AscNullsFirst,
    AscNullsLast,
    Desc,
    DescNullsFirst,
    DescNullsLast,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_order_by"
)]
pub struct NodesOrderBy {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub index: Option<OrderBy>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<OrderBy>,
    // A tiebreaker, so a paged query is deterministic. Rows created in the same
    // instant (a burst of reactions, an import) have no inherent order, and
    // Postgres is free to return them differently per query — which makes
    // offset paging repeat some rows and skip others.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<OrderBy>,
}

/// One drawer-tree row: just enough to render the node, plus a count of the
/// node's visible children so the expander chevron only shows when there is
/// something to expand (mirrors the React `DrawerElement`'s `children_aggregate`
/// gate). Kept separate from `ChildNodeFields` so the `$child_visible` variable
/// stays local to the drawer query.
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes",
    variables = "DrawerChildrenVariables"
)]
pub struct DrawerChildFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
    pub mutable: bool,
    pub data: Option<Jsonb>,
    #[cynic(rename = "children_aggregate")]
    #[arguments(where: $child_visible)]
    pub children_aggregate: NodesAggregate,
}

impl DrawerChildFields {
    /// Whether this node has at least one child visible to the caller — the gate
    /// for showing the drawer expander. Matches React's `childrenCount > 0`.
    pub fn has_children(&self) -> bool {
        self.children_aggregate
            .aggregate
            .as_ref()
            .map(|a| a.count)
            .unwrap_or(0)
            > 0
    }
}

/// Just an id — for walking a subtree, where nothing else is needed.
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct NodeIdFields {
    pub id: Uuid,
}

// --- Input types ---

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_bool_exp"
)]
pub struct NodesBoolExp {
    #[cynic(rename = "_and", skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<NodesBoolExp>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<UuidComparisonExp>,
    #[cynic(rename = "_or", skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<NodesBoolExp>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub key: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<StringComparisonExp>,
    // Full-text of the document body (Slate content extracted to plain text by a
    // Postgres generated column); lets search match inside content, not just the
    // title. Snake-case in the API (unlike the camelCased columns), so renamed.
    #[cynic(rename = "content_text", skip_serializing_if = "Option::is_none")]
    pub content_text: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<UuidComparisonExp>,
    /// The materialised ancestor path. `_eq` resolves a whole URL in one query;
    /// `_in` fetches a whole breadcrumb trail in one; `_like 'x/%'` is a subtree.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub path: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<UuidComparisonExp>,
    /// Every node above this one, by id. `_contains [x]` is "anywhere under x",
    /// exactly and without escaping — which a `path _like 'x/%'` is not, since
    /// keys contain the underscore that LIKE reads as a wildcard.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub ancestors: Option<UuidArrayComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub mime_id: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<UuidComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub mutable: Option<BooleanComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub members: Option<MembersBoolExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub mime: Option<MimesBoolExp>,
    // The node's context (nearest group/event). Boxed for the self-reference.
    // Used to keep the "Newest" list to contexts the user belongs to.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context: Option<Box<NodesBoolExp>>,
    #[cynic(rename = "_not", skip_serializing_if = "Option::is_none")]
    pub not: Option<Box<NodesBoolExp>>,
    // The node's parent row. `_not: { parent: {} }` is how an orphan is found:
    // `parent_id` still holds the deleted parent's id (there is no foreign key
    // on it), so the id is not null — the row it points at simply is not there.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Box<NodesBoolExp>>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mimes_bool_exp"
)]
pub struct MimesBoolExp {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<BooleanComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context: Option<BooleanComparisonExp>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "String_comparison_exp"
)]
pub struct StringComparisonExp {
    #[cynic(rename = "_eq", skip_serializing_if = "Option::is_none")]
    pub eq: Option<String>,
    #[cynic(rename = "_ilike", skip_serializing_if = "Option::is_none")]
    pub ilike: Option<String>,
    #[cynic(rename = "_in", skip_serializing_if = "Option::is_none")]
    pub in_: Option<Vec<String>>,
    #[cynic(rename = "_is_null", skip_serializing_if = "Option::is_none")]
    pub is_null: Option<bool>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "uuid_comparison_exp"
)]
pub struct UuidComparisonExp {
    #[cynic(rename = "_eq", skip_serializing_if = "Option::is_none")]
    pub eq: Option<Uuid>,
    #[cynic(rename = "_is_null", skip_serializing_if = "Option::is_none")]
    pub is_null: Option<bool>,
}

/// Comparison on a `uuid[]` column. Only containment is used: "is x among this
/// node's ancestors", which is the subtree test the feed rolls a group up with.
#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "uuid_array_comparison_exp"
)]
pub struct UuidArrayComparisonExp {
    #[cynic(rename = "_contains", skip_serializing_if = "Option::is_none")]
    pub contains: Option<Vec<Uuid>>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "Boolean_comparison_exp"
)]
pub struct BooleanComparisonExp {
    #[cynic(rename = "_eq", skip_serializing_if = "Option::is_none")]
    pub eq: Option<bool>,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_insert_input"
)]
pub struct NodesInsertInput {
    // Unset fields must be omitted, not sent as `null`: Hasura rejects an
    // explicit null for the non-null columns (e.g. `mutable`), which silently
    // broke every insert that left a field unset (votes, speaker entries, …).
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub key: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub mime_id: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub data: Option<Jsonb>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub mutable: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    // Only a copy sets this: it carries the original's date so a pasted node
    // keeps its age instead of surfacing as the newest thing in the folder.
    // Left unset everywhere else, where the column default (now()) is right.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamptz>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "relations_bool_exp"
)]
pub struct RelationsBoolExp {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<UuidComparisonExp>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "relations")]
pub struct RelationRef {
    pub id: Uuid,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_pk_columns_input"
)]
pub struct NodesPkColumnsInput {
    pub id: Uuid,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_set_input"
)]
pub struct NodesSetInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub data: Option<Jsonb>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub mutable: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub index: Option<i32>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub attachable: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub created_at: Option<Timestamptz>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_aggregate_fields"
)]
pub struct NodesAggregateFields {
    pub count: i32,
}
