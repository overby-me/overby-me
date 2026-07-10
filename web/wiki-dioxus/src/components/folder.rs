use dioxus::prelude::*;

use crate::graphql::{self, ChildNodeFields, NodeWithChildren};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, visible_sorted};

const FOLDER_VIEW_KEY: &str = "wiki_folder_grid";

/// Read the remembered folder view mode (grid = true) from localStorage.
fn read_grid_pref() -> bool {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item(FOLDER_VIEW_KEY).ok().flatten())
        .map(|v| v == "1")
        .unwrap_or(false)
}

/// Persist the folder view mode so it is remembered on the next visit.
fn write_grid_pref(grid: bool) {
    if let Some(store) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = store.set_item(FOLDER_VIEW_KEY, if grid { "1" } else { "0" });
    }
}

#[component]
pub fn FolderApp(node: NodeWithChildren, parent_path: Vec<String>) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let access_token = session.read().access_token.clone();
    let name = node.name.clone();
    let mime_id = node
        .mime_id
        .clone()
        .unwrap_or_else(|| "wiki/folder".to_string());
    let node_id = node.id.0.clone();

    // Live children: subscribe to this folder's child nodes so additions and
    // removals (by anyone) show up immediately, filtered + ordered like React.
    let refresh = use_signal(|| 0u32);
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{node_id}\" }} }}) {{ id }} }}"
        ),
        refresh,
    );
    let initial = visible_sorted(&node.children);
    // Re-fetch when the folder (node_id) changes or a live update bumps refresh.
    // Depend on these reactively via use_reactive rather than a keyed remount,
    // which the web renderer does not perform reliably (see PathResolver).
    let rev = *refresh.read();
    let children_res =
        crate::use_data_resource!(|(node_id, access_token, user_id, rev)| async move {
            let _ = rev;
            let uid = user_id?;
            graphql::query_children(access_token.as_deref(), &node_id, &uid)
                .await
                .ok()
        });
    // Use live children once loaded; fall back to the already-resolved set.
    let children = children_res.read().clone().flatten().unwrap_or(initial);
    let children = &children;
    let mime_id = mime_id.as_str();
    let name = name.as_str();

    // Folder view mode: list (default) or a tile grid (#125). Remembered across
    // navigations / sessions in localStorage.
    let mut grid = use_signal(read_grid_pref);
    let is_grid = *grid.read();
    let count = children.len();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el(mime_id)} }
                h3 { class: "title-medium", "{name}" }
                // Child count (#143).
                if count > 0 {
                    span { class: "count-badge", title: "{t(\"common.items\")}", "{count}" }
                }
                div { class: "flex-grow" }
                // Toggle list/grid layout (#125).
                if count > 1 {
                    button {
                        class: "btn-icon",
                        title: if is_grid { "{t(\"common.listView\")}" } else { "{t(\"common.gridView\")}" },
                        onclick: move |_| {
                            let v = !*grid.read();
                            grid.set(v);
                            write_grid_pref(v);
                        },
                        if is_grid {
                            span { class: "material-icons", "view_list" }
                        } else {
                            span { class: "material-icons", "grid_view" }
                        }
                    }
                }
                // Export the folder and everything nested under it to an .odt.
                if is_auth && count > 0 {
                    button {
                        class: "btn-icon",
                        title: "{t(\"folder.export\")}",
                        onclick: {
                            let id = node.id.0.clone();
                            let fname = name.to_string();
                            move |_| {
                                let token = session.read().access_token.clone();
                                let id = id.clone();
                                let fname = fname.clone();
                                spawn(async move {
                                    crate::export::export_tree(token, id, fname).await;
                                });
                            }
                        },
                        span { class: "material-icons", "download" }
                    }
                }
                // Reorder children (the sort app) — only worth showing when there
                // is more than one child and the user can act on it.
                if is_auth && count > 1 && !parent_path.is_empty() {
                    Link {
                        to: Route::PathPage {
                            segments: parent_path.clone(),
                            app: Some("sort".to_string()),
                        },
                        class: "btn-icon",
                        title: "{t(\"mime.sort\")}",
                        {icon_el("app/sort")}
                    }
                }
            }
            // The node's own description: groups, events and folders can carry
            // rich text shown above their children (#missing content text).
            if super::content::has_rich_content(node.data.as_ref().map(|d| &d.0)) {
                div { class: "card-content",
                    super::content::SlateRenderer { data: node.data.as_ref().map(|d| d.0.clone()) }
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
                div { class: if is_grid { "folder-grid" } else { "list" },
                    for (child , ordinal) in children.iter().zip(super::loader::sibling_ordinals(children)) {
                        FolderItem {
                            key: "{child.id.0}",
                            node: child.clone(),
                            parent_path: parent_path.clone(),
                            grid: is_grid,
                            ordinal,
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

/// "Add content" floating action button (matching the old wiki's AddContentFab):
/// a fixed bottom-right FAB that opens a modal to pick document or folder, name
/// it, and insert it. (dioxus-primitives has no FAB — it's a Material pattern —
/// so it is a styled fixed button.)
#[component]
fn FolderAdd(parent_id: String, context_id: Option<String>) -> Element {
    let session = use_session();
    let mut open = use_signal(|| false);
    let mut title = use_signal(String::new);
    let mut kind = use_signal(|| "wiki/document".to_string());

    let submit = {
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
                    // Refetch the folder to show the new child (no full reload).
                    crate::session::bump_data_version();
                    title.set(String::new());
                    open.set(false);
                }
            });
        }
    };

    rsx! {
        // The floating action button.
        button {
            class: "fab",
            title: "{t(\"content.addContent\")}",
            "aria-label": "{t(\"content.addContent\")}",
            onclick: move |_| open.set(true),
            span { class: "material-icons", "add" }
        }

        // Modal add-content form (click the backdrop or Cancel to dismiss).
        if *open.read() {
            div { class: "modal-backdrop", onclick: move |_| open.set(false),
                div {
                    class: "modal-card",
                    onclick: move |e| e.stop_propagation(),
                    h3 { class: "title-medium mb-2", "{t(\"content.addContent\")}" }
                    div { class: "text-field",
                        label { "{t(\"common.title\")}" }
                        input {
                            r#type: "text",
                            maxlength: "{crate::components::editor::NODE_NAME_MAXLEN}",
                            value: "{title}",
                            oninput: move |e| title.set(e.value()),
                        }
                    }
                    div { class: "stack stack-h mt-2", style: "align-items: center; gap: 8px;",
                        // TODO: migrate to the shadcn Select once its trigger shows
                        // the option label (not the raw value) for value != label.
                        select {
                            value: "{kind}",
                            onchange: move |e| kind.set(e.value()),
                            option { value: "wiki/document", "{t(\"mime.document\")}" }
                            option { value: "wiki/folder", "{t(\"mime.folder\")}" }
                        }
                        div { class: "flex-grow" }
                        button {
                            class: "btn btn-outlined",
                            onclick: move |_| open.set(false),
                            "{t(\"common.cancel\")}"
                        }
                        button {
                            class: "btn btn-primary",
                            disabled: title.read().trim().is_empty(),
                            onclick: submit,
                            "{t(\"common.add\")}"
                        }
                    }
                }
            }
        }
    }
}

#[component]
fn FolderItem(
    node: ChildNodeFields,
    parent_path: Vec<String>,
    grid: bool,
    ordinal: Option<usize>,
) -> Element {
    let name = node.name.as_str();
    let mime_id = node.mime_id.as_deref().unwrap_or("");
    let is_mutable = node.mutable;

    // Build full path by appending this child's key to the parent path
    let mut full_path = parent_path.clone();
    full_path.push(node.key.clone());

    rsx! {
        Link {
            to: Route::PathPage { segments: full_path, app: None },
            class: if grid { "folder-tile" } else { "folder-item" },
            super::loader::NodeAvatar {
                mime: super::loader::node_icon_mime_id(mime_id, node.data.as_ref().map(|d| &d.0)),
                name: name.to_string(),
                ordinal,
                mutable: is_mutable,
                small: true,
            }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{name}" }
            }
        }
    }
}
