use cynic::QueryBuilder;
use serde::{Deserialize, Serialize};

use crate::nhost::graphql_url;

mod schema {
    cynic::use_schema!("graphql/schema.graphql");
}

// Custom scalar types
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, Hash, Eq)]
pub struct Uuid(pub String);
cynic::impl_scalar!(Uuid, schema::uuid);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Timestamptz(pub String);
cynic::impl_scalar!(Timestamptz, schema::timestamptz);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Jsonb(pub serde_json::Value);
cynic::impl_scalar!(Jsonb, schema::jsonb);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bigint(pub String);
cynic::impl_scalar!(Bigint, schema::bigint);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Bytea(pub String);
cynic::impl_scalar!(Bytea, schema::bytea);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Citext(pub String);
cynic::impl_scalar!(Citext, schema::citext);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Text(pub String);
cynic::impl_scalar!(Text, schema::_text);

// --- Query: Fetch a single node by ID ---

#[derive(cynic::QueryVariables, Debug)]
pub struct NodeByIdVariables {
    pub id: Uuid,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodeByIdVariables"
)]
pub struct NodeByIdQuery {
    #[arguments(id: $id)]
    pub node: Option<NodeFields>,
}

// --- Query: a node's allowed child mimes (the `inserts` computed field) ---

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodeByIdVariables"
)]
pub struct NodeInsertsQuery {
    #[arguments(id: $id)]
    pub node: Option<NodeInserts>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct NodeInserts {
    pub inserts: Option<Vec<InsertMime>>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "mimes")]
pub struct InsertMime {
    pub id: String,
}

// --- Query: Fetch nodes with a where filter ---

#[derive(cynic::QueryVariables, Debug)]
pub struct NodesWhereVariables {
    pub where_clause: NodesBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesWhereVariables"
)]
pub struct NodesWhereQuery {
    #[arguments(where: $where_clause)]
    pub nodes: Vec<NodeFields>,
}

// --- Node fields (basic — no children, no data) ---

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
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

// --- Node with children ---

#[derive(cynic::QueryVariables, Debug)]
pub struct NodeWithChildrenVariables {
    pub id: Uuid,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodeWithChildrenVariables"
)]
pub struct NodeWithChildrenQuery {
    #[arguments(id: $id)]
    pub node: Option<NodeWithChildren>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
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
    // Backend-computed permission flags: `is_owner` = the session user owns this
    // node; `is_context_owner` = they own its context. Drive owner-only UI gating.
    pub is_owner: Option<bool>,
    pub is_context_owner: Option<bool>,
    // Whether children may be added (the folder "lock"; owner-toggleable).
    pub attachable: bool,
    pub created_at: Option<Timestamptz>,
    // The creating user (fallback author label when no explicit author chip).
    pub owner: Option<UserRef>,
}

/// A membership row on a node — used as the author chips on documents and the
/// member list of a context (mirrors the React `MemberChips`).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "members")]
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

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "users")]
pub struct UserRef {
    pub display_name: String,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct MemberNodeRef {
    pub mime_id: Option<String>,
}

impl MemberFields {
    /// The display label for a member: their explicit name, else the linked
    /// user's display name, else their email-less fallback.
    pub fn label(&self) -> String {
        self.name
            .clone()
            .filter(|n| !n.is_empty())
            .or_else(|| self.user.as_ref().map(|u| u.display_name.clone()))
            .unwrap_or_default()
    }
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct ParentNodeFields {
    pub id: Uuid,
    pub name: String,
    pub key: String,
    pub mime_id: Option<String>,
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

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesWhereVariables"
)]
pub struct ContextsWhereQuery {
    #[arguments(where: $where_clause)]
    pub nodes: Vec<ContextNodeFields>,
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
}

// --- Query: children of a node, filtered + ordered (drawer MenuList) ---

#[derive(cynic::QueryVariables, Debug)]
pub struct ChildrenVariables {
    pub where_clause: NodesBoolExp,
    pub order_by: Option<Vec<NodesOrderBy>>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "ChildrenVariables"
)]
pub struct ChildrenQuery {
    #[arguments(where: $where_clause, order_by: $order_by)]
    pub nodes: Vec<ChildNodeFields>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct DrawerChildrenVariables {
    pub where_clause: NodesBoolExp,
    pub order_by: Option<Vec<NodesOrderBy>>,
    // Filter for the per-row `children_aggregate` count, so the drawer expander
    // only appears for nodes that actually have children the user can see.
    pub child_visible: NodesBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "DrawerChildrenVariables"
)]
pub struct DrawerChildrenQuery {
    #[arguments(where: $where_clause, order_by: $order_by)]
    pub nodes: Vec<DrawerChildFields>,
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

// --- Query: recent nodes across the user's contexts (home "Newest") ---

#[derive(cynic::QueryVariables, Debug)]
pub struct RecentNodesVariables {
    pub where_clause: NodesBoolExp,
    pub order_by: Option<Vec<NodesOrderBy>>,
    pub limit: Option<i32>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "RecentNodesVariables"
)]
pub struct RecentNodesQuery {
    #[arguments(where: $where_clause, order_by: $order_by, limit: $limit)]
    pub nodes: Vec<ChildNodeFields>,
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
    #[cynic(rename = "_or", skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<NodesBoolExp>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub key: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<UuidComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<UuidComparisonExp>,
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
    graphql_type = "members_bool_exp"
)]
pub struct MembersBoolExp {
    #[cynic(rename = "_and", skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<MembersBoolExp>>,
    #[cynic(rename = "_or", skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<MembersBoolExp>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<BooleanComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub active: Option<BooleanComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub email: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<UuidComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<UuidComparisonExp>,
    // Boxed to break the NodesBoolExp <-> MembersBoolExp type cycle.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent: Option<Box<NodesBoolExp>>,
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

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "Boolean_comparison_exp"
)]
pub struct BooleanComparisonExp {
    #[cynic(rename = "_eq", skip_serializing_if = "Option::is_none")]
    pub eq: Option<bool>,
}

// --- Mutations ---

#[derive(cynic::QueryVariables, Debug)]
pub struct InsertNodeVariables {
    pub object: NodesInsertInput,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "InsertNodeVariables"
)]
pub struct InsertNodeMutation {
    #[arguments(object: $object)]
    pub insert_node: Option<InsertedNode>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct InsertedNode {
    pub id: Uuid,
    pub key: String,
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
}

// --- Relations (the context's "active" node, e.g. the live poll) ---

#[derive(cynic::QueryVariables, Debug)]
pub struct RelationsWhereVariables {
    pub where_clause: RelationsBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "RelationsWhereVariables"
)]
pub struct RelationsQuery {
    #[arguments(where: $where_clause)]
    pub relations: Vec<RelationFields>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "relations")]
pub struct RelationFields {
    pub name: String,
    pub node_id: Option<Uuid>,
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

/// The id of a context's "active" node — during a vote this is the open poll,
/// mirroring the React VoteApp's `get("active")`.
pub async fn active_node_id(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Option<String>, String> {
    let where_clause = RelationsBoolExp {
        name: Some(StringComparisonExp {
            eq: Some("active".to_string()),
            ..Default::default()
        }),
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
    };
    let operation = RelationsQuery::build(RelationsWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result
        .relations
        .into_iter()
        .find_map(|r| r.node_id.map(|n| n.0)))
}

// --- Set the context's `active` relation (upsert, keyed on parentId+name) ---

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "relations_constraint",
    rename_all = "snake_case"
)]
pub enum RelationsConstraint {
    RelationsParentIdNameKey,
    RelationsPkey,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "relations_update_column",
    rename_all = "camelCase"
)]
pub enum RelationsUpdateColumn {
    Id,
    Name,
    NodeId,
    ParentId,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "relations_on_conflict"
)]
pub struct RelationsOnConflict {
    pub constraint: RelationsConstraint,
    // Hasura's on_conflict meta-field stays snake_case (unlike the camelCase
    // column fields), so keep cynic from rewriting it to `updateColumns`.
    #[cynic(rename = "update_columns")]
    pub update_columns: Vec<RelationsUpdateColumn>,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "relations_insert_input"
)]
pub struct RelationsInsertInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct InsertRelationVariables {
    pub object: RelationsInsertInput,
    pub on_conflict: RelationsOnConflict,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "InsertRelationVariables"
)]
pub struct InsertRelationMutation {
    #[arguments(object: $object, on_conflict: $on_conflict)]
    pub insert_relation: Option<RelationRef>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "relations")]
