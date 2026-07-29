use crate::model;
use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

#[component]
pub fn HomeApp() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();

    // The welcome text is the root node's content, editable by its owner. It
    // refetches after an edit (use_data_resource tracks the global data version).
    let token = session.read().access_token.clone();
    let root = crate::use_data_resource!(|(token)| async move {
        graphql::query_root_node(token.as_deref())
            .await
            .ok()
            .flatten()
    });
    let root_node = root.read().clone().flatten();
    let can_edit = root_node
        .as_ref()
        .map(|n| n.is_owner.unwrap_or(false) || n.is_context_owner.unwrap_or(false))
        .unwrap_or(false);
    let welcome_data = root_node.as_ref().and_then(|n| n.data.clone()).map(|d| d.0);
    let has_welcome = welcome_data
        .as_ref()
        .and_then(|d| d.get("content"))
        .and_then(|c| c.as_array())
        .is_some_and(|a| !a.is_empty());
    let members: Vec<_> = root_node
        .as_ref()
        .map(|n| n.members.iter().filter(|m| !m.hidden).cloned().collect())
        .unwrap_or_default();
    // The header title is the home (root) node's own name, falling back to the
    // default welcome string until the node has a name.
    let title = root_node
        .as_ref()
        .map(|n| n.name.clone())
        .filter(|s| !s.trim().is_empty())
        .unwrap_or_else(|| t("layout.welcomeTitle"));

    // DESIGN (home hero): a time-aware greeting above the title, from the
    // browser's local hour, with an animated waving hand in a tonal hero header.
    let greeting_key = {
        let hour = js_sys::Date::new_0().get_hours();
        if hour < 5 {
            "layout.greetNight"
        } else if hour < 12 {
            "layout.greetMorning"
        } else if hour < 18 {
            "layout.greetAfternoon"
        } else {
            "layout.greetEvening"
        }
    };

    rsx! {
        div { class: "grid grid-3",
            // Main content column
            div {
                div { class: "card",
                    div { class: "home-hero-head",
                        div { class: "home-hero-icon",
                            span { class: "material-icons", "waving_hand" }
                        }
                        div { class: "home-hero-text",
                            p { class: "home-hero-greeting", "{t(greeting_key)}" }
                            h3 { class: "home-hero-title", "{title}" }
                        }
                        div { class: "flex-grow" }
                        // Owner-only: edit the welcome text (root node content).
                        if can_edit {
                            Link {
                                to: Route::Home { app: Some("editor".to_string()) },
                                class: "btn-icon",
                                title: "{t(\"mime.editor\")}",
                                span { class: "material-icons", "edit" }
                            }
                        }
                    }
                    // Authors of the welcome (the root node's members). Each chip
                    // opens the identity popover (profile link etc.), same as the
                    // author chips on content nodes.
                    if !members.is_empty() {
                        div { class: "chip-row chip-row-authors",
                            for member in members.iter() {
                                super::loader::UserPopover {
                                    key: "{member.id.0}",
                                    name: member.label(),
                                    avatar_url: member.user.as_ref().map(|u| u.avatar_url.clone()).unwrap_or_default(),
                                    user_id: member.user.as_ref().map(|u| u.id.0.clone()),
                                    super::widgets::Chip {
                                        icon: super::loader::mime_icon(member.node.as_ref().and_then(|n| n.mime_id.as_deref()).unwrap_or("wiki/user")).to_string(),
                                        label: member.label(),
                                        title: t("member.author"),
                                        // The author's profile picture (e.g. their
                                        // linked Bluesky avatar) shows on the chip.
                                        avatar_url: member.user.as_ref().map(|u| u.avatar_url.clone()),
                                    }
                                }
                            }
                        }
                    }
                    div { class: "card-content",
                        // The editable welcome: the root node's content, or the
                        // original static copy until an owner writes one.
                        if has_welcome {
                            super::content::SlateRenderer { data: welcome_data.clone() }
                        } else if is_auth {
                            p { class: "body-large mb-1", "{t(\"layout.acceptInvitations\")}" }
                            p { class: "body-medium", "{t(\"layout.noInvitationsHint\")}" }
                        } else {
                            p { class: "body-large mb-1", "{t(\"layout.loginOrRegister\")}" }
                            p { class: "body-medium mb-2", "{t(\"layout.rememberEmail\")}" }
                        }
                        if !is_auth {
                            div { class: "stack stack-h mt-2",
                                Link {
                                    to: Route::Login {},
                                    class: "btn btn-outlined",
                                    span { class: "material-icons", "login" }
                                    " {t(\"common.logIn\")}"
                                }
                                Link {
                                    to: Route::Register {},
                                    class: "btn btn-outlined",
                                    span { class: "material-icons", "person_add" }
                                    " {t(\"auth.register\")}"
                                }
                            }
                        }
                    }
                }
                // The user's groups/events — shown here only on mobile, where the
                // drawer (which carries this list on desktop) is hidden. DESIGN:
                // as_cards renders Groups and Events as two separate home cards.
                if is_auth {
                    div { class: "home-mobile-list mt-1",
                        crate::components::layout::HomeList { as_cards: true }
                    }
                }
                // Newest content across the user's contexts (#34). Pending group /
                // event invitations now appear inline in the groups/events lists
                // (HomeList), so there is no separate invitations card.
                if is_auth {
                    RecentContents {}
                }
            }
        }
    }
}

