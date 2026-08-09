//! The page's initial `?query`, captured before the router can normalize it.
//!
//! Both `/cardioid` and `/screensaver/:name` encode their settings in the query
//! string so a configured toy is a shareable link. The Dioxus web router strips
//! the query off `window.location` while it works out the route, so a shared
//! link's settings are gone by the time the page component first runs. Grabbing
//! it in `main()` before `dioxus::launch` is the one place it is still there.

use std::cell::RefCell;

thread_local! {
    static INITIAL_QUERY: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Snapshot the URL query string. Must be called from `main()`, before
/// `dioxus::launch`.
pub fn capture_url_query() {
    if let Some(search) = web_sys::window().and_then(|w| w.location().search().ok()) {
        INITIAL_QUERY.with(|q| *q.borrow_mut() = search);
    }
}

/// The query captured at load, with its leading `?` if it had one.
pub fn captured_query() -> String {
    INITIAL_QUERY.with(|q| q.borrow().clone())
}

/// Mirror a query string into the address bar without remounting anything.
///
/// `replaceState` rather than a router navigation on purpose: a navigation
/// would tear down the canvas and restart the animation on every slider drag.
pub fn replace_query(query: &str) {
    let Some(history) = web_sys::window().and_then(|w| w.history().ok()) else {
        return;
    };
    let url = if query.is_empty() {
        web_sys::window()
            .and_then(|w| w.location().pathname().ok())
            .unwrap_or_else(|| "?".to_string())
    } else {
        format!("?{query}")
    };
    let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&url));
}
