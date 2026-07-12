use dioxus::prelude::*;

use crate::graphql::{self, ChildNodeFields, NodeWithChildren};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, visible_sorted};

const FOLDER_VIEW_KEY: &str = "wiki_folder_grid";

/// The copy/paste clipboard: node ids the owner has selected to deep-duplicate.
/// A GlobalSignal so a selection survives navigating to the paste target (React
/// keeps it on the session).
static SELECTED: GlobalSignal<Vec<String>> = Signal::global(Vec::new);

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
    // Owner-only admin (reorder), and the folder "lock": adding children is only
    // offered when the node is `attachable`. Mirrors React FolderDial/AddContentFab.
    let is_context_owner = node.is_context_owner.unwrap_or(false);
    let attachable = node.attachable;
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

    // Surface child events in their own section (#132): a group / folder that
    // contains events lists them separately, above its other content.
    let event_children: Vec<ChildNodeFields> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("wiki/event"))
        .cloned()
        .collect();
    let other_children: Vec<ChildNodeFields> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() != Some("wiki/event"))
        .cloned()
        .collect();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el(mime_id)} }
                h3 { class: "title-medium", "{name}" }
                div { class: "flex-grow" }
                // Secondary/admin folder actions live in the M3 tools sheet
                // (bottom sheet on mobile, right side sheet on desktop).
                if (is_auth && count > 0) || is_context_owner {
                    super::widgets::ToolSheet {
                        title: t("common.tools"),
                        // Export the folder and everything nested under it to an .odt.
                        if is_auth && count > 0 {
                            button {
                                class: "sheet-action",
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
                                "{t(\"folder.export\")}"
                            }
                        }
                        // Reorder children (the sort app), for owners with >1 child.
                        if is_context_owner && count > 1 && !parent_path.is_empty() {
                            Link {
                                to: Route::PathPage {
                                    segments: parent_path.clone(),
                                    app: Some("sort".to_string()),
                                },
                                class: "sheet-action",
                                {icon_el("app/sort")}
                                "{t(\"mime.sort\")}"
                            }
                        }
                        // Lock / unlock adding children (the attachable flag).
                        if is_context_owner {
                            button {
                                class: "sheet-action",
                                onclick: {
                                    let id = node.id.0.clone();
                                    move |_| {
                                        let token = session.read().access_token.clone();
                                        let id = id.clone();
                                        spawn(async move {
                                            let _ = graphql::update_node(
                                                token.as_deref(),
                                                &id,
                                                graphql::NodesSetInput {
                                                    attachable: Some(!attachable),
                                                    ..Default::default()
                                                },
                                            )
                                            .await;
                                            crate::session::bump_data_version();
                                        });
                                    }
                                },
                                span { class: "material-icons",
                                    if attachable { "lock_open" } else { "lock" }
                                }
                                if attachable {
                                    "{t(\"folder.lock\")}"
                                } else {
                                    "{t(\"folder.unlock\")}"
                                }
                            }
                        }
                        // Paste the clipboard selection here (deep-copy), for owners
                        // when something is selected.
                        if is_context_owner && !SELECTED.read().is_empty() {
                            button {
                                class: "sheet-action",
                                onclick: {
                                    let target = node.id.0.clone();
                                    let ctx = node.context_id.clone().map(|c| c.0);
                                    move |_| {
                                        let token = session.read().access_token.clone();
                                        let target = target.clone();
                                        let ctx = ctx.clone();
                                        spawn(async move {
                                            let ids = SELECTED.read().clone();
                                            for id in ids {
                                                // Never paste a folder into itself or
                                                // its own subtree (would recurse).
                                                if graphql::is_descendant_of(token.as_deref(), &target, &id).await {
                                                    continue;
                                                }
                                                let _ = graphql::deep_copy_node(
                                                    token.clone(),
                                                    id,
                                                    target.clone(),
                                                    ctx.clone(),
                                                    true,
                                                )
                                                .await;
                                            }
                                            *SELECTED.write() = vec![];
                                            crate::session::bump_data_version();
                                        });
                                    }
                                },
                                span { class: "material-icons", "content_paste" }
                                "{t(\"folder.paste\")} ({SELECTED.read().len()})"
                            }
                        }
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
        }
        // The folder's contents in a separate card, so the content card's header
        // stays simple (identity + tools). The item count and the list/grid toggle
        // live here, with the children they control.
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small",
                    span { class: "material-icons", "folder_open" }
                }
                h3 { class: "title-medium", "{t(\"common.items\")}" }
                // Child count (#143).
                if count > 0 {
                    span { class: "count-badge", title: "{t(\"common.items\")}", "{count}" }
                }
                div { class: "flex-grow" }
                // Toggle list/grid layout (#125).
                if count > 1 {
                    super::widgets::SegmentedButton {
                        segments: vec![
                            ("list".to_string(), "view_list".to_string()),
                            ("grid".to_string(), "grid_view".to_string()),
                        ],
                        selected: if is_grid { "grid".to_string() } else { "list".to_string() },
                        on_select: move |v: String| {
                            let g = v == "grid";
                            grid.set(g);
                            write_grid_pref(g);
                        },
                    }
                }
            }
            if children.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.noContent\")}"
                    }
                }
            }
            // Events section (#132) — always a list, even in grid mode.
            if !event_children.is_empty() {
                h4 { class: "title-small", class: "text-muted", style: "padding: 8px 16px 0;",
                    "{t(\"layout.events\")}"
                }
                div { class: "list",
                    for child in event_children.iter() {
                        FolderItem {
                            key: "{child.id.0}",
                            node: child.clone(),
                            parent_path: parent_path.clone(),
                            grid: false,
                            ordinal: None,
                            selectable: is_context_owner,
                        }
                    }
                }
            }
            if !other_children.is_empty() {
                div { class: if is_grid { "folder-grid" } else { "list" },
                    for (child , ordinal) in other_children.iter().zip(super::loader::sibling_ordinals(&other_children)) {
                        FolderItem {
                            key: "{child.id.0}",
                            node: child.clone(),
                            parent_path: parent_path.clone(),
                            grid: is_grid,
                            ordinal,
                            selectable: is_context_owner,
                        }
                    }
                }
            }
        }
        // Create a document or subfolder here — only when the folder accepts
        // children (`attachable`); the backend permissions gate what mimes.
        if is_auth && attachable {
            FolderAdd {
                parent_id: node.id.0.clone(),
                context_id: node.context_id.clone().map(|c| c.0),
            }
        }

        // A group/event context also carries a discussion below its listing,
        // mirroring React's ContentApp(hideMembers) stacked above the FolderApp.
        if matches!(mime_id, "wiki/group" | "wiki/event") {
            super::comments::CommentSection {
                node_id: node.id.0.clone(),
                context_id: node.context_id.clone().map(|c| c.0),
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
    // File-upload state for the `wiki/file` kind: the uploaded file's id / type
    // (persisted on the node's data) and an in-flight flag while it streams up.
    let mut file_id = use_signal(|| Option::<String>::None);
    let mut file_type = use_signal(String::new);
    let mut file_name = use_signal(String::new);
    let mut uploading = use_signal(|| false);

    let is_file = *kind.read() == "wiki/file";

    // Upload the chosen file to NHost storage, then remember its id/type so the
    // Add button can attach it. Mirrors React's FileUploader (upload on select).
    let on_pick_file = move |evt: FormEvent| {
        let files = evt.files();
        let Some(fd) = files.into_iter().next() else {
            return;
        };
        let name = fd.name();
        let ctype = fd.content_type().unwrap_or_default();
        let token = session.read().access_token.clone();
        uploading.set(true);
        file_id.set(None);
        spawn(async move {
            match fd.read_bytes().await {
                Ok(bytes) => {
                    match crate::nhost::upload_file(token.as_deref(), bytes.to_vec(), &name, &ctype)
                        .await
                    {
                        Ok(up) => {
                            // Default the title to the file's stem when the user
                            // has not typed one, so the node has a sensible name.
                            if title.read().trim().is_empty() {
                                let stem = name.rsplit_once('.').map(|(s, _)| s).unwrap_or(&name);
                                title.set(stem.to_string());
                            }
                            file_type.set(up.mime_type.unwrap_or(ctype));
                            file_name.set(name);
                            file_id.set(Some(up.id));
                        }
                        Err(e) => crate::snackbar::show_snackbar(&format!(
                            "{}: {e}",
                            t("error.somethingWentWrong")
                        )),
                    }
                }
                Err(_) => crate::snackbar::show_snackbar(&t("error.somethingWentWrong")),
            }
            uploading.set(false);
        });
    };

    let submit = {
        let parent_id = parent_id.clone();
        let context_id = context_id.clone();
        move |_| {
            let mime = kind.read().clone();
            let typed = title.read().trim().to_string();
            // A file node carries `{ fileId, type }` and requires an upload; the
            // name falls back to the uploaded filename. Other kinds need a title.
            let data = if mime == "wiki/file" {
                let Some(fid) = file_id.read().clone() else {
                    return;
                };
                Some(crate::graphql::Jsonb(serde_json::json!({
                    "fileId": fid,
                    "type": file_type.read().clone(),
                })))
            } else {
                if typed.is_empty() {
                    return;
                }
                None
            };
            let name = if typed.is_empty() {
                file_name.read().clone()
            } else {
                typed
            };
            if name.is_empty() {
                return;
            }
            let token = session.read().access_token.clone();
            let parent_id = parent_id.clone();
            let context_id = context_id.clone();
            spawn(async move {
                let key = crate::components::loader::slugify(&name);
                let input = crate::graphql::NodesInsertInput {
                    name: Some(name),
                    key: Some(key),
                    mime_id: Some(mime),
                    parent_id: Some(crate::graphql::Uuid(parent_id)),
                    context_id: context_id.map(crate::graphql::Uuid),
                    data,
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
                    file_id.set(None);
                    file_name.set(String::new());
                    open.set(false);
                }
            });
        }
    };

    // The Add button is enabled once the form can produce a node: a title for
    // document/folder, or a finished upload for a file.
    let can_submit = if is_file {
        file_id.read().is_some() && !*uploading.read()
    } else {
        !title.read().trim().is_empty()
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

        // Add-content dialog (the reusable M3 widgets::Dialog).
        super::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: t("content.addContent"),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: !can_submit,
                    onclick: submit,
                    "{t(\"common.add\")}"
                }
            },
            div { class: "text-field",
                label { "{t(\"common.title\")}" }
                input {
                    r#type: "text",
                    maxlength: "{crate::components::editor::NODE_NAME_MAXLEN}",
                    value: "{title}",
                    oninput: move |e| title.set(e.value()),
                }
            }
            // File picker (only for the `wiki/file` kind).
            if is_file {
                div { class: "mt-2",
                    div { class: "file-upload-label", "{t(\"content.uploadFile\")}" }
                    // Styled picker: a dashed drop-zone wrapping the hidden
                    // native file input, so it matches the Material UI.
                    label { class: "file-upload",
                        input {
                            r#type: "file",
                            class: "file-upload-input",
                            onchange: on_pick_file,
                        }
                        span { class: "material-icons", "upload_file" }
                        span { class: "file-upload-text",
                            if file_name.read().is_empty() {
                                "{t(\"content.chooseFile\")}"
                            } else {
                                "{file_name}"
                            }
                        }
                    }
                    if *uploading.read() {
                        div { class: "stack stack-h mt-1", style: "align-items: center; gap: 8px;",
                            div { class: "spinner" }
                            span { class: "body-small text-muted", "{t(\"content.uploadFile\")}\u{2026}" }
                        }
                    } else if file_id.read().is_some() {
                        div { class: "file-upload-done",
                            span { class: "material-icons", "check_circle" }
                            span { "{file_name}" }
                        }
                    }
                }
            }
            div { class: "mt-2",
                // TODO: migrate to the shadcn Select once its trigger shows the
                // option label (not the raw value) for value != label.
                select {
                    value: "{kind}",
                    onchange: move |e| kind.set(e.value()),
                    option { value: "wiki/document", "{t(\"mime.document\")}" }
                    option { value: "wiki/folder", "{t(\"mime.folder\")}" }
                    option { value: "wiki/file", "{t(\"mime.file\")}" }
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
    selectable: bool,
) -> Element {
    let name = node.name.as_str();
    let mime_id = node.mime_id.as_deref().unwrap_or("");
    let is_mutable = node.mutable;
    let node_id = node.id.0.clone();
    let is_selected = SELECTED.read().contains(&node_id);

    // Build full path by appending this child's key to the parent path
    let mut full_path = parent_path.clone();
    full_path.push(node.key.clone());

    let avatar = rsx! {
        super::loader::NodeAvatar {
            mime: super::loader::node_icon_mime_id(mime_id, node.data.as_ref().map(|d| &d.0)),
            name: name.to_string(),
            ordinal,
            mutable: is_mutable,
            small: true,
        }
    };
    // Owner copy-toggle: add/remove this node from the paste clipboard without
    // navigating (stop the click reaching the Link/anchor).
    let copy_btn = rsx! {
        if selectable {
            button {
                class: "btn-icon",
                style: "margin-left: auto;",
                title: "{t(\"folder.copy\")}",
                onclick: move |e| {
                    e.stop_propagation();
                    e.prevent_default();
                    let mut sel = SELECTED.write();
                    if let Some(pos) = sel.iter().position(|x| x == &node_id) {
                        sel.remove(pos);
                    } else {
                        sel.push(node_id.clone());
                    }
                },
                span { class: "material-icons",
                    if is_selected { "check_box" } else { "content_copy" }
                }
            }
        }
    };

    rsx! {
        Link {
            to: Route::PathPage { segments: full_path, app: None },
            class: if grid { "folder-tile" } else { "list-link" },
            if grid {
                {avatar}
                div { class: "list-item-text",
                    div { class: "list-item-primary", "{name}" }
                }
                {copy_btn}
            } else {
                super::widgets::ListItem {
                    headline: name.to_string(),
                    selected: is_selected,
                    leading: avatar,
                    trailing: copy_btn,
                }
            }
        }
    }
}
