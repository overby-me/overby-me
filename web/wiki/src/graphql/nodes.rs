//! Reading and writing nodes: resolving a path to one, its children, its
//! crumbs, and creating, editing, copying, moving and binning them. The wiki's
//! own vocabulary, on which every other module here is a view.

use super::*;

// --- Anti-corruption seam: cynic wire types <-> frontend-owned `model` types ---
//
// The cynic types below (bound to `graphql/schema.graphql`) are the wire shapes.
// Every public query/mutation fn converts at its boundary so components only ever
// see `model` types. This is the single place the two type families meet; swapping
// the backend replaces this file, not every component.

impl From<Uuid> for model::Uuid {
    fn from(v: Uuid) -> Self {
        model::Uuid(v.0)
    }
}
impl From<Timestamptz> for model::Timestamptz {
    fn from(v: Timestamptz) -> Self {
        model::Timestamptz(v.0)
    }
}
impl From<Jsonb> for model::Jsonb {
    fn from(v: Jsonb) -> Self {
        model::Jsonb(v.0)
    }
}

impl From<MimeFields> for model::MimeFields {
    fn from(m: MimeFields) -> Self {
        model::MimeFields {
            id: m.id,
            icon: m.icon,
            hidden: m.hidden,
            context: m.context,
        }
    }
}
impl From<UserRef> for model::UserRef {
    fn from(u: UserRef) -> Self {
        model::UserRef {
            id: u.id.into(),
            display_name: u.display_name,
            avatar_url: u.avatar_url,
        }
    }
}
impl From<MemberNodeRef> for model::MemberNodeRef {
    fn from(n: MemberNodeRef) -> Self {
        model::MemberNodeRef { mime_id: n.mime_id }
    }
}
impl From<ParentNodeFields> for model::ParentNodeFields {
    fn from(p: ParentNodeFields) -> Self {
        model::ParentNodeFields {
            id: p.id.into(),
            name: p.name,
            key: p.key,
            mime_id: p.mime_id,
            data: p.data.map(Into::into),
            author_avatar: p.author_avatar,
        }
    }
}
impl From<MemberFields> for model::MemberFields {
    fn from(m: MemberFields) -> Self {
        model::MemberFields {
            id: m.id.into(),
            name: m.name,
            // Not selected on a node read; see MemberFields.
            email: None,
            accepted: m.accepted,
            active: m.active,
            owner: m.owner,
            hidden: m.hidden,
            node_id: m.node_id.map(Into::into),
            user: m.user.map(Into::into),
            node: m.node.map(Into::into),
        }
    }
}
impl From<ChildNodeFields> for model::ChildNodeFields {
    fn from(c: ChildNodeFields) -> Self {
        model::ChildNodeFields {
            id: c.id.into(),
            name: c.name,
            key: c.key,
            mime_id: c.mime_id,
            mutable: c.mutable,
            index: c.index,
            created_at: c.created_at.map(Into::into),
            owner_id: c.owner_id.map(Into::into),
            data: c.data.map(Into::into),
            mime: c.mime.map(Into::into),
            is_owner: c.is_owner,
            is_context_owner: c.is_context_owner,
            owner: c.owner.map(Into::into),
            author_name: c.author_name,
            author_avatar: c.author_avatar,
            parent: c.parent.map(Into::into),
        }
    }
}
impl From<NodeWithChildren> for model::NodeWithChildren {
    fn from(n: NodeWithChildren) -> Self {
        model::NodeWithChildren {
            id: n.id.into(),
            name: n.name,
            key: n.key,
            path: n.path,
            mime_id: n.mime_id,
            parent_id: n.parent_id.map(Into::into),
            context_id: n.context_id.map(Into::into),
            owner_id: n.owner_id.map(Into::into),
            mutable: n.mutable,
            index: n.index,
            get_index: n.get_index,
            data: n.data.map(Into::into),
            mime: n.mime.map(Into::into),
            parent: n.parent.map(|p| Box::new((*p).into())),
            children: n.children.into_iter().map(Into::into).collect(),
            members: n.members.into_iter().map(Into::into).collect(),
            is_owner: n.is_owner,
            is_context_owner: n.is_context_owner,
            attachable: n.attachable,
            created_at: n.created_at.map(Into::into),
            owner: n.owner.map(Into::into),
        }
    }
}
impl From<NodeFields> for model::NodeFields {
    fn from(n: NodeFields) -> Self {
        model::NodeFields {
            id: n.id.into(),
            name: n.name,
            key: n.key,
            path: n.path,
            mime_id: n.mime_id,
            parent_id: n.parent_id.map(Into::into),
            context_id: n.context_id.map(Into::into),
            owner_id: n.owner_id.map(Into::into),
            mutable: n.mutable,
            index: n.index,
            get_index: n.get_index,
            data: n.data.map(Into::into),
            mime: n.mime.map(Into::into),
            is_owner: n.is_owner,
            is_context_owner: n.is_context_owner,
            created_at: n.created_at.map(Into::into),
            parent: n.parent.map(Into::into),
        }
    }
}
/// Put a `data(path: "type")` value back under the key the icon helper reads.
///
/// The fragments that only ever draw an icon ask the server for the value at
/// `$.type` instead of the whole document. What comes back is that value — a
/// bare string like `"application/pdf"`, or null — so it is wrapped again here.
/// The point is that every row shape then feeds the SAME `node_icon_mime_id`,
/// and a search hit, a drawer row and a folder row cannot drift apart.
fn icon_data(value: Option<Jsonb>) -> Option<model::Jsonb> {
    value.map(|t| model::Jsonb(serde_json::json!({ "type": t.0 })))
}

