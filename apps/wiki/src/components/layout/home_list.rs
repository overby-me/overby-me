use crate::model;
use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::{t, t_with};
use crate::route::Route;
use crate::session::use_session;

/// Everywhere the signed-in reader can go, in one load: the places they belong
/// to, by kind, and the ones they have been offered.
///
/// A struct rather than the tuple this was, because a third kind of place made
/// `Some(Ok((_, events, _)))` a puzzle to read at every use.
#[derive(Clone, PartialEq, Default)]
struct Places {
    groups: Vec<model::ContextNodeFields>,
    events: Vec<model::ContextNodeFields>,
    sites: Vec<model::ContextNodeFields>,
    invites: Vec<model::InvitationFields>,
}

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

    // The home list follows membership changes, on a timer rather than a live
    // query -- the same question, and for the same reason, as the invitation
    // badge in layout/mod.rs. It asks about the reader's OWN member rows, so
    // Hasura can share no work between readers and every one of them is a
    // cohort of one; at a congress that is a cost that grows with the number of
    // people in the room rather than with anything changing.
    //
    // Your own actions do not wait for this. Accepting an invitation is a
    // mutation, and a mutation bumps DATA_VERSION, which `use_data_resource!`
    // already re-runs on -- so the list is immediate for the person who changed
    // it. The timer is only for being added to something by somebody else while
    // you happen to be looking at the list.
    let mut refresh = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(90_000).await;
            refresh += 1;
        }
    });

    let contexts = crate::use_data_resource!(move || {
        let token = access_token.clone();
        let user_id = user_id.clone();
        let email = email.clone();
        let _ = refresh.read();
        async move {
            let Some(user_id) = user_id else {
                return Ok::<Places, String>(Places::default());
            };
            let groups = graphql::query_contexts(token.as_deref(), &user_id, "wiki/group").await?;
            let events = graphql::query_contexts(token.as_deref(), &user_id, "wiki/event").await?;
            let sites = graphql::query_contexts(token.as_deref(), &user_id, "wiki/site").await?;
            let invites = graphql::query_invitations(token.as_deref(), &user_id, &email)
                .await
                .unwrap_or_default();
            Ok(Places {
                groups,
                events,
                sites,
                invites,
            })
        }
    });

    // The root node backs the owner-only "add group / add event" actions: its id
    // and own context, plus which context mimes the signed-in user may create
    // there. Non-owners get an empty mime list (server-side `inserts` gate), so
    // the buttons never appear.
    let root_token = session.read().access_token.clone();
    let root_who = session.read().identity();
    let root = crate::use_data_resource!(|(root_token, root_who)| async move {
        let Some(node) = graphql::query_root_node(root_token.as_deref(), &root_who)
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

    let can_site = root_id.is_some() && root_mimes.iter().any(|m| m == "wiki/site");

    let state = contexts.read().clone();
    // One resource backs all three lists, so one failure is one failure. It used
    // to be logged from inside each list body, which meant a render-time log per
    // section per re-render for a single dropped request.
    let loading = state.is_none();
    let failed = matches!(state, Some(Err(_)));
    if let Some(Err(e)) = &state {
        crate::errors::log_handled("home places load failed", e);
    }
    let places = state.and_then(|r| r.ok()).unwrap_or_default();

    // Pending invitations, split into the list they belong to, so each shows
    // inline at the top of that one.
    let invited_by_mime = |mime: &str| -> Vec<model::InvitationFields> {
        places
            .invites
            .iter()
            .filter(|i| i.parent.as_ref().and_then(|p| p.mime_id.as_deref()) == Some(mime))
            .cloned()
            .collect()
    };
    let invited_groups = invited_by_mime("wiki/group");
    let invited_events = invited_by_mime("wiki/event");
    let invited_sites = invited_by_mime("wiki/site");

    // The groups a new event may be placed under (see NewContextButton).
    let group_choices = places.groups.clone();

    // DESIGN: on the home app (as_cards) each kind of place is its own card with
    // an icon-avatar header, so they read as distinct home sections rather than
    // one long list. The drawer keeps the compact bare list.
    //
    // Sites show only when there are some, when one has been offered, or when
    // this reader could make one. A site is rare beside groups and events, and a
    // permanently empty third heading in a narrow drawer is furniture.
    let show_sites = !places.sites.is_empty() || !invited_sites.is_empty() || can_site;

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
                div { class: "home-section-body",
                    ContextSection {
                        nodes: places.groups.clone(), invites: invited_groups.clone(),
                        empty_text: t("layout.noGroups"), empty_icon: "groups".to_string(),
                        loading, failed, as_cards,
                    }
                }
            }
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar small", span { class: "material-icons", "event" } }
                    h3 { class: "title-large", "{t(\"layout.events\")}" }
                    if can_event {
                        div { class: "flex-grow" }
                        NewContextButton { mime: "wiki/event".to_string(), root_id: rid.clone(), root_context_id: rctx.clone(), groups: group_choices.clone() }
                    }
                }
                div { class: "home-section-body",
                    ContextSection {
                        nodes: places.events.clone(), invites: invited_events.clone(),
                        empty_text: t("layout.noEvents"), empty_icon: "event".to_string(),
                        by_year: true, loading, failed, as_cards,
                    }
                }
            }
            if show_sites {
                div { class: "card mt-1",
                    div { class: "card-header",
                        div { class: "avatar small", span { class: "material-icons", "web" } }
                        h3 { class: "title-large", "{t(\"layout.sites\")}" }
                        if can_site {
                            div { class: "flex-grow" }
                            NewContextButton { mime: "wiki/site".to_string(), root_id: rid.clone(), root_context_id: rctx.clone() }
                        }
                    }
                    div { class: "home-section-body",
                        ContextSection {
                            nodes: places.sites.clone(), invites: invited_sites.clone(),
                            empty_text: t("layout.noSites"), empty_icon: "web".to_string(),
                            loading, failed, as_cards,
                        }
                    }
                }
            }
        }
    } else {
        rsx! {
            div { class: "mt-2",
                div { class: "list-section-header",
                    div { class: "avatar small", span { class: "material-icons", "groups" } }
                    h4 { class: "title-medium", "{t(\"layout.groups\")}" }
                    if can_group {
                        NewContextButton { mime: "wiki/group".to_string(), root_id: rid.clone(), root_context_id: rctx.clone() }
                    }
                }
                ContextSection {
                    nodes: places.groups.clone(), invites: invited_groups.clone(),
                    empty_text: t("layout.noGroups"), empty_icon: "groups".to_string(),
                    loading, failed, as_cards,
                }
                div { class: "list-section-header mt-1",
                    div { class: "avatar small", span { class: "material-icons", "event" } }
                    h4 { class: "title-medium", "{t(\"layout.events\")}" }
                    if can_event {
                        NewContextButton { mime: "wiki/event".to_string(), root_id: rid.clone(), root_context_id: rctx.clone(), groups: group_choices.clone() }
                    }
                }
                ContextSection {
                    nodes: places.events.clone(), invites: invited_events.clone(),
                    empty_text: t("layout.noEvents"), empty_icon: "event".to_string(),
                    by_year: true, loading, failed, as_cards,
                }
                if show_sites {
                    div { class: "list-section-header mt-1",
                        div { class: "avatar small", span { class: "material-icons", "web" } }
                        h4 { class: "title-medium", "{t(\"layout.sites\")}" }
                        if can_site {
                            NewContextButton { mime: "wiki/site".to_string(), root_id: rid.clone(), root_context_id: rctx.clone() }
                        }
                    }
                    ContextSection {
                        nodes: places.sites.clone(), invites: invited_sites.clone(),
                        empty_text: t("layout.noSites"), empty_icon: "web".to_string(),
                        loading, failed, as_cards,
                    }
                }
            }
        }
    }
}

