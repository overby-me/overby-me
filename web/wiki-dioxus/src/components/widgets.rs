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

/// A slot-based Material 3 list item: an optional leading element (avatar/icon),
/// a headline, an optional supporting line, and an optional trailing element
/// (badge/action). Carries the `.list-item` state layer; pass `selected` for the
/// M3 selected treatment. Wrap in a `Link` (or add an `onclick`) for navigation.
#[component]
pub fn ListItem(
    headline: String,
    leading: Option<Element>,
    supporting: Option<String>,
    trailing: Option<Element>,
    #[props(default)] selected: bool,
) -> Element {
    rsx! {
        div { class: if selected { "list-item selected" } else { "list-item" },
            if let Some(leading) = leading {
                {leading}
            }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{headline}" }
                if let Some(supporting) = supporting {
                    div { class: "list-item-secondary", "{supporting}" }
                }
            }
            if let Some(trailing) = trailing {
                div { class: "list-item-trailing", {trailing} }
            }
        }
    }
}

/// An adaptive M3 supporting-pane / list-detail scaffold: the `primary` pane and
/// a `supporting` pane stack into one column on compact/medium and sit side by
/// side (roughly 2:1) on large+ — a pure-CSS responsive grid off the window
/// width, so foldables/split-window resolve for free.
#[component]
pub fn SupportingPaneLayout(primary: Element, supporting: Element) -> Element {
    rsx! {
        div { class: "m3-supporting-pane",
            div { class: "m3-pane-primary", {primary} }
            div { class: "m3-pane-supporting", {supporting} }
        }
    }
}

/// An M3 single-select segmented button: a connected row of icon segments with
/// the selected one filled (secondary-container). Emits the chosen value through
/// `on_select`. `segments` is a list of `(value, material-icon)` pairs.
#[component]
pub fn SegmentedButton(
    segments: Vec<(String, String)>,
    selected: String,
    on_select: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "m3-segmented", role: "group",
            for (value , icon) in segments {
                {
                    let is_sel = value == selected;
                    let chosen = value.clone();
                    rsx! {
                        button {
                            key: "{value}",
                            r#type: "button",
                            class: if is_sel { "m3-segment selected state-layer" } else { "m3-segment state-layer" },
                            "aria-pressed": if is_sel { "true" } else { "false" },
                            onclick: move |_| on_select.call(chosen.clone()),
                            if is_sel {
                                span { class: "material-icons m3-segment-check", "check" }
                            }
                            span { class: "material-icons", "{icon}" }
                        }
                    }
                }
            }
        }
    }
}

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
    children: Element,
) -> Element {
    if !open {
        return rsx! {};
    }
    rsx! {
        div {
            class: "m3-dialog-scrim",
            role: "presentation",
            onclick: move |_| on_dismiss.call(()),
            div {
                class: "m3-dialog",
                role: "dialog",
                "aria-modal": "true",
                onclick: move |e| e.stop_propagation(),
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
