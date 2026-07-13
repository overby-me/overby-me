use dioxus::prelude::*;

use crate::graphql::{self, MemberFields, MembersSetInput, NodeWithChildren};
use crate::i18n::{t, t_with};
use crate::session::use_session;
use crate::snackbar::show_snackbar;

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

    // Roster paging: a search box + a single-select filter + a page, all resolved
    // SERVER-SIDE (Hasura limit/offset/where + an aggregate count) so the roster
    // scales past thousands of members without loading them all.
    const PAGE_SIZE: usize = 25;
    let search = use_signal(String::new);
    let filter = use_signal(|| "all".to_string());
    let page = use_signal(|| 0usize);

    let member_filter = {
        let mut mf = graphql::MemberPageFilter {
            search: search.read().clone(),
            ..Default::default()
        };
        match filter.read().as_str() {
            "owners" => mf.owner = Some(true),
            "active" => mf.active = Some(true),
            "invited" => {
                mf.accepted = Some(false);
                mf.hidden = Some(false);
            }
            "hidden" => mf.hidden = Some(true),
            _ => {}
        }
        // Non-managers never see hidden members.
        if !can_manage && mf.hidden.is_none() {
            mf.hidden = Some(false);
        }
        mf
    };

    let token = session.read().access_token.clone();
    let cur_page = *page.read();
    let node_key = node_id.clone();
    let roster =
        crate::use_data_resource!(|(node_key, member_filter, cur_page, token)| async move {
            graphql::query_members_page(
                token.as_deref(),
                &node_key,
                &member_filter,
                PAGE_SIZE,
                cur_page * PAGE_SIZE,
            )
            .await
            .unwrap_or_default()
        });
    let (rows, total) = roster.read().clone().unwrap_or_default();

    // Edit dialog (name/email) and remove confirm, shared across the rows.
    let mut edit_id = use_signal(|| Option::<String>::None);
    let mut edit_name = use_signal(String::new);
    let mut edit_email = use_signal(String::new);
    let mut remove_target = use_signal(|| Option::<(String, String)>::None);
    // Invite-by-name autocomplete: matching users, and a monotonic request id so
    // out-of-order search responses don't clobber a newer one.
    let mut user_matches = use_signal(Vec::<graphql::Author>::new);
    let mut search_seq = use_signal(|| 0u32);

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
        super::widgets::SupportingPaneLayout {
            primary: rsx! {
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
                        div { class: "flex-grow" }
                        // Roster actions in the M3 tools sheet (CSV export, #41).
                        // Export pulls the FULL roster server-side (the table itself
                        // only holds one page).
                        if can_manage {
                            super::widgets::ToolSheet {
                                title: t("common.tools"),
                                button {
                                    class: "sheet-action",
                                    onclick: {
                                        let fname = name.clone();
                                        let export_id = node_id.clone();
                                        move |_| {
                                            let token = session.read().access_token.clone();
                                            let fname = fname.clone();
                                            let export_id = export_id.clone();
                                            spawn(async move {
                                                let (all, _) = graphql::query_members_page(
                                                    token.as_deref(),
                                                    &export_id,
                                                    &graphql::MemberPageFilter::default(),
                                                    100_000,
                                                    0,
                                                )
                                                .await
                                                .unwrap_or_default();
                                                let mut csv = String::from("Name,Email\n");
                                                for m in &all {
                                                    csv.push_str(&csv_field(&m.label()));
                                                    csv.push(',');
                                                    csv.push_str(&csv_field(m.email.as_deref().unwrap_or("")));
                                                    csv.push('\n');
                                                }
                                                let file = format!("{}-participants.csv", crate::export::sanitize_filename(&fname));
                                                crate::export::download_bytes(&file, "text/csv;charset=utf-8", csv.as_bytes());
                                            });
                                        }
                                    },
                                    span { class: "material-icons", "download" }
                                    "{t(\"member.export\")}"
                                }
                            }
                        }
                    }

                    // Member roster: a SERVER-paginated, searchable, filterable
                    // table (scales to thousands via Hasura limit/offset/where),
                    // rendered through the reusable widgets::PaginatedTable.
                    super::widgets::PaginatedTable {
                        columns: member_columns(can_manage),
                        filters: member_filters(can_manage),
                        search,
                        filter,
                        page,
                        page_size: PAGE_SIZE,
                        total,
                        search_placeholder: t("member.search"),
                        prev_label: t("common.previous"),
                        next_label: t("common.next"),
                        for m in rows.iter() {
                            MemberTableRow {
                                key: "{m.id.0}",
                                member: m.clone(),
                                can_manage,
                                on_edit: move |mm: MemberFields| {
                                    edit_name.set(mm.name.clone().unwrap_or_default());
                                    edit_email.set(mm.email.clone().unwrap_or_default());
                                    edit_id.set(Some(mm.id.0.clone()));
                                },
                                on_remove: move |mm: MemberFields| {
                                    remove_target.set(Some((mm.id.0.clone(), mm.label())));
                                },
                            }
                        }
                    }
                }
            },
            supporting: rsx! {
                // Invite input (owner action, mirroring React InvitesFab).
                if can_manage {
                    div { class: "card",
                        // EXPERIMENT: give the invite card a header identity like the
                        // other cards (it was headerless).
                        div { class: "card-header",
                            div { class: "avatar small",
                                span { class: "material-icons", "person_add" }
                            }
                            h3 { class: "title-medium", "{t(\"invite.invite\")}" }
                        }
                        div { class: "card-content",
                            div { class: "text-field",
                                label { "{t(\"invite.nameOrEmail\")}" }
                                input {
                                    r#type: "text",
                                    placeholder: "{t(\"invite.nameOrEmail\")}",
                                    value: "{invite_input}",
                                    oninput: move |evt| {
                                        let q = evt.value();
                                        invite_input.set(q.clone());
                                        // Autocomplete known users by name; an email (with
                                        // '@') falls through to the email-invite button.
                                        if q.trim().is_empty() || q.contains('@') {
                                            user_matches.set(vec![]);
                                            return;
                                        }
                                        let token = session.read().access_token.clone();
                                        let seq = *search_seq.read() + 1;
                                        search_seq.set(seq);
                                        spawn(async move {
                                            let results = graphql::search_users(token.as_deref(), &q).await;
                                            if *search_seq.read() == seq {
                                                user_matches.set(results);
                                            }
                                        });
                                    },
                                }
                            }
                            // Matching users — click to invite by node id (binds the user).
                            if !user_matches.read().is_empty() {
                                div { class: "list",
                                    for u in user_matches.read().iter() {
                                        {
                                            let uname = u.name.clone();
                                            let nid = u.node_id.clone();
                                            let parent = node_id.clone();
                                            let avatar_url = u.avatar_url.clone();
                                            rsx! {
                                                button {
                                                    key: "{u.node_id.clone().unwrap_or_default()}",
                                                    class: "list-item list-button",
                                                    onclick: move |_| {
                                                        let Some(nid) = nid.clone() else {
                                                            return;
                                                        };
                                                        let token = session.read().access_token.clone();
                                                        let parent = parent.clone();
                                                        let uname = uname.clone();
                                                        invite_input.set(String::new());
                                                        user_matches.set(vec![]);
                                                        spawn(async move {
                                                            match graphql::invite_member_by_node(token.as_deref(), &parent, &nid, &uname).await {
                                                                Ok(true) => {
                                                                    show_snackbar(&t("invite.invite"));
                                                                    crate::session::bump_data_version();
                                                                }
                                                                _ => show_snackbar(&t("error.somethingWentWrong")),
                                                            }
                                                        });
                                                    },
                                                    div { class: "avatar small",
                                                        {super::loader::user_avatar(&avatar_url, rsx! { span { class: "material-icons", "person" } })}
                                                    }
                                                    div { class: "list-item-text",
                                                        div { class: "list-item-primary", "{u.name}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
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
                            // Bulk-import a Fornavn/Efternavn/Email roster from an .xlsx
                            // (React InvitesFab). Each row with an email becomes an invite.
                            div { class: "mt-2",
                                div { class: "file-upload-label", "{t(\"invite.importRoster\")}" }
                                input {
                                    id: "roster-xlsx-input",
                                    class: "file-upload-input",
                                    r#type: "file",
                                    accept: ".xlsx,application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                                    onchange: {
                                        let node_id = node_id.clone();
                                        move |evt: FormEvent| {
                                            let files = evt.files();
                                            let Some(fd) = files.into_iter().next() else {
                                                return;
                                            };
                                            let token = session.read().access_token.clone();
                                            let node_id = node_id.clone();
                                            spawn(async move {
                                                let Ok(bytes) = fd.read_bytes().await else {
                                                    show_snackbar(&t("error.somethingWentWrong"));
                                                    return;
                                                };
                                                let roster: Vec<(String, String)> =
                                                    crate::roster::parse_member_roster(bytes.to_vec())
                                                        .into_iter()
                                                        .map(|e| (e.name, e.email))
                                                        .collect();
                                                if roster.is_empty() {
                                                    show_snackbar(&t("invite.noRosterRows"));
                                                    return;
                                                }
                                                match graphql::invite_members(token.as_deref(), &node_id, &roster).await {
                                                    Ok(n) if n > 0 => {
                                                        show_snackbar(&t_with("invite.imported", &[("count", &n.to_string())]));
                                                        crate::session::bump_data_version();
                                                    }
                                                    _ => show_snackbar(&t("error.somethingWentWrong")),
                                                }
                                            });
                                        }
                                    },
                                }
                                label { r#for: "roster-xlsx-input", class: "file-upload",
                                    span { class: "material-icons", "table_view" }
                                    span { class: "file-upload-text", "{t(\"content.chooseFile\")}" }
                                }
                            }
                        }
                    }
                }
            },
        }

        // Edit member (name / email) dialog.
        super::widgets::Dialog {
            open: edit_id.read().is_some(),
            on_dismiss: move |_| edit_id.set(None),
            headline: t("member.edit"),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| edit_id.set(None),
                    "{t(\"common.cancel\")}"
                }
                button { class: "btn btn-primary", onclick: save_edit, "{t(\"common.save\")}" }
            },
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
        }

        // Remove member confirm. Uses the custom Dialog (plain `bool` open): the
        // primitives AlertDialog's controlled `open` prop never opened here.
        super::widgets::Dialog {
            open: remove_target.read().is_some(),
            on_dismiss: move |_| remove_target.set(None),
            headline: t("member.confirmRemove"),
            icon: "person_remove".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| remove_target.set(None),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    onclick: move |_| {
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
            },
            if let Some((_, label)) = remove_target.read().clone() {
                p { class: "body-medium", "{label}" }
            }
        }
    }
}

/// The roster table's columns (adds an Actions column for managers).
fn member_columns(can_manage: bool) -> Vec<String> {
    let mut c = vec![t("member.name"), t("member.email"), t("member.status")];
    if can_manage {
        c.push(t("member.actions"));
    }
    c
}

/// The roster filter chips as `(value, label)` (Hidden only for managers). The
/// values map to the server-side predicate built in `MemberApp`.
fn member_filters(can_manage: bool) -> Vec<(String, String)> {
    let mut f = vec![
        ("all".to_string(), t("member.filterAll")),
        ("owners".to_string(), t("member.owner")),
        ("active".to_string(), t("member.active")),
        ("invited".to_string(), t("invite.invitations")),
    ];
    if can_manage {
        f.push(("hidden".to_string(), t("member.hidden")));
    }
    f
}

/// The M3 status chip (icon + label) for a member's state: hidden > owner >
/// active > pending-invitation.
fn member_status(m: &MemberFields) -> (&'static str, String) {
    if m.hidden {
        ("visibility_off", t("member.hidden"))
    } else if m.owner {
        ("star", t("member.owner"))
    } else if m.accepted {
        ("check_circle", t("member.active"))
    } else {
        ("mail", t("invite.invitations"))
    }
}

/// One member table row (`<tr>`): identity, email, status chip, and — for owners
/// — the inline admin controls (promote/activate/hide/edit/remove).
#[component]
fn MemberTableRow(
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
    let (status_icon, status_label) = member_status(&member);
    let avatar_url = member
        .user
        .as_ref()
        .map(|u| u.avatar_url.clone())
        .unwrap_or_default();
    let user_id = member.user.as_ref().map(|u| u.id.0.clone());
    let member_name = member.label();
    // EXPERIMENT: colour-code the status chip by category.
    let status_class = match status_icon {
        "check_circle" => "status-active",
        "mail" => "status-pending",
        "star" => "status-owner",
        _ => "",
    };

    rsx! {
        tr { class: if hidden { "member-row-hidden" } else { "" },
            td {
                span { class: "m3-cell-icon",
                    super::loader::UserPopover {
                        name: member_name.clone(),
                        avatar_url: avatar_url.clone(),
                        user_id: user_id.clone(),
                        role: Some(status_label.clone()),
                        div { class: "avatar small",
                            {super::loader::user_avatar(&avatar_url, rsx! { span { class: "material-icons", "person" } })}
                        }
                    }
                    span { "{member_name}" }
                }
            }
            td { class: "member-email-cell", "{member.email.clone().unwrap_or_default()}" }
            td {
                span { class: "member-status {status_class}",
                    super::widgets::Chip { icon: status_icon.to_string(), label: status_label }
                }
            }
            if can_manage {
                td {
                    div { class: "member-row-actions",
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
    }
}

/// Quote a CSV field when it contains a comma, quote, or newline (RFC 4180:
/// wrap in quotes and double any inner quotes).
fn csv_field(s: &str) -> String {
    if s.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
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
