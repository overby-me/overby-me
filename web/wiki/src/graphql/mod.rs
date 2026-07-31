//! The GraphQL seam: every request this app makes to Hasura.
//!
//! Split by what is being asked about rather than by query-versus-mutation, so
//! a change to (say) the bin is one file rather than a search through four
//! thousand lines. This root keeps only what every module needs: the transport
//! (`execute` and friends, which classify a failure and decide whether it is
//! worth telling anyone about) and the schema handle.
//!
//! Every module re-exports flat, so callers still write `graphql::query_node`
//! and never name a submodule: the split is an organising principle here, not
//! a new vocabulary for the rest of the app to learn.

mod bin;
mod feed;
mod members;
mod nodes;
mod screen;
mod search;
mod social;
mod types;
mod vote;

pub use bin::*;
pub use feed::*;
pub use members::*;
pub use nodes::*;
pub use screen::*;
pub use search::*;
pub use social::*;
pub use types::*;
pub use vote::*;

use cynic::QueryBuilder;
use serde::{Deserialize, Serialize};

use crate::model;
use crate::model::{Author, BallotRules, Crumb, MemberPageFilter};
use crate::nhost::graphql_url;

mod schema {
    cynic::use_schema!("graphql/schema.graphql");
}
cynic::impl_scalar!(Uuid, schema::uuid);
cynic::impl_scalar!(Timestamptz, schema::timestamptz);
cynic::impl_scalar!(Jsonb, schema::jsonb);

/// Escape a value for embedding inside a GraphQL double-quoted string literal.
/// Used by the hand-built subscription strings in components too, so an id that
/// ever carried a `"`/`\` can't rewrite the query's `where` filter.
pub(crate) fn gql_escape(s: &str) -> String {
    s.replace('\\', "\\\\").replace('"', "\\\"")
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
    let result = match execute_once(access_token, &operation).await {
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
    };
    // Log the final failure centrally (shipped in remote-logging builds) so every
    // GraphQL error is captured with its operation, regardless of how the caller
    // surfaces it — many only show a generic toast and discard the detail.
    if let Err(e) = &result {
        // Every caller of this swallows the error into an empty list, so this is
        // the last place that knows anything went wrong.
        let failure = crate::errors::classify(e);
        // The level decides what leaves the device: logging.rs ships warn and
        // error to Better Stack. Only a genuine fault is worth paying to store.
        //
        // A refusal is normal traffic — every signed-out reader generates them by
        // existing — and a dropped connection is the venue's wifi, not this code;
        // at a congress that would be thousands of records saying the hall has bad
        // reception. Both stay on the console, where they are still there when
        // someone is debugging.
        match failure {
            crate::errors::Failure::Broken => {
                // `error`, not `warn`: this class is defined as "always a bug",
                // and it was being filed under the level people filter OUT when
                // looking for bugs.
                let summary = format!("graphql error [{}]: {e}", short_type_name::<Q>());
                log::error!("{summary}");
                // ...and into the feedback app, not only the log sink. The five
                // queries broken by one bad variable showed every reader
                // "something went wrong" and told nobody what; the detail existed
                // the whole time and only a person reading the logs could see it.
                let token = access_token.map(str::to_string);
                let path = web_sys::window()
                    .and_then(|w| w.location().pathname().ok())
                    .unwrap_or_default();
                wasm_bindgen_futures::spawn_local(async move {
                    crate::backend_api::report_error(token.as_deref(), &summary, &path).await;
                });
            }
            _ => log::info!(
                "graphql {} [{}]: {e}",
                failure.label(),
                short_type_name::<Q>()
            ),
        }
        // The user hears about it only if it is theirs to care about, once,
        // throttled.
        crate::errors::report(failure);
    }
    result
}

