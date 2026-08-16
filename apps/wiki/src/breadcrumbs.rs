//! The trail of what the reader did, kept for whatever has to explain a failure.
//!
//! Clicks, field edits, form submissions and route changes, as identities rather
//! than values: `click button.btn-icon "Slet"`, `change input[name=title]`,
//! `navigate /radikal_ungdom/landsmøde_2026`. A password field is recorded as
//! having been typed in and nothing more, and no input value is ever kept.
//!
//! This is deliberately OUTSIDE the `remote-logging` feature. It used to live in
//! [`crate::logging`], which is compiled out unless a Better Stack token was
//! present at build time -- so a build without one collected no trail at all,
//! and the crash report it filed said what broke without a word about what the
//! reader was doing when it did. A crash report should stand on its own: the
//! wiki's own feedback list is the copy everyone can read, and Better Stack is
//! the copy that pages someone.

use std::cell::RefCell;
use std::collections::VecDeque;

use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

/// How many steps back the trail goes. Fifty is a few minutes of ordinary use
/// and about a kilobyte, which a crash report can carry.
const MAX_BREADCRUMBS: usize = 50;

thread_local! {
    static BREADCRUMBS: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
    /// The last path recorded, to dedupe the router's initial + repeated route
    /// effects.
    static LAST_NAV: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Add a step to the trail, dropping the oldest when it is full.
pub fn record(text: String) {
    BREADCRUMBS.with(|b| {
        let mut b = b.borrow_mut();
        if b.len() >= MAX_BREADCRUMBS {
            b.pop_front();
        }
        b.push_back(text);
    });
}

/// The trail so far, oldest first.
pub fn trail() -> Vec<String> {
    BREADCRUMBS.with(|b| b.borrow().iter().cloned().collect())
}

/// The last few steps as lines, newest LAST, within a character budget.
///
/// For a crash report, which travels in a URL: the tail is what matters, so the
/// budget is spent from the end backwards.
pub fn tail(budget: usize) -> String {
    let mut out: Vec<String> = Vec::new();
    let mut used = 0usize;
    for step in trail().into_iter().rev() {
        used += step.chars().count() + 1;
        if used > budget {
            break;
        }
        out.push(step);
    }
    out.reverse();
    out.join("\n")
}

/// Record a client-side navigation. Called by the router on each route change
/// (see `layout::Layout`), so a trail shows the pages the reader moved through
/// rather than only the one they were on when it broke.
pub fn record_navigation(path: &str) {
    let changed = LAST_NAV.with(|l| {
        let mut l = l.borrow_mut();
        if *l == path {
            false
        } else {
            *l = path.to_string();
            true
        }
    });
    if changed {
        record(format!("navigate {path}"));
    }
}

/// A compact description of an element for a breadcrumb: `tag#id.class "label"`,
/// resolved to the nearest interactive ancestor. Never includes input values.
///
/// Anything under `[data-private]` is described by shape alone. The label here
/// comes from the element's own text, and for a control whose text IS the answer
/// -- a ballot option -- that turns the trail into a record of how someone voted,
/// filed under their name and user id. A secret ballot goes to lengths to keep
/// the vote off the vote row; it must not arrive in a log instead.
fn describe(el: &web_sys::Element) -> String {
    let target = el
        .closest("button, a, input, textarea, select, [role=button], .btn, .btn-icon, .list-item, .folder-item")
        .ok()
        .flatten()
        .unwrap_or_else(|| el.clone());
    let tag = target.tag_name().to_lowercase();
    // Asked of the CLICKED element, not of `target`: `target` is an ancestor, so
    // a marker on a container between them would be missed by the wider one.
    if el.closest("[data-private]").ok().flatten().is_some() {
        let id = target.id();
        let id_part = if id.is_empty() {
            String::new()
        } else {
            format!("#{id}")
        };
        let class_part = target
            .get_attribute("class")
            .unwrap_or_default()
            .split_whitespace()
            .next()
            .map(|c| format!(".{c}"))
            .unwrap_or_default();
        return format!("{tag}{id_part}{class_part} [private]");
    }
    let id = target.id();
    let id_part = if id.is_empty() {
        String::new()
    } else {
        format!("#{id}")
    };
    let class = target.get_attribute("class").unwrap_or_default();
    let class_part = class
        .split_whitespace()
        .next()
        .map(|c| format!(".{c}"))
        .unwrap_or_default();
    // Field inputs: identity only, never the value; hide password fields.
    if matches!(tag.as_str(), "input" | "textarea" | "select") {
        if target.get_attribute("type").as_deref() == Some("password") {
            return format!("{tag}{id_part} [password]");
        }
        let name = target
            .get_attribute("name")
            .map(|n| format!("[name={n}]"))
            .unwrap_or_default();
        return format!("{tag}{id_part}{name}");
    }
    let label = target
        .get_attribute("aria-label")
        .or_else(|| target.get_attribute("title"))
        .or_else(|| {
            let t = target.text_content().unwrap_or_default();
            let t = t.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.chars().take(40).collect())
            }
        })
        .map(|l| format!(" \"{}\"", l.replace('"', "'")))
        .unwrap_or_default();
    // For a link, also record where it points (the destination is the relevant
    // context for a click that navigates). A same-origin href is trimmed to its
    // path so the trail reads as in-app routes.
    let href = if tag == "a" {
        target
            .get_attribute("href")
            .filter(|h| !h.is_empty())
            .map(|h| format!(" -> {h}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!("{tag}{id_part}{class_part}{label}{href}")
}

pub(crate) fn target_element(ev: &web_sys::Event) -> Option<web_sys::Element> {
    ev.target()?.dyn_into::<web_sys::Element>().ok()
}

/// Bubble-phase, for the window's own error hooks: only [`crate::logging`] wants
/// those, and only in a build that ships them.
#[cfg(feature = "remote-logging")]
pub(crate) fn add_listener<F: FnMut(&web_sys::Event) + 'static>(
    target: &web_sys::EventTarget,
    event: &str,
    mut f: F,
) {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| f(&ev));
    let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    // Leak the closure so the listener lives for the app's lifetime.
    closure.forget();
}

