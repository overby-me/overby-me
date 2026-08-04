//! What a visitor with no account can reach.
//!
//! A place is open to everyone when its context carries an active `public`
//! permission row granting `select`. That row IS the setting, so this asks the
//! permission table rather than inferring openness from the nodes: a node being
//! published says it is finished, not that a stranger may read it, and the two
//! came apart the moment one old event was left public and nothing in the app
//! said so.
//!
//! Deliberately blind to what kind of place it is. Groups, events and any
//! context type added later all appear here on the same rule, so the list needs
//! no edit when the model grows.

use super::*;

#[derive(cynic::QueryVariables, Debug)]
pub struct PublicPlacesVariables {
    pub where_clause: PermissionsBoolExp,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "query_root",
    variables = "PublicPlacesVariables"
)]
pub struct PublicPlacesQuery {
    #[arguments(where: $where_clause)]
    pub permissions: Vec<PublicPlaceRow>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "permissions")]
pub struct PublicPlaceRow {
    pub context: Option<PublicPlaceNode>,
}

#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct PublicPlaceNode {
    pub id: Uuid,
    pub name: String,
    pub path: Option<String>,
    pub mime_id: Option<String>,
}

/// The places a signed-out visitor may read, newest rule first, each listed once.
///
/// A context has a row per mime type it opens up, so the same place comes back
/// several times; they are folded here. The root is excluded: it is the front
/// page, it is already the drawer's top entry, and listing "Home" among the
/// places to go is furniture.
pub async fn query_public_places(
    access_token: Option<&str>,
) -> Result<Vec<model::PublicPlace>, String> {
    let where_clause = PermissionsBoolExp {
        role: Some(StringComparisonExp {
            eq: Some("public".to_string()),
            ..Default::default()
        }),
        select: Some(BooleanComparisonExp { eq: Some(true) }),
        active: Some(BooleanComparisonExp { eq: Some(true) }),
        // Only the root has no parent, so this drops it without naming its mime.
        context: Some(NodesBoolExp {
            parent_id: Some(UuidComparisonExp {
                in_: None,
                eq: None,
                is_null: Some(false),
            }),
            ..Default::default()
        }),
        context_id: None,
    };
    let operation = PublicPlacesQuery::build(PublicPlacesVariables { where_clause });
    let result = execute(access_token, operation).await?;

    let mut seen = std::collections::HashSet::new();
    let mut places = Vec::new();
    for row in result.permissions {
        let Some(node) = row.context else { continue };
        // A place with no path cannot be linked to, which is the only thing this
        // list does with it.
        let Some(path) = node.path.filter(|p| !p.is_empty()) else {
            continue;
        };
        if !seen.insert(node.id.0.clone()) {
            continue;
        }
        places.push(model::PublicPlace {
            id: node.id.0,
            name: node.name,
            path,
            mime_id: node.mime_id.unwrap_or_default(),
        });
    }
    places.sort_by_key(|p| p.name.to_lowercase());
    Ok(places)
}
