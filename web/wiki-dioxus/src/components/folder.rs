use crate::model;
use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::{ChildNodeFields, NodeWithChildren};
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, visible_sorted};

const FOLDER_VIEW_KEY: &str = "wiki_folder_grid";

/// A child shown optimistically the instant it is added, before the insert is
/// confirmed. Reconciled by `key` (the node's slug key) against the fetched
/// children.
#[derive(Clone, PartialEq)]
struct PendingChild {
    key: String,
    name: String,
    mime: String,
}

/// The copy/paste clipboard: node ids the owner has selected to deep-duplicate.
/// A GlobalSignal so a selection survives navigating to the paste target (React
/// keeps it on the session).
static SELECTED: GlobalSignal<Vec<String>> = Signal::global(Vec::new);

/// Clear the paste clipboard. Called on logout so one user's selection never
/// carries into the next session.
pub(crate) fn clear_selection() {
    *SELECTED.write() = Vec::new();
}

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
pub fn FolderApp(
    node: NodeWithChildren,
    parent_path: Vec<String>,
    /// Set only by the Screen/projector view: renders a lean, read-only card (no
    /// tools sheet, no add/lock/paste chrome) for the room-facing screen.
    #[props(default)]
    projector: bool,
) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    // Owner-only admin (reorder), and the folder "lock": adding children is only
    // offered when the node is `attachable`. Mirrors React FolderDial/AddContentFab.
    let is_context_owner = node.is_context_owner.unwrap_or(false);
    // Optimistic lock/unlock: flip the attachable state at once, reconcile against
    // the refetched node, revert on error.
    let mut attachable_opt = use_signal(|| None::<bool>);
    {
        let na = node.attachable;
        use_effect(use_reactive!(|(na)| {
            if *attachable_opt.peek() == Some(na) {
                attachable_opt.set(None);
            }
        }));
    }
    let attachable = attachable_opt().unwrap_or(node.attachable);
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let access_token = session.read().access_token.clone();
    let name = node.name.clone();
    let mime_id = node
        .mime_id
        .clone()
        .unwrap_or_else(|| "wiki/folder".to_string());
    let node_id = node.id.0.clone();
    let nav = use_navigator();

    // Parity with ContentApp: a group/event/folder is content too, so it gets the
    // same tools (project, edit, delete, share, comments-on-screen). A node/context
    // owner may manage; editing also requires the node to still be mutable. The
    // projector context is the node's context (or itself when it is its own).
    let can_manage = node.is_owner.unwrap_or(false) || is_context_owner;
    let can_edit = can_manage && node.mutable;
    let node_context = node
        .context_id
        .as_ref()
        .map(|c| c.0.clone())
        .unwrap_or_else(|| node.id.0.clone());
    let mut confirm_open = use_signal(|| false);
    // Bluesky link status gates the share action (like ContentApp).
    let link_token = access_token.clone();
    let bsky_link = crate::use_data_resource!(|(link_token)| async move {
        match link_token {
            Some(t) => crate::backend_api::atproto_status(&t).await.linked,
            None => false,
        }
    });
    let bsky_linked = (*bsky_link.read()).unwrap_or(false);
    // Owner toggle state for showing this context's comments on the projector.
    let mut screen_comments = use_signal(|| None::<bool>);
    {
        let token = access_token.clone();
        let ctx = node_context.clone();
        let can = can_manage;
        use_hook(move || {
            if can {
                spawn(async move {
                    let on = crate::graphql::screen_comments_on(token.as_deref(), &ctx)
                        .await
                        .unwrap_or(false);
                    screen_comments.set(Some(on));
                });
            }
        });
    }
    // The path to return to after deleting this node (its parent).
    let delete_parent: Vec<String> = if parent_path.is_empty() {
        vec![]
    } else {
        parent_path[..parent_path.len() - 1].to_vec()
    };

    // Live children: subscribe to this folder's child nodes so additions and
    // removals (by anyone) show up immediately, filtered + ordered like React.
    let refresh = use_signal(|| 0u32);
    let sub_node = crate::graphql::gql_escape(&node_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{sub_node}\" }} }}) {{ id }} }}"
        ),
        refresh,
    );
    // Sort the resolver-provided children once per mount rather than re-cloning +
    // re-sorting on every render (multi-select toggles SELECTED, which re-renders
    // this whole component). node.children is fixed for the mount (nav remounts).
    let initial = use_hook(|| visible_sorted(&node.children));
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
    // Optimistic add-child (FolderAdd pushes here): muted tiles shown at once and
    // reconciled by key against the fetched children.
    let pending = use_signal(Vec::<PendingChild>::new);
    let child_keys: std::collections::HashSet<String> =
        children.iter().map(|c| c.key.clone()).collect();
    let pending_shown = crate::components::optimistic::reconcile_by_key(
        &pending.read(),
        |p| p.key.as_str(),
        &child_keys,
    );

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el(mime_id)} }
                h3 { class: "title-medium", "{name}" }
                div { class: "flex-grow" }
                // Secondary/admin folder actions live in the M3 tools sheet
                // (bottom sheet on mobile, right side sheet on desktop). Hidden on
                // the projector, which is read-only for the room.
                if !projector {
                    super::widgets::ToolSheet {
                        title: t("common.tools"),
                        // Copy a shareable link to this folder (keeps æøå literal).
                        // Unconditional, like ContentApp: a read-only action anyone
                        // may use (a document link was already ungated).
                        super::widgets::CopyLinkAction {}
                        // Export the folder and everything nested under it to an .odt,
                        // aligned with the document export (unconditional).
                        super::widgets::ExportAction { node_id: node.id.0.clone(), name: name.to_string() }
                        // Share this page to the signed-in user's linked Bluesky account.
                        if is_auth && bsky_linked {
                            button {
                                class: "sheet-action",
                                onclick: {
                                    let share_name = name.to_string();
                                    move |_| {
                                        let token = session.read().access_token.clone();
                                        let title = share_name.clone();
                                        spawn(async move {
                                            let Some(token) = token else { return };
                                            let href = web_sys::window()
                                                .and_then(|w| w.location().href().ok())
                                                .unwrap_or_default();
                                            let title: String = title.chars().take(200).collect();
                                            let text = format!("{title}\n\n{href}");
                                            crate::snackbar::show_snackbar(&t("content.sharing"));
                                            match crate::backend_api::atproto_post(&token, &text, &href, &title).await {
                                                Ok(()) => crate::snackbar::show_snackbar(&t("content.shared")),
                                                Err(e) if e.contains("no linked") => {
                                                    crate::snackbar::show_snackbar(&t("content.shareNoLink"))
                                                }
                                                Err(_) => crate::snackbar::show_snackbar(&t("content.shareErr")),
                                            }
                                        });
                                    }
                                },
                                {icon_el("app/social")}
                                "{t(\"content.shareBluesky\")}"
                            }
                        }
                        // Owner: project this container onto the context's Screen.
                        if can_manage {
                            button {
                                class: "sheet-action",
                                onclick: {
                                    let target = node.id.0.clone();
                                    let ctx = node_context.clone();
                                    move |_| {
                                        let target = target.clone();
                                        let ctx = ctx.clone();
                                        let token = session.read().access_token.clone();
                                        spawn(async move {
                                            match crate::graphql::set_active_relation(token.as_deref(), &ctx, Some(&target)).await {
                                                Ok(_) => crate::snackbar::show_snackbar(&t("content.projected")),
                                                Err(_) => crate::snackbar::show_snackbar(&t("error.somethingWentWrong")),
                                            }
                                        });
                                    }
                                },
                                span { class: "material-icons", "cast" }
                                "{t(\"content.projectScreen\")}"
                            }
                            // Owner: also show this context's comments on the Screen.
                            button {
                                class: "sheet-action",
                                onclick: {
                                    let ctx = node_context.clone();
                                    move |_| {
                                        let ctx = ctx.clone();
                                        let token = session.read().access_token.clone();
                                        let next = !(*screen_comments.read()).unwrap_or(false);
                                        spawn(async move {
                                            match crate::graphql::set_screen_comments(token.as_deref(), &ctx, next).await {
                                                Ok(_) => {
                                                    screen_comments.set(Some(next));
                                                    crate::snackbar::show_snackbar(&t(if next {
                                                        "content.commentsOnScreen"
                                                    } else {
                                                        "content.commentsOffScreen"
                                                    }));
                                                }
                                                Err(_) => crate::snackbar::show_snackbar(&t("error.somethingWentWrong")),
                                            }
                                        });
                                    }
                                },
                                span { class: "material-icons",
                                    if (*screen_comments.read()).unwrap_or(false) { "speaker_notes_off" } else { "forum" }
                                }
                                if (*screen_comments.read()).unwrap_or(false) {
                                    "{t(\"content.hideCommentsScreen\")}"
                                } else {
                                    "{t(\"content.showCommentsScreen\")}"
                                }
                            }
                        }
                        // Owner: edit this container's own rich-text description.
                        if can_edit && !parent_path.is_empty() {
                            Link {
                                to: Route::PathPage {
                                    segments: parent_path.clone(),
                                    app: Some("editor".to_string()),
                                },
                                class: "sheet-action",
                                {icon_el("app/editor")}
                                "{t(\"mime.editor\")}"
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
                                        let new_val = !attachable;
                                        // Optimistic: flip the lock icon now.
                                        attachable_opt.set(Some(new_val));
                                        spawn(async move {
                                            match graphql::update_node(
                                                token.as_deref(),
                                                &id,
                                                model::NodesSetInput {
                                                    attachable: Some(new_val),
                                                    ..Default::default()
                                                },
                                            )
                                            .await
                                            {
                                                Ok(_) => crate::session::bump_data_version(),
                                                Err(e) => {
                                                    attachable_opt.set(None);
                                                    log::error!("lock toggle failed: {e}");
                                                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                }
                                            }
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
                        // Owner: delete this container (with a confirm dialog).
                        if can_manage && !parent_path.is_empty() {
                            button {
                                class: "sheet-action danger",
                                onclick: move |_| confirm_open.set(true),
                                span { class: "material-icons", "delete" }
                                "{t(\"common.delete\")}"
                            }
                        }
                    }
                }
            }
            // Delete confirm dialog (owner action), mirroring ContentApp.
            if can_manage && !parent_path.is_empty() {
                super::widgets::Dialog {
                    open: confirm_open(),
                    on_dismiss: move |_| confirm_open.set(false),
                    headline: t("content.confirmDelete"),
                    icon: "delete".to_string(),
                    actions: rsx! {
                        button {
                            class: "btn btn-outlined",
                            onclick: move |_| confirm_open.set(false),
                            "{t(\"common.cancel\")}"
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: {
                                let node_del = node.id.0.clone();
                                let dest = delete_parent.clone();
                                move |_| {
                                    let token = session.read().access_token.clone();
                                    let node_del = node_del.clone();
                                    let dest = dest.clone();
                                    confirm_open.set(false);
                                    spawn(async move {
                                        if let Err(e) = graphql::delete_node_members(token.as_deref(), &node_del).await {
                                            log::error!("delete_node_members failed: {e}");
                                            crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                            return;
                                        }
                                        match graphql::delete_node(token.as_deref(), &node_del).await {
                                            Ok(true) => {
                                                crate::session::bump_data_version();
                                                nav.push(Route::PathPage { segments: dest, app: None });
                                            }
                                            other => {
                                                log::error!("delete_node failed: {other:?}");
                                                crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                            }
                                        }
                                    });
                                }
                            },
                            "{t(\"common.delete\")}"
                        }
                    },
                    p { class: "body-medium", "{name}" }
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
                h3 { class: "title-medium", "{t(\"mime.folder\")}" }
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
                // Create a document or subfolder here — the add action lives in this
                // section's header (only when the folder accepts children; the
                // backend permissions gate which mimes). Never on the projector.
                if is_auth && attachable && !projector {
                    FolderAdd {
                        parent_id: node.id.0.clone(),
                        context_id: node.context_id.clone().map(|c| c.0),
                        pending,
                    }
                }
            }
            if children.is_empty() {
                // DESIGN: a compact characterful empty state (floating orb).
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "folder_open" }
                    }
                    p { class: "empty-state-body", "{t(\"common.noContent\")}" }
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
                            selectable: is_context_owner && !projector,
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
                            selectable: is_context_owner && !projector,
                        }
                    }
                }
            }
            // Optimistic new children (muted "sending" rows), dropped once confirmed.
            if !pending_shown.is_empty() {
                div { class: "list",
                    for p in pending_shown.iter() {
                        div { key: "{p.key}", class: "list-item is-pending",
                            div { class: "avatar small", {icon_el(&p.mime)} }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{p.name}" }
                                div { class: "list-item-secondary", "{t(\"vote.sending\")}" }
                            }
                        }
                    }
                }
            }
        }

        // Containers (group/event/folder) do not carry comments: the permission
        // model puts vote/comment on CONTENT nodes (motions, amendments, documents,
        // …), not on the container. Dropping the dead section here makes the three
        // FolderApp types consistent (folder never had one) rather than showing a
        // comment box on group/event that could never accept a post.
    }
}

