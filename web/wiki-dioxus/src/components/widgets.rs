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

/// A small outlined chip matching the old wiki's MUI outlined-secondary Chip: an
/// optional leading Material icon (the mime icon) plus a label.
#[component]
pub fn Chip(icon: Option<String>, label: String, title: Option<String>) -> Element {
    rsx! {
        span { class: "chip", title: title.unwrap_or_default(),
            if let Some(icon) = icon {
                span { class: "material-icons", "{icon}" }
            }
            span { class: "chip-label", "{label}" }
        }
    }
}

/// An image that opens full-screen on click (#114). Click the overlay (or press
/// its close affordance) to dismiss. Keeps its own open/closed state.
#[component]
pub fn ZoomableImage(src: String, alt: String) -> Element {
    let mut zoomed = use_signal(|| false);
    rsx! {
        img {
            src: "{src}",
            alt: "{alt}",
            class: "zoomable",
            onclick: move |_| zoomed.set(true),
        }
        if *zoomed.read() {
            div {
                class: "image-lightbox",
                role: "dialog",
                onclick: move |_| zoomed.set(false),
                img { src: "{src}", alt: "{alt}" }
            }
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
