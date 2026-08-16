//! Finding things and people: node search over titles and body text, and the
//! user lookups the author pickers need.

use super::*;

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "users")]
pub struct UserRef {
    /// The user's id, used to link to their profile (`/profile/:id`).
    pub id: Uuid,
    pub display_name: String,
    /// The user's avatar URL (gravatar by default, their Bluesky picture once
    /// linked). Readable via the `user` role's `users` select permission.
    pub avatar_url: String,
}

/// Search nodes by name (case-insensitive substring match)
pub async fn search_nodes(
    access_token: Option<&str>,
    query: &str,
    context_id: Option<&str>,
) -> Result<Vec<model::NodeFields>, String> {
    if query.is_empty() {
        return Ok(vec![]);
    }

    // Split into terms so a multi-word query matches when the words appear in any
    // order (each term must be in the title or the body), not only contiguously —
    // "budget klima" then finds "Klima og budget". Keeps substring (ilike) matching,
    // which a pure-tsvector switch would lose.
    let patterns: Vec<String> = {
        let terms: Vec<&str> = query.split_whitespace().collect();
        if terms.is_empty() {
            vec![format!("%{query}%")]
        } else {
            terms.iter().map(|t| format!("%{t}%")).collect()
        }
    };
    let term_filters = |titles_only: bool| -> Vec<NodesBoolExp> {
        patterns
            .iter()
            .map(|like| {
                let in_name = NodesBoolExp {
                    name: Some(StringComparisonExp {
                        ilike: Some(like.clone()),
                        ..Default::default()
                    }),
                    ..Default::default()
                };
                if titles_only {
                    return in_name;
                }
                // This term must appear in the title OR the extracted body.
                NodesBoolExp {
                    or: Some(vec![
                        in_name,
                        NodesBoolExp {
                            content_text: Some(StringComparisonExp {
                                ilike: Some(like.clone()),
                                ..Default::default()
                            }),
                            ..Default::default()
                        },
                    ]),
                    ..Default::default()
                }
            })
            .collect()
    };
    let mut filters: Vec<NodesBoolExp> = Vec::new();
    // Exclude orphan/root nodes (no parent), matching React's search.
    filters.push(NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            in_: None,
            eq: None,
            is_null: Some(false),
        }),
        ..Default::default()
    });
    // Hide system/hidden mimes unless they are contexts (groups/events):
    // `mime.hidden = false OR mime.context = true`.
    filters.push(NodesBoolExp {
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
    });
    // Scope the search to a single context (group/event) when requested.
    if let Some(ctx) = context_id {
        filters.push(NodesBoolExp {
            context_id: Some(UuidComparisonExp {
                in_: None,
                eq: Some(Uuid(ctx.to_string())),
                is_null: None,
            }),
            ..Default::default()
        });
    }
    // The shared half — not an orphan, not a hidden mime, inside the scope — is
    // the same for both searches below.
    let common = filters;
    let clause = |titles_only: bool| {
        let mut all = term_filters(titles_only);
        all.extend(common.iter().cloned());
        NodesBoolExp {
            and: Some(all),
            ..Default::default()
        }
    };

    // A search box shows a page of hits, not every hit: unbounded, this answered
    // 407 rows and 1.5 MB for three letters, because each row carries its whole
    // document. Thirty is more than anyone reads before retyping.
    // TWO searches, not one, and the reason is selection rather than order.
    // Hasura applies the cap with no ordering, so among many body matches a
    // TITLE match could simply not be in the thirty that came back — searching
    // "Uddan" found a candidate whose text mentions Uddannelsesordfører while the
    // node actually called Uddannelsesordfører was missing. Asking for titles
    // separately guarantees them a place; both are indexed (name and content_text
    // each have a trigram index), and they run at the same time.
    let titles_op = NodesSearchQuery::build(NodesLimitVariables {
        where_clause: clause(true),
        limit: Some(30),
    });
    let all_op = NodesSearchQuery::build(NodesLimitVariables {
        where_clause: clause(false),
        limit: Some(30),
    });
    let (titles, all) = futures_util::join!(
        execute(access_token, titles_op),
        execute(access_token, all_op)
    );
    let mut nodes = titles?.nodes;
    let mut seen: std::collections::HashSet<String> =
        nodes.iter().map(|n| n.id.0.clone()).collect();
    for node in all?.nodes {
        if seen.insert(node.id.0.clone()) {
            nodes.push(node);
        }
    }
    // Rank by WHERE the match is, then by recency inside each rank. A title is
    // what a thing is called; a body mention is a thing that talks about it.
    nodes.sort_by(|a, b| {
        let rank = name_rank(&a.name, query).cmp(&name_rank(&b.name, query));
        rank.then_with(|| {
            let at = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
            let bt = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
            bt.cmp(at)
        })
    });
    nodes.truncate(30);
    Ok(nodes.into_iter().map(Into::into).collect())
}

