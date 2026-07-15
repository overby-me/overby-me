use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren, PollSummaryFields};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, mime_icon};

/// Child mimes that are not agenda items (comments, ballots, questions, member
/// nodes) — the console walks the real content the room discusses.
fn is_agenda_item(mime: &str) -> bool {
    !matches!(
        mime,
        "vote/comment" | "vote/vote" | "vote/question" | "wiki/user"
    )
}

/// AdminApp — the chair's run-the-meeting console (`?app=admin`). One owner-facing
/// screen to drive a live assembly: walk the agenda and project the current item
/// to the room + followers (the `active` relation), jump to the screen/speaker
/// views, and watch every poll's live tally (with a close action). Non-owners see
/// the agenda + results read-only.
#[component]
pub fn AdminApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());
    let can_manage = node.is_owner.unwrap_or(false) || node.is_context_owner.unwrap_or(false);

    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };

    // Live "now showing": subscribe to the context `active` relation so the agenda
    // highlight tracks whatever the chair projected, from any device.
    let refresh = use_signal(|| 0u32);
    let sub_ctx = graphql::gql_escape(&context_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ relations(where: {{ parentId: {{ _eq: \"{sub_ctx}\" }}, name: {{ _eq: \"active\" }} }}) {{ nodeId }} }}"
        ),
        refresh,
    );
    let rev = *refresh.read();
    let active_ctx = context_id.clone();
    let active_token = access_token.clone();
    let active = crate::use_data_resource!(|(active_ctx, active_token, rev)| async move {
        let _ = rev;
        graphql::active_node_id(active_token.as_deref(), &active_ctx)
            .await
            .ok()
            .flatten()
    });
    let server_active = active.read().clone().flatten();
    // Optimistic projection: reflect a project/stop tap at once (the agenda highlight
    // and the "on screen" chip), reconciled against the `active` subscription. None =
    // no override; Some(None) = optimistically stopped; Some(Some(id)) = projecting id.
    let mut projected_opt = use_signal(|| None::<Option<String>>);
    // Drop the override once the server (subscription refetch) reflects it, so a
    // concurrent chair's projection is not masked by a stale local override.
    {
        let sa = server_active.clone();
        use_effect(use_reactive!(|(sa)| {
            if projected_opt.peek().as_ref() == Some(&sa) {
                projected_opt.set(None);
            }
        }));
    }
    let active_id = projected_opt
        .read()
        .clone()
        .unwrap_or_else(|| server_active.clone());

    // The active document's headings, so the chair can bring the room to a section
    // of a document too long to show whole on the projector (#projector-focus).
    let hn_id = active_id.clone().unwrap_or_default();
    let hn_token = access_token.clone();
    let active_headings = crate::use_data_resource!(|(hn_id, hn_token, rev)| async move {
        let _ = rev;
        if hn_id.is_empty() {
            return Vec::new();
        }
        match graphql::query_node_by_id(hn_token.as_deref(), &hn_id).await {
            Ok(Some(n)) => {
                crate::components::content::content_headings(n.data.as_ref().map(|d| &d.0))
            }
            _ => Vec::new(),
        }
    });
    let active_headings = active_headings.read().clone().unwrap_or_default();

    // Agenda = the context's content children, in order.
    let mut agenda: Vec<_> = node
        .children
        .iter()
        .filter(|c| is_agenda_item(c.mime_id.as_deref().unwrap_or("")))
        .cloned()
        .collect();
    agenda.sort_by_key(|c| c.index);

    let polls_ctx = context_id.clone();
    let polls = crate::use_data_resource!(|(polls_ctx, access_token)| async move {
        graphql::query_context_polls(access_token.as_deref(), &polls_ctx)
            .await
            .unwrap_or_default()
    });
    let polls = polls.read().clone().unwrap_or_default();

    rsx! {
        div { class: "console",
            // Jump to the room-facing views without leaving the console flow.
            div { class: "console-actions",
                Link {
                    to: Route::PathPage { segments: segments.clone(), app: Some("screen".to_string()) },
                    class: "btn btn-tonal",
                    span { class: "material-icons", "connected_tv" }
                    "{t(\"mime.screen\")}"
                }
                Link {
                    to: Route::PathPage { segments: segments.clone(), app: Some("follow".to_string()) },
                    class: "btn btn-tonal",
                    span { class: "material-icons", "sensors" }
                    "{t(\"mime.follow\")}"
                }
                Link {
                    to: Route::PathPage { segments: segments.clone(), app: Some("speak".to_string()) },
                    class: "btn btn-tonal",
                    span { class: "material-icons", "record_voice_over" }
                    "{t(\"mime.speak\")}"
                }
            }

            // Agenda: project any item to the room + followers with one tap.
            div { class: "card",
                div { class: "card-header",
                    div { class: "avatar", {icon_el("app/program")} }
                    h3 { class: "title-medium", "{t(\"console.agenda\")}" }
                    div { class: "flex-grow" }
                    if can_manage && active_id.is_some() {
                        button {
                            class: "btn-icon",
                            title: "{t(\"console.stopProjecting\")}",
                            onclick: {
                                let ctx = context_id.clone();
                                move |_| {
                                    let ctx = ctx.clone();
                                    let token = session.read().access_token.clone();
                                    // Optimistic: clear the projection at once.
                                    projected_opt.set(Some(None));
                                    spawn(async move {
                                        match graphql::set_active_relation(token.as_deref(), &ctx, None).await {
                                            Ok(true) => crate::session::bump_data_version(),
                                            other => {
                                                projected_opt.set(None);
                                                log::error!("stop projecting failed: {other:?}");
                                                crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                            }
                                        }
                                    });
                                }
                            },
                            span { class: "material-icons", "cancel_presentation" }
                        }
                    }
                }
                if agenda.is_empty() {
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "list_alt" }
                        }
                        p { class: "empty-state-body", "{t(\"common.noContent\")}" }
                    }
                } else {
                    div { class: "list",
                        for item in agenda.iter() {
                            {
                                let is_active = active_id.as_deref() == Some(item.id.0.as_str());
                                let mut item_path = segments.clone();
                                item_path.push(item.key.clone());
                                let item_id = item.id.0.clone();
                                let proj_ctx = context_id.clone();
                                rsx! {
                                    div {
                                        key: "{item.id.0}",
                                        class: if is_active { "list-item agenda-item active" } else { "list-item agenda-item" },
                                        span { class: "material-icons agenda-icon",
                                            "{mime_icon(item.mime_id.as_deref().unwrap_or(\"wiki/document\"))}"
                                        }
                                        Link {
                                            to: Route::PathPage { segments: item_path, app: None },
                                            class: "list-item-text agenda-name",
                                            "{item.name}"
                                        }
                                        if is_active {
                                            span { class: "chip agenda-live",
                                                span { class: "material-icons", "connected_tv" }
                                                span { class: "chip-label", "{t(\"console.onScreen\")}" }
                                            }
                                        } else if can_manage {
                                            button {
                                                class: "btn btn-tonal btn-sm",
                                                onclick: move |_| {
                                                    let ctx = proj_ctx.clone();
                                                    let item_id = item_id.clone();
                                                    let token = session.read().access_token.clone();
                                                    // Optimistic: highlight this item as projected now.
                                                    projected_opt.set(Some(Some(item_id.clone())));
                                                    spawn(async move {
                                                        match graphql::set_active_relation(token.as_deref(), &ctx, Some(&item_id)).await {
                                                            Ok(_) => crate::snackbar::show_snackbar(&t("content.projected")),
                                                            Err(_) => {
                                                                projected_opt.set(None);
                                                                crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                            }
                                                        }
                                                    });
                                                },
                                                span { class: "material-icons", "cast" }
                                                "{t(\"content.projectScreen\")}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Projector focus: for a long active document, scroll the room's screen
            // to a chosen section (or back to the whole document). Shown whenever
            // something is projected; when the item has no sections, the card stays
            // (with a note) rather than silently vanishing.
            if can_manage && active_id.is_some() {
                div { class: "card",
                    div { class: "card-header",
                        div { class: "avatar", span { class: "material-icons", "center_focus_strong" } }
                        h3 { class: "title-medium", "{t(\"console.focusSection\")}" }
                    }
                    div { class: "list",
                        button {
                            class: "list-item admin-focus-item",
                            onclick: {
                                let ctx = context_id.clone();
                                move |_| {
                                    let ctx = ctx.clone();
                                    let token = session.read().access_token.clone();
                                    spawn(async move {
                                        if let Err(e) = graphql::set_screen_focus(token.as_deref(), &ctx, None).await {
                                            log::error!("clear screen focus failed: {e}");
                                        }
                                    });
                                }
                            },
                            span { class: "material-icons", "fullscreen" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{t(\"console.focusWhole\")}" }
                            }
                        }
                        if active_headings.is_empty() {
                            p { class: "body-small text-muted", style: "padding: 8px 16px;",
                                "{t(\"console.focusNoSections\")}"
                            }
                        }
                        for (anchor , text , level) in active_headings.iter() {
                            {
                                let indent = (*level as usize).saturating_sub(1) * 12 + 12;
                                let anchor = anchor.clone();
                                rsx! {
                                    button {
                                        key: "{anchor}",
                                        class: "list-item admin-focus-item",
                                        style: "padding-left: {indent}px;",
                                        onclick: {
                                            let ctx = context_id.clone();
                                            let anchor = anchor.clone();
                                            move |_| {
                                                let ctx = ctx.clone();
                                                let anchor = anchor.clone();
                                                let token = session.read().access_token.clone();
                                                spawn(async move {
                                                    if let Err(e) = graphql::set_screen_focus(token.as_deref(), &ctx, Some(&anchor)).await {
                                                        log::error!("set screen focus failed: {e}");
                                                    }
                                                });
                                            }
                                        },
                                        span { class: "material-icons", "cast" }
                                        div { class: "list-item-text",
                                            div { class: "list-item-primary", "{text}" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            // Live results: every poll's tally, with a close action for owners.
            div { class: "card",
                div { class: "card-header",
                    div { class: "avatar", {icon_el("vote/poll")} }
                    h3 { class: "title-medium", "{t(\"admin.results\")}" }
                }
                if polls.is_empty() {
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "how_to_vote" }
                        }
                        p { class: "empty-state-body", "{t(\"common.noContent\")}" }
                    }
                } else {
                    super::widgets::DataTable {
                        columns: vec![t("admin.poll"), t("admin.results"), t("admin.votes")],
                        for poll in polls.iter() {
                            AdminPollRow { key: "{poll.id.0}", poll: poll.clone(), can_manage }
                        }
                    }
                }
            }
        }
    }
}

/// One poll's row: its name and each option's vote count (live), plus a close
/// action for owners while the poll is open.
#[component]
fn AdminPollRow(poll: PollSummaryFields, #[props(default)] can_manage: bool) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let poll_id = poll.id.0.clone();

    let options: Vec<String> = poll
        .data
        .as_ref()
        .and_then(|d| d.0.get("options"))
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    let n_opts = options.len();
    let tally = crate::use_data_resource!(|(poll_id, access_token, n_opts)| async move {
        let votes = graphql::query_poll_votes(access_token.as_deref(), &poll_id)
            .await
            .unwrap_or_default();
        let mut counts = vec![0usize; n_opts];
        for vote in &votes {
            for &i in vote {
                if let Some(c) = counts.get_mut(i) {
                    *c += 1;
                }
            }
        }
        (counts, votes.len())
    });
    let (counts, total) = tally.read().clone().unwrap_or((vec![], 0));
    // A hidden-tally poll (data.hidden) gets an eye-off badge so an organizer can
    // tell at a glance which polls suppress the running count.
    let hidden = poll
        .data
        .as_ref()
        .and_then(|d| d.0.get("hidden"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let opts: Vec<String> = poll
        .data
        .as_ref()
        .and_then(|d| d.0.get("options"))
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let poll_id_close = poll.id.0.clone();

    rsx! {
        tr {
            td {
                span { class: "m3-cell-icon",
                    // Open (mutable) vs closed poll — the organizer's at-a-glance status.
                    span {
                        class: "material-icons",
                        title: if poll.mutable { "{t(\"speak.open\")}" } else { "{t(\"vote.closed\")}" },
                        style: if poll.mutable {
                            "font-size: 18px; color: var(--md-primary);"
                        } else {
                            "font-size: 18px; color: var(--md-on-surface-variant);"
                        },
                        if poll.mutable { "lock_open" } else { "lock" }
                    }
                    if hidden {
                        span {
                            class: "material-icons",
                            title: "{t(\"poll.hideResult\")}",
                            style: "font-size: 18px; color: var(--md-on-surface-variant);",
                            "visibility_off"
                        }
                    }
                    div { class: "list-item-primary", "{poll.name}" }
                }
            }
            td {
                div { class: "admin-results",
                    for (i , option) in opts.iter().enumerate() {
                        span { class: "chip",
                            span { class: "chip-label", "{option}: {counts.get(i).copied().unwrap_or(0)}" }
                        }
                    }
                }
            }
            td {
                span { class: "admin-total", "{total}" }
                if can_manage && poll.mutable {
                    button {
                        class: "btn-icon",
                        title: "{t(\"poll.stopPoll\")}",
                        onclick: move |_| {
                            let token = session.read().access_token.clone();
                            let poll_id = poll_id_close.clone();
                            spawn(async move {
                                match graphql::update_node(
                                    token.as_deref(),
                                    &poll_id,
                                    graphql::NodesSetInput {
                                        mutable: Some(false),
                                        ..Default::default()
                                    },
                                )
                                .await
                                {
                                    Ok(true) => crate::session::bump_data_version(),
                                    other => {
                                        log::error!("close poll failed: {other:?}");
                                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                    }
                                }
                            });
                        },
                        span { class: "material-icons", "stop" }
                    }
                }
            }
        }
    }
}
