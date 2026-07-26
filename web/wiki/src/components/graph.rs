use dioxus::prelude::*;

use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;

use super::loader::{icon_el, mime_icon, visible_sorted};

/// GraphApp — a simple node-link visualisation of the context and its immediate
/// children (#68). The parent sits on top with an edge down to each child box;
/// child boxes link into the node.
#[component]
pub fn GraphApp(node: NodeWithChildren, path: Vec<String>) -> Element {
    let children = visible_sorted(&node.children);
    let n = children.len().max(1);

    // Layout constants (viewBox units).
    let col = 180.0;
    let width = (n as f64 * col).max(360.0);
    let height = 240.0;
    let parent_cx = width / 2.0;
    let parent_top = 24.0;
    let box_w = 150.0;
    let box_h = 44.0;
    let child_y = 168.0;
    let parent_name = node.name.clone();
    let parent_icon = mime_icon(&super::loader::node_icon_mime_id(
        node.mime_id.as_deref().unwrap_or(""),
        node.data.as_ref().map(|d| &d.0),
    ));

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("app/graph")} }
                h3 { class: "title-medium", "{node.name}" }
            }
            div { class: "card-content scroll-x",
                if children.is_empty() {
                    // DESIGN: orb empty state (consistent with the rest of the app).
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "hub" }
                        }
                        p { class: "empty-state-body", "{t(\"common.noContent\")}" }
                    }
                } else {
                    // DESIGN: frame the graph like the map's viewport.
                    div { class: "viewport-frame",
                    svg {
                        class: "graph-svg",
                        width: "100%",
                        view_box: "0 0 {width} {height}",
                        style: "min-width: {width}px;",
                        // The diagram is decorative-looking but conveys structure,
                        // so expose it as a single labelled image to assistive tech.
                        // (svg uses raw attribute names, not the typed HTML set.)
                        "role": "img",
                        "aria-label": format!("{}: {}", t("mime.graph"), node.name),
                        // Edges first so boxes draw on top.
                        for (i , _child) in children.iter().enumerate() {
                            line {
                                x1: "{parent_cx}",
                                y1: "{parent_top + box_h}",
                                x2: "{child_center(i, n, col)}",
                                y2: "{child_y}",
                                stroke: "var(--md-outline)",
                                stroke_width: "2",
                            }
                        }
                        // Parent node.
                        GraphBox {
                            x: parent_cx - box_w / 2.0,
                            y: parent_top,
                            w: box_w,
                            h: box_h,
                            icon: parent_icon.to_string(),
                            label: parent_name,
                            primary: true,
                        }
                        // Child nodes (each links into its route).
                        for (i , child) in children.iter().enumerate() {
                            {
                                let mut full = path.clone();
                                full.push(child.key.clone());
                                let icon = mime_icon(&super::loader::node_icon_mime_id(
                                    child.mime_id.as_deref().unwrap_or(""),
                                    child.data.as_ref().map(|d| &d.0),
                                ));
                                rsx! {
                                    Link {
                                        key: "{child.id.0}",
                                        to: Route::PathPage { segments: full, app: None },
                                        GraphBox {
                                            x: child_center(i, n, col) - box_w / 2.0,
                                            y: child_y,
                                            w: box_w,
                                            h: box_h,
                                            icon: icon.to_string(),
                                            label: child.name.clone(),
                                            primary: false,
                                        }
                                    }
                                }
                            }
                        }
                    }
                    }
                }
            }
        }
    }
}

/// Horizontal centre of child `i` of `n` across a row of `col`-wide columns.
fn child_center(i: usize, n: usize, col: f64) -> f64 {
    let width = (n as f64 * col).max(360.0);
    (i as f64 + 0.5) * (width / n as f64)
}

/// A rounded box with an icon glyph and a (truncated) label — one graph node.
#[component]
fn GraphBox(x: f64, y: f64, w: f64, h: f64, icon: String, label: String, primary: bool) -> Element {
    let fill = if primary {
        "var(--md-primary-container)"
    } else {
        "var(--md-surface-variant)"
    };
    let shown = truncate(&label, 16);
    rsx! {
        g {
            rect {
                x: "{x}",
                y: "{y}",
                width: "{w}",
                height: "{h}",
                rx: "8",
                fill: "{fill}",
                stroke: "var(--md-outline)",
                stroke_width: "1.5",
            }
            text {
                x: "{x + 12.0}",
                y: "{y + h / 2.0 + 6.0}",
                font_family: "Material Icons",
                font_size: "18",
                fill: "var(--md-on-surface)",
                "{icon}"
            }
            text {
                x: "{x + 34.0}",
                y: "{y + h / 2.0 + 5.0}",
                font_size: "13",
                fill: "var(--md-on-surface)",
                "{shown}"
            }
        }
    }
}

/// Truncate to `max` chars with an ellipsis (char-safe).
fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max.saturating_sub(1)).collect();
        format!("{cut}\u{2026}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn child_centres_are_evenly_spaced() {
        // Two children across a 360-wide canvas → quarter and three-quarter.
        assert_eq!(child_center(0, 2, 180.0), 90.0);
        assert_eq!(child_center(1, 2, 180.0), 270.0);
    }

    #[test]
    fn truncate_is_char_safe() {
        assert_eq!(truncate("short", 16), "short");
        assert_eq!(truncate("abcdefghijklmnopqrstuvwxyz", 5), "abcd\u{2026}");
    }
}
