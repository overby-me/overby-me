use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::session::use_session;

use super::loader::icon_el;

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

    let perms = crate::use_data_resource!(|(context_id, access_token)| async move {
        graphql::query_permissions(access_token.as_deref(), &context_id)
            .await
            .unwrap_or_default()
    });
    let mut perms = perms.read().clone().unwrap_or_default();
    perms.sort_by(|a, b| {
        (a.role.as_str(), a.mime_id.as_deref().unwrap_or(""))
            .cmp(&(b.role.as_str(), b.mime_id.as_deref().unwrap_or("")))
    });

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "lock" } }
                h3 { class: "title-medium", "{node.name}" }
            }
            if perms.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                div { class: "list",
                    for perm in perms.iter() {
                        {
                            let mime = perm.mime_id.clone().unwrap_or_default();
                            let flags = [
                                ("add", perm.insert),
                                ("visibility", perm.select),
                                ("delete", perm.delete),
                                ("check", perm.active),
                            ];
                            rsx! {
                                div { class: "list-item", key: "{perm.id.0}",
                                    div { class: "avatar small", {icon_el(&mime)} }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{mime} — {perm.role}" }
                                        div { class: "list-item-secondary",
                                            for (label , on) in flags {
                                                if on {
                                                    span { class: "material-icons", style: "margin-right: 8px; font-size: 16px;", "{label}" }
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
