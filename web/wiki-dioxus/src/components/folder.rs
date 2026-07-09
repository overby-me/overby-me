use dioxus::prelude::*;

use crate::graphql::{ChildNodeFields, NodeWithChildren};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{mime_icon, visible_sorted};

#[component]
pub fn FolderApp(node: NodeWithChildren, parent_path: Vec<String>) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let name = node.name.as_str();
    let mime_id = node.mime_id.as_deref().unwrap_or("wiki/folder");
    let children = visible_sorted(&node.children);
    let children = &children;

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "{mime_icon(mime_id)}" }
                h3 { class: "title-medium", "{name}" }
                // Reorder children (the sort app) — only worth showing when there
                // is more than one child and the user can act on it.
                if is_auth && children.len() > 1 && !parent_path.is_empty() {
                    div { class: "flex-grow" }
                    Link {
                        to: Route::PathPage {
                            segments: parent_path.clone(),
                            app: Some("sort".to_string()),
                        },
                        class: "btn-icon",
                        title: "{t(\"mime.sort\")}",
                        "{mime_icon(\"app/sort\")}"
                    }
                }
            }
            if children.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                div { class: "list",
                    for child in children.iter() {
                        FolderItem {
                            key: "{child.id.0}",
                            node: child.clone(),
                            parent_path: parent_path.clone(),
                        }
                    }
                }
            }

            // Create a document or subfolder here (a folder/group/event the user
            // can add to). Mirrors the React AddContent flow for the simple mimes.
            if is_auth {
                FolderAdd {
                    parent_id: node.id.0.clone(),
                    context_id: node.context_id.clone().map(|c| c.0),
                }
            }
        }
    }
}

/// Inline "add content" form: pick document or folder, name it, insert it.
#[component]
fn FolderAdd(parent_id: String, context_id: Option<String>) -> Element {
    let session = use_session();
    let mut open = use_signal(|| false);
    let mut title = use_signal(String::new);
    let mut kind = use_signal(|| "wiki/document".to_string());

    if !*open.read() {
        return rsx! {
            div { class: "card-content",
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(true),
                    "\u{2795} {t(\"content.addContent\")}"
                }
            }
        };
    }

    rsx! {
        div { class: "card-content",
            div { class: "text-field",
                label { "{t(\"common.title\")}" }
                input {
                    r#type: "text",
                    value: "{title}",
                    oninput: move |e| title.set(e.value()),
                }
            }
            div { class: "stack stack-h mt-1", style: "align-items: center; gap: 8px;",
                select {
                    value: "{kind}",
                    onchange: move |e| kind.set(e.value()),
                    option { value: "wiki/document", "{t(\"mime.document\")}" }
                    option { value: "wiki/folder", "{t(\"mime.folder\")}" }
                }
                button {
                    class: "btn btn-primary",
                    disabled: title.read().trim().is_empty(),
                    onclick: {
                        let parent_id = parent_id.clone();
                        let context_id = context_id.clone();
                        move |_| {
                            let name = title.read().trim().to_string();
                            if name.is_empty() {
                                return;
                            }
                            let token = session.read().access_token.clone();
                            let parent_id = parent_id.clone();
                            let context_id = context_id.clone();
                            let mime = kind.read().clone();
                            spawn(async move {
                                let key = crate::components::loader::slugify(&name);
                                let input = crate::graphql::NodesInsertInput {
                                    name: Some(name),
                                    key: Some(key),
                                    mime_id: Some(mime),
                                    parent_id: Some(crate::graphql::Uuid(parent_id)),
                                    context_id: context_id.map(crate::graphql::Uuid),
                                    data: None,
                                    mutable: Some(true),
                                    index: None,
                                };
                                if crate::graphql::insert_node(token.as_deref(), input)
                                    .await
                                    .is_ok()
                                {
                                    // Re-resolve the folder to show the new child.
                                    if let Some(w) = web_sys::window() {
                                        let _ = w.location().reload();
                                    }
                                }
                            });
                        }
                    },
                    "{t(\"common.add\")}"
                }
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
            }
        }
    }
}

#[component]
fn FolderItem(node: ChildNodeFields, parent_path: Vec<String>) -> Element {
    let name = node.name.as_str();
    let mime_id = node.mime_id.as_deref().unwrap_or("");
    let icon = mime_icon(mime_id);
    let is_mutable = node.mutable;

    // Build full path by appending this child's key to the parent path
    let mut full_path = parent_path.clone();
    full_path.push(node.key.clone());

    rsx! {
        Link {
            to: Route::PathPage { segments: full_path, app: None },
            class: "folder-item",
            div { class: "avatar small", "{icon}" }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{name}" }
                if is_mutable {
                    div { class: "list-item-secondary",
                        "\u{1F513} {t(\"layout.notSubmitted\")}"
                    }
                }
            }
        }
    }
}
