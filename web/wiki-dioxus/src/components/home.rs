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

    // EXPERIMENT (home hero): a time-aware greeting above the title, from the
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
                div { class: "card home-hero",
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
                    // Authors of the welcome (the root node's members).
                    if !members.is_empty() {
                        div { class: "chip-row", style: "padding: 12px 16px 0;",
                            for member in members.iter() {
                                super::widgets::Chip {
                                    key: "{member.id.0}",
                                    icon: super::loader::mime_icon(member.node.as_ref().and_then(|n| n.mime_id.as_deref()).unwrap_or("wiki/user")).to_string(),
                                    label: member.label(),
                                    title: t("member.author"),
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
                // drawer (which carries this list on desktop) is hidden. EXPERIMENT:
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

/// "Newest" — the most recently created content the user can see, each linking
/// to its full path (resolved lazily on click, like search). #34.
#[component]
fn RecentContents() -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());

    let recent = crate::use_data_resource!(|(token, user_id)| async move {
        let Some(user_id) = user_id else {
            return Vec::new();
        };
        graphql::query_recent_nodes(token.as_deref(), 8, &user_id).await
    });
    let items = recent.read().clone().unwrap_or_default();
    if items.is_empty() {
        return rsx! {};
    }

    rsx! {
        div { class: "card mt-2",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "schedule" } }
                h3 { class: "title-medium", "{t(\"layout.newestContent\")}" }
            }
            div { class: "list",
                for node in items.iter() {
                    RecentItem { key: "{node.id.0}", node: node.clone() }
                }
            }
        }
    }
}

#[component]
fn RecentItem(node: graphql::ChildNodeFields) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let node_id = node.id.0.clone();
    let key = node.key.clone();

    // Author (owner) name + initials for the social-style avatar.
    let author = node
        .owner
        .as_ref()
        .map(|o| o.display_name.clone())
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
        .unwrap_or_default();
    let owner_id = node.owner.as_ref().map(|o| o.id.0.clone());
    let author_name = author.clone().unwrap_or_else(|| t("common.unknown"));
    let created = node.created_at.as_ref().map(|t| t.0.clone());
    let parent_name = node.parent.as_ref().map(|p| p.name.clone());
    let mime = node.mime_id.clone().unwrap_or_default();
    let data = node.data.as_ref().map(|d| d.0.clone());

    rsx! {
        div {
            class: "recent-item",
            onclick: move |_| {
                let node_id = node_id.clone();
                let key = key.clone();
                let token = session.read().access_token.clone();
                // Resolve the node's full ancestor path, then navigate.
                spawn(async move {
                    let mut segments = graphql::path_from_id(token.as_deref(), &node_id)
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
                        span { class: "recent-author", "{a}" }
                    }
                    if let Some(iso) = created.as_ref() {
                        if author.is_some() {
                            span { class: "recent-sep", "\u{00b7}" }
                        }
                        span {
                            class: "recent-time",
                            title: "{super::loader::full_datetime(iso)}",
                            "{super::loader::relative_time(iso)}"
                        }
                    }
                }
                // What.
                div { class: "recent-title", "{node.name}" }
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
