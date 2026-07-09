use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// MemberApp — member list and invitation management
#[component]
pub fn MemberApp(node: NodeWithChildren) -> Element {
    let name = node.name.as_str();
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let node_id = node.id.0.clone();
    let mut invite_input = use_signal(String::new);

    // The context owner may hide/unhide members (#51). Others don't see hidden
    // members at all.
    let is_owner = user_id.is_some() && node.owner_id.as_ref().map(|o| o.0.clone()) == user_id;
    let members: Vec<&graphql::MemberFields> = node
        .members
        .iter()
        .filter(|m| is_owner || !m.hidden)
        .collect();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "group" } }
                div {
                    h3 { class: "title-medium", "{name}" }
                    p { class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.members\")}"
                    }
                }
            }

            // Member list (the node's actual memberships, not its children).
            if members.is_empty() {
                div { class: "card-content",
                    p { class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                div { class: "list",
                    for member in members.iter() {
                        div {
                            class: "list-item",
                            key: "{member.id.0}",
                            style: if member.hidden { "opacity: 0.55;" } else { "" },
                            div { class: "avatar small", span { class: "material-icons", "person" } }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{member.label()}" }
                                div { class: "list-item-secondary",
                                    if member.hidden {
                                        span { class: "material-icons", style: "font-size: 14px; vertical-align: middle;", "visibility_off" }
                                        " {t(\"member.hidden\")}"
                                    } else if member.owner {
                                        "{t(\"member.owner\")}"
                                    } else if member.accepted {
                                        "{t(\"member.active\")}"
                                    } else {
                                        "{t(\"invite.invitations\")}"
                                    }
                                }
                            }
                            // Owner can hide/unhide a non-owner member (#51).
                            if is_owner && !member.owner {
                                {
                                    let member_id = member.id.0.clone();
                                    let hidden = member.hidden;
                                    let token = session.read().access_token.clone();
                                    rsx! {
                                        button {
                                            class: "btn-icon",
                                            title: if hidden { "{t(\"member.show\")}" } else { "{t(\"member.hide\")}" },
                                            onclick: move |_| {
                                                let token = token.clone();
                                                let id = member_id.clone();
                                                spawn(async move {
                                                    let _ = graphql::set_member_hidden(token.as_deref(), &id, !hidden).await;
                                                    if let Some(w) = web_sys::window() {
                                                        let _ = w.location().reload();
                                                    }
                                                });
                                            },
                                            if hidden { span { class: "material-icons", "visibility" } } else { span { class: "material-icons", "visibility_off" } }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Invite input
            if is_auth {
                div { class: "card-content",
                    div { class: "text-field",
                        label { "{t(\"invite.nameOrEmail\")}" }
                        input {
                            r#type: "text",
                            placeholder: "{t(\"invite.nameOrEmail\")}",
                            value: "{invite_input}",
                            oninput: move |evt| invite_input.set(evt.value()),
                        }
                    }
                    button {
                        class: "btn btn-primary mt-1",
                        disabled: invite_input.read().is_empty(),
                        onclick: {
                            let node_id = node_id.clone();
                            move |_| {
                                let email = invite_input.read().trim().to_string();
                                if email.is_empty() {
                                    return;
                                }
                                let token = session.read().access_token.clone();
                                let node_id = node_id.clone();
                                invite_input.set(String::new());
                                spawn(async move {
                                    match graphql::invite_member(token.as_deref(), &node_id, &email).await {
                                        Ok(true) => show_snackbar(&t("invite.invite")),
                                        _ => show_snackbar(&t("error.somethingWentWrong")),
                                    }
                                });
                            }
                        },
                        "{t(\"invite.invite\")}"
                    }
                }
            }
        }
    }
}
