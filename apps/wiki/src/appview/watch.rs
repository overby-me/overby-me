//! The watches a tab holds on the AppView's `/ws`, and what each is handed.
//!
//! The bookkeeping and the asking, apart from the socket (`hub.rs`) so that
//! both can be tested without a browser. Views asking for the same watch share
//! one, and watches on one context share its topic: forty reaction bars on a
//! motion are one subscription, as they are in the interim's hub.

use super::seen::{place_of, placed};
use super::wire::{Change, Scope, Shape, Wire};
use super::{ask_quiet, client};
use appview_client::{get_node, list_children, list_contexts};
use serde_json::json;
use std::collections::{BTreeSet, HashMap};

/// How far under a context a feed listens. A group holds events; nothing in
/// use nests deeper, and every context listened to is a grant asked for.
const DEPTH_UNDER: usize = 2;
const MOST_TOPICS: usize = 24;

fn topic(context: &str) -> String {
    format!("context:{context}")
}

/// The topics a watch listens on, asked of the AppView where the watch does
/// not say. Empty when its node is nowhere this reader can see.
pub(crate) async fn topics_of(access_token: Option<&str>, scope: &Scope) -> Vec<String> {
    let client = client(access_token);
    match scope {
        Scope::Context(context) => vec![topic(context)],
        Scope::ContextOf(node) => {
            if let Some(context) = place_of(node) {
                return vec![topic(&context)];
            }
            let params = get_node::Params {
                id: Some(node.clone()),
                path: None,
            };
            match ask_quiet(true, || client.get_node(&params)).await {
                Ok(read) => {
                    let context = match &read.node {
                        get_node::OutputNode::Context(c) => &c.id,
                        get_node::OutputNode::Document(d) => &d.context_id,
                    };
                    placed(node, context);
                    vec![topic(context)]
                }
                Err(_) => Vec::new(),
            }
        }
        Scope::Under(context) => {
            let mut found = vec![context.clone()];
            let mut level = vec![context.clone()];
            for _ in 0..DEPTH_UNDER {
                let mut next = Vec::new();
                for parent in &level {
                    let params = list_children::Params {
                        parent: parent.clone(),
                    };
                    let Ok(listed) = ask_quiet(true, || client.list_children(&params)).await else {
                        continue;
                    };
                    next.extend(
                        listed
                            .children
                            .into_iter()
                            .filter(|child| child.node == "context")
                            .map(|child| child.id),
                    );
                }
                found.extend(next.iter().cloned());
                level = next;
            }
            found.iter().take(MOST_TOPICS).map(|c| topic(c)).collect()
        }
        Scope::Mine => {
            let params = list_contexts::Params {
                scope: Some("mine".to_string()),
                ..Default::default()
            };
            match ask_quiet(true, || client.list_contexts(&params)).await {
                Ok(mine) => mine
                    .contexts
                    .iter()
                    .take(MOST_TOPICS)
                    .map(|context| topic(&context.id))
                    .collect(),
                Err(_) => Vec::new(),
            }
        }
    }
}

/// What a view is handed for `rows` of changes. A canvas and a poll are asked
/// what changed, since a change only ever says that something did.
pub(crate) async fn payload(
    access_token: Option<&str>,
    wire: &Wire,
    rows: Vec<serde_json::Value>,
    cursor: &mut Option<String>,
) -> Option<serde_json::Value> {
    match &wire.shape {
        Shape::Touch => Some(json!({ "changed": true })),
        Shape::Rows => (!rows.is_empty()).then(|| json!({ "nodes_stream": rows })),
        Shape::Cells { canvas } => {
            let cells = super::canvas::cells_since(access_token, canvas, cursor).await;
            (!cells.is_empty()).then(|| json!({ "nodes_stream": cells }))
        }
        Shape::State { poll } => {
            let poll = super::vote::read_poll(access_token, poll).await?;
            Some(json!({ "nodes_stream": [{ "mutable": poll.open }] }))
        }
    }
}

/// One listener's place among the watches.
#[derive(Clone, Debug, PartialEq)]
pub(crate) struct Handle {
    watch: u64,
    sink: u64,
}

