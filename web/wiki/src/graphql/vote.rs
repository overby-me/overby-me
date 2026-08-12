//! The ballot: polls, the votes cast on them, and the permission rows that say
//! who may do either.

use super::*;

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
    rules: BallotRules,
) -> Result<model::InsertedNode, String> {
    // Close the context's current active poll, if any (only one is open at once).
    if let Ok(Some(prior)) = active_node_id(access_token, context_id).await {
        let _ = update_node(
            access_token,
            &prior,
            model::NodesSetInput {
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
        "hidden": rules.hide_tally,
        "secret": rules.secret,
        "nodeId": parent_id,
    });
    let inserted = insert_node(
        access_token,
        model::NodesInsertInput {
            name: Some(name.to_string()),
            key: Some(key.to_string()),
            mime_id: Some("vote/poll".to_string()),
            parent_id: Some(model::Uuid(parent_id.to_string())),
            context_id: Some(model::Uuid(context_id.to_string())),
            data: Some(model::Jsonb(data)),
            mutable: Some(true),
            index: None,
            created_at: None,
        },
    )
    .await?
    .ok_or_else(|| "poll insert returned no node".to_string())?;
    set_active_relation(access_token, context_id, Some(&inserted.id.0)).await?;
    Ok(inserted)
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
    /// `public`, `member` or `owner`. Used by the signed-out place list, which
    /// asks the permission rows themselves which contexts are open.
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub role: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub select: Option<BooleanComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub active: Option<BooleanComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context: Option<NodesBoolExp>,
}

/// The permission rows configured on a context, for the perm overview.
pub async fn query_permissions(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Vec<model::PermissionFields>, String> {
    let where_clause = PermissionsBoolExp {
        context_id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
        ..Default::default()
    };
    let operation = PermissionsQuery::build(PermissionsWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.permissions.into_iter().map(Into::into).collect())
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
    /// Whether the poll is still open (mutable) — drives the admin open/closed badge.
    pub mutable: bool,
}

/// The number of votes cast on a poll (its visible `vote/vote` children).
pub async fn poll_vote_count(access_token: Option<&str>, poll_id: &str) -> Result<usize, String> {
    let where_clause = NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            in_: None,
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
    let input = model::NodesInsertInput {
        name: Some(format!("vote-{key_suffix}")),
        key: Some(format!("vote-{key_suffix}")),
        mime_id: Some("vote/vote".to_string()),
        parent_id: Some(model::Uuid(poll_id.to_string())),
        context_id: context_id.map(|c| model::Uuid(c.to_string())),
        data: Some(model::Jsonb(data)),
        mutable: None,
        index: None,
        created_at: None,
    };
    Ok(insert_node(access_token, input).await?.is_some())
}

/// Count a poll's votes per option WITHOUT fetching the ballots.
///
/// The tally used to be computed in the browser from every vote row: at 500
/// ballots that is 28 KB per device, re-fetched on every vote cast by anyone,
/// and 500 devices doing that during one vote is the load a general assembly
/// actually generates. Counting is the server's job — measured against a
/// 500-vote poll in production, the same answer is 0.15 KB and no slower
/// (205 ms against 248 ms), and no delegate's ballot leaves the database.
///
/// `options` is the number of choices; pass 0 for the turnout total alone, which
/// is what a poll whose results are hidden still needs. Multi-choice ballots
/// count in every option they contain, matching the tally this replaces.
pub async fn poll_tally(
    access_token: Option<&str>,
    poll_id: &str,
    options: usize,
    own_of: Option<&str>,
) -> Result<(Vec<usize>, usize, usize), String> {
    let query = poll_tally_query(poll_id, options, own_of);
    let data = execute_raw(access_token, &query).await?;
    Ok(parse_tally(&data, options))
}

/// The aliased aggregates back into `(per option, total, own)`.
///
/// Separate and pure because the shape is easy to get wrong and impossible to
/// notice: `execute_raw` returns the `data` OBJECT, not the whole response, and
/// reaching for `data.data.o0` silently yielded zero for every option. The query
/// was verified against production and the parsing was not, so a poll would have
/// shown an empty result with no error anywhere.
///
/// `own` is 0 when it was not asked for, which is the same answer a voter who has
/// not voted gets. Both mean "do not treat this as having voted", so the caller
/// that cannot use it (a secret poll) does not have to tell them apart.
fn parse_tally(data: &serde_json::Value, options: usize) -> (Vec<usize>, usize, usize) {
    let count_at = |key: &str| -> usize {
        data.get(key)
            .and_then(|a| a.get("aggregate"))
            .and_then(|a| a.get("count"))
            .and_then(|c| c.as_u64())
            .unwrap_or(0) as usize
    };
    let counts = (0..options).map(|i| count_at(&format!("o{i}"))).collect();
    (counts, count_at("total"), count_at("own"))
}

/// The tally query: one aliased aggregate per option, the total, and — when
/// `own_of` is given — how many of them are this voter's.
///
/// A vote's `data` is the array of chosen indices, so option `i` is counted with
/// jsonb containment — `data @> [i]` — which is what `_contains` compiles to.
///
/// `own` rides along rather than being asked for separately. Casting a ballot
/// refreshed two requests: this one, and a whole-node query whose rows were
/// counted with `.len()`. Two requests on the same trigger resolve at different
/// times, which is what made the result bars move twice; and the second was the
/// expensive kind, selecting path, data, mime and parent for every ballot to
/// learn one integer. One aggregate answers it. A secret poll cannot use this at
/// all, since its ballots carry no `ownerId` (that is the point of them), and
/// asks the backend's marker instead.
fn poll_tally_query(poll_id: &str, options: usize, own_of: Option<&str>) -> String {
    let id = gql_escape(poll_id);
    let base = format!("parentId: {{_eq: \"{id}\"}}, mimeId: {{_eq: \"vote/vote\"}}");
    let mut q = String::from("query PollTally {\n");
    for i in 0..options {
        q.push_str(&format!(
            "  o{i}: nodesAggregate(where: {{{base}, data: {{_contains: [{i}]}}}}) \
             {{ aggregate {{ count }} }}\n"
        ));
    }
    q.push_str(&format!(
        "  total: nodesAggregate(where: {{{base}}}) {{ aggregate {{ count }} }}\n"
    ));
    if let Some(uid) = own_of {
        let uid = gql_escape(uid);
        q.push_str(&format!(
            "  own: nodesAggregate(where: {{{base}, ownerId: {{_eq: \"{uid}\"}}}}) \
             {{ aggregate {{ count }} }}\n"
        ));
    }
    q.push('}');
    q
}

#[cfg(test)]
mod tally_tests {
    use super::*;

    /// One request per refresh, not two.
    ///
    /// The voter's own count rides in the tally. Asked separately it was a second
    /// request on the same trigger, landing at a different moment and moving the
    /// result bars again; and it was a whole-node query counted with `.len()`.
    #[test]
    fn the_tally_counts_the_voters_own_ballots_in_the_same_request() {
        let q = poll_tally_query("poll-1", 3, Some("user-1"));
        assert!(q.contains("own: nodesAggregate"), "{q}");
        assert!(q.contains(r#"ownerId: {_eq: "user-1"}"#), "{q}");
        // Still one query, and still no rows.
        assert_eq!(q.matches("query ").count(), 1, "{q}");
        assert!(!q.contains("nodes("), "{q}");

        // A secret poll has no ownerId to count by, so it does not ask.
        let secret = poll_tally_query("poll-1", 3, None);
        assert!(!secret.contains("own:"), "{secret}");
        assert!(!secret.contains("ownerId"), "{secret}");
    }

    #[test]
    fn the_tally_asks_the_server_to_count_each_option() {
        let q = poll_tally_query("poll-1", 3, None);
        // One aggregate per option, each matching ballots that contain it...
        for i in 0..3 {
            assert!(q.contains(&format!("o{i}: nodesAggregate")), "{q}");
            assert!(q.contains(&format!("data: {{_contains: [{i}]}}")), "{q}");
        }
        // ...plus the turnout total, and NO row selection anywhere.
        assert!(q.contains("total: nodesAggregate"), "{q}");
        assert!(
            !q.contains("nodes("),
            "the tally must not fetch ballots: {q}"
        );
    }

    /// A hidden poll still shows turnout, and must not count what it will not show.
    #[test]
    fn a_hidden_poll_asks_only_for_the_total() {
        let q = poll_tally_query("poll-1", 0, None);
        assert!(q.contains("total: nodesAggregate"), "{q}");
        assert!(!q.contains("o0:"), "{q}");
        // Still true when the voter's own count rides along: hiding the tally
        // hides the per-option counts, not whether this reader has voted.
        let with_own = poll_tally_query("poll-1", 0, Some("user-1"));
        assert!(with_own.contains("own: nodesAggregate"), "{with_own}");
        assert!(!with_own.contains("o0:"), "{with_own}");
    }

    /// The shape `execute_raw` actually returns, captured from production.
    ///
    /// This is the test that was missing when the tally shipped counting zero.
    #[test]
    fn a_real_tally_response_is_parsed() {
        let data = serde_json::json!({
            "o0": {"aggregate": {"count": 166}},
            "o1": {"aggregate": {"count": 167}},
            "o2": {"aggregate": {"count": 167}},
            "total": {"aggregate": {"count": 500}},
            "own": {"aggregate": {"count": 1}}
        });
        assert_eq!(parse_tally(&data, 3), (vec![166, 167, 167], 500, 1));
        // A hidden poll asks for the total alone.
        assert_eq!(parse_tally(&data, 0), (vec![], 500, 1));
        // An absent `own` (a secret poll never asks) reads as not having voted,
        // which is the same answer and needs no separate case.
        let no_own = serde_json::json!({"total": {"aggregate": {"count": 7}}});
        assert_eq!(parse_tally(&no_own, 0), (vec![], 7, 0));
    }

    /// A response wrapped one level too deep must not read as "no votes".
    #[test]
    fn a_wrapped_response_is_not_silently_zero() {
        let wrapped = serde_json::json!({"data": {"total": {"aggregate": {"count": 500}}}});
        assert_eq!(
            parse_tally(&wrapped, 0).1,
            0,
            "the old shape yields zero, which is why it was invisible"
        );
    }

    /// A poll id is escaped like any other interpolated value.
    #[test]
    fn the_poll_id_is_escaped() {
        assert!(poll_tally_query("a\"b", 1, None).contains("a\\\"b"));
        // And so is the user id, which is interpolated the same way.
        assert!(poll_tally_query("p", 1, Some("c\"d")).contains("c\\\"d"));
    }
}
