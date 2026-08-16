//! Where the reader was: which page, and how far down it.
//!
//! Two separate questions, kept apart because they have different keys.
//!
//! **Which page**, per context and app. The app rail is a CONTEXT rail: every
//! entry targets the context root, and it has to, because an app renders the
//! node it sits on and a speaker list hanging off a document is not the group's
//! speaker list. That is right going TO an app and wrong coming BACK. A delegate
//! three levels into an agenda who checked the speaker list returned to the top
//! of the group, having lost the page they were reading. Held in memory only:
//! this is "where was I a moment ago", not a preference.
//!
//! **How far down**, per URL, in `sessionStorage`. Not `history.state`, which
//! the router owns and which back/forward would break if we clobbered it. One
//! key per URL covers all three ways of coming back to a page at once: the rail,
//! browser back/forward, and a reload, since sessionStorage outlives a reload
//! and dies with the tab.

use std::collections::HashMap;

use dioxus::prelude::*;

static SPOTS: GlobalSignal<HashMap<String, Vec<String>>> = Signal::global(HashMap::new);

const SCROLL_PREFIX: &str = "wiki_scroll:";

/// One key per context and app view.
///
/// `\u{1}` rather than `/`, so a context named after an app cannot collide with
/// one that has that app open.
fn key(ctx: &[String], app: Option<&str>) -> String {
    format!("{}\u{1}{}", ctx.join("/"), app.unwrap_or(""))
}

/// Record which page the reader was on in this context's app.
///
/// Only routes at or below the context root are worth recording; anything else
/// is a different context and belongs to its own key.
pub fn remember(ctx: &[String], app: Option<&str>, segments: &[String]) {
    if ctx.is_empty() || !segments.starts_with(ctx) {
        return;
    }
    let slot = key(ctx, app);
    // Only write a change. The rail subscribes to this, and re-rendering it to
    // store the value it already held would be a render for nothing.
    if SPOTS.peek().get(&slot).is_some_and(|held| held == segments) {
        return;
    }
    SPOTS.write().insert(slot, segments.to_vec());
}

/// The route to send the rail's entry for `app` to: where the reader was, or the
/// context root if they have not been there.
///
/// Subscribes on purpose. The page is recorded in an effect, which runs after
/// the render that navigated, so a rail that only peeked would draw its links
/// from the previous page's memory and send a quick tap to the wrong place.
pub fn destination(ctx: &[String], app: Option<&str>) -> Vec<String> {
    SPOTS
        .read()
        .get(&key(ctx, app))
        .cloned()
        .unwrap_or_else(|| ctx.to_vec())
}

fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok().flatten()
}

thread_local! {
    /// Where the reader last was on each URL, updated on EVERY scroll event.
    ///
    /// The `sessionStorage` trail is throttled, and a throttle with no trailing
    /// edge never records the position a reader comes to REST at: the last
    /// sample is whichever one happened to land on a window boundary, and
    /// everything after it is dropped. That is what made returning land in the
    /// wrong place even once the clamp stopped erasing the value outright.
    ///
    /// This is a plain map write, so it can afford to run on every event and be
    /// exact. In memory only: a reload has the throttled trail, which is what it
    /// is for.
    static LAST_SEEN: std::cell::RefCell<HashMap<String, f64>> =
        std::cell::RefCell::new(HashMap::new());
}

/// Note where the reader is now, for [`last_scroll`]. Cheap enough for every
/// scroll event.
pub fn note_scroll(url: &str, y: f64) {
    LAST_SEEN.with(|m| m.borrow_mut().insert(url.to_string(), y));
}

/// Where the reader last was on `url`, exactly, if they have been there since
/// the page loaded.
pub fn last_scroll(url: &str) -> Option<f64> {
    LAST_SEEN.with(|m| m.borrow().get(url).copied())
}

thread_local! {
    /// While a restore is settling, the stored trail is not to be believed.
    static SETTLING_UNTIL: std::cell::Cell<f64> = const { std::cell::Cell::new(0.0) };
}

/// Hold the stored trail still for `ms` while a restore lands.
///
/// Putting the reader back produces scroll events of its own, and the throttled
/// writer records whichever of them falls on a window boundary, which is an
/// early frame rather than the destination. The position then looked like three
/// pixels down for anyone who RELOADED just after returning to a page. The
/// in-memory note is unaffected, so navigation was always right; this is only
/// about what survives a reload.
pub fn hold_trail(ms: f64) {
    SETTLING_UNTIL.with(|c| c.set(js_sys::Date::now() + ms));
}

fn settling() -> bool {
    SETTLING_UNTIL.with(|c| js_sys::Date::now() < c.get())
}

