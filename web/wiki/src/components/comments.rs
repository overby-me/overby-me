//! Nested comment threads (Bluesky-style). Comments are `vote/comment` nodes
//! whose parent is the post (or another comment for a reply); each thread level
//! lazily fetches its own children, so nesting is unbounded. The design is a
//! modern threaded view: an avatar rail, author + relative time, the text, and a
//! reply affordance, with replies indented under their parent.

use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::{self, ChildNodeFields};
use crate::session::use_session;

use super::loader::relative_time;

/// The comment text stored on a `vote/comment` node (`data.text`).
fn comment_text(comment: &ChildNodeFields) -> String {
    comment
        .data
        .as_ref()
        .and_then(|d| d.0.get("text"))
        .and_then(|t| t.as_str())
        .unwrap_or("")
        .to_string()
}

/// How large an attached image may be.
///
/// Ten megabytes is roughly a phone photo straight from the camera; the point is
/// to stop a video renamed to .jpg, not to make people compress a screenshot
/// before answering a motion.
const MAX_COMMENT_IMAGE_BYTES: usize = 10 * 1024 * 1024;

/// Whether a picked file may be attached, or the key of the reason it may not.
///
/// Pure, so the rules can be tested without a browser — and worth stating
/// server-shaped rather than trusting `accept="image/*"`, which is a hint the
/// file picker may ignore.
fn image_rejection(content_type: &str, size: usize) -> Option<&'static str> {
    if !content_type.starts_with("image/") {
        return Some("vote.imageNotAnImage");
    }
    if size > MAX_COMMENT_IMAGE_BYTES {
        return Some("vote.imageTooLarge");
    }
    None
}

