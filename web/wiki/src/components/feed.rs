//! The feed: what has been posted recently, newest first, as content, comments
//! and reactions.
//!
//! One list, two scopes. [`FeedApp`] is a context's own feed, an app of that
//! group or event ("what has happened here"). The home page mounts the same
//! [`FeedList`] unscoped: everything recent across the groups and events you
//! belong to. Both are pages, so both get a full reading column and page as you
//! scroll.

use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::model;
use crate::route::Route;
use crate::session::use_session;

/// A context's own feed, opened from the rail as `?app=feed`.
///
/// Scoped by the node's context, so it is the same list wherever in the context
/// it was opened from. Note that a context is its own context: an event under a
/// group has its own feed, and does not appear in the group's.
///
/// At the ROOT (the parent-less node behind `/`) that scoping would be almost
/// empty, since every group and event is its own context. There the feed means
/// what it should mean at the top of the wiki: everything across the groups and
/// events you belong to.
#[component]
pub fn FeedApp(node: model::NodeWithChildren) -> Element {
    let context_id = if node.parent_id.is_none() {
        None
    } else {
        Some(
            node.context_id
                .clone()
                .map(|c| c.0)
                .unwrap_or_else(|| node.id.0.clone()),
        )
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "view_agenda" } }
                h3 { class: "title-large", "{t(\"layout.feed\")}" }
            }
            FeedList { context_id, autoload: true }
        }
    }
}

/// How many feed rows to fetch per page.
const FEED_PAGE: i32 = 12;

/// The feed list itself.
///
/// `context_id` scopes it to one group or event; without it the list is
/// everything the reader may see. `autoload` fetches the next page as the reader
/// nears the end of the window; without it the list pages on its button.
#[component]
pub fn FeedList(
    #[props(default)] context_id: Option<String>,
    #[props(default)] autoload: bool,
) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());

    // Pages accumulate here rather than in a resource: a resource re-runs and
    // replaces, which would drop everything already read past.
    let mut items = use_signal(Vec::<model::ChildNodeFields>::new);
    let mut loading = use_signal(|| false);
    // Cleared when a page comes back short: that is the end of the feed, and
    // without it the list would keep asking for more.
    let mut has_more = use_signal(|| true);

    let page_size = FEED_PAGE;

    // First page, and a reset if the scope or the signed-in user changes.
    {
        let token = token.clone();
        let user_id = user_id.clone();
        let ctx = context_id.clone();
        use_effect(use_reactive!(|(token, user_id, ctx)| {
            items.set(Vec::new());
            has_more.set(true);
            let Some(uid) = user_id.clone() else { return };
            let token = token.clone();
            let ctx = ctx.clone();
            loading.set(true);
            spawn(async move {
                let page = graphql::query_recent_nodes(
                    token.as_deref(),
                    page_size,
                    0,
                    &uid,
                    ctx.as_deref(),
                )
                .await;
                has_more.set(page.len() as i32 == page_size);
                items.set(page);
                loading.set(false);
            });
        }));
    }

    // Fetch the next page. Shared by the button and (on a page) the scroll
    // sentinel, so both stop at the same end and neither can double-fetch.
    let mut fetch_more = move |token: Option<String>, uid: Option<String>, ctx: Option<String>| {
        if *loading.peek() || !*has_more.peek() {
            return;
        }
        let Some(uid) = uid else { return };
        let offset = items.peek().len() as i32;
        loading.set(true);
        spawn(async move {
            let page = graphql::query_recent_nodes(
                token.as_deref(),
                FEED_PAGE,
                offset,
                &uid,
                ctx.as_deref(),
            )
            .await;
            // Ask for more only while pages come back full; a short page is the
            // end. Judged on what the server sent, before deduping.
            has_more.set(page.len() as i32 == FEED_PAGE);
            // Never append a row already on screen. Offset paging repeats rows
            // whenever something is inserted between two fetches, and a repeat is
            // not merely untidy here: the list is keyed by id, and Dioxus panics
            // on duplicate keys among siblings.
            let mut list = items.write();
            let seen: std::collections::HashSet<String> =
                list.iter().map(|n| n.id.0.clone()).collect();
            list.extend(page.into_iter().filter(|n| !seen.contains(&n.id.0)));
            drop(list);
            loading.set(false);
        });
    };

    // Endless scroll on a page: `near_bottom` is driven by the shell's single
    // window scroll listener, which a sheet's own scroll container never moves.
    {
        let token = token.clone();
        let user_id = user_id.clone();
        let ctx = context_id.clone();
        let near = autoload && crate::components::back_to_top::near_bottom();
        use_effect(use_reactive!(|(near, token, user_id, ctx)| {
            if !near {
                return;
            }
            fetch_more(token.clone(), user_id.clone(), ctx.clone());
        }));
    }

    let rows = items.read().clone();
    if rows.is_empty() {
        return rsx! {
            if *loading.read() {
                div { class: "empty-state empty-state-sm",
                    div { class: "spinner spinner-sm" }
                }
            } else {
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "view_agenda" }
                    }
                    p { class: "empty-state-body", "{t(\"layout.feedEmpty\")}" }
                }
            }
        };
    }

    rsx! {
        div { class: "list",
            for node in rows.iter() {
                RecentItem { key: "{node.id.0}", node: node.clone() }
            }
        }
        if *loading.read() {
            div { class: "empty-state empty-state-sm",
                div { class: "spinner spinner-sm" }
            }
        } else if *has_more.read() {
            button {
                class: "btn btn-text",
                onclick: move |_| fetch_more(token.clone(), user_id.clone(), context_id.clone()),
                "{t(\"layout.showMore\")}"
            }
        }
    }
}

