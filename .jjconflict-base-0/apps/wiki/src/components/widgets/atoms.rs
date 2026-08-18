//! Small display atoms: spinner, chip, badge, list item, supporting-pane layout,
//! carousel, and a proportion bar. All domain-free and slot/prop driven.

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
/// optional leading Material icon (the mime icon) plus a label. A real profile
/// picture in `avatar_url` (e.g. a linked Bluesky avatar) replaces the icon;
/// NHost's gravatar placeholders don't count (same rule as `loader::user_avatar`).
#[component]
pub fn Chip(
    icon: Option<String>,
    label: String,
    title: Option<String>,
    avatar_url: Option<String>,
) -> Element {
    let avatar = avatar_url.filter(|u| !u.is_empty() && !u.contains("gravatar"));
    rsx! {
        span { class: "chip", title: title.unwrap_or_default(),
            if let Some(url) = avatar {
                img { class: "chip-avatar", src: "{url}", alt: "", loading: "lazy" }
            } else if let Some(icon) = icon {
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

/// A Material 3 carousel: a horizontally scrollable, snapping strip of rounded
/// items, with the next item peeking to signal there is more. Pass the items as
/// `children`, each carrying the `m3-carousel-item` class.
#[component]
pub fn Carousel(#[props(default)] label: String, children: Element) -> Element {
    rsx! {
        div {
            class: "m3-carousel",
            role: "group",
            "aria-label": "{label}",
            // Keyboard-focusable so arrow keys scroll the strip.
            tabindex: "0",
            {children}
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