struct Watch<S> {
    wire: Wire,
    /// `None` until the AppView has been asked where the watch listens.
    topics: Option<Vec<String>>,
    sinks: Vec<(u64, S)>,
    /// A grant after the first means the socket was down in between.
    granted_before: bool,
    cursor: Option<String>,
}

/// Generic over the sink, so this is tested without a renderer.
pub(crate) struct Watches<S> {
    watches: HashMap<u64, Watch<S>>,
    by_wire: HashMap<Wire, u64>,
    next: u64,
}

impl<S> Default for Watches<S> {
    fn default() -> Self {
        Watches {
            watches: HashMap::new(),
            by_wire: HashMap::new(),
            next: 0,
        }
    }
}

/// What a grant means to one watch.
#[derive(Debug, PartialEq)]
pub(crate) struct Granted {
    pub watch: u64,
    /// The socket was down since this watch was last granted.
    pub again: bool,
}

impl<S: Clone> Watches<S> {
    /// Add a listener. `Some(watch)` when the watch is new and its topics are
    /// still to be asked for.
    pub fn register(&mut self, wire: Wire, sink: S) -> (Handle, Option<u64>) {
        self.next += 1;
        let sink_id = self.next;
        if let Some(&watch) = self.by_wire.get(&wire) {
            if let Some(entry) = self.watches.get_mut(&watch) {
                entry.sinks.push((sink_id, sink));
                return (
                    Handle {
                        watch,
                        sink: sink_id,
                    },
                    None,
                );
            }
        }
        self.next += 1;
        let watch = self.next;
        self.by_wire.insert(wire.clone(), watch);
        self.watches.insert(
            watch,
            Watch {
                wire,
                topics: None,
                sinks: vec![(sink_id, sink)],
                granted_before: false,
                cursor: None,
            },
        );
        (
            Handle {
                watch,
                sink: sink_id,
            },
            Some(watch),
        )
    }

    fn listened(&self) -> BTreeSet<String> {
        self.watches
            .values()
            .flat_map(|w| w.topics.iter().flatten().cloned())
            .collect()
    }

    /// Record where a watch listens. Returns the topics nobody listened on yet.
    pub fn resolved(&mut self, watch: u64, topics: Vec<String>) -> Vec<String> {
        let before = self.listened();
        match self.watches.get_mut(&watch) {
            Some(entry) => entry.topics = Some(topics.clone()),
            // Gone while the AppView was being asked.
            None => return Vec::new(),
        }
        let fresh: BTreeSet<String> = topics.into_iter().collect();
        fresh.difference(&before).cloned().collect()
    }

    /// Remove a listener. Returns the topics nobody listens on any more.
    pub fn deregister(&mut self, handle: &Handle) -> Vec<String> {
        let Some(entry) = self.watches.get_mut(&handle.watch) else {
            return Vec::new();
        };
        entry.sinks.retain(|(id, _)| *id != handle.sink);
        if !entry.sinks.is_empty() {
            return Vec::new();
        }
        let before = self.listened();
        if let Some(gone) = self.watches.remove(&handle.watch) {
            self.by_wire.remove(&gone.wire);
        }
        before.difference(&self.listened()).cloned().collect()
    }

    /// Every topic to ask for, after a (re)connect.
    pub fn topics(&self) -> Vec<String> {
        self.listened().into_iter().collect()
    }

    pub fn is_empty(&self) -> bool {
        self.watches.is_empty()
    }

    /// The AppView granted `topic`: the watches on it, and whether each has
    /// been granted before.
    pub fn granted(&mut self, topic: &str) -> Vec<Granted> {
        self.watches
            .iter_mut()
            .filter(|(_, w)| w.topics.iter().flatten().any(|t| t == topic))
            .map(|(&watch, w)| Granted {
                watch,
                again: std::mem::replace(&mut w.granted_before, true),
            })
            .collect()
    }

    pub fn on_topic(&self, topic: &str) -> Vec<u64> {
        self.watches
            .iter()
            .filter(|(_, w)| w.topics.iter().flatten().any(|t| t == topic))
            .map(|(&watch, _)| watch)
            .collect()
    }