// --- Authors: search + replace a node's members ---

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "users_bool_exp"
)]
pub struct UsersBoolExp {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub id: Option<UuidComparisonExp>,
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
    pub avatar_url: String,
}

/// Search groups and users by name for the author autocomplete (each carries the
/// node id used as the member's `node_id`).
pub async fn search_authors(access_token: Option<&str>, query: &str) -> Vec<Author> {
    if query.trim().is_empty() {
        return vec![];
    }
    let mut out: Vec<Author> = Vec::new();
    // Groups, asked for as GROUPS. This used to run the full node search and
    // filter the answer down to `wiki/group` in the client, which meant the
    // server sent every matching node of every kind — with its whole document
    // in `data`, and its parent's — so the picker could keep at most ten names
    // from it. On production one keystroke of "ann" cost 407 rows and 1.5 MB.
    // Asking for what is wanted costs 0.2 KB.
    let groups_where = NodesBoolExp {
        and: Some(vec![
            NodesBoolExp {
                or: Some(vec![
                    NodesBoolExp {
                        name: Some(StringComparisonExp {
                            ilike: Some(format!("%{query}%")),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                    NodesBoolExp {
                        content_text: Some(StringComparisonExp {
                            ilike: Some(format!("%{query}%")),
                            ..Default::default()
                        }),
                        ..Default::default()
                    },
                ]),
                ..Default::default()
            },
            NodesBoolExp {
                mime_id: Some(StringComparisonExp {
                    eq: Some("wiki/group".to_string()),
                    ..Default::default()
                }),
                ..Default::default()
            },
        ]),
        ..Default::default()
    };
    // The two searches are independent, so they go together rather than one
    // after the other. Awaited in sequence they cost the SUM of two round trips
    // on every keystroke — about a second from Denmark — for no reason beyond
    // the order they were written in.
    use cynic::QueryBuilder;
    let groups_op = NodePickerQuery::build(NodePickerVariables {
        where_clause: groups_where,
        limit: Some(10),
    });
    let users_op = UsersSearchQuery::build(UsersSearchVariables {
        where_clause: UsersBoolExp {
            display_name: Some(StringComparisonExp {
                ilike: Some(format!("%{query}%")),
                ..Default::default()
            }),
            ..Default::default()
        },
    });
    let (groups_res, users_res) = futures_util::join!(
        execute(access_token, groups_op),
        execute(access_token, users_op)
    );
    // Groups can author content.
    if let Ok(r) = groups_res {
        for n in r.nodes {
            out.push(Author {
                name: n.name,
                node_id: Some(n.id.0),
                avatar_url: String::new(),
                user_id: None,
            });
        }
    }
    // Users.
    if let Ok(r) = users_res {
        for u in r.users.into_iter().take(10) {
            out.push(Author {
                name: u.display_name,
                node_id: Some(u.id.0.clone()),
                avatar_url: u.avatar_url,
                user_id: Some(u.id.0),
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
            ..Default::default()
        },
    });
    let mut out = Vec::new();
    if let Ok(r) = execute(access_token, op).await {
        for u in r.users.into_iter().take(10) {
            out.push(Author {
                name: u.display_name,
                node_id: Some(u.id.0.clone()),
                avatar_url: u.avatar_url,
                user_id: Some(u.id.0),
            });
        }
    }
    out
}

/// Public identities for a set of user ids, in one round trip.
///
/// Used to put faces on a crash's reporters. Raw rather than cynic because the
/// generated `UuidComparisonExp` carries only `_eq` and `_is_null`, and one query
/// per reporter would be a request per person on a crash that hit fifty.
///
/// Ids the viewer may not read (the `users` select rule wants a shared context)
/// are simply absent from the result; the caller shows what it got.
pub async fn query_users_by_ids(access_token: Option<&str>, ids: &[String]) -> Vec<Author> {
    let wanted: Vec<String> = ids
        .iter()
        .filter(|id| id.len() == 36 && id.chars().all(|c| c.is_ascii_hexdigit() || c == '-'))
        .map(|id| format!("\"{}\"", gql_escape(id)))
        .collect();
    if wanted.is_empty() {
        return vec![];
    }
    let query = format!(
        "query {{ users(where: {{ id: {{ _in: [{}] }} }}) {{ id displayName avatarUrl }} }}",
        wanted.join(",")
    );
    let Ok(data) = execute_raw(access_token, &query).await else {
        return vec![];
    };
    data.get("users")
        .and_then(|u| u.as_array())
        .map(|arr| {
            arr.iter()
                .map(|u| {
                    let field = |k: &str| {
                        u.get(k)
                            .and_then(|v| v.as_str())
                            .unwrap_or_default()
                            .to_string()
                    };
                    let id = field("id");
                    Author {
                        name: field("displayName"),
                        node_id: Some(id.clone()),
                        avatar_url: field("avatarUrl"),
                        user_id: Some(id),
                    }
                })
                .collect()
        })
        .unwrap_or_default()
}

/// Fetch a single user's public identity (id + name + avatar) by id, for the
/// per-user profile page. Only readable when the viewer shares a context with
/// them (the `users` select permission), so returns None otherwise.
pub async fn query_user(access_token: Option<&str>, id: &str) -> Option<model::UserSearchFields> {
    use cynic::QueryBuilder;
    let op = UsersSearchQuery::build(UsersSearchVariables {
        where_clause: UsersBoolExp {
            id: Some(UuidComparisonExp {
                in_: None,
                eq: Some(Uuid(id.to_string())),
                is_null: None,
            }),
            ..Default::default()
        },
    });
    execute(access_token, op)
        .await
        .ok()?
        .users
        .into_iter()
        .next()
        .map(Into::into)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search bar's query must ask for the file TYPE, not the document.
    ///
    /// Thirty results carrying a whole Slate document each is how a three-letter
    /// search became 1.5 MB. Hasura selects inside a jsonb column, so the row
    /// costs one string instead — and this asserts the operation as sent, since
    /// the difference is a single argument that is easy to lose in a refactor.
    #[test]
    fn the_search_query_selects_only_the_file_type_and_caps_its_results() {
        use cynic::QueryBuilder;
        let op = NodesSearchQuery::build(NodesLimitVariables {
            where_clause: NodesBoolExp::default(),
            limit: Some(30),
        });
        assert!(
            op.query.contains(r#"data(path: "type")"#),
            "must select inside the document: {}",
            op.query
        );
        assert!(op.query.contains("limit: $limit"), "{}", op.query);
        // And the parent must stay lean: its document is what the feed needs,
        // not what a result row prints.
        assert!(
            !op.query.contains("authorAvatar"),
            "the parent is only a name here: {}",
            op.query
        );
    }
}

/// Where a query matched in a node's title, as a sort key: lower is better.
///
/// A title is what a thing is CALLED; a body mention is a thing that talks about
/// it. Searching "Uddan" should find the node named Uddannelsesordfører before a
/// candidate whose statement mentions the role, and before this the hits were
/// ordered by date alone, so the newest body match won.
///
/// Pure, and the whole of the ranking: everything the search knows about
/// relevance is here, where it can be read and tested.
pub(crate) fn name_rank(name: &str, query: &str) -> u8 {
    let name = name.trim().to_lowercase();
    let query = query.trim().to_lowercase();
    if query.is_empty() {
        return 4;
    }
    if name == query {
        return 0;
    }
    if name.starts_with(&query) {
        return 1;
    }
    // The start of any word in the title: "ordfører" should find
    // "Uddannelsesordfører" less strongly than "Uddan" does, but still ahead of
    // anything that only mentions it in passing.
    if name
        .split(|c: char| !c.is_alphanumeric())
        .any(|w| w.starts_with(&query))
    {
        return 2;
    }
    if name.contains(&query) {
        return 3;
    }
    // No title match at all: the hit came from the body.
    4
}

#[cfg(test)]
mod rank_tests {
    use super::*;

    /// The case that was reported: a title beats a body mention.
    #[test]
    fn a_title_outranks_a_body_mention() {
        // The node actually called this.
        assert_eq!(name_rank("Uddannelsesordfører", "Uddan"), 1);
        // A candidate whose statement mentions the role; its own title says
        // nothing about it, so it ranks last whatever its date.
        assert_eq!(name_rank("Asger Holm Ørskov", "Uddan"), 4);
        assert!(
            name_rank("Uddannelsesordfører", "Uddan") < name_rank("Asger Holm Ørskov", "Uddan")
        );
    }

    /// Closer matches come first among titles.
    #[test]
    fn a_closer_title_match_ranks_higher() {
        assert_eq!(name_rank("Budget", "budget"), 0, "exact");
        assert_eq!(name_rank("Budget 2026", "budget"), 1, "starts with");
        assert_eq!(name_rank("Klima og budget", "budget"), 2, "starts a word");
        assert_eq!(name_rank("Rambudgettering", "budget"), 3, "inside a word");
        assert_eq!(name_rank("Klimapolitik", "budget"), 4, "not in the title");
    }

    /// Danish letters and case are not a special case.
    #[test]
    fn matching_ignores_case_and_keeps_danish_letters() {
        assert_eq!(name_rank("Ørskov", "ørskov"), 0);
        assert_eq!(name_rank("Landsmøde 2026", "LANDSMØDE"), 1);
        assert_eq!(name_rank("Årsmøde", "års"), 1);
    }
}