/// The bare query-struct name (last `::` segment) for a GraphQL log line, e.g.
/// `NodeInsertsQuery` rather than the full `wiki_dioxus::graphql::…` path.
fn short_type_name<T>() -> &'static str {
    std::any::type_name::<T>()
        .rsplit("::")
        .next()
        .unwrap_or("query")
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
    let result = match execute_raw_once(access_token, query).await {
        Err(msg) if is_jwt_error(&msg) => match crate::session::ensure_fresh_token().await {
            Some(fresh) if Some(fresh.as_str()) != access_token => {
                execute_raw_once(Some(&fresh), query).await
            }
            _ => Err(msg),
        },
        other => other,
    };
    if let Err(e) = &result {
        log::warn!("graphql error (raw): {e}");
    }
    result
}

async fn execute_raw_vars_once(
    access_token: Option<&str>,
    query: &str,
    variables: &serde_json::Value,
) -> Result<serde_json::Value, String> {
    let client = reqwest::Client::new();
    let mut req = client.post(graphql_url());
    if let Some(token) = access_token {
        req = req.bearer_auth(token);
    }
    let body = serde_json::json!({ "query": query, "variables": variables });
    let resp = req.json(&body).send().await.map_err(|e| e.to_string())?;
    let result: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    if let Some(errors) = result.get("errors") {
        return Err(errors.to_string());
    }
    Ok(result.get("data").cloned().unwrap_or_default())
}

/// Like [`execute_raw`] but with GraphQL `variables` (for mutations that pass
/// structured input, e.g. seeding a new context's permission template), with the
/// same JWT refresh-and-retry.
pub async fn execute_raw_vars(
    access_token: Option<&str>,
    query: &str,
    variables: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let result = match execute_raw_vars_once(access_token, query, &variables).await {
        Err(msg) if is_jwt_error(&msg) => match crate::session::ensure_fresh_token().await {
            Some(fresh) if Some(fresh.as_str()) != access_token => {
                execute_raw_vars_once(Some(&fresh), query, &variables).await
            }
            _ => Err(msg),
        },
        other => other,
    };
    if let Err(e) = &result {
        log::warn!("graphql error (raw vars): {e}");
    }
    result
}

