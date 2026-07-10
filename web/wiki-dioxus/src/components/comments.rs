//! Nested comment threads (Bluesky-style). Comments are `vote/comment` nodes
//! whose parent is the post (or another comment for a reply); each thread level
//! lazily fetches its own children, so nesting is unbounded. The design is a
//! modern threaded view: an avatar rail, author + relative time, the text, and a
//! reply affordance, with replies indented under their parent.

use dioxus::prelude::*;

use crate::graphql::{self, ChildNodeFields};
use crate::i18n::t;
use crate::session::use_session;

/// A short relative time ("5m", "3h", "2d") from an ISO timestamp.
fn relative_time(iso: &str) -> String {
    let then = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(iso)).get_time();
    if then.is_nan() {
        return String::new();
    }
    let secs = ((js_sys::Date::now() - then) / 1000.0).max(0.0);
    if secs < 60.0 {
        format!("{}s", secs as u64)
    } else if secs < 3600.0 {
        format!("{}m", (secs / 60.0) as u64)
    } else if secs < 86_400.0 {
        format!("{}h", (secs / 3600.0) as u64)
    } else {
        format!("{}d", (secs / 86_400.0) as u64)
    }
}

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
    let comments = use_resource(use_reactive!(|(nid, token, rev)| async move {
        let _ = rev;
        graphql::query_comments(token.as_deref(), &nid)
            .await
            .unwrap_or_default()
    }));
    let list = comments.read().clone().unwrap_or_default();

    rsx! {
        div { class: "card comment-section",
            div { class: "card-header",
                div { class: "avatar", {crate::components::loader::icon_el("vote/comment")} }
                h3 { class: "title-medium", "{t(\"vote.comments\")}" }
                if !list.is_empty() {
                    span { class: "count-badge", "{list.len()}" }
                }
            }
            div { class: "card-content",
                if is_auth {
                    CommentComposer {
                        parent_id: node_id.clone(),
                        context_id: context_id.clone(),
                        refresh,
                        placeholder: t("vote.newComment"),
                        on_posted: move |_| {},
                    }
                }
                if list.is_empty() {
                    p { class: "body-medium comment-empty",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"vote.noComments\")}"
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
) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let is_auth = session.read().is_authenticated();
    let mut replying = use_signal(|| false);

    let cid = comment.id.0.clone();
    let rev = refresh();
    let replies_res = use_resource(use_reactive!(|(cid, token, rev)| async move {
        let _ = rev;
        graphql::query_comments(token.as_deref(), &cid)
            .await
            .unwrap_or_default()
    }));
    let replies = replies_res.read().clone().unwrap_or_default();

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
                div { class: "avatar small comment-avatar", "{initial}" }
                div { class: "comment-body",
                    div { class: "comment-meta",
                        span { class: "comment-author", "{author}" }
                        if !when.is_empty() {
                            span { class: "comment-dot", "·" }
                            span { class: "comment-time", "{when}" }
                        }
                    }
                    p { class: "comment-text", "{text}" }
                    div { class: "comment-actions",
                        if is_auth {
                            button {
                                class: "comment-action",
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
            let ok = graphql::insert_comment(
                token.as_deref(),
                &parent_id,
                context_id.as_deref(),
                &key,
                &author,
                &body,
            )
            .await
            .unwrap_or(false);
            posting.set(false);
            if ok {
                text.set(String::new());
                let v = refresh();
                refresh.set(v + 1);
                on_posted.call(());
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
                disabled: *posting.read(),
                onclick: post,
                span { class: "material-icons", "send" }
            }
        }
    }
}