pub struct RelationRef {
    pub id: Uuid,
}

/// Upsert the context's `active` relation to point at `node_id` (keyed on
/// parentId+name so it replaces any prior active). Mirrors `contextSet("active")`.
pub async fn set_active_relation(
    access_token: Option<&str>,
    context_id: &str,
    node_id: Option<&str>,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let object = RelationsInsertInput {
        name: Some("active".to_string()),
        node_id: node_id.map(|n| Uuid(n.to_string())),
        parent_id: Some(Uuid(context_id.to_string())),
    };
    let on_conflict = RelationsOnConflict {
        constraint: RelationsConstraint::RelationsParentIdNameKey,
        update_columns: vec![RelationsUpdateColumn::NodeId],
    };
    let operation = InsertRelationMutation::build(InsertRelationVariables {
        object,
        on_conflict,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_relation.is_some())
}

/// Open a poll on `parent_id` (a policy/change/position): close any prior active
/// poll, insert a `vote/poll` node with the ballot config, and set the context's
/// `active` relation to it. Mirrors React's PollDialog.
#[allow(clippy::too_many_arguments)]
pub async fn create_poll(
    access_token: Option<&str>,
    parent_id: &str,
    context_id: &str,
    name: &str,
    key: &str,
    options: &[String],
    min_vote: usize,
    max_vote: usize,
    hidden: bool,
) -> Result<InsertedNode, String> {
    // Close the context's current active poll, if any (only one is open at once).
    if let Ok(Some(prior)) = active_node_id(access_token, context_id).await {
        let _ = update_node(
            access_token,
            &prior,
            NodesSetInput {
                mutable: Some(false),
                ..Default::default()
            },
        )
        .await;
    }
    let data = serde_json::json!({
        "options": options,
        "minVote": min_vote,
        "maxVote": max_vote,
        "hidden": hidden,
        "nodeId": parent_id,
    });
    let inserted = insert_node(
        access_token,
        NodesInsertInput {
            name: Some(name.to_string()),
            key: Some(key.to_string()),
            mime_id: Some("vote/poll".to_string()),
            parent_id: Some(Uuid(parent_id.to_string())),
            context_id: Some(Uuid(context_id.to_string())),
            data: Some(Jsonb(data)),
            mutable: Some(true),
            index: None,
        },
    )
    .await?
    .ok_or_else(|| "poll insert returned no node".to_string())?;
    set_active_relation(access_token, context_id, Some(&inserted.id.0)).await?;
    Ok(inserted)
}

// --- Invitations (pending memberships on groups / events) ---

#[derive(cynic::QueryVariables, Debug)]
pub struct MembersWhereVariables {
    pub where_clause: MembersBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "MembersWhereVariables"
)]
pub struct InvitationsQuery {
    #[arguments(where: $where_clause)]
    pub members: Vec<InvitationFields>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "members")]
pub struct InvitationFields {
    pub id: Uuid,
    pub parent: Option<ParentNodeFields>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateMemberVariables {
    pub pk: MembersPkColumnsInput,
    pub set: MembersSetInput,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "UpdateMemberVariables"
)]
pub struct UpdateMemberMutation {
    #[arguments(pk_columns: $pk, _set: $set)]
    pub update_member: Option<UpdatedMember>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "members")]
pub struct UpdatedMember {
    pub id: Uuid,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_pk_columns_input"
)]
pub struct MembersPkColumnsInput {
    pub id: Uuid,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_set_input"
)]
pub struct MembersSetInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub active: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub owner: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteMemberVariables {
    pub id: Uuid,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "DeleteMemberVariables"
)]
pub struct DeleteMemberMutation {
    #[arguments(id: $id)]
    pub delete_member: Option<UpdatedMember>,
}

// --- Invite a member (by email) to a context ---

#[derive(cynic::QueryVariables, Debug)]
pub struct InsertMemberVariables {
    pub object: MembersInsertInput,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "InsertMemberVariables"
)]
pub struct InsertMemberMutation {
    #[arguments(object: $object)]
    pub insert_member: Option<UpdatedMember>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_insert_input"
)]
pub struct MembersInsertInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub email: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct InsertMembersVariables {
    pub objects: Vec<MembersInsertInput>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "InsertMembersVariables"
)]
pub struct InsertMembersMutation {
    #[arguments(objects: $objects)]
    pub insert_members: Option<MembersAffected>,
}

/// Bulk-invite members from an imported roster: one email invite per `(name,
/// email)` pair in a single `insertMembers`. Mirrors React InvitesFab's bulk
/// insert. Returns how many rows were inserted.
pub async fn invite_members(
    access_token: Option<&str>,
    parent_id: &str,
    roster: &[(String, String)],
) -> Result<usize, String> {
    use cynic::MutationBuilder;
    let objects: Vec<MembersInsertInput> = roster
        .iter()
        .filter(|(_, email)| !email.trim().is_empty())
        .map(|(name, email)| MembersInsertInput {
            name: (!name.trim().is_empty()).then(|| name.clone()),
            email: Some(email.to_lowercase()),
            parent_id: Some(Uuid(parent_id.to_string())),
            ..Default::default()
        })
        .collect();
    if objects.is_empty() {
        return Ok(0);
    }
    let op = InsertMembersMutation::build(InsertMembersVariables { objects });
    let r = execute(access_token, op).await?;
    Ok(r.insert_members
        .map(|m| m.affected_rows.max(0) as usize)
        .unwrap_or(0))
}

/// A server-side page filter for a node's members: each `Option<bool>` narrows the
/// query when `Some`, plus a free-text `search` matched case-insensitively against
/// name or email. Plain data so the UI never has to touch GraphQL.
#[derive(Default, Clone, PartialEq)]
pub struct MemberPageFilter {
    pub owner: Option<bool>,
    pub active: Option<bool>,
    pub accepted: Option<bool>,
    pub hidden: Option<bool>,
    pub search: String,
}

/// Fetch one page of a node's members from Hasura with server-side filtering,
/// search and pagination (`limit`/`offset`), plus the total count for the same
/// predicate — so the roster scales to thousands without loading them all.
/// Returns `(page rows, total matching)`.
pub async fn query_members_page(
    access_token: Option<&str>,
    parent_id: &str,
    filter: &MemberPageFilter,
    limit: usize,
    offset: usize,
) -> Result<(Vec<MemberFields>, usize), String> {
    let where_clause = members_where(parent_id, filter);
    let query = format!(
        "query {{ \
           members(where: {w}, order_by: {{ name: asc }}, limit: {limit}, offset: {offset}) {{ \
             id name email accepted active owner hidden nodeId \
             user {{ displayName }} node {{ mimeId }} \
           }} \
           members_aggregate(where: {w}) {{ aggregate {{ count }} }} \
         }}",
        w = where_clause,
    );
    let data = execute_raw(access_token, &query).await?;
    let rows = data
        .get("members")
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().filter_map(parse_member_row).collect())
        .unwrap_or_default();
    let total = data
        .get("members_aggregate")
        .and_then(|a| a.get("aggregate"))
        .and_then(|a| a.get("count"))
        .and_then(|c| c.as_u64())
        .unwrap_or(0) as usize;
    Ok((rows, total))
}

