use dioxus::prelude::*;

use crate::graphql::{self, InvitationFields};
use crate::i18n::{t, t_with};
use crate::route::Route;
use crate::session::use_session;

#[component]
pub fn HomeApp() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let display_name = session
        .read()
        .user
        .as_ref()
        .map(|u| u.display_name.clone())
        .unwrap_or_default();

    rsx! {
        div { class: "grid grid-3",
            // Main content column
            div {
                div { class: "card",
                    div { class: "card-header",
                        div { class: "avatar", "\u{1F44B}" }
                        h3 { class: "headline-small", "{t(\"layout.welcomeTitle\")}" }
                    }
                    div { class: "card-content",
                        if !is_auth {
                            p { class: "body-large mb-1", "{t(\"layout.loginOrRegister\")}" }
                            p { class: "body-medium mb-2", "{t(\"layout.rememberEmail\")}" }
                            div { class: "stack stack-h",
                                Link {
                                    to: Route::Login {},
                                    class: "btn btn-outlined",
                                    "\u{1F511} {t(\"common.logIn\")}"
                                }
                                Link {
                                    to: Route::Register {},
                                    class: "btn btn-outlined",
                                    "\u{1F464} {t(\"auth.register\")}"
                                }
                            }
                        } else {
                            p { class: "body-large mb-1",
                                "{t_with(\"layout.greeting\", &[(\"name\", &display_name)])}"
                            }
                            p { class: "body-large mb-1", "{t(\"layout.acceptInvitations\")}" }
                            p { class: "body-medium", "{t(\"layout.noInvitationsHint\")}" }
                        }
                    }
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

    let invites = use_resource(move || {
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
                div { class: "avatar", "\u{2709}\u{FE0F}" }
                h3 { class: "title-medium", "{t(\"invite.invitations\")}" }
            }
            {match list {
                None => rsx! {
                    div { class: "card-content",
                        p { class: "body-medium", style: "color: var(--md-on-surface-variant);", "\u{2026}" }
                    }
                },
                Some(items) if items.is_empty() => rsx! {
                    div { class: "card-content",
                        p { class: "body-medium", style: "color: var(--md-on-surface-variant);", "{t(\"invite.noInvitations\")}" }
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
        move |_| {
            let token = session.read().access_token.clone();
            let uid = session.read().user.as_ref().map(|u| u.id.clone());
            let member_id = member_id.clone();
            spawn(async move {
                if let Some(uid) = uid {
                    let _ = graphql::accept_invitation(token.as_deref(), &member_id, &uid).await;
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
            div { class: "avatar small secondary", "{super::loader::mime_icon(&mime_id)}" }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{name}" }
            }
            button {
                class: "btn-icon",
                title: "{t_with(\"invite.acceptInvitation\", &[(\"name\", &name)])}",
                onclick: accept,
                "\u{2795}"
            }
            button {
                class: "btn-icon",
                title: "{t(\"common.delete\")}",
                onclick: decline,
                "\u{2715}"
            }
        }
    }
}
