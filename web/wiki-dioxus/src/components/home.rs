use dioxus::prelude::*;

use crate::graphql::{self, InvitationFields};
use crate::i18n::{t, t_with};
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

    rsx! {
        div { class: "grid grid-3",
            // Main content column
            div {
                div { class: "card",
                    div { class: "card-header",
                        div { class: "avatar", span { class: "material-icons", "waving_hand" } }
                        h3 { class: "headline-small", "{t(\"layout.welcomeTitle\")}" }
                        div { class: "flex-grow" }
                        // Owner-only: edit the welcome text (root node content).
                        if can_edit {
                            Link {
                                to: Route::EditWelcome {},
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
                // drawer (which carries this list on desktop) is hidden.
                if is_auth {
                    div { class: "home-mobile-list mt-1",
                        crate::components::layout::HomeList {}
                    }
                }
                // Newest content across the user's contexts (#34).
                if is_auth {
                    RecentContents {}
                }
            }

            // Sidebar column (invitations, etc.)
            if is_auth {
                div {
                    InvitesUserList {}
                }
            }
        }
    }
}

/// Owner-only editor for the root node's content (the welcome text). The normal
/// `?app=editor` route can't reach the root (it has no URL path, so
/// `resolve_path(&[])` is `None`), so this thin wrapper loads the root node and
/// hands it to the shared [`EditorApp`].
#[component]
pub fn EditWelcome() -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let root = crate::use_data_resource!(|(token)| async move {
        graphql::query_root_node(token.as_deref())
            .await
            .ok()
            .flatten()
    });

    let state = root.read().clone();
    match state {
        Some(Some(node)) => {
            let can_edit = node.is_owner.unwrap_or(false) || node.is_context_owner.unwrap_or(false);
            if can_edit {
                rsx! {
                    super::editor::EditorApp { node }
                }
            } else {
                rsx! {
                    div { class: "card",
                        div { class: "card-content",
                            p { class: "body-large", "{t(\"node.documentUnavailable\")}" }
                        }
                    }
                }
            }
        }
        Some(None) => rsx! {},
        None => rsx! {
            super::widgets::Spinner {}
        },
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
                div { class: "avatar", span { class: "material-icons", "schedule" } }
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
            // Author avatar (initials), like a social post header.
            div { class: "avatar small recent-avatar", "{initials}" }
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

/// The user's pending group/event invitations, with accept / decline — mirrors
/// the React `InvitesUserList`.
#[component]
fn InvitesUserList() -> Element {
    let session = use_session();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let email = session
        .read()
        .user
        .as_ref()
        .map(|u| u.email.clone())
        .unwrap_or_default();
    let access_token = session.read().access_token.clone();

    let refresh = use_signal(|| 0u32);

    // Live invitations: any change to this user's memberships re-runs the query.
    let sub_uid = user_id
        .clone()
        .unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".to_string());
    crate::subscription::use_live(
        format!(
            "subscription {{ members(where: {{ nodeId: {{ _eq: \"{sub_uid}\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    let invites = crate::use_data_resource!(move || {
        let token = access_token.clone();
        let user_id = user_id.clone();
        let email = email.clone();
        let _ = refresh.read();
        async move {
            let Some(user_id) = user_id else {
                return Vec::new();
            };
            graphql::query_invitations(token.as_deref(), &user_id, &email)
                .await
                .unwrap_or_default()
        }
    });

    let list = invites.read().clone();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "mail" } }
                h3 { class: "title-medium", "{t(\"invite.invitations\")}" }
            }
            {match list {
                None => rsx! {
                    div { class: "card-content",
                        p { class: "body-medium", class: "text-muted", "\u{2026}" }
                    }
                },
                Some(items) if items.is_empty() => rsx! {
                    div { class: "card-content",
                        p { class: "body-medium", class: "text-muted", "{t(\"invite.noInvitations\")}" }
                    }
                },
                Some(items) => rsx! {
                    div { class: "list",
                        for invite in items.iter() {
                            InviteItem { key: "{invite.id.0}", invite: invite.clone(), refresh }
                        }
                    }
                },
            }}
        }
    }
}

#[component]
fn InviteItem(invite: InvitationFields, refresh: Signal<u32>) -> Element {
    let session = use_session();
    let mut refresh = refresh;
    let name = invite
        .parent
        .as_ref()
        .map(|p| p.name.clone())
        .unwrap_or_default();
    let mime_id = invite
        .parent
        .as_ref()
        .and_then(|p| p.mime_id.clone())
        .unwrap_or_default();
    let member_id = invite.id.0.clone();

    let accept = {
        let member_id = member_id.clone();
        let parent_id = invite.parent.as_ref().map(|p| p.id.0.clone());
        move |_| {
            let token = session.read().access_token.clone();
            let uid = session.read().user.as_ref().map(|u| u.id.clone());
            let member_id = member_id.clone();
            let parent_id = parent_id.clone();
            spawn(async move {
                if let Some(uid) = uid {
                    // Accept by binding the invite to the user. If that fails —
                    // typically the (parentId, nodeId) unique constraint because
                    // the user already has a membership for this context — accept
                    // that existing row instead, then drop the duplicate invite.
                    // Ordering it this way never destroys the invite on a
                    // transient failure (safer than React's delete-then-accept).
                    let accepted = graphql::accept_invitation(token.as_deref(), &member_id, &uid)
                        .await
                        .unwrap_or(false);
                    if !accepted {
                        if let Some(pid) = parent_id {
                            if graphql::accept_existing_member(token.as_deref(), &pid, &uid)
                                .await
                                .unwrap_or(false)
                            {
                                let _ =
                                    graphql::decline_invitation(token.as_deref(), &member_id).await;
                            }
                        }
                    }
                    refresh += 1;
                }
            });
        }
    };
    let decline = {
        let member_id = member_id.clone();
        move |_| {
            let token = session.read().access_token.clone();
            let member_id = member_id.clone();
            spawn(async move {
                let _ = graphql::decline_invitation(token.as_deref(), &member_id).await;
                refresh += 1;
            });
        }
    };

    rsx! {
        div { class: "list-item",
            div { class: "avatar small secondary", {super::loader::icon_el(&mime_id)} }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{name}" }
            }
            button {
                class: "btn-icon",
                title: "{t_with(\"invite.acceptInvitation\", &[(\"name\", &name)])}",
                onclick: accept,
                span { class: "material-icons", "add" }
            }
            button {
                class: "btn-icon",
                title: "{t(\"common.delete\")}",
                onclick: decline,
                span { class: "material-icons", "close" }
            }
        }
    }
}
