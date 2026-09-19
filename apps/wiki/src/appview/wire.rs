//! What a view watches, said the way the AppView's live updates are asked for.
//!
//! The interim's views subscribe with a GraphQL filter over rows. Here a change
//! names a context, a kind and an id (`crates/appview/src/live.rs`), so a watch
//! is which context to listen to and which changes there are the view's own.
//! The builders keep the interim's names and arguments, so a component reads
//! the same under either.

use serde_json::json;

/// Where a watch listens.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Scope {
    Context(String),
    /// The context this node is in, which the builder was not told.
    ContextOf(String),
    /// A context and the ones under it: a group's feed shows its events' too.
    Under(String),
    /// Every context the reader belongs to: the feed across all of them.
    Mine,
}

/// A change that is a view's own: its kind, and the id it names if that matters.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Match {
    kind: &'static str,
    id: Option<String>,
}

fn any(kind: &'static str) -> Match {
    Match { kind, id: None }
}

fn on(kind: &'static str, id: &str) -> Match {
    Match {
        kind,
        id: Some(id.to_string()),
    }
}

/// What a push hands the view.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum Shape {
    /// Nothing the view reads: it refetches.
    Touch,
    /// `nodes_stream` rows naming what changed, by `id` and `parentId`.
    Rows,
    /// A canvas's cells since the last push, as `parse_cell_full` reads them.
    Cells { canvas: String },
    /// Whether a poll is open, as a row's `mutable`.
    State { poll: String },
}

/// Which nodes a view cares about. `or` is public because one screen builds a
/// filter of two by hand (the speaker list: its entries, and the list itself).
#[derive(Clone, Debug, Default, PartialEq)]
pub struct NodesBoolExp {
    pub or: Option<Vec<NodesBoolExp>>,
    // Reachable crate-wide only so that `..Default::default()` works there.
    pub(crate) scope: Option<Scope>,
    pub(crate) matches: Vec<Match>,
}

impl NodesBoolExp {
    fn scope(&self) -> Option<Scope> {
        let nested = self.or.iter().flatten().find_map(NodesBoolExp::scope);
        self.scope.clone().or(nested)
    }

    fn matches(&self) -> Vec<Match> {
        let nested = self.or.iter().flatten().flat_map(NodesBoolExp::matches);
        self.matches.iter().cloned().chain(nested).collect()
    }

    /// The id the filter is about: a canvas's, a poll's.
    fn subject(&self) -> String {
        self.matches()
            .into_iter()
            .find_map(|m| m.id)
            .unwrap_or_default()
    }
}

/// The projector's settings on a context. One row there, so one kind of change.
#[derive(Clone, Debug, PartialEq)]
pub struct RelationsBoolExp {
    context: String,
}

/// A watch, ready for the hub.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Wire {
    pub scope: Scope,
    pub matches: Vec<Match>,
    pub shape: Shape,
}

/// One frame from the AppView's `/ws`.
#[derive(Clone, Debug, PartialEq, serde::Deserialize)]
pub struct Change {
    pub topic: String,
    pub kind: String,
    pub id: String,
    /// A new comment's or reaction's own id, `id` being what it is to.
    #[serde(default)]
    pub row: Option<String>,
}

impl Wire {
    /// The row a change is to this watch, if the change is one of its own.
    pub fn row_for(&self, change: &Change) -> Option<serde_json::Value> {
        let own = self
            .matches
            .iter()
            .any(|m| m.kind == change.kind && m.id.as_deref().is_none_or(|id| id == change.id));
        own.then(
            || json!({ "id": change.row.as_deref().unwrap_or(&change.id), "parentId": change.id }),
        )
    }

    /// The rows that make a view of children look again at everything it
    /// watches: what a socket that was down hands over, having missed who knows
    /// what. Empty for a watch with no one thing to look at, such as a feed.
    pub fn rows_after_a_gap(&self) -> Vec<serde_json::Value> {
        self.matches
            .iter()
            .filter_map(|m| m.id.as_deref())
            .map(|id| json!({ "id": id, "parentId": id }))
            .collect()
    }
}

fn wire(filter: &NodesBoolExp, shape: Shape) -> Wire {
    let subject = filter.subject();
    Wire {
        scope: filter.scope().unwrap_or(Scope::ContextOf(subject)),
        matches: filter.matches(),
        shape,
    }
}

/// An id needs no escaping here: it travels as JSON, never inside a query.
pub fn gql_escape(s: &str) -> String {
    s.to_string()
}