/// Build the Hasura `where` object (as GraphQL literal text) for a member page.
fn members_where(parent_id: &str, f: &MemberPageFilter) -> String {
    let mut parts = vec![format!(
        "parentId: {{ _eq: \"{}\" }}",
        gql_escape(parent_id)
    )];
    for (col, val) in [
        ("owner", f.owner),
        ("active", f.active),
        ("accepted", f.accepted),
        ("hidden", f.hidden),
    ] {
        if let Some(v) = val {
            parts.push(format!("{col}: {{ _eq: {v} }}"));
        }
    }
    let q = f.search.trim();
    if !q.is_empty() {
        let pat = format!("%{}%", gql_escape(q));
        parts.push(format!(
            "_or: [{{ name: {{ _ilike: \"{pat}\" }} }}, {{ email: {{ _ilike: \"{pat}\" }} }}]"
        ));
    }
    format!("{{ {} }}", parts.join(", "))
}

/// Escape a value for embedding inside a GraphQL double-quoted string literal.
fn gql_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
}

/// Parse one raw `members` JSON row into a [`MemberFields`].
fn parse_member_row(v: &serde_json::Value) -> Option<MemberFields> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    Some(MemberFields {
        id: Uuid(v.get("id")?.as_str()?.to_string()),
        name: s("name"),
        email: s("email"),
        accepted: b("accepted"),
        active: b("active"),
        owner: b("owner"),
        hidden: b("hidden"),
        node_id: s("nodeId").map(Uuid),
        user: v
            .get("user")
            .and_then(|u| u.get("displayName"))
            .and_then(|d| d.as_str())
            .map(|s| UserRef {
                display_name: s.to_string(),
            }),
        node: v.get("node").map(|n| MemberNodeRef {
            mime_id: n.get("mimeId").and_then(|m| m.as_str()).map(String::from),
        }),
    })
}

/// Invite someone to a context by email (a pending membership they accept from
/// their home screen). Mirrors the React invite: email set, no node id yet.
pub async fn invite_member(
    access_token: Option<&str>,
    parent_id: &str,
    email: &str,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = InsertMemberMutation::build(InsertMemberVariables {
        object: MembersInsertInput {
            email: Some(email.to_string()),
            parent_id: Some(Uuid(parent_id.to_string())),
            ..Default::default()
        },
    });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_member.is_some())
}

