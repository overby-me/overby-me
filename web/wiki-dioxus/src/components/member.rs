use dioxus::prelude::*;

use crate::graphql::{self, MemberFields, MembersSetInput, NodeWithChildren};
use crate::i18n::{t, t_with};
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

    let mut members: Vec<MemberFields> = node
        .members
        .iter()
        .filter(|m| can_manage || !m.hidden)
        .cloned()
        .collect();
    // Order by display name (React's members_order_by.user), case-insensitively.
    members.sort_by_key(|m| m.label().to_lowercase());

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
                        // Export the participant roster as CSV (owner action, #41).
                        if can_manage && !members.is_empty() {
                            button {
                                class: "btn-icon",
                                title: "{t(\"member.export\")}",
                                aria_label: "{t(\"member.export\")}",
                                onclick: {
                                    let members = members.clone();
                                    let fname = name.clone();
                                    move |_| {
                                        let mut csv = String::from("Name,Email\n");
                                        for m in &members {
                                            csv.push_str(&csv_field(&m.label()));
                                            csv.push(',');
                                            csv.push_str(&csv_field(m.email.as_deref().unwrap_or("")));
                                            csv.push('\n');
                                        }
                                        let file = format!("{}-participants.csv", crate::export::sanitize_filename(&fname));
                                        crate::export::download_bytes(&file, "text/csv;charset=utf-8", csv.as_bytes());
                                    }
                                },
                                span { class: "material-icons", "download" }
                            }
                        }
                    }

                    // Member roster: a searchable, filterable, paginated M3
                    // Expressive table (handles 1000+ members client-side).
                    if members.is_empty() {
                        div { class: "card-content",
                            p { class: "body-medium",
                                class: "text-muted",
                                "{t(\"common.noContent\")}"
                            }
                        }
                    } else {
                        MemberTable {
                            members: members.clone(),
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
            },
            supporting: rsx! {
                // Invite input (owner action, mirroring React InvitesFab).
                if can_manage {
                    div { class: "card",
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
                                                    div { class: "avatar small", span { class: "material-icons", "person" } }
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

/// The single-select roster filter (a row of M3 filter chips).
#[derive(Clone, Copy, PartialEq)]
enum MemberFilter {
    All,
    Owners,
    Active,
    Invited,
    Hidden,
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

/// A searchable, filterable, paginated M3 Expressive member table. Search (name
/// or email), filter chips and paging all run client-side over the loaded roster,
/// so it stays responsive with 1000+ members. Page size is fixed at 25.
#[component]
fn MemberTable(
    members: Vec<MemberFields>,
    can_manage: bool,
    on_edit: EventHandler<MemberFields>,
    on_remove: EventHandler<MemberFields>,
) -> Element {
    const PAGE_SIZE: usize = 25;
    let mut search = use_signal(String::new);
    let mut filter = use_signal(|| MemberFilter::All);
    let mut page = use_signal(|| 0usize);

    let q = search.read().trim().to_lowercase();
    let active_filter = *filter.read();
    let filtered: Vec<MemberFields> = members
        .iter()
        .filter(|m| {
            let pass = match active_filter {
                MemberFilter::All => true,
                MemberFilter::Owners => m.owner,
                MemberFilter::Active => m.active,
                MemberFilter::Invited => !m.accepted && !m.hidden,
                MemberFilter::Hidden => m.hidden,
            };
            pass && (q.is_empty()
                || m.label().to_lowercase().contains(&q)
                || m.email.as_deref().unwrap_or("").to_lowercase().contains(&q))
        })
        .cloned()
        .collect();

    let total = filtered.len();
    let page_count = total.div_ceil(PAGE_SIZE).max(1);
    let cur = (*page.read()).min(page_count - 1);
    let start = cur * PAGE_SIZE;
    let end = (start + PAGE_SIZE).min(total);
    let first = if total == 0 { 0 } else { start + 1 };
    let page_slice = filtered[start..end].to_vec();

    let filters = [
        (MemberFilter::All, t("member.filterAll")),
        (MemberFilter::Owners, t("member.owner")),
        (MemberFilter::Active, t("member.active")),
        (MemberFilter::Invited, t("invite.invitations")),
        (MemberFilter::Hidden, t("member.hidden")),
    ];

    let mut columns = vec![t("member.name"), t("member.email"), t("member.status")];
    if can_manage {
        columns.push(t("member.actions"));
    }

    rsx! {
        div { class: "member-table",
            // Search + filter toolbar.
            div { class: "member-table-toolbar",
                label { class: "search-field",
                    span { class: "material-icons", "search" }
                    input {
                        r#type: "text",
                        placeholder: "{t(\"member.search\")}",
                        value: "{search}",
                        oninput: move |e| {
                            search.set(e.value());
                            page.set(0);
                        },
                    }
                }
                div { class: "filter-chips", role: "group",
                    for (kind , label) in filters {
                        {
                            let selected = active_filter == kind;
                            rsx! {
                                button {
                                    key: "{label}",
                                    r#type: "button",
                                    class: if selected { "m3-filter-chip selected" } else { "m3-filter-chip" },
                                    "aria-pressed": if selected { "true" } else { "false" },
                                    onclick: move |_| {
                                        filter.set(kind);
                                        page.set(0);
                                    },
                                    if selected {
                                        span { class: "material-icons", "check" }
                                    }
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }

            // The table itself.
            super::widgets::DataTable {
                columns,
                for m in page_slice.iter() {
                    MemberTableRow {
                        key: "{m.id.0}",
                        member: m.clone(),
                        can_manage,
                        on_edit: move |mm: MemberFields| on_edit.call(mm),
                        on_remove: move |mm: MemberFields| on_remove.call(mm),
                    }
                }
            }

            // Pagination footer: "1-25 / 1043" plus prev/next.
            div { class: "member-table-footer",
                span { class: "member-count body-medium", "{first}-{end} / {total}" }
                div { class: "pagination-controls",
                    button {
                        class: "btn-icon",
                        disabled: cur == 0,
                        title: "{t(\"common.previous\")}",
                        aria_label: "{t(\"common.previous\")}",
                        onclick: move |_| page.set(cur.saturating_sub(1)),
                        span { class: "material-icons", "chevron_left" }
                    }
                    span { class: "body-medium page-indicator", "{cur + 1} / {page_count}" }
                    button {
                        class: "btn-icon",
                        disabled: cur + 1 >= page_count,
                        title: "{t(\"common.next\")}",
                        aria_label: "{t(\"common.next\")}",
                        onclick: move |_| page.set((cur + 1).min(page_count - 1)),
                        span { class: "material-icons", "chevron_right" }
                    }
                }
            }
        }
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

    rsx! {
        tr { class: if hidden { "member-row-hidden" } else { "" },
            td {
                span { class: "m3-cell-icon",
                    div { class: "avatar small", span { class: "material-icons", "person" } }
                    span { "{member.label()}" }
                }
            }
            td { class: "member-email-cell", "{member.email.clone().unwrap_or_default()}" }
            td {
                super::widgets::Chip { icon: status_icon.to_string(), label: status_label }
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
