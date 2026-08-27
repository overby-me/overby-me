//! The bin: what a delete stamped, how it is listed, and the two ways out of
//! it — back into the tree, or gone for good.

use super::*;

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct DeletedNode {
    pub id: Uuid,
}

// --- The bin: soft delete, list, restore ---

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "deletedNodes")]
pub struct DeletedNodeFields {
    pub id: Option<Uuid>,
    pub name: Option<String>,
    pub key: Option<String>,
    pub path: Option<String>,
    pub mime_id: Option<String>,
    pub deleted_at: Option<Timestamptz>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "deletedNodes_bool_exp"
)]
pub struct DeletedNodesBoolExp {
    #[cynic(rename = "_or", skip_serializing_if = "Option::is_none")]
    pub or: Option<Vec<DeletedNodesBoolExp>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub context_id: Option<UuidComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<UuidComparisonExp>,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "deletedNodes_order_by"
)]
pub struct DeletedNodesOrderBy {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub deleted_at: Option<OrderBy>,
}

#[derive(cynic::QueryVariables)]
pub struct DeletedNodesVariables {
    pub where_clause: DeletedNodesBoolExp,
    pub order_by: Option<Vec<DeletedNodesOrderBy>>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "DeletedNodesVariables"
)]
pub struct DeletedNodesQuery {
    #[arguments(where: $where_clause, order_by: $order_by)]
    pub deleted_nodes: Vec<DeletedNodeFields>,
}

/// What is in a context's bin: one row per delete someone asked for, newest
/// first. Who sees which rows is the view's own permission, not this clause.
///
/// Two ways in, because a context is its own context: everything binned INSIDE
/// this context, and any context binned directly UNDER this node. Without the
/// second, deleting a group or an event put it in a bin reachable only through
/// itself — the one delete in the app that could not be undone.
pub async fn query_deleted(
    access_token: Option<&str>,
    context_id: &str,
    node_id: &str,
) -> Result<Vec<DeletedNodeFields>, String> {
    use cynic::QueryBuilder;
    let op = DeletedNodesQuery::build(DeletedNodesVariables {
        where_clause: DeletedNodesBoolExp {
            or: Some(vec![
                DeletedNodesBoolExp {
                    context_id: Some(UuidComparisonExp {
                        neq: None,
                        in_: None,
                        eq: Some(Uuid(context_id.to_string())),
                        is_null: None,
                    }),
                    ..Default::default()
                },
                DeletedNodesBoolExp {
                    parent_id: Some(UuidComparisonExp {
                        neq: None,
                        in_: None,
                        eq: Some(Uuid(node_id.to_string())),
                        is_null: None,
                    }),
                    ..Default::default()
                },
            ]),
            ..Default::default()
        },
        order_by: Some(vec![DeletedNodesOrderBy {
            deleted_at: Some(OrderBy::Desc),
        }]),
    });
    Ok(execute(access_token, op).await?.deleted_nodes)
}

/// Bin a node and everything under it, in one statement.
///
/// The subtree is found by path prefix, which is what the `path` column is for:
/// the old deep delete walked it a request per node. Every stamped row carries
/// `deleted_root`, so restore can undo exactly this action rather than guessing
/// at a tree that may have changed since.
///
/// Rows the caller may not update are simply not updated — the same authority
/// the old delete had, since it deleted node by node under the same rules.
///
/// `path` is what the caller already has on screen. Callers that do not carry it
/// (a comment, a poll: shapes the lists fetch without it) pass `None` and the
/// path is looked up, one query, rather than binning the node alone and leaving
/// its replies or its ballots behind.
pub async fn bin_node(
    access_token: Option<&str>,
    node_id: &str,
    path: Option<&str>,
    actor: Option<&str>,
) -> Result<u32, String> {
    let looked_up = match path.filter(|p| !p.is_empty()) {
        Some(_) => None,
        None => {
            let segments = path_from_id(access_token, node_id)
                .await
                .unwrap_or_default();
            (!segments.is_empty()).then(|| segments.join("/"))
        }
    };
    let path = path.filter(|p| !p.is_empty()).or(looked_up.as_deref());
    let subtree = match path {
        Some(p) => format!(r#"{{path: {{_like: "{}/%"}}}}"#, gql_escape(p)),
        // No path (an orphan, or a node the trigger could not place) means the
        // node alone: a prefix of nothing would match everything.
        None => r#"{id: {_is_null: true}}"#.to_string(),
    };
    let query = format!(
        r#"mutation($id: uuid!, $set: nodes_set_input!) {{
             updateNodes(where: {{_and: [
                 {{deleted_at: {{_is_null: true}}}},
                 {{_or: [{{id: {{_eq: $id}}}}, {subtree}]}}
             ]}}, _set: $set) {{ affected_rows }}
           }}"#
    );
    let set = serde_json::json!({
        "deleted_at": "now()",
        "deleted_by": actor,
        "deleted_root": node_id,
    });
    let data = execute_raw_vars(
        access_token,
        &query,
        serde_json::json!({ "id": node_id, "set": set }),
    )
    .await?;
    Ok(data
        .get("updateNodes")
        .and_then(|u| u.get("affected_rows"))
        .and_then(|a| a.as_u64())
        .unwrap_or(0) as u32)
}

/// Delete for good everything one bin action took, by the `deleted_root` it
/// stamped: the way out of the bin that restore is not.
///
/// Members, fields, relations and permissions hanging off these nodes go with
/// them — every foreign key pointing at `nodes` cascades — so this leaves
/// nothing behind, which is the whole point of it.
///
/// The row rules decide who may: the database applies the same delete
/// permission it always did, so a caller who could not have deleted the node in
/// the first place removes nothing here either. The app offers this to context
/// owners only.
pub async fn purge_node(access_token: Option<&str>, root_id: &str) -> Result<u32, String> {
    let query = r#"mutation($id: uuid!) {
        deleteNodes(where: {deleted_root: {_eq: $id}}) { affected_rows }
    }"#;
    let data = execute_raw_vars(access_token, query, serde_json::json!({ "id": root_id })).await?;
    Ok(data
        .get("deleteNodes")
        .and_then(|u| u.get("affected_rows"))
        .and_then(|a| a.as_u64())
        .unwrap_or(0) as u32)
}

/// Put back everything one bin action took, by the `deleted_root` it stamped.
pub async fn restore_node(access_token: Option<&str>, root_id: &str) -> Result<u32, String> {
    let query = r#"mutation($id: uuid!) {
        updateNodes(where: {deleted_root: {_eq: $id}},
                    _set: {deleted_at: null, deleted_by: null, deleted_root: null}) {
            affected_rows
        }
    }"#;
    let data = execute_raw_vars(access_token, query, serde_json::json!({ "id": root_id })).await?;
    Ok(data
        .get("updateNodes")
        .and_then(|u| u.get("affected_rows"))
        .and_then(|a| a.as_u64())
        .unwrap_or(0) as u32)
}