/// Filter for the home invitations list: pending (accepted=false) memberships on
/// a group or event that belong to this user (by node id or invited email).
fn invitations_where_clause(user_id: &str, email: &str) -> MembersBoolExp {
    MembersBoolExp {
        and: Some(vec![
            MembersBoolExp {
                accepted: Some(BooleanComparisonExp { eq: Some(false) }),
                ..Default::default()
            },
            MembersBoolExp {
                or: Some(vec![
                    MembersBoolExp {
                        node_id: Some(UuidComparisonExp {
                            eq: Some(Uuid(user_id.to_string())),
                            is_null: None,
                        }),
                        ..Default::default()
                    },
                    MembersBoolExp {
                        email: Some(StringComparisonExp {
                            eq: Some(email.to_string()),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            },
            MembersBoolExp {
                parent: Some(Box::new(NodesBoolExp {
                    or: Some(vec![
                        NodesBoolExp {
                            mime_id: Some(StringComparisonExp {
                                eq: Some("wiki/group".to_string()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                        NodesBoolExp {
                            mime_id: Some(StringComparisonExp {
                                eq: Some("wiki/event".to_string()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                })),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

/// The user's pending group/event invitations.
pub async fn query_invitations(
    access_token: Option<&str>,
    user_id: &str,
    email: &str,
) -> Result<Vec<InvitationFields>, String> {
    let where_clause = invitations_where_clause(user_id, email);
    let operation = InvitationsQuery::build(MembersWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result
        .members
        .into_iter()
        .filter(|m| m.parent.is_some())
        .collect())
}

/// Accept an invitation: mark the membership accepted and bind it to the user.
pub async fn accept_invitation(
    access_token: Option<&str>,
    member_id: &str,
    user_id: &str,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = UpdateMemberMutation::build(UpdateMemberVariables {
        pk: MembersPkColumnsInput {
            id: Uuid(member_id.to_string()),
        },
        set: MembersSetInput {
            accepted: Some(true),
            node_id: Some(Uuid(user_id.to_string())),
            ..Default::default()
        },
    });
    let result = execute(access_token, operation).await?;
    Ok(result.update_member.is_some())
}

/// Hide or unhide a member in its context (#51). Owners use this to keep a
/// user's membership private within a group without removing them.
pub async fn set_member_hidden(
    access_token: Option<&str>,
    member_id: &str,
    hidden: bool,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = UpdateMemberMutation::build(UpdateMemberVariables {
        pk: MembersPkColumnsInput {
            id: Uuid(member_id.to_string()),
        },
        set: MembersSetInput {
            hidden: Some(hidden),
            ..Default::default()
        },
    });
    let result = execute(access_token, operation).await?;
    Ok(result.update_member.is_some())
}

/// Decline an invitation by deleting the membership row.
pub async fn decline_invitation(
    access_token: Option<&str>,
    member_id: &str,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = DeleteMemberMutation::build(DeleteMemberVariables {
        id: Uuid(member_id.to_string()),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.delete_member.is_some())
}

/// Update a member row by primary key (owner/active/name/email/hidden). The
/// owner-only member admin uses this to promote/demote, (de)activate, or rename
/// a member, mirroring React's editable MembersDataGrid.
pub async fn update_member(
    access_token: Option<&str>,
    member_id: &str,
    set: MembersSetInput,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = UpdateMemberMutation::build(UpdateMemberVariables {
        pk: MembersPkColumnsInput {
            id: Uuid(member_id.to_string()),
        },
        set,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.update_member.is_some())
}

/// Remove a member from a context (owner action) — the same mutation as
/// declining an invitation, named for the admin use.
pub async fn remove_member(access_token: Option<&str>, member_id: &str) -> Result<bool, String> {
    decline_invitation(access_token, member_id).await
}

// --- Update mutation ---

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateNodeVariables {
    pub pk: NodesPkColumnsInput,
    pub set: NodesSetInput,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "UpdateNodeVariables"
)]
pub struct UpdateNodeMutation {
    #[arguments(pk_columns: $pk, _set: $set)]
    pub update_node: Option<UpdatedNode>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct UpdatedNode {
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

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteNodeVariables {
    pub id: Uuid,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "DeleteNodeVariables"
)]
pub struct DeleteNodeMutation {
    #[arguments(id: $id)]
    pub delete_node: Option<DeletedNode>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct DeletedNode {
    pub id: Uuid,
}

// --- HTTP execution ---

/// A Hasura error whose JWT is expired/invalid, so refreshing the token and
/// retrying the request may recover (e.g. "Could not verify JWT: JWTExpired").
fn is_jwt_error(msg: &str) -> bool {
    msg.to_ascii_lowercase().contains("jwt")
}

async fn execute_once<Q, V>(
    access_token: Option<&str>,
    operation: &cynic::Operation<Q, V>,
) -> Result<Q, String>
where
    Q: serde::de::DeserializeOwned + 'static,
    V: serde::Serialize,
{
    let client = reqwest::Client::new();
    let mut req = client.post(graphql_url());

    if let Some(token) = access_token {
        req = req.bearer_auth(token);
    }

    let resp = req
        .json(operation)
        .send()
        .await
        .map_err(|e| e.to_string())?;

    let body: cynic::GraphQlResponse<Q> = resp.json().await.map_err(|e| e.to_string())?;

    if let Some(errors) = body.errors {
        let msgs: Vec<String> = errors.into_iter().map(|e| e.message).collect();
        return Err(msgs.join(", "));
    }

    body.data.ok_or_else(|| "No data returned".to_string())
}

pub async fn execute<Q, V>(
    access_token: Option<&str>,
    operation: cynic::Operation<Q, V>,
) -> Result<Q, String>
where
    Q: serde::de::DeserializeOwned + 'static,
    V: serde::Serialize,
{
    match execute_once(access_token, &operation).await {
        Err(msg) if is_jwt_error(&msg) => {
            // The token likely lapsed (e.g. the tab was backgrounded past expiry).
            // Refresh once and retry with the new token before surfacing the error
            // so a returning tab recovers instead of showing a JWT error.
            match crate::session::ensure_fresh_token().await {
                Some(fresh) if Some(fresh.as_str()) != access_token => {
                    execute_once(Some(&fresh), &operation).await
                }
                _ => Err(msg),
            }
        }
        other => other,
    }
}

async fn execute_raw_once(
    access_token: Option<&str>,
    query: &str,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let mut req = client.post(graphql_url());

    if let Some(token) = access_token {
        req = req.bearer_auth(token);
    }

    let body = serde_json::json!({ "query": query });
    let resp = req.json(&body).send().await.map_err(|e| e.to_string())?;

    let result: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    if let Some(errors) = result.get("errors") {
        return Err(errors.to_string());
    }

    Ok(result.get("data").cloned().unwrap_or_default())
}

/// Execute a raw GraphQL query/mutation string (for operations not covered by
/// cynic types), with the same JWT refresh-and-retry as [`execute`].
pub async fn execute_raw(
    access_token: Option<&str>,
    query: &str,
) -> Result<serde_json::Value, String> {
    match execute_raw_once(access_token, query).await {
        Err(msg) if is_jwt_error(&msg) => match crate::session::ensure_fresh_token().await {
            Some(fresh) if Some(fresh.as_str()) != access_token => {
                execute_raw_once(Some(&fresh), query).await
            }
            _ => Err(msg),
        },
        other => other,
    }
}

// --- High-level query functions ---

pub async fn query_node_by_key(
    access_token: Option<&str>,
    key: &str,
    parent_id: Option<&str>,
) -> Result<Option<NodeFields>, String> {
    let where_clause = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                key: Some(StringComparisonExp {
                    eq: Some(key.to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            NodesBoolExp {
                parent_id: Some(match parent_id {
                    Some(id) => UuidComparisonExp {
                        eq: Some(Uuid(id.to_string())),
                        is_null: None,
                    },
                    None => UuidComparisonExp {
                        eq: None,
                        is_null: Some(true),
                    },
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };

    let operation = NodesWhereQuery::build(NodesWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes.into_iter().next())
}

pub async fn query_node_by_id(
    access_token: Option<&str>,
    id: &str,
) -> Result<Option<NodeWithChildren>, String> {
    let operation = NodeWithChildrenQuery::build(NodeWithChildrenVariables {
        id: Uuid(id.to_string()),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.node)
}

/// Id of the single parent-less root node ("Hjem", key "root"). Paths are
/// resolved relative to it: the root's key is not part of any URL path.
async fn query_root_id(access_token: Option<&str>) -> Result<Option<String>, String> {
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            is_null: Some(true),
            eq: None,
        }),
        ..Default::default()
    };
    let operation = NodesWhereQuery::build(NodesWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes.into_iter().next().map(|n| n.id.0))
}

/// The parent-less root node ("Hjem"), whose `data.content` backs the editable
/// welcome page. `resolve_path(&[])` returns `None` (the root's key is not a path
/// segment), so fetch it via the root id directly.
pub async fn query_root_node(
    access_token: Option<&str>,
) -> Result<Option<NodeWithChildren>, String> {
    let Some(root_id) = query_root_id(access_token).await? else {
        return Ok(None);
    };
    query_node_by_id(access_token, &root_id).await
}

pub async fn resolve_path(
    access_token: Option<&str>,
    segments: &[String],
) -> Result<Option<NodeWithChildren>, String> {
    // Path segments are keys of nodes below the root, so start the walk at the
    // root node rather than at the (parent-less) top level.
    let Some(root_id) = query_root_id(access_token).await? else {
        return Ok(None);
    };
    let mut parent_id: Option<String> = Some(root_id);
    let mut last_node_id: Option<String> = None;

    for segment in segments {
        let found = query_node_by_key(access_token, segment, parent_id.as_deref()).await?;
        match found {
            Some(n) => {
                last_node_id = Some(n.id.0.clone());
                parent_id = Some(n.id.0);
            }
            None => return Ok(None),
        }
    }

    if let Some(id) = last_node_id {
        return query_node_by_id(access_token, &id).await;
    }

    Ok(None)
}

/// A resolved breadcrumb segment: its display name, mime, and (for policy /
/// change nodes) its 0-based ordinal among same-type siblings, so the crumb
/// avatar can show the same letter/number label as elsewhere.
#[derive(Debug, Clone, PartialEq)]
pub struct Crumb {
    pub name: String,
    pub mime_id: Option<String>,
    pub ordinal: Option<usize>,
    // A file crumb's content `type`, so it shows a format-specific icon.
    pub data: Option<Jsonb>,
}

/// How many leading path segments belong to the current node's context: the
/// depth of the deepest group/event in the crumbs (the node's `contextId` in the
/// React app). 0 means the path has no context (the groups/events home applies).
pub fn deepest_context_depth(crumbs: &[Crumb]) -> usize {
    crumbs
        .iter()
        .rposition(|c| matches!(c.mime_id.as_deref(), Some("wiki/group" | "wiki/event")))
        .map(|i| i + 1)
        .unwrap_or(0)
}

/// Resolve each path segment to its `(name, mime_id)`, walking from the root like
/// `resolve_path`. Feeds the breadcrumb trail its per-segment avatar + name.
pub async fn path_crumbs(
    access_token: Option<&str>,
    segments: &[String],
) -> Result<Vec<Crumb>, String> {
    let Some(root_id) = query_root_id(access_token).await? else {
        return Ok(segments
            .iter()
            .map(|s| Crumb {
                name: s.clone(),
                mime_id: None,
                ordinal: None,
                data: None,
            })
            .collect());
    };
    let mut parent_id: Option<String> = Some(root_id);
    let mut out = Vec::with_capacity(segments.len());
    for segment in segments {
        match query_node_by_key(access_token, segment, parent_id.as_deref()).await? {
            Some(n) => {
                // getIndex is 1-based; node_avatar wants a 0-based ordinal.
                let ordinal = n.get_index.filter(|i| *i >= 1).map(|i| (i - 1) as usize);
                out.push(Crumb {
                    name: n.name.clone(),
                    mime_id: n.mime_id.clone(),
                    ordinal,
                    data: n.data.clone(),
                });
                parent_id = Some(n.id.0);
            }
            None => {
                out.push(Crumb {
                    name: segment.clone(),
                    mime_id: None,
                    ordinal: None,
                    data: None,
                });
                break;
            }
        }
    }
    Ok(out)
}

/// Insert a node
pub async fn insert_node(
    access_token: Option<&str>,
    input: NodesInsertInput,
) -> Result<Option<InsertedNode>, String> {
    use cynic::MutationBuilder;
    let operation = InsertNodeMutation::build(InsertNodeVariables { object: input });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_node)
}

/// Recursively deep-copy a node and its whole subtree (data + members) under
/// `parent_id`. Mirrors React FolderDial's copy: fetch node + children + members,
/// insert the copy, copy the members, then recurse into each child. The root
/// copy's key is suffixed for uniqueness (it may land beside its source); child
/// keys are kept verbatim (they sit under a fresh parent, so no collision).
/// Boxed + owned args so the recursive async future is `'static`.
pub fn deep_copy_node(
    access_token: Option<String>,
    copy_id: String,
    parent_id: String,
    context_id: Option<String>,
    is_root: bool,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>>>> {
    Box::pin(async move {
        let node = match query_node_by_id(access_token.as_deref(), &copy_id).await? {
            Some(n) => n,
            None => return Ok(()),
        };
        let key = if is_root {
            format!("{}-{}", node.key, (js_sys::Date::now() as u64) % 1_000_000)
        } else {
            node.key.clone()
        };
        let input = NodesInsertInput {
            name: Some(node.name.clone()),
            key: Some(key),
            mime_id: node.mime_id.clone(),
            parent_id: Some(Uuid(parent_id.clone())),
            context_id: context_id.clone().map(Uuid),
            data: node.data.clone(),
            mutable: Some(node.mutable),
            index: Some(node.index),
        };
        let new_id = match insert_node(access_token.as_deref(), input).await? {
            Some(inserted) => inserted.id.0,
            None => return Ok(()),
        };
        // Copy the members onto the new node.
        for m in &node.members {
            use cynic::MutationBuilder;
            let object = MembersInsertInput {
                name: m.name.clone(),
                email: m.email.clone(),
                node_id: m.node_id.clone(),
                parent_id: Some(Uuid(new_id.clone())),
            };
            let op = InsertMemberMutation::build(InsertMemberVariables { object });
            let _ = execute(access_token.as_deref(), op).await;
        }
        // Recurse over the children (keys kept verbatim under the fresh parent).
        for child in &node.children {
            deep_copy_node(
                access_token.clone(),
                child.id.0.clone(),
                new_id.clone(),
                context_id.clone(),
                false,
            )
            .await?;
        }
        Ok(())
    })
}

/// Whether `target` is `ancestor` itself or a descendant of it, by walking up the
/// parent chain. Blocks pasting a folder into itself or its own subtree (which
/// would recurse forever). Depth-bounded as a safety net.
pub async fn is_descendant_of(access_token: Option<&str>, target: &str, ancestor: &str) -> bool {
    let mut cur = Some(target.to_string());
    let mut guard = 0;
    while let Some(id) = cur {
        if id == ancestor {
            return true;
        }
        guard += 1;
        if guard > 64 {
            break;
        }
        cur = query_node_by_id(access_token, &id)
            .await
            .ok()
            .flatten()
            .and_then(|n| n.parent_id.map(|p| p.0));
    }
    false
}

/// Update a node's mutable columns (name / data / mutable / index). The jsonb
/// `data` is passed as a GraphQL variable, not inlined (inlining a JSON object
/// into the mutation string is invalid GraphQL and silently failed).
pub async fn update_node(
    access_token: Option<&str>,
    id: &str,
    set: NodesSetInput,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = UpdateNodeMutation::build(UpdateNodeVariables {
        pk: NodesPkColumnsInput {
            id: Uuid(id.to_string()),
        },
        set,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.update_node.is_some())
}

/// Delete a node by ID
pub async fn delete_node(access_token: Option<&str>, id: &str) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = DeleteNodeMutation::build(DeleteNodeVariables {
        id: Uuid(id.to_string()),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.delete_node.is_some())
}

/// How many votes the given user has already cast on a poll (used to show the
/// "you have voted" state and hide the ballot). Own votes are visible to the
/// voter via row permissions.
pub async fn count_user_votes(
    access_token: Option<&str>,
    poll_id: &str,
    user_id: &str,
) -> Result<usize, String> {
    let where_clause = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                parent_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(poll_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    eq: Some("vote/vote".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            NodesBoolExp {
                owner_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(user_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    let operation = NodesWhereQuery::build(NodesWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes.len())
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesWhereVariables"
)]
pub struct VotesWhereQuery {
    #[arguments(where: $where_clause)]
    pub nodes: Vec<VoteNodeFields>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct VoteNodeFields {
    pub id: Uuid,
    pub data: Option<Jsonb>,
}

// --- Permissions of a context (the perm view) ---

#[derive(cynic::QueryVariables, Debug)]
pub struct PermissionsWhereVariables {
    pub where_clause: PermissionsBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "PermissionsWhereVariables"
)]
pub struct PermissionsQuery {
    #[arguments(where: $where_clause)]
    pub permissions: Vec<PermissionFields>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "permissions")]
pub struct PermissionFields {
    pub id: Uuid,
    pub mime_id: Option<String>,
    pub role: String,
    pub insert: bool,
    pub select: bool,
    pub delete: bool,
    pub active: bool,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "permissions_bool_exp"
)]
pub struct PermissionsBoolExp {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<UuidComparisonExp>,
}

/// The permission rows configured on a context, for the perm overview.
pub async fn query_permissions(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Vec<PermissionFields>, String> {
    let where_clause = PermissionsBoolExp {
        context_id: Some(UuidComparisonExp {
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
    };
    let operation = PermissionsQuery::build(PermissionsWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.permissions)
}

// --- Polls of a context (the admin results overview) ---

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesWhereVariables"
)]
pub struct PollsWhereQuery {
    #[arguments(where: $where_clause)]
    pub nodes: Vec<PollSummaryFields>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct PollSummaryFields {
    pub id: Uuid,
    pub name: String,
    pub data: Option<Jsonb>,
    pub created_at: Option<Timestamptz>,
}

/// Every `vote/poll` in a context, newest first — for the admin results grid.
pub async fn query_context_polls(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Vec<PollSummaryFields>, String> {
    let where_clause = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                context_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(context_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    eq: Some("vote/poll".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    let operation = PollsWhereQuery::build(NodesWhereVariables { where_clause });
    let mut result = execute(access_token, operation).await?;
    result.nodes.sort_by(|a, b| {
        let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        b_ts.cmp(a_ts)
    });
    Ok(result.nodes)
}

/// Every vote cast on a poll (each as its list of selected option indices),
/// for tallying results. Visibility follows row permissions.
pub async fn query_poll_votes(
    access_token: Option<&str>,
    poll_id: &str,
) -> Result<Vec<Vec<usize>>, String> {
    let where_clause = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                parent_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(poll_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    eq: Some("vote/vote".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    let operation = VotesWhereQuery::build(NodesWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result
        .nodes
        .into_iter()
        .map(|n| {
            n.data
                .and_then(|d| d.0.as_array().cloned())
                .map(|arr| {
                    arr.iter()
                        .filter_map(|v| v.as_u64().and_then(|n| usize::try_from(n).ok()))
                        .collect()
                })
                .unwrap_or_default()
        })
        .collect())
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesWhereVariables"
)]
pub struct NodesCountQuery {
    #[arguments(where: $where_clause)]
    pub nodes_aggregate: NodesAggregate,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_aggregate"
)]
pub struct NodesAggregate {
    pub aggregate: Option<NodesAggregateFields>,
}

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_aggregate_fields"
)]
pub struct NodesAggregateFields {
    pub count: i32,
}

/// Count the nodes matching a filter via `nodes_aggregate` (the §1 aggregate the
/// poll-list vote badge and other counts build on).
pub async fn count_nodes(
    access_token: Option<&str>,
    where_clause: NodesBoolExp,
) -> Result<usize, String> {
    use cynic::QueryBuilder;
    let op = NodesCountQuery::build(NodesWhereVariables { where_clause });
    let r = execute(access_token, op).await?;
    Ok(r.nodes_aggregate
        .aggregate
        .map(|a| a.count.max(0) as usize)
        .unwrap_or(0))
}

/// The number of votes cast on a poll (its visible `vote/vote` children).
pub async fn poll_vote_count(access_token: Option<&str>, poll_id: &str) -> Result<usize, String> {
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(poll_id.to_string())),
            is_null: None,
        }),
        mime_id: Some(StringComparisonExp {
            eq: Some("vote/vote".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    count_nodes(access_token, where_clause).await
}

#[derive(cynic::QueryVariables, Debug)]
pub struct MembersExistVariables {
    pub where_clause: MembersBoolExp,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "MembersExistVariables"
)]
pub struct MembersExistQuery {
    #[arguments(where: $where_clause, limit: 1)]
    pub members: Vec<MemberIdRef>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "members")]
pub struct MemberIdRef {
    pub id: Uuid,
}

/// Whether the user is an active member of a context — the port's approximation
/// of React VoteApp's `canVote` (an active membership carrying the vote/vote
/// insert permission), used for the voting-rights card.
pub async fn is_active_member(access_token: Option<&str>, context_id: &str, user_id: &str) -> bool {
    use cynic::QueryBuilder;
    let where_clause = MembersBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
        node_id: Some(UuidComparisonExp {
            eq: Some(Uuid(user_id.to_string())),
            is_null: None,
        }),
        active: Some(BooleanComparisonExp { eq: Some(true) }),
        ..Default::default()
    };
    let op = MembersExistQuery::build(MembersExistVariables { where_clause });
    execute(access_token, op)
        .await
        .map(|r| !r.members.is_empty())
        .unwrap_or(false)
}

/// Cast a vote on a poll: insert a `vote/vote` child whose data is the array of
/// selected option indices (matching the React VoteApp).
pub async fn cast_vote(
    access_token: Option<&str>,
    poll_id: &str,
    context_id: Option<&str>,
    selected: &[usize],
    key_suffix: &str,
) -> Result<bool, String> {
    let data = serde_json::Value::Array(
        selected
            .iter()
            .map(|i| serde_json::Value::from(u64::try_from(*i).unwrap_or(0)))
            .collect(),
    );
    let input = NodesInsertInput {
        name: Some(format!("vote-{key_suffix}")),
        key: Some(format!("vote-{key_suffix}")),
        mime_id: Some("vote/vote".to_string()),
        parent_id: Some(Uuid(poll_id.to_string())),
        context_id: context_id.map(|c| Uuid(c.to_string())),
        data: Some(Jsonb(data)),
        mutable: None,
        index: None,
    };
    Ok(insert_node(access_token, input).await?.is_some())
}

/// Search nodes by name (case-insensitive substring match)
pub async fn search_nodes(
    access_token: Option<&str>,
    query: &str,
    context_id: Option<&str>,
) -> Result<Vec<NodeFields>, String> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    let mut filters = vec![
        NodesBoolExp {
            name: Some(StringComparisonExp {
                ilike: Some(format!("%{query}%")),
                ..Default::default()
            }),
            ..Default::default()
        },
        // Exclude orphan/root nodes (no parent), matching React's search.
        NodesBoolExp {
            parent_id: Some(UuidComparisonExp {
                eq: None,
                is_null: Some(false),
            }),
            ..Default::default()
        },
        // Hide system/hidden mimes unless they are contexts (groups/events):
        // `mime.hidden = false OR mime.context = true`.
        NodesBoolExp {
            or: Some(vec![
                NodesBoolExp {
                    mime: Some(MimesBoolExp {
                        hidden: Some(BooleanComparisonExp { eq: Some(false) }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
                NodesBoolExp {
                    mime: Some(MimesBoolExp {
                        context: Some(BooleanComparisonExp { eq: Some(true) }),
                        ..Default::default()
                    }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        },
    ];
    // Scope the search to a single context (group/event) when requested.
    if let Some(ctx) = context_id {
        filters.push(NodesBoolExp {
            context_id: Some(UuidComparisonExp {
                eq: Some(Uuid(ctx.to_string())),
                is_null: None,
            }),
            ..Default::default()
        });
    }
    let where_clause = NodesBoolExp {
        and: Some(filters),
        ..Default::default()
    };

    let operation = NodesWhereQuery::build(NodesWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes)
}

/// The most recently created content nodes for the home "Newest" list (#34):
/// submitted (immutable) content whose context (group/event) the user belongs
/// to. Drafts and content from contexts the user is not part of are excluded.
pub async fn query_recent_nodes(
    access_token: Option<&str>,
    limit: i32,
    user_id: &str,
) -> Vec<ChildNodeFields> {
    let where_clause = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    in_: Some(vec![
                        "wiki/document".to_string(),
                        "vote/policy".to_string(),
                        "vote/change".to_string(),
                        "vote/position".to_string(),
                        "vote/candidate".to_string(),
                        "wiki/file".to_string(),
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            // Submitted only — drafts (mutable) never appear in Newest.
            NodesBoolExp {
                mutable: Some(BooleanComparisonExp { eq: Some(false) }),
                ..Default::default()
            },
            // Only content in a context (group/event) the user belongs to.
            NodesBoolExp {
                context: Some(Box::new(belongs_to_user(user_id))),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    let order_by = vec![NodesOrderBy {
        created_at: Some(OrderBy::Desc),
        index: None,
    }];
    let op = RecentNodesQuery::build(RecentNodesVariables {
        where_clause,
        order_by: Some(order_by),
        limit: Some(limit),
    });
    match execute(access_token, op).await {
        Ok(d) => d.nodes,
        Err(_) => Vec::new(),
    }
}

// --- Authors: search + replace a node's members ---

/// An author option: a group/user (carrying its node id) or a free-text name.
#[derive(Clone, Debug, PartialEq)]
pub struct Author {
    pub name: String,
    pub node_id: Option<String>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "users_bool_exp"
)]
pub struct UsersBoolExp {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub display_name: Option<StringComparisonExp>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UsersSearchVariables {
    pub where_clause: UsersBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "UsersSearchVariables"
)]
pub struct UsersSearchQuery {
    #[arguments(where: $where_clause, limit: 10)]
    pub users: Vec<UserSearchFields>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "users")]
pub struct UserSearchFields {
    pub id: Uuid,
    pub display_name: String,
}

/// Search groups and users by name for the author autocomplete (each carries the
/// node id used as the member's `node_id`).
pub async fn search_authors(access_token: Option<&str>, query: &str) -> Vec<Author> {
    if query.trim().is_empty() {
        return vec![];
    }
    let mut out: Vec<Author> = Vec::new();
    // Groups can author content.
    if let Ok(nodes) = search_nodes(access_token, query, None).await {
        for n in nodes
            .into_iter()
            .filter(|n| n.mime_id.as_deref() == Some("wiki/group"))
            .take(10)
        {
            out.push(Author {
                name: n.name,
                node_id: Some(n.id.0),
            });
        }
    }
    // Users.
    use cynic::QueryBuilder;
    let op = UsersSearchQuery::build(UsersSearchVariables {
        where_clause: UsersBoolExp {
            display_name: Some(StringComparisonExp {
                ilike: Some(format!("%{query}%")),
                ..Default::default()
            }),
        },
    });
    if let Ok(r) = execute(access_token, op).await {
        for u in r.users.into_iter().take(10) {
            out.push(Author {
                name: u.display_name,
                node_id: Some(u.id.0),
            });
        }
    }
    out
}

/// Search users by display name for the member-invite autocomplete. Unlike
/// [`search_authors`] this excludes groups — an invite binds a real user.
pub async fn search_users(access_token: Option<&str>, query: &str) -> Vec<Author> {
    if query.trim().is_empty() {
        return vec![];
    }
    use cynic::QueryBuilder;
    let op = UsersSearchQuery::build(UsersSearchVariables {
        where_clause: UsersBoolExp {
            display_name: Some(StringComparisonExp {
                ilike: Some(format!("%{query}%")),
                ..Default::default()
            }),
        },
    });
    let mut out = Vec::new();
    if let Ok(r) = execute(access_token, op).await {
        for u in r.users.into_iter().take(10) {
            out.push(Author {
                name: u.display_name,
                node_id: Some(u.id.0),
            });
        }
    }
    out
}

/// Invite a known user by node id (binds `nodeId` + `name`), as opposed to the
/// email-only [`invite_member`]. Mirrors selecting a `users` match in React
/// InvitesTextField.
pub async fn invite_member_by_node(
    access_token: Option<&str>,
    parent_id: &str,
    node_id: &str,
    name: &str,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = InsertMemberMutation::build(InsertMemberVariables {
        object: MembersInsertInput {
            node_id: Some(Uuid(node_id.to_string())),
            name: Some(name.to_string()),
            parent_id: Some(Uuid(parent_id.to_string())),
            ..Default::default()
        },
    });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_member.is_some())
}

#[derive(cynic::QueryVariables, Debug)]
pub struct DeleteMembersVariables {
    pub where_clause: MembersBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "DeleteMembersVariables"
)]
pub struct DeleteMembersMutation {
    #[arguments(where: $where_clause)]
    pub delete_members: Option<MembersAffected>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_mutation_response"
)]
pub struct MembersAffected {
    #[cynic(rename = "affected_rows")]
    pub affected_rows: i32,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct UpdateMembersWhereVariables {
    pub where_clause: MembersBoolExp,
    pub set: MembersSetInput,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "UpdateMembersWhereVariables"
)]
pub struct UpdateMembersMutation {
    #[arguments(where: $where_clause, _set: $set)]
    pub update_members: Option<MembersAffected>,
}

/// Accept a pre-existing membership by `(parent, node)` instead of an email
/// invite row. The fallback when accepting an email invite would violate the
/// `(parentId, nodeId)` unique constraint because the user is already a member
/// of that context (React InvitesUserList's accept-then-fallback).
pub async fn accept_existing_member(
    access_token: Option<&str>,
    parent_id: &str,
    node_id: &str,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let op = UpdateMembersMutation::build(UpdateMembersWhereVariables {
        where_clause: MembersBoolExp {
            parent_id: Some(UuidComparisonExp {
                eq: Some(Uuid(parent_id.to_string())),
                is_null: None,
            }),
            node_id: Some(UuidComparisonExp {
                eq: Some(Uuid(node_id.to_string())),
                is_null: None,
            }),
            ..Default::default()
        },
        set: MembersSetInput {
            accepted: Some(true),
            ..Default::default()
        },
    });
    let r = execute(access_token, op).await?;
    Ok(r.update_members
        .map(|m| m.affected_rows > 0)
        .unwrap_or(false))
}

/// Delete every member row belonging to a node (`members` where
/// `parent_id = node_id`). React's DeleteButton removes members before the node
/// itself so no orphan member rows are left behind.
pub async fn delete_node_members(
    access_token: Option<&str>,
    node_id: &str,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let del = DeleteMembersMutation::build(DeleteMembersVariables {
        where_clause: MembersBoolExp {
            parent_id: Some(UuidComparisonExp {
                eq: Some(Uuid(node_id.to_string())),
                is_null: None,
            }),
            ..Default::default()
        },
    });
    execute(access_token, del).await?;
    Ok(true)
}

/// Replace a node's authors: delete the current members and insert `authors`. A
/// group/user author carries its `node_id`; a free-text author is stored by name
/// only. Mirrors the React editor's save.
pub async fn set_node_authors(
    access_token: Option<&str>,
    node_id: &str,
    authors: &[Author],
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let del = DeleteMembersMutation::build(DeleteMembersVariables {
        where_clause: MembersBoolExp {
            parent_id: Some(UuidComparisonExp {
                eq: Some(Uuid(node_id.to_string())),
                is_null: None,
            }),
            ..Default::default()
        },
    });
    execute(access_token, del).await?;
    for author in authors {
        let op = InsertMemberMutation::build(InsertMemberVariables {
            object: MembersInsertInput {
                name: Some(author.name.clone()),
                node_id: author.node_id.clone().map(Uuid),
                parent_id: Some(Uuid(node_id.to_string())),
                ..Default::default()
            },
        });
        execute(access_token, op).await?;
    }
    Ok(true)
}

// --- Comments (vote/comment child nodes, nested) ---

#[derive(cynic::QueryVariables, Debug)]
pub struct CommentsVariables {
    pub where_clause: NodesBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "CommentsVariables"
)]
pub struct CommentsQuery {
    #[arguments(where: $where_clause)]
    pub nodes: Vec<ChildNodeFields>,
}

/// The `vote/comment` children of a node (a post or another comment), oldest
/// first. Called per level to build the nested comment thread.
/// The mime ids this node allows as children (its `inserts` computed field,
/// evaluated server-side against the caller's membership). Used to gate the
/// comment composer to where `vote/comment` can actually be inserted, mirroring
/// the old wiki's AddCommentButton. Returns an empty list on error or for a
/// caller with no permission.
pub async fn node_insert_mimes(access_token: Option<&str>, node_id: &str) -> Vec<String> {
    let op = NodeInsertsQuery::build(NodeByIdVariables {
        id: Uuid(node_id.to_string()),
    });
    match execute(access_token, op).await {
        Ok(data) => data
            .node
            .and_then(|n| n.inserts)
            .map(|mimes| mimes.into_iter().map(|m| m.id).collect())
            .unwrap_or_default(),
        Err(_) => Vec::new(),
    }
}

pub async fn query_comments(
    access_token: Option<&str>,
    parent_id: &str,
) -> Result<Vec<ChildNodeFields>, String> {
    use cynic::QueryBuilder;
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(parent_id.to_string())),
            is_null: None,
        }),
        mime_id: Some(StringComparisonExp {
            eq: Some("vote/comment".to_string()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let op = CommentsQuery::build(CommentsVariables { where_clause });
    let mut nodes = execute(access_token, op).await?.nodes;
    nodes.sort_by(|a, b| {
        let at = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        let bt = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        at.cmp(bt)
    });
    Ok(nodes)
}

/// Post a comment (a `vote/comment` node) under `parent_id` (a post or another
/// comment for a nested reply). `author` is stored as the node name and `text`
/// as `data.text`, matching the reference comment shape.
#[allow(clippy::too_many_arguments)]
pub async fn insert_comment(
    access_token: Option<&str>,
    parent_id: &str,
    context_id: Option<&str>,
    key: &str,
    author: &str,
    text: &str,
) -> Result<bool, String> {
    let input = NodesInsertInput {
        name: Some(author.to_string()),
        key: Some(key.to_string()),
        mime_id: Some("vote/comment".to_string()),
        parent_id: Some(Uuid(parent_id.to_string())),
        context_id: context_id.map(|c| Uuid(c.to_string())),
        data: Some(Jsonb(serde_json::json!({ "text": text }))),
        mutable: Some(false),
        index: None,
    };
    insert_node(access_token, input)
        .await
        .map(|inserted| inserted.is_some())
}

/// Build the `where` filter for the user's context nodes (groups or events) of
/// a given mime type: nodes the user owns or has an accepted membership in.
/// Nodes the user "belongs to": ones they own or have an accepted membership in.
/// Shared by the contexts list and the "Newest" list's context filter.
fn belongs_to_user(user_id: &str) -> NodesBoolExp {
    let owned = NodesBoolExp {
        owner_id: Some(UuidComparisonExp {
            eq: Some(Uuid(user_id.to_string())),
            is_null: None,
        }),
        ..Default::default()
    };
    let member = NodesBoolExp {
        members: Some(MembersBoolExp {
            and: Some(vec![
                MembersBoolExp {
                    accepted: Some(BooleanComparisonExp { eq: Some(true) }),
                    ..Default::default()
                },
                MembersBoolExp {
                    node_id: Some(UuidComparisonExp {
                        eq: Some(Uuid(user_id.to_string())),
                        is_null: None,
                    }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        }),
        ..Default::default()
    };
    NodesBoolExp {
        or: Some(vec![owned, member]),
        ..Default::default()
    }
}

fn contexts_where_clause(user_id: &str, mime_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    eq: Some(mime_id.to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            belongs_to_user(user_id),
        ]),
        ..Default::default()
    }
}

/// Fetch the user's context nodes (groups or events) of a given mime type.
/// Matches nodes the user owns or has an accepted membership in, newest first.
pub async fn query_contexts(
    access_token: Option<&str>,
    user_id: &str,
    mime_id: &str,
) -> Result<Vec<ContextNodeFields>, String> {
    let where_clause = contexts_where_clause(user_id, mime_id);
    let operation = ContextsWhereQuery::build(NodesWhereVariables { where_clause });
    let mut result = execute(access_token, operation).await?;
    // Newest first (the API returns no guaranteed order).
    result.nodes.sort_by(|a, b| {
        let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        b_ts.cmp(a_ts)
    });
    Ok(result.nodes)
}

/// Nodes with a missing parent (`parentId is null`) — the "Missing parent" admin
/// view (#149). The single legitimate root is one of these, so callers filter it
/// out; anything else is an orphan that lost its parent.
pub async fn query_orphans(access_token: Option<&str>) -> Result<Vec<ContextNodeFields>, String> {
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            is_null: Some(true),
            eq: None,
        }),
        ..Default::default()
    };
    let operation = ContextsWhereQuery::build(NodesWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes)
}

/// Build the `where` filter for a node's visible children, mirroring the React
/// drawer (`DrawerList`): children the user may see (immutable, or owned, or a
/// member of) whose mime is not hidden.
/// A node is "visible" to the user when it is published, owned by them, or one
/// they are a member of. Shared by the drawer's child query and its per-row
/// `children_aggregate` count.
fn visible_to_user(user_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        or: Some(vec![
            NodesBoolExp {
                mutable: Some(BooleanComparisonExp { eq: Some(false) }),
                ..Default::default()
            },
            NodesBoolExp {
                owner_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(user_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
            NodesBoolExp {
                members: Some(MembersBoolExp {
                    node_id: Some(UuidComparisonExp {
                        eq: Some(Uuid(user_id.to_string())),
                        is_null: None,
                    }),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

/// Filter excluding nodes whose mime type is marked hidden.
fn mime_not_hidden() -> NodesBoolExp {
    NodesBoolExp {
        mime: Some(MimesBoolExp {
            hidden: Some(BooleanComparisonExp { eq: Some(false) }),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// The filter for a node's *visible* children (no parent constraint — the
/// `children_aggregate` relation is already scoped to the node). Drives the
/// drawer expander so it only appears when there are children to reveal.
fn child_visibility_clause(user_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        and: Some(vec![visible_to_user(user_id), mime_not_hidden()]),
        ..Default::default()
    }
}

fn children_where_clause(parent_id: &str, user_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                parent_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(parent_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
            visible_to_user(user_id),
            mime_not_hidden(),
        ]),
        ..Default::default()
    }
}

/// Fetch a node's visible children, ordered by index then creation time — the
/// data behind one level of the drawer's lazy node tree.
pub async fn query_children(
    access_token: Option<&str>,
    parent_id: &str,
    user_id: &str,
) -> Result<Vec<ChildNodeFields>, String> {
    let where_clause = children_where_clause(parent_id, user_id);
    let order_by = drawer_child_order();
    let operation = ChildrenQuery::build(ChildrenVariables {
        where_clause,
        order_by: Some(order_by),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes)
}

/// The drawer-tree variant of `query_children`: same visible-children filter and
/// ordering, but each row also carries a `children_aggregate` count so the
/// expander chevron only shows for nodes that actually have visible children.
pub async fn query_drawer_children(
    access_token: Option<&str>,
    parent_id: &str,
    user_id: &str,
) -> Result<Vec<DrawerChildFields>, String> {
    let where_clause = children_where_clause(parent_id, user_id);
    let child_visible = child_visibility_clause(user_id);
    let order_by = drawer_child_order();
    let operation = DrawerChildrenQuery::build(DrawerChildrenVariables {
        where_clause,
        order_by: Some(order_by),
        child_visible,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes)
}

/// Shared ordering for a node's children: by explicit index, then creation time
/// (the folder view's order).
fn drawer_child_order() -> Vec<NodesOrderBy> {
    vec![
        NodesOrderBy {
            index: Some(OrderBy::Asc),
            created_at: None,
        },
        NodesOrderBy {
            index: None,
            created_at: Some(OrderBy::Asc),
        },
    ]
}

/// Resolve the path (list of keys from the root's child down to the node) for a
/// node id by walking up the parent chain. Mirrors the React `fromId` helper:
/// the root node contributes no segment.
pub async fn path_from_id(access_token: Option<&str>, id: &str) -> Result<Vec<String>, String> {
    let mut segments: Vec<String> = Vec::new();
    let mut current = Some(id.to_string());
    // Guard against cycles / unexpectedly deep trees.
    for _ in 0..32 {
        let Some(node_id) = current.take() else {
            break;
        };
        let operation = NodeByIdQuery::build(NodeByIdVariables { id: Uuid(node_id) });
        let data = execute(access_token, operation).await?;
        match data.node {
            Some(node) => match node.parent_id {
                Some(parent) => {
                    segments.push(node.key);
                    current = Some(parent.0);
                }
                // Reached the root — stop without adding its key.
                None => break,
            },
            None => break,
        }
    }
    segments.reverse();
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_jwt_errors_for_refresh_retry() {
        // Hasura's JWT failures all mention "JWT"; refresh + retry may recover.
        assert!(is_jwt_error("Could not verify JWT: JWTExpired"));
        assert!(is_jwt_error("Could not verify JWT: JWTInvalid signature"));
        assert!(is_jwt_error(r#"[{"message":"invalid-jwt"}]"#));
        // Unrelated errors must NOT trigger a pointless refresh + retry.
        assert!(!is_jwt_error("permission denied on nodes"));
        assert!(!is_jwt_error("No data returned"));
    }

    /// The Hasura API rejects `null` for a comparison expression
    /// (`expected an object for type 'String_comparison_exp', but found null`),
    /// so unset `Option` input fields must be omitted from the wire format
    /// rather than serialized as `null`.
    #[test]
    fn contexts_where_clause_omits_null_fields() {
        let clause = contexts_where_clause("user-123", "wiki/group");
        let json = serde_json::to_string(&clause).expect("serialize where clause");

        assert!(
            !json.contains("null"),
            "where clause must not send null comparison expressions: {json}"
        );
        // The filter the query actually depends on must survive serialization.
        assert!(json.contains("\"mimeId\""), "missing mimeId filter: {json}");
        assert!(json.contains("wiki/group"), "missing mime value: {json}");
        assert!(
            json.contains("\"ownerId\""),
            "missing ownerId filter: {json}"
        );
        assert!(
            json.contains("\"members\""),
            "missing members filter: {json}"
        );
        assert!(
            json.contains("\"accepted\""),
            "missing accepted filter: {json}"
        );
        assert!(json.contains("user-123"), "missing user id: {json}");
    }

    /// A single-field comparison expression must serialize to just that field,
    /// with no sibling `null` keys.
    #[test]
    fn string_comparison_exp_omits_null_fields() {
        let exp = StringComparisonExp {
            eq: Some("wiki/event".to_string()),
            ..Default::default()
        };
        let json = serde_json::to_string(&exp).expect("serialize comparison exp");
        assert_eq!(json, r#"{"_eq":"wiki/event"}"#);
    }

    /// The invitations filter must omit null fields and carry the pending +
    /// group/event + user/email conditions the home list depends on.
    #[test]
    fn invitations_where_clause_is_well_formed() {
        let clause = invitations_where_clause("user-1", "me@example.com");
        let json = serde_json::to_string(&clause).expect("serialize invitations clause");
        assert!(!json.contains("null"), "must omit null fields: {json}");
        assert!(json.contains("\"accepted\""), "missing accepted: {json}");
        assert!(json.contains("\"_or\""), "missing _or (user/email): {json}");
        assert!(json.contains("me@example.com"), "missing email: {json}");
        assert!(json.contains("user-1"), "missing user id: {json}");
        assert!(
            json.contains("wiki/group") && json.contains("wiki/event"),
            "missing parent mime filter: {json}"
        );
    }
}
