//! Membership: a context's roster, its owners, and the invitations that get
//! people onto it.

use super::*;

/// A membership row on a node — used as the author chips on documents and the
/// member list of a context (mirrors the React `MemberChips`).
#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "members")]
pub struct MemberFields {
    pub id: Uuid,
    pub name: Option<String>,
    // No `email` here, deliberately. This fragment rides along with every node
    // read, including a signed-out one, and the public role may not select a
    // member's email — which failed the WHOLE node query, so an anonymous
    // visitor could not open a single page of a public wiki. Nothing rendered
    // from a node's members wanted it: the roster that does (member.rs) asks
    // for it in its own query, as a signed-in owner.
    pub accepted: bool,
    pub active: bool,
    pub owner: bool,
    pub hidden: bool,
    pub node_id: Option<Uuid>,
    pub user: Option<UserRef>,
    pub node: Option<MemberNodeRef>,
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

#[derive(cynic::QueryVariables, Debug)]
pub struct MembersCountVariables {
    pub where_clause: MembersBoolExp,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "MembersCountVariables"
)]
pub struct MembersCountQuery {
    #[arguments(where: $where_clause)]
    pub members: Vec<MemberIdRef>,
}

/// Count the active members of a context (its eligible voters), for poll turnout.
/// The schema exposes no `members_aggregate`, so this fetches ids and counts them.
pub async fn count_active_members(access_token: Option<&str>, context_id: &str) -> usize {
    use cynic::QueryBuilder;
    let op = MembersCountQuery::build(MembersCountVariables {
        where_clause: MembersBoolExp {
            parent_id: Some(UuidComparisonExp {
                eq: Some(Uuid(context_id.to_string())),
                is_null: None,
            }),
            active: Some(BooleanComparisonExp { eq: Some(true) }),
            ..Default::default()
        },
    });
    execute(access_token, op)
        .await
        .map(|r| r.members.len())
        .unwrap_or(0)
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

/// Build the Hasura `where` object (as GraphQL literal text) for a member page.
pub(crate) fn members_where(parent_id: &str, f: &MemberPageFilter) -> String {
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

/// Parse one raw `members` JSON row into a [`MemberFields`].
pub(crate) fn parse_member_row(v: &serde_json::Value) -> Option<model::MemberFields> {
    let s = |k: &str| v.get(k).and_then(|x| x.as_str()).map(String::from);
    let b = |k: &str| v.get(k).and_then(|x| x.as_bool()).unwrap_or(false);
    Some(model::MemberFields {
        id: model::Uuid(v.get("id")?.as_str()?.to_string()),
        name: s("name"),
        email: s("email"),
        accepted: b("accepted"),
        active: b("active"),
        owner: b("owner"),
        hidden: b("hidden"),
        node_id: s("nodeId").map(model::Uuid),
        user: v.get("user").and_then(|u| {
            let display_name = u.get("displayName").and_then(|d| d.as_str())?;
            Some(model::UserRef {
                id: model::Uuid(
                    u.get("id")
                        .and_then(|d| d.as_str())
                        .unwrap_or_default()
                        .to_string(),
                ),
                display_name: display_name.to_string(),
                avatar_url: u
                    .get("avatarUrl")
                    .and_then(|d| d.as_str())
                    .unwrap_or_default()
                    .to_string(),
            })
        }),
        node: v.get("node").map(|n| model::MemberNodeRef {
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
pub(crate) fn invitations_where_clause(user_id: &str, email: &str) -> MembersBoolExp {
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
) -> Result<Vec<model::InvitationFields>, String> {
    let where_clause = invitations_where_clause(user_id, email);
    let operation = InvitationsQuery::build(MembersWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result
        .members
        .into_iter()
        .filter(|m| m.parent.is_some())
        .map(Into::into)
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
    set: model::MembersSetInput,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = UpdateMemberMutation::build(UpdateMemberVariables {
        pk: MembersPkColumnsInput {
            id: Uuid(member_id.to_string()),
        },
        set: set.into(),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.update_member.is_some())
}

/// Remove a member from a context (owner action) — the same mutation as
/// declining an invitation, named for the admin use.
pub async fn remove_member(access_token: Option<&str>, member_id: &str) -> Result<bool, String> {
    decline_invitation(access_token, member_id).await
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
