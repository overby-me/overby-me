//! Expressive masked characters for a password field.

use dioxus::prelude::*;

/// Draws the masked characters of a password field so each one can arrive with
/// the motion-physics system's spring instead of simply existing.
///
/// The real `<input type="password">` stays exactly as it was: focus, autofill,
/// password managers, submit and screen readers all keep working, because this
/// only paints over it. The input's own glyphs are hidden by CSS that requires
/// this overlay to be present (`.pw-input:has(+ .pw-dots)`), so if this component
/// ever fails to render, the browser's own dots come back rather than the field
/// going blank — which is the failure mode that would matter on a login screen.
///
/// `aria-hidden`, because the input is what assistive technology should read; a
/// row of decorative circles saying nothing is worse than silence.
///
/// One caveat worth knowing: the caret is drawn after the last dot rather than
/// at the true insertion point, so moving the caret into the middle of a password
/// and typing puts the character in the right place while the bar stays at the
/// end. Everything still works; only the bar is in the wrong spot, in a case
/// almost nobody exercises.
#[component]
pub fn PasswordDots(len: usize) -> Element {
    rsx! {
        span { class: "pw-dots", aria_hidden: "true",
            for i in 0..len {
                // Keyed by position, so a new character mounts a new dot (and
                // animates), while the ones already there stay still.
                span { key: "{i}", class: "pw-dot", style: "--dot-i: {i}" }
            }
            span { class: "pw-caret" }
        }
    }
}
