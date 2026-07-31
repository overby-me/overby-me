//! What has happened lately — the feed, a person's or a group's contributions,
//! and the context lists behind them.

use super::*;

// --- Query: recent nodes across the user's contexts (home "Newest") ---

#[derive(cynic::QueryVariables, Debug)]
pub struct RecentNodesVariables {
    pub where_clause: NodesBoolExp,
    pub order_by: Option<Vec<NodesOrderBy>>,
    pub limit: Option<i32>,
    /// How many rows to skip — the feed pages through by raising this.
    pub offset: Option<i32>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "RecentNodesVariables"
)]
pub struct RecentNodesQuery {
    #[arguments(where: $where_clause, order_by: $order_by, limit: $limit, offset: $offset)]
    pub nodes: Vec<ChildNodeFields>,
}

/// Count the nodes matching a filter via `nodes_aggregate` (the §1 aggregate the
/// poll-list vote badge and other counts build on).
pub async fn count_nodes(
    access_token: Option<&str>,
    where_clause: NodesBoolExp,
) -> Result<usize, String> {
    use cynic::QueryBuilder;
    let op = NodesCountQuery::build(NodesWhereVariables {
        where_clause,
    });
    let r = execute(access_token, op).await?;
    Ok(r.nodes_aggregate
        .aggregate
        .map(|a| a.count.max(0) as usize)
        .unwrap_or(0))
}

/// The content a GROUP is credited on, newest first.
///
/// A person's contributions are found by ownership ([`query_user_contributions`]),
/// which cannot work for a group: a group owns nothing, it is named as an author.
/// Authorship is a `members` row on the content pointing at the group, so that is
/// what this asks for.
///
/// Typed like every other node query rather than raw. `ChildNodeFields` has no
/// serde renaming, so deserialising camelCase JSON into it fails on the first
/// snake_case field and yields an EMPTY list — which reads as "this group has
/// contributed nothing" rather than as a bug.
pub async fn query_group_contributions(
    access_token: Option<&str>,
    group_id: &str,
    limit: i32,
) -> Vec<model::ChildNodeFields> {
    use cynic::QueryBuilder;
    let where_clause = NodesBoolExp {
        members: Some(MembersBoolExp {
            node_id: Some(UuidComparisonExp {
                eq: Some(Uuid(group_id.to_string())),
                is_null: None,
            }),
            ..Default::default()
        }),
        ..Default::default()
    };
    // Two entries rather than one object with two keys: Hasura does not promise
    // the order it applies the keys of a single order_by, and picked the id.
    let order_by = vec![
        NodesOrderBy {
            created_at: Some(OrderBy::Desc),
            index: None,
            id: None,
        },
        NodesOrderBy {
            created_at: None,
            index: None,
            id: Some(OrderBy::Desc),
        },
    ];
    let op = RecentNodesQuery::build(RecentNodesVariables {
        where_clause,
        order_by: Some(order_by),
        limit: Some(limit),
        offset: None,
    });
    match execute(access_token, op).await {
        Ok(d) => d.nodes.into_iter().map(Into::into).collect(),
        Err(_) => Vec::new(),
    }
}

/// The signed-in user's most recent contributions for the profile: the
/// meaningful content THEY authored (`owner_id`) — resolutions, amendments,
/// candidacies, comments and questions — newest first. Unlike
/// [`query_recent_nodes`] (membership-scoped "Newest"), this is
/// authorship-scoped.
pub async fn query_user_contributions(
    access_token: Option<&str>,
    user_id: &str,
    limit: i32,
) -> Vec<model::ChildNodeFields> {
    let where_clause = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                owner_id: Some(UuidComparisonExp {
                    eq: Some(Uuid(user_id.to_string())),
                    ..Default::default()
                }),
                ..Default::default()
            },
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    in_: Some(vec![
                        "vote/policy".to_string(),
                        "vote/change".to_string(),
                        "vote/candidate".to_string(),
                        "vote/comment".to_string(),
                        "vote/question".to_string(),
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    // Two entries, not one object with two fields: Hasura applies the keys of
    // a single order_by object in an order it does not promise, and it picked
    // the id first - sorting the feed by random UUID instead of by date.
    let order_by = vec![
        NodesOrderBy {
            created_at: Some(OrderBy::Desc),
            index: None,
            id: None,
        },
        NodesOrderBy {
            created_at: None,
            index: None,
            id: Some(OrderBy::Desc),
        },
    ];
    let op = RecentNodesQuery::build(RecentNodesVariables {
        where_clause,
        order_by: Some(order_by),
        limit: Some(limit),
        offset: None,
    });
    match execute(access_token, op).await {
        Ok(d) => d.nodes.into_iter().map(Into::into).collect(),
        Err(_) => Vec::new(),
    }
}

/// The most recently created content nodes for a feed (#34): submitted
/// (immutable) content, newest first. Drafts never appear.
///
/// `context_id` chooses the scope. `None` is the cross-context feed: everything
/// in a group or event the user belongs to. `Some(id)` is one context's own
/// feed, for the context's feed app.
///
/// A context node is its OWN context (`contextId == id`), so a group's feed
/// holds the group's own content and NOT its events' — an event under a group is
/// its own context. Rolling those up would need the ancestor chain, which this
/// column cannot express.
/// The feed's predicate: which nodes count as activity worth listing.
///
/// Pure and separate from the request so it can be read and tested on its own —
/// it is the part of the feed with the actual meaning in it.
pub(crate) fn recent_where_clause(user_id: &str, context_id: Option<&str>) -> NodesBoolExp {
    NodesBoolExp {
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
                        // Comments and reactions belong in the feed too — they are
                        // activity in the same contexts. Their rows show what they
                        // are about and open the thread's host (see RecentItem).
                        "vote/comment".to_string(),
                        "vote/reaction".to_string(),
                    ]),
                    ..Default::default()
                }),
                ..Default::default()
            },
            // Submitted only — drafts (mutable) never appear in Newest. Comments
            // are written immutable, so this does not exclude them.
            NodesBoolExp {
                mutable: Some(BooleanComparisonExp { eq: Some(false) }),
                ..Default::default()
            },
            // Scope: one context's own content, or (unscoped) everything in a
            // context the user belongs to. Inside a context the membership test
            // is redundant — you are reading its page — so the id alone stands.
            match context_id {
                // Everything that happened UNDER this context, not only what
                // carries it as its own. A group holds events, an event's content
                // belongs to the event, so a group's feed — filtered on the id
                // alone — was blind to the meetings that are the reason the group
                // exists: 52 items where 2924 had happened, at the time this was
                // written. The id test stays alongside, for anything filed with
                // this context but sitting elsewhere in the tree.
                Some(id) => NodesBoolExp {
                    or: Some(vec![
                        NodesBoolExp {
                            context_id: Some(UuidComparisonExp {
                                eq: Some(Uuid(id.to_string())),
                                is_null: None,
                            }),
                            ..Default::default()
                        },
                        NodesBoolExp {
                            ancestors: Some(UuidArrayComparisonExp {
                                contains: Some(vec![Uuid(id.to_string())]),
                            }),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                },
                None => NodesBoolExp {
                    context: Some(Box::new(belongs_to_user(user_id))),
                    ..Default::default()
                },
            },
            // Its parent must still exist. An orphan (see `query_orphans`) has
            // nowhere to open — the row would quote a comment that is gone, or
            // resolve to no path at all.
            NodesBoolExp {
                parent: Some(Box::new(NodesBoolExp::default())),
                ..Default::default()
            },
        ]),
        ..Default::default()
    }
}