impl From<ContextNodeFields> for model::ContextNodeFields {
    fn from(c: ContextNodeFields) -> Self {
        model::ContextNodeFields {
            id: c.id.into(),
            name: c.name,
            key: c.key,
            mime_id: c.mime_id,
            parent_id: c.parent_id.map(Into::into),
            created_at: c.created_at.map(Into::into),
            data: icon_data(c.data),
        }
    }
}
impl From<DrawerChildFields> for model::DrawerChildFields {
    fn from(d: DrawerChildFields) -> Self {
        let child_count = d
            .children_aggregate
            .aggregate
            .as_ref()
            .map(|a| a.count)
            .unwrap_or(0);
        model::DrawerChildFields {
            id: d.id.into(),
            name: d.name,
            key: d.key,
            mime_id: d.mime_id,
            mutable: d.mutable,
            data: icon_data(d.data),
            child_count,
        }
    }
}
impl From<PollSummaryFields> for model::PollSummaryFields {
    fn from(p: PollSummaryFields) -> Self {
        model::PollSummaryFields {
            id: p.id.into(),
            name: p.name,
            data: p.data.map(Into::into),
            created_at: p.created_at.map(Into::into),
            mutable: p.mutable,
        }
    }
}
impl From<InvitationFields> for model::InvitationFields {
    fn from(i: InvitationFields) -> Self {
        model::InvitationFields {
            id: i.id.into(),
            parent: i.parent.map(Into::into),
        }
    }
}
impl From<PermissionFields> for model::PermissionFields {
    fn from(p: PermissionFields) -> Self {
        model::PermissionFields {
            id: p.id.into(),
            mime_id: p.mime_id,
            role: p.role,
            insert: p.insert,
            select: p.select,
            delete: p.delete,
            active: p.active,
        }
    }
}
impl From<UserSearchFields> for model::UserSearchFields {
    fn from(u: UserSearchFields) -> Self {
        model::UserSearchFields {
            id: u.id.into(),
            display_name: u.display_name,
            avatar_url: u.avatar_url,
        }
    }
}
impl From<InsertedNode> for model::InsertedNode {
    fn from(n: InsertedNode) -> Self {
        model::InsertedNode {
            id: n.id.into(),
            key: n.key,
        }
    }
}

// Write side: frontend-owned `model` inputs -> cynic input objects.
impl From<model::NodesInsertInput> for NodesInsertInput {
    fn from(m: model::NodesInsertInput) -> Self {
        NodesInsertInput {
            name: m.name,
            key: m.key,
            mime_id: m.mime_id,
            parent_id: m.parent_id.map(|u| Uuid(u.0)),
            context_id: m.context_id.map(|u| Uuid(u.0)),
            data: m.data.map(|j| Jsonb(j.0)),
            mutable: m.mutable,
            index: m.index,
            created_at: m.created_at.map(|t| Timestamptz(t.0)),
        }
    }
}
impl From<model::NodesSetInput> for NodesSetInput {
    fn from(m: model::NodesSetInput) -> Self {
        NodesSetInput {
            name: m.name,
            data: m.data.map(|j| Jsonb(j.0)),
            mutable: m.mutable,
            index: m.index,
            attachable: m.attachable,
            context_id: m.context_id.map(|u| Uuid(u.0)),
            owner_id: m.owner_id.map(|u| Uuid(u.0)),
            parent_id: m.parent_id.map(|u| Uuid(u.0)),
            created_at: m.created_at.map(|t| Timestamptz(t.0)),
        }
    }
}
impl From<model::MembersSetInput> for MembersSetInput {
    fn from(m: model::MembersSetInput) -> Self {
        MembersSetInput {
            accepted: m.accepted,
            active: m.active,
            email: m.email,
            name: m.name,
            owner: m.owner,
            hidden: m.hidden,
            node_id: m.node_id.map(|u| Uuid(u.0)),
            parent_id: m.parent_id.map(|u| Uuid(u.0)),
        }
    }
}

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

/// The same filter, plus a cap — for the queries that declare `limit: $limit`.
///
/// Deliberately a SEPARATE struct rather than an `Option<i32>` on the one above.
/// cynic declares only the variables an operation actually uses, but serialises
/// every field of the struct it was given, so a shared optional `limit` rode
/// along in the JSON of five queries that never mentioned it — and Hasura
/// rejects an undeclared variable outright ("unexpected variables in
/// variableValues: limit"), failing the whole query rather than ignoring it.
/// Votes, polls, the home context list, the subtree walk and the feed count all
/// stopped answering in production. One struct per operation shape means the
/// compiler, not a reviewer, keeps the two in step.
#[derive(cynic::QueryVariables, Debug)]
pub struct NodesLimitVariables {
    pub where_clause: NodesBoolExp,
    pub limit: Option<i32>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesLimitVariables"
)]
pub struct NodesWhereQuery {
    #[arguments(where: $where_clause, limit: $limit)]
    pub nodes: Vec<NodeFields>,
}

/// A node reduced to what a PICKER needs: something to show and something to
/// return. No `data`, which on a wiki node is the whole document.
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct NodeRef {
    pub id: Uuid,
    pub name: String,
}

#[derive(cynic::QueryVariables)]
pub struct NodePickerVariables {
    pub where_clause: NodesBoolExp,
    pub limit: Option<i32>,
}

