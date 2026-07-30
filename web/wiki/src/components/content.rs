use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, mime_icon};

/// Allowlist filter for an authored link href. Members can type link URLs into
/// the rich-text editor; only http/https/mailto and app-relative (`/`, `#`) URLs
/// are safe to render as a live anchor. Anything else — `javascript:`, `data:`,
/// `vbscript:` — is neutralized to `#` so a planted link can't run script in
/// another member's session when clicked (stored-XSS defense at the render sink).
pub(crate) fn safe_href(url: &str) -> String {
    let u = url.trim();
    let lower = u.to_ascii_lowercase();
    if lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("mailto:")
        || u.starts_with('/')
        || u.starts_with('#')
    {
        u.to_string()
    } else {
        "#".to_string()
    }
}

/// The content card only (title, image, members, body). Comments are a separate
/// [`super::comments::CommentSection`] composed by each caller, so composite
/// views (policy/position) can place amendments/candidates above the thread.
#[component]
pub fn ContentApp(
    node: NodeWithChildren,
    /// A section that belongs to this node but is not its text, rendered inside
    /// the same card under the body: a position's candidates, which are the
    /// substance of the page when (as usually) the position itself is untitled
    /// prose. Its presence also means an empty body is not "no content", so the
    /// empty state stands down rather than announcing a lack nobody felt.
    #[props(default)]
    extra: Option<Element>,
) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    // Whether the user has a linked Bluesky account — gates the "share to Bluesky"
    // tools action so it only appears when sharing would actually work.
    let link_token = session.read().access_token.clone();
    let bsky_link = crate::use_data_resource!(|(link_token)| async move {
        match link_token {
            Some(t) => crate::backend_api::atproto_status(&t).await.linked,
            None => false,
        }
    });
    let bsky_linked = (*bsky_link.read()).unwrap_or(false);
    let nav = use_navigator();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let node_id = node.id.0.clone();
    // Where the bin finds this node's subtree, and who to record as having binned
    // it (see `graphql::bin_node`).
    let node_path = node.path.clone();
    let actor = session.read().user.as_ref().map(|u| u.id.clone());
    let mut confirm_open = use_signal(|| false);
    // Submitting is irreversible (it makes the node immutable), so the sheet's
    // row opens the same warning dialog the editor's submit button opens.
    let mut confirm_submit = use_signal(|| false);
    // Binning is one statement, but it still round-trips, so the confirm button
    // reports that it is working rather than appearing to do nothing.
    let mut deleting = use_signal(|| false);
    let name = node.name.clone();
    let members = node.members.clone();
    let created = node.created_at.as_ref().map(|t| t.0.clone());
    let data = node.data.map(|d| d.0);
    // Owner-only actions (mirrors the React ContentToolbar gating): a node/context
    // owner may delete; editing also requires the node to still be mutable.
    let can_manage = node.is_owner.unwrap_or(false) || node.is_context_owner.unwrap_or(false);
    let can_edit = can_manage && node.mutable;
    // Still mutable means not yet submitted: the header avatar carries the same
    // badge the node's row carries in a list.
    let is_mutable = node.mutable;

    // What members may add under this node, per the context permission template:
    // candidatures under a position, amendments under a motion or an amendment.
    // Its owner closes them with the same `attachable` lock a folder uses for its
    // content, and the row says which of the three it is rather than "content"
    // for all of them. `None` means nothing member-inserted lives here, and then
    // there is no lock row at all.
    let lock_labels: Option<(String, String)> = match node.mime_id.as_deref() {
        Some("vote/position") => Some((t("folder.lockCandidates"), t("folder.unlockCandidates"))),
        Some("vote/policy") | Some("vote/change") => {
            Some((t("folder.lockAmendments"), t("folder.unlockAmendments")))
        }
        _ => None,
    };
    let lockable = lock_labels.is_some();
    let (lock_text, unlock_text) = lock_labels.unwrap_or_default();
    // Optimistic lock/unlock: flip now, reconcile against the refetched node,
    // revert on error (the same shape FolderApp uses).
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
    // A context owner may reorder this node's children (candidates in an election,
    // amendments on a motion, questions), when there is more than one to arrange.
    // Restores the old per-list "sort" affordance the port had dropped, in one
    // place: the sort app reorders all of a node's visible children.
    let is_ctx_owner = node.is_context_owner.unwrap_or(false);
    let reorderable_children = super::loader::visible_sorted(&node.children).len() > 1;
    // The context whose projector (Screen) this node can be pushed to; falls back
    // to the node itself when it is its own context (a top-level group/event).
    let node_context = node
        .context_id
        .as_ref()
        .map(|c| c.0.clone())
        .unwrap_or_else(|| node.id.0.clone());

    // Owner toggle state: whether the context is currently set to also show the
    // active node's comments on the projector (the `screenComments` relation).
    // The hook is unconditional (stable order); only the fetch is owner-gated.
    let mut screen_comments = use_signal(|| None::<bool>);
    {
        let token = session.read().access_token.clone();
        let ctx = node_context.clone();
        let can = can_manage;
        // Reactive on the context — NOT a one-shot `use_hook` — since this
        // component is reused across sibling navigations without remounting;
        // keyed on `ctx`, so moving to a node in a different context refetches
        // that context's setting instead of showing the previous one's.
        use_effect(use_reactive!(|(ctx, token, can)| {
            if can {
                spawn(async move {
                    let on = crate::graphql::screen_comments_on(token.as_deref(), &ctx)
                        .await
                        .unwrap_or(false);
                    screen_comments.set(Some(on));
                });
            }
        }));
    }

    // Optional inline image (a `data.image` file id), mirroring React's Content.
    // Fetched with the token in the Authorization header → a blob: URL, so the JWT
    // never enters an <img src> attribute.
    let image_file_id = data
        .as_ref()
        .and_then(|d| d.get("image"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(String::from);
    let image_url = super::loader::use_file_object_url(image_file_id.unwrap_or_default());
    // A candidate's photo is a portrait, not a landscape cover, so the wide hero
    // band crops it hard. Candidate heroes get the taller frame; documents keep
    // the wide cover proportions.
    let portrait_hero = node.mime_id.as_deref() == Some("vote/candidate");
    // The header avatar shows what the node's own row shows in a list: its icon,
    // or the A/B/C of a policy and the 1/2/3 of a change. ContentApp is not only
    // the document view (candidates, positions, policies and changes all render
    // through it), so it must not assume the document icon. `get_index` is the
    // backend's 1-based ordinal among same-type siblings; `node_avatar` wants it
    // 0-based, and ignores it for every mime that is not lettered or numbered.
    let header_icon = super::loader::node_avatar(
        &super::loader::node_icon_mime_id(
            node.mime_id.as_deref().unwrap_or("wiki/document"),
            data.as_ref(),
        ),
        &name,
        node.get_index.filter(|i| *i >= 1).map(|i| (i - 1) as usize),
    );

    rsx! {
        div { class: "card",
            // Identity zone: when the document carries an image it becomes a full-bleed
            // cover hero with the title/date overlaid on a legibility scrim, so the
            // image frames the document instead of sitting as a plain block above it.
            if let Some(url) = image_url {
                div { class: if portrait_hero { "content-hero is-portrait" } else { "content-hero" },
                    // ZoomableImage keeps the click-to-expand lightbox; the veil above
                    // is click-through so the image underneath still receives it.
                    super::widgets::ZoomableImage { src: url.clone(), alt: t("content.imageAlt") }
                    div { class: "content-hero-veil",
                        // The same not-submitted mark the list avatars carry, so a
                        // document's own page says what its row in the list said.
                        super::loader::AvatarBadged { mutable: is_mutable,
                            div { class: "avatar content-hero-avatar", {header_icon.clone()} }
                        }
                        div { class: "content-hero-meta",
                            h3 { class: "content-hero-title", "{name}" }
                            if let Some(iso) = created.as_ref() {
                                p {
                                    class: "content-hero-date",
                                    title: "{super::loader::full_datetime(iso)}",
                                    span { class: "material-icons icon-inline", "schedule" }
                                    " {super::loader::relative_time(iso)}"
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "card-header",
                    super::loader::AvatarBadged { mutable: is_mutable,
                        div { class: "avatar", {header_icon.clone()} }
                    }
                    div {
                        h3 { class: "title-medium", "{name}" }
                        if let Some(iso) = created.as_ref() {
                            p {
                                class: "body-small",
                                class: "text-muted",
                                title: "{super::loader::full_datetime(iso)}",
                                span { class: "material-icons icon-inline", "schedule" }
                                " {super::loader::relative_time(iso)}"
                            }
                        }
                    }
                }
            }
            // Document actions live in the M3 tools sheet — a fixed FAB (or the docked
            // side sheet on wide screens), never an in-header button — with a
            // delete-confirm dialog.
            super::widgets::ToolSheet {
                title: t("common.tools"),
                // Pinned quick group (copy link is the sheet's own first segment):
                // export this document and anything nested to .odt, and share the
                // page to the signed-in user's linked Bluesky account. The share
                // segment only appears once an account is actually linked.
                quick: rsx! {
                    super::widgets::ExportAction { node_id: node_id.clone(), name: name.clone() }
                    if is_auth && bsky_linked {
                        button {
                            class: "sheet-quick-action",
                            r#type: "button",
                            title: "{t(\"content.shareBluesky\")}",
                            aria_label: "{t(\"content.shareBluesky\")}",
                            onclick: {
                            let share_name = name.clone();
                            move |_| {
                                let token = session.read().access_token.clone();
                                let title = share_name.clone();
                                spawn(async move {
                                    let Some(token) = token else { return };
                                    let href = web_sys::window()
                                        .and_then(|w| w.location().href().ok())
                                        .unwrap_or_default();
                                    // Keep the title within Bluesky's 300-grapheme cap.
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
                        }
                    }
                },
                // Owner: what the chair puts in front of the room — this node on
                // the context's projector (Screen view), and its comments beside it.
                if can_manage {
                    super::widgets::SheetGroup { title: t("common.toolsMeeting"),
                    button {
                        class: "sheet-action",
                        onclick: {
                            let target = node_id.clone();
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
                    // Owner: also show the active node's comments on the Screen.
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
                }
                // Every way of changing this node: its text, and the order of its
                // children. Both happen to be routes to another view rather than
                // actions in place, but that is a fact about the code, not about
                // the choice being made here.
                //
                // Gated on the disjunction of its rows: an empty group would still
                // draw its header.
                if !segments.is_empty()
                    && (can_edit || (is_ctx_owner && (reorderable_children || lockable)))
                {
                    super::widgets::SheetGroup { title: t("common.toolsManage"),
                        if is_ctx_owner && reorderable_children {
                            Link {
                                to: Route::PathPage {
                                    segments: segments.clone(),
                                    app: Some("sort".to_string()),
                                },
                                class: "sheet-action",
                                {icon_el("app/sort")}
                                "{t(\"mime.sort\")}"
                            }
                        }
                        if can_edit {
                            Link {
                                to: Route::PathPage {
                                    segments: segments.clone(),
                                    app: Some("editor".to_string()),
                                },
                                class: "sheet-action",
                                {icon_el("app/editor")}
                                "{t(\"mime.editor\")}"
                            }
                            // Submit without going through the editor: the node is
                            // written already, and submitting only makes it
                            // immutable. Same gate as editing (`can_edit` is owner
                            // AND still mutable), so it disappears once submitted.
                            button {
                                class: "sheet-action",
                                onclick: move |_| confirm_submit.set(true),
                                span { class: "material-icons", "publish" }
                                "{t(\"content.submit\")}"
                            }
                        }
                        // Close (or reopen) what members may add here, the same
                        // lock a folder has over its content.
                        if lockable && is_ctx_owner {
                            button {
                                class: "sheet-action",
                                onclick: {
                                    let id = node_id.clone();
                                    move |_| {
                                        let token = session.read().access_token.clone();
                                        let id = id.clone();
                                        let new_val = !attachable;
                                        attachable_opt.set(Some(new_val));
                                        spawn(async move {
                                            match graphql::update_node(
                                                token.as_deref(),
                                                &id,
                                                crate::model::NodesSetInput {
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
                                                    crate::snackbar::show_snackbar(
                                                        &t("error.somethingWentWrong"),
                                                    );
                                                }
                                            }
                                        });
                                    }
                                },
                                span { class: "material-icons",
                                    if attachable { "lock_open" } else { "lock" }
                                }
                                if attachable {
                                    "{lock_text}"
                                } else {
                                    "{unlock_text}"
                                }
                            }
                        }
                    }
                }
                if can_manage && !segments.is_empty() {
                    super::widgets::SheetGroup { danger: true,
                        button {
                            class: "sheet-action danger",
                            onclick: move |_| confirm_open.set(true),
                            span { class: "material-icons", "delete" }
                            "{t(\"common.delete\")}"
                        }
                    }
                }
            }
            // Submit confirm, carrying the same warning the editor's does: after
            // this the node can no longer be edited.
            if can_edit {
                super::widgets::Dialog {
                    open: confirm_submit(),
                    on_dismiss: move |_| confirm_submit.set(false),
                    headline: t("content.submit"),
                    icon: "publish".to_string(),
                    actions: rsx! {
                        button {
                            class: "btn btn-outlined",
                            onclick: move |_| confirm_submit.set(false),
                            "{t(\"common.cancel\")}"
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: {
                                let id = node_id.clone();
                                move |_| {
                                    confirm_submit.set(false);
                                    let token = session.read().access_token.clone();
                                    let id = id.clone();
                                    spawn(async move {
                                        match graphql::update_node(
                                            token.as_deref(),
                                            &id,
                                            crate::model::NodesSetInput {
                                                mutable: Some(false),
                                                ..Default::default()
                                            },
                                        )
                                        .await
                                        {
                                            Ok(_) => {
                                                crate::session::bump_data_version();
                                                crate::snackbar::show_snackbar(&t(
                                                    "content.submit",
                                                ));
                                            }
                                            Err(e) => {
                                                log::error!("submit failed: {e}");
                                                crate::snackbar::show_snackbar(&t(
                                                    "error.somethingWentWrong",
                                                ));
                                            }
                                        }
                                    });
                                }
                            },
                            "{t(\"content.submit\")}"
                        }
                    },
                    p { class: "body-medium", "{t(\"content.submitWarning\")}" }
                }
            }
            if can_manage && !segments.is_empty() {
                // Delete via the app's standard accessible confirm dialog.
                super::widgets::Dialog {
                    open: confirm_open(),
                    on_dismiss: move |_| confirm_open.set(false),
                    // Deleting is a move to the bin, so the dialog asks for that
                    // rather than for a deletion the app no longer performs.
                    headline: t("content.confirmDeleteBin"),
                    icon: "delete".to_string(),
                    actions: rsx! {
                        button {
                            class: "btn btn-outlined",
                            onclick: move |_| confirm_open.set(false),
                            "{t(\"common.cancel\")}"
                        }
                        button {
                            class: "btn btn-primary",
                            disabled: deleting(),
                            onclick: {
                                let node_id = node_id.clone();
                                let parent = segments[..segments.len() - 1].to_vec();
                                let node_path = node_path.clone();
                                let actor = actor.clone();
                                move |_| {
                                    if deleting() {
                                        return;
                                    }
                                    let token = session.read().access_token.clone();
                                    let node_id = node_id.clone();
                                    let parent = parent.clone();
                                    let node_path = node_path.clone();
                                    let actor = actor.clone();
                                    // The dialog stays open so the spinner has
                                    // somewhere to be.
                                    deleting.set(true);
                                    spawn(async move {
                                        // To the bin, not out of existence: one
                                        // statement stamps this node and everything
                                        // under it (found by path prefix), and the
                                        // context's bin can put the lot back. The
                                        // old deep delete walked the subtree a
                                        // request at a time and was final.
                                        match graphql::bin_node(
                                            token.as_deref(),
                                            &node_id,
                                            node_path.as_deref(),
                                            actor.as_deref(),
                                        )
                                        .await
                                        {
                                            Ok(_) => {
                                                crate::session::bump_data_version();
                                                deleting.set(false);
                                                confirm_open.set(false);
                                                nav.push(Route::PathPage {
                                                    segments: parent,
                                                    app: None,
                                                });
                                            }
                                            other => {
                                                log::error!("delete_node failed: {other:?}");
                                                deleting.set(false);
                                                crate::snackbar::show_snackbar(&t(
                                                    "error.somethingWentWrong",
                                                ));
                                            }
                                        }
                                    });
                                }
                            },
                            if deleting() {
                                div { class: "spinner spinner-xs" }
                            }
                            "{t(\"common.delete\")}"
                        }
                    },
                    p { class: "body-medium", "{name}" }
                    p { class: "body-medium text-muted", "{t(\"content.deleteRecoverableTree\")}" }
                }
            }
            // Author chips (the document's members), mirroring MemberChips.
            if !members.is_empty() {
                div { class: "chip-row chip-row-authors",
                    for member in members.iter() {
                        // An author is either a person or a group. A person opens
                        // the identity popover; a group is a place in the wiki, so
                        // its chip goes there — it used to open a popover with no
                        // identity behind it and nowhere to go.
                        if member.user.is_none() && member.node.is_some() {
                            GroupAuthorChip {
                                key: "{member.id.0}",
                                node_id: member.node_id.as_ref().map(|n| n.0.clone()).unwrap_or_default(),
                                label: member.label(),
                                mime: member.node.as_ref().and_then(|n| n.mime_id.clone()).unwrap_or_default(),
                            }
                        } else {
                            super::loader::UserPopover {
                                key: "{member.id.0}",
                                name: member.label(),
                                avatar_url: member.user.as_ref().map(|u| u.avatar_url.clone()).unwrap_or_default(),
                                user_id: member.user.as_ref().map(|u| u.id.0.clone()),
                                super::widgets::Chip {
                                    icon: mime_icon(member.node.as_ref().and_then(|n| n.mime_id.as_deref()).unwrap_or("wiki/user")).to_string(),
                                    label: member.label(),
                                    title: t("member.author"),
                                    // The author's profile picture (e.g. their linked
                                    // Bluesky avatar) shows on the chip itself.
                                    avatar_url: member.user.as_ref().map(|u| u.avatar_url.clone()),
                                }
                            }
                        }
                    }
                }
            }
            // Render the body when there is one; otherwise a compact orb empty
            // state (matching FileApp/FolderApp) instead of a bare empty paragraph
            // — unless a section below carries the page, in which case neither the
            // empty state nor its padding has anything to say.
            if has_rich_content(data.as_ref()) {
                div { class: "card-content", SlateRenderer { data: data.clone() } }
            } else if extra.is_none() {
                div { class: "card-content",
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "description" }
                        }
                        p { class: "empty-state-body", "{t(\"common.noContent\")}" }
                    }
                }
            }
            if let Some(extra) = extra.clone() {
                {extra}
            }
        }
    }
}

/// Whether a node's `data` carries non-empty rich-text content (so a folder or
/// context can decide whether to show its description).
pub fn has_rich_content(data: Option<&serde_json::Value>) -> bool {
    data.and_then(|d| d.get("content"))
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .any(|b| !block_plain_text(b).trim().is_empty())
        })
        .unwrap_or(false)
}

/// Renders Slate.js JSON content as HTML
#[component]
pub fn SlateRenderer(data: Option<serde_json::Value>) -> Element {
    let content = data
        .as_ref()
        .and_then(|d| d.get("content"))
        .or(data.as_ref());

    match content {
        Some(serde_json::Value::Array(blocks)) => {
            rsx! {
                div { class: "slate-content",
                    for (i , block) in blocks.iter().enumerate() {
                        SlateBlock { key: "{i}", index: i, block: block.clone() }
                    }
                }
            }
        }
        _ => {
            rsx! {
                div { class: "slate-content",
                    p { class: "body-medium",
                        class: "text-muted",
                        ""
                    }
                }
            }
        }
    }
}

/// Heading depth (1..6) for a Slate block type, or None if it is not a heading.
fn heading_level(block_type: &str) -> Option<u8> {
    match block_type {
        "heading-one" | "h1" => Some(1),
        "heading-two" | "h2" => Some(2),
        "heading-three" | "h3" => Some(3),
        "heading-four" | "h4" => Some(4),
        "heading-five" | "h5" => Some(5),
        "heading-six" | "h6" => Some(6),
        _ => None,
    }
}

/// Flatten a whole Slate document to plain text: every block's leaf text, one
/// block per line. Accepts either the node `data` object (`{ "content": [...] }`),
/// a bare blocks array, or a single block. Used by the amendment diff to compare
/// a motion and an amendment as text.
pub(crate) fn slate_plain_text(data: &serde_json::Value) -> String {
    let blocks = data
        .get("content")
        .and_then(|c| c.as_array())
        .or_else(|| data.as_array());
    match blocks {
        Some(arr) => arr
            .iter()
            .map(block_plain_text)
            .collect::<Vec<_>>()
            .join("\n"),
        None => block_plain_text(data),
    }
}

/// All leaf text of a block, concatenated (for the TOC label / anchor).
fn block_plain_text(block: &serde_json::Value) -> String {
    fn collect(v: &serde_json::Value, out: &mut String) {
        if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
            out.push_str(t);
        }
        if let Some(children) = v.get("children").and_then(|c| c.as_array()) {
            for c in children {
                collect(c, out);
            }
        }
    }
    let mut s = String::new();
    collect(block, &mut s);
    s
}

/// A unique, stable anchor id for the heading at block `index` — slug of its
/// text prefixed with the index so duplicate headings do not collide.
fn heading_anchor(index: usize, text: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in text.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    format!("h{index}-{slug}")
}

/// The headings in a node's Slate `content`, as (anchor id, text, level), using
/// the SAME index-based anchor the renderer applies — so a presenter's picks
/// match the projector's heading ids. Powers the projector section-focus control.
pub(crate) fn content_headings(data: Option<&serde_json::Value>) -> Vec<(String, String, u8)> {
    let Some(blocks) = data
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_array())
    else {
        return Vec::new();
    };
    blocks
        .iter()
        .enumerate()
        .filter_map(|(i, b)| {
            let level = heading_level(b.get("type").and_then(|t| t.as_str())?)?;
            let raw = block_plain_text(b);
            if raw.trim().is_empty() {
                return None;
            }
            Some((heading_anchor(i, &raw), raw.trim().to_string(), level))
        })
        .collect()
}

#[component]
fn SlateBlock(block: serde_json::Value, index: usize) -> Element {
    let block_type = block
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("paragraph");
    let children = block
        .get("children")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let rendered_children = rsx! {
        for (i , child) in children.iter().enumerate() {
            SlateInline { key: "{i}", node: child.clone() }
        }
    };

    // Heading anchor id so the table of contents can link to it (#117).
    let hid = heading_level(block_type)
        .map(|_| heading_anchor(index, &block_plain_text(&block)))
        .unwrap_or_default();

    // Per-block alignment set in the editor.
    let astyle = match block.get("align").and_then(|a| a.as_str()) {
        Some(a @ ("center" | "right" | "justify" | "left")) => format!("text-align:{a}"),
        _ => String::new(),
    };

    match block_type {
        "heading-one" | "h1" => rsx! { h1 { id: "{hid}", style: "{astyle}", {rendered_children} } },
        "heading-two" | "h2" => rsx! { h2 { id: "{hid}", style: "{astyle}", {rendered_children} } },
        "heading-three" | "h3" => {
            rsx! { h3 { id: "{hid}", style: "{astyle}", {rendered_children} } }
        }
        "heading-four" | "h4" => {
            rsx! { h4 { id: "{hid}", style: "{astyle}", {rendered_children} } }
        }
        "heading-five" | "h5" => {
            rsx! { h5 { id: "{hid}", style: "{astyle}", {rendered_children} } }
        }
        "heading-six" | "h6" => rsx! { h6 { id: "{hid}", style: "{astyle}", {rendered_children} } },
        "block-quote" => rsx! { blockquote { style: "{astyle}", {rendered_children} } },
        // Only the explicit `block-pre` type is a code block. `code` here would be a
        // legacy/imported block type that the old wiki rendered as a normal paragraph
        // (its default case), so let it fall through. Otherwise prose stored as
        // `type: "code"` renders in a non-wrapping monospace <pre> and overflows.
        "block-pre" => rsx! { pre { style: "{astyle}", {rendered_children} } },
        "bulleted-list" | "ul" => rsx! { ul { style: "{astyle}", {rendered_children} } },
        "numbered-list" | "ol" => rsx! { ol { style: "{astyle}", {rendered_children} } },
        "list-item" | "li" => rsx! { li { style: "{astyle}", {rendered_children} } },
        "image" => {
            let url = block.get("url").and_then(|u| u.as_str()).unwrap_or("");
            // Emoji pasted from Facebook/Messenger arrive as one `image` block each,
            // pointing at a codepoint-named PNG. Rendered as block images they stack
            // one-per-line; recover the emoji and render it inline so a run flows with
            // the text.
            if let Some(emoji) = emoji_from_image_url(url) {
                rsx! { span { class: "content-emoji", "{emoji}" } }
            } else {
                rsx! {
                    super::widgets::ZoomableImage { src: url.to_string(), alt: "content image".to_string() }
                }
            }
        }
        _ => rsx! { p { style: "{astyle}", {rendered_children} } },
    }
}

#[component]
fn SlateInline(node: serde_json::Value) -> Element {
    // Leaf text node
    if let Some(text) = node.get("text").and_then(|t| t.as_str()) {
        let bold = node.get("bold").and_then(|b| b.as_bool()).unwrap_or(false);
        let italic = node
            .get("italic")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let underline = node
            .get("underline")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let strikethrough = node
            .get("strikethrough")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let code = node.get("code").and_then(|b| b.as_bool()).unwrap_or(false);

        let mut style_parts = Vec::new();
        if bold {
            style_parts.push("font-weight: bold");
        }
        if italic {
            style_parts.push("font-style: italic");
        }
        if underline && strikethrough {
            style_parts.push("text-decoration: underline line-through");
        } else if underline {
            style_parts.push("text-decoration: underline");
        } else if strikethrough {
            style_parts.push("text-decoration: line-through");
        }

        let style = style_parts.join("; ");

        // An explicit link mark turns the whole leaf into an anchor (matching the
        // editor, which stores links as a leaf mark rather than an element).
        if let Some(url) = node
            .get("link")
            .and_then(|l| l.as_str())
            .filter(|l| !l.is_empty())
        {
            let url = safe_href(url);
            return rsx! {
                a { href: "{url}", target: "_blank", rel: "noopener noreferrer",
                    if code {
                        code { "{text}" }
                    } else if style.is_empty() {
                        "{text}"
                    } else {
                        span { style: "{style}", "{text}" }
                    }
                }
            };
        }

        if code {
            return rsx! {
                code { "{text}" }
            };
        }

        if style.is_empty() {
            return rsx! {
                AutoLinked { text: text.to_string() }
            };
        }

        return rsx! {
            span { style: "{style}", AutoLinked { text: text.to_string() } }
        };
    }

    // Inline element (link, etc.)
    if let Some(element_type) = node.get("type").and_then(|t| t.as_str()) {
        let children = node
            .get("children")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        match element_type {
            "link" => {
                let url = safe_href(node.get("url").and_then(|u| u.as_str()).unwrap_or("#"));
                return rsx! {
                    a { href: "{url}", target: "_blank", rel: "noopener noreferrer",
                        for (i , child) in children.iter().enumerate() {
                            SlateInline { key: "{i}", node: child.clone() }
                        }
                    }
                };
            }
            "list-item" | "li" => {
                return rsx! {
                    li {
                        for (i , child) in children.iter().enumerate() {
                            SlateInline { key: "{i}", node: child.clone() }
                        }
                    }
                };
            }
            _ => {
                return rsx! {
                    span {
                        for (i , child) in children.iter().enumerate() {
                            SlateInline { key: "{i}", node: child.clone() }
                        }
                    }
                };
            }
        }
    }

    rsx! {}
}

/// A plain-text run with bare URLs and email addresses turned into links (#97),
/// and with the author's own line breaks kept.
///
/// A shift-enter inside a paragraph is stored as a newline in the text run (see
/// richtext's serializer, which turns a `<br>` into one). HTML collapses that to
/// a space, so an address block or a motion's preamble came back as one running
/// line — the break survived the editor, the save and the round trip, and died
/// at the last step.
#[component]
pub(crate) fn AutoLinked(text: String) -> Element {
    rsx! {
        for (line_no , line) in text.split('\n').enumerate() {
            if line_no > 0 {
                br {}
            }
            AutoLinkedLine { key: "{line_no}", text: line.to_string() }
        }
    }
}

/// One line of a text run: the links in it, and nothing about breaks.
#[component]
fn AutoLinkedLine(text: String) -> Element {
    rsx! {
        for (i , token) in autolink_tokens(&text).into_iter().enumerate() {
            match token {
                LinkToken::Text(s) => rsx! { "{s}" },
                LinkToken::Url(url, trail) => rsx! {
                    a { key: "{i}", href: "{url}", target: "_blank", rel: "noopener", "{url}" }
                    "{trail}"
                },
                LinkToken::Email(addr, trail) => rsx! {
                    a { key: "{i}", href: "mailto:{addr}", "{addr}" }
                    "{trail}"
                },
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum LinkToken {
    Text(String),
    Url(String, String),
    Email(String, String),
}

/// Split a text run into plain / URL / email tokens, keeping spacing. URLs must
/// start http(s)://; emails are `local@domain.tld`. Trailing punctuation is kept
/// out of the link target.
fn autolink_tokens(text: &str) -> Vec<LinkToken> {
    let mut out = Vec::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(LinkToken::Text(" ".to_string()));
        }
        if word.is_empty() {
            continue;
        }
        let end = word.trim_end_matches(|c: char| ".,;:!?)]}'\"".contains(c));
        let trail = word[end.len()..].to_string();
        if end.starts_with("http://") || end.starts_with("https://") {
            out.push(LinkToken::Url(end.to_string(), trail));
        } else if is_email(end) {
            out.push(LinkToken::Email(end.to_string(), trail));
        } else {
            out.push(LinkToken::Text(word.to_string()));
        }
    }
    out
}

fn is_email(w: &str) -> bool {
    let mut parts = w.splitn(2, '@');
    match (parts.next(), parts.next()) {
        (Some(local), Some(domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
        }
        _ => false,
    }
}

/// An author chip for a GROUP, which navigates to that group.
///
/// The path is resolved on CLICK, not while rendering. Nodes are addressed by
/// path and there is no id route, so the group's ancestors have to be walked —
/// one request per level. A page can carry several author chips and most are
/// never followed, so paying for that up front would be a request per chip per
/// page view.
#[component]
fn GroupAuthorChip(node_id: String, label: String, mime: String) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let mut going = use_signal(|| false);

    rsx! {
        button {
            class: "chip-button",
            r#type: "button",
            title: t("member.author"),
            disabled: *going.read() || node_id.is_empty(),
            onclick: {
                let node_id = node_id.clone();
                move |_| {
                    let token = session.read().access_token.clone();
                    let node_id = node_id.clone();
                    going.set(true);
                    spawn(async move {
                        let segments =
                            crate::graphql::node_path(token.as_deref(), &node_id).await;
                        going.set(false);
                        if segments.is_empty() {
                            // Nothing to navigate to: the group is not readable,
                            // or its path does not resolve. Better to say so than
                            // to send the reader to a 404.
                            crate::snackbar::show_snackbar(&t("node.notFoundOrNoAccess"));
                            return;
                        }
                        nav.push(Route::PathPage { segments, app: None });
                    });
                }
            },
            super::widgets::Chip {
                icon: mime_icon(&mime).to_string(),
                label: label.clone(),
                title: t("member.author"),
            }
        }
    }
}

/// Recover a Unicode emoji from an emoji-image URL. Editors that paste from
/// Facebook/Messenger store each emoji as its own `image` block pointing at a
/// codepoint-named PNG (e.g. `…/emoji.php/v9/…/1fa77.png`, or `1f469_200d_1f467.png`
/// for ZWJ sequences). Rendered as block images they stack one-per-line, so we turn
/// them back into the inline emoji character. Returns None for ordinary images: it
/// only fires when the URL is clearly an emoji one AND every filename segment parses
/// as a codepoint.
pub(crate) fn emoji_from_image_url(url: &str) -> Option<String> {
    if !url.to_ascii_lowercase().contains("emoji") {
        return None;
    }
    let file = url.rsplit('/').next()?;
    let stem = file.split(['.', '?', '#']).next()?;
    let mut emoji = String::new();
    for part in stem.split(['_', '-']) {
        let cp = u32::from_str_radix(part, 16).ok()?;
        emoji.push(char::from_u32(cp)?);
    }
    (!emoji.is_empty()).then_some(emoji)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn heading_anchor_is_stable_and_slugified() {
        // Anchor = block index + a slug of the text, so duplicate headings at
        // different positions still get distinct, stable ids.
        assert_eq!(heading_anchor(0, "Intro"), "h0-intro");
        assert_eq!(heading_anchor(2, "Intro"), "h2-intro");
        assert_eq!(heading_anchor(1, "  Hello, World!  "), "h1-hello-world");
    }

    #[test]
    fn safe_href_allows_web_and_app_urls_and_blocks_scripts() {
        // Allowed schemes pass through unchanged (trimmed).
        assert_eq!(safe_href("https://example.com/x"), "https://example.com/x");
        assert_eq!(safe_href("  http://a.b  "), "http://a.b");
        assert_eq!(safe_href("mailto:a@b.dk"), "mailto:a@b.dk");
        assert_eq!(safe_href("/group/doc"), "/group/doc");
        assert_eq!(safe_href("#section"), "#section");
        // Dangerous schemes (any casing / whitespace) are neutralized to "#".
        assert_eq!(safe_href("javascript:alert(1)"), "#");
        assert_eq!(safe_href("JavaScript:alert(1)"), "#");
        assert_eq!(safe_href("  javascript:alert(1)"), "#");
        assert_eq!(safe_href("data:text/html,<script>"), "#");
        assert_eq!(safe_href("vbscript:msgbox"), "#");
    }

    #[test]
    fn recovers_emoji_from_facebook_image_urls() {
        // Facebook single-codepoint and ZWJ-sequence emoji images.
        assert_eq!(
            emoji_from_image_url(
                "https://static.xx.fbcdn.net/images/emoji.php/v9/t99/1/16/1fa77.png"
            )
            .as_deref(),
            Some("\u{1fa77}")
        );
        assert_eq!(
            emoji_from_image_url("https://x/emoji.php/v9/x/1f469_200d_1f467.png").as_deref(),
            Some("\u{1f469}\u{200d}\u{1f467}")
        );
        // Ordinary content images are left alone (rendered as images).
        assert_eq!(
            emoji_from_image_url("https://cdn.example.com/photo.jpg"),
            None
        );
        assert_eq!(
            emoji_from_image_url("https://cdn.example.com/emoji-banner.png"),
            None
        );
    }

    #[test]
    fn autolink_detects_url_and_email() {
        let toks = autolink_tokens("see https://x.org, mail me@x.dk!");
        assert!(toks.contains(&LinkToken::Url("https://x.org".into(), ",".into())));
        assert!(toks.contains(&LinkToken::Email("me@x.dk".into(), "!".into())));
        // Plain words stay text.
        assert!(toks.contains(&LinkToken::Text("see".into())));
        assert!(!is_email("not-an-email"));
        assert!(!is_email("a@b"));
        assert!(is_email("a@b.dk"));
    }

    /// Only http(s) is linked, which is what keeps this safe to run over text
    /// anyone can write. A comment is now autolinked too, so a `javascript:`
    /// URL typed into one must stay a word — the renderer's `safe_href` is the
    /// second line, not the first.
    #[test]
    fn autolink_never_makes_a_link_out_of_a_scheme_it_should_not() {
        for dangerous in [
            "javascript:alert(1)",
            "data:text/html,<script>alert(1)</script>",
            "vbscript:msgbox",
            "file:///etc/passwd",
        ] {
            let toks = autolink_tokens(dangerous);
            assert!(
                !toks.iter().any(|t| matches!(t, LinkToken::Url(..))),
                "{dangerous} must stay text: {toks:?}"
            );
        }
        assert_eq!(safe_href("javascript:alert(1)"), "#");
    }

    /// The rendered comment splits on newlines before tokenising, so a run never
    /// carries one — but a URL at the end of a line still has to keep its own
    /// trailing punctuation out of the href.
    #[test]
    fn a_url_keeps_its_sentence_punctuation_outside_the_link() {
        let toks = autolink_tokens("read https://radikal.wiki/hb1.");
        assert!(
            toks.contains(&LinkToken::Url(
                "https://radikal.wiki/hb1".into(),
                ".".into()
            )),
            "{toks:?}"
        );
        // A path that genuinely ends in a slash keeps it.
        let toks = autolink_tokens("see https://radikal.wiki/hb1/ please");
        assert!(
            toks.contains(&LinkToken::Url(
                "https://radikal.wiki/hb1/".into(),
                String::new()
            )),
            "{toks:?}"
        );
    }
}
