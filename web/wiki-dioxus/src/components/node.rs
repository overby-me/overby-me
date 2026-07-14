use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;
use crate::i18n::t;

use super::loader::{mime_icon, node_icon_el, visible_sorted};

/// Generic node viewer — shows node info and its children
#[component]
pub fn NodeApp(node: NodeWithChildren, title: String) -> Element {
    let name = node.name.as_str();
    let mime_id = node.mime_id.as_deref().unwrap_or("");
    let icon = mime_icon(mime_id);
    let children = visible_sorted(&node.children);
    let children = &children;

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "{icon}" } }
                div {
                    h3 { class: "title-medium", "{name}" }
                    p { class: "body-medium",
                        class: "text-muted",
                        "{title}"
                    }
                }
            }
            if !children.is_empty() {
                div { class: "list",
                    for child in children.iter() {
                        super::widgets::ListItem {
                            key: "{child.id.0}",
                            headline: child.name.clone(),
                            leading: rsx! {
                                div { class: "avatar small",
                                    {node_icon_el(child.mime_id.as_deref().unwrap_or(""), child.data.as_ref().map(|d| &d.0))}
                                }
                            },
                        }
                    }
                }
            } else {
                // DESIGN: orb empty state for unknown / empty node types.
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "help_outline" }
                    }
                    p { class: "empty-state-body", "{t(\"common.noContent\")}" }
                }
            }
        }
    }
}
