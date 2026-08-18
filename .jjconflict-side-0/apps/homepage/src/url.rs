//! The page's initial `?query`, captured before the router can normalize it.
//!
//! Both `/cardioid` and `/screensaver/:name` encode their settings in the query
//! string so a configured toy is a shareable link. The Dioxus web router strips
//! the query off `window.location` while it works out the route, so a shared
//! link's settings are gone by the time the page component first runs. Grabbing
//! it in `main()` before `dioxus::launch` is the one place it is still there.
//!
//! It also puts back the `#` a URL cannot carry. A saver takes its pictures
//! from a hashtag with `?images=#art`, which is what the panel shows and what
//! anyone would type, but `#` starts the fragment: the browser reports the
//! query as `?images=` and hands the tag to `location.hash` instead. See
//! [`splice_fragment`].

use std::cell::RefCell;

thread_local! {
    static INITIAL_QUERY: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Snapshot the URL query string. Must be called from `main()`, before
/// `dioxus::launch`.
pub fn capture_url_query() {
    let Some(location) = web_sys::window().map(|w| w.location()) else {
        return;
    };
    let Ok(search) = location.search() else {
        return;
    };
    let hash = location.hash().unwrap_or_default();
    INITIAL_QUERY.with(|q| *q.borrow_mut() = splice_fragment(&search, &hash));
}

/// Put back a value the `#` split off.
///
/// A query ending in `=` and a non-empty fragment is the signature of a URL
/// like `?images=#art`: the value went missing exactly where the fragment
/// begins, so the two halves are one value that the browser cut in half. Any
/// parameters the user wrote after the tag were swept into the fragment too, so
/// the join stops at the first `&` and hands the rest back as query string.
///
/// A URL with a real fragment is left alone: `?fps=1#notes` does not end in
/// `=`, so nothing is spliced.
fn splice_fragment(search: &str, hash: &str) -> String {
    let fragment = hash.strip_prefix('#').unwrap_or(hash);
    if fragment.is_empty() || !search.ends_with('=') {
        return search.to_string();
    }
    match fragment.split_once('&') {
        Some((value, rest)) => format!("{search}%23{value}&{rest}"),
        None => format!("{search}%23{fragment}"),
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

#[cfg(test)]
mod tests {
    use super::splice_fragment;

    /// What a person types: `?images=#art`, which the browser splits.
    #[test]
    fn a_hashtag_is_put_back_together() {
        assert_eq!(splice_fragment("?images=", "#art"), "?images=%23art");
        assert_eq!(splice_fragment("?text=", "#poetry"), "?text=%23poetry");
    }

    /// Everything written after the tag was swept into the fragment with it.
    #[test]
    fn parameters_after_the_tag_come_back_as_parameters() {
        assert_eq!(
            splice_fragment("?images=", "#art&delay=20000"),
            "?images=%23art&delay=20000"
        );
        assert_eq!(
            splice_fragment("?delay=1&images=", "#art&fps=1"),
            "?delay=1&images=%23art&fps=1"
        );
    }

    /// A fragment that is a fragment is not a value.
    #[test]
    fn a_real_fragment_is_left_alone() {
        assert_eq!(splice_fragment("?fps=1", "#notes"), "?fps=1");
        assert_eq!(splice_fragment("", "#notes"), "");
    }

    /// Nothing to put back.
    #[test]
    fn an_ordinary_query_is_unchanged() {
        assert_eq!(splice_fragment("?images=%23art", ""), "?images=%23art");
        assert_eq!(splice_fragment("?images=", ""), "?images=");
        assert_eq!(splice_fragment("?images=", "#"), "?images=");
    }
}
