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

/// M3 badge overlaying the top-trailing corner of a positioned parent: a small
/// dot (`count` `None`/`0`) or a large numeric pill (`count > 0`, capped 999+).
/// The parent must establish a positioning context (e.g. `position: relative`).
#[component]
pub fn Badge(count: Option<usize>) -> Element {
    match count {
        Some(n) if n > 0 => {
            let label = if n > 999 {
                "999+".to_string()
            } else {
                n.to_string()
            };
            rsx! {
                span { class: "md-badge", "aria-label": "{label}", "{label}" }
            }
        }
        _ => rsx! {
            span { class: "md-badge md-badge-dot" }
        },
    }
}

/// A reusable Material Design 3 colour-swatch row: a label, preset swatches, and
/// a trailing custom "any colour" chip ([`CustomColorSwatch`]) as the last circle.
/// Emits the chosen `#rrggbb` hex through `on_change`; `value` is the active
/// colour (it highlights a matching swatch). App-agnostic: labels are passed in.
#[component]
pub fn ColorPicker(
    label: String,
    value: String,
    swatches: Vec<String>,
    on_change: EventHandler<String>,
    custom_title: Option<String>,
) -> Element {
    rsx! {
        div { class: "color-picker",
            span { class: "color-picker-label", "{label}" }
            div { class: "color-swatches",
                for swatch in swatches.iter().cloned() {
                    {
                        let is_active = swatch.eq_ignore_ascii_case(&value);
                        let picked = swatch.clone();
                        rsx! {
                            button {
                                key: "{swatch}",
                                r#type: "button",
                                class: if is_active { "color-swatch active" } else { "color-swatch" },
                                style: "background-color: {swatch};",
                                title: "{swatch}",
                                onclick: move |_| on_change.call(picked.clone()),
                                if is_active {
                                    span { class: "material-icons", "check" }
                                }
                            }
                        }
                    }
                }
                // The last circle is the freeform custom picker.
                CustomColorSwatch { value: value.clone(), on_change, title: custom_title.clone() }
            }
        }
    }
}

/// The "any colour" chip: a rainbow swatch wrapping a native OS colour input.
/// Emits the chosen `#rrggbb` through `on_change`. Pairs with [`ColorPicker`].
#[component]
pub fn CustomColorSwatch(
    value: String,
    on_change: EventHandler<String>,
    title: Option<String>,
) -> Element {
    rsx! {
        label {
            class: "color-swatch color-swatch-custom",
            title: title.clone().unwrap_or_default(),
            span { class: "material-icons", "colorize" }
            input {
                r#type: "color",
                class: "color-input-native",
                value: "{value}",
                oninput: move |e| on_change.call(e.value()),
            }
        }
    }
}

/// An image that opens full-screen on click (#114). Click the overlay (or press
/// its close affordance) to dismiss. Keeps its own open/closed state.
#[component]
pub fn ZoomableImage(src: String, alt: String) -> Element {
    let mut zoomed = use_signal(|| false);
    let mut errored = use_signal(|| false);
    rsx! {
        if *errored.read() {
            // Error state: a broken-image placeholder instead of a dead <img>.
            div { class: "image-error", title: "{alt}",
                span { class: "material-icons", "broken_image" }
            }
        } else {
            img {
                src: "{src}",
                alt: "{alt}",
                // `.zoomable` fades the image in on mount (see CSS).
                class: "zoomable",
                onerror: move |_| errored.set(true),
                onclick: move |_| zoomed.set(true),
            }
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
