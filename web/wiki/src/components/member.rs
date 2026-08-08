use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::{t, t_with};
use crate::model::{self, MemberFields, MembersSetInput, NodeWithChildren};
use crate::route::Route;
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

    // atproto-link nudge (#4): the DID map is empty today (0 members linked), so
    // prompt a signed-in member who has not linked their atproto identity to do so,
    // filling the future member->DID migration map one login at a time. Dismissible.
    let nav = use_navigator();
    let is_auth = session.read().is_authenticated();
    let nudge_token = session.read().access_token.clone();
    let atproto_linked = crate::use_data_resource!(|(nudge_token)| async move {
        match nudge_token {
            Some(t) => crate::backend_api::atproto_status(&t).await.linked,
            None => true,
        }
    });
    let show_link_nudge = is_auth && !(*atproto_linked.read()).unwrap_or(true);
    let mut nudge_dismissed = use_signal(|| false);

    // Roster paging: a search box + a single-select filter + a page, all resolved
    // SERVER-SIDE (Hasura limit/offset/where + an aggregate count) so the roster
    // scales past thousands of members without loading them all.
    const PAGE_SIZE: usize = 25;
    let search = use_signal(String::new);
    let filter = use_signal(|| "all".to_string());
    let page = use_signal(|| 0usize);

    let member_filter = roster_filter(filter.read().as_str(), search.read().clone(), can_manage);

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
        });
    // A roster that failed to load looked exactly like a context with no members
    // — on the screen an owner uses to decide who is missing.
    let load = roster.read().clone();
    let roster_failed = matches!(load, Some(Err(_)));
    if let Some(Err(e)) = &load {
        crate::errors::log_handled("roster load failed", e);
    }
    let (rows, total) = load.and_then(|r| r.ok()).unwrap_or_default();

    // Edit dialog (name/email) and remove confirm, shared across the rows.
    let mut edit_id = use_signal(|| Option::<String>::None);
    let mut edit_name = use_signal(String::new);
    let mut edit_email = use_signal(String::new);
    let mut remove_target = use_signal(|| Option::<(String, String)>::None);
    // Pending owner promote/demote, confirmed via a dialog: (member id, make-owner,
    // label). Ownership changes are as consequential as removal, so they confirm too.
    let mut owner_target = use_signal(|| Option::<(String, bool, String)>::None);
    // Invite-by-name autocomplete: matching users, and a monotonic request id so
    // out-of-order search responses don't clobber a newer one.
    let mut user_matches = use_signal(Vec::<model::Author>::new);
    let mut search_seq = use_signal(|| 0u32);
    // Same reasoning as the author picker: a debounce is silence, and silence
    // reads as nothing happening.
    let mut searching_users = use_signal(|| false);

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
                if show_link_nudge && !nudge_dismissed() {
                    div { class: "card app-card mb-1",
                        div {
                            class: "card-content stack stack-h",
                            span { class: "material-icons", "cloud_off" }
                            div { class: "flex-grow",
                                div { class: "title-small", "{t(\"member.linkNudgeTitle\")}" }
                                div { class: "body-small text-muted", "{t(\"member.linkNudgeBody\")}" }
                            }
                            button {
                                class: "btn btn-primary",
                                onclick: {
                                    let my_id = user_id.clone();
                                    move |_| {
                                        if let Some(id) = my_id.clone() {
                                            nav.push(Route::UserProfile { id });
                                        }
                                    }
                                },
                                "{t(\"member.linkNudgeAction\")}"
                            }
                            button {
                                class: "btn-icon",
                                aria_label: t("common.close"),
                                title: t("common.close"),
                                onclick: move |_| nudge_dismissed.set(true),
                                span { class: "material-icons", "close" }
                            }
                        }
                    }
                }
                div { class: "card app-card",
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
                                // Pinned quick group: copy link (the sheet's own
                                // first segment) and the roster CSV export.
                                quick: rsx! {
                                button {
                                    class: "sheet-quick-action",
                                    r#type: "button",
                                    title: "{t(\"member.export\")}",
                                    aria_label: "{t(\"member.export\")}",
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
                                                    &model::MemberPageFilter::default(),
                                                    100_000,
                                                    0,
                                                )
                                                .await
                                                .unwrap_or_default();
                                                let mut csv = String::from("Name,Email\n");
                                                for m in &all {
                                                    csv.push_str(&crate::export::csv_field(&m.label()));
                                                    csv.push(',');
                                                    csv.push_str(&crate::export::csv_field(m.email.as_deref().unwrap_or("")));
                                                    csv.push('\n');
                                                }
                                                let file = format!("{}-participants.csv", crate::export::sanitize_filename(&fname));
                                                crate::export::download_bytes(&file, "text/csv;charset=utf-8", csv.as_bytes());
                                            });
                                        }
                                    },
                                    span { class: "material-icons", "download" }
                                }
                                },
                            }
                        }
                    }

                    if roster_failed {
                        super::widgets::ErrorState {
                            title: t("error.couldNotLoad"),
                            small: true,
                            on_retry: move |_| crate::session::bump_data_version(),
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
                                on_owner_change: move |(id, make_owner, label): (String, bool, String)| {
                                    owner_target.set(Some((id, make_owner, label)));
                                },
                            }
                        }
                    }
                }
            },
            supporting: rsx! {
                // Invite input (owner action, mirroring React InvitesFab).
                if can_manage {
                    div { class: "card app-card",
                        // DESIGN: give the invite card a header identity like the
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
                                            searching_users.set(false);
                                            return;
                                        }
                                        searching_users.set(true);
                                        let token = session.read().access_token.clone();
                                        let seq = *search_seq.read() + 1;
                                        search_seq.set(seq);
                                        spawn(async move {
                                            // Same debounce as the author picker: a
                                            // roster search per keystroke is a request
                                            // per keystroke, all but the last wasted.
                                            gloo_timers::future::TimeoutFuture::new(
                                                super::editor::AUTHOR_SEARCH_DEBOUNCE_MS,
                                            )
                                            .await;
                                            if *search_seq.peek() != seq {
                                                return;
                                            }
                                            let results = graphql::search_users(token.as_deref(), &q).await;
                                            if *search_seq.read() == seq {
                                                user_matches.set(results);
                                                searching_users.set(false);
                                            }
                                        });
                                    },
                                }
                            }
                            if *searching_users.read() {
                                div { class: "list",
                                    div { class: "list-item",
                                        div { class: "spinner spinner-xs" }
                                        div { class: "list-item-text",
                                            div { class: "list-item-primary", "{t(\"content.searching\")}" }
                                        }
                                    }
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
                                                                    show_snackbar(&t("invite.sent"));
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
                                                    show_snackbar(&t("invite.sent"));
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
                                                    crate::backend_api::parse_roster(token.as_deref(), bytes.to_vec())
                                                        .await;
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
            // A form, so it takes the screen on a phone (see widgets::Dialog).
            form: true,
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| edit_id.set(None),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    // A member needs at least a name or an email; do not let save
                    // persist a blank-on-both row (the invite button gates the same way).
                    disabled: edit_name.read().trim().is_empty() && edit_email.read().trim().is_empty(),
                    onclick: save_edit,
                    "{t(\"common.save\")}"
                }
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

        // Remove-member confirm, via the app's standard Dialog.
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

        // Promote / demote owner confirm.
        super::widgets::Dialog {
            open: owner_target.read().is_some(),
            on_dismiss: move |_| owner_target.set(None),
            headline: match owner_target.read().as_ref() {
                Some((_, true, _)) => t("member.promote"),
                _ => t("member.demote"),
            },
            icon: "star".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| owner_target.set(None),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    onclick: move |_| {
                        let Some((id, make_owner, _)) = owner_target.read().clone() else {
                            return;
                        };
                        owner_target.set(None);
                        apply_member_update(
                            session.read().access_token.clone(),
                            id,
                            MembersSetInput { owner: Some(make_owner), ..Default::default() },
                        );
                    },
                    if owner_target.read().as_ref().map(|t| t.1).unwrap_or(false) {
                        "{t(\"member.promote\")}"
                    } else {
                        "{t(\"member.demote\")}"
                    }
                }
            },
            if let Some((_, _, label)) = owner_target.read().clone() {
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

/// The server-side predicate behind a roster filter chip.
///
/// The chips are named after `accepted`, the column that actually says whether
/// someone joined. They used to say "Active", which is a different column
/// entirely — attendance, toggled per row — and an invitation is created with
/// `active = true` and `accepted = false`. So "Active" returned the whole roster:
/// on the 2026 landsmøde, 887 rows of which 830 were invitations, each one
/// labelled "Invitation" in its own status column. One word, two meanings, and
/// the filter picked the wrong one.
fn roster_filter(chip: &str, search: String, can_manage: bool) -> model::MemberPageFilter {
    let mut mf = model::MemberPageFilter {
        search,
        ..Default::default()
    };
    match chip {
        "owners" => mf.owner = Some(true),
        "accepted" => mf.accepted = Some(true),
        "invited" => {
            mf.accepted = Some(false);
            mf.hidden = Some(false);
        }
        "hidden" => mf.hidden = Some(true),
        _ => {}
    }
    // Non-managers never see hidden members. Unconditional, not "unless one was
    // asked for": the previous `is_none()` guard let a "hidden" chip value
    // through untouched, which contradicted this line. Only the chip list keeps
    // that value away from non-managers, and a UI list is not where an access
    // rule belongs.
    if !can_manage {
        mf.hidden = Some(false);
    }
    mf
}

/// The roster filter chips as `(value, label)` (Hidden only for managers). The
/// values map to the server-side predicate built by [`roster_filter`].
fn member_filters(can_manage: bool) -> Vec<(String, String)> {
    let mut f = vec![
        ("all".to_string(), t("member.filterAll")),
        ("owners".to_string(), t("member.owner")),
        ("accepted".to_string(), t("member.accepted")),
        ("invited".to_string(), t("member.invited")),
    ];
    if can_manage {
        f.push(("hidden".to_string(), t("member.hidden")));
    }
    f
}

/// The M3 status chip (icon + label) for a member's state: hidden > owner >
/// accepted > still-invited.
///
/// Says `accepted`/`invited`, matching the filter chips. It used to say "Active"
/// for an accepted member, which read as though it described the attendance flag
/// this column never consults.
fn member_status(m: &MemberFields) -> (&'static str, String) {
    if m.hidden {
        ("visibility_off", t("member.hidden"))
    } else if m.owner {
        ("star", t("member.owner"))
    } else if m.accepted {
        ("check_circle", t("member.accepted"))
    } else {
        ("mail", t("member.invited"))
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
    /// Request an owner promote/demote: (member id, make-owner, label). The parent
    /// confirms it via a dialog rather than mutating immediately.
    on_owner_change: EventHandler<(String, bool, String)>,
) -> Element {
    let session = use_session();
    let mid = member.id.0.clone();
    let owner = member.owner;
    // Optimistic attendance/visibility toggles: flip the chip at once, reconcile
    // against the refetched member, revert on error.
    let mut active_opt = use_signal(|| None::<bool>);
    let mut hidden_opt = use_signal(|| None::<bool>);
    {
        let (ma, mh) = (member.active, member.hidden);
        use_effect(use_reactive!(|(ma, mh)| {
            if *active_opt.peek() == Some(ma) {
                active_opt.set(None);
            }
            if *hidden_opt.peek() == Some(mh) {
                hidden_opt.set(None);
            }
        }));
    }
    let active = active_opt().unwrap_or(member.active);
    let hidden = hidden_opt().unwrap_or(member.hidden);
    let (status_icon, status_label) = member_status(&member);
    let avatar_url = member
        .user
        .as_ref()
        .map(|u| u.avatar_url.clone())
        .unwrap_or_default();
    let user_id = member.user.as_ref().map(|u| u.id.0.clone());
    let member_name = member.label();
    // DESIGN: colour-code the status chip by category.
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
                    super::loader::UserPopover {
                        name: member_name.clone(),
                        avatar_url: avatar_url.clone(),
                        user_id: user_id.clone(),
                        role: Some(status_label.clone()),
                        span { "{member_name}" }
                    }
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
                        // Unclaimed roster entry (no bound account): copy a claim link
                        // to hand to the person, whose signup email may not match the
                        // roster email. Claiming binds their account to this row.
                        if member.user.is_none() {
                            button {
                                class: "btn-icon",
                                title: "{t(\"invite.copyClaimLink\")}",
                                onclick: {
                                    let mid = mid.clone();
                                    move |_| {
                                        let mid = mid.clone();
                                        let token = session.read().access_token.clone();
                                        spawn(async move {
                                            let Some(token) = token else { return };
                                            match crate::backend_api::member_claim_link(&token, &mid).await {
                                                Ok(ct) => {
                                                    let origin = web_sys::window()
                                                        .and_then(|w| w.location().origin().ok())
                                                        .unwrap_or_default();
                                                    let link = format!("{origin}/?claim={ct}");
                                                    if let Some(win) = web_sys::window() {
                                                        let _ = win.navigator().clipboard().write_text(&link);
                                                    }
                                                    show_snackbar(&t("invite.claimLinkCopied"));
                                                }
                                                Err(_) => show_snackbar(&t("error.somethingWentWrong")),
                                            }
                                        });
                                    }
                                },
                                span { class: "material-icons", "person_add" }
                            }
                        }
                        // Resend a pending invitation: compose a ready-to-send email
                        // (subject + body carrying a fresh claim link) to the invitee.
                        // This backend sends no mail, so "resend" hands the owner the
                        // message to send themselves. Only for not-yet-accepted rows
                        // that carry an email address.
                        if !member.accepted && member.email.is_some() {
                            button {
                                class: "btn-icon",
                                title: "{t(\"invite.resendInvitation\")}",
                                onclick: {
                                    let mid = mid.clone();
                                    let email = member.email.clone().unwrap_or_default();
                                    move |_| {
                                        let mid = mid.clone();
                                        let email = email.clone();
                                        let token = session.read().access_token.clone();
                                        spawn(async move {
                                            let Some(token) = token else { return };
                                            match crate::backend_api::member_claim_link(&token, &mid).await {
                                                Ok(ct) => {
                                                    let origin = web_sys::window()
                                                        .and_then(|w| w.location().origin().ok())
                                                        .unwrap_or_default();
                                                    let link = format!("{origin}/?claim={ct}");
                                                    let subject = t("invite.emailSubject");
                                                    let body = t_with("invite.emailBody", &[("link", &link)]);
                                                    open_mailto(&email, &subject, &body);
                                                    show_snackbar(&t("invite.sent"));
                                                }
                                                Err(_) => show_snackbar(&t("error.somethingWentWrong")),
                                            }
                                        });
                                    }
                                },
                                span { class: "material-icons", "forward_to_inbox" }
                            }
                        }
                        // Promote / demote owner (confirmed via a dialog).
                        button {
                            class: "btn-icon",
                            title: if owner { "{t(\"member.demote\")}" } else { "{t(\"member.promote\")}" },
                            onclick: {
                                let mid = mid.clone();
                                let label = member_name.clone();
                                move |_| on_owner_change.call((mid.clone(), !owner, label.clone()))
                            },
                            span { class: "material-icons", if owner { "star" } else { "star_outline" } }
                        }
                        // Mark active / inactive (attendance).
                        button {
                            class: "btn-icon",
                            title: if active { "{t(\"member.deactivate\")}" } else { "{t(\"member.activate\")}" },
                            onclick: {
                                let mid = mid.clone();
                                move |_| {
                                    let new_val = !active;
                                    let token = session.read().access_token.clone();
                                    let id = mid.clone();
                                    active_opt.set(Some(new_val));
                                    spawn(async move {
                                        match graphql::update_member(token.as_deref(), &id, MembersSetInput { active: Some(new_val), ..Default::default() }).await {
                                            Ok(true) => crate::session::bump_data_version(),
                                            _ => {
                                                active_opt.set(None);
                                                show_snackbar(&t("error.somethingWentWrong"));
                                            }
                                        }
                                    });
                                }
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
                                move |_| {
                                    let new_val = !hidden;
                                    let token = session.read().access_token.clone();
                                    let id = mid.clone();
                                    hidden_opt.set(Some(new_val));
                                    spawn(async move {
                                        match graphql::update_member(token.as_deref(), &id, MembersSetInput { hidden: Some(new_val), ..Default::default() }).await {
                                            Ok(true) => crate::session::bump_data_version(),
                                            _ => {
                                                hidden_opt.set(None);
                                                show_snackbar(&t("error.somethingWentWrong"));
                                            }
                                        }
                                    });
                                }
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

/// Open the user's mail client with a pre-composed message (an `<a mailto:>`
/// clicked, so the SPA never navigates). Used to "resend" a pending invitation as
/// a ready-to-send email: this interim backend sends no mail itself, so the owner
/// gets a composed invite (with the claim link) to send from their own account.
fn open_mailto(to: &str, subject: &str, body: &str) {
    use wasm_bindgen::JsCast;
    let enc = |s: &str| String::from(&js_sys::encode_uri_component(s));
    let href = format!("mailto:{to}?subject={}&body={}", enc(subject), enc(body));
    let Some(window) = web_sys::window() else {
        return;
    };
    let Some(document) = window.document() else {
        return;
    };
    let Ok(el) = document.create_element("a") else {
        return;
    };
    let _ = el.set_attribute("href", &href);
    if let Ok(anchor) = el.dyn_into::<web_sys::HtmlElement>() {
        anchor.click();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accepted_excludes_pending_invitations() {
        // The reported bug: an invitation is stored with active = true and
        // accepted = false, so a chip that filtered on attendance returned it
        // alongside people who had actually joined.
        let f = roster_filter("accepted", String::new(), true);
        assert_eq!(f.accepted, Some(true));
        // Attendance is a separate question, and not this chip's.
        assert_eq!(f.active, None);
    }

    #[test]
    fn the_accepted_and_invited_chips_cannot_both_match_a_row() {
        let accepted = roster_filter("accepted", String::new(), true);
        let invited = roster_filter("invited", String::new(), true);
        assert_eq!(accepted.accepted, Some(true));
        assert_eq!(invited.accepted, Some(false));
    }

    #[test]
    fn the_old_active_value_no_longer_selects_anything() {
        // Renamed, so a stale value must fall through to "everything" rather
        // than silently filtering on the attendance column again.
        let f = roster_filter("active", String::new(), true);
        assert_eq!(f.accepted, None);
        assert_eq!(f.active, None);
    }

    #[test]
    fn only_managers_may_ask_for_hidden_members() {
        assert_eq!(
            roster_filter("hidden", String::new(), true).hidden,
            Some(true)
        );
        // Asking as a non-manager is overridden, not honoured — the chip list
        // hides the option, but that is a UI list, not an access rule.
        assert_eq!(
            roster_filter("hidden", String::new(), false).hidden,
            Some(false)
        );
        assert_eq!(
            roster_filter("all", String::new(), false).hidden,
            Some(false)
        );
        assert_eq!(roster_filter("all", String::new(), true).hidden, None);
    }

    #[test]
    fn the_search_text_survives_every_chip() {
        for chip in ["all", "owners", "accepted", "invited", "hidden"] {
            assert_eq!(
                roster_filter(chip, "pauline".into(), true).search,
                "pauline"
            );
        }
    }
}
