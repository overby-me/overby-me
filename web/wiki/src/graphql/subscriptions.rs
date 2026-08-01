//! Every live subscription the app opens, as TYPED operations.
//!
//! These used to be format! strings. Nothing checked them: not the compiler,
//! which cannot read GraphQL, and not a test, which would only have asserted the
//! same string back. The first thing that checked one was the server, in front of
//! a user — twice in one evening. A filter that carried its own braces went out
//! as `where: { {_or: …} }` and every reader of the feed silently stopped getting
//! updates, and an aggregate over a table with no timestamp would have been
//! rejected the same way.
//!
//! Built with cynic against `graphql/schema.graphql`, so a wrong field name, a
//! wrong argument or a filter on a column that does not exist is a compile error
//! here rather than a refused subscription there. The `where` clauses are the
//! same typed input objects the queries use, so a filter can no longer be
//! malformed at all.

use super::*;

// --- Streaming cursor ---

/// Which column a stream advances on, and from where.
///
/// `updated_at` is the only usable cursor on `nodes`: it is trigger-maintained,
/// so it moves on an edit as well as an insert. `created_at` would miss every
/// update, and no other table this app subscribes to has either.
#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_stream_cursor_value_input"
)]
pub struct NodesStreamCursorValue {
    #[cynic(rename = "updatedAt", skip_serializing_if = "Option::is_none")]
    pub updated_at: Option<Timestamptz>,
}

#[derive(cynic::InputObject, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_stream_cursor_input"
)]
pub struct NodesStreamCursor {
    #[cynic(rename = "initial_value")]
    pub initial_value: NodesStreamCursorValue,
    #[cynic(skip_serializing_if = "Option::is_none")]
    pub ordering: Option<CursorOrdering>,
}

#[derive(cynic::Enum, Clone, Copy, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "cursor_ordering"
)]
pub enum CursorOrdering {
    #[cynic(rename = "ASC")]
    Asc,
    #[cynic(rename = "DESC")]
    Desc,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct StreamVariables {
    pub where_clause: NodesBoolExp,
    pub cursor: Vec<NodesStreamCursor>,
    pub batch: i32,
}

// --- The rows a stream carries ---

/// Just the id: the caller fetches the rows properly (the feed).
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct StreamedId {
    pub id: Uuid,
}

/// Whose child changed, so a watcher can tell whether it was its own.
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct StreamedParent {
    pub parent_id: Option<Uuid>,
}

/// A node's live state (a poll opening or closing).
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct StreamedState {
    pub id: Uuid,
    pub mutable: bool,
}

/// A painted cell.
#[derive(cynic::QueryFragment, Debug, Clone)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "nodes")]
pub struct StreamedCell {
    pub key: String,
    pub data: Option<Jsonb>,
    /// Who painted it, so a cell that arrives while you are looking can be
    /// attributed without re-reading the board.
    pub owner_id: Option<Uuid>,
}

// --- The subscriptions ---

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "subscription_root",
    variables = "StreamVariables"
)]
pub struct IdStream {
    #[cynic(rename = "nodes_stream")]
    #[arguments(batch_size: $batch, cursor: $cursor, where: $where_clause)]
    pub nodes_stream: Vec<StreamedId>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "subscription_root",
    variables = "StreamVariables"
)]
pub struct ParentStream {
    #[cynic(rename = "nodes_stream")]
    #[arguments(batch_size: $batch, cursor: $cursor, where: $where_clause)]
    pub nodes_stream: Vec<StreamedParent>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "subscription_root",
    variables = "StreamVariables"
)]
pub struct StateStream {
    #[cynic(rename = "nodes_stream")]
    #[arguments(batch_size: $batch, cursor: $cursor, where: $where_clause)]
    pub nodes_stream: Vec<StreamedState>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "subscription_root",
    variables = "StreamVariables"
)]
pub struct CellStream {
    #[cynic(rename = "nodes_stream")]
    #[arguments(batch_size: $batch, cursor: $cursor, where: $where_clause)]
    pub nodes_stream: Vec<StreamedCell>,
}

/// A query and its variables, ready for the subscription hub.
///
/// The hub speaks the wire protocol, not cynic, so an operation is flattened
/// here — but it was BUILT from the schema, which is the whole point.
pub struct Wire {
    pub query: String,
    pub variables: serde_json::Value,
}

fn wire<Q, V: serde::Serialize>(op: cynic::StreamingOperation<Q, V>) -> Wire {
    // A StreamingOperation keeps its parts private and serialises to exactly the
    // wire shape, `{query, variables}`, which is what the hub sends.
    let body = serde_json::to_value(&op).unwrap_or(serde_json::Value::Null);
    Wire {
        query: body
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or_default()
            .to_string(),
        variables: body
            .get("variables")
            .cloned()
            .unwrap_or(serde_json::Value::Null),
    }
}