    /// The watches a change is for, each with the row it is to that watch.
    pub fn heard(&self, change: &Change) -> Vec<(u64, serde_json::Value)> {
        self.watches
            .iter()
            .filter(|(_, w)| w.topics.iter().flatten().any(|t| *t == change.topic))
            .filter_map(|(&watch, w)| Some((watch, w.wire.row_for(change)?)))
            .collect()
    }

    /// A watch's wire, cursor and listeners, to make and hand out a payload.
    pub fn parts(&self, watch: u64) -> Option<(Wire, Option<String>, Vec<S>)> {
        let entry = self.watches.get(&watch)?;
        let sinks = entry.sinks.iter().map(|(_, sink)| sink.clone()).collect();
        Some((entry.wire.clone(), entry.cursor.clone(), sinks))
    }

    pub fn moved_on(&mut self, watch: u64, cursor: Option<String>) {
        if let Some(entry) = self.watches.get_mut(&watch) {
            entry.cursor = cursor;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::wire::{
        children_of, in_context_or_under, node_changed, parent_stream, relation_named,
        relations_changed,
    };
    use super::*;

    fn change(topic: &str, kind: &str, id: &str) -> Change {
        Change {
            topic: topic.into(),
            kind: kind.into(),
            id: id.into(),
            row: None,
        }
    }

    /// Forty reaction bars on one motion are one watch, and every watch in a
    /// context rides one topic: a hall of phones is a grant each, not forty.
    #[test]
    fn the_same_watch_is_held_once_and_a_context_is_one_topic() {
        let mut watches = Watches::<&str>::default();
        let folder = || node_changed(Some("c1"), children_of("d1"));
        let (first, new) = watches.register(folder(), "drawer");
        let (second, again) = watches.register(folder(), "page");
        let folder_watch = new.expect("new");
        assert_eq!(again, None, "asked for twice");
        assert_eq!(
            watches.resolved(folder_watch, vec!["context:c1".into()]),
            ["context:c1"]
        );

        let screen = relations_changed(relation_named("c1", "active"));
        let (third, new) = watches.register(screen, "screen");
        assert!(
            watches
                .resolved(new.expect("new"), vec!["context:c1".into()])
                .is_empty(),
            "a topic already listened on was asked for again"
        );

        let heard = watches.heard(&change("context:c1", "node", "d7"));
        assert_eq!(heard.len(), 1, "the screen is not a node changing");
        let (_, _, sinks) = watches.parts(heard[0].0).expect("the folder");
        assert_eq!(sinks, ["drawer", "page"]);

        assert!(watches.deregister(&first).is_empty());
        assert!(
            watches.deregister(&second).is_empty(),
            "the screen still listens"
        );
        assert_eq!(watches.deregister(&third), ["context:c1"]);
        assert!(watches.is_empty());
    }

    #[test]
    fn a_second_grant_means_there_was_a_gap() {
        let mut watches = Watches::<u8>::default();
        let thread = parent_stream(
            in_context_or_under(Some("c1"), "k1", "vote/comment"),
            "",
            100,
        );
        let (_, new) = watches.register(thread, 1);
        let watch = new.expect("new");
        assert!(
            watches.granted("context:c1").is_empty(),
            "not yet known to listen there"
        );
        watches.resolved(watch, vec!["context:c1".into()]);
        assert_eq!(
            watches.granted("context:c1"),
            [Granted {
                watch,
                again: false
            }]
        );
        assert_eq!(
            watches.granted("context:c1"),
            [Granted { watch, again: true }]
        );
        assert!(watches.granted("context:c2").is_empty());
    }

    #[test]
    fn a_watch_that_left_while_being_placed_is_not_listened_for() {
        let mut watches = Watches::<u8>::default();
        let (handle, new) = watches.register(node_changed(None, children_of("d1")), 1);
        assert!(watches.deregister(&handle).is_empty());
        assert!(watches
            .resolved(new.expect("new"), vec!["context:c1".into()])
            .is_empty());
        assert!(watches.topics().is_empty());
    }
}