/// One list of places under its heading: the invitations that need answering,
/// then the places themselves, shortened to the newest few with a way to see the
/// rest.
///
/// This was written out once per kind, and adding a third would have been a
/// third copy of eighty lines of rsx that differed in a label, a glyph, and
/// whether the expanded list buckets by year.
#[component]
fn ContextSection(
    nodes: Vec<model::ContextNodeFields>,
    invites: Vec<model::InvitationFields>,
    /// What to say when there is nothing, and the glyph for the card variant's
    /// orb.
    empty_text: String,
    empty_icon: String,
    /// Bucket the expanded list by year. Events only: the roster of past meetings
    /// is long and reads as history, where a handful of groups or sites does not.
    #[props(default)]
    by_year: bool,
    loading: bool,
    failed: bool,
    as_cards: bool,
) -> Element {
    // Keep the list short by default (newest first) and let the reader open it.
    const LIST_LIMIT: usize = 4;
    let mut expanded = use_signal(|| false);

    if loading {
        return rsx! {
            p { class: "body-medium list-subheader", "…" }
        };
    }
    if failed {
        return rsx! {
            crate::components::widgets::ErrorState {
                title: t("error.somethingWentWrong"),
                small: true,
                // This list is the first thing a signed-in reader sees, and the
                // way it fails is a dropped request rather than a bad one: a
                // phone changing cell on the way into the venue. Without this the
                // only way back is reloading the page. Bumping the data version
                // rather than re-running one resource is deliberate: it is what
                // pull-to-refresh means here, and a connection that dropped one
                // query usually dropped its neighbours too.
                on_retry: move |_| crate::session::bump_data_version(),
            }
        };
    }
    if nodes.is_empty() && invites.is_empty() {
        // Cards get the orb empty state the rest of the app uses; the drawer
        // keeps the compact line, where an orb would be a lot of furniture in a
        // narrow rail.
        return rsx! {
            if as_cards {
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "{empty_icon}" }
                    }
                    p { class: "empty-state-body", "{empty_text}" }
                }
            } else {
                p { class: "body-medium list-subheader", "{empty_text}" }
            }
        };
    }

    let is_expanded = *expanded.read();
    let total = nodes.len();
    rsx! {
        // Invitations first: they need answering.
        if !invites.is_empty() {
            div { class: "list",
                for inv in invites.iter() {
                    InvitedContextItem { key: "inv-{inv.id.0}", invite: inv.clone() }
                }
            }
        }
        if is_expanded && by_year {
            for (year , items) in group_by_year(&nodes) {
                div { key: "{year}",
                    p { class: "title-small list-subheader", "{year}" }
                    div { class: "list",
                        for node in items.iter() {
                            ContextItem { key: "{node.id.0}", node: node.clone() }
                        }
                    }
                }
            }
        } else {
            div { class: "list",
                for node in nodes.iter().take(if is_expanded { total } else { LIST_LIMIT }) {
                    ContextItem { key: "{node.id.0}", node: node.clone() }
                }
            }
        }
        if total > LIST_LIMIT {
            button {
                class: "btn btn-text",
                // Stop the click reaching the drawer's close-on-item handler, so
                // expanding the list on mobile does not dismiss the drawer.
                onclick: move |evt: Event<MouseData>| {
                    evt.stop_propagation();
                    let e = *expanded.read();
                    expanded.set(!e);
                },
                if is_expanded {
                    "{t(\"layout.showLess\")}"
                } else {
                    "{t_with(\"layout.showAll\", &[(\"count\", &total.to_string())])}"
                }
            }
        }
    }
}

