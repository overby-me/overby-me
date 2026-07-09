use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::session::use_session;

use super::loader::mime_icon;

/// PermApp — a read-only view of a context's permission rows (`?app=perm`):
/// which mime types each role may insert / select / delete. React left this
/// unimplemented; here it lists the configured permissions.
#[component]
pub fn PermApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    let perms = use_resource(use_reactive!(|(context_id, access_token)| async move {
        graphql::query_permissions(access_token.as_deref(), &context_id)
            .await
            .unwrap_or_default()
    }));
    let mut perms = perms.read().clone().unwrap_or_default();
    perms.sort_by(|a, b| {
        (a.role.as_str(), a.mime_id.as_deref().unwrap_or(""))
            .cmp(&(b.role.as_str(), b.mime_id.as_deref().unwrap_or("")))
    });

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "\u{1F510}" }
                h3 { class: "title-medium", "{node.name}" }
            }
            if perms.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                div { class: "list",
                    for perm in perms.iter() {
                        {
                            let mime = perm.mime_id.clone().unwrap_or_default();
                            let flags = [
                                ("+", perm.insert),
                                ("\u{1F441}", perm.select),
                                ("\u{1F5D1}", perm.delete),
                                ("\u{2713}", perm.active),
                            ];
                            rsx! {
                                div { class: "list-item", key: "{perm.id.0}",
                                    div { class: "avatar small", "{mime_icon(&mime)}" }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{mime} — {perm.role}" }
                                        div { class: "list-item-secondary",
                                            for (label , on) in flags {
                                                if on {
                                                    span { style: "margin-right: 8px;", "{label}" }
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
}
