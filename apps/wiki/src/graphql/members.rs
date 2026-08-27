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

#[derive(cynic::InputObject, Debug, Default, Clone)]
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
    pub id: Option<UuidComparisonExp>,
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
    #[cynic(rename = "membersAggregate")]
    #[arguments(where: $where_clause)]
    pub members_aggregate: MembersAggregate,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_aggregate"
)]
pub struct MembersAggregate {
    pub aggregate: Option<MembersAggregateFields>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_aggregate_fields"
)]
pub struct MembersAggregateFields {
    pub count: i32,
}

/// Count the active members of a context (its eligible voters), for poll turnout.
///
/// Asks the server for a number rather than for the rows. The earlier form
/// selected every member id and took `.len()`, which on the largest context here
/// (1001 members) meant 46 KB over the wire to learn one integer; the aggregate
/// is 0.1 KB. Turnout is shown on every poll, so this runs on a hot path.
pub async fn count_active_members(access_token: Option<&str>, context_id: &str) -> usize {
    use cynic::QueryBuilder;
    let op = MembersCountQuery::build(MembersCountVariables {
        where_clause: MembersBoolExp {
            parent_id: Some(UuidComparisonExp {
                neq: None,
                in_: None,
                eq: Some(Uuid(context_id.to_string())),
                is_null: None,
            }),
            active: Some(BooleanComparisonExp { eq: Some(true) }),
            ..Default::default()
        },
    });
    execute(access_token, op)
        .await
        .ok()
        .and_then(|r| r.members_aggregate.aggregate)
        .map(|a| a.count.max(0) as usize)
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

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_constraint",
    rename_all = "snake_case"
)]
#[expect(
    clippy::enum_variant_names,
    reason = "the variants are the database's constraint names; cynic derives the wire name from the Rust one, so they cannot be shortened"
)]
pub enum MembersConstraint {
    MembersParentIdEmailKey,
    MembersParentIdNameEmailNodeIdKey,
    MembersParentIdNodeIdKey,
    MembersPkey,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_update_column",
    rename_all = "camelCase"
)]
pub enum MembersUpdateColumn {
    Accepted,
    Active,
    Email,
    Hidden,
    Id,
    Name,
    NodeId,
    Owner,
    ParentId,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_on_conflict"
)]
pub struct MembersOnConflict {
    pub constraint: MembersConstraint,
    // Hasura's on_conflict meta-field stays snake_case (unlike the camelCase
    // column fields), so keep cynic from rewriting it to `updateColumns`.
    #[cynic(rename = "update_columns")]
    pub update_columns: Vec<MembersUpdateColumn>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct InsertMembersVariables {
    pub objects: Vec<MembersInsertInput>,
    /// Nullable so the same mutation can be sent without it, which is the
    /// fallback when Hasura will not let this role upsert at all.
    pub on_conflict: Option<MembersOnConflict>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "InsertMembersVariables"
)]
pub struct InsertMembersMutation {
    #[arguments(objects: $objects, on_conflict: $on_conflict)]
    pub insert_members: Option<MembersAffected>,
}

/// What a roster import did. `skipped` names people this context had already
/// invited: a roster says who belongs here, not that none of them are here
/// yet, so they are passed over rather than failing the import.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct RosterImport {
    pub inserted: usize,
    pub skipped: usize,
    /// Of those inserted, how many the file gave no address for. On the roster,
    /// but not invitable until someone fills one in.
    pub without_email: usize,
}

/// The rows worth sending: anything naming a person or an address, lowercased,
/// with each address sent once. Deduplicating here rather than at the database
/// keeps `skipped` meaningful, since a file listing someone twice is a
/// duplicate the importer can see for itself.
fn roster_objects(parent_id: &str, roster: &[(String, String)]) -> Vec<MembersInsertInput> {
    let mut seen = std::collections::HashSet::new();
    roster
        .iter()
        .filter_map(|(name, email)| {
            let name = name.trim();
            let email = email.trim().to_lowercase();
            // Only addresses can collide, so only addresses are deduplicated.
            // Two rows sharing a name and no address are two people, which is
            // how the database's unique index reads them too.
            if !email.is_empty() && !seen.insert(email.clone()) {
                return None;
            }
            if name.is_empty() && email.is_empty() {
                return None;
            }
            Some(MembersInsertInput {
                name: (!name.is_empty()).then(|| name.to_string()),
                email: (!email.is_empty()).then_some(email),
                parent_id: Some(Uuid(parent_id.to_string())),
                ..Default::default()
            })
        })
        .collect()
}

