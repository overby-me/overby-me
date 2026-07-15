use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::{t, t_with};
use crate::route::Route;
use crate::session::use_session;

/// The result of loading the user's contexts: (groups, events, pending invites).
type ContextLists = (
    Vec<graphql::ContextNodeFields>,
    Vec<graphql::ContextNodeFields>,
    Vec<graphql::InvitationFields>,
);

/// HomeList — shows the user's groups and events, loaded from GraphQL. Pending
/// invitations appear inline at the top of the matching list (group or event),
/// each with accept / reject actions.
#[component]
pub fn HomeList(#[props(default = false)] as_cards: bool) -> Element {
    let session = use_session();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let email = session
        .read()
        .user
        .as_ref()
        .map(|u| u.email.clone())
        .unwrap_or_default();
    let access_token = session.read().access_token.clone();

    // Live home list: accepting an invitation or a membership change re-fetches
    // the user's groups and events.
    let refresh = use_signal(|| 0u32);
    let sub_uid = user_id
        .clone()
        .unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".to_string());
    crate::subscription::use_live(
        format!(
            "subscription {{ members(where: {{ nodeId: {{ _eq: \"{sub_uid}\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    let contexts = crate::use_data_resource!(move || {
        let token = access_token.clone();
        let user_id = user_id.clone();
        let email = email.clone();
        let _ = refresh.read();
        async move {
            let Some(user_id) = user_id else {
                return Ok::<ContextLists, String>((Vec::new(), Vec::new(), Vec::new()));
            };
            let groups = graphql::query_contexts(token.as_deref(), &user_id, "wiki/group").await?;
            let events = graphql::query_contexts(token.as_deref(), &user_id, "wiki/event").await?;
            let invites = graphql::query_invitations(token.as_deref(), &user_id, &email)
                .await
                .unwrap_or_default();
            Ok((groups, events, invites))
        }
    });

    // The root node backs the owner-only "add group / add event" actions: its id
    // and own context, plus which context mimes the signed-in user may create
    // there. Non-owners get an empty mime list (server-side `inserts` gate), so
    // the buttons never appear.
    let root_token = session.read().access_token.clone();
    let root = crate::use_data_resource!(|(root_token)| async move {
        let Some(node) = graphql::query_root_node(root_token.as_deref())
            .await
            .ok()
            .flatten()
        else {
            return (None, None, Vec::<String>::new());
        };
        let id = node.id.0.clone();
        let ctx = node
            .context_id
            .clone()
            .map(|c| c.0)
            .unwrap_or_else(|| id.clone());
        let mimes = graphql::node_insert_mimes(root_token.as_deref(), &id).await;
        (Some(id), Some(ctx), mimes)
    });
    let (root_id, root_ctx, root_mimes) = root.read().clone().unwrap_or((None, None, Vec::new()));
    let rid = root_id.clone().unwrap_or_default();
    let rctx = root_ctx.unwrap_or_else(|| rid.clone());
    let can_group = root_id.is_some() && root_mimes.iter().any(|m| m == "wiki/group");
    let can_event = root_id.is_some() && root_mimes.iter().any(|m| m == "wiki/event");

    // Keep each list short by default (newest first) and let the user expand to
    // the full list — the roster of past events especially can be long.
    const LIST_LIMIT: usize = 4;
    let mut groups_expanded = use_signal(|| false);
    let mut events_expanded = use_signal(|| false);

    let state = contexts.read().clone();
    let hint_style = "padding: 4px 16px;";
    // Pending invitations, split into the list they belong to (group vs event) so
    // each shows inline at the top of that list.
    let invited_by_mime = |mime: &str| -> Vec<graphql::InvitationFields> {
        match &state {
            Some(Ok((_, _, invites))) => invites
                .iter()
                .filter(|i| i.parent.as_ref().and_then(|p| p.mime_id.as_deref()) == Some(mime))
                .cloned()
                .collect(),
            _ => Vec::new(),
        }
    };
    let invited_groups = invited_by_mime("wiki/group");
    let invited_events = invited_by_mime("wiki/event");

    // The two section bodies (shared between the drawer's bare list and the home's
    // two-card layout).
    let groups_body = rsx! {
        {match &state {
            None => rsx! {
                p { class: "body-medium text-muted", style: "{hint_style}", "…" }
            },
            Some(Err(e)) => rsx! {
                p { class: "body-medium text-muted", style: "{hint_style}", "{e}" }
            },
            Some(Ok((groups, _, _))) if groups.is_empty() && invited_groups.is_empty() => rsx! {
                p { class: "body-medium text-muted", style: "{hint_style}", "{t(\"layout.noGroups\")}" }
            },
            Some(Ok((groups, _, _))) => {
                let expanded = *groups_expanded.read();
                let total = groups.len();
                let visible: Vec<graphql::ContextNodeFields> = if expanded {
                    groups.clone()
                } else {
                    groups.iter().take(LIST_LIMIT).cloned().collect()
                };
                rsx! {
                    div { class: "list",
                        // Invitations first — they need action.
                        for inv in invited_groups.iter() {
                            InvitedContextItem { key: "inv-{inv.id.0}", invite: inv.clone() }
                        }
                        for node in visible.iter() {
                            ContextItem { key: "{node.id.0}", node: node.clone() }
                        }
                    }
                    if total > LIST_LIMIT {
                        button {
                            class: "btn btn-text list-expand-toggle",
                            // Stop the click reaching the drawer's close-on-item handler,
                            // so expanding the list on mobile doesn't dismiss the drawer.
                            onclick: move |evt: Event<MouseData>| {
                                evt.stop_propagation();
                                let e = *groups_expanded.read();
                                groups_expanded.set(!e);
                            },
                            if expanded {
                                "{t(\"layout.showLess\")}"
                            } else {
                                "{t_with(\"layout.showAll\", &[(\"count\", &total.to_string())])}"
                            }
                        }
                    }
                }
            }
        }}
    };
    let events_body = rsx! {
        {match &state {
            None => rsx! {
                p { class: "body-medium text-muted", style: "{hint_style}", "…" }
            },
            Some(Err(e)) => rsx! {
                p { class: "body-medium text-muted", style: "{hint_style}", "{e}" }
            },
            Some(Ok((_, events, _))) if events.is_empty() && invited_events.is_empty() => rsx! {
                p { class: "body-medium text-muted", style: "{hint_style}", "{t(\"layout.noEvents\")}" }
            },
            Some(Ok((_, events, _))) => {
                let expanded = *events_expanded.read();
                let total = events.len();
                rsx! {
                    // Invited events first (no year bucket — they need action).
                    if !invited_events.is_empty() {
                        div { class: "list",
                            for inv in invited_events.iter() {
                                InvitedContextItem { key: "inv-{inv.id.0}", invite: inv.clone() }
                            }
                        }
                    }
                    // Collapsed: just the newest few, flat. Expanded: the full list
                    // bucketed by year (the historical view).
                    if expanded {
                        for (year , items) in group_by_year(events) {
                            div { key: "{year}",
                                p { class: "label-medium",
                                    class: "text-muted", style: "padding: 4px 16px; font-weight: 600;",
                                    "{year}"
                                }
                                div { class: "list",
                                    for node in items.iter() {
                                        ContextItem { key: "{node.id.0}", node: node.clone() }
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "list",
                            for node in events.iter().take(LIST_LIMIT) {
                                ContextItem { key: "{node.id.0}", node: node.clone() }
                            }
                        }
                    }
                    if total > LIST_LIMIT {
                        button {
                            class: "btn btn-text list-expand-toggle",
                            onclick: move |evt: Event<MouseData>| {
                                evt.stop_propagation();
                                let e = *events_expanded.read();
                                events_expanded.set(!e);
                            },
                            if expanded {
                                "{t(\"layout.showLess\")}"
                            } else {
                                "{t_with(\"layout.showAll\", &[(\"count\", &total.to_string())])}"
                            }
                        }
                    }
                }
            }
        }}
    };

    // DESIGN: on the home app (as_cards), Groups and Events are SEPARATE cards,
    // each with its own icon-avatar header — so they read as distinct home sections
    // rather than one bare list. The drawer keeps the compact bare list.
    if as_cards {
        rsx! {
            div { class: "card",
                div { class: "card-header",
                    div { class: "avatar small", span { class: "material-icons", "groups" } }
                    h3 { class: "title-large", "{t(\"layout.groups\")}" }
                    if can_group {
                        div { class: "flex-grow" }
                        NewContextButton { mime: "wiki/group".to_string(), root_id: rid.clone(), root_context_id: rctx.clone() }
                    }
                }
                div { class: "home-section-body", {groups_body} }
            }
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar small", span { class: "material-icons", "event" } }
                    h3 { class: "title-large", "{t(\"layout.events\")}" }
                    if can_event {
                        div { class: "flex-grow" }
                        NewContextButton { mime: "wiki/event".to_string(), root_id: rid.clone(), root_context_id: rctx.clone() }
                    }
                }
                div { class: "home-section-body", {events_body} }
            }
        }
    } else {
        rsx! {
            div { style: "margin-top: 16px;",
                div { style: "display: flex; align-items: center; padding: 8px 16px;",
                    h4 { class: "title-small", class: "text-muted", style: "flex: 1; margin: 0;",
                        "{t(\"layout.groups\")}"
                    }
                    if can_group {
                        NewContextButton { mime: "wiki/group".to_string(), root_id: rid.clone(), root_context_id: rctx.clone() }
                    }
                }
                {groups_body}
                div { style: "display: flex; align-items: center; padding: 8px 16px; margin-top: 8px;",
                    h4 { class: "title-small", class: "text-muted", style: "flex: 1; margin: 0;",
                        "{t(\"layout.events\")}"
                    }
                    if can_event {
                        NewContextButton { mime: "wiki/event".to_string(), root_id: rid.clone(), root_context_id: rctx.clone() }
                    }
                }
                {events_body}
            }
        }
    }
}

/// A per-list "add group" / "add event" action, rendered in a list header for
/// the root owner (the caller gates on the root's `inserts`). It opens a name
/// dialog and drives [`graphql::create_context`], which creates the node under
/// the root, makes it its own context, and seeds the permission template — so the
/// new group/event is usable immediately. On success it jumps into it.
#[component]
fn NewContextButton(mime: String, root_id: String, root_context_id: String) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let is_group = mime == "wiki/group";
    let label = if is_group {
        t("layout.newGroup")
    } else {
        t("layout.newEvent")
    };

    let mut open = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let submit = move |_| {
        let title = name.read().trim().to_string();
        if title.is_empty() || *busy.read() {
            return;
        }
        let key = crate::components::loader::slugify(&title);
        let mime = mime.clone();
        let root_id = root_id.clone();
        let root_context_id = root_context_id.clone();
        let token = session.read().access_token.clone();
        busy.set(true);
        error.set(String::new());
        spawn(async move {
            match graphql::create_context(
                token.as_deref(),
                &root_id,
                &root_context_id,
                &mime,
                &title,
                &key,
            )
            .await
            {
                Ok(inserted) => {
                    crate::session::bump_data_version();
                    busy.set(false);
                    open.set(false);
                    name.set(String::new());
                    nav.push(Route::PathPage {
                        segments: vec![inserted.key],
                        app: None,
                    });
                }
                Err(e) => {
                    busy.set(false);
                    log::error!("create context failed: {e}");
                    error.set(t("layout.createFailed"));
                }
            }
        });
    };

    rsx! {
        button {
            class: "btn-icon add-action state-layer",
            title: "{label}",
            aria_label: "{label}",
            onclick: move |_| open.set(true),
            span { class: "material-icons", "add" }
        }
        crate::components::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: label.clone(),
            icon: (if is_group { "group_add" } else { "event" }).to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: name.read().trim().is_empty() || *busy.read(),
                    onclick: submit,
                    "{t(\"common.add\")}"
                }
            },
            div { class: "text-field",
                label { "{t(\"common.title\")}" }
                input {
                    r#type: "text",
                    maxlength: "{crate::components::editor::NODE_NAME_MAXLEN}",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                }
            }
            if !error.read().is_empty() {
                p {
                    class: "body-medium mt-1",
                    style: "color: var(--md-error, #b3261e);",
                    "{error}"
                }
            }
        }
    }
}

