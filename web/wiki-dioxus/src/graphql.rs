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
    pub mutable: bool,
    pub index: i32,
    pub data: Option<Jsonb>,
    pub mime: Option<MimeFields>,
    pub parent: Option<Box<ParentNodeFields>>,
    pub children: Vec<ChildNodeFields>,
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
    pub mime_id: Option<StringComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub owner_id: Option<UuidComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub members: Option<MembersBoolExp>,
}

#[derive(cynic::InputObject, Debug, Default)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "members_bool_exp"
)]
pub struct MembersBoolExp {
    #[cynic(rename = "_and", skip_serializing_if = "Option::is_none")]
    pub and: Option<Vec<MembersBoolExp>>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub accepted: Option<BooleanComparisonExp>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<UuidComparisonExp>,
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
    pub name: Option<String>,
    pub key: Option<String>,
    pub mime_id: Option<String>,
    pub parent_id: Option<Uuid>,
    pub context_id: Option<Uuid>,
    pub data: Option<Jsonb>,
    pub mutable: Option<bool>,
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

/// Delete a node by ID
pub async fn delete_node(access_token: Option<&str>, id: &str) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let operation = DeleteNodeMutation::build(DeleteNodeVariables {
        id: Uuid(id.to_string()),
    });
    let result = execute(access_token, operation).await?;
    Ok(result.delete_node.is_some())
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
}
