//! The activity surface: what is new for the signed-in user (pending
//! invitations, then everything recently posted they may see), shown as an
//! overlay over wherever they already are.
//!
//! Checking what happened is not a place, so it is not a page. A delegate
//! reading a vote can open this, accept an invitation, close it and still be on
//! the same vote at the same scroll position. That is why it is ephemeral UI
//! state (like the search field) rather than a route.

use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::model;
use crate::route::Route;
use crate::session::use_session;

/// Whether the activity sheet is open. Global because the trigger lives in the
/// top app bar while the sheet itself is rendered at the app-shell root, so its
/// scrim escapes the bar's stacking context.
pub static ACTIVITY_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// Close the sheet and put focus back on the trigger that opened it. The trigger
/// is a single, always-present button, so it is found by class rather than
/// stashed in a signal.
fn close_activity() {
    *ACTIVITY_OPEN.write() = false;
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.query_selector(".activity-trigger").ok().flatten())
        .and_then(|e| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(e).ok())
    {
        let _ = el.focus();
    }
}

/// The top app bar's activity trigger, badged with the pending-invitation count
/// so an invitation is visible from every page instead of only from the home
/// page the reader has no reason to visit.
#[component]
pub fn ActivityButton() -> Element {
    let session = use_session();
    if !session.read().is_authenticated() {
        return rsx! {};
    }
    let pending = crate::components::layout::PENDING_INVITES();
    let label = t("common.activity");
    rsx! {
        button {
            class: "expressive-search-btn activity-trigger state-layer",
            aria_label: "{label}",
            title: "{label}",
            "aria-haspopup": "dialog",
            "aria-expanded": if ACTIVITY_OPEN() { "true" } else { "false" },
            onclick: move |_| {
                let now = !ACTIVITY_OPEN();
                if now {
                    *ACTIVITY_OPEN.write() = true;
                } else {
                    close_activity();
                }
            },
            span { class: "activity-trigger-indicator",
                span { class: "material-icons", "notifications" }
                if pending > 0 {
                    crate::components::widgets::Badge { count: Some(pending) }
                }
            }
        }
    }
}

/// The activity sheet itself: a bottom sheet on compact, a right side sheet on
/// medium+ (the app's standard sheet geometry). Rendered by `Layout` at the
/// app-shell root.
#[component]
pub fn ActivitySheet() -> Element {
    let session = use_session();
    if !session.read().is_authenticated() {
        return rsx! {};
    }
    let open = ACTIVITY_OPEN();

    rsx! {
        div {
            class: if open { "sheet-scrim open" } else { "sheet-scrim" },
            role: "presentation",
            onclick: move |_| close_activity(),
        }
        aside {
            class: if open { "tool-sheet activity-sheet open" } else { "tool-sheet activity-sheet" },
            role: "dialog",
            "aria-modal": "true",
            "aria-label": t("common.activity"),
            tabindex: "-1",
            onkeydown: move |e| {
                match e.key() {
                    Key::Escape => close_activity(),
                    Key::Tab
                        if crate::components::widgets::trap_tab_focus(
                            ".activity-sheet.open",
                            e.modifiers().shift(),
                        ) =>
                    {
                        e.prevent_default();
                    }
                    _ => {}
                }
            },
            // Focus sentinel: pulls focus into the sheet when it opens, so Escape
            // and Tab work without a click first.
            if open {
                div {
                    class: "sheet-focus-sentinel",
                    tabindex: "-1",
                    onmounted: move |e| {
                        spawn(async move {
                            let _ = e.set_focus(true).await;
                        });
                    },
                }
            }
            div { class: "tool-sheet-header",
                div { class: "sheet-handle" }
                span { class: "tool-sheet-icon material-icons", "notifications" }
                h3 { class: "title-medium", "{t(\"common.activity\")}" }
                button {
                    class: "btn-icon state-layer",
                    aria_label: t("common.close"),
                    onclick: move |_| close_activity(),
                    span { class: "material-icons", "close" }
                }
            }
            div { class: "tool-sheet-body activity-body",
                // Mounted only while open: the feed is a real query, and a closed
                // sheet has no business running it. Reopening therefore also
                // refreshes, which is what "what is new" should do.
                if open {
                    PendingInvitations {}
                    RecentContents {}
                }
            }
        }
    }
}

/// Invitations waiting for an answer, at the top of the sheet because they are
/// the only part of it that asks the reader to do something. Accepting or
/// declining happens in place (the same row the groups/events lists use).
#[component]
fn PendingInvitations() -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let email = session
        .read()
        .user
        .as_ref()
        .map(|u| u.email.clone())
        .unwrap_or_default();

    let invites = crate::use_data_resource!(|(token, user_id, email)| async move {
        let Some(uid) = user_id else {
            return Vec::new();
        };
        graphql::query_invitations(token.as_deref(), &uid, &email)
            .await
            .unwrap_or_default()
    });

    let list = invites.read().clone().unwrap_or_default();
    if list.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "activity-section",
            p { class: "title-small list-subheader", "{t(\"invite.invitations\")}" }
            div { class: "list",
                for invite in list.iter() {
                    crate::components::layout::InvitedContextItem {
                        key: "{invite.id.0}",
                        invite: invite.clone(),
                    }
                }
            }
        }
    }
}

/// How many feed rows to fetch per page.
const FEED_PAGE: i32 = 12;

/// The feed: everything recent the reader may see (content, comments and
/// reactions), newest first (#34).
///
/// Paged by an explicit "show more" rather than by scroll position: the sheet
/// scrolls in its own container, which the shell's window scroll listener (what
/// drives the pages elsewhere) cannot see.
#[component]
fn RecentContents() -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());

    // Pages accumulate here rather than in a resource: a resource re-runs and
    // replaces, which would drop everything already read past.
    let mut items = use_signal(Vec::<model::ChildNodeFields>::new);
    let mut loading = use_signal(|| false);
    // Cleared when a page comes back short: that is the end of the feed, and
    // without it "show more" would keep offering nothing.
    let mut has_more = use_signal(|| true);

    // First page, and a reset if the signed-in user changes.
    {
        let token = token.clone();
        let user_id = user_id.clone();
        use_effect(use_reactive!(|(token, user_id)| {
            items.set(Vec::new());
            has_more.set(true);
            let Some(uid) = user_id.clone() else { return };
            let token = token.clone();
            loading.set(true);
            spawn(async move {
                let page = graphql::query_recent_nodes(token.as_deref(), FEED_PAGE, 0, &uid).await;
                has_more.set(page.len() as i32 == FEED_PAGE);
                items.set(page);
                loading.set(false);
            });
        }));
    }

    let more = move |_| {
        if *loading.peek() || !*has_more.peek() {
            return;
        }
        let Some(uid) = user_id.clone() else { return };
        let token = token.clone();
        let offset = items.peek().len() as i32;
        loading.set(true);
        spawn(async move {
            let page = graphql::query_recent_nodes(token.as_deref(), FEED_PAGE, offset, &uid).await;
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
                        span { class: "material-icons", "schedule" }
                    }
                    p { class: "empty-state-body", "{t(\"layout.feedEmpty\")}" }
                }
            }
        };
    }

    rsx! {
        div { class: "activity-section",
            p { class: "title-small list-subheader", "{t(\"layout.feed\")}" }
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
                button { class: "btn btn-text", onclick: more, "{t(\"layout.showMore\")}" }
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
                // Following a row is going somewhere, so the overlay has done its
                // job and gets out of the way (the navigation below then lands on
                // an unobstructed page).
                close_activity();
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
