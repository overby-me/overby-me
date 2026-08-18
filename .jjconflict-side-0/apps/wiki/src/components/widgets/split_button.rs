//! An M3 Expressive split button: one action, plus a menu of related ones.
//!
//! "Split buttons are made of a common button and a menu icon button", and the
//! menu button "spins and changes shape when activated". The pairing is for a
//! frequent action that has less frequent relatives — not for two equal actions,
//! which is a button group.

use dioxus::prelude::*;

/// A filled action with a connected menu button beside it.
///
/// `children` are the menu's items; give each the `split-menu-item` class (or
/// use whatever markup suits — the menu is a plain surface). The menu closes on
/// Escape, on losing focus, and after a click inside it, so an item does not
/// have to close it itself.
#[component]
pub fn SplitButton(
    label: String,
    icon: String,
    disabled: bool,
    menu_label: String,
    on_click: EventHandler<()>,
    children: Element,
) -> Element {
    let mut open = use_signal(|| false);
    rsx! {
        div { class: "m3-split",
            button {
                class: "btn btn-primary m3-split-action",
                r#type: "button",
                disabled,
                onclick: move |_| on_click.call(()),
                span { class: "material-icons", "{icon}" }
                " {label}"
            }
            button {
                class: if open() { "btn btn-primary m3-split-menu open" } else { "btn btn-primary m3-split-menu" },
                r#type: "button",
                disabled,
                "aria-haspopup": "menu",
                "aria-expanded": if open() { "true" } else { "false" },
                aria_label: "{menu_label}",
                onclick: move |_| {
                    let now = open();
                    open.set(!now);
                },
                span { class: "material-icons", "expand_more" }
            }
            if open() {
                // A click anywhere else closes it, including a click on the page
                // behind — without this the menu would outlive the decision.
                div {
                    class: "m3-split-scrim",
                    onclick: move |_| open.set(false),
                }
                div {
                    class: "m3-split-sheet",
                    role: "menu",
                    onkeydown: move |e| {
                        if e.key() == Key::Escape {
                            open.set(false);
                        }
                    },
                    onclick: move |_| open.set(false),
                    {children}
                }
            }
        }
    }
}
