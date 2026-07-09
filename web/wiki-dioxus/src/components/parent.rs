use dioxus::prelude::*;

use crate::graphql::{self, ContextNodeFields};
use crate::i18n::t;
use crate::session::use_session;

use super::loader::icon_el;

/// ParentApp — the "Missing parent" admin view (#149): lists nodes whose
/// `parentId` is null, i.e. that have lost their parent. The single legitimate
/// root (the home node) is filtered out, so anything shown is an orphan worth
/// investigating. Reachable via `?app=parent`.
#[component]
pub fn ParentApp() -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();

    let orphans = use_resource(move || {
        let token = access_token.clone();
        async move {
            graphql::query_orphans(token.as_deref())
                .await
                .map(|nodes| nodes.into_iter().filter(is_orphan).collect::<Vec<_>>())
        }
    });

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("app/parent")} }
                div {
                    h3 { class: "title-medium", "{t(\"parent.title\")}" }
                    p {
                        class: "body-small",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"parent.description\")}"
                    }
                }
            }
            match &*orphans.read() {
                Some(Ok(list)) if !list.is_empty() => rsx! {
                    div { class: "list",
                        for node in list.iter() {
                            div { class: "list-item", key: "{node.id.0}",
                                div { class: "avatar small", {icon_el(node.mime_id.as_deref().unwrap_or(""))} }
                                div { class: "list-item-text",
                                    div { class: "list-item-primary", "{node.name}" }
                                    div {
                                        class: "list-item-secondary",
                                        style: "color: var(--md-on-surface-variant);",
                                        "{node.mime_id.clone().unwrap_or_default()} · {node.id.0}"
                                    }
                                }
                            }
                        }
                    }
                },
                Some(Ok(_)) => rsx! {
                    div { class: "card-content",
                        p {
                            class: "body-medium",
                            style: "color: var(--md-on-surface-variant);",
                            span { class: "material-icons", style: "vertical-align: middle;", "check_circle" }
                            " {t(\"parent.none\")}"
                        }
                    }
                },
                Some(Err(e)) => rsx! {
                    div { class: "card-content",
                        p { class: "body-medium", "{t(\"error.somethingWentWrong\")}" }
                        pre { class: "error-fallback", "{e}" }
                    }
                },
                None => rsx! { super::widgets::Spinner {} },
            }
        }
    }
}

/// Whether a parent-less node is a genuine orphan rather than the legitimate
/// root (the home node, conventionally keyed `root`).
fn is_orphan(node: &ContextNodeFields) -> bool {
    node.mime_id.as_deref() != Some("wiki/home") && node.key != "root"
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::Uuid;

    fn node(key: &str, mime: &str) -> ContextNodeFields {
        ContextNodeFields {
            id: Uuid("id".to_string()),
            name: key.to_string(),
            key: key.to_string(),
            mime_id: Some(mime.to_string()),
            parent_id: None,
            created_at: None,
        }
    }

    #[test]
    fn root_and_home_are_not_orphans() {
        assert!(!is_orphan(&node("root", "wiki/folder")));
        assert!(!is_orphan(&node("home", "wiki/home")));
        assert!(is_orphan(&node("stray", "wiki/folder")));
    }
}