/// The image attached to a comment (`data.image`), if any.
fn comment_image(comment: &ChildNodeFields) -> Option<String> {
    comment
        .data
        .as_ref()
        .and_then(|d| d.0.get("image"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// A blob URL for bytes already in the browser, for showing a picked image
/// before it exists anywhere else.
fn object_url(bytes: &[u8], content_type: &str) -> Option<String> {
    let array = js_sys::Uint8Array::from(bytes);
    let parts = js_sys::Array::new();
    parts.push(&array.buffer());
    let mut opts = web_sys::BlobPropertyBag::new();
    opts.set_type(content_type);
    let blob = web_sys::Blob::new_with_u8_array_sequence_and_options(&parts, &opts).ok()?;
    web_sys::Url::create_object_url_with_blob(&blob).ok()
}

/// An attached image, fetched with the session token into a blob URL so the JWT
/// never enters an `<img src>`, and opening full size through the same lightbox
/// a document's image uses.
#[component]
fn CommentImage(file_id: String) -> Element {
    let url = super::loader::use_file_object_url(file_id);
    rsx! {
        if let Some(src) = url {
            div { class: "comment-image",
                super::widgets::ZoomableImage { src, alt: t("vote.imageAlt") }
            }
        }
    }
}

/// A comment shown optimistically before the server confirms it. Reconciled by
/// `key`: once the refetch returns a comment with the same key, the pending row is
/// dropped — no duplicate, no flicker.
#[derive(Clone, PartialEq)]
struct PendingComment {
    key: String,
    author: String,
    text: String,
    /// The blob URL of the picked image, handed over from the composer when the
    /// comment was posted. NOT the stored file id: until the insert lands there
    /// is no node pointing at the file, so fetching it back would show nothing.
    /// The row owns this URL and revokes it when it goes away.
    image: Option<String>,
}

/// Reconcile optimistic comments against the fetched set: keep only the pending
/// entries whose `key` has NOT yet come back from the server. Once the refetch
/// includes a comment with the same key, its optimistic row is dropped — no
/// duplicate, no flicker.
fn reconcile_pending(
    pending: &[PendingComment],
    fetched_keys: &std::collections::HashSet<String>,
) -> Vec<PendingComment> {
    pending
        .iter()
        .filter(|p| !fetched_keys.contains(&p.key))
        .cloned()
        .collect()
}

/// An optimistic (not-yet-confirmed) comment row at `depth`, muted with a
/// "sending" marker.
#[component]
fn PendingRow(author: String, text: String, image: Option<String>, depth: usize) -> Element {
    {
        // The composer handed this URL over rather than revoking it; this row is
        // its owner now, and it lives exactly as long as the row does.
        let owned = image.clone();
        use_drop(move || {
            if let Some(url) = owned.as_ref() {
                let _ = web_sys::Url::revoke_object_url(url);
            }
        });
    }
    let initial = author
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let indent = depth.min(6) as f32 * 1.5;
    rsx! {
        div { class: "comment comment-pending", style: "margin-left: {indent}rem;",
            div { class: "comment-main",
                div { class: "avatar small comment-avatar", "{initial}" }
                div { class: "comment-body",
                    div { class: "comment-meta",
                        span { class: "comment-author", "{author}" }
                        span { class: "comment-dot", "·" }
                        span { class: "comment-time", "{t(\"vote.sending\")}" }
                    }
                    p { class: "comment-text",
                        // A comment is where people actually paste links: the
                        // motion they are answering, the article they are citing.
                        super::content::AutoLinked { text: text.clone() }
                    }
                    if let Some(src) = image.clone() {
                        div { class: "comment-image",
                            img { src: "{src}", alt: "{t(\"vote.imageAlt\")}" }
                        }
                    }
                }
            }
        }
    }
}

/// The comment section for a post: a composer and the top-level threads.
#[component]
pub fn CommentSection(node_id: String, context_id: Option<String>) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let is_auth = session.read().is_authenticated();
    // Bumped after any post so every thread refetches (scoped to comments, so it
    // does not refetch the whole post like the global data version would).
    let refresh = use_signal(|| 0u32);

    let nid = node_id.clone();
    let rev = refresh();
    // Refetches on a local post (`rev`) and on a global refresh (pull-to-refresh),
    // so comments update alongside the rest of the view.
    // LEE: keep the error so a failed load shows an error state, not "no comments".
    let comments = crate::use_data_resource!(|(nid, token, rev)| async move {
        let _ = rev;
        graphql::query_comments(token.as_deref(), &nid).await
    });
    let state = comments.read().clone();

    // Live: refetch when any comment/reply in this context (or, lacking a context,
    // directly under this node) changes, so new comments and replies appear at
    // once — and drive the "someone replied to you" notification below.
    {
        let filter = match &context_id {
            Some(ctx) => format!(
                "contextId: {{ _eq: \"{}\" }}",
                crate::graphql::gql_escape(ctx)
            ),
            None => format!(
                "parentId: {{ _eq: \"{}\" }}",
                crate::graphql::gql_escape(&node_id)
            ),
        };
        crate::subscription::use_live(
            format!(
                "subscription {{ nodes(where: {{ {filter}, mimeId: {{ _eq: \"vote/comment\" }} }}) {{ id }} }}"
            ),
            refresh,
        );
    }

    // Whether the current user may comment here: gate the composer on the node's
    // `inserts` (allowed child mimes), matching the old wiki's AddCommentButton.
    // Within a context this also governs replies, since comment nodes share the
    // context's permission, so the post's verdict is passed down to every thread.
    let nid2 = node_id.clone();
    let tok2 = session.read().access_token.clone();
    let can_comment_res = crate::use_data_resource!(|(nid2, tok2)| async move {
        if tok2.is_none() {
            return false;
        }
        graphql::node_insert_mimes(tok2.as_deref(), &nid2)
            .await
            .iter()
            .any(|m| m == "vote/comment")
    });
    let can_comment = (*can_comment_res.read()).unwrap_or(false);

    // Optimistic comments: shown before the server confirms, reconciled by key —
    // an entry is hidden once the refetch returns a comment with the same key.
    let mut pending = use_signal(Vec::<PendingComment>::new);
    // Tied to the node it was posted under. This component is reused across a
    // route change rather than remounted, so navigating away with a post still in
    // flight would carry the muted row onto the next node, where its key can
    // never come back from the server and so never reconciles away. Same defect
    // FolderApp had, where adding a folder navigates into it every time.
    let shown_node = node_id.clone();
    use_effect(use_reactive!(|(shown_node)| {
        let _ = &shown_node;
        if !pending.peek().is_empty() {
            pending.set(Vec::new());
        }
    }));
    let fetched_keys: std::collections::HashSet<String> = match &state {
        Some(Ok(list)) => list.iter().map(|c| c.key.clone()).collect(),
        _ => Default::default(),
    };
    let pending_shown = reconcile_pending(&pending.read(), &fetched_keys);

    rsx! {
        div { class: "card comment-section",
            div { class: "card-header",
                div { class: "avatar small", {crate::components::loader::icon_el("vote/comment")} }
                h3 { class: "title-medium", "{t(\"vote.comments\")}" }
            }
            div { class: "card-content",
                if is_auth && can_comment {
                    CommentComposer {
                        parent_id: node_id.clone(),
                        context_id: context_id.clone(),
                        refresh,
                        pending,
                        placeholder: t("vote.newComment"),
                        on_posted: move |_| {},
                    }
                }
                // Optimistic rows for top-level comments still in flight.
                if !pending_shown.is_empty() {
                    div { class: "comment-thread-list",
                        for p in pending_shown.iter() {
                            PendingRow {
                                key: "{p.key}",
                                author: p.author.clone(),
                                text: p.text.clone(),
                                image: p.image.clone(),
                                depth: 0,
                            }
                        }
                    }
                }
                match &state {
                    // Loading: a spinner rather than a premature "no comments".
                    None => rsx! {
                        div { class: "empty-state empty-state-sm",
                            crate::components::widgets::Spinner {}
                        }
                    },
                    // Error: never looks like an empty thread (log the detail).
                    Some(Err(e)) => {
                        log::error!("Loading comments failed: {e}");
                        rsx! {
                            crate::components::widgets::ErrorState {
                                title: t("error.somethingWentWrong"),
                                small: true,
                            }
                        }
                    }
                    Some(Ok(list)) if list.is_empty() => rsx! {
                        // DESIGN: a compact characterful empty state (floating orb).
                        div { class: "empty-state empty-state-sm",
                            div { class: "empty-state-orb empty-state-orb-sm",
                                span { class: "material-icons", "forum" }
                            }
                            p { class: "empty-state-body", "{t(\"vote.noComments\")}" }
                        }
                    },
                    Some(Ok(list)) => rsx! {
                        div { class: "comment-thread-list",
                            // Newest first, at the TOP level only. What was just
                            // said about a motion is what the room is discussing,
                            // and it should not be at the bottom of forty older
                            // remarks. Replies keep their order (see CommentThread):
                            // an answer read before the thing it answers is not a
                            // conversation, it is a puzzle.
                            for c in list.iter().rev() {
                                CommentThread {
                                    key: "{c.id.0}",
                                    comment: c.clone(),
                                    context_id: context_id.clone(),
                                    depth: 0,
                                    refresh,
                                    can_comment,
                                }
                            }
                        }
                    },
                }
            }
        }
    }
}

/// Bin a comment and everything under it: replies, and the reactions on those.
///
/// Stamped in one statement by path prefix, so a thread goes as a thread and
/// comes back as one. Deleting used to be final here, which made a mis-click on
/// someone else's argument unrecoverable — the one place in the app where that
/// was true of writing rather than of a container.
async fn delete_comment_subtree(
    token: Option<String>,
    root: String,
    actor: Option<String>,
) -> Result<(), String> {
    graphql::bin_node(token.as_deref(), &root, None, actor.as_deref())
        .await
        .map(|_| ())
}

/// Whether a comment has been emptied rather than removed (see
/// [`tombstone_comment`]).
fn is_tombstone(comment: &ChildNodeFields) -> bool {
    comment
        .data
        .as_ref()
        .and_then(|d| d.0.get("deleted"))
        .and_then(serde_json::Value::as_bool)
        .unwrap_or(false)
}

/// Empty a comment in place, keeping the row.
///
/// Deleting a comment used to take its whole subthread, so one person changing
/// their mind erased everyone who had answered them. A comment with replies is
/// load-bearing: it is what the answers hang from. So the words go and the
/// hanger stays, and the thread keeps its shape.
///
/// What goes: the text, the author's name, and the reactions, which reacted to
/// something that no longer says anything. What stays: the row, its position,
/// and its replies.
///
/// The name is what a comment carries its author in, so blanking it is the scrub
/// that matters here. `owner_id` still holds the account, since the update input
/// cannot send an explicit null; nothing renders it for a tombstone, but a real
/// scrub of that column needs a mutation that can (worth doing when the bin
/// lands).
async fn tombstone_comment(token: Option<String>, id: String) -> Result<(), String> {
    // Reactions first: if the update fails, the comment is still whole, whereas
    // the reverse would leave a live comment stripped of its reactions.
    for reaction in graphql::query_reactions(token.as_deref(), &id)
        .await
        .unwrap_or_default()
    {
        let _ = graphql::delete_node(token.as_deref(), &reaction.id.0).await;
    }
    let set = model::NodesSetInput {
        name: Some(String::new()),
        data: Some(model::Jsonb(serde_json::json!({ "deleted": true }))),
        ..Default::default()
    };
    graphql::update_node(token.as_deref(), &id, set).await?;
    Ok(())
}

/// One comment and its nested replies (recursive; each level fetches its own
/// `vote/comment` children).
#[component]
fn CommentThread(
    comment: ChildNodeFields,
    context_id: Option<String>,
    depth: usize,
    refresh: Signal<u32>,
    can_comment: bool,
) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let is_auth = session.read().is_authenticated();
    let current_user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let mut replying = use_signal(|| false);
    // The author or a context owner may delete a comment (and its replies).
    let can_del = comment.is_owner.unwrap_or(false) || comment.is_context_owner.unwrap_or(false);
    let mut del_confirm = use_signal(|| false);

    let cid = comment.id.0.clone();
    let rev = refresh();
    let replies_res = crate::use_data_resource!(|(cid, token, rev)| async move {
        let _ = rev;
        graphql::query_comments(token.as_deref(), &cid)
            .await
            .unwrap_or_default()
    });
    let replies = replies_res.read().clone().unwrap_or_default();
    // Optimistic replies — same reconcile-by-key pattern as top-level comments.
    let reply_pending = use_signal(Vec::<PendingComment>::new);
    let reply_keys: std::collections::HashSet<String> =
        replies.iter().map(|c| c.key.clone()).collect();
    let reply_pending_shown = reconcile_pending(&reply_pending.read(), &reply_keys);

    // Notify me when a new reply lands on a comment I wrote — not my own replies,
    // and not on first load. Only shows if notification permission was granted
    // (requested when you post a comment). Live comments (see CommentSection)
    // make this fire in real time.
    let mut seen_replies = use_signal(|| None::<std::collections::HashSet<String>>);
    {
        let mine = current_user_id.is_some()
            && comment.owner_id.as_ref().map(|o| o.0.clone()) == current_user_id;
        let snapshot: Vec<(String, bool)> = replies
            .iter()
            .map(|r| {
                let from_other = r.owner_id.as_ref().map(|o| o.0.clone()) != current_user_id;
                (r.id.0.clone(), from_other)
            })
            .collect();
        let sig = snapshot
            .iter()
            .map(|(id, _)| id.clone())
            .collect::<Vec<_>>()
            .join(",");
        use_effect(use_reactive!(|(sig, mine, snapshot)| {
            let _ = &sig;
            let ids: std::collections::HashSet<String> =
                snapshot.iter().map(|(id, _)| id.clone()).collect();
            // Clone out of the signal first so its read guard is released before
            // we write back below.
            let previous = seen_replies.peek().clone();
            match previous {
                // First load: remember what's here without notifying.
                None => seen_replies.set(Some(ids)),
                Some(prev) => {
                    if mine
                        && snapshot
                            .iter()
                            .any(|(id, other)| *other && !prev.contains(id))
                    {
                        crate::pwa::notify(&t("vote.replyNotifyTitle"), &t("vote.replyNotifyBody"));
                    }
                    seen_replies.set(Some(ids));
                }
            }
        }));
    }

    // An emptied comment shows as one: no author, no text, no reactions, just
    // the hanger its replies are on.
    let deleted = is_tombstone(&comment);
    let author = if deleted {
        t("vote.commentDeleted")
    } else if comment.name.trim().is_empty() {
        t("common.unknown")
    } else {
        comment.name.clone()
    };
    let initial = author
        .chars()
        .next()
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_default();
    let avatar_url = comment
        .owner
        .as_ref()
        .map(|o| o.avatar_url.clone())
        .unwrap_or_default();
    // The comment author's user id (for the identity popover); None for legacy
    // free-text authors with no linked account.
    let author_id = comment.owner.as_ref().map(|o| o.id.0.clone());
    let text = comment_text(&comment);
    let image = comment_image(&comment);
    let when = comment
        .created_at
        .as_ref()
        .map(|ts| relative_time(&ts.0))
        .unwrap_or_default();
    // Cap the indent so deep threads stay readable.
    let indent = depth.min(6) as f32 * 1.5;

    rsx! {
        div {
            class: if deleted { "comment comment-deleted" } else { "comment" },
            style: "margin-left: {indent}rem;",
            div { class: "comment-main",
                if deleted {
                    div { class: "avatar small comment-avatar",
                        span { class: "material-icons", "block" }
                    }
                } else {
                    super::loader::UserPopover {
                        name: author.clone(),
                        avatar_url: avatar_url.clone(),
                        user_id: author_id.clone(),
                        div { class: "avatar small comment-avatar",
                            {super::loader::user_avatar(&avatar_url, rsx! { "{initial}" })}
                        }
                    }
                }
                div { class: "comment-body",
                    div { class: "comment-meta",
                        if deleted {
                            span { class: "comment-author", "{author}" }
                        } else {
                            super::loader::UserPopover {
                                name: author.clone(),
                                avatar_url: avatar_url.clone(),
                                user_id: author_id.clone(),
                                span { class: "comment-author", "{author}" }
                            }
                        }
                        if !when.is_empty() {
                            span { class: "comment-dot", "·" }
                            span { class: "comment-time", "{when}" }
                        }
                    }
                    if !deleted {
                        p { class: "comment-text",
                            // A comment is where people actually paste links: the
                            // motion they are answering, the article they are citing.
                            super::content::AutoLinked { text: text.clone() }
                        }
                        if let Some(id) = image.clone() {
                            CommentImage { file_id: id }
                        }
                        ReactionBar {
                            comment_id: comment.id.0.clone(),
                            context_id: context_id.clone(),
                            can_react: is_auth && can_comment,
                        }
                    }
                    div { class: "comment-actions",
                        if is_auth && can_comment && !deleted {
                            button {
                                class: "comment-action",
                                aria_label: "{t(\"vote.reply\")}",
                                onclick: move |_| {
                                    let v = replying();
                                    replying.set(!v);
                                },
                                span { class: "material-icons", "reply" }
                                "{t(\"vote.reply\")}"
                            }
                        }
                        if !replies.is_empty() {
                            span { class: "comment-reply-count",
                                "{replies.len()} {t(\"vote.replies\")}"
                            }
                        }
                        // Nothing left to delete on an emptied comment, and
                        // deleting the hanger would take the replies.
                        if can_del && !deleted {
                            button {
                                class: "comment-action comment-action-danger",
                                aria_label: "{t(\"common.delete\")}",
                                onclick: move |_| del_confirm.set(true),
                                span { class: "material-icons", "delete" }
                            }
                        }
                    }
                    if can_del {
                        super::widgets::Dialog {
                            open: del_confirm(),
                            on_dismiss: move |_| del_confirm.set(false),
                            headline: t("vote.confirmDeleteComment"),
                            icon: "delete".to_string(),
                            actions: rsx! {
                                button {
                                    class: "btn btn-outlined",
                                    onclick: move |_| del_confirm.set(false),
                                    "{t(\"common.cancel\")}"
                                }
                                button {
                                    class: "btn btn-primary",
                                    onclick: {
                                        let del_id = comment.id.0.clone();
                                        // Answered comments are emptied, not removed: the
                                        // replies hang from this row, and taking it would
                                        // take them with it.
                                        let has_replies = !replies.is_empty();
                                        move |_| {
                                            let token = session.read().access_token.clone();
                                            let actor = session.read().user.as_ref().map(|u| u.id.clone());
                                            let del_id = del_id.clone();
                                            del_confirm.set(false);
                                            spawn(async move {
                                                let outcome = if has_replies {
                                                    tombstone_comment(token, del_id).await
                                                } else {
                                                    delete_comment_subtree(token, del_id, actor).await
                                                };
                                                match outcome {
                                                    Ok(()) => {
                                                        let mut refresh = refresh;
                                                        refresh += 1;
                                                        crate::session::bump_data_version();
                                                    }
                                                    Err(e) => {
                                                        log::error!("delete comment failed: {e}");
                                                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                    }
                                                }
                                            });
                                        }
                                    },
                                    "{t(\"common.delete\")}"
                                }
                            },
                            p { class: "body-medium", "{t(\"vote.confirmDeleteComment\")}" }
                            p { class: "body-medium text-muted", "{t(\"content.deleteRecoverable\")}" }
                        }
                    }
                    if replying() {
                        CommentComposer {
                            parent_id: comment.id.0.clone(),
                            context_id: context_id.clone(),
                            refresh,
                            pending: reply_pending,
                            placeholder: t("vote.reply"),
                            on_posted: move |_| replying.set(false),
                        }
                    }
                }
            }
            // Optimistic reply rows still in flight.
            for p in reply_pending_shown.iter() {
                PendingRow {
                    key: "{p.key}",
                    author: p.author.clone(),
                    text: p.text.clone(),
                    image: p.image.clone(),
                    depth: depth + 1,
                }
            }
            for r in replies.iter() {
                CommentThread {
                    key: "{r.id.0}",
                    comment: r.clone(),
                    context_id: context_id.clone(),
                    depth: depth + 1,
                    refresh,
                    can_comment,
                }
            }
        }
    }
}

/// The quick-react emoji set: the first tab of the picker and the default row.
const QUICK_REACTIONS: &[&str] = &["👍", "❤️", "😂", "🎉", "😮", "😢", "🙏", "🚀"];

/// The full add-reaction picker, grouped into tabs. Kept as a curated set (no
/// giant Unicode table shipped to the client): a few dozen common reactions
/// across the categories members actually reach for, with `QUICK_REACTIONS`
/// first so the common case is one tap.
/// Category labels are i18n KEYS, not text: they are the picker's only words, so
/// leaving them in English put six English headings inside a Danish app. The
/// i18n test cannot catch that class on its own — it checks that keys used exist,
/// and a bare literal uses no key — so the fix is to make them keys.
const EMOJI_CATEGORIES: &[(&str, &[&str])] = &[
    ("emoji.quick", QUICK_REACTIONS),
    (
        "emoji.smileys",
        &[
            "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😊", "🙂", "😉", "😍", "🥰", "😘", "😜",
            "🤪", "🤔", "🤨", "😐", "😴", "😮", "😯", "🥳", "😎", "🤩", "😢", "😭", "😤", "😠",
            "🤯", "😱", "🤗", "🤭",
        ],
    ),
    (
        "emoji.gestures",
        &[
            "👍", "👎", "👌", "🤌", "✌️", "🤞", "🤟", "🤙", "👏", "🙌", "🙏", "💪", "👊", "✊",
            "🤝", "👋", "🫶",
        ],
    ),
    (
        "emoji.hearts",
        &[
            "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "💯", "💖", "💕", "💗",
        ],
    ),
    (
        "emoji.celebration",
        &[
            "🎉", "🎊", "🥳", "🎁", "🎈", "✨", "🔥", "🚀", "⭐", "🌟", "💫", "🏆", "🥇", "👑",
        ],
    ),
    (
        "emoji.symbols",
        &[
            "✅", "❌", "⚠️", "❓", "❗", "💡", "📌", "🔔", "👀", "💬", "♻️", "🕊️", "⚖️", "📣",
        ],
    ),
];

/// The emoji stored on a `vote/reaction` node (`data.emoji`).
fn reaction_emoji(node: &ChildNodeFields) -> String {
    node.data
        .as_ref()
        .and_then(|d| d.0.get("emoji"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string()
}

/// Set the caller's reaction on `parent_id` to `emoji`. The backend permits only
/// ONE reaction per user per parent (a Hasura insert-permission check), so this
/// is swap-not-add: `mine` is the caller's current reaction on this parent (its
/// node id + emoji), if any.
/// - clicking the emoji they already have removes it (toggle off);
/// - clicking a different emoji swaps: delete the old, then insert the new;
/// - with no existing reaction, it just inserts.
async fn set_reaction(
    token: Option<String>,
    parent_id: String,
    context_id: Option<String>,
    emoji: String,
    mine: Option<(String, String)>,
) -> Result<(), String> {
    if let Some((id, current_emoji)) = mine {
        graphql::delete_node(token.as_deref(), &id).await?;
        // Same emoji → that was a toggle-off; nothing more to add.
        if current_emoji == emoji {
            return Ok(());
        }
    }
    graphql::insert_reaction(token.as_deref(), &parent_id, context_id.as_deref(), &emoji)
        .await
        .map(|_| ())
}

/// A reaction bar under a comment: grouped emoji chips with counts (the caller's
/// own reactions highlighted, tap to toggle) plus an add-reaction popover. Live:
/// refetches whenever a `vote/reaction` under this comment changes.
#[component]
fn ReactionBar(comment_id: String, context_id: Option<String>, can_react: bool) -> Element {
    let session = use_session();
    let current_user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let refresh = use_signal(|| 0u32);

    let sub_id = comment_id.clone();
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{sub_id}\" }}, mimeId: {{ _eq: \"vote/reaction\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    let load_id = comment_id.clone();
    let token = session.read().access_token.clone();
    let rev = refresh();
    let reactions_res = crate::use_data_resource!(|(load_id, token, rev)| async move {
        let _ = rev;
        graphql::query_reactions(token.as_deref(), &load_id)
            .await
            .unwrap_or_default()
    });
    let reactions = reactions_res.read().clone().unwrap_or_default();

    // Group by emoji in first-seen order: (emoji, count, my reaction node id).
    let mut groups: Vec<(String, usize, Option<String>)> = Vec::new();
    for r in reactions.iter() {
        let emoji = reaction_emoji(r);
        if emoji.is_empty() {
            continue;
        }
        let mine = current_user_id.is_some()
            && r.owner_id.as_ref().map(|o| o.0.clone()) == current_user_id;
        if let Some(g) = groups.iter_mut().find(|(e, _, _)| *e == emoji) {
            g.1 += 1;
            if mine {
                g.2 = Some(r.id.0.clone());
            }
        } else {
            groups.push((emoji, 1, mine.then(|| r.id.0.clone())));
        }
    }
    // The caller's current reaction on this parent (node id + emoji), if any.
    // The backend allows only one, so a new pick swaps this out (see set_reaction).
    let my_current: Option<(String, String)> = groups
        .iter()
        .find_map(|(emoji, _, mine)| mine.clone().map(|id| (id, emoji.clone())));

    let mut picker_open = use_signal(|| false);
    let mut picker_cat = use_signal(|| 0usize);
    // Nothing to show and no permission to add: render nothing.
    if groups.is_empty() && !can_react {
        return rsx! {};
    }

    rsx! {
        div { class: "reaction-bar",
            for (emoji, count, my_id) in groups.iter() {
                button {
                    key: "{emoji}",
                    class: if my_id.is_some() { "reaction-chip is-mine" } else { "reaction-chip" },
                    disabled: !can_react,
                    onclick: {
                        let emoji = emoji.clone();
                        let my_current = my_current.clone();
                        let parent_id = comment_id.clone();
                        let context_id = context_id.clone();
                        move |_| {
                            if !can_react {
                                return;
                            }
                            let token = session.read().access_token.clone();
                            let (emoji, my_current, parent_id, context_id) =
                                (emoji.clone(), my_current.clone(), parent_id.clone(), context_id.clone());
                            spawn(async move {
                                match set_reaction(token, parent_id, context_id, emoji, my_current).await {
                                    Ok(()) => {
                                        let mut refresh = refresh;
                                        refresh += 1;
                                    }
                                    Err(e) => {
                                        log::error!("reaction failed: {e}");
                                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                    }
                                }
                            });
                        }
                    },
                    span { class: "reaction-emoji", "{emoji}" }
                    span { class: "reaction-count", "{count}" }
                }
            }
            if can_react {
                div { class: "reaction-add",
                    button {
                        class: "reaction-chip reaction-add-btn",
                        aria_label: "{t(\"vote.addReaction\")}",
                        onclick: move |_| {
                            let open = picker_open();
                            picker_open.set(!open);
                        },
                        span { class: "material-icons", "add_reaction" }
                    }
                    if picker_open() {
                        div { class: "reaction-picker",
                            div { class: "reaction-picker-tabs",
                                for (i , (label , _)) in EMOJI_CATEGORIES.iter().enumerate() {
                                    button {
                                        key: "{label}",
                                        class: if picker_cat() == i { "reaction-picker-tab is-active" } else { "reaction-picker-tab" },
                                        title: "{t(label)}",
                                        onclick: move |_| picker_cat.set(i),
                                        span { "{EMOJI_CATEGORIES[i].1[0]}" }
                                    }
                                }
                            }
                            div { class: "reaction-picker-grid",
                                for e in EMOJI_CATEGORIES[picker_cat().min(EMOJI_CATEGORIES.len() - 1)].1.iter() {
                                    button {
                                        key: "{e}",
                                        class: "reaction-picker-item",
                                        onclick: {
                                            let emoji = e.to_string();
                                            let my_current = my_current.clone();
                                            let parent_id = comment_id.clone();
                                            let context_id = context_id.clone();
                                            move |_| {
                                                picker_open.set(false);
                                                let token = session.read().access_token.clone();
                                                let (emoji, my_current, parent_id, context_id) = (
                                                    emoji.clone(),
                                                    my_current.clone(),
                                                    parent_id.clone(),
                                                    context_id.clone(),
                                                );
                                                spawn(async move {
                                                    match set_reaction(token, parent_id, context_id, emoji, my_current).await {
                                                        Ok(()) => {
                                                            let mut refresh = refresh;
                                                            refresh += 1;
                                                        }
                                                        Err(e) => {
                                                            log::error!("reaction failed: {e}");
                                                            crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                        }
                                                    }
                                                });
                                            }
                                        },
                                        "{e}"
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

/// A comment / reply composer: a text area and a post button. On a successful
/// post it clears, bumps `refresh` (so threads refetch) and calls `on_posted`.
#[component]
fn CommentComposer(
    parent_id: String,
    context_id: Option<String>,
    refresh: Signal<u32>,
    pending: Signal<Vec<PendingComment>>,
    placeholder: String,
    on_posted: EventHandler,
) -> Element {
    let session = use_session();
    let mut text = use_signal(String::new);
    let mut posting = use_signal(|| false);
    let mut pending = pending;
    // The attached image, uploaded as soon as it is picked so posting is one
    // insert rather than an upload the reader waits through. Picking a second
    // replaces the first: the node carries one image (see graphql::comment_data).
    let mut attached = use_signal(|| None::<String>);
    let mut uploading = use_signal(|| false);
    // The preview is made from the bytes in the browser, NOT fetched back from
    // storage: until the comment exists there is no node pointing at the file,
    // and a file nothing points at is readable by nobody — including the person
    // who just uploaded it, since `uploaded_by_user_id` is null in this
    // deployment. Fetching it back showed an empty box every time. The local
    // blob also needs no round trip, which is what a preview should cost.
    let mut preview = use_signal(|| None::<String>);
    // Revoke the object URL when this composer goes away, so the bytes are not
    // held for the life of the tab.
    use_drop(move || {
        if let Some(url) = preview.peek().as_ref() {
            let _ = web_sys::Url::revoke_object_url(url);
        }
    });
    // Replace the preview, revoking whatever it was showing.
    let mut set_preview = move |url: Option<String>| {
        if let Some(old) = preview.peek().clone() {
            let _ = web_sys::Url::revoke_object_url(&old);
        }
        preview.set(url);
    };

    let pick_image = move |evt: FormEvent| {
        let Some(fd) = evt.files().into_iter().next() else {
            return;
        };
        let token = session.read().access_token.clone();
        spawn(async move {
            let name = fd.name();
            let ctype = fd.content_type().unwrap_or_default();
            let bytes = match fd.read_bytes().await {
                Ok(b) => b,
                Err(e) => {
                    log::error!("read attachment failed: {e}");
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                    return;
                }
            };
            if let Some(reason) = image_rejection(&ctype, bytes.len()) {
                crate::snackbar::show_snackbar(&t(reason));
                return;
            }
            // Show it at once, from the bytes already in hand, and upload behind
            // that: the reader sees what they picked while the network works.
            set_preview(object_url(&bytes, &ctype));
            uploading.set(true);
            match crate::nhost::upload_file(token.as_deref(), bytes.to_vec(), &name, &ctype).await {
                Ok(f) => attached.set(Some(f.id)),
                Err(e) => {
                    log::error!("attachment upload failed: {e:?}");
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                    set_preview(None);
                }
            }
            uploading.set(false);
        });
    };

    let post = move |_| {
        let body = text.read().trim().to_string();
        let image = attached.read().clone();
        // Handed to the optimistic row, which revokes it when it is reconciled
        // away — so the picked image stays on screen from the moment it is
        // chosen until the real comment replaces it, with no gap and no leak.
        let shown = preview.peek().clone();
        // A comment that is only an image is still a comment.
        if body.is_empty() && image.is_none() {
            return;
        }
        // Natural opt-in: if you're joining the conversation, offer to notify you
        // when someone replies (no-op once the choice has been made).
        crate::pwa::request_notification_permission();
        let token = session.read().access_token.clone();
        let author = session
            .read()
            .user
            .as_ref()
            .map(|u| u.display_name.clone())
            .unwrap_or_default();
        let parent_id = parent_id.clone();
        let context_id = context_id.clone();
        let mut refresh = refresh;
        // A unique per-parent key for the comment node — also the reconciliation
        // join: the refetch drops the optimistic row once a real comment shares it.
        let key = format!("c{}", (js_sys::Math::random() * 1e12) as u64);
        // Optimistic: show it and clear the input immediately; a failed post rolls
        // it back and restores the text.
        pending.write().push(PendingComment {
            key: key.clone(),
            author: author.clone(),
            text: body.clone(),
            image: shown,
        });
        text.set(String::new());
        attached.set(None);
        // Cleared WITHOUT revoking: the pending row owns that URL now.
        preview.set(None);
        spawn(async move {
            posting.set(true);
            let result = graphql::insert_comment(
                token.as_deref(),
                &parent_id,
                context_id.as_deref(),
                &key,
                &author,
                &body,
                image.as_deref(),
            )
            .await;
            posting.set(false);
            match result {
                // Any non-error is a success: Hasura may return no row when the
                // fresh comment is not yet selectable, but it WAS inserted.
                Ok(_) => {
                    let v = refresh();
                    refresh.set(v + 1);
                    on_posted.call(());
                    // Best-effort background push to the author of the node being
                    // commented on ("someone replied to you"), so they hear about it
                    // with the app closed. The page URL is the node the reply lands
                    // on; the backend gates on the caller being a context member.
                    if let Some(tok) = token.as_ref() {
                        let link = web_sys::window()
                            .and_then(|w| w.location().pathname().ok())
                            .unwrap_or_default();
                        let body = if author.is_empty() {
                            t("vote.replyNotifyBody")
                        } else {
                            author.clone()
                        };
                        let _ = crate::backend_api::push_reply(
                            tok,
                            &parent_id,
                            &t("vote.replyNotifyTitle"),
                            &body,
                            &link,
                        )
                        .await;
                    }
                }
                Err(e) => {
                    log::error!("comment post failed: {e}");
                    // Roll back the optimistic row and restore the unsent text —
                    // and the attachment, which is already uploaded, so a retry
                    // does not ask for the photo again.
                    pending.write().retain(|p| p.key != key);
                    text.set(body);
                    attached.set(image);
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    let input_id = use_hook(|| format!("comment-image-{}", (js_sys::Math::random() * 1e9) as u64));
    let attachment = preview.read().clone();
    // What the box will turn into a link when this is posted. A textarea holds
    // text rather than markup, so it cannot show a link AS a link — but leaving
    // the writer to guess whether their pasted address was understood is the
    // actual complaint, and naming what was found answers it.
    let links = super::content::detected_links(&text.read());

    rsx! {
        div { class: "comment-composer-wrap",
            div { class: "comment-composer",
                textarea {
                    class: "comment-input",
                    placeholder: "{placeholder}",
                    rows: "2",
                    value: "{text}",
                    oninput: move |evt| text.set(evt.value()),
                }
                // The input itself is never shown; its label is the button.
                input {
                    id: "{input_id}",
                    class: "file-upload-input",
                    r#type: "file",
                    accept: "image/*",
                    onchange: pick_image,
                }
                label {
                    r#for: "{input_id}",
                    class: "btn-icon state-layer comment-attach",
                    title: "{t(\"vote.addImage\")}",
                    aria_label: "{t(\"vote.addImage\")}",
                    if *uploading.read() {
                        div { class: "spinner spinner-xs" }
                    } else {
                        span { class: "material-icons", "image" }
                    }
                }
                button {
                    class: "btn-icon",
                    r#type: "button",
                    aria_label: "{t(\"common.send\")}",
                    disabled: *posting.read() || *uploading.read(),
                    onclick: post,
                    span { class: "material-icons", "send" }
                }
            }
            if !links.is_empty() {
                div { class: "comment-links",
                    for url in links.iter() {
                        {
                            // Shown as written: an address, not the mailto: the
                            // renderer will actually put in the href.
                            let shown = url.strip_prefix("mailto:").unwrap_or(url).to_string();
                            rsx! {
                                div { class: "comment-link-chip", key: "{url}",
                                    span { class: "material-icons", "link" }
                                    span { class: "comment-link-url", "{shown}" }
                                }
                            }
                        }
                    }
                }
            }
            if let Some(src) = attachment {
                div { class: "comment-attachment",
                    img { src: "{src}", alt: "{t(\"vote.imageAlt\")}" }
                    button {
                        class: "btn-icon comment-attachment-remove",
                        r#type: "button",
                        aria_label: "{t(\"common.delete\")}",
                        onclick: move |_| {
                            attached.set(None);
                            set_preview(None);
                        },
                        span { class: "material-icons", "close" }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{reconcile_pending, PendingComment};
    use std::collections::HashSet;

    fn pending(key: &str) -> PendingComment {
        PendingComment {
            key: key.to_string(),
            author: "Me".to_string(),
            text: "hi".to_string(),
            image: None,
        }
    }

    #[test]
    fn an_attachment_must_be_an_image_of_a_sane_size() {
        use super::{image_rejection, MAX_COMMENT_IMAGE_BYTES};
        assert_eq!(image_rejection("image/jpeg", 3_000_000), None);
        // `accept="image/*"` is a hint the picker may ignore, and a scripted
        // post never sees it at all.
        assert_eq!(
            image_rejection("application/pdf", 10),
            Some("vote.imageNotAnImage")
        );
        assert_eq!(
            image_rejection("video/mp4", 10),
            Some("vote.imageNotAnImage")
        );
        // A video renamed .jpg is caught by size rather than by its name.
        assert_eq!(
            image_rejection("image/jpeg", MAX_COMMENT_IMAGE_BYTES + 1),
            Some("vote.imageTooLarge")
        );
    }

    /// The id must land under `data.image`, because `nodes.file_id` — the column
    /// the storage permission joins on to decide who may read the file — is
    /// GENERATED from it. Anywhere else and the image is readable by nobody.
    #[test]
    fn an_attached_image_is_stored_where_the_generated_column_reads_it() {
        let with = crate::graphql::comment_data("hi", Some("7b0d79b7-de73-4291-9cd5-e8b3413d9246"));
        assert_eq!(
            with,
            serde_json::json!({"text": "hi", "image": "7b0d79b7-de73-4291-9cd5-e8b3413d9246"})
        );
        // And a plain comment keeps the shape every comment has always had, so
        // older readers need learn nothing.
        assert_eq!(
            crate::graphql::comment_data("hi", None),
            serde_json::json!({"text": "hi"})
        );
        assert_eq!(
            crate::graphql::comment_data("hi", Some("")),
            serde_json::json!({"text": "hi"})
        );
    }

    #[test]
    fn keeps_only_unconfirmed_pending_comments() {
        let pend = vec![pending("a"), pending("b"), pending("c")];
        // "b" has come back from the server, so its optimistic row is dropped.
        let fetched: HashSet<String> = ["b".to_string()].into_iter().collect();
        let shown = reconcile_pending(&pend, &fetched);
        assert_eq!(
            shown.iter().map(|p| p.key.as_str()).collect::<Vec<_>>(),
            vec!["a", "c"]
        );
    }

    #[test]
    fn empty_cases() {
        // Nothing pending -> nothing shown.
        assert!(reconcile_pending(&[], &HashSet::new()).is_empty());
        // All confirmed -> nothing shown (no duplicate rows).
        let pend = vec![pending("a")];
        let fetched: HashSet<String> = ["a".to_string()].into_iter().collect();
        assert!(reconcile_pending(&pend, &fetched).is_empty());
        // None confirmed yet -> all shown, order preserved.
        let shown = reconcile_pending(&pend, &HashSet::new());
        assert_eq!(shown.len(), 1);
    }
}