/// The same, but in the CAPTURE phase: down the tree to the target rather than
/// up from it.
///
/// This is what a trail needs. A press that panics traps the wasm inside the
/// element's OWN handler, and a listener waiting on the way back up never runs
/// -- so the one click a crash report most wants to name was the one click never
/// recorded. Going down, it is already written before the handler runs.
fn add_capturing<F: FnMut(&web_sys::Event) + 'static>(
    target: &web_sys::EventTarget,
    event: &str,
    mut f: F,
) {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| f(&ev));
    let _ = target.add_event_listener_with_callback_and_bool(
        event,
        closure.as_ref().unchecked_ref(),
        true,
    );
    closure.forget();
}

/// Record clicks, field edits and form submissions as breadcrumbs (no values).
pub fn watch() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let et: &web_sys::EventTarget = doc.as_ref();
    add_capturing(et, "click", |ev| {
        if let Some(el) = target_element(ev) {
            record(format!("click {}", describe(&el)));
        }
    });
    add_capturing(et, "change", |ev| {
        if let Some(el) = target_element(ev) {
            record(format!("change {}", describe(&el)));
        }
    });
    add_capturing(et, "submit", |ev| {
        if let Some(el) = target_element(ev) {
            record(format!("submit {}", describe(&el)));
        }
    });
}

#[cfg(test)]
mod tests {
    /// The tail is the LAST steps, in order, inside its budget.
    ///
    /// A crash report travels in a URL, so the trail has a character budget; what
    /// matters is what happened just before it broke, so the budget is spent from
    /// the end backwards and the steps still read oldest-first.
    #[test]
    fn the_trail_keeps_its_end() {
        for i in 0..8 {
            super::record(format!("click button-{i}"));
        }
        let all = super::tail(10_000);
        assert!(all.starts_with("click button-0"), "oldest first: {all}");
        assert!(all.ends_with("click button-7"), "newest last: {all}");

        // Twenty characters holds "click button-7" (14) and no more: the next
        // one back would take it past the budget.
        let end = super::tail(20);
        assert_eq!(end, "click button-7");

        // And it never grows past its bound, however long the session runs.
        for i in 0..200 {
            super::record(format!("navigate /page-{i}"));
        }
        assert_eq!(super::trail().len(), super::MAX_BREADCRUMBS);
        assert!(super::trail()
            .last()
            .is_some_and(|l| l.ends_with("/page-199")));
    }
}