/// Anything changing under a node. A change does not say whose child it is, so
/// this is any node of the context changing, as the interim's context signal is.
pub fn children_of(parent_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        scope: Some(Scope::ContextOf(parent_id.to_string())),
        matches: vec![any("node"), any("poll")],
        ..Default::default()
    }
}

/// Children of one kind, for the kinds that have a change of their own.
pub fn children_of_mime(parent_id: &str, mime: &str) -> NodesBoolExp {
    let matches = match mime {
        "canvas/pixel" => vec![on("canvas", parent_id)],
        "vote/vote" => vec![on("tally", parent_id), on("poll", parent_id)],
        "speak/speak" => vec![on("speak", parent_id)],
        "vote/comment" => vec![on("comment", parent_id)],
        "vote/reaction" => vec![on("reaction", parent_id)],
        _ => return children_of(parent_id),
    };
    NodesBoolExp {
        scope: Some(Scope::ContextOf(parent_id.to_string())),
        matches,
        ..Default::default()
    }
}

/// One node, whatever kind it turns out to be.
pub fn node_is(id: &str) -> NodesBoolExp {
    NodesBoolExp {
        scope: Some(Scope::ContextOf(id.to_string())),
        matches: ["node", "poll", "speak", "canvas"]
            .into_iter()
            .map(|kind| on(kind, id))
            .collect(),
        ..Default::default()
    }
}

/// The comments or the reactions on a node.
pub fn in_context_or_under(context_id: Option<&str>, node_id: &str, mime: &str) -> NodesBoolExp {
    let under = children_of_mime(node_id, mime);
    match context_id {
        Some(context) => NodesBoolExp {
            scope: Some(Scope::Context(context.to_string())),
            ..under
        },
        None => under,
    }
}

/// What a feed shows: a context's and those under it, or all the reader's.
pub fn feed_scope(context_id: Option<&str>, _user_id: &str) -> NodesBoolExp {
    NodesBoolExp {
        scope: Some(match context_id {
            Some(context) => Scope::Under(context.to_string()),
            None => Scope::Mine,
        }),
        matches: vec![any("node"), any("comment"), any("reaction")],
        ..Default::default()
    }
}

pub fn relation_named(parent_id: &str, _name: &str) -> RelationsBoolExp {
    RelationsBoolExp {
        context: parent_id.to_string(),
    }
}

pub fn relations_named(parent_id: &str, _names: &[&str]) -> RelationsBoolExp {
    relation_named(parent_id, "")
}

pub fn relations_like(parent_id: &str, _pattern: &str) -> RelationsBoolExp {
    relation_named(parent_id, "")
}

/// What the projector of a context shows, changing.
pub fn relations_changed(where_clause: RelationsBoolExp) -> Wire {
    Wire {
        matches: vec![on("screen", &where_clause.context)],
        scope: Scope::Context(where_clause.context),
        shape: Shape::Touch,
    }
}

/// Something a view shows changing, in `context_id` where the view knows it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the interim's signature, which the call sites are written to"
)]
pub fn node_changed(context_id: Option<&str>, fallback: NodesBoolExp) -> Wire {
    let mut wire = wire(&fallback, Shape::Touch);
    if let Some(context) = context_id {
        wire.scope = Scope::Context(context.to_string());
    }
    wire
}

/// The ids of what lands in a scope. `since` is the interim's cursor: here a
/// push says what changed, and a gap is made up by looking again.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the interim's signature, which the call sites are written to"
)]
pub fn id_stream(where_clause: NodesBoolExp, _since: &str, _batch: i32) -> Wire {
    wire(&where_clause, Shape::Rows)
}

/// Which parent's children changed, for a watcher that only cares about its own.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the interim's signature, which the call sites are written to"
)]
pub fn parent_stream(where_clause: NodesBoolExp, _since: &str, _batch: i32) -> Wire {
    wire(&where_clause, Shape::Rows)
}

/// One poll's `mutable`, as the chair opens and closes it.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the interim's signature, which the call sites are written to"
)]
pub fn state_stream(where_clause: NodesBoolExp, _since: &str) -> Wire {
    let poll = where_clause.subject();
    wire(&where_clause, Shape::State { poll })
}

