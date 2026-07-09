//! Small app-agnostic UI primitives that dioxus-components does not ship. Kept
//! wiki-free (no GraphQL / domain knowledge) so they could move to a shared
//! crate or be upstreamed. Screen components compose these plus the
//! dioxus-primitives in `components::ui`.

use dioxus::prelude::*;

/// A centred loading spinner overlay.
#[component]
pub fn Spinner() -> Element {
    rsx! {
        div { class: "spinner-overlay",
            div { class: "spinner" }
        }
    }
}

/// A small outlined chip: an optional leading icon/avatar and a label.
#[component]
pub fn Chip(icon: Option<String>, label: String, title: Option<String>) -> Element {
    rsx! {
        span { class: "chip", title: title.unwrap_or_default(),
            if let Some(icon) = icon {
                span { class: "avatar small secondary", "{icon}" }
            }
            "{label}"
        }
    }
}

/// A horizontal proportion bar (e.g. a poll option's share of the vote).
#[component]
pub fn Bar(fraction: f64) -> Element {
    let pct = (fraction.clamp(0.0, 1.0) * 100.0).round() as i64;
    rsx! {
        div { class: "vote-bar",
            div { class: "vote-bar-fill", style: "width: {pct}%;" }
        }
    }
}
