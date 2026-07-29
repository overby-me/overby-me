//! Nested comment threads (Bluesky-style). Comments are `vote/comment` nodes
//! whose parent is the post (or another comment for a reply); each thread level
//! lazily fetches its own children, so nesting is unbounded. The design is a
//! modern threaded view: an avatar rail, author + relative time, the text, and a
//! reply affordance, with replies indented under their parent.

use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::ChildNodeFields;
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

/// A comment shown optimistically before the server confirms it. Reconciled by
/// `key`: once the refetch returns a comment with the same key, the pending row is
/// dropped — no duplicate, no flicker.
#[derive(Clone, PartialEq)]
struct PendingComment {
    key: String,
    author: String,
    text: String,
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
fn PendingRow(author: String, text: String, depth: usize) -> Element {
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
                    p { class: "comment-text", "{text}" }
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
                            for c in list.iter() {
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

/// Delete a comment and everything under it, deepest-first.
///
/// Delegates to [`graphql::delete_node_deep`], which walks children of EVERY
/// mime. This used to walk `vote/comment` children only, so replies went but the
/// reactions on them stayed — pointing at a comment that no longer existed.
async fn delete_comment_subtree(token: Option<String>, root: String) -> Result<(), String> {
    graphql::delete_node_deep(token, root).await
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

    let author = if comment.name.trim().is_empty() {
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
    let when = comment
        .created_at
        .as_ref()
        .map(|ts| relative_time(&ts.0))
        .unwrap_or_default();
    // Cap the indent so deep threads stay readable.
    let indent = depth.min(6) as f32 * 1.5;

    rsx! {
        div { class: "comment", style: "margin-left: {indent}rem;",
            div { class: "comment-main",
                super::loader::UserPopover {
                    name: author.clone(),
                    avatar_url: avatar_url.clone(),
                    user_id: author_id.clone(),
                    div { class: "avatar small comment-avatar",
                        {super::loader::user_avatar(&avatar_url, rsx! { "{initial}" })}
                    }
                }
                div { class: "comment-body",
                    div { class: "comment-meta",
                        super::loader::UserPopover {
                            name: author.clone(),
                            avatar_url: avatar_url.clone(),
                            user_id: author_id.clone(),
                            span { class: "comment-author", "{author}" }
                        }
                        if !when.is_empty() {
                            span { class: "comment-dot", "·" }
                            span { class: "comment-time", "{when}" }
                        }
                    }
                    p { class: "comment-text", "{text}" }
                    ReactionBar {
                        comment_id: comment.id.0.clone(),
                        context_id: context_id.clone(),
                        can_react: is_auth && can_comment,
                    }
                    div { class: "comment-actions",
                        if is_auth && can_comment {
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
                        if can_del {
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
                                        move |_| {
                                            let token = session.read().access_token.clone();
                                            let del_id = del_id.clone();
                                            del_confirm.set(false);
                                            spawn(async move {
                                                match delete_comment_subtree(token, del_id).await {
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
const EMOJI_CATEGORIES: &[(&str, &[&str])] = &[
    ("Quick", QUICK_REACTIONS),
    (
        "Smileys",
        &[
            "😀", "😃", "😄", "😁", "😆", "😅", "🤣", "😊", "🙂", "😉", "😍", "🥰", "😘", "😜",
            "🤪", "🤔", "🤨", "😐", "😴", "😮", "😯", "🥳", "😎", "🤩", "😢", "😭", "😤", "😠",
            "🤯", "😱", "🤗", "🤭",
        ],
    ),
    (
        "Gestures",
        &[
            "👍", "👎", "👌", "🤌", "✌️", "🤞", "🤟", "🤙", "👏", "🙌", "🙏", "💪", "👊", "✊",
            "🤝", "👋", "🫶",
        ],
    ),
    (
        "Hearts",
        &[
            "❤️", "🧡", "💛", "💚", "💙", "💜", "🖤", "🤍", "🤎", "💔", "💯", "💖", "💕", "💗",
        ],
    ),
    (
        "Celebration",
        &[
            "🎉", "🎊", "🥳", "🎁", "🎈", "✨", "🔥", "🚀", "⭐", "🌟", "💫", "🏆", "🥇", "👑",
        ],
    ),
    (
        "Symbols",
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
                                        title: "{label}",
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

    let post = move |_| {
        let body = text.read().trim().to_string();
        if body.is_empty() {
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
        });
        text.set(String::new());
        spawn(async move {
            posting.set(true);
            let result = graphql::insert_comment(
                token.as_deref(),
                &parent_id,
                context_id.as_deref(),
                &key,
                &author,
                &body,
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
                    // Roll back the optimistic row and restore the unsent text.
                    pending.write().retain(|p| p.key != key);
                    text.set(body);
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        div { class: "comment-composer",
            textarea {
                class: "comment-input",
                placeholder: "{placeholder}",
                rows: "2",
                value: "{text}",
                oninput: move |evt| text.set(evt.value()),
            }
            button {
                class: "btn-icon",
                r#type: "button",
                aria_label: "{t(\"common.send\")}",
                disabled: *posting.read(),
                onclick: post,
                span { class: "material-icons", "send" }
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
        }
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
