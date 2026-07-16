use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
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
                super::widgets::DataTable {
                    columns: vec![
                        t("perm.type"),
                        t("perm.role"),
                        t("perm.insert"),
                        t("perm.select"),
                        t("perm.delete"),
                        t("perm.active"),
                    ],
                    for perm in perms.iter() {
                        {
                            let mime = perm.mime_id.clone().unwrap_or_default();
                            rsx! {
                                tr { key: "{perm.id.0}",
                                    td {
                                        span { class: "m3-cell-icon",
                                            {icon_el(&mime)}
                                            span { "{mime}" }
                                        }
                                    }
                                    td { "{perm.role}" }
                                    td { {flag_cell(perm.insert)} }
                                    td { {flag_cell(perm.select)} }
                                    td { {flag_cell(perm.delete)} }
                                    td { {flag_cell(perm.active)} }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A boolean permission cell: a filled primary check when granted, else a muted
/// minus glyph. The glyph is exposed to assistive tech as "granted"/"denied" so
/// screen readers don't announce the raw icon ligature ("check"/"remove").
fn flag_cell(on: bool) -> Element {
    if on {
        rsx! {
            span {
                class: "material-icons m3-flag-on",
                role: "img",
                aria_label: t("perm.granted"),
                "check"
            }
        }
    } else {
        rsx! {
            span {
                class: "material-icons m3-flag-off",
                role: "img",
                aria_label: t("perm.denied"),
                "remove"
            }
        }
    }
}