/// Cells painted on a canvas.
#[expect(
    clippy::needless_pass_by_value,
    reason = "the interim's signature, which the call sites are written to"
)]
pub fn cell_stream(where_clause: NodesBoolExp, _since: &str) -> Wire {
    let canvas = where_clause.subject();
    wire(&where_clause, Shape::Cells { canvas })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn change(kind: &str, id: &str, row: Option<&str>) -> Change {
        Change {
            topic: "context:c1".into(),
            kind: kind.into(),
            id: id.into(),
            row: row.map(str::to_string),
        }
    }

    /// A thread wakes for an answer to ITS comment, and is told which row is
    /// new; the forty other threads on the page sleep through it.
    #[test]
    fn a_thread_hears_its_own_answers_and_no_others() {
        let thread = parent_stream(
            in_context_or_under(Some("c1"), "k1", "vote/comment"),
            "",
            100,
        );
        assert_eq!(thread.scope, Scope::Context("c1".into()));
        assert_eq!(
            thread.row_for(&change("comment", "k1", Some("k2"))),
            Some(json!({ "id": "k2", "parentId": "k1" }))
        );
        assert_eq!(thread.row_for(&change("comment", "k9", Some("k3"))), None);
        assert_eq!(thread.row_for(&change("reaction", "k1", None)), None);
        assert_eq!(
            thread.rows_after_a_gap(),
            [json!({ "id": "k1", "parentId": "k1" })]
        );
    }

    #[test]
    fn a_watch_without_a_context_asks_where_its_node_is() {
        let thread = parent_stream(in_context_or_under(None, "k1", "vote/comment"), "", 100);
        assert_eq!(thread.scope, Scope::ContextOf("k1".into()));
        let cells = cell_stream(children_of_mime("cv", "canvas/pixel"), "");
        assert_eq!(cells.scope, Scope::ContextOf("cv".into()));
        assert_eq!(
            cells.shape,
            Shape::Cells {
                canvas: "cv".into()
            }
        );
        assert!(cells.row_for(&change("canvas", "cv", None)).is_some());
        assert!(cells.row_for(&change("canvas", "other", None)).is_none());
    }

    /// The speaker list's filter is built by hand at its call site, from two.
    #[test]
    fn a_filter_of_two_hears_either() {
        let list = node_changed(
            Some("c1"),
            NodesBoolExp {
                or: Some(vec![children_of_mime("l1", "speak/speak"), node_is("l1")]),
                ..Default::default()
            },
        );
        assert_eq!(list.scope, Scope::Context("c1".into()));
        assert!(list.row_for(&change("speak", "l1", None)).is_some());
        assert!(list.row_for(&change("node", "l1", None)).is_some());
        assert!(list.row_for(&change("speak", "l2", None)).is_none());
    }

    #[test]
    fn a_feed_hears_what_lands_and_names_the_row_to_fetch() {
        let group = id_stream(feed_scope(Some("c1"), "did:plc:a"), "", 100);
        assert_eq!(group.scope, Scope::Under("c1".into()));
        let landed = group.row_for(&change("comment", "d1", Some("k7")));
        assert_eq!(landed.expect("heard")["id"], "k7");
        assert_eq!(
            group.row_for(&change("node", "d2", None)).expect("heard")["id"],
            "d2"
        );
        assert!(group.row_for(&change("member", "m1", None)).is_none());
        assert!(group.rows_after_a_gap().is_empty());
        assert_eq!(
            id_stream(feed_scope(None, "did:plc:a"), "", 100).scope,
            Scope::Mine
        );
    }

    #[test]
    fn the_screen_and_a_poll_are_watched_by_what_changes_them() {
        let screen = relations_changed(relations_named("c1", &["active", "screenFeed"]));
        assert!(screen.row_for(&change("screen", "c1", None)).is_some());
        assert!(screen.row_for(&change("node", "c1", None)).is_none());

        let tally = node_changed(Some("c1"), children_of_mime("p1", "vote/vote"));
        assert!(tally.row_for(&change("tally", "p1", None)).is_some());
        let open = state_stream(node_is("p1"), "");
        assert_eq!(open.shape, Shape::State { poll: "p1".into() });
        assert!(open.row_for(&change("poll", "p1", None)).is_some());
    }

    #[test]
    fn a_frame_is_read_with_or_without_a_row() {
        let plain: Change =
            serde_json::from_str(r#"{"topic":"context:c1","kind":"node","id":"d1"}"#)
                .expect("a frame");
        assert_eq!(plain, change("node", "d1", None));
        let with: Change =
            serde_json::from_str(r#"{"topic":"context:c1","kind":"comment","id":"d1","row":"k1"}"#)
                .expect("a frame");
        assert_eq!(with.row.as_deref(), Some("k1"));
    }
}
