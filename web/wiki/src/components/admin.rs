use crate::model;
use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::{NodeWithChildren, PollSummaryFields};
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
/// Which tab body the console shows, given the stored selection and whether the
/// agenda has a pane of its own.
///
/// Agenda is tab 0. With the agenda always on screen its tab would select a pane
/// that is already there, so the wide layout starts at the speaker list. The
/// stored selection is deliberately NOT rewritten: narrowing the window puts the
/// reader back on the agenda they had chosen, rather than on a tab they never did.
fn console_tab(selected: usize, wide: bool) -> usize {
    if wide && selected == 0 {
        1
    } else {
        selected
    }
}

/// Where the wide tab bar's index sits in the stored selection: the wide bar is
/// the same bar without its first tab, so both directions shift by one.
fn console_tab_from_bar(bar_index: usize, wide: bool) -> usize {
    if wide {
        bar_index + 1
    } else {
        bar_index
    }
}

/// The reverse: which entry of the bar is highlighted for a stored selection.
fn console_bar_index(selected: usize, wide: bool) -> usize {
    if wide {
        console_tab(selected, wide).saturating_sub(1)
    } else {
        selected
    }
}

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
        crate::graphql::relations_changed(crate::graphql::relation_named(&sub_ctx, "active")),
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

    // Which nodes are unfolded in the agenda. Held here, not per row, so a whole
    // meeting's structure can be folded away at once: a chair looking for the next
    // item wants the shape of the day on one screen, not one item and its
    // paperwork.
    let mut expanded = use_signal(std::collections::HashSet::<String>::new);
    // Which panel is open. Local state, not a route: a chair switching between the
    // agenda and the speaker list is not going anywhere, and their place in the
    // agenda should survive the trip.
    let mut tab = use_signal(|| 0usize);

    // Whether the room's screen is currently showing the feed rather than an
    // agenda item. Live, so two chairs on two devices agree.
    let feed_ctx = context_id.clone();
    let feed_token = access_token.clone();
    let screen_feed = crate::use_data_resource!(|(feed_ctx, feed_token, rev)| async move {
        let _ = rev;
        graphql::screen_feed_on(feed_token.as_deref(), &feed_ctx)
            .await
            .unwrap_or(false)
    });
    let screen_feed = screen_feed.read().unwrap_or(false);

    let polls_ctx = context_id.clone();
    let polls = crate::use_data_resource!(|(polls_ctx, access_token)| async move {
        graphql::query_context_polls(access_token.as_deref(), &polls_ctx)
            .await
            .unwrap_or_default()
    });
    let polls = polls.read().clone().unwrap_or_default();

    // Wide enough for two panes, and the agenda stops being a tab: it becomes
    // the console's supporting pane, standing beside whatever the chair is
    // working in. Running a meeting is reading the agenda WHILE giving someone
    // the floor or opening a poll, and a tab bar makes those alternatives — the
    // chair kept flicking back to see where they were.
    // Read the class rather than install the bridge: `use_window_size` leaks a
    // resize listener holding a handle owned by the calling scope, and the console
    // is a page that comes and goes (see `window_size::BRIDGED`).
    let wide = crate::window_size::WINDOW_SIZE().is_expanded_rail();
    let sel = console_tab(tab(), wide);

    let agenda_pane = rsx! {
            div { class: "card app-card",
                div { class: "card-header",
                    div { class: "avatar", {icon_el("app/program")} }
                    h3 { class: "title-medium", "{t(\"console.agenda\")}" }
                    div { class: "flex-grow" }
                    if !expanded.read().is_empty() {
                        button {
                            class: "btn-icon",
                            title: "{t(\"console.collapseAll\")}",
                            aria_label: "{t(\"console.collapseAll\")}",
                            onclick: move |_| expanded.write().clear(),
                            span { class: "material-icons", "unfold_less" }
                        }
                    }
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
                // The whole tree, not just the top level: an agenda item's papers
                // (a motion's amendments, a folder's documents) are what the room
                // actually turns to, and the chair had to leave the console to
                // reach them. Each level is fetched only when it is opened.
                AgendaLevel {
                    parent_id: node.id.0.clone(),
                    path: segments.clone(),
                    depth: 0,
                    context_id: context_id.clone(),
                    can_manage,
                    active_id: active_id.clone(),
                    projected: projected_opt,
                    expanded,
                }
            }

            // Projector focus: for a long active document, scroll the room's screen
            // to a chosen section (or back to the whole document). Shown whenever
            // something is projected; when the item has no sections, the card stays
            // (with a note) rather than silently vanishing.
            if can_manage && active_id.is_some() {
                div { class: "card app-card",
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
                                            crate::errors::log_handled("clear screen focus failed", e);
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
                            p { class: "body-small list-subheader",
                                "{t(\"console.focusNoSections\")}"
                            }
                        }
                        for (anchor , text , level) in active_headings.iter() {
                            {
                                // Indent by heading depth ON the spacing scale (the
                                // shape the drawer tree uses), so a density change
                                // moves the console with everything else.
                                let depth = (*level as usize).saturating_sub(1);
                                let anchor = anchor.clone();
                                rsx! {
                                    button {
                                        key: "{anchor}",
                                        class: "list-item admin-focus-item",
                                        style: "padding-left: calc(var(--md-sys-spacing-3) + {depth} * var(--md-sys-spacing-3));",
                                        onclick: {
                                            let ctx = context_id.clone();
                                            let anchor = anchor.clone();
                                            move |_| {
                                                let ctx = ctx.clone();
                                                let anchor = anchor.clone();
                                                let token = session.read().access_token.clone();
                                                spawn(async move {
                                                    if let Err(e) = graphql::set_screen_focus(token.as_deref(), &ctx, Some(&anchor)).await {
                                                        crate::errors::log_handled("set screen focus failed", e);
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
    };

    let tab_bar = rsx! {
        super::widgets::Tabs {
            tabs: if wide {
                vec![
                    (t("mime.speak"), "record_voice_over".to_string()),
                    (t("mime.vote"), "how_to_vote".to_string()),
                    (t("layout.feed"), "view_agenda".to_string()),
                ]
            } else {
                vec![
                    (t("console.agenda"), "list_alt".to_string()),
                    (t("mime.speak"), "record_voice_over".to_string()),
                    (t("mime.vote"), "how_to_vote".to_string()),
                    (t("layout.feed"), "view_agenda".to_string()),
                ]
            },
            // The wide bar is the same bar without its first tab, so both the
            // reading and the writing of the selection shift by one.
            selected: console_bar_index(tab(), wide),
            on_select: move |i: usize| tab.set(console_tab_from_bar(i, wide)),
        }
    };

    let tab_body = rsx! {
            // ── Speak ───────────────────────────────────────────────────────
            // The speaker list in place. It was a link out of the console, which
            // meant the chair left the agenda to give someone the floor and had to
            // find their way back.
            if sel == 1 {
                super::speak::SpeakApp { node: node.clone(), mode: super::speak::SpeakMode::Full }
            }
            // ── Polls ───────────────────────────────────────────────────────
            if sel == 2 {
                div { class: "card app-card",
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

            // ── Feed ────────────────────────────────────────────────────────
            // What has landed in this context while the meeting ran: an amendment
            // posted from the floor shows up here without the chair going looking.
            if sel == 3 {
                div { class: "card app-card",
                    div { class: "card-header",
                        div { class: "avatar small", span { class: "material-icons", "view_agenda" } }
                        h3 { class: "title-medium", "{t(\"layout.feed\")}" }
                    }
                    crate::components::feed::FeedList {
                        context_id: context_id.clone(),
                        autoload: true,
                        // The chair is watching for what just landed, not reading.
                        instant: true,
                    }
                }
            }
    };

    rsx! {
        div {
            // The room-facing views are still links: a projector and a follower
            // screen are somewhere the chair SENDS the room, not something they
            // work in, so they leave the console on purpose. Everything they work
            // in is a tab below.
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
                // Put the feed on the room's screen. Not an agenda item, so it is
                // not something the agenda can project: it is its own instruction
                // about what the room should be looking at, and it wins over
                // whatever was last projected until the chair turns it off.
                if can_manage {
                    button {
                        class: if screen_feed { "btn btn-filled" } else { "btn btn-tonal" },
                        "aria-pressed": if screen_feed { "true" } else { "false" },
                        onclick: {
                            let ctx = context_id.clone();
                            move |_| {
                                let ctx = ctx.clone();
                                let token = session.read().access_token.clone();
                                let next = !screen_feed;
                                spawn(async move {
                                    match graphql::set_screen_feed(token.as_deref(), &ctx, next).await {
                                        Ok(_) => crate::session::bump_data_version(),
                                        Err(e) => {
                                            crate::errors::log_handled("screen feed toggle failed", e);
                                            crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                        }
                                    }
                                });
                            }
                        },
                        span { class: "material-icons", "view_agenda" }
                        if screen_feed { "{t(\"console.feedOnScreenStop\")}" } else { "{t(\"console.feedOnScreen\")}" }
                    }
                }
            }

            // Two panes where they fit, one column where they do not. The pane
            // scaffold decides that from the width of the column it is in, so a
            // docked tools sheet or an open tree collapses it back on its own.
            if wide {
                super::widgets::SupportingPaneLayout {
                    primary: rsx! {
                        {tab_bar}
                        {tab_body}
                    },
                    supporting: rsx! { {agenda_pane} },
                }
            } else {
                {tab_bar}
                if sel == 0 {
                    {agenda_pane}
                }
                {tab_body}
            }
        }
    }
}

/// One level of the agenda tree: the content children of `parent_id`, in meeting
/// order, each projectable and each unfoldable if it has papers of its own.
///
/// Lazy by level, like the drawer's tree: a congress is thousands of nodes and
/// the chair opens a handful of them. Recursion is what keeps a row identical at
/// every depth, so a motion's amendment projects exactly the way the motion does.
#[component]
fn AgendaLevel(
    parent_id: String,
    path: Vec<String>,
    depth: usize,
    context_id: String,
    can_manage: bool,
    active_id: Option<String>,
    projected: Signal<Option<Option<String>>>,
    expanded: Signal<std::collections::HashSet<String>>,
) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let parent = parent_id.clone();

    let children = crate::use_data_resource!(|(parent, token, user_id)| async move {
        let Some(user_id) = user_id else {
            return Ok(Vec::new());
        };
        graphql::query_drawer_children(token.as_deref(), &parent, &user_id).await
    });

    let load = children.read().clone();
    let failed = matches!(load, Some(Err(_)));
    if let Some(Err(e)) = &load {
        crate::errors::log_handled("agenda level load failed", e);
    }
    let items: Vec<_> = load
        .and_then(|r| r.ok())
        .unwrap_or_default()
        .into_iter()
        .filter(|c| is_agenda_item(c.mime_id.as_deref().unwrap_or("")))
        .collect();

    if items.is_empty() {
        // Only the top level speaks up about being empty; a leaf that turned out
        // to have no agenda children just stops. A failed load does speak up
        // wherever it happens: a branch that silently stopped expanding read as
        // an agenda item with nothing under it, which is a thing a chair acts on.
        return rsx! {
            if failed {
                crate::components::widgets::ErrorState {
                    title: t("error.couldNotLoad"),
                    small: true,
                    on_retry: move |_| crate::session::bump_data_version(),
                }
            } else if depth == 0 {
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "list_alt" }
                    }
                    p { class: "empty-state-body", "{t(\"common.noContent\")}" }
                }
            }
        };
    }

    rsx! {
        div { class: if depth == 0 { "list" } else { "list agenda-children" },
            for item in items.iter() {
                AgendaRow {
                    key: "{item.id.0}",
                    item: item.clone(),
                    path: path.clone(),
                    depth,
                    context_id: context_id.clone(),
                    can_manage,
                    active_id: active_id.clone(),
                    projected,
                    expanded,
                }
            }
        }
    }
}

/// One agenda row, and its subtree when unfolded.
#[component]
fn AgendaRow(
    item: model::DrawerChildFields,
    path: Vec<String>,
    depth: usize,
    context_id: String,
    can_manage: bool,
    active_id: Option<String>,
    mut projected: Signal<Option<Option<String>>>,
    mut expanded: Signal<std::collections::HashSet<String>>,
) -> Element {
    let session = use_session();
    let item_id = item.id.0.clone();
    let is_active = active_id.as_deref() == Some(item_id.as_str());
    let is_open = expanded.read().contains(&item_id);
    let has_children = item.has_children();

    let mut item_path = path.clone();
    item_path.push(item.key.clone());

    // Indent on the spacing scale, the same step the drawer tree uses, so a
    // density change moves both together.
    let indent =
        format!("padding-left: calc(var(--md-sys-spacing-3) + {depth} * var(--nav-indent-step));");

    rsx! {
        div {
            class: if is_active { "list-item agenda-item active" } else { "list-item agenda-item" },
            style: "{indent}",
            if has_children {
                button {
                    class: "btn-icon agenda-expander",
                    aria_label: "{t(\"common.expand\")}",
                    "aria-expanded": if is_open { "true" } else { "false" },
                    onclick: {
                        let id = item_id.clone();
                        move |_| {
                            let mut set = expanded.write();
                            if !set.remove(&id) {
                                set.insert(id.clone());
                            }
                        }
                    },
                    span { class: "material-icons",
                        if is_open { "expand_more" } else { "chevron_right" }
                    }
                }
            } else {
                // Keeps the names of childless rows on the same line as the rest.
                span { class: "agenda-expander-spacer" }
            }
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
                    onclick: {
                        let ctx = context_id.clone();
                        let id = item_id.clone();
                        move |_| {
                            let ctx = ctx.clone();
                            let id = id.clone();
                            let token = session.read().access_token.clone();
                            // Optimistic: highlight this item as projected now.
                            projected.set(Some(Some(id.clone())));
                            spawn(async move {
                                match graphql::set_active_relation(token.as_deref(), &ctx, Some(&id)).await {
                                    Ok(_) => crate::snackbar::show_snackbar(&t("content.projected")),
                                    Err(_) => {
                                        projected.set(None);
                                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                    }
                                }
                            });
                        }
                    },
                    span { class: "material-icons", "cast" }
                    "{t(\"content.projectScreen\")}"
                }
            }
        }
        if has_children && is_open {
            AgendaLevel {
                parent_id: item.id.0.clone(),
                path: {
                    let mut p = path.clone();
                    p.push(item.key.clone());
                    p
                },
                depth: depth + 1,
                context_id: context_id.clone(),
                can_manage,
                active_id: active_id.clone(),
                projected,
                expanded,
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
                        // Open reads in the accent, closed in the muted tone —
                        // the same status pairing the rest of the app uses.
                        class: if poll.mutable { "material-icons text-accent" } else { "material-icons text-muted" },
                        title: if poll.mutable { "{t(\"speak.open\")}" } else { "{t(\"vote.closed\")}" },
                        if poll.mutable { "lock_open" } else { "lock" }
                    }
                    if hidden {
                        span {
                            class: "material-icons text-muted",
                            title: "{t(\"poll.hideResult\")}",
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
                                    model::NodesSetInput {
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

#[cfg(test)]
mod tests {
    use super::{console_bar_index, console_tab, console_tab_from_bar};

    #[test]
    fn narrow_console_keeps_every_tab() {
        for i in 0..4 {
            assert_eq!(console_tab(i, false), i);
            assert_eq!(console_bar_index(i, false), i);
            assert_eq!(console_tab_from_bar(i, false), i);
        }
    }

    #[test]
    fn wide_console_drops_the_agenda_tab() {
        // The agenda has its own pane, so its tab is gone and the body starts at
        // the speaker list. Everything else keeps the body it selected.
        assert_eq!(console_tab(0, true), 1);
        for i in 1..4 {
            assert_eq!(console_tab(i, true), i);
        }
    }

    #[test]
    fn wide_bar_indices_round_trip() {
        // Whatever the bar reports must come back as the same highlighted entry,
        // or clicking Polls would leave Speak underlined.
        for bar in 0..3 {
            let stored = console_tab_from_bar(bar, true);
            assert_eq!(console_bar_index(stored, true), bar);
        }
        // A stored agenda selection highlights the first wide entry rather than
        // underflowing to the last one.
        assert_eq!(console_bar_index(0, true), 0);
    }
}
