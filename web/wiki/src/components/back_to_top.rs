//! DESIGN (functional): scroll-driven navigation aids.
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

/// Whether the compact bottom dock is currently hidden (scrolled away). Read by
/// the shell so the dock slides out of the way while reading a long page and
/// returns on scroll up, reclaiming its two-row footprint on small screens.
static DOCK_HIDDEN: GlobalSignal<bool> = Signal::global(|| false);
/// Never hide the dock within this many pixels of the top of the page.
const DOCK_SHOW_ABOVE: f64 = 64.0;
/// Minimum scroll delta (px) before toggling the dock, to avoid jitter.
const DOCK_SCROLL_DELTA: f64 = 6.0;

/// Whether the software keyboard is up. Read by the dock's hide-on-scroll, which
/// must stand down while it is: the scroll iOS performs to reveal a focused field
/// is indistinguishable from the user scrolling down.
static KEYBOARD_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// Whether the viewport has come within [`NEAR_BOTTOM_PX`] of the end of the
/// page — what an endless list watches to fetch its next page.
static NEAR_BOTTOM: GlobalSignal<bool> = Signal::global(|| false);
/// Start the next page this far from the bottom, so it is usually there by the
/// time the reader arrives.
const NEAR_BOTTOM_PX: f64 = 800.0;

/// How often the scroll position is filed for the current URL, so a reload comes
/// back to it.
///
/// This used to say that only a reload reads a value this stale, because every
/// navigation files the exact one on its way out. That was the bug: by the time
/// the navigation reads it, the page has already been replaced and the browser
/// has clamped the scroll to the shorter document, so what it files is a zero.
/// This throttled trail, written while the reader was actually there, is what
/// survives; `nav_memory::worth_recording` is what stops the clamp overwriting
/// it.
const STASH_EVERY_MS: f64 = 250.0;

/// Whether the compact bottom dock should be hidden right now (hide-on-scroll).
pub fn dock_hidden() -> bool {
    DOCK_HIDDEN()
}

/// Whether the page is scrolled near its end. Reactive: reading it in a
/// component subscribes that component to the change. Only meaningful for a list
/// the WINDOW scrolls; one inside its own scroll container (a sheet) never moves
/// this and pages on a button instead.
/// Take the reader back to the top of the page.
///
/// Smooth via the `html { scroll-behavior: smooth }` rule, which reduced-motion
/// neutralises.
pub fn scroll_to_top() {
    if let Some(win) = web_sys::window() {
        win.scroll_to_with_x_and_y(0.0, 0.0);
    }
}

pub fn near_bottom() -> bool {
    NEAR_BOTTOM()
}

/// Attach the single window scroll listener that feeds all scroll-driven signals.
/// Leaked so it lives for the app's lifetime (the shell hosts these for the run).
fn install_listener() {
    let Some(win) = web_sys::window() else { return };
    // Last observed scroll position, for the dock's scroll-direction detection.
    let mut last_y = 0.0f64;
    // When this page's scroll was last filed, so a reload lands where the reader
    // is. Throttled: navigating away files the exact position anyway (see
    // Layout), so this only has to be recent, not current.
    let mut last_stash = 0.0f64;
    let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(move || {
        // The browser calls this, so the runtime has to be put back before
        // touching a signal (see `crate::runtime`).
        crate::runtime::enter(|| {
            let Some(w) = web_sys::window() else { return };
            let y = w.scroll_y().unwrap_or(0.0);

            let now = y > SHOW_AFTER;
            if now != *VISIBLE.peek() {
                *VISIBLE.write() = now;
            }

            // Hide-on-scroll for the compact bottom dock: hide when scrolling down
            // past a small threshold, reveal on scroll up or near the top of the page.
            //
            // Never while the keyboard is up. Focusing a field makes iOS scroll the
            // page to reveal it, which reads here as scrolling down — so tapping the
            // search box slid the dock, and the box with it, off the screen.
            let dy = y - last_y;
            let hidden_now = if *KEYBOARD_OPEN.peek() || y <= DOCK_SHOW_ABOVE {
                false
            } else if dy > DOCK_SCROLL_DELTA {
                true
            } else if dy < -DOCK_SCROLL_DELTA {
                false
            } else {
                *DOCK_HIDDEN.peek()
            };
            if hidden_now != *DOCK_HIDDEN.peek() {
                *DOCK_HIDDEN.write() = hidden_now;
            }
            last_y = y;

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

            // Endless lists fetch their next page from here rather than installing a
            // second scroll listener (this one already runs on every scroll event).
            let near = doc_h - (y + inner_h) < NEAR_BOTTOM_PX;
            if near != *NEAR_BOTTOM.peek() {
                *NEAR_BOTTOM.write() = near;
            }

            if let Some(url) = crate::nav_memory::current_url() {
                // Every event, exactly: this is what a navigation files, and a
                // throttled trail would have already forgotten the last moments
                // of the scroll by then.
                crate::nav_memory::note_scroll(&url, y);
                let now_ms = js_sys::Date::now();
                if now_ms - last_stash > STASH_EVERY_MS {
                    last_stash = now_ms;
                    crate::nav_memory::stash_scroll(&url, y);
                }
            }
        });
    });
    let _ = win.add_event_listener_with_callback("scroll", cb.as_ref().unchecked_ref());
    cb.forget();
}

/// Whether focus currently sits in something you type into.
fn focus_is_text_entry() -> bool {
    web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.active_element())
        .map(|e| {
            let tag = e.tag_name().to_lowercase();
            tag == "input"
                || tag == "textarea"
                || tag == "select"
                || e.get_attribute("contenteditable")
                    .is_some_and(|v| v != "false")
        })
        .unwrap_or(false)
}