/// A capped, lean node lookup for autocomplete.
///
/// `NodesWhereQuery` above answers with every field a page needs — including
/// `data`, the node's entire document, and the parent's — and has no limit. For
/// a search box that is the wrong shape twice over: it fetches everything that
/// matches and then throws nearly all of it away. Measured on production, one
/// keystroke of "ann" answered 407 rows and 1.5 MB in 4.4 seconds, to show at
/// most ten names.
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodePickerVariables"
)]
pub struct NodePickerQuery {
    #[arguments(where: $where_clause, limit: $limit)]
    pub nodes: Vec<NodeRef>,
}

/// The search bar's own query: the lean fragment, and a cap.
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesLimitVariables"
)]
pub struct NodesSearchQuery {
    #[arguments(where: $where_clause, limit: $limit)]
    pub nodes: Vec<SearchNodeFields>,
}

impl From<SearchNodeFields> for model::NodeFields {
    fn from(n: SearchNodeFields) -> Self {
        model::NodeFields {
            id: n.id.into(),
            name: n.name,
            key: n.key,
            path: n.path,
            mime_id: n.mime_id,
            parent_id: n.parent_id.map(Into::into),
            context_id: n.context_id.map(Into::into),
            owner_id: n.owner_id.map(Into::into),
            mutable: n.mutable,
            index: n.index,
            get_index: n.get_index,
            data: icon_data(n.data),
            mime: n.mime.map(Into::into),
            is_owner: n.is_owner,
            is_context_owner: n.is_context_owner,
            created_at: n.created_at.map(Into::into),
            parent: n.parent.map(|p| model::ParentNodeFields {
                id: p.id.into(),
                name: p.name,
                key: p.key,
                mime_id: p.mime_id,
                // Not fetched for a search result; see ParentNameRef.
                data: None,
                author_avatar: None,
            }),
        }
    }
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
    /// The node's own trail, for the subtree operations that key off it (the
    /// bin). Maintained by a database trigger.
    pub path: Option<String>,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub context_id: Option<Uuid>,
    pub owner_id: Option<Uuid>,
    pub mutable: bool,
    pub index: i32,
    // Computed ordinal among same-type siblings (1-based), as NodeFields selects
    // for the drawer: it is what letters a policy A/B/C and numbers a change
    // 1/2/3 on its own page, where no sibling list is loaded to count from.
    pub get_index: Option<i32>,
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

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "NodesWhereVariables"
)]
pub struct ChildIdsQuery {
    #[arguments(where: $where_clause)]
    pub nodes: Vec<NodeIdFields>,
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

#[derive(cynic::QueryVariables, Debug)]
pub struct InsertRelationVariables {
    pub object: RelationsInsertInput,
    pub on_conflict: RelationsOnConflict,
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
) -> Result<(Vec<model::MemberFields>, usize), String> {
    let where_clause = members_where(parent_id, filter);
    // The `members` table exposes no `_aggregate` in this schema, so the total is
    // counted with a separate id-only query (just UUIDs) instead of
    // `members_aggregate` — which fails validation and would empty the whole page.
    let query = format!(
        "query {{ \
           page: members(where: {w}, order_by: {{ name: asc }}, limit: {limit}, offset: {offset}) {{ \
             id name email accepted active owner hidden nodeId \
             user {{ id displayName avatarUrl }} node {{ mimeId }} \
           }} \
           all: members(where: {w}) {{ id }} \
         }}",
        w = where_clause,
    );
    let data = execute_raw(access_token, &query).await?;
    let rows = data
        .get("page")
        .and_then(|m| m.as_array())
        .map(|arr| arr.iter().filter_map(parse_member_row).collect())
        .unwrap_or_default();
    let total = data
        .get("all")
        .and_then(|a| a.as_array())
        .map(|arr| arr.len())
        .unwrap_or(0);
    Ok((rows, total))
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

// --- High-level query functions ---

pub async fn query_node_by_id(
    access_token: Option<&str>,
    id: &str,
) -> Result<Option<model::NodeWithChildren>, String> {
    let operation = NodeWithChildrenQuery::build(NodeWithChildrenVariables {
        id: Uuid(id.to_string()),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.node.map(Into::into))
}

thread_local! {
    /// The root node's id, cached for the session. The parentless root never
    /// changes, yet `resolve_path` and `path_crumbs` each re-queried it on every
    /// navigation (2 redundant round-trips per nav).
    static ROOT_ID: std::cell::OnceCell<String> = const { std::cell::OnceCell::new() };
}

/// Id of the single parent-less root node ("Hjem", key "root"). Paths are
/// resolved relative to it: the root's key is not part of any URL path.
pub(crate) async fn query_root_id(access_token: Option<&str>) -> Result<Option<String>, String> {
    if let Some(id) = ROOT_ID.with(|c| c.get().cloned()) {
        return Ok(Some(id));
    }
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            in_: None,
            is_null: Some(true),
            eq: None,
        }),
        ..Default::default()
    };
    let operation = NodesWhereQuery::build(NodesLimitVariables {
        where_clause,
        limit: None,
    });
    let result = execute(access_token, operation).await?;
    let id = result.nodes.into_iter().next().map(|n| n.id.0);
    if let Some(id) = &id {
        ROOT_ID.with(|c| {
            let _ = c.set(id.clone());
        });
    }
    Ok(id)
}

/// The parent-less root node ("Hjem"), whose `data.content` backs the editable
/// welcome page. `resolve_path(&[])` returns `None` (the root's key is not a path
/// segment), so fetch it via the root id directly.
pub async fn query_root_node(
    access_token: Option<&str>,
) -> Result<Option<model::NodeWithChildren>, String> {
    let Some(root_id) = query_root_id(access_token).await? else {
        return Ok(None);
    };
    query_node_by_id(access_token, &root_id).await
}

