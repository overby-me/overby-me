//! EXPERIMENT (functional): a back-to-top button. It appears once the page has
//! scrolled down past a threshold and smooth-scrolls to the top on click — a
//! navigation aid for long documents and folder listings. The whole document
//! scrolls (there is no inner scroll container), so it tracks `window.scrollY`,
//! mirroring the pull-to-refresh approach.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use crate::i18n::t;

/// Whether the button is currently shown (driven by the window scroll listener).
static VISIBLE: GlobalSignal<bool> = Signal::global(|| false);

/// Show the button once scrolled past this many pixels.
const SHOW_AFTER: f64 = 500.0;

#[component]
pub fn BackToTop() -> Element {
    // Install the window scroll listener once; the closure is leaked so it lives
    // for the app's lifetime (the shell hosts this component for the whole run).
    use_hook(|| {
        let Some(win) = web_sys::window() else { return };
        let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
            let y = web_sys::window()
                .and_then(|w| w.scroll_y().ok())
                .unwrap_or(0.0);
            let now = y > SHOW_AFTER;
            if now != *VISIBLE.peek() {
                *VISIBLE.write() = now;
            }
        });
        let _ = win.add_event_listener_with_callback("scroll", cb.as_ref().unchecked_ref());
        cb.forget();
    });

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