/// The remembered answer to a read, if the failure was the kind a copy answers.
///
/// A refusal must not fall back: serving what someone could read yesterday would
/// be the app overriding a permission change made since.
fn offline_copy<T: serde::de::DeserializeOwned>(key: &str, error: &str) -> Option<T> {
    if crate::errors::classify(error) != crate::errors::Failure::Offline {
        return None;
    }
    let copy = crate::offline::get::<T>(key)?;
    crate::errors::report_offline_copy();
    Some(copy)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The bin's list query names types Hasura actually defines.
    ///
    /// A tracked view gets its GraphQL types from its custom name, so the view
    /// `deleted_nodes` produced `deletedNodes_bool_exp`, not the
    /// `deleted_nodes_bool_exp` the hand-written schema entry claimed. Nothing
    /// caught it: the local schema is the only thing cynic checks against, so
    /// the query compiled and would have been rejected by the server, on a
    /// screen the tests never open. This asserts the operation as sent.
    /// The feed scoped to a context asks for its whole subtree, not just the
    /// rows that name it. A group holds events and an event's content belongs to
    /// the event, so without the ancestor test a group's feed shows almost none
    /// of what happened in it.
    #[test]
    fn a_scoped_feed_rolls_up_the_subtree() {
        let clause = recent_where_clause("user-1", Some("ctx-1"));
        let json = serde_json::to_string(&clause).expect("serialize");
        assert!(
            json.contains(r#""ancestors":{"_contains":["ctx-1"]}"#),
            "scoped feed must include everything under the context: {json}"
        );
        assert!(
            json.contains(r#""contextId":{"_eq":"ctx-1"}"#),
            "and anything filed with it directly: {json}"
        );
        // Unset comparison expressions must stay off the wire (Hasura rejects a
        // null where a comparison object is expected).
        assert!(!json.contains("null"), "no null comparisons: {json}");
    }

    /// Unscoped, the feed is still "contexts you belong to" — the ancestor test
    /// belongs to the scoped branch only, or the home feed would widen to every
    /// context that happens to sit under one you are in.
    #[test]
    fn an_unscoped_feed_stays_on_membership() {
        let clause = recent_where_clause("user-1", None);
        let json = serde_json::to_string(&clause).expect("serialize");
        assert!(!json.contains("ancestors"), "{json}");
        assert!(json.contains("members"), "{json}");
    }

    #[test]
    fn bin_query_declares_the_types_hasura_defines() {
        use cynic::QueryBuilder;
        let op = DeletedNodesQuery::build(DeletedNodesVariables {
            where_clause: DeletedNodesBoolExp::default(),
            order_by: Some(vec![DeletedNodesOrderBy {
                deleted_at: Some(OrderBy::Desc),
            }]),
        });
        assert!(
            op.query.contains("deletedNodes_bool_exp")
                && op.query.contains("deletedNodes_order_by"),
            "the view's types are camelCase after its custom name: {}",
            op.query
        );
        assert!(
            !op.query.contains("deleted_nodes_"),
            "no snake_case type survives: {}",
            op.query
        );
    }

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

    /// `gql_escape` guards the hand-built subscription/where strings: a value
    /// carrying `"` or `\` must be neutralised so it can't break out of the
    /// string literal and rewrite the query filter (a GraphQL injection).
    #[test]
    fn gql_escape_neutralises_quotes_and_backslashes() {
        assert_eq!(gql_escape("plain-id"), "plain-id");
        assert_eq!(gql_escape(r#"a"b"#), r#"a\"b"#);
        assert_eq!(gql_escape(r"a\b"), r"a\\b");
        // A backslash must be doubled BEFORE quotes are escaped, so a crafted
        // `\"` can't survive as an unescaped quote.
        assert_eq!(gql_escape(r#"\""#), r#"\\\""#);
        // A classic injection attempt stays inside the literal.
        assert_eq!(
            gql_escape(r#"" }, name: { _eq: "x"#),
            r#"\" }, name: { _eq: \"x"#
        );
    }

    /// The member-page `where` builder must escape the parent id + search term
    /// and omit unset bool filters.
    #[test]
    fn members_where_escapes_and_omits_unset() {
        let base = MemberPageFilter::default();
        let clause = members_where("ctx-1", &base);
        assert!(clause.contains(r#"parentId: { _eq: "ctx-1" }"#), "{clause}");
        // No bool filters and empty search -> only the parentId clause.
        assert!(!clause.contains("owner:"), "{clause}");
        assert!(!clause.contains("_ilike"), "{clause}");

        let filtered = MemberPageFilter {
            owner: Some(true),
            active: Some(false),
            search: "  a\"b  ".to_string(),
            ..Default::default()
        };
        let clause = members_where("ctx-1", &filtered);
        assert!(clause.contains("owner: { _eq: true }"), "{clause}");
        assert!(clause.contains("active: { _eq: false }"), "{clause}");
        // Search is trimmed, wrapped in %..%, and the embedded quote is escaped.
        assert!(clause.contains(r#"_ilike: "%a\"b%""#), "{clause}");
        assert!(
            !clause.contains("accepted:"),
            "unset filter omitted: {clause}"
        );
    }

    /// The invitations filter must omit null fields and carry the pending +
    /// group/event + user/email conditions the home list depends on.
    #[test]
    fn a_taken_key_is_told_apart_from_other_failures() {
        // Hasura's wording for the (parent_id, key) index. Matching too broadly
        // would retry a key that was never the problem; too narrowly would give
        // up on a clean key at the first collision.
        let taken = "hasura error: [{\"message\":\"Uniqueness violation. duplicate key \
                     value violates unique constraint \\\"nodes_parent_id_namespace_key\\\"\"}]";
        assert!(super::is_key_taken(taken));

        for other in [
            "hasura error: [{\"message\":\"permission denied\"}]",
            "network error",
            "hasura error: [{\"message\":\"not-null violation\"}]",
        ] {
            assert!(!super::is_key_taken(other), "should not retry on: {other}");
        }
    }

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

#[cfg(test)]
mod variable_tests {
    use super::*;
    use cynic::QueryBuilder;

    /// Names every variable the operation SENDS but does not DECLARE.
    ///
    /// cynic declares only the variables an operation actually uses, while
    /// serialising every field of the struct it was handed. Hasura rejects an
    /// undeclared variable outright — the whole query fails — so the two must
    /// agree, and nothing in the type system makes them.
    fn undeclared<Q, V: serde::Serialize>(op: &cynic::Operation<Q, V>) -> Vec<String> {
        let json = serde_json::to_value(&op.variables).unwrap_or(serde_json::Value::Null);
        json.as_object()
            .map(|o| {
                o.keys()
                    .filter(|k| !op.query.contains(&format!("${k}")))
                    .cloned()
                    .collect()
            })
            .unwrap_or_default()
    }

    macro_rules! assert_declared {
        ($($op:expr),+ $(,)?) => {
            $({
                let op = $op;
                let extra = undeclared(&op);
                assert!(
                    extra.is_empty(),
                    "sends undeclared variable(s) {:?}:\n{}",
                    extra, op.query
                );
            })+
        };
    }

    /// Every operation must declare every variable it sends.
    ///
    /// This is the test that was missing. A shared `NodesWhereVariables` gained
    /// an optional `limit` for the search box; the five other queries built from
    /// it — votes, polls, the home context list, the subtree walk, the feed
    /// count — kept sending it without declaring it, and Hasura failed all five
    /// in production ("unexpected variables in variableValues: limit"). The
    /// existing tests asserted the query TEXT and never the variables beside it,
    /// so nothing noticed. Assert the pair, for every shape the app sends.
    #[test]
    fn no_operation_sends_a_variable_it_does_not_declare() {
        let node_where = || NodesBoolExp::default();
        assert_declared!(
            // The two that legitimately carry a cap...
            NodesWhereQuery::build(NodesLimitVariables {
                where_clause: node_where(),
                limit: Some(30),
            }),
            NodesSearchQuery::build(NodesLimitVariables {
                where_clause: node_where(),
                limit: Some(30),
            }),
            NodePickerQuery::build(NodePickerVariables {
                where_clause: node_where(),
                limit: Some(10),
            }),
            // ...and the five that must not.
            ContextsWhereQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            ChildIdsQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            NodesCountQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            VotesWhereQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            PollsWhereQuery::build(NodesWhereVariables {
                where_clause: node_where(),
            }),
            // The rest of the read path, so the next shared struct cannot repeat it.
            ChildrenQuery::build(ChildrenVariables {
                where_clause: node_where(),
                order_by: None,
            }),
            DrawerChildrenQuery::build(DrawerChildrenVariables {
                where_clause: node_where(),
                order_by: None,
                child_visible: node_where(),
            }),
            RecentNodesQuery::build(RecentNodesVariables {
                where_clause: node_where(),
                order_by: None,
                limit: Some(20),
                offset: Some(0),
            }),
            RelationsQuery::build(RelationsWhereVariables {
                where_clause: RelationsBoolExp::default(),
            }),
            MembersCountQuery::build(MembersCountVariables {
                where_clause: MembersBoolExp::default(),
            }),
            MembersExistQuery::build(MembersExistVariables {
                where_clause: MembersBoolExp::default(),
            }),
            InvitationsQuery::build(MembersWhereVariables {
                where_clause: MembersBoolExp::default(),
            }),
            UsersSearchQuery::build(UsersSearchVariables {
                where_clause: UsersBoolExp::default(),
            }),
            DeletedNodesQuery::build(DeletedNodesVariables {
                where_clause: DeletedNodesBoolExp::default(),
                order_by: None,
            }),
        );
    }
}