/// The cursor every stream starts from: "everything after this moment".
fn from(since: &str) -> Vec<NodesStreamCursor> {
    vec![NodesStreamCursor {
        initial_value: NodesStreamCursorValue {
            updated_at: Some(Timestamptz(since.to_string())),
        },
        ordering: Some(CursorOrdering::Asc),
    }]
}

/// Ids of nodes matching `where_clause`, as they change.
pub fn id_stream(where_clause: NodesBoolExp, since: &str, batch: i32) -> Wire {
    use cynic::SubscriptionBuilder;
    wire(IdStream::build(StreamVariables {
        where_clause,
        cursor: from(since),
        batch,
    }))
}

/// Which parent's children changed, for a watcher that only cares about its own.
pub fn parent_stream(where_clause: NodesBoolExp, since: &str, batch: i32) -> Wire {
    use cynic::SubscriptionBuilder;
    wire(ParentStream::build(StreamVariables {
        where_clause,
        cursor: from(since),
        batch,
    }))
}

/// One node's `mutable`, as it changes.
pub fn state_stream(where_clause: NodesBoolExp, since: &str) -> Wire {
    use cynic::SubscriptionBuilder;
    wire(StateStream::build(StreamVariables {
        where_clause,
        cursor: from(since),
        batch: 10,
    }))
}

/// Cells painted on a canvas.
pub fn cell_stream(where_clause: NodesBoolExp, since: &str) -> Wire {
    use cynic::SubscriptionBuilder;
    wire(CellStream::build(StreamVariables {
        where_clause,
        cursor: from(since),
        batch: 200,
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The filter is a typed input object, so the double-wrapped `where` that
    /// broke the feed cannot be expressed: there is no string to wrap.
    #[test]
    fn a_stream_carries_its_filter_as_a_variable() {
        let w = id_stream(NodesBoolExp::default(), "2026-01-01T00:00:00Z", 100);
        assert!(w.query.contains("nodes_stream"), "{}", w.query);
        assert!(
            w.query.contains("$whereClause"),
            "the filter travels as a variable: {}",
            w.query
        );
        assert!(!w.query.contains("{ {"), "{}", w.query);
        // ...and every variable it declares is actually sent.
        let vars = w.variables.as_object().expect("variables are an object");
        for key in ["whereClause", "cursor", "batch"] {
            assert!(vars.contains_key(key), "missing {key} in {vars:?}");
            assert!(
                w.query.contains(&format!("${key}")),
                "undeclared {key}: {}",
                w.query
            );
        }
    }

    /// The cursor is ascending on `updated_at`, which is what makes a stream
    /// deliver edits and not only insertions.
    #[test]
    fn the_cursor_is_ascending_on_updated_at() {
        let w = cell_stream(NodesBoolExp::default(), "2026-01-01T00:00:00Z");
        let cursor = &w.variables["cursor"][0];
        assert_eq!(cursor["initial_value"]["updatedAt"], "2026-01-01T00:00:00Z");
        assert_eq!(cursor["ordering"], "ASC");
    }
}

// --- Change tokens (for the tables a stream cannot cover) ---

/// The newest timestamp among the matching rows, beside their count.
///
/// Together these move on an insert, an edit and a delete, which is what makes a
/// two-field aggregate a complete change token for `nodes` — and 101 bytes rather
/// than the rows themselves.
#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_max_fields"
)]
pub struct NodesMaxUpdated {
    pub updated_at: Option<Timestamptz>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_aggregate_fields"
)]
pub struct NodesChangeFields {
    pub count: i32,
    pub max: Option<NodesMaxUpdated>,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "nodes_aggregate"
)]
pub struct NodesChangeAggregate {
    pub aggregate: Option<NodesChangeFields>,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct NodesWhereOnly {
    pub where_clause: NodesBoolExp,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "subscription_root",
    variables = "NodesWhereOnly"
)]
pub struct NodesChanged {
    #[cynic(rename = "nodesAggregate")]
    #[arguments(where: $where_clause)]
    pub nodes_aggregate: NodesChangeAggregate,
}

/// Something under `where_clause` changed. Carries no rows.
pub fn nodes_changed_typed(where_clause: NodesBoolExp) -> Wire {
    use cynic::SubscriptionBuilder;
    wire(NodesChanged::build(NodesWhereOnly { where_clause }))
}