/// A per-list "add group" / "add event" action, rendered in a list header for
/// the root owner (the caller gates on the root's `inserts`). It opens a name
/// dialog and drives [`graphql::create_context`], which creates the node under
/// the chosen parent, makes it its own context, and seeds the permission
/// template — so the new group/event is usable immediately. On success it jumps
/// into it.
///
/// An event may be placed inside one of the user's groups instead of at the top
/// level; `groups` carries the choices (empty for the group button, which always
/// creates at the top level).
#[component]
fn NewContextButton(
    mime: String,
    root_id: String,
    root_context_id: String,
    #[props(default = Vec::new())] groups: Vec<model::ContextNodeFields>,
) -> Element {
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
    // The group to create the event under; empty means the top level (the root).
    let mut parent = use_signal(String::new);

    let submit = {
        let groups = groups.clone();
        move |_| {
            let title = name.read().trim().to_string();
            if title.is_empty() || *busy.read() {
                return;
            }
            let mime = mime.clone();
            // A group is its own context (create_context locks it that way), so the
            // chosen group serves as both parent and context. An unknown or empty
            // selection falls back to the root rather than guessing.
            let chosen = parent.read().clone();
            let chosen = groups.iter().find(|g| g.id.0 == chosen);
            let (root_id, root_context_id) = match chosen {
                Some(g) => (g.id.0.clone(), g.id.0.clone()),
                None => (root_id.clone(), root_context_id.clone()),
            };
            // The new node sits under the group it was placed in, so its path is
            // prefixed by that group's key — navigating to the bare key would 404.
            let parent_segments: Vec<String> =
                chosen.map(|g| vec![g.key.clone()]).unwrap_or_default();
            let token = session.read().access_token.clone();
            // The creator becomes the new context's first owner member, so the
            // owner-only surfaces (members, console) show for them right away.
            let creator = session.read().user.clone();
            busy.set(true);
            error.set(String::new());
            spawn(async move {
                match graphql::create_context(
                    token.as_deref(),
                    &root_id,
                    &root_context_id,
                    &mime,
                    &title,
                    creator.as_ref(),
                )
                .await
                {
                    Ok(inserted) => {
                        crate::session::bump_data_version();
                        busy.set(false);
                        open.set(false);
                        name.set(String::new());
                        let mut segments = parent_segments;
                        segments.push(inserted.key);
                        nav.push(Route::PathPage {
                            segments,
                            app: None,
                        });
                    }
                    Err(e) => {
                        busy.set(false);
                        crate::errors::log_handled("create context failed", e);
                        error.set(t("layout.createFailed"));
                    }
                }
            });
        }
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
            // A form, so it takes the screen on a phone (see widgets::Dialog).
            form: true,
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
            // Where the event goes. Only for events, and only when there is a
            // group to choose — otherwise the top level is the only answer and a
            // one-option select is just noise.
            if !is_group && !groups.is_empty() {
                div { class: "text-field mt-2",
                    label { "{t(\"layout.parentGroup\")}" }
                    select {
                        value: "{parent}",
                        onchange: move |e| parent.set(e.value()),
                        option { value: "", "{t(\"layout.topLevel\")}" }
                        for g in groups.iter() {
                            option { key: "{g.id.0}", value: "{g.id.0}", "{g.name}" }
                        }
                    }
                }
            }
            if !error.read().is_empty() {
                p { class: "body-medium text-error mt-1", "{error}" }
            }
        }
    }
}

