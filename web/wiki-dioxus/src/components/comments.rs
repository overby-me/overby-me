//! Nested comment threads (Bluesky-style). Comments are `vote/comment` nodes
//! whose parent is the post (or another comment for a reply); each thread level
//! lazily fetches its own children, so nesting is unbounded. The design is a
//! modern threaded view: an avatar rail, author + relative time, the text, and a
//! reply affordance, with replies indented under their parent.

use dioxus::prelude::*;

use crate::graphql::{self, ChildNodeFields};
use crate::i18n::t;
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
    let comments = crate::use_data_resource!(|(nid, token, rev)| async move {
        let _ = rev;
        graphql::query_comments(token.as_deref(), &nid)
            .await
            .unwrap_or_default()
    });
    let list = comments.read().clone().unwrap_or_default();

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

    rsx! {
        div { class: "card comment-section",
            div { class: "card-header",
                div { class: "avatar small", {crate::components::loader::icon_el("vote/comment")} }
                h3 { class: "title-medium", "{t(\"vote.comments\")}" }
                if !list.is_empty() {
                    span { class: "count-badge", "{list.len()}" }
                }
            }
            div { class: "card-content",
                if is_auth && can_comment {
                    CommentComposer {
                        parent_id: node_id.clone(),
                        context_id: context_id.clone(),
                        refresh,
                        placeholder: t("vote.newComment"),
                        on_posted: move |_| {},
                    }
                }
                if list.is_empty() {
                    // DESIGN: a compact characterful empty state (floating orb).
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "forum" }
                        }
                        p { class: "empty-state-body", "{t(\"vote.noComments\")}" }
                    }
                } else {
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
                }
            }
        }
    }
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

    let cid = comment.id.0.clone();
    let rev = refresh();
    let replies_res = crate::use_data_resource!(|(cid, token, rev)| async move {
        let _ = rev;
        graphql::query_comments(token.as_deref(), &cid)
            .await
            .unwrap_or_default()
    });
    let replies = replies_res.read().clone().unwrap_or_default();

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
                    }
                    if replying() {
                        CommentComposer {
                            parent_id: comment.id.0.clone(),
                            context_id: context_id.clone(),
                            refresh,
                            placeholder: t("vote.reply"),
                            on_posted: move |_| replying.set(false),
                        }
                    }
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

/// A comment / reply composer: a text area and a post button. On a successful
/// post it clears, bumps `refresh` (so threads refetch) and calls `on_posted`.
#[component]
fn CommentComposer(
    parent_id: String,
    context_id: Option<String>,
    refresh: Signal<u32>,
    placeholder: String,
    on_posted: EventHandler,
) -> Element {
    let session = use_session();
    let mut text = use_signal(String::new);
    let mut posting = use_signal(|| false);

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
        spawn(async move {
            posting.set(true);
            // A unique per-parent key for the comment node.
            let key = format!("c{}", (js_sys::Math::random() * 1e12) as u64);
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
                    text.set(String::new());
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
                        let _ = crate::nhost::push_reply(
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
                class: "btn-icon comment-send",
                r#type: "button",
                aria_label: "{t(\"common.send\")}",
                disabled: *posting.read(),
                onclick: post,
                span { class: "material-icons", "send" }
            }
        }
    }
}
