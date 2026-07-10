use dioxus::prelude::*;

use crate::graphql::{self, MemberFields, MembersSetInput, NodeWithChildren};
use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

use super::ui::alert_dialog::{
    AlertDialog, AlertDialogAction, AlertDialogActions, AlertDialogCancel, AlertDialogDescription,
    AlertDialogTitle,
};

/// MemberApp — member roster + invitation management. Owners (direct or context)
/// get the full MembersDataGrid admin: hide/unhide, promote/demote, (de)activate,
/// edit name/email, and remove a member.
#[component]
pub fn MemberApp(node: NodeWithChildren) -> Element {
    let name = node.name.clone();
    let session = use_session();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let node_id = node.id.0.clone();
    let mut invite_input = use_signal(String::new);

    // A direct owner (owner_id) or a context owner may manage the roster; others
    // only see the non-hidden members.
    let is_owner = user_id.is_some() && node.owner_id.as_ref().map(|o| o.0.clone()) == user_id;
    let can_manage = is_owner || node.is_context_owner.unwrap_or(false);

    let members: Vec<MemberFields> = node
        .members
        .iter()
        .filter(|m| can_manage || !m.hidden)
        .cloned()
        .collect();

    // Edit dialog (name/email) and remove confirm, shared across the rows.
    let mut edit_id = use_signal(|| Option::<String>::None);
    let mut edit_name = use_signal(String::new);
    let mut edit_email = use_signal(String::new);
    let mut remove_target = use_signal(|| Option::<(String, String)>::None);

    let save_edit = move |_| {
        let Some(id) = edit_id.read().clone() else {
            return;
        };
        let token = session.read().access_token.clone();
        let name = edit_name.read().trim().to_string();
        let email = edit_email.read().trim().to_string();
        spawn(async move {
            let set = MembersSetInput {
                name: (!name.is_empty()).then_some(name),
                email: (!email.is_empty()).then_some(email),
                ..Default::default()
            };
            match graphql::update_member(token.as_deref(), &id, set).await {
                Ok(true) => {
                    crate::session::bump_data_version();
                    edit_id.set(None);
                }
                _ => show_snackbar(&t("error.somethingWentWrong")),
            }
        });
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "group" } }
                div {
                    h3 { class: "title-medium", "{name}" }
                    p { class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.members\")}"
                    }
                }
            }

            // Member list (the node's actual memberships, not its children).
            if members.is_empty() {
                div { class: "card-content",
                    p { class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                div { class: "list",
                    for member in members.iter() {
                        MemberRow {
                            key: "{member.id.0}",
                            member: member.clone(),
                            can_manage,
                            on_edit: move |m: MemberFields| {
                                edit_name.set(m.name.clone().unwrap_or_default());
                                edit_email.set(m.email.clone().unwrap_or_default());
                                edit_id.set(Some(m.id.0.clone()));
                            },
                            on_remove: move |m: MemberFields| {
                                remove_target.set(Some((m.id.0.clone(), m.label())));
                            },
                        }
                    }
                }
            }

            // Invite input (owner action, mirroring React InvitesFab).
            if can_manage {
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
                                        Ok(true) => {
                                            show_snackbar(&t("invite.invite"));
                                            crate::session::bump_data_version();
                                        }
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

        // Edit member (name / email) modal.
        if edit_id.read().is_some() {
            div { class: "modal-backdrop", onclick: move |_| edit_id.set(None),
                div {
                    class: "modal-card",
                    onclick: move |e| e.stop_propagation(),
                    h3 { class: "title-medium mb-2", "{t(\"member.edit\")}" }
                    div { class: "text-field",
                        label { "{t(\"member.name\")}" }
                        input {
                            r#type: "text",
                            value: "{edit_name}",
                            oninput: move |e| edit_name.set(e.value()),
                        }
                    }
                    div { class: "text-field mt-2",
                        label { "{t(\"member.email\")}" }
                        input {
                            r#type: "email",
                            value: "{edit_email}",
                            oninput: move |e| edit_email.set(e.value()),
                        }
                    }
                    div { class: "stack stack-h mt-2", style: "align-items: center; gap: 8px;",
                        div { class: "flex-grow" }
                        button {
                            class: "btn btn-outlined",
                            onclick: move |_| edit_id.set(None),
                            "{t(\"common.cancel\")}"
                        }
                        button { class: "btn btn-primary", onclick: save_edit, "{t(\"common.save\")}" }
                    }
                }
            }
        }

        // Remove member confirm.
        AlertDialog {
            open: Some(remove_target.read().is_some()),
            on_open_change: move |v: bool| {
                if !v {
                    remove_target.set(None);
                }
            },
            AlertDialogTitle { "{t(\"member.confirmRemove\")}" }
            AlertDialogDescription {
                if let Some((_, label)) = remove_target.read().clone() {
                    "{label}"
                }
            }
            AlertDialogActions {
                AlertDialogCancel { "{t(\"common.cancel\")}" }
                AlertDialogAction {
                    on_click: move |_| {
                        let Some((id, _)) = remove_target.read().clone() else {
                            return;
                        };
                        let token = session.read().access_token.clone();
                        remove_target.set(None);
                        spawn(async move {
                            match graphql::remove_member(token.as_deref(), &id).await {
                                Ok(true) => crate::session::bump_data_version(),
                                _ => show_snackbar(&t("error.somethingWentWrong")),
                            }
                        });
                    },
                    "{t(\"common.delete\")}"
                }
            }
        }
    }
}

/// One roster row: identity + status, and (for owners) the admin controls that
/// toggle hidden/owner/active in place and raise edit / remove to the parent.
#[component]
fn MemberRow(
    member: MemberFields,
    can_manage: bool,
    on_edit: EventHandler<MemberFields>,
    on_remove: EventHandler<MemberFields>,
) -> Element {
    let session = use_session();
    let mid = member.id.0.clone();
    let owner = member.owner;
    let active = member.active;
    let hidden = member.hidden;

    rsx! {
        div {
            class: "list-item",
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
            if can_manage {
                // Promote / demote owner.
                button {
                    class: "btn-icon",
                    title: if owner { "{t(\"member.demote\")}" } else { "{t(\"member.promote\")}" },
                    onclick: {
                        let mid = mid.clone();
                        move |_| apply_member_update(
                            session.read().access_token.clone(),
                            mid.clone(),
                            MembersSetInput { owner: Some(!owner), ..Default::default() },
                        )
                    },
                    span { class: "material-icons", if owner { "star" } else { "star_outline" } }
                }
                // Mark active / inactive (attendance).
                button {
                    class: "btn-icon",
                    title: if active { "{t(\"member.deactivate\")}" } else { "{t(\"member.activate\")}" },
                    onclick: {
                        let mid = mid.clone();
                        move |_| apply_member_update(
                            session.read().access_token.clone(),
                            mid.clone(),
                            MembersSetInput { active: Some(!active), ..Default::default() },
                        )
                    },
                    span { class: "material-icons",
                        if active { "check_circle" } else { "radio_button_unchecked" }
                    }
                }
                // Hide / show within the context (#51).
                button {
                    class: "btn-icon",
                    title: if hidden { "{t(\"member.show\")}" } else { "{t(\"member.hide\")}" },
                    onclick: {
                        let mid = mid.clone();
                        move |_| apply_member_update(
                            session.read().access_token.clone(),
                            mid.clone(),
                            MembersSetInput { hidden: Some(!hidden), ..Default::default() },
                        )
                    },
                    span { class: "material-icons", if hidden { "visibility" } else { "visibility_off" } }
                }
                // Edit name / email.
                button {
                    class: "btn-icon",
                    title: "{t(\"member.edit\")}",
                    onclick: {
                        let member = member.clone();
                        move |_| on_edit.call(member.clone())
                    },
                    span { class: "material-icons", "edit" }
                }
                // Remove from the context.
                button {
                    class: "btn-icon",
                    title: "{t(\"member.remove\")}",
                    onclick: {
                        let member = member.clone();
                        move |_| on_remove.call(member.clone())
                    },
                    span { class: "material-icons", "person_remove" }
                }
            }
        }
    }
}

/// Apply a member update (a single toggled field) and refresh the roster. Kept
/// as a free helper so each toolbar button can call it without sharing a closure.
fn apply_member_update(token: Option<String>, id: String, set: MembersSetInput) {
    spawn(async move {
        match graphql::update_member(token.as_deref(), &id, set).await {
            Ok(true) => crate::session::bump_data_version(),
            _ => show_snackbar(&t("error.somethingWentWrong")),
        }
    });
}