/// Bulk-invite members from an imported roster: one email invite per `(name,
/// email)` pair in a single `insertMembers`.
///
/// Anyone already invited to this context is skipped by the database rather
/// than aborting the insert. Without that, one familiar face in a spreadsheet
/// of two hundred failed the whole import on
/// `members_parent_id_email_key` and nobody was invited at all.
pub async fn invite_members(
    access_token: Option<&str>,
    parent_id: &str,
    roster: &[(String, String)],
) -> Result<RosterImport, String> {
    use cynic::MutationBuilder;
    let objects = roster_objects(parent_id, roster);
    let submitted = objects.len();
    // Counted before the objects are sent: a row with no address never
    // conflicts, so every one of these is among the rows that go in.
    let without_email = objects.iter().filter(|o| o.email.is_none()).count();
    if submitted == 0 {
        return Ok(RosterImport::default());
    }
    let op = InsertMembersMutation::build(InsertMembersVariables {
        objects,
        // No columns to update: a conflict means "leave the existing invite
        // alone", not "overwrite it with the spreadsheet".
        on_conflict: Some(MembersOnConflict {
            constraint: MembersConstraint::MembersParentIdEmailKey,
            update_columns: Vec::new(),
        }),
    });
    let affected = match execute(access_token, op).await {
        Ok(r) => r.insert_members,
        // Hasura refuses `on_conflict` outright for a role it will not let
        // upsert. Retrying without it keeps a clean roster importing as it did
        // before, rather than making a permission an import cannot check into
        // an import that never works.
        Err(e) if e.to_lowercase().contains("upsert") => {
            crate::errors::log_handled("roster upsert refused, retrying plain", &e);
            let retry = InsertMembersMutation::build(InsertMembersVariables {
                objects: roster_objects(parent_id, roster),
                on_conflict: None,
            });
            execute(access_token, retry).await?.insert_members
        }
        Err(e) => return Err(e),
    };
    let inserted = affected
        .map(|m| m.affected_rows.max(0) as usize)
        .unwrap_or(0)
        .min(submitted);
    Ok(RosterImport {
        inserted,
        skipped: submitted - inserted,
        without_email: without_email.min(inserted),
    })
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
                            neq: None,
                            in_: None,
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

/// Whether the user is an active member of a context, for the voting-rights
/// banner.
///
/// `None` means "could not tell", and is NOT "no". It used to answer `false` on
/// a failed query, which is a confident "you have no voting rights" shown to a
/// member whose network hiccuped. Callers must treat `None` as unknown and say
/// nothing.
///
/// Advisory, never a gate. It answers one of the four things
/// `insert_with_email_invites` (migrations/0015) asks before it accepts a
/// ballot: there must also be a `vote/vote` permission in the context, the poll
/// must be open, and its parent must be attachable. And it is narrower than
/// even that one arm, because the rule matches a member row by `nodeId` OR by
/// `email` and this matches only `nodeId`, so someone invited by an address they
/// have not yet linked reads as a stranger here and is not. The server decides;
/// this only warns.
pub async fn is_active_member(
    access_token: Option<&str>,
    context_id: &str,
    user_id: &str,
) -> Option<bool> {
    use cynic::QueryBuilder;
    let where_clause = MembersBoolExp {
        parent_id: Some(UuidComparisonExp {
            neq: None,
            in_: None,
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
        node_id: Some(UuidComparisonExp {
            neq: None,
            in_: None,
            eq: Some(Uuid(user_id.to_string())),
            is_null: None,
        }),
        active: Some(BooleanComparisonExp { eq: Some(true) }),
        ..Default::default()
    };
    let op = MembersExistQuery::build(MembersExistVariables { where_clause });
    execute(access_token, op)
        .await
        .ok()
        .map(|r| !r.members.is_empty())
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

/// "Is this person already a member of this place, on some OTHER row?"
///
/// `invitation_id` is excluded, and that exclusion is the whole correctness of
/// this filter. An invitation may already carry the reader's node id -- adding
/// a member by an email that matches an account produces exactly that -- and
/// without the exclusion the invitation matches itself. The caller reads a hit
/// as "a separate membership exists" and deletes the invitation as a duplicate,
/// which leaves the person a member of nothing: they press accept and the place
/// disappears as though they had declined.
pub(crate) fn existing_member_where(
    parent_id: &str,
    node_id: &str,
    invitation_id: &str,
) -> MembersBoolExp {
    MembersBoolExp {
        parent_id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(parent_id.to_string())),
            neq: None,
            is_null: None,
        }),
        node_id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(node_id.to_string())),
            neq: None,
            is_null: None,
        }),
        id: Some(UuidComparisonExp {
            in_: None,
            eq: None,
            neq: Some(Uuid(invitation_id.to_string())),
            is_null: None,
        }),
        ..Default::default()
    }
}