/// "Add content" floating action button (matching the old wiki's AddContentFab):
/// a fixed bottom-right FAB that opens a modal to pick document or folder, name
/// it, and insert it. (dioxus-primitives has no FAB — it's a Material pattern —
/// so it is a styled fixed button.)
#[component]
fn FolderAdd(
    parent_id: String,
    context_id: Option<String>,
    /// Optimistic children owned by FolderApp: this dialog pushes the new node here
    /// (shown at once), reconciled/rolled back there and here.
    mut pending: Signal<Vec<PendingChild>>,
) -> Element {
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

    // Which content types may be created here, derived from the permission system
    // (like the old wiki's add-content dialog) rather than a fixed list. This is
    // what restores creating motions (`vote/policy`) and elections
    // (`vote/position`) through the UI, not only documents/folders/files: the
    // dropdown lists exactly the mimes the server would accept under this node.
    let add_nid = parent_id.clone();
    let add_tok = session.read().access_token.clone();
    let insertable_res = crate::use_data_resource!(|(add_nid, add_tok)| async move {
        crate::graphql::node_insert_mimes(add_tok.as_deref(), &add_nid).await
    });
    // The creatable content mimes offered here, in display order, each paired with
    // its label key. Types with a dedicated add button (speaker lists, amendments,
    // candidacies, questions, comments, polls) are intentionally omitted.
    const CONTENT_MIMES: &[(&str, &str)] = &[
        ("wiki/document", "mime.document"),
        ("vote/policy", "mime.policy"),
        ("vote/position", "mime.position"),
        ("wiki/folder", "mime.folder"),
        ("wiki/file", "mime.file"),
    ];
    let insertable = insertable_res.read().clone().unwrap_or_default();
    let options: Vec<(&str, &str)> = CONTENT_MIMES
        .iter()
        .copied()
        .filter(|(m, _)| insertable.iter().any(|i| i == m))
        .collect();
    // Keep the chosen kind valid once permissions load: if the current selection
    // is not among the offered options, snap to the first one so a plain "Add"
    // click can never submit a mime the server will reject. Reads the resource
    // inside the effect so it re-runs when the insertable list actually resolves.
    use_effect(move || {
        let insertable = insertable_res.read().clone().unwrap_or_default();
        let opts: Vec<&str> = CONTENT_MIMES
            .iter()
            .map(|(m, _)| *m)
            .filter(|m| insertable.iter().any(|i| i == m))
            .collect();
        let valid = opts.iter().any(|m| *m == *kind.read());
        if !opts.is_empty() && !valid {
            kind.set(opts[0].to_string());
        }
    });

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
                Some(crate::model::Jsonb(serde_json::json!({
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
            let key = crate::components::loader::slugify(&name);
            // Optimistic: show the child tile now and close the dialog; reconciled by
            // key against the fetched children, removed on error.
            pending.write().push(PendingChild {
                key: key.clone(),
                name: name.clone(),
                mime: mime.clone(),
            });
            title.set(String::new());
            file_id.set(None);
            file_name.set(String::new());
            open.set(false);
            spawn(async move {
                let input = crate::model::NodesInsertInput {
                    name: Some(name),
                    key: Some(key.clone()),
                    mime_id: Some(mime),
                    parent_id: Some(crate::model::Uuid(parent_id)),
                    context_id: context_id.map(crate::model::Uuid),
                    data,
                    mutable: Some(true),
                    index: None,
                };
                match crate::graphql::insert_node(token.as_deref(), input).await {
                    Ok(_) => crate::session::bump_data_version(),
                    Err(e) => {
                        pending.write().retain(|p| p.key != key);
                        log::error!("add child failed: {e}");
                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                    }
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
        // The create action, anchored in the Items card header (M3 Expressive
        // filled-tonal icon button) rather than a floating FAB.
        button {
            class: "btn-icon add-action state-layer",
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
                // A native <select> (styled to match the editor's), used because it
                // renders localized option labels correctly when value != label.
                select {
                    class: "editor-select",
                    "aria-label": t("common.type"),
                    value: "{kind}",
                    onchange: move |e| kind.set(e.value()),
                    for (mime , label_key) in options.iter().copied() {
                        option { key: "{mime}", value: "{mime}", "{t(label_key)}" }
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