pub async fn resolve_path(
    access_token: Option<&str>,
    segments: &[String],
) -> Result<Option<model::NodeWithChildren>, String> {
    if segments.is_empty() {
        return Ok(None);
    }
    // The URL IS the stored path (`nodes.path`, kept by a database trigger), so
    // one query finds the node however deep it is. This used to walk key by key
    // from the root, a round trip per segment, on every navigation.
    let where_clause = NodesBoolExp {
        path: Some(StringComparisonExp {
            eq: Some(segments.join("/")),
            ..Default::default()
        }),
        ..Default::default()
    };
    let op = NodesWhereQuery::build(NodesLimitVariables {
        where_clause,
        limit: None,
    });
    let key = format!("node:{}", segments.join("/"));
    let live = async {
        let Some(found) = execute(access_token, op).await?.nodes.into_iter().next() else {
            return Ok(None);
        };
        query_node_by_id(access_token, &found.id.0).await
    }
    .await;
    // The page a reader opened before the tunnel is the page they meant to read
    // in it. Anything that answered — including "no such node" — replaces the
    // copy; only an unreachable server falls back to one.
    match live {
        Ok(Some(node)) => {
            crate::offline::put(&key, &node);
            Ok(Some(node))
        }
        Ok(None) => Ok(None),
        Err(e) => match offline_copy::<model::NodeWithChildren>(&key, &e) {
            Some(node) => Ok(Some(node)),
            None => Err(e),
        },
    }
}

/// Resolve each path segment to its `(name, mime_id)`, walking from the root like
/// `resolve_path`. Feeds the breadcrumb trail its per-segment avatar + name.
pub async fn path_crumbs(
    access_token: Option<&str>,
    segments: &[String],
) -> Result<Vec<Crumb>, String> {
    if segments.is_empty() {
        return Ok(Vec::new());
    }
    // Every crumb in one query. A trail is the set of prefixes of the current
    // path, and each node stores its own, so `path _in [...]` fetches the lot.
    // This used to resolve a segment at a time, from the root down, which meant
    // five sequential round trips to draw the trail on a five-deep page — on
    // whatever network the reader happened to be on.
    let prefixes: Vec<String> = (1..=segments.len())
        .map(|n| segments[..n].join("/"))
        .collect();
    let where_clause = NodesBoolExp {
        path: Some(StringComparisonExp {
            in_: Some(prefixes.clone()),
            ..Default::default()
        }),
        ..Default::default()
    };
    let op = NodesWhereQuery::build(NodesLimitVariables {
        where_clause,
        limit: None,
    });
    // The trail is cached alongside the page: a bar of raw url slugs is how the
    // app looked on a dropped connection, and the trail is the part of the
    // chrome that says where you are.
    let key = format!("crumbs:{}", segments.join("/"));
    let found = match execute(access_token, op).await {
        Ok(data) => data.nodes,
        Err(e) => {
            return match offline_copy::<Vec<Crumb>>(&key, &e) {
                Some(cached) => Ok(cached),
                None => Err(e),
            };
        }
    };

    // Back into path order, and a crumb for every segment either way: a step the
    // reader may not see (permissions) or that does not resolve still holds its
    // place in the trail, showing its url slug, exactly as before.
    let crumbs: Vec<Crumb> = prefixes
        .iter()
        .zip(segments)
        .map(|(prefix, segment)| {
            match found.iter().find(|n| n.path.as_deref() == Some(prefix)) {
                Some(n) => Crumb {
                    key: segment.clone(),
                    name: n.name.clone(),
                    mime_id: n.mime_id.clone(),
                    // getIndex is 1-based; node_avatar wants a 0-based ordinal.
                    ordinal: n.get_index.filter(|i| *i >= 1).map(|i| (i - 1) as usize),
                    data: n.data.clone().map(Into::into),
                },
                None => Crumb {
                    key: segment.clone(),
                    name: segment.clone(),
                    mime_id: None,
                    ordinal: None,
                    data: None,
                },
            }
        })
        .collect();
    crate::offline::put(&key, &crumbs);
    Ok(crumbs)
}

/// Insert a node
/// How many numbered keys to try before giving up on a readable one.
pub(crate) const KEY_ATTEMPTS: u32 = 20;

/// Insert a node under a name, giving it the cleanest key that is free.
///
/// The key is what appears in the URL forever, so it is worth spending a request
/// on: `asger-holm-oerskov`, not `asger-holm-oerskov-8417`. A number is appended
/// only when the plain slug is taken, and then it counts up from 2.
///
/// Collisions are found by ATTEMPTING the insert, not by looking first. The
/// database has a unique index on (parent_id, key) and row-level permissions
/// mean a caller cannot see every sibling, so a look-first check would still
/// collide on a node it was not allowed to know about. Trying is also the common
/// case in one round trip, since most names are free.
///
/// After [`KEY_ATTEMPTS`] the timestamped key from
/// [`crate::components::loader::slugify`] ends it, so a pathological name can
/// never loop or fail outright.
///
/// The attempts run QUIET. A collision here is the mechanism working, not a
/// fault: reported normally, adding a second canvas called "test" showed the
/// person a database-constraint error, filed it in the feedback app as a bug,
/// and then created their canvas as `test-3` anyway. Only the final attempt —
/// the one whose failure is real — reports.
pub async fn insert_node_named(
    access_token: Option<&str>,
    mut input: model::NodesInsertInput,
    name: &str,
) -> Result<Option<model::InsertedNode>, String> {
    let base = crate::components::loader::slug_base(name);
    let base = if base.is_empty() {
        "n".to_string()
    } else {
        base
    };
    for attempt in 1..=KEY_ATTEMPTS {
        let key = if attempt == 1 {
            base.clone()
        } else {
            format!("{base}-{attempt}")
        };
        input.key = Some(key);
        match insert_node_quiet(access_token, input.clone()).await {
            Err(e) if is_key_taken(&e) => continue,
            other => return other,
        }
    }
    input.key = Some(crate::components::loader::slugify(name));
    insert_node(access_token, input).await
}