/// Accept a pre-existing membership by `(parent, node)` instead of an email
/// invite row. The fallback when accepting an email invite would violate the
/// `(parentId, nodeId)` unique constraint because the user is already a member
/// of that context (React InvitesUserList's accept-then-fallback).
pub async fn accept_existing_member(
    access_token: Option<&str>,
    parent_id: &str,
    node_id: &str,
    invitation_id: &str,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let op = UpdateMembersMutation::build(UpdateMembersWhereVariables {
        where_clause: existing_member_where(parent_id, node_id, invitation_id),
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
                neq: None,
                in_: None,
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
                neq: None,
                in_: None,
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

#[cfg(test)]
mod tests {
    use super::*;

    fn roster(rows: &[(&str, &str)]) -> Vec<(String, String)> {
        rows.iter()
            .map(|(n, e)| ((*n).to_string(), (*e).to_string()))
            .collect()
    }

    /// The reported failure: a roster naming someone already invited aborted
    /// the whole insert on `members_parent_id_email_key`, so two hundred good
    /// rows were lost to one familiar face.
    #[test]
    fn a_roster_import_asks_the_database_to_skip_people_already_invited() {
        use cynic::MutationBuilder;
        let op = InsertMembersMutation::build(InsertMembersVariables {
            objects: roster_objects("ctx", &roster(&[("Ada", "ada@example.org")])),
            on_conflict: Some(MembersOnConflict {
                constraint: MembersConstraint::MembersParentIdEmailKey,
                update_columns: Vec::new(),
            }),
        });
        let q = &op.query;
        assert!(
            q.contains("on_conflict:"),
            "the insert handles conflicts: {q}"
        );
        let vars = serde_json::to_string(&op.variables).expect("variables serialize");
        assert!(
            vars.contains("members_parent_id_email_key"),
            "on the constraint that was failing: {vars}"
        );
        // Hasura's meta-field stays snake_case while columns are camelCase, and
        // an empty list is what makes a conflict skip rather than overwrite.
        assert!(
            vars.contains(r#""update_columns":[]"#),
            "leaving the existing invite alone: {vars}"
        );
    }

    /// The retry for a role Hasura will not let upsert sends the same mutation
    /// with no `on_conflict`, so the variable has to be declared nullable. A
    /// `members_on_conflict!` here would make that fallback unsendable.
    #[test]
    fn the_same_mutation_can_be_sent_without_an_on_conflict() {
        use cynic::MutationBuilder;
        let op = InsertMembersMutation::build(InsertMembersVariables {
            objects: roster_objects("ctx", &roster(&[("Ada", "ada@example.org")])),
            on_conflict: None,
        });
        assert!(
            op.query.contains("$onConflict: members_on_conflict)")
                || op.query.contains("$onConflict: members_on_conflict,"),
            "on_conflict must be nullable: {}",
            op.query
        );
    }

    #[test]
    fn a_roster_is_lowercased_and_each_address_sent_once() {
        let objects = roster_objects(
            "ctx",
            &roster(&[
                ("Ada", "Ada@Example.org"),
                ("Ada again", " ada@example.org "),
                ("No mail", "  "),
                ("Grace", "grace@example.org"),
            ]),
        );
        let emails: Vec<_> = objects.iter().filter_map(|o| o.email.clone()).collect();
        assert_eq!(emails, ["ada@example.org", "grace@example.org"]);
    }

    #[test]
    fn a_nameless_row_still_gets_its_invite() {
        let objects = roster_objects("ctx", &roster(&[("   ", "ada@example.org")]));
        assert_eq!(objects.len(), 1);
        assert_eq!(objects[0].name, None, "no name is not a reason to skip");
    }

    /// The reported case: a roster with a blank Email column. Those rows are
    /// members too, so they are sent with no address rather than dropped.
    #[test]
    fn rows_with_no_address_are_imported_without_one() {
        let objects = roster_objects(
            "ctx",
            &roster(&[
                ("Ada Lovelace", ""),
                ("Grace Hopper", "  "),
                ("", ""),
                ("Alan Turing", "alan@example.org"),
            ]),
        );
        let sent: Vec<_> = objects
            .iter()
            .map(|o| (o.name.as_deref(), o.email.as_deref()))
            .collect();
        assert_eq!(
            sent,
            [
                (Some("Ada Lovelace"), None),
                (Some("Grace Hopper"), None),
                (Some("Alan Turing"), Some("alan@example.org")),
            ],
            "a row with neither name nor address is the only one dropped"
        );
    }

    /// Two people can share a name; only an address is unique, and it is the
    /// only thing the database's index collides on.
    #[test]
    fn rows_without_an_address_are_not_deduplicated_by_name() {
        let objects = roster_objects(
            "ctx",
            &roster(&[("Anders Jensen", ""), ("Anders Jensen", "")]),
        );
        assert_eq!(objects.len(), 2, "both are imported: {objects:?}");
    }

    /// Turnout asks the server for a number, not for the members.
    ///
    /// The earlier form selected every member id and took `.len()`. On the
    /// largest context in production — 1001 members — that was 46 KB over the
    /// wire to learn one integer, on a query that runs for every poll shown.
    #[test]
    fn the_member_count_is_an_aggregate_and_selects_no_rows() {
        use cynic::QueryBuilder;
        let op = MembersCountQuery::build(MembersCountVariables {
            where_clause: MembersBoolExp::default(),
        });
        assert!(
            op.query.contains("membersAggregate"),
            "must count server-side: {}",
            op.query
        );
        assert!(
            !op.query.contains("nodes {"),
            "an aggregate must not drag rows along: {}",
            op.query
        );
    }
}