#[component]
fn RecentItem(node: model::ChildNodeFields) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let node_id = node.id.0.clone();
    let key = node.key.clone();

    // Author name + initials for the social-style avatar. Prefer the `owner`
    // relationship (it also carries the id, so the popover can link to a
    // profile) and fall back to the computed `author_name`, which is readable
    // for content in a context you do not belong to — most of this list.
    let author = node
        .owner
        .as_ref()
        .map(|o| o.display_name.clone())
        .or_else(|| node.author_name.clone())
        .filter(|s| !s.is_empty());
    let initials = author
        .as_ref()
        .map(|a| {
            a.split_whitespace()
                .filter_map(|w| w.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".to_string());
    let avatar_url = node
        .owner
        .as_ref()
        .map(|o| o.avatar_url.clone())
        .or_else(|| node.author_avatar.clone())
        .unwrap_or_default();
    let owner_id = node.owner.as_ref().map(|o| o.id.0.clone());
    let author_name = author.clone().unwrap_or_else(|| t("common.unknown"));
    let created = node.created_at.as_ref().map(|t| t.0.clone());
    let parent_name = node.parent.as_ref().map(|p| p.name.clone());
    let mime = node.mime_id.clone().unwrap_or_default();
    let data = node.data.as_ref().map(|d| d.0.clone());
    // Three kinds of row, because three kinds of thing happened. A comment and a
    // reaction node's `name` is its AUTHOR (already shown above the row), so
    // neither can use it as a headline the way content does.
    let is_comment = mime == "vote/comment";
    let is_reaction = mime == "vote/reaction";
    let comment_text = |d: Option<&crate::model::Jsonb>| {
        d.and_then(|d| d.0.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
    };
    let title = if is_comment {
        comment_text(node.data.as_ref()).unwrap_or_else(|| t("vote.comments"))
    } else if is_reaction {
        // The emoji is the whole of what happened.
        node.data
            .as_ref()
            .and_then(|d| d.0.get("emoji"))
            .and_then(|e| e.as_str())
            .map(str::to_string)
            .unwrap_or_else(|| node.name.clone())
    } else {
        node.name.clone()
    };
    // What this row is ABOUT: the comment a reaction is on, or the comment a
    // reply answers — quoted beneath with WHOSE it is, so the row stands on its
    // own. A comment node's `name` is its author, which is why the quote can
    // name them without another lookup.
    let about = if is_reaction || is_comment {
        node.parent
            .as_ref()
            .filter(|p| p.mime_id.as_deref() == Some("vote/comment"))
            .and_then(|p| {
                comment_text(p.data.as_ref()).map(|text| {
                    (
                        p.name.trim().to_string(),
                        p.author_avatar.clone().unwrap_or_default(),
                        text,
                    )
                })
            })
    } else {
        None
    };
    let quote_initials = about
        .as_ref()
        .map(|(who, _, _)| {
            who.split_whitespace()
                .filter_map(|w| w.chars().next())
                .take(2)
                .collect::<String>()
                .to_uppercase()
        })
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "?".to_string());
    // The post's cover image (`data.image`, a file id). Resolved to a blob URL
    // so the JWT stays in the Authorization header and never in an <img src>.
    // Called unconditionally with an empty id when there is none, because it is
    // a hook — the same shape ContentApp uses.
    let cover_id = node
        .data
        .as_ref()
        .and_then(|d| {
            // A document, policy or candidate carries a cover in `image`; an
            // uploaded picture IS its own image, under `fileId`.
            d.0.get("image")
                .and_then(|v| v.as_str())
                .or_else(|| {
                    d.0.get("type")
                        .and_then(|t| t.as_str())
                        .filter(|t| t.starts_with("image/"))
                        .and_then(|_| d.0.get("fileId").and_then(|v| v.as_str()))
                })
                .filter(|s| !s.is_empty())
        })
        .map(String::from);
    let cover_url = super::loader::use_file_object_url(cover_id.unwrap_or_default());
    // For content, the opening of the text itself, so the feed shows something
    // rather than a list of titles.
    let excerpt = if is_comment || is_reaction {
        None
    } else {
        node.data
            .as_ref()
            .map(|d| super::content::slate_plain_text(&d.0))
            .map(|s| s.split_whitespace().collect::<Vec<_>>().join(" "))
            .filter(|s| !s.is_empty())
    };

    rsx! {
        div {
            class: "recent-item",
            onclick: move |_| {
                let node_id = node_id.clone();
                let key = key.clone();
                let token = session.read().access_token.clone();
                // Resolve the node's full ancestor path, then navigate. A comment
                // is not a page, so it opens the content hosting its thread.
                spawn(async move {
                    // A reaction hangs on a comment, which hangs on content, so
                    // both climb to the page that actually renders the thread.
                    let target = if is_comment || is_reaction {
                        graphql::thread_host_id(token.as_deref(), &node_id).await
                    } else {
                        node_id.clone()
                    };
                    let mut segments = graphql::path_from_id(token.as_deref(), &target)
                        .await
                        .unwrap_or_default();
                    if segments.is_empty() {
                        segments = vec![key];
                    }
                    nav.push(Route::PathPage { segments, app: None });
                });
            },
            // Author avatar (Bluesky picture when linked, else initials), like a
            // social post header — click for the identity popover.
            super::loader::UserPopover {
                name: author_name.clone(),
                avatar_url: avatar_url.clone(),
                user_id: owner_id.clone(),
                div { class: "avatar small recent-avatar",
                    {super::loader::user_avatar(&avatar_url, rsx! { "{initials}" })}
                }
            }
            div { class: "recent-body",
                // Who + when.
                div { class: "recent-meta",
                    if let Some(a) = author.as_ref() {
                        super::loader::UserPopover {
                            name: author_name.clone(),
                            avatar_url: avatar_url.clone(),
                            user_id: owner_id.clone(),
                            span { class: "recent-author", "{a}" }
                        }
                    }
                    if let Some(iso) = created.as_ref() {
                        if author.is_some() {
                            span { "\u{00b7}" }
                        }
                        span {
                            title: "{super::loader::full_datetime(iso)}",
                            "{super::loader::relative_time(iso)}"
                        }
                    }
                }
                // What. A reaction is one emoji, so it gets its own size rather
                // than being set as a line of body text.
                div {
                    class: if is_reaction { "recent-title recent-reaction" } else { "recent-title" },
                    "{title}"
                }
                // What it was about: whose comment, and what it said.
                if let Some((who, face, quote)) = about.as_ref() {
                    blockquote { class: "recent-quote",
                        div { class: "recent-quote-head",
                            div { class: "avatar recent-quote-avatar",
                                {super::loader::user_avatar(face, rsx! { "{quote_initials}" })}
                            }
                            if !who.is_empty() {
                                span { class: "recent-quote-author", "{who}" }
                            }
                        }
                        span { class: "recent-quote-text", "{quote}" }
                    }
                }
                // The opening of the content itself, clamped by CSS.
                if let Some(text) = excerpt.as_ref() {
                    p { class: "recent-excerpt", "{text}" }
                }
                // The node's own image, once its blob URL resolves.
                if let Some(url) = cover_url.as_ref() {
                    img {
                        class: "recent-cover",
                        src: "{url}",
                        alt: "",
                        loading: "lazy",
                        "referrerpolicy": "no-referrer",
                    }
                }
                // Where. Skipped when the quote above already names the parent:
                // a comment node's name IS its author, so for a reply or a
                // reaction this line would just repeat the credit.
                if about.is_none() {
                    if let Some(parent) = parent_name.as_ref() {
                        div { class: "recent-context",
                            {super::loader::node_icon_el(&mime, data.as_ref())}
                            span { "{parent}" }
                        }
                    }
                }
            }
        }
    }
}
