//! EXPERIMENT (functional): scroll-driven navigation aids.
//!
//! - [`BackToTop`]: a button that appears once the page has scrolled down past a
//!   threshold and smooth-scrolls to the top on click.
//! - [`ReadingProgress`]: a thin top bar showing how far through the page you have
//!   scrolled — orientation for long documents and listings.
//!
//! The whole document scrolls (there is no inner scroll container), so both track
//! `window.scrollY`, mirroring the pull-to-refresh approach. A single window
//! listener drives both signals.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use crate::i18n::t;

/// Whether the back-to-top button is shown.
static VISIBLE: GlobalSignal<bool> = Signal::global(|| false);
/// Scroll progress through the page, 0-100 (integer steps to bound re-renders).
static PROGRESS: GlobalSignal<i32> = Signal::global(|| 0);

/// Show the button once scrolled past this many pixels.
const SHOW_AFTER: f64 = 500.0;

/// Attach the single window scroll listener that feeds both signals. Leaked so it
/// lives for the app's lifetime (the shell hosts these for the whole run).
fn install_listener() {
    let Some(win) = web_sys::window() else { return };
    let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        let Some(w) = web_sys::window() else { return };
        let y = w.scroll_y().unwrap_or(0.0);

        let now = y > SHOW_AFTER;
        if now != *VISIBLE.peek() {
            *VISIBLE.write() = now;
        }

        let doc_h = w
            .document()
            .and_then(|d| d.document_element())
            .map(|e| e.scroll_height() as f64)
            .unwrap_or(0.0);
        let inner_h = w
            .inner_height()
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);
        let scrollable = (doc_h - inner_h).max(1.0);
        let pct = (y / scrollable * 100.0).clamp(0.0, 100.0).round() as i32;
        if pct != *PROGRESS.peek() {
            *PROGRESS.write() = pct;
        }
    });
    let _ = win.add_event_listener_with_callback("scroll", cb.as_ref().unchecked_ref());
    cb.forget();
}

#[component]
pub fn BackToTop() -> Element {
    use_hook(install_listener);

    rsx! {
        button {
            class: if VISIBLE() { "back-to-top visible" } else { "back-to-top" },
            aria_label: t("common.backToTop"),
            title: t("common.backToTop"),
            onclick: move |_| {
                if let Some(win) = web_sys::window() {
                    // Smooth via the html { scroll-behavior: smooth } rule (which
                    // reduced-motion neutralises).
                    win.scroll_to_with_x_and_y(0.0, 0.0);
                }
            },
            span { class: "material-icons", "arrow_upward" }
        }
    }
}

#[component]
pub fn ReadingProgress() -> Element {
    let pct = PROGRESS();
    rsx! {
        div { class: "reading-progress",
            div { class: "reading-progress-fill", style: "width: {pct}%;" }
        }
    }
}