// --- relations and members: no cursor, so a live query and not a stream ---

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "relations")]
pub struct WatchedRelation {
    pub node_id: Option<Uuid>,
    /// Selected as well as the id because a subscription fires when its RESULT
    /// changes: these rows are upserted, and the name is what tells two of them
    /// apart when one is replaced.
    pub name: String,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct RelationsWhereOnly {
    pub where_clause: RelationsBoolExp,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "subscription_root",
    variables = "RelationsWhereOnly"
)]
pub struct RelationsChanged {
    #[arguments(where: $where_clause)]
    pub relations: Vec<WatchedRelation>,
}

/// What the chair has pointed the room at, as it changes.
pub fn relations_changed(where_clause: RelationsBoolExp) -> Wire {
    use cynic::SubscriptionBuilder;
    wire(RelationsChanged::build(RelationsWhereOnly { where_clause }))
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(schema_path = "graphql/schema.graphql", graphql_type = "members")]
pub struct WatchedMember {
    pub id: Uuid,
    /// `accepted` and `active` as well as the id: accepting an invitation is an
    /// UPDATE, and a selection of ids alone never noticed it.
    pub accepted: bool,
    pub active: bool,
}

#[derive(cynic::QueryVariables, Debug)]
pub struct MembersWhereOnly {
    pub where_clause: MembersBoolExp,
}

#[derive(cynic::QueryFragment, Debug)]
#[cynic(
    schema_path = "graphql/schema.graphql",
    graphql_type = "subscription_root",
    variables = "MembersWhereOnly"
)]
pub struct MembersChanged {
    #[arguments(where: $where_clause)]
    pub members: Vec<WatchedMember>,
}

/// This person's memberships and invitations, as they change.
pub fn members_changed(where_clause: MembersBoolExp) -> Wire {
    use cynic::SubscriptionBuilder;
    wire(MembersChanged::build(MembersWhereOnly { where_clause }))
}

// --- Filters, spelled once ---
//
// Each of these used to be a format! string at a call site. As values they are
// checked by the compiler, cannot be composed wrongly, and read the same
// wherever they appear.

/// Children of a node.
pub fn children_of(parent_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(parent_id.to_string())),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Children of a node, of one kind.
pub fn children_of_mime(parent_id: &str, mime: &str) -> NodesBoolExp {
    NodesBoolExp {
        and: Some(vec![children_of(parent_id), of_mime(mime)]),
        ..Default::default()
    }
}

/// One kind of node.
pub fn of_mime(mime: &str) -> NodesBoolExp {
    NodesBoolExp {
        mime_id: Some(StringComparisonExp {
            eq: Some(mime.to_string()),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// One node, by id.
pub fn node_is(id: &str) -> NodesBoolExp {
    NodesBoolExp {
        id: Some(UuidComparisonExp {
            eq: Some(Uuid(id.to_string())),
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// Everything of one kind in a context — or, lacking a context, under one node.
///
/// The fallback matters: a comment on a node outside any context still has to
/// wake its own thread.
pub fn in_context_or_under(context_id: Option<&str>, node_id: &str, mime: &str) -> NodesBoolExp {
    let scope = match context_id {
        Some(ctx) => NodesBoolExp {
            context_id: Some(UuidComparisonExp {
                eq: Some(Uuid(ctx.to_string())),
                ..Default::default()
            }),
            ..Default::default()
        },
        None => children_of(node_id),
    };
    NodesBoolExp {
        and: Some(vec![scope, of_mime(mime)]),
        ..Default::default()
    }
}

/// A named relation on a context (the projector's active node, and friends).
pub fn relation_named(parent_id: &str, name: &str) -> RelationsBoolExp {
    RelationsBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(parent_id.to_string())),
            ..Default::default()
        }),
        name: Some(StringComparisonExp {
            eq: Some(name.to_string()),
            ..Default::default()
        }),
    }
}

/// Several named relations on a context, watched together.
pub fn relations_named(parent_id: &str, names: &[&str]) -> RelationsBoolExp {
    RelationsBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(parent_id.to_string())),
            ..Default::default()
        }),
        name: Some(StringComparisonExp {
            in_: Some(names.iter().map(|n| n.to_string()).collect()),
            ..Default::default()
        }),
    }
}

/// Relations whose name matches a pattern (the projector's `focus:<anchor>`).
pub fn relations_like(parent_id: &str, pattern: &str) -> RelationsBoolExp {
    RelationsBoolExp {
        parent_id: Some(UuidComparisonExp {
            eq: Some(Uuid(parent_id.to_string())),
            ..Default::default()
        }),
        name: Some(StringComparisonExp {
            ilike: Some(pattern.to_string()),
            ..Default::default()
        }),
    }
}

/// This person's memberships, wherever they are.
pub fn memberships_of(user_id: &str) -> MembersBoolExp {
    MembersBoolExp {
        node_id: Some(UuidComparisonExp {
            eq: Some(Uuid(user_id.to_string())),
            ..Default::default()
        }),
        ..Default::default()
    }
}
