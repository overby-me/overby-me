//! Which box the page scrolls in.
//!
//! On compact it is the window, as it has always been: a phone browser hides its
//! address bar only when the WINDOW scrolls, and taking that away to gain a
//! rounded corner would be a bad trade on the screen with the least room.
//!
//! Everywhere else the content well scrolls its own content. The well is drawn
//! as a rounded panel floating in the chrome, and while the window scrolled it,
//! the panel went with it: its rounded top corners slid up under the bar within
//! a couple of hundred pixels and never came back until you returned to the top.
//! A frame that scrolls inside itself keeps its corners where they were drawn.
//!
//! Nothing here decides which it is. The stylesheet does, by making the well a
//! scroller or not, and this reads that back — so the breakpoint lives in one
//! place and cannot drift from the rule that produces it.

use wasm_bindgen::JsCast;

/// The element that scrolls, or `None` when that is the window.
pub fn host() -> Option<web_sys::Element> {
    let win = web_sys::window()?;
    let pane = win.document()?.query_selector(".content-pane").ok()??;
    let overflow = win
        .get_computed_style(&pane)
        .ok()??
        .get_property_value("overflow-y")
        .ok()?;
    matches!(overflow.as_str(), "auto" | "scroll").then_some(pane)
}

/// How far down the reader is.
pub fn scroll_top() -> f64 {
    match host() {
        Some(el) => el.scroll_top() as f64,
        None => web_sys::window()
            .and_then(|w| w.scroll_y().ok())
            .unwrap_or(0.0),
    }
}

/// The full height of what there is to scroll through.
pub fn scroll_height() -> f64 {
    match host() {
        Some(el) => el.scroll_height() as f64,
        None => web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.document_element())
            .map(|e| e.scroll_height() as f64)
            .unwrap_or(0.0),
    }
}

/// The height of the window onto it.
pub fn client_height() -> f64 {
    match host() {
        Some(el) => el.client_height() as f64,
        None => web_sys::window()
            .and_then(|w| w.inner_height().ok())
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0),
    }
}

/// Jump (or glide, per `scroll-behavior`) to a position.
pub fn scroll_to(y: f64) {
    match host() {
        Some(el) => el.scroll_to_with_x_and_y(0.0, y),
        None => {
            if let Some(win) = web_sys::window() {
                win.scroll_to_with_x_and_y(0.0, y);
            }
        }
    }
}

/// Listen for scrolling, wherever it happens.
///
/// On `document`, in the CAPTURE phase: a scroll event on an element does not
/// bubble, so a listener on the window would hear nothing once the well owns the
/// scroll — and capturing means this keeps working through a resize that moves
/// the scroll from one box to the other, with no listener to re-attach.
pub fn on_scroll(cb: &wasm_bindgen::closure::Closure<dyn FnMut()>) {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let _ = doc.add_event_listener_with_callback_and_bool(
        "scroll",
        cb.as_ref().unchecked_ref(),
        true,
    );
}
