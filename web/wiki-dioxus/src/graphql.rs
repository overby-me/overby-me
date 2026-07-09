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
    pub mime: Option<MimeFields>,
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
}

/// A membership row on a node — used as the author chips on documents and the
/// member list of a context (mirrors the React `MemberChips`).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "members")]
pub struct MemberFields {
    pub id: Uuid,
    pub name: Option<String>,
    pub accepted: bool,
    pub owner: bool,
    pub hidden: bool,
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
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mimes_bool_exp"
)]
pub struct MimesBoolExp {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub hidden: Option<BooleanComparisonExp>,
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
    pub email: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<UuidComparisonExp>,
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
    pub hidden: Option<bool>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<Uuid>,
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

pub async fn execute<Q, V>(
    access_token: Option<&str>,
    operation: cynic::Operation<Q, V>,
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
        .json(&operation)
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

/// Execute a raw GraphQL query/mutation string (for operations not covered by cynic types)
pub async fn execute_raw(
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

/// A resolved breadcrumb segment: its display name and mime (for the avatar).
#[derive(Debug, Clone, PartialEq)]
pub struct Crumb {
    pub name: String,
    pub mime_id: Option<String>,
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
            })
            .collect());
    };
    let mut parent_id: Option<String> = Some(root_id);
    let mut out = Vec::with_capacity(segments.len());
    for segment in segments {
        match query_node_by_key(access_token, segment, parent_id.as_deref()).await? {
            Some(n) => {
                out.push(Crumb {
                    name: n.name.clone(),
                    mime_id: n.mime_id.clone(),
                });
                parent_id = Some(n.id.0);
            }
            None => {
                out.push(Crumb {
                    name: segment.clone(),
                    mime_id: None,
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
) -> Result<Vec<NodeFields>, String> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    let where_clause = NodesBoolExp {
        and: Some(vec![NodesBoolExp {
            name: Some(StringComparisonExp {
                ilike: Some(format!("%{query}%")),
                ..Default::default()
            }),
            ..Default::default()
        }]),
        ..Default::default()
    };

    let operation = NodesWhereQuery::build(NodesWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes)
}

/// Build the `where` filter for the user's context nodes (groups or events) of
/// a given mime type: nodes the user owns or has an accepted membership in.
fn contexts_where_clause(user_id: &str, mime_id: &str) -> NodesBoolExp {
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
        and: Some(vec![
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    eq: Some(mime_id.to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
            NodesBoolExp {
                or: Some(vec![owned, member]),
                ..Default::default()
            },
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
fn children_where_clause(parent_id: &str, user_id: &str) -> NodesBoolExp {
    let visible = NodesBoolExp {
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
    };

    NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                parent_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(parent_id.to_string())),
                    is_null: None,
                }),
                ..Default::default()
            },
            visible,
            NodesBoolExp {
                mime: Some(MimesBoolExp {
                    hidden: Some(BooleanComparisonExp { eq: Some(false) }),
                }),
                ..Default::default()
            },
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
    let order_by = vec![
        NodesOrderBy {
            index: Some(OrderBy::Asc),
            created_at: None,
        },
        NodesOrderBy {
            index: None,
            created_at: Some(OrderBy::Asc),
        },
    ];
    let operation = ChildrenQuery::build(ChildrenVariables {
        where_clause,
        order_by: Some(order_by),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes)
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
