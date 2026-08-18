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

/// The rows that make a context readable by anyone: the place itself, so it can
/// be opened at all, and the folders, documents and files inside it.
///
/// Deliberately NOT the ballot (`vote/*`). A motion under discussion and a
/// published page are different decisions, and someone opening a meeting's
/// agenda to the world should not thereby publish how the room voted. If that is
/// wanted it should be asked for.
///
/// Read only. Every write flag is false, so being public can never be a way in.
fn public_permission_rows(ctx_id: &str, context_mime: &str) -> Vec<serde_json::Value> {
    // A context sits under the root or inside another place; its contents sit
    // inside a container.
    let place_parents = [
        "wiki/home",
        "wiki/event",
        "wiki/folder",
        "wiki/group",
        "wiki/site",
    ];
    let mut mimes: Vec<(&str, &[&str])> = vec![(context_mime, &place_parents)];
    for content in ["wiki/folder", "wiki/document", "wiki/file"] {
        mimes.push((content, super::CONTAINER_PARENTS));
    }
    mimes
        .into_iter()
        .map(|(mime, parents)| {
            serde_json::json!({
                "contextId": ctx_id,
                "nodeId": ctx_id,
                "mimeId": mime,
                "role": "public",
                "parents": parents,
                "active": true,
                "insert": false,
                "select": true,
                "update": false,
                "delete": false,
            })
        })
        .collect()
}

/// Open this context to visitors, or close it again.
///
/// Closing DEACTIVATES rather than deletes, so reopening restores exactly what
/// was configured, including any rows added by hand that this template does not
/// know about. Opening reactivates what is there before adding what is missing,
/// for the same reason: a context closed and reopened should come back as it
/// was, not as the template thinks it should be.
pub async fn set_context_public(
    access_token: Option<&str>,
    context_id: &str,
    context_mime: &str,
    on: bool,
) -> Result<(), String> {
    const SET_ACTIVE: &str = "mutation($ctx: uuid!, $on: Boolean!) { \
         updatePermissions(where: {contextId: {_eq: $ctx}, role: {_eq: \"public\"}}, \
         _set: {active: $on}) { affected_rows } }";

    super::execute_raw_vars(
        access_token,
        SET_ACTIVE,
        serde_json::json!({ "ctx": context_id, "on": on }),
    )
    .await?;
    if !on {
        return Ok(());
    }

    // What the reactivation above could not cover: a context that has never been
    // public has no rows to turn back on.
    let existing = super::query_permissions(access_token, context_id).await?;
    let have: std::collections::HashSet<&str> = existing
        .iter()
        .filter(|p| p.role == "public")
        .filter_map(|p| p.mime_id.as_deref())
        .collect();
    let missing: Vec<serde_json::Value> = public_permission_rows(context_id, context_mime)
        .into_iter()
        .filter(|row| {
            row.get("mimeId")
                .and_then(|m| m.as_str())
                .is_some_and(|m| !have.contains(m))
        })
        .collect();
    if missing.is_empty() {
        return Ok(());
    }
    super::execute_raw_vars(
        access_token,
        "mutation($objs: [permissions_insert_input!]!) { \
         insertPermissions(objects: $objs) { affected_rows } }",
        serde_json::json!({ "objs": missing }),
    )
    .await?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::public_permission_rows;

    /// Opening a place grants READING and nothing else. A public row that could
    /// insert or delete would make "anyone may read this" a way in.
    #[test]
    fn opening_a_place_grants_reading_only() {
        for row in public_permission_rows("ctx-1", "wiki/site") {
            let name = row["mimeId"].as_str().unwrap().to_string();
            assert_eq!(row["role"], "public", "{name}");
            assert_eq!(row["select"], true, "{name} should be readable");
            for write in ["insert", "update", "delete"] {
                assert_eq!(row[write], false, "{name} must not grant {write}");
            }
            assert_eq!(row["contextId"], "ctx-1");
        }
    }

    /// The place's own mime is covered, or the container itself stays invisible
    /// and everything inside it is unreachable by descent.
    #[test]
    fn the_place_itself_is_covered_whatever_kind_it_is() {
        for mime in crate::model::CONTEXT_MIMES {
            let rows = public_permission_rows("ctx-1", mime);
            let mimes: Vec<&str> = rows.iter().map(|r| r["mimeId"].as_str().unwrap()).collect();
            assert!(
                mimes.contains(mime),
                "{mime} is not covered by its own rows"
            );
            for content in ["wiki/folder", "wiki/document", "wiki/file"] {
                assert!(mimes.contains(&content), "{content} missing for {mime}");
            }
            // The ballot is a separate decision (see the doc comment).
            assert!(!mimes.iter().any(|m| m.starts_with("vote/")));
        }
    }
}
