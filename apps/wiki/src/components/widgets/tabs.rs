//! M3 Expressive primary tabs.

use dioxus::prelude::*;

/// A row of primary tabs with a sliding active indicator.
///
/// `tabs` is a list of `(label, material-icon)` pairs; the caller owns the
/// selected index and renders the panel itself, so a tab can hold anything.
///
/// The indicator is one element that springs between slots rather than a border
/// on each tab: tabs share the width equally, so its position is arithmetic
/// (`--tab-i` of `--tab-count`) and the motion comes free from a transform
/// transition. That is what makes it read as one thing moving rather than two
/// things blinking.
///
/// Keyboard behaviour follows the tabs pattern: arrows move between tabs, Home
/// and End jump to the ends.
#[component]
pub fn Tabs(
    tabs: Vec<(String, String)>,
    selected: usize,
    on_select: EventHandler<usize>,
) -> Element {
    let count = tabs.len().max(1);
    let selected = selected.min(count - 1);

    rsx! {
        div {
            class: "m3-tabs",
            role: "tablist",
            style: "--tab-count: {count}; --tab-i: {selected};",
            onkeydown: move |e| {
                let next = match e.key() {
                    Key::ArrowRight => (selected + 1) % count,
                    Key::ArrowLeft => (selected + count - 1) % count,
                    Key::Home => 0,
                    Key::End => count - 1,
                    _ => return,
                };
                e.prevent_default();
                on_select.call(next);
            },
            for (i , (label , icon)) in tabs.into_iter().enumerate() {
                button {
                    key: "{label}",
                    r#type: "button",
                    class: if i == selected { "m3-tab selected state-layer" } else { "m3-tab state-layer" },
                    role: "tab",
                    "aria-selected": if i == selected { "true" } else { "false" },
                    // Only the active tab is in the tab order; arrows move within
                    // the set, which is what the pattern asks for.
                    tabindex: if i == selected { "0" } else { "-1" },
                    onclick: move |_| on_select.call(i),
                    span { class: "material-icons m3-tab-icon", "{icon}" }
                    span { class: "m3-tab-label md-label-large", "{label}" }
                }
            }
            // Drawn last so it sits over the tab row's baseline without a stacking
            // context of its own.
            div { class: "m3-tab-indicator" }
        }
    }
}