/// How many feed rows to fetch per page.
const FEED_PAGE: i32 = 12;

/// The activity feed: everything recent the reader may see — content, comments
/// and reactions — newest first, paged in as they scroll (#34).
#[component]
fn RecentContents() -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());

    // Pages accumulate here rather than in a resource: a resource re-runs and
    // replaces, which would drop everything already scrolled past.
    let mut items = use_signal(Vec::<model::ChildNodeFields>::new);
    let mut loading = use_signal(|| false);
    // Cleared when a page comes back short — that is the end of the feed, and
    // without it the sentinel would keep asking forever.
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

    // Next page, once the reader nears the end. `near_bottom` is driven by the
    // shell's single scroll listener.
    {
        let token = token.clone();
        let user_id = user_id.clone();
        let near = crate::components::back_to_top::near_bottom();
        use_effect(use_reactive!(|(near, token, user_id)| {
            if !near || *loading.peek() || !*has_more.peek() {
                return;
            }
            let Some(uid) = user_id.clone() else { return };
            let token = token.clone();
            let offset = items.peek().len() as i32;
            loading.set(true);
            spawn(async move {
                let page =
                    graphql::query_recent_nodes(token.as_deref(), FEED_PAGE, offset, &uid).await;
                has_more.set(page.len() as i32 == FEED_PAGE);
                items.write().extend(page);
                loading.set(false);
            });
        }));
    }

    let rows = items.read().clone();
    if rows.is_empty() && !*loading.read() {
        return rsx! {};
    }

    rsx! {
        div { class: "card mt-2",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "schedule" } }
                h3 { class: "title-medium", "{t(\"layout.feed\")}" }
            }
            div { class: "list",
                for node in rows.iter() {
                    RecentItem { key: "{node.id.0}", node: node.clone() }
                }
            }
            if *loading.read() {
                div { class: "empty-state empty-state-sm",
                    div { class: "spinner spinner-sm" }
                }
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
    // reply answers — quoted beneath, so the row stands on its own.
    let about = if is_reaction || is_comment {
        node.parent
            .as_ref()
            .filter(|p| p.mime_id.as_deref() == Some("vote/comment"))
            .and_then(|p| comment_text(p.data.as_ref()))
    } else {
        None
    };
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
                // What it was about: the comment reacted to, or replied to.
                if let Some(quote) = about.as_ref() {
                    blockquote { class: "recent-quote", "{quote}" }
                }
                // The opening of the content itself, clamped by CSS.
                if let Some(text) = excerpt.as_ref() {
                    p { class: "recent-excerpt", "{text}" }
                }
                // Where (the content type + its context).
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
