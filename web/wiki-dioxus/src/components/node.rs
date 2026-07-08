use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;
use crate::i18n::t;

use super::loader::{mime_icon, visible_sorted};

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
                div { class: "avatar", "{icon}" }
                div {
                    h3 { class: "title-medium", "{name}" }
                    p { class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{title}"
                    }
                }
            }
            if !children.is_empty() {
                div { class: "list",
                    for child in children.iter() {
                        div { class: "list-item", key: "{child.id.0}",
                            div { class: "avatar small",
                                "{mime_icon(child.mime_id.as_deref().unwrap_or(\"\"))}"
                            }
                            div { class: "list-item-text",
                                div { class: "list-item-primary",
                                    "{child.name}"
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "card-content",
                    p { class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.noContent\")}"
                    }
                }
            }
        }
    }
}