/// Whether an insert failed because the key was already used under that parent,
/// as opposed to anything else that can go wrong.
pub(crate) fn is_key_taken(error: &str) -> bool {
    // Both indexes that a taken name trips, BY NAME. A key is unique per parent
    // (`nodes_parent_id_namespace_key`) and the path it produces is unique among
    // live nodes (`nodes_path_live_idx`); it is the PATH one that fires in
    // practice, since it covers every live node while the key index is partial.
    //
    // This used to fall back to "a uniqueness violation mentioning `key`", which
    // matched the path index only through Postgres happening to say "duplicate
    // key value" — and matched every `*_pkey` in the schema as well. A primary
    // key violation is not a taken name: retrying it twenty times with new keys
    // cannot fix it, and the twentieth failure is what would be reported.
    error.contains("nodes_parent_id_namespace_key") || error.contains("nodes_path_live_idx")
}

pub async fn insert_node(
    access_token: Option<&str>,
    input: model::NodesInsertInput,
) -> Result<Option<model::InsertedNode>, String> {
    use cynic::MutationBuilder;
    let operation = InsertNodeMutation::build(InsertNodeVariables {
        object: input.into(),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_node.map(Into::into))
}

/// [`insert_node`] for an attempt whose failure the caller handles — the key
/// search in [`insert_node_named`]. Same insert; the error comes back, but a
/// taken key is not announced to the person or filed as a bug.
async fn insert_node_quiet(
    access_token: Option<&str>,
    input: model::NodesInsertInput,
) -> Result<Option<model::InsertedNode>, String> {
    use cynic::MutationBuilder;
    let operation = InsertNodeMutation::build(InsertNodeVariables {
        object: input.into(),
    });
    let result = execute_quiet(access_token, operation).await?;
    Ok(result.insert_node.map(Into::into))
}

/// The per-context permission template seeded when a new group/event is created,
/// mirroring the old wiki's `contextPerm`. Each row is stamped with the new
/// context's id (as both `contextId` and `nodeId`). Without these rows the
/// context is an empty shell: nothing (documents, polls, votes, comments, …) can
/// be inserted under it, since the server-side `insert` gate reads this table.
/// Every rule is insertable + selectable; only `vote/vote` is immutable (a cast
/// ballot is never edited or deleted by a member).
pub(crate) fn context_permission_objects(ctx_id: &str) -> serde_json::Value {
    // (child mime, role, parent mimes it may be created under)
    let rules: &[(&str, &str, &[&str])] = &[
        ("vote/vote", "member", &["vote/poll"]),
        ("vote/policy", "member", &["wiki/folder"]),
        ("vote/candidate", "member", &["vote/position"]),
        (
            "wiki/document",
            "owner",
            &["wiki/event", "wiki/folder", "wiki/group"],
        ),
        (
            "vote/poll",
            "owner",
            &["vote/policy", "vote/change", "vote/position"],
        ),
        ("vote/question", "member", &["vote/position", "wiki/file"]),
        // Comments belong on CONTENT nodes (motions, amendments, documents, files,
        // election posts and candidacies), not on containers (folders/groups/events).
        // The UI mounts a CommentSection on every content type above, so allow the
        // insert everywhere it is shown, keeping the affordance honest. (Seeded per
        // context at creation, so it applies to newly created contexts.)
        (
            "vote/comment",
            "member",
            &[
                "vote/policy",
                "vote/change",
                "wiki/document",
                "wiki/file",
                "vote/position",
                "vote/candidate",
                // Replies are a vote/comment under a vote/comment.
                "vote/comment",
            ],
        ),
        // Emoji reactions: a vote/reaction lives under a comment (or, in future,
        // directly on the content it reacts to), so it shares the comment's
        // parent set. Any member may react.
        ("vote/reaction", "member", REACTION_PARENTS),
        ("speak/speak", "member", &["speak/list"]),
        (
            "vote/change",
            "member",
            &["vote/policy", "vote/change", "wiki/file"],
        ),
        (
            "wiki/folder",
            "owner",
            &["wiki/folder", "wiki/group", "wiki/event"],
        ),
        ("vote/position", "owner", &["wiki/folder"]),
        (
            "wiki/file",
            "owner",
            &["wiki/event", "wiki/folder", "wiki/group"],
        ),
        // Speaker lists (talerlister): owner-managed, one or more per context.
        (
            "speak/list",
            "owner",
            &["wiki/event", "wiki/group", "wiki/folder"],
        ),
    ];
    let objs: Vec<serde_json::Value> = rules
        .iter()
        .map(|(mime, role, parents)| {
            let mutable_row = *mime != "vote/vote";
            serde_json::json!({
                "contextId": ctx_id,
                "nodeId": ctx_id,
                "mimeId": mime,
                "role": role,
                "parents": parents,
                "active": true,
                "insert": true,
                "select": true,
                "update": mutable_row,
                "delete": mutable_row,
            })
        })
        .collect();
    serde_json::Value::Array(objs)
}

/// The parent mimes a `vote/reaction` may be created under. Reactions attach to
/// comments today; the wider content set keeps them consistent with `vote/comment`
/// and ready for reactions placed directly on content later.
pub(crate) const REACTION_PARENTS: &[&str] = &[
    "vote/policy",
    "vote/change",
    "wiki/document",
    "wiki/file",
    "vote/position",
    "vote/candidate",
    "vote/comment",
];

/// Create a new context node — a group (`wiki/group`) or event (`wiki/event`) —
/// under `parent_id`, mirroring the old wiki's create-context flow. First it
/// inserts the node in the *parent's* context, so the server-side `insert`
/// permission (which gates on `node.context_id = permission.context_id`) passes;
/// then it flips the node to be its own context and locks it (`mutable = false`);
/// then it seeds the per-context permission template so content can be added
/// under it; finally it seeds `creator` as the context's first OWNER member —
/// the backend's `isContextOwner` reads owner member rows, so without this the
/// creator would not be a context owner of their own group (and the owner-only
/// surfaces — members, console — would hide from them). Returns the new node's
/// id + key.
pub async fn create_context(
    access_token: Option<&str>,
    parent_id: &str,
    parent_context_id: &str,
    mime_id: &str,
    name: &str,
    creator: Option<&crate::session::User>,
) -> Result<model::InsertedNode, String> {
    // No `key` parameter: the caller would only have slugified the name, and the
    // key it lands on depends on what is already there.
    let inserted = insert_node_named(
        access_token,
        model::NodesInsertInput {
            name: Some(name.to_string()),
            key: None,
            mime_id: Some(mime_id.to_string()),
            parent_id: Some(model::Uuid(parent_id.to_string())),
            context_id: Some(model::Uuid(parent_context_id.to_string())),
            data: None,
            mutable: Some(true),
            index: None,
            created_at: None,
        },
        name,
    )
    .await?
    .ok_or("insert returned no node")?;
    let id = inserted.id.0.clone();
    // Become its own context (locked, like the old wiki's create).
    update_node(
        access_token,
        &id,
        model::NodesSetInput {
            context_id: Some(model::Uuid(id.clone())),
            mutable: Some(false),
            ..Default::default()
        },
    )
    .await?;
    // Seed the permission template so the context is actually usable.
    execute_raw_vars(
        access_token,
        "mutation($objs: [permissions_insert_input!]!) { insertPermissions(objects: $objs) { affected_rows } }",
        serde_json::json!({ "objs": context_permission_objects(&id) }),
    )
    .await?;
    // Seed the creator as the context's first owner member (see the doc above).
    if let Some(user) = creator {
        execute_raw_vars(
            access_token,
            "mutation($objs: [members_insert_input!]!) { insertMembers(objects: $objs) { affected_rows } }",
            serde_json::json!({ "objs": [{
                "parentId": id,
                "nodeId": user.id,
                "email": user.email,
                "name": user.display_name,
                "owner": true,
                "accepted": true,
                "active": true,
            }] }),
        )
        .await?;
    }
    Ok(inserted)
}

/// Create a new speaker list (`speak/list`) under `context_id`. A context can hold
/// several lists. Older contexts predate the speak/list permission, so — since the
/// owner may seed permissions for their own context — this first grants speak/list
/// there if it is missing, then inserts the list.
pub async fn create_speaker_list(
    access_token: Option<&str>,
    context_id: &str,
    name: &str,
) -> Result<model::InsertedNode, String> {
    let can_insert = node_insert_mimes(access_token, context_id)
        .await
        .iter()
        .any(|m| m == "speak/list");
    if !can_insert {
        execute_raw_vars(
            access_token,
            "mutation($objs: [permissions_insert_input!]!) { insertPermissions(objects: $objs) { affected_rows } }",
            serde_json::json!({ "objs": [{
                "contextId": context_id,
                "nodeId": context_id,
                "mimeId": "speak/list",
                "role": "owner",
                "parents": ["wiki/event", "wiki/group", "wiki/folder"],
                "active": true,
                "insert": true,
                "select": true,
                "update": true,
                "delete": true,
            }] }),
        )
        .await?;
    }
    insert_node_named(
        access_token,
        model::NodesInsertInput {
            name: Some(name.to_string()),
            key: None,
            mime_id: Some("speak/list".to_string()),
            parent_id: Some(model::Uuid(context_id.to_string())),
            context_id: Some(model::Uuid(context_id.to_string())),
            data: None,
            mutable: Some(true),
            index: None,
            created_at: None,
        },
        name,
    )
    .await?
    .ok_or_else(|| "insert returned no list".to_string())
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
        // A copy may land beside its source, so the root's key can collide; a
        // child's cannot, sitting under a fresh parent. `insert_node_named` takes
        // the plain name and counts up only if it has to, so a second copy reads
        // `budget-2` rather than carrying six digits of clock.
        let input = model::NodesInsertInput {
            name: Some(node.name.clone()),
            key: (!is_root).then(|| node.key.clone()),
            mime_id: node.mime_id.clone(),
            parent_id: Some(model::Uuid(parent_id.clone())),
            context_id: context_id.clone().map(model::Uuid),
            data: node.data.clone(),
            mutable: Some(node.mutable),
            index: Some(node.index),
            // Keep the original's date. A copy is the same content in a new
            // place, not new content: without this it took `now()` and sorted
            // to the end of the folder as the newest item, dated today.
            created_at: node.created_at.clone(),
        };
        let inserted = if is_root {
            insert_node_named(access_token.as_deref(), input, &node.name).await?
        } else {
            insert_node(access_token.as_deref(), input).await?
        };
        let new_id = match inserted {
            Some(inserted) => inserted.id.0,
            None => return Ok(()),
        };
        // Copy the members onto the new node.
        for m in &node.members {
            use cynic::MutationBuilder;
            let object = MembersInsertInput {
                name: m.name.clone(),
                email: m.email.clone(),
                node_id: m.node_id.clone().map(|u| Uuid(u.0)),
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
    set: model::NodesSetInput,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = UpdateNodeMutation::build(UpdateNodeVariables {
        pk: NodesPkColumnsInput {
            id: Uuid(id.to_string()),
        },
        set: set.into(),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.update_node.is_some())
}

/// Delete a node by ID
/// Delete a node and everything under it, deepest first.
///
/// Plain [`delete_node`] removes one row. Nothing cleans up after it — there is
/// no foreign key on `parent_id` and so no cascade — so every child was left
/// pointing at an id that no longer resolves. That is where the orphans in the
/// missing-parent view come from: delete a comment and its replies and its
/// reactions simply stay, unreachable.
///
/// Children go first so a failure part-way leaves a subtree that is still
/// reachable from its parent, rather than a floating one. Members are removed
/// with each node, as the single-node callers already do. Depth-bounded: the
/// tree is shallow, and a cycle must not become an endless walk.
pub fn delete_node_deep(
    access_token: Option<String>,
    id: String,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>>>> {
    delete_node_deep_bounded(access_token, id, 32)
}

pub(crate) fn delete_node_deep_bounded(
    access_token: Option<String>,
    id: String,
    depth: u32,
) -> std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), String>>>> {
    Box::pin(async move {
        if depth == 0 {
            return Err("delete: tree deeper than expected".to_string());
        }
        for child in child_ids(access_token.as_deref(), &id).await? {
            delete_node_deep_bounded(access_token.clone(), child, depth - 1).await?;
        }
        delete_node_members(access_token.as_deref(), &id).await?;
        delete_node(access_token.as_deref(), &id).await?;
        Ok(())
    })
}

/// The ids of a node's direct children, whatever their mime.
pub(crate) async fn child_ids(
    access_token: Option<&str>,
    parent_id: &str,
) -> Result<Vec<String>, String> {
    use cynic::QueryBuilder;
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(parent_id.to_string())),
            is_null: None,
        }),
        ..Default::default()
    };
    let op = ChildIdsQuery::build(NodesWhereVariables { where_clause });
    let data = execute(access_token, op).await?;
    Ok(data.nodes.into_iter().map(|n| n.id.0).collect())
}

