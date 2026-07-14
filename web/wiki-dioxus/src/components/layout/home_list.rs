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
            Some(Ok((groups, _, _))) => rsx! {
                div { class: "list",
                    // Invitations first — they need action.
                    for inv in invited_groups.iter() {
                        InvitedContextItem { key: "inv-{inv.id.0}", invite: inv.clone() }
                    }
                    for node in groups.iter() {
                        ContextItem { key: "{node.id.0}", node: node.clone() }
                    }
                }
            },
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
            Some(Ok((_, events, _))) => rsx! {
                // Invited events first (no year bucket — they need action).
                if !invited_events.is_empty() {
                    div { class: "list",
                        for inv in invited_events.iter() {
                            InvitedContextItem { key: "inv-{inv.id.0}", invite: inv.clone() }
                        }
                    }
                }
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
            },
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
                    h3 { class: "title-medium", "{t(\"layout.groups\")}" }
                }
                div { class: "home-section-body", {groups_body} }
            }
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar small", span { class: "material-icons", "event" } }
                    h3 { class: "title-medium", "{t(\"layout.events\")}" }
                }
                div { class: "home-section-body", {events_body} }
            }
        }
    } else {
        rsx! {
            div { style: "margin-top: 16px;",
                h4 { class: "title-small", class: "text-muted", style: "padding: 8px 16px;",
                    "{t(\"layout.groups\")}"
                }
                {groups_body}
                h4 { class: "title-small", class: "text-muted", style: "padding: 8px 16px; margin-top: 8px;",
                    "{t(\"layout.events\")}"
                }
                {events_body}
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
