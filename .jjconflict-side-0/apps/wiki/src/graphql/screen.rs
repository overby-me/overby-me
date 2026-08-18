//! The room's screen: what is projected, where the projector is focused, and
//! the toggles a chair drives it with.

use super::*;

#[derive(cynic::QueryFragment, Debug, Clone, PartialEq)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "relations")]
pub struct RelationFields {
    pub name: String,
    pub node_id: Option<Uuid>,
}

/// The id of a context's "active" node — during a vote this is the open poll,
/// mirroring the React VoteApp's `get("active")`.
pub async fn active_node_id(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Option<String>, String> {
    let where_clause = RelationsBoolExp {
        name: Some(StringComparisonExp {
            eq: Some("active".to_string()),
            ..Default::default()
        }),
        parent_id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
    };
    let operation = RelationsQuery::build(RelationsWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result
        .relations
        .into_iter()
        .find_map(|r| r.node_id.map(|n| n.0)))
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "relations_insert_input"
)]
pub struct RelationsInsertInput {
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub name: Option<String>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub node_id: Option<Uuid>,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub parent_id: Option<Uuid>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "mutation_root",
    variables = "InsertRelationVariables"
)]
pub struct InsertRelationMutation {
    #[arguments(object: $object, on_conflict: $on_conflict)]
    pub insert_relation: Option<RelationRef>,
}

/// Upsert the context's `active` relation to point at `node_id` (keyed on
/// parentId+name so it replaces any prior active). Mirrors `contextSet("active")`.
pub async fn set_active_relation(
    access_token: Option<&str>,
    context_id: &str,
    node_id: Option<&str>,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let object = RelationsInsertInput {
        name: Some("active".to_string()),
        node_id: node_id.map(|n| Uuid(n.to_string())),
        parent_id: Some(Uuid(context_id.to_string())),
    };
    let on_conflict = RelationsOnConflict {
        constraint: RelationsConstraint::RelationsParentIdNameKey,
        update_columns: vec![RelationsUpdateColumn::NodeId],
    };
    let operation = InsertRelationMutation::build(InsertRelationVariables {
        object,
        on_conflict,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_relation.is_some())
}

/// Set (or clear) the projector's focused section for a context — a heading
/// anchor the ScreenApp scrolls to when a document is too long to show whole.
/// Stored as a `focus:<anchor>` relation (the relations table has no free-text
/// column), replacing any previous focus. `None` clears it (show the whole doc).
pub async fn set_screen_focus(
    access_token: Option<&str>,
    context_id: &str,
    anchor: Option<&str>,
) -> Result<(), String> {
    execute_raw_vars(
        access_token,
        "mutation($p: uuid!) { deleteRelations(where: {parentId: {_eq: $p}, name: {_like: \"focus:%\"}}) { affected_rows } }",
        serde_json::json!({ "p": context_id }),
    )
    .await?;
    if let Some(a) = anchor {
        execute_raw_vars(
            access_token,
            "mutation($o: relations_insert_input!) { insertRelation(object: $o) { id } }",
            serde_json::json!({ "o": { "name": format!("focus:{a}"), "parentId": context_id } }),
        )
        .await?;
    }
    Ok(())
}

/// The projector's current focus anchor for a context, if any.
pub async fn screen_focus_anchor(access_token: Option<&str>, context_id: &str) -> Option<String> {
    let q = format!(
        "query {{ relations(where: {{ parentId: {{ _eq: \"{}\" }}, name: {{ _like: \"focus:%\" }} }}, limit: 1) {{ name }} }}",
        gql_escape(context_id)
    );
    let v = execute_raw(access_token, &q).await.ok()?;
    v.pointer("/relations/0/name")
        .and_then(|n| n.as_str())
        .and_then(|s| s.strip_prefix("focus:"))
        .map(str::to_string)
}

/// Whether the context owner has opted to also project the active node's comments
/// on the Screen. Stored as a `screenComments` relation whose non-null `nodeId`
/// means "on" (mirrors how `active` is keyed on parentId+name).
pub async fn screen_comments_on(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<bool, String> {
    let where_clause = RelationsBoolExp {
        name: Some(StringComparisonExp {
            eq: Some("screenComments".to_string()),
            ..Default::default()
        }),
        parent_id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
    };
    let operation = RelationsQuery::build(RelationsWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.relations.into_iter().any(|r| r.node_id.is_some()))
}

/// Owner toggle: show (`on`) or hide the active node's comments on the projector.
/// Upserts the `screenComments` relation, setting `nodeId` to the context (on) or
/// null (off) so ScreenApp's relation subscription picks up the change live.
pub async fn set_screen_comments(
    access_token: Option<&str>,
    context_id: &str,
    on: bool,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let object = RelationsInsertInput {
        name: Some("screenComments".to_string()),
        node_id: on.then(|| Uuid(context_id.to_string())),
        parent_id: Some(Uuid(context_id.to_string())),
    };
    let on_conflict = RelationsOnConflict {
        constraint: RelationsConstraint::RelationsParentIdNameKey,
        update_columns: vec![RelationsUpdateColumn::NodeId],
    };
    let operation = InsertRelationMutation::build(InsertRelationVariables {
        object,
        on_conflict,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_relation.is_some())
}

/// Whether the chair has put the context's feed on the room's screen.
pub async fn screen_feed_on(access_token: Option<&str>, context_id: &str) -> Result<bool, String> {
    let where_clause = RelationsBoolExp {
        name: Some(StringComparisonExp {
            eq: Some("screenFeed".to_string()),
            ..Default::default()
        }),
        parent_id: Some(UuidComparisonExp {
            in_: None,
            eq: Some(Uuid(context_id.to_string())),
            is_null: None,
        }),
    };
    let operation = RelationsQuery::build(RelationsWhereVariables { where_clause });
    let result = execute(access_token, operation).await?;
    Ok(result.relations.into_iter().any(|r| r.node_id.is_some()))
}

/// Chair toggle: show the context's feed on the projector, or stop.
///
/// A projection target that is not a node, so it cannot be the `active` relation
/// (which points at one). It is its own flag, upserted the way `screenComments`
/// is, and the projector prefers it over `active`: asking for the feed is an
/// explicit instruction about what the room should be looking at.
pub async fn set_screen_feed(
    access_token: Option<&str>,
    context_id: &str,
    on: bool,
) -> Result<bool, String> {
    use cynic::MutationBuilder;
    let object = RelationsInsertInput {
        name: Some("screenFeed".to_string()),
        node_id: on.then(|| Uuid(context_id.to_string())),
        parent_id: Some(Uuid(context_id.to_string())),
    };
    let on_conflict = RelationsOnConflict {
        constraint: RelationsConstraint::RelationsParentIdNameKey,
        update_columns: vec![RelationsUpdateColumn::NodeId],
    };
    let operation = InsertRelationMutation::build(InsertRelationVariables {
        object,
        on_conflict,
    });
    let result = execute(access_token, operation).await?;
    Ok(result.insert_relation.is_some())
}
