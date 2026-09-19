//! What kind of thing an id names.
//!
//! The components write through a handful of generic calls (`update_node`,
//! `delete_node`, `bin_node`) because to the interim everything is a row of one
//! table. The AppView has a method per thing, so a write has to know what it is
//! writing to. Every id a component can hand to a write came out of a read
//! through this layer first, and each read says here what it saw.

use std::cell::RefCell;
use std::collections::HashMap;

#[derive(Debug, Clone, PartialEq)]
pub(crate) enum Seen {
    Context,
    /// A document, by its AppView kind (`poll` and `canvas` have methods of
    /// their own for some writes).
    Document(String),
    Comment,
    /// Taken back by what it is to and what it says, not by its own id.
    Reaction {
        subject: String,
        emoji: String,
    },
    Feedback,
    SpeakerList,
    SpeakerEntry,
}

thread_local! {
    static SEEN: RefCell<HashMap<String, Seen>> = RefCell::new(HashMap::new());
}

pub(crate) fn saw(id: &str, what: Seen) {
    SEEN.with(|seen| {
        seen.borrow_mut().insert(id.to_string(), what);
    });
}

pub(crate) fn seen(id: &str) -> Option<Seen> {
    SEEN.with(|seen| seen.borrow().get(id).cloned())
}

/// A node of the tree, by the `node` and `kind` a view carries.
pub(crate) fn saw_node(id: &str, node: &str, kind: &str) {
    saw(
        id,
        match node {
            "context" => Seen::Context,
            _ => Seen::Document(kind.to_string()),
        },
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_read_says_what_it_saw_and_a_write_asks() {
        assert_eq!(seen("nobody"), None);
        saw_node("c1", "context", "group");
        saw_node("p1", "document", "poll");
        saw(
            "r1",
            Seen::Reaction {
                subject: "k1".into(),
                emoji: "👍".into(),
            },
        );
        assert_eq!(seen("c1"), Some(Seen::Context));
        assert_eq!(seen("p1"), Some(Seen::Document("poll".into())));
        assert!(matches!(seen("r1"), Some(Seen::Reaction { .. })));
    }
}