/// A single group/event entry. Clicking resolves the node's path and navigates.
#[component]
pub(super) fn ContextItem(node: graphql::ContextNodeFields) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let name = node.name.clone();
    let node_id = node.id.0.clone();
    let abbr = abbrev_context_name(&name);

    rsx! {
        div {
            class: "list-item",
            style: "cursor: pointer;",
            onclick: move |_| {
                let node_id = node_id.clone();
                let token = session.read().access_token.clone();
                spawn(async move {
                    if let Ok(segments) = graphql::path_from_id(token.as_deref(), &node_id).await {
                        if !segments.is_empty() {
                            nav.push(Route::PathPage { segments, app: None });
                        }
                    }
                });
            },
            div { class: "avatar small secondary avatar-abbr", "{abbr}" }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{name}" }
            }
        }
    }
}

/// A pending invitation shown inline in the groups/events list: accept (join) or
/// reject it. Rejecting asks for confirmation first. The accept flow mirrors the
/// old invites card — bind the invite to the user, or (on the unique-constraint
/// conflict when a membership already exists) accept that row and drop the
/// duplicate invite.
#[component]
pub(super) fn InvitedContextItem(invite: graphql::InvitationFields) -> Element {
    let session = use_session();
    let mut confirm_open = use_signal(|| false);
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
                    crate::session::bump_data_version();
                }
            });
        }
    };
    let reject = {
        let member_id = member_id.clone();
        move |_| {
            let token = session.read().access_token.clone();
            let member_id = member_id.clone();
            confirm_open.set(false);
            spawn(async move {
                let _ = graphql::decline_invitation(token.as_deref(), &member_id).await;
                crate::session::bump_data_version();
            });
        }
    };

    rsx! {
        div { class: "list-item",
            div { class: "avatar small secondary", {crate::components::loader::icon_el(&mime_id)} }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{name}" }
                div { class: "list-item-secondary", "{t(\"invite.invited\")}" }
            }
            button {
                class: "btn-icon add-action state-layer",
                title: "{t_with(\"invite.acceptInvitation\", &[(\"name\", &name)])}",
                aria_label: "{t_with(\"invite.acceptInvitation\", &[(\"name\", &name)])}",
                onclick: accept,
                span { class: "material-icons", "check" }
            }
            button {
                class: "btn-icon state-layer",
                title: "{t(\"invite.declineInvitation\")}",
                aria_label: "{t(\"invite.declineInvitation\")}",
                onclick: move |_| confirm_open.set(true),
                span { class: "material-icons", "close" }
            }
        }
        // Confirm before rejecting an invitation, via the app's standard Dialog.
        crate::components::widgets::Dialog {
            open: confirm_open(),
            on_dismiss: move |_| confirm_open.set(false),
            headline: t("invite.confirmReject"),
            icon: "close".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| confirm_open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button { class: "btn btn-primary", onclick: reject, "{t(\"invite.reject\")}" }
            },
            p { class: "body-medium", "{name}" }
        }
    }
}

