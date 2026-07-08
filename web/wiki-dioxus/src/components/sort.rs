use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

use super::loader::mime_icon;

/// SortApp — drag-and-drop reordering of child nodes
#[component]
pub fn SortApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let node_id = node.id.0.clone();

    let mut items = use_signal(|| node.children.clone());
    let mut dragging_idx = use_signal(|| None::<usize>);
    let mut saving = use_signal(|| false);

    let handle_save = {
        let token = session.read().access_token.clone();
        let node_id = node_id.clone();
        move |_| {
            let token = token.clone();
            let _node_id = node_id.clone();
            let current_items = items.read().clone();
            spawn(async move {
                saving.set(true);

                // Persist each child's new index.
                for (i, item) in current_items.iter().enumerate() {
                    let set = graphql::NodesSetInput {
                        index: Some(i32::try_from(i).unwrap_or(0)),
                        ..Default::default()
                    };
                    let _ = graphql::update_node(token.as_deref(), &item.id.0, set).await;
                }

                show_snackbar(&t("sort.saveSorting"));
                saving.set(false);
            });
        }
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "\u{2195}" }
                h3 { class: "title-medium", "{t(\"mime.sort\")}" }
                div { class: "flex-grow" }
                if is_auth {
                    button {
                        class: "btn btn-primary",
                        disabled: *saving.read(),
                        onclick: handle_save,
                        "\u{1F4BE} {t(\"sort.saveSorting\")}"
                    }
                    if *saving.read() {
                        div { class: "spinner", style: "margin-left: 8px;" }
                    }
                }
            }

            div { class: "list sort-list",
                for (i , item) in items.read().iter().enumerate() {
                    div {
                        class: if *dragging_idx.read() == Some(i) { "list-item sort-item dragging" } else { "list-item sort-item" },
                        key: "{item.id.0}",
                        draggable: "true",
                        ondragstart: move |_| {
                            dragging_idx.set(Some(i));
                        },
                        ondragover: move |evt| {
                            evt.prevent_default();
                        },
                        ondrop: move |evt| {
                            evt.prevent_default();
                            if let Some(from) = *dragging_idx.read() {
                                if from != i {
                                    let mut new_items = items.read().clone();
                                    let moved = new_items.remove(from);
                                    new_items.insert(i, moved);
                                    items.set(new_items);
                                }
                            }
                            dragging_idx.set(None);
                        },
                        ondragend: move |_| {
                            dragging_idx.set(None);
                        },

                        // Drag handle
                        span {
                            style: "cursor: grab; margin-right: 8px; color: var(--md-on-surface-variant);",
                            "\u{2630}"
                        }
                        div { class: "avatar small",
                            "{mime_icon(item.mime_id.as_deref().unwrap_or(\"\"))}"
                        }
                        div { class: "list-item-text",
                            div { class: "list-item-primary", "{item.name}" }
                        }
                    }
                }
            }
        }
    }
}