/// A single group/event entry. Clicking resolves the node's path and navigates.
#[component]
pub(super) fn ContextItem(node: model::ContextNodeFields) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let name = node.name.clone();
    let node_id = node.id.0.clone();
    let abbr = abbrev_context_name(&name);

    rsx! {
        div {
            class: "list-item",
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
pub(super) fn InvitedContextItem(invite: model::InvitationFields) -> Element {
    let session = use_session();
    let mut confirm_open = use_signal(|| false);
    // Optimistic: accepting or declining hides the invite at once; the refetch then
    // removes it for good. On failure the row is restored (dismissed back to false).
    let mut dismissed = use_signal(|| false);
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
            // Optimistic: hide the invite immediately.
            dismissed.set(true);
            spawn(async move {
                let mut ok = false;
                if let Some(uid) = uid {
                    // Already in this room? Then the invitation is a SECOND row
                    // for the same person -- someone was invited by mail who was
                    // already a member -- and the row to keep is the membership
                    // they have: accept that, and the invitation goes.
                    //
                    // Asked FIRST, and this is the whole point of the order: a
                    // membership is unique per (context, person), so claiming
                    // the invitation onto their node is a write the database is
                    // bound to refuse, and that refusal was reaching the error
                    // log as a fault. It is not a fault. Where they are NOT a
                    // member this touches nothing and says so, at the cost of
                    // one round trip on an action taken once.
                    let already = match &parent_id {
                        Some(pid) => graphql::accept_existing_member(token.as_deref(), pid, &uid)
                            .await
                            .unwrap_or(false),
                        None => false,
                    };
                    if already {
                        let _ = graphql::decline_invitation(token.as_deref(), &member_id).await;
                        ok = true;
                    } else {
                        ok = graphql::accept_invitation(token.as_deref(), &member_id, &uid)
                            .await
                            .unwrap_or(false);
                    }
                    crate::session::bump_data_version();
                }
                if !ok {
                    // Restore the row and report the failure.
                    dismissed.set(false);
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
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
            // Optimistic: hide the invite immediately; restore on error.
            dismissed.set(true);
            spawn(async move {
                match graphql::decline_invitation(token.as_deref(), &member_id).await {
                    Ok(_) => crate::session::bump_data_version(),
                    Err(e) => {
                        dismissed.set(false);
                        crate::errors::log_handled("decline invitation failed", e);
                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                    }
                }
            });
        }
    };

    // Optimistically removed: render nothing until the refetch drops it for good.
    if dismissed() {
        return rsx! {};
    }

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
    events: &[model::ContextNodeFields],
) -> Vec<(String, Vec<model::ContextNodeFields>)> {
    let mut out: Vec<(String, Vec<model::ContextNodeFields>)> = Vec::new();
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
            // The is_empty filter above guarantees a first char.
            let Some(first) = w.chars().next() else {
                return false;
            };
            let has_digit = w.chars().any(|c| c.is_ascii_digit());
            (first.is_uppercase() && !(has_digit && w.chars().count() > 1)) || upper_count(w) > 1
        })
        .map(|w| match w {
            "Hovedbestyrelsesmøde" => "HB".to_string(),
            "Landsmøde" => "LM".to_string(),
            // An acronym (e.g. "EU-"): keep only its uppercase letters, so
            // trailing punctuation like a hyphen cannot break onto a new line.
            _ if upper_count(w) > 1 => w.chars().filter(|c| c.is_uppercase()).collect(),
            _ => w.chars().next().map(String::from).unwrap_or_default(),
        })
        .collect();

    match words.len() {
        // Two characters sit comfortably in the avatar circle (e.g. "EU", "KM",
        // "HB"); more than that reads as crammed.
        1..=3 => words.concat().chars().take(2).collect(),
        _ => String::new(),
    }
}