pub async fn delete_node(access_token: Option<&str>, id: &str) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = DeleteNodeMutation::build(DeleteNodeVariables {
        id: Uuid(id.to_string()),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.delete_node.is_some())
}

/// Every `vote/poll` in a context, newest first — for the admin results grid.
pub async fn query_context_polls(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Vec<model::PollSummaryFields>, String> {
    let where_clause = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                context_id: Some(UuidComparisonExp {
                    in_: None,
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
    Ok(result.nodes.into_iter().map(Into::into).collect())
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
                    in_: None,
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

/// The URL path of a node, as the segments the router wants.
///
/// Walks parents up to the root, whose key is not a segment. Nodes are addressed
/// by path, not id, and there is no id route to fall back on — so a chip that
/// wants to link to a group has to ask.
///
/// One request per level, which is why the caller should do this on CLICK rather
/// than while rendering: a page can carry many author chips and most are never
/// followed.
pub async fn node_path(access_token: Option<&str>, node_id: &str) -> Vec<String> {
    // A path deeper than this is a cycle or a mistake; either way, stop.
    const MAX_DEPTH: usize = 12;
    let mut segments = Vec::new();
    let mut current = Some(node_id.to_string());
    for _ in 0..MAX_DEPTH {
        let Some(id) = current.take() else { break };
        let Ok(Some(node)) = query_node_by_id(access_token, &id).await else {
            break;
        };
        // The root is the path's origin, not a step in it.
        let Some(parent) = node.parent_id.as_ref().map(|p| p.0.clone()) else {
            break;
        };
        segments.push(node.key.clone());
        current = Some(parent);
    }
    segments.reverse();
    segments
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

/// Build the `where` filter for a node's visible children, mirroring the React
/// drawer (`DrawerList`): children the user may see (immutable, or owned, or a
/// member of) whose mime is not hidden.
/// A node is "visible" to the user when it is published, owned by them, or one
/// they are a member of. Shared by the drawer's child query and its per-row
/// `children_aggregate` count.
pub(crate) fn visible_to_user(user_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        or: Some(vec![
            NodesBoolExp {
                mutable: Some(BooleanComparisonExp { eq: Some(false) }),
                ..Default::default()
            },
            NodesBoolExp {
                owner_id: Some(UuidComparisonExp {
                    in_: None,
                    eq: Some(Uuid(user_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
            NodesBoolExp {
                members: Some(MembersBoolExp {
                    node_id: Some(UuidComparisonExp {
                        in_: None,
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
pub(crate) fn mime_not_hidden() -> NodesBoolExp {
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
pub(crate) fn child_visibility_clause(user_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        and: Some(vec![visible_to_user(user_id), mime_not_hidden()]),
        ..Default::default()
    }
}

pub(crate) fn children_where_clause(parent_id: &str, user_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                parent_id: Some(UuidComparisonExp {
                    in_: None,
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
) -> Result<Vec<model::ChildNodeFields>, String> {
    let where_clause = children_where_clause(parent_id, user_id);
    let order_by = drawer_child_order();
    let operation = ChildrenQuery::build(ChildrenVariables {
        where_clause,
        order_by: Some(order_by),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes.into_iter().map(Into::into).collect())
}

/// The drawer-tree variant of `query_children`: same visible-children filter and
/// ordering, but each row also carries a `children_aggregate` count so the
/// expander chevron only shows for nodes that actually have visible children.
pub async fn query_drawer_children(
    access_token: Option<&str>,
    parent_id: &str,
    user_id: &str,
) -> Result<Vec<model::DrawerChildFields>, String> {
    let where_clause = children_where_clause(parent_id, user_id);
    let child_visible = child_visibility_clause(user_id);
    let order_by = drawer_child_order();
    let operation = DrawerChildrenQuery::build(DrawerChildrenVariables {
        where_clause,
        order_by: Some(order_by),
        child_visible,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes.into_iter().map(Into::into).collect())
}

/// Shared ordering for a node's children: by explicit index, then creation time
/// (the folder view's order).
pub(crate) fn drawer_child_order() -> Vec<NodesOrderBy> {
    vec![
        NodesOrderBy {
            index: Some(OrderBy::Asc),
            created_at: None,
            id: None,
        },
        NodesOrderBy {
            index: None,
            created_at: Some(OrderBy::Asc),
            id: None,
        },
    ]
}

pub async fn path_from_id(access_token: Option<&str>, id: &str) -> Result<Vec<String>, String> {
    // One query: the node stores its own trail (`nodes.path`, kept by a database
    // trigger). This used to climb parent by parent, a round trip per level, on
    // every feed row, search result and contribution that someone opened.
    let where_clause = NodesBoolExp {
        id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(id.to_string())),
            is_null: None,
        }),
        ..Default::default()
    };
    let op = NodesWhereQuery::build(NodesLimitVariables {
        where_clause,
        limit: None,
    });
    let found = execute(access_token, op).await?.nodes.into_iter().next();
    let segments: Vec<String> = found
        .and_then(|n| n.path)
        .filter(|p| !p.is_empty())
        .map(|p| p.split('/').map(str::to_string).collect())
        .unwrap_or_default();
    Ok(segments)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The rows that only draw an icon must ask for the icon, not the document.
    ///
    /// A drawer row and a home-screen row read exactly one thing out of `data`:
    /// a file's content type, for the glyph. Selecting the whole jsonb sent the
    /// entire Slate document of every sibling — measured on the widest folder in
    /// production (48 children), 54 KB where 11 KB carries the same screen. The
    /// argument is one token and easy to drop in a refactor, so it is asserted
    /// on the operation as actually built.
    #[test]
    fn the_icon_only_queries_select_inside_the_document() {
        use cynic::QueryBuilder;

        let drawer = DrawerChildrenQuery::build(DrawerChildrenVariables {
            where_clause: NodesBoolExp::default(),
            order_by: None,
            child_visible: NodesBoolExp::default(),
        });
        assert!(
            drawer.query.contains(r#"data(path: "type")"#),
            "the drawer must not fetch whole documents: {}",
            drawer.query
        );

        let contexts = ContextsWhereQuery::build(NodesWhereVariables {
            where_clause: NodesBoolExp::default(),
        });
        assert!(
            contexts.query.contains(r#"data(path: "type")"#),
            "the context list must not fetch whole documents: {}",
            contexts.query
        );
    }

    /// What comes back from `data(path: "type")` is the VALUE, so it has to be
    /// wrapped again for the icon helper — which is the whole point of doing it:
    /// one glyph rule for search hits, drawer rows and folder rows alike.
    #[test]
    fn a_path_selected_type_is_rewrapped_for_the_icon_helper() {
        let wrapped = icon_data(Some(Jsonb(serde_json::json!("application/pdf"))))
            .expect("a present type stays present");
        assert_eq!(
            crate::components::loader::node_icon_mime_id("wiki/file", Some(&wrapped.0)),
            "application/pdf"
        );
        // A node with no type at all keeps its own mime as the icon.
        assert!(icon_data(None).is_none());
        assert_eq!(
            crate::components::loader::node_icon_mime_id("wiki/file", None),
            "wiki/file"
        );
    }

    /// The real message a taken name produces, captured from production on
    /// 2026-08-01. It is the PATH index that fires, not the key index, and the
    /// retry that finds a free name depends on recognising it.
    #[test]
    fn a_taken_name_is_recognised_by_the_index_that_actually_fires() {
        let path_idx = "graphql error [InsertNodeMutation]: Uniqueness violation. \
             duplicate key value violates unique constraint \"nodes_path_live_idx\"";
        assert!(super::is_key_taken(path_idx));
        let key_idx = "Uniqueness violation. duplicate key value violates unique \
             constraint \"nodes_parent_id_namespace_key\"";
        assert!(super::is_key_taken(key_idx));
    }

    /// Anything else must NOT be swallowed as a name collision: the retry would
    /// burn twenty inserts on an error no new key can fix, and then report the
    /// twentieth instead of the first.
    #[test]
    fn another_failure_is_not_a_taken_name() {
        assert!(!super::is_key_taken(
            "permission has failed: insert on nodes"
        ));
        assert!(!super::is_key_taken(
            "Uniqueness violation. duplicate value violates unique constraint \"members_pkey\""
        ));
        assert!(!super::is_key_taken("rate limited: retry_after_ms=54977"));
    }
}