/// Hold the dock open from the moment a field takes focus.
///
/// The visual viewport is too late to drive this. Focusing a field on iOS goes:
/// focus, then Safari scrolls the page to reveal the field, and only then does
/// the keyboard animate in and the visual viewport resize. The dock's
/// hide-on-scroll runs on that middle step, so by the time the resize said
/// "keyboard", the dock — and the search box inside it — had already slid away.
///
/// `focusin` fires first and synchronously, so the hide is already standing down
/// before that scroll arrives. The visual viewport still supplies the inset; this
/// only decides WHEN to stop hiding.
fn install_focus_listener() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let focus_in = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(|| {
        // The browser calls this, so the runtime has to be put back before
        // touching a signal (see `crate::runtime`).
        crate::runtime::enter(|| {
            if !focus_is_text_entry() {
                return;
            }
            if !*KEYBOARD_OPEN.peek() {
                *KEYBOARD_OPEN.write() = true;
            }
            if *DOCK_HIDDEN.peek() {
                *DOCK_HIDDEN.write() = false;
            }
        });
    });
    let focus_out = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(|| {
        // Deferred: focusout fires before focusin when moving between two
        // fields, and dropping the guard in that gap would let the scroll
        // between them hide the dock.
        if let Some(win) = web_sys::window() {
            let later = wasm_bindgen::closure::Closure::once_into_js(move || {
                // The browser calls this, so the runtime has to be put back before
                // touching a signal (see `crate::runtime`).
                crate::runtime::enter(|| {
                    if !focus_is_text_entry() && *KEYBOARD_OPEN.peek() {
                        *KEYBOARD_OPEN.write() = false;
                    }
                });
            });
            let _ =
                win.set_timeout_with_callback_and_timeout_and_arguments_0(later.unchecked_ref(), 0);
        }
    });
    let _ = doc.add_event_listener_with_callback("focusin", focus_in.as_ref().unchecked_ref());
    let _ = doc.add_event_listener_with_callback("focusout", focus_out.as_ref().unchecked_ref());
    focus_in.forget();
    focus_out.forget();
}

/// Publish the software keyboard's height as `--md-sys-keyboard-inset`.
///
/// iOS does not shrink the LAYOUT viewport when the keyboard opens, so anything
/// fixed to the bottom of the screen — the compact dock — is left sitting behind
/// it. The VISUAL viewport does shrink, and the gap between the two is the
/// keyboard. Reached through `Reflect` because `VisualViewport` is not in this
/// crate's web-sys features (the same way `pwa` probes for `PushManager`).
fn install_keyboard_listener() {
    let Some(win) = web_sys::window() else { return };
    let Ok(vv) = js_sys::Reflect::get(&win, &"visualViewport".into()) else {
        return;
    };
    if vv.is_undefined() || vv.is_null() {
        return;
    }
    let cb = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(update_keyboard_inset);
    let target: &web_sys::EventTarget = vv.unchecked_ref();
    // `resize` fires as the keyboard animates; `scroll` covers the page being
    // panned to keep the focused field visible, which moves the visual viewport
    // without resizing it.
    for event in ["resize", "scroll"] {
        let _ = target.add_event_listener_with_callback(event, cb.as_ref().unchecked_ref());
    }
    cb.forget();
    update_keyboard_inset();
}

fn update_keyboard_inset() {
    let Some(win) = web_sys::window() else { return };
    let Ok(vv) = js_sys::Reflect::get(&win, &"visualViewport".into()) else {
        return;
    };
    let number = |key: &str| {
        js_sys::Reflect::get(&vv, &key.into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    let layout = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let covered = layout - (number("height") + number("offsetTop"));
    // Pinch-zoom shrinks the visual viewport too, and it is not a keyboard —
    // lifting the dock then would move it for no reason. Scale is 1 whenever the
    // page is not zoomed, which is the only state a keyboard inset means
    // anything in.
    let zoomed = number("scale") > 1.01;
    // Below this it is rounding or rubber-banding, not a keyboard; treating it
    // as one would make the dock twitch on every scroll.
    let inset = if zoomed || covered < 24.0 {
        0.0
    } else {
        covered
    };
    if let Some(html) = win
        .document()
        .and_then(|d| d.document_element())
        .and_then(|e| e.dyn_into::<web_sys::HtmlElement>().ok())
    {
        let _ = html
            .style()
            .set_property("--md-sys-keyboard-inset", &format!("{inset}px"));
    }

    // Reached from a visualViewport listener, so the runtime has to be put back
    // for the signals below (see `crate::runtime`). Also called once directly at
    // install, from inside it, which `enter` leaves alone.
    crate::runtime::enter(|| {
        // Only ever RAISE the flag here. Lowering it is focusout's job: the inset
        // reads 0 for the whole moment between the tap and the keyboard animating
        // in, and clearing it then would reopen the very window this guards.
        if inset > 0.0 && !*KEYBOARD_OPEN.peek() {
            *KEYBOARD_OPEN.write() = true;
        }
        // Bring the dock back the moment the keyboard appears, rather than waiting
        // for a scroll to re-evaluate: it may well have been hidden before the field
        // was tapped, and the field is inside it.
        if inset > 0.0 && *DOCK_HIDDEN.peek() {
            *DOCK_HIDDEN.write() = false;
        }
    });
}

#[component]
pub fn BackToTop() -> Element {
    use_hook(install_listener);
    use_hook(install_keyboard_listener);
    use_hook(install_focus_listener);

    rsx! {
        button {
            class: if VISIBLE() { "back-to-top visible" } else { "back-to-top" },
            aria_label: t("common.backToTop"),
            title: t("common.backToTop"),
            onclick: move |_| scroll_to_top(),
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
