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
                // No feed here: it is an app of the root now (`/?app=feed`, first
                // on the rail), which is everything recent across the groups and
                // events you belong to. Repeating it on the page under it would
                // be the same list at two addresses.
            }
        }
    }
}
