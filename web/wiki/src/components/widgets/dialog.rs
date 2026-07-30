//! A reusable Material Design 3 basic dialog.

use dioxus::prelude::*;

use super::focus::{active_html_element, dialog_dismiss, trap_tab_focus};

/// A reusable Material Design 3 basic dialog: a scrim over a surface-container-
/// high card (extra-large corners, level-3 elevation, spring rise-in) with an
/// optional leading icon, a headline, the passed `children` as content, and an
/// `actions` row. Dismisses on scrim click. Retires the ad-hoc
/// `.modal-backdrop`/`.modal-card` markup across screens.
#[component]
pub fn Dialog(
    open: bool,
    on_dismiss: EventHandler<()>,
    headline: String,
    actions: Element,
    icon: Option<String>,
    /// This dialog is a FORM rather than a question. On a phone it then takes
    /// the whole screen, which is what M3 asks for and what the content wants:
    /// a card floating in a scrim, with a keyboard over its lower half and its
    /// buttons somewhere under that, is the worst place to fill in three fields.
    /// Confirmations stay as cards at every size — they are one sentence, and a
    /// full screen for "delete this?" reads as a page you have navigated to.
    #[props(default)]
    form: bool,
    children: Element,
) -> Element {
    // Remember the trigger so focus returns to it on dismiss (a11y). Declared
    // before the early return so the hook order stays stable.
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);
    if !open {
        return rsx! {};
    }
    rsx! {
        div {
            class: "m3-dialog-scrim",
            role: "presentation",
            onclick: move |_| dialog_dismiss(on_dismiss, return_focus),
            div {
                class: if form { "m3-dialog m3-dialog-form" } else { "m3-dialog" },
                role: "dialog",
                "aria-modal": "true",
                // Name the dialog by its headline so screen readers announce it
                // (the headline h2 is rendered below).
                "aria-label": "{headline}",
                tabindex: "-1",
                onkeydown: move |e| {
                    match e.key() {
                        Key::Escape => dialog_dismiss(on_dismiss, return_focus),
                        Key::Tab if trap_tab_focus(".m3-dialog", e.modifiers().shift()) => {
                            e.prevent_default();
                        }
                        _ => {}
                    }
                },
                onclick: move |e| e.stop_propagation(),
                // Remember the trigger, then pull focus into the dialog on open (it
                // mounts only when open).
                div {
                    class: "sheet-focus-sentinel",
                    tabindex: "-1",
                    onmounted: move |e| {
                        return_focus.set(active_html_element());
                        spawn(async move {
                            let _ = e.set_focus(true).await;
                        });
                    },
                }
                if let Some(icon) = icon {
                    span { class: "m3-dialog-icon material-icons", "{icon}" }
                }
                h2 { class: "m3-dialog-headline", "{headline}" }
                div { class: "m3-dialog-content", {children} }
                div { class: "m3-dialog-actions", {actions} }
            }
        }
    }
}