/// Group events into (year, events) buckets, preserving the input order. Since
/// events arrive newest-first, buckets come out in descending-year order.
pub(super) fn group_by_year(
    events: &[graphql::ContextNodeFields],
) -> Vec<(String, Vec<graphql::ContextNodeFields>)> {
    let mut out: Vec<(String, Vec<graphql::ContextNodeFields>)> = Vec::new();
    for event in events {
        let year = event
            .created_at
            .as_ref()
            .and_then(|t| t.0.get(0..4))
            .unwrap_or("")
            .to_string();
        match out.last_mut() {
            Some((last_year, items)) if *last_year == year => items.push(event.clone()),
            _ => out.push((year, vec![event.clone()])),
        }
    }
    out
}

/// Abbreviate a context name into a short avatar badge (ported from the React
/// `abrivContextName`): keep capitalised words, collapse each to its acronym or
/// initial, and join at most three of them.
pub(super) fn abbrev_context_name(name: &str) -> String {
    fn upper_count(word: &str) -> usize {
        word.chars().filter(|c| c.is_uppercase()).count()
    }

    let words: Vec<String> = name
        .trim()
        .split(' ')
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let first = w.chars().next().unwrap();
            let has_digit = w.chars().any(|c| c.is_ascii_digit());
            (first.is_uppercase() && !(has_digit && w.chars().count() > 1)) || upper_count(w) > 1
        })
        .map(|w| match w {
            "Hovedbestyrelsesmøde" => "HB".to_string(),
            "Landsmøde" => "LM".to_string(),
            // An acronym (e.g. "EU-"): keep only its uppercase letters, so
            // trailing punctuation like a hyphen cannot break onto a new line.
            _ if upper_count(w) > 1 => w.chars().filter(|c| c.is_uppercase()).collect(),
            _ => w.chars().next().unwrap().to_string(),
        })
        .collect();

    match words.len() {
        // Two characters sit comfortably in the avatar circle (e.g. "EU", "KM",
        // "HB"); more than that reads as crammed.
        1..=3 => words.concat().chars().take(2).collect(),
        _ => String::new(),
    }
}