pub async fn query_recent_nodes(
    access_token: Option<&str>,
    limit: i32,
    offset: i32,
    user_id: &str,
    context_id: Option<&str>,
) -> Vec<model::ChildNodeFields> {
    let where_clause = recent_where_clause(user_id, context_id);
    // Two entries, not one object with two fields: Hasura applies the keys of
    // a single order_by object in an order it does not promise, and it picked
    // the id first - sorting the feed by random UUID instead of by date.
    let order_by = vec![
        NodesOrderBy {
            created_at: Some(OrderBy::Desc),
            index: None,
            id: None,
        },
        NodesOrderBy {
            created_at: None,
            index: None,
            id: Some(OrderBy::Desc),
        },
    ];
    let op = RecentNodesQuery::build(RecentNodesVariables {
        where_clause,
        order_by: Some(order_by),
        limit: Some(limit),
        offset: Some(offset),
    });
    match execute(access_token, op).await {
        Ok(d) => d.nodes.into_iter().map(Into::into).collect(),
        Err(_) => Vec::new(),
    }
}

/// Build the `where` filter for the user's context nodes (groups or events) of
/// a given mime type: nodes the user owns or has an accepted membership in.
/// Nodes the user "belongs to": ones they own or have an accepted membership in.
/// Shared by the contexts list and the "Newest" list's context filter.
pub(crate) fn belongs_to_user(user_id: &str) -> NodesBoolExp {
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

pub(crate) fn contexts_where_clause(user_id: &str, mime_id: &str) -> NodesBoolExp {
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
) -> Result<Vec<model::ContextNodeFields>, String> {
    let where_clause = contexts_where_clause(user_id, mime_id);
    let operation = ContextsWhereQuery::build(NodesWhereVariables {
        where_clause,
    });
    let mut result = execute(access_token, operation).await?;
    // Newest first (the API returns no guaranteed order).
    result.nodes.sort_by(|a, b| {
        let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
        b_ts.cmp(a_ts)
    });
    Ok(result.nodes.into_iter().map(Into::into).collect())
}

/// Nodes with a missing parent (`parentId is null`) — the "Missing parent" admin
/// view (#149). The single legitimate root is one of these, so callers filter it
/// out; anything else is an orphan that lost its parent.
pub async fn query_orphans(
    access_token: Option<&str>,
) -> Result<Vec<model::ContextNodeFields>, String> {
    let where_clause = NodesBoolExp {
        or: Some(vec![
            // Parentless outright. Only the home node should be here, and the
            // caller filters it out.
            NodesBoolExp {
                parent_id: Some(UuidComparisonExp {
                    is_null: Some(true),
                    eq: None,
                }),
                ..Default::default()
            },
            // The real orphans: a parent id pointing at a row that no longer
            // exists. Deleting a node does not null its children's parent_id —
            // there is no foreign key — so this, not `is_null`, is what losing a
            // parent actually looks like. A reaction outlives the comment it was
            // on this way.
            NodesBoolExp {
                parent_id: Some(UuidComparisonExp {
                    is_null: Some(false),
                    eq: None,
                }),
                not: Some(Box::new(NodesBoolExp {
                    parent: Some(Box::new(NodesBoolExp::default())),
                    ..Default::default()
                })),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    let operation = ContextsWhereQuery::build(NodesWhereVariables {
        where_clause,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.nodes.into_iter().map(Into::into).collect())
}