/// Whether `y` is worth recording as where the reader left a page.
///
/// A zero the BROWSER produced is not a decision the reader made. When the page
/// under them shrinks, which is what switching to an app view does, the browser
/// pulls the scroll down to the new maximum and fires a scroll event. Both the
/// scroll listener and the navigation effect then file that as "they were at the
/// top", and the position is destroyed by the act of leaving the page it belongs
/// to. That is why coming back to a context landed at the top: the memory had
/// been overwritten a frame earlier, by leaving.
///
/// So a zero counts only when the page could still hold what is remembered. If
/// it cannot, the page has already shrunk and the zero is the clamp talking.
/// Anything above the top is always the reader.
pub(crate) fn worth_recording(y: f64, remembered: Option<f64>, reachable: f64) -> bool {
    if y > 1.0 {
        return true;
    }
    match remembered {
        Some(prev) if prev > 1.0 => reachable >= prev,
        _ => true,
    }
}

/// How far down this page can be scrolled right now.
fn reachable_scroll() -> f64 {
    (crate::scroll_host::scroll_height() - crate::scroll_host::client_height()).max(0.0)
}

/// Note how far down `url` was left.
///
/// Rounded to whole pixels: this is a place to return to, not a measurement,
/// and a fractional tail would only churn the store.
///
/// Declines to record a clamp (see [`worth_recording`]).
pub fn stash_scroll(url: &str, y: f64) {
    let Some(store) = session_storage() else {
        return;
    };
    if settling() || !worth_recording(y, stashed_scroll(url), reachable_scroll()) {
        return;
    }
    let _ = store.set_item(&format!("{SCROLL_PREFIX}{url}"), &y.round().to_string());
}

/// How far down `url` was left, if the reader has been there this session.
pub fn stashed_scroll(url: &str) -> Option<f64> {
    session_storage()?
        .get_item(&format!("{SCROLL_PREFIX}{url}"))
        .ok()
        .flatten()?
        .parse::<f64>()
        .ok()
}

/// Where the reader was before the sign-in screens.
///
/// Signing in is a detour, not a destination: someone who taps "log in" while
/// reading an agenda wants the agenda back, not the front page. Recorded on
/// every page that is not one of the auth screens, so it covers all the ways
/// into them at once -- the menu, a link, and the app sending a signed-out
/// reader there itself.
///
/// In `sessionStorage`, like the scroll: the detour can include a password
/// reset, a mail and a reload, and it should die with the tab.
const WAY_BACK: &str = "wiki_way_back";

/// Note that this is where the reader is, in case they sign in from here.
pub fn note_way_back(url: &str) {
    let Some(store) = session_storage() else {
        return;
    };
    let _ = store.set_item(WAY_BACK, url);
}

/// Where to put the reader after signing in, TAKEN: it is worth one journey.
/// Left behind, it would send them back there again the next time they signed
/// in from somewhere else in the same tab.
pub fn way_back() -> Option<String> {
    let store = session_storage()?;
    let url = store.get_item(WAY_BACK).ok().flatten()?;
    let _ = store.remove_item(WAY_BACK);
    (!url.is_empty()).then_some(url)
}

/// The current URL as the router writes it: path plus query, which is the key
/// the scroll is filed under.
pub fn current_url() -> Option<String> {
    let loc = web_sys::window()?.location();
    Some(format!("{}{}", loc.pathname().ok()?, loc.search().ok()?))
}

/// Forget everything. Called on sign-out, so the next reader does not inherit
/// the last one's places, which may be in a context they cannot even see --
/// the way back after signing in included, for exactly that reason.
pub fn clear() {
    SPOTS.write().clear();
    let Some(store) = session_storage() else {
        return;
    };
    // Collect first: removing while walking the store would shift the indices
    // under the walk and skip half the keys.
    let _ = store.remove_item(WAY_BACK);
    let mut ours = Vec::new();
    for i in 0..store.length().unwrap_or(0) {
        if let Ok(Some(k)) = store.key(i) {
            if k.starts_with(SCROLL_PREFIX) {
                ours.push(k);
            }
        }
    }
    for k in ours {
        let _ = store.remove_item(&k);
    }
}

#[cfg(test)]
mod tests {
    use super::key;

    /// A reader who scrolls back to the top of a page that is still tall means
    /// it: record the zero, or returning would send them back down.
    #[test]
    fn a_reader_may_choose_the_top() {
        assert!(super::worth_recording(0.0, Some(1200.0), 4000.0));
        assert!(super::worth_recording(0.0, None, 4000.0));
        assert!(super::worth_recording(0.0, Some(0.0), 0.0));
    }

    /// The zero the browser produces when the page shrinks under the reader is
    /// not a choice, and must not erase where they actually were. This is what
    /// made coming back to a context land at the top.
    #[test]
    fn a_clamp_does_not_erase_the_place() {
        // Remembered 1200 down, but the page can now only reach 300.
        assert!(!super::worth_recording(0.0, Some(1200.0), 300.0));
        // Exactly reachable still counts: nothing was taken away.
        assert!(super::worth_recording(0.0, Some(1200.0), 1200.0));
    }

    /// Any real position is the reader, whatever the page can hold.
    #[test]
    fn a_position_below_the_top_is_always_recorded() {
        assert!(super::worth_recording(900.0, Some(1200.0), 300.0));
        assert!(super::worth_recording(2.0, Some(1200.0), 0.0));
    }

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
