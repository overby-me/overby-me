use crate::model;
use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

use super::loader::icon_el;

/// SortApp — drag-and-drop reordering of child nodes
#[component]
pub fn SortApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let node_id = node.id.0.clone();
    let nav = use_navigator();
    // The folder's own path, so saving returns to its rendered (non-app) view.
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };

    // Only the visible children are reorderable (hidden mimes — e.g. vote/vote
    // entries — must not appear in the sort list); seeded in current index order.
    let mut items = use_signal(|| super::loader::visible_sorted(&node.children));
    let mut dragging_idx = use_signal(|| None::<usize>);
    let mut saving = use_signal(|| false);

    let handle_save = {
        let token = session.read().access_token.clone();
        let node_id = node_id.clone();
        let segments = segments.clone();
        move |_| {
            let token = token.clone();
            let _node_id = node_id.clone();
            let segments = segments.clone();
            let current_items = items.read().clone();
            spawn(async move {
                saving.set(true);

                // Persist each child's new index. Only claim success and navigate
                // away when every update lands; a failed write must not read as a
                // saved reorder (the folder subscription won't refire on failure).
                let mut ok = true;
                for (i, item) in current_items.iter().enumerate() {
                    let set = model::NodesSetInput {
                        index: Some(i32::try_from(i).unwrap_or(0)),
                        ..Default::default()
                    };
                    if let Err(e) = graphql::update_node(token.as_deref(), &item.id.0, set).await {
                        crate::errors::log_handled("sort: update_node failed", e);
                        ok = false;
                        break;
                    }
                }
                saving.set(false);
                if !ok {
                    show_snackbar(&t("error.somethingWentWrong"));
                    return;
                }

                show_snackbar(&t("sort.saveSorting"));
                // Invalidate the cached folder so the new order shows on return.
                crate::session::bump_data_version();
                // Return to the folder's rendered (non-app) view.
                nav.push(Route::PathPage {
                    segments,
                    app: None,
                });
            });
        }
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("app/sort")} }
                h3 { class: "title-medium", "{t(\"mime.sort\")}" }
                div { class: "flex-grow" }
                if is_auth {
                    button {
                        class: "btn btn-primary",
                        disabled: *saving.read(),
                        onclick: handle_save,
                        span { class: "material-icons", "save" }
                        " {t(\"sort.saveSorting\")}"
                    }
                    if *saving.read() {
                        div { class: "spinner spinner-sm" }
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

                        // The drag-handle glyph and the grab cursor are the
                        // `.sort-list .sort-item` treatment (a ::before), so the
                        // row never carries a second, hand-placed handle.
                        div { class: "avatar small",
                            {super::loader::node_icon_el(item.mime_id.as_deref().unwrap_or(""), item.data.as_ref().map(|d| &d.0))}
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
