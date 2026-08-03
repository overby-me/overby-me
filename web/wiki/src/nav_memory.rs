//! Where the reader was, per context and app.
//!
//! The app rail is a CONTEXT rail: every entry targets the context root, and it
//! has to, because an app renders the node it sits on and a speaker list hanging
//! off a document is not the group's speaker list. That is right for going TO an
//! app and wrong for coming BACK. A delegate three levels into an agenda who
//! checks the speaker list and returns landed at the top of the group, having
//! lost both the page they were reading and their place on it.
//!
//! So each (context, app) pair remembers the last route seen in it and the
//! scroll it was left at. The rail consults that on the way back.
//!
//! Deliberately in memory only: this is "where was I a moment ago", not a
//! preference, and a remembered path that outlived the tab would be a surprise
//! rather than a convenience.

use std::collections::HashMap;

use dioxus::prelude::*;

/// A place in a context's app.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Spot {
    /// The full route path, which is at or below the context root.
    pub segments: Vec<String>,
    /// Window scroll when the reader left it.
    pub scroll: f64,
}

static SPOTS: GlobalSignal<HashMap<String, Spot>> = Signal::global(HashMap::new);

/// One key per context and app view.
///
/// `\u{1}` rather than `/`, so a context named after an app cannot collide with
/// one that has that app open.
fn key(ctx: &[String], app: Option<&str>) -> String {
    format!("{}\u{1}{}", ctx.join("/"), app.unwrap_or(""))
}

/// Record where the reader is in this context's app.
///
/// Only routes at or below the context root are worth remembering; anything
/// else is a different context and belongs to its own key.
pub fn remember(ctx: &[String], app: Option<&str>, segments: &[String], scroll: f64) {
    if ctx.is_empty() || !segments.starts_with(ctx) {
        return;
    }
    let spot = Spot {
        segments: segments.to_vec(),
        scroll,
    };
    let slot = key(ctx, app);
    // Only write a change. The rail subscribes to this, and re-rendering it to
    // store the value it already held would be a render for nothing.
    if SPOTS.peek().get(&slot) == Some(&spot) {
        return;
    }
    SPOTS.write().insert(slot, spot);
}

/// Where the reader last was in this context's app, if they have been.
///
/// `peek`: the caller is the navigation effect, which also writes here, and a
/// subscription would make that effect re-trigger itself.
pub fn recall(ctx: &[String], app: Option<&str>) -> Option<Spot> {
    SPOTS.peek().get(&key(ctx, app)).cloned()
}

/// The route to send the rail's entry for `app` to: where the reader was, or the
/// context root if they have not been there.
///
/// Subscribes on purpose. The spot is recorded in an effect, which runs after
/// the render that navigated, so a rail that only peeked would draw its links
/// from the previous page's memory and send a quick tap to the wrong place.
pub fn destination(ctx: &[String], app: Option<&str>) -> Vec<String> {
    SPOTS
        .read()
        .get(&key(ctx, app))
        .map(|spot| spot.segments.clone())
        .unwrap_or_else(|| ctx.to_vec())
}

/// Forget everything. Called on sign-out, so the next reader does not inherit
/// the last one's place in a context they may not even see.
pub fn clear() {
    SPOTS.write().clear();
}

#[cfg(test)]
mod tests {
    use super::key;

    #[test]
    fn an_app_view_cannot_collide_with_a_context_of_the_same_name() {
        // A context "a/speak" with no app, against context "a" with speak open.
        // Joined with "/" these would both be "a/speak"; they are not the same
        // place and must not share a slot.
        assert_ne!(
            key(&["a".into(), "speak".into()], None),
            key(&["a".into()], Some("speak")),
        );
    }

    #[test]
    fn the_same_place_keys_the_same() {
        assert_eq!(
            key(&["radikal".into(), "lm26".into()], Some("vote")),
            key(&["radikal".into(), "lm26".into()], Some("vote")),
        );
        assert_ne!(
            key(&["radikal".into(), "lm26".into()], Some("vote")),
            key(&["radikal".into(), "lm26".into()], None),
        );
    }
}
