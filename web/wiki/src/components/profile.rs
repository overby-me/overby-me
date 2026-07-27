use crate::model;
use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, user_avatar};

/// ProfileApp — the signed-in user's profile (#78): who they are plus the groups
/// and events they belong to. Reachable via `?app=profile`.
#[component]
pub fn ProfileApp() -> Element {
    let session = use_session();
    let user = session.read().user.clone();
    let access_token = session.read().access_token.clone();
    let user_id = user.as_ref().map(|u| u.id.clone());
    // Bluesky handle to link — declared before the no-user early return below so
    // the hook order stays stable across renders.
    let mut bsky_handle = use_signal(String::new);
    // Whether the account was just unlinked this session (flips the card back to
    // the link form without needing to refetch), and the current link status.
    let mut just_unlinked = use_signal(|| false);
    let status_token = access_token.clone();
    let bsky_status = crate::use_data_resource!(move || {
        let token = status_token.clone();
        async move {
            match token {
                Some(t) => crate::backend_api::atproto_status(&t).await,
                None => crate::backend_api::AtprotoLink::default(),
            }
        }
    });

    // Bluesky handle typeahead: preview matching accounts as the user types, so
    // they can pick their handle instead of typing it exactly. Hidden once a
    // suggestion is chosen (until they edit the field again).
    let mut sug_dismissed = use_signal(|| false);
    let suggestions = crate::use_data_resource!(move || {
        let q = bsky_handle.read().trim().to_string();
        async move {
            if q.len() < 2 {
                return Vec::new();
            }
            // Debounce: while the user keeps typing, this resource re-runs and the
            // pending future is dropped before the request fires.
            gloo_timers::future::TimeoutFuture::new(220).await;
            crate::backend_api::search_bsky_actors(&q).await
        }
    });

    // Background Web Push (#128): whether THIS browser is subscribed (None until
    // the async check resolves). Declared before the early return for hook order.
    let mut push_on = use_signal(|| None::<bool>);
    use_hook(|| {
        spawn(async move {
            push_on.set(Some(crate::pwa::push_subscribed().await));
        });
    });

    // The user's most recent authored contributions (resolutions, amendments,
    // candidacies, comments, questions), each linking to the item.
    let contrib_token = access_token.clone();
    let contrib_uid = user_id.clone();
    let contributions = crate::use_data_resource!(|(contrib_token, contrib_uid)| async move {
        match contrib_uid {
            Some(uid) => {
                graphql::query_user_contributions(contrib_token.as_deref(), &uid, 12).await
            }
            None => Vec::new(),
        }
    });

    // The user's groups + events (same query the home list uses), returned as two
    // separate lists. A Result so a failed load shows an error state, not an empty
    // membership list.
    let memberships = crate::use_data_resource!(move || {
        let token = access_token.clone();
        let uid = user_id.clone();
        async move {
            type Lists = (Vec<model::ContextNodeFields>, Vec<model::ContextNodeFields>);
            let Some(uid) = uid else {
                return Ok::<Lists, String>((Vec::new(), Vec::new()));
            };
            let groups = graphql::query_contexts(token.as_deref(), &uid, "wiki/group").await?;
            let events = graphql::query_contexts(token.as_deref(), &uid, "wiki/event").await?;
            Ok((groups, events))
        }
    });

    let Some(user) = user else {
        return rsx! {
            div { class: "card",
                div { class: "card-content",
                    p { class: "body-large", "{t(\"node.maybeLoginForAccess\")}" }
                    Link { to: Route::Login {}, class: "btn btn-primary", "{t(\"common.logIn\")}" }
                }
            }
        };
    };

    let mem_state = memberships.read().clone();
    let contrib_state = contributions.read().clone();
    let link = bsky_status.read().clone().unwrap_or_default();
    let show_linked = link.linked && !*just_unlinked.read();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {user_avatar(&user.avatar_url, icon_el("app/profile"))} }
                div {
                    h3 { class: "title-medium", "{user.display_name}" }
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"profile.signedInAs\")} {user.email}"
                    }
                }
            }
            div { class: "card-content",
                p {
                    class: "body-small",
                    class: "text-muted",
                    "{t(\"profile.userId\")}: {user.id}"
                }
            }
        }

        // Bluesky (atproto) account: when linked, show the handle + an unlink
        // action; otherwise hand off to the backend OAuth flow with the handle +
        // current NHost access token (it redirects back to APP_ORIGIN with
        // ?linked=bluesky|error, surfaced in a snackbar by App on load).
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small", {crate::components::loader::bsky_logo()} }
                h3 { class: "title-medium",
                    if show_linked { "{t(\"profile.blueskyAccount\")}" } else { "{t(\"profile.linkBluesky\")}" }
                }
            }
            div { class: "card-content",
                if show_linked {
                    p { class: "body-medium mb-1",
                        "{t(\"profile.linkedAs\")} "
                        a {
                            class: "link-accent",
                            href: "https://bsky.app/profile/{link.handle}",
                            target: "_blank",
                            rel: "noopener noreferrer",
                            "@{link.handle}"
                        }
                    }
                    button {
                        class: "btn btn-secondary mt-1",
                        onclick: move |_| {
                            let tok = session.read().access_token.clone();
                            spawn(async move {
                                let Some(tok) = tok else { return };
                                if crate::backend_api::atproto_unlink(&tok).await {
                                    just_unlinked.set(true);
                                    crate::snackbar::show_snackbar(&t("profile.unlinkedOk"));
                                } else {
                                    crate::snackbar::show_snackbar(&t("profile.unlinkErr"));
                                }
                            });
                        },
                        span { class: "material-icons", "link_off" }
                        " {t(\"profile.unlink\")}"
                    }
                } else {
                    p { class: "body-medium text-muted mb-1", "{t(\"profile.linkBlueskyHint\")}" }
                    div { class: "text-field",
                        label { "{t(\"profile.blueskyHandle\")}" }
                        input {
                            r#type: "text",
                            placeholder: "alice.bsky.social",
                            autocomplete: "off",
                            value: "{bsky_handle}",
                            oninput: move |e| {
                                bsky_handle.set(e.value());
                                sug_dismissed.set(false);
                            },
                        }
                    }
                    // Live preview of matching Bluesky accounts — click to pick.
                    {
                        let items = suggestions.read().clone().unwrap_or_default();
                        if !items.is_empty() && !*sug_dismissed.read() {
                            rsx! {
                                div { class: "list mt-1",
                                    for actor in items.iter() {
                                        {
                                            let handle = actor.handle.clone();
                                            rsx! {
                                                div {
                                                    key: "{actor.handle}",
                                                    class: "list-item",
                                                    onclick: move |_| {
                                                        bsky_handle.set(handle.clone());
                                                        sug_dismissed.set(true);
                                                    },
                                                    div { class: "avatar small",
                                                        {user_avatar(&actor.avatar, icon_el("wiki/user"))}
                                                    }
                                                    div { class: "list-item-text",
                                                        div { class: "list-item-primary", "@{actor.handle}" }
                                                        if !actor.display_name.is_empty() {
                                                            div { class: "list-item-secondary", "{actor.display_name}" }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } else {
                            rsx! {}
                        }
                    }
                    button {
                        class: "btn btn-primary mt-1",
                        disabled: bsky_handle.read().trim().is_empty(),
                        onclick: move |_| {
                            let handle = bsky_handle.read().trim().to_string();
                            let token = session.read().access_token.clone();
                            if let (false, Some(token)) = (handle.is_empty(), token) {
                                // handle (a domain) and the base64url JWT are URL-safe.
                                let url = crate::backend_api::atproto_start_url(&handle, &token);
                                if let Some(w) = web_sys::window() {
                                    let _ = w.location().set_href(&url);
                                }
                            }
                        },
                        span { class: "material-icons", "link" }
                        " {t(\"profile.linkBluesky\")}"
                    }
                }
            }
        }

        // Background Web Push: opt this device in/out of notifications that arrive
        // even when the app is closed (e.g. a vote opening in a group you're in).
        if crate::pwa::push_supported() {
            div { class: "card",
                div { class: "card-header",
                    div { class: "avatar small", span { class: "material-icons", "notifications" } }
                    h3 { class: "title-medium", "{t(\"profile.notifications\")}" }
                }
                div { class: "card-content",
                    p { class: "body-medium text-muted mb-1", "{t(\"profile.notifyHint\")}" }
                    {
                        let on = *push_on.read();
                        let label = match on {
                            Some(true) => t("profile.disableNotify"),
                            _ => t("profile.enableNotify"),
                        };
                        rsx! {
                            button {
                                class: if on == Some(true) { "btn btn-secondary mt-1" } else { "btn btn-primary mt-1" },
                                disabled: on.is_none(),
                                onclick: move |_| {
                                    let tok = session.read().access_token.clone();
                                    let currently = (*push_on.read()).unwrap_or(false);
                                    spawn(async move {
                                        let Some(tok) = tok else { return };
                                        if currently {
                                            let _ = crate::pwa::unsubscribe_push(&tok).await;
                                            push_on.set(Some(false));
                                            crate::snackbar::show_snackbar(&t("profile.notifyDisabled"));
                                        } else {
                                            match crate::pwa::subscribe_push(&tok).await {
                                                Ok(()) => {
                                                    push_on.set(Some(true));
                                                    crate::snackbar::show_snackbar(&t("profile.notifyEnabled"));
                                                }
                                                Err(_) => {
                                                    crate::snackbar::show_snackbar(&t("profile.notifyErr"));
                                                }
                                            }
                                        }
                                    });
                                },
                                span { class: "material-icons",
                                    if on == Some(true) { "notifications_off" } else { "notifications_active" }
                                }
                                " {label}"
                            }
                        }
                    }
                }
            }
        }

        // Groups and Events as separate lists (mirroring the home page).
        {
            match &mem_state {
                None => rsx! {
                    div { class: "card",
                        div { class: "card-content", crate::components::widgets::Spinner {} }
                    }
                },
                Some(Err(e)) => {
                    log::error!("Loading memberships failed: {e}");
                    rsx! {
                        crate::components::widgets::ErrorState {
                            title: t("error.somethingWentWrong"),
                            small: true,
                        }
                    }
                }
                Some(Ok((groups, events))) => rsx! {
                    div { class: "card",
                        div { class: "card-header",
                            div { class: "avatar small", span { class: "material-icons", "groups" } }
                            h3 { class: "title-medium", "{t(\"layout.groups\")}" }
                        }
                        ContextList { contexts: groups.clone() }
                    }
                    div { class: "card",
                        div { class: "card-header",
                            div { class: "avatar small", span { class: "material-icons", "event" } }
                            h3 { class: "title-medium", "{t(\"layout.events\")}" }
                        }
                        ContextList { contexts: events.clone() }
                    }
                },
            }
        }

        // Latest contributions the user authored, each linking to the item.
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "history_edu" } }
                h3 { class: "title-medium", "{t(\"profile.contributions\")}" }
            }
            {match &contrib_state {
                None => rsx! {
                    div { class: "card-content", crate::components::widgets::Spinner {} }
                },
                Some(items) => rsx! {
                    ContributionList { items: items.clone() }
                },
            }}
        }
    }
}

/// The profile's authored-contributions list, revealed a handful at a time via an
/// incremental "show more".
#[component]
fn ContributionList(items: Vec<model::ChildNodeFields>) -> Element {
    const STEP: usize = 5;
    let mut shown = use_signal(|| STEP);
    if items.is_empty() {
        return rsx! {
            div { class: "card-content",
                p { class: "body-medium", class: "text-muted", "{t(\"common.noContent\")}" }
            }
        };
    }
    let n = (*shown.read()).min(items.len());
    rsx! {
        div { class: "list",
            for node in items[..n].iter() {
                ContributionItem { key: "{node.id.0}", node: node.clone() }
            }
        }
        if items.len() > n {
            button {
                class: "btn btn-text",
                onclick: move |_| {
                    let s = *shown.read();
                    shown.set(s + STEP);
                },
                "{t(\"layout.showMore\")}"
            }
        }
    }
}

/// A group/event list body for the profile: links into each context, or an empty
/// hint. Shared by the Groups and Events cards. Reveals a handful at a time via
/// an incremental "show more".
#[component]
fn ContextList(contexts: Vec<model::ContextNodeFields>) -> Element {
    const STEP: usize = 5;
    let mut shown = use_signal(|| STEP);
    if contexts.is_empty() {
        return rsx! {
            div { class: "card-content",
                p { class: "body-medium", class: "text-muted", "{t(\"common.noContent\")}" }
            }
        };
    }
    let n = (*shown.read()).min(contexts.len());
    rsx! {
        div { class: "list",
            for ctx in contexts[..n].iter() {
                Link {
                    key: "{ctx.id.0}",
                    to: Route::PathPage { segments: vec![ctx.key.clone()], app: None },
                    class: "list-link",
                    super::widgets::ListItem {
                        headline: ctx.name.clone(),
                        leading: rsx! {
                            div { class: "avatar small", {icon_el(ctx.mime_id.as_deref().unwrap_or(""))} }
                        },
                    }
                }
            }
        }
        if contexts.len() > n {
            button {
                class: "btn btn-text",
                onclick: move |_| {
                    let s = *shown.read();
                    shown.set(s + STEP);
                },
                "{t(\"layout.showMore\")}"
            }
        }
    }
}

/// One authored contribution row: its mime icon, a primary line (the comment text
/// for a comment, else the node name) and its parent as context. Clicking resolves
/// the node's full path (like the home "Newest" list) and navigates to it.
#[component]
fn ContributionItem(node: model::ChildNodeFields) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let node_id = node.id.0.clone();
    let mime = node.mime_id.clone().unwrap_or_default();
    let primary = if mime == "vote/comment" {
        node.data
            .as_ref()
            .and_then(|d| d.0.get("text"))
            .and_then(|t| t.as_str())
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| t("vote.comments"))
    } else {
        node.name.clone()
    };
    let parent_name = node.parent.as_ref().map(|p| p.name.clone());

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
            div { class: "avatar small", {icon_el(&mime)} }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{primary}" }
                if let Some(pn) = parent_name {
                    div { class: "list-item-secondary", "{pn}" }
                }
            }
        }
    }
}

/// A per-user public profile viewed by user id (`/profile/:id`): the person's
/// name + avatar and the groups/events you share with them. The memberships
/// query is filtered by what the viewer may see, so it is the intersection —
/// permission-safe. Distinct from the self-only [`ProfileApp`]; user
/// representations across the app link here via [`super::loader::UserPopover`].
#[component]
pub fn UserProfile(id: String) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();

    let uid = id.clone();
    let tok = access_token.clone();
    let user_res = crate::use_data_resource!(|(uid, tok)| async move {
        graphql::query_user(tok.as_deref(), &uid).await
    });

    let uid2 = id.clone();
    let tok2 = access_token.clone();
    let memberships = crate::use_data_resource!(|(uid2, tok2)| async move {
        let mut out = Vec::new();
        for mime in ["wiki/group", "wiki/event"] {
            if let Ok(nodes) = graphql::query_contexts(tok2.as_deref(), &uid2, mime).await {
                out.extend(nodes);
            }
        }
        out
    });

    let user = user_res.read().clone().flatten();
    let name = user
        .as_ref()
        .map(|u| u.display_name.clone())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| t("common.unknown"));
    let avatar_url = user
        .as_ref()
        .map(|u| u.avatar_url.clone())
        .unwrap_or_default();
    let contexts = memberships.read().clone().unwrap_or_default();

    rsx! {
        div { class: "card",
            div { class: "profile-hero",
                div { class: "profile-hero-avatar",
                    {user_avatar(&avatar_url, icon_el("wiki/user"))}
                }
                div {
                    h3 { class: "profile-hero-name", "{name}" }
                    if !contexts.is_empty() {
                        span { class: "count-badge", "{contexts.len()} · {t(\"profile.memberships\")}" }
                    }
                }
            }
        }
        div { class: "card mt-1",
            div { class: "card-header",
                h3 { class: "title-medium", "{t(\"profile.sharedMemberships\")}" }
            }
            if contexts.is_empty() {
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "groups" }
                    }
                    p { class: "empty-state-body", "{t(\"common.noContent\")}" }
                }
            } else {
                div { class: "list",
                    for ctx in contexts.iter() {
                        Link {
                            key: "{ctx.id.0}",
                            to: Route::PathPage { segments: vec![ctx.key.clone()], app: None },
                            class: "list-link",
                            super::widgets::ListItem {
                                headline: ctx.name.clone(),
                                leading: rsx! {
                                    div { class: "avatar small", {icon_el(ctx.mime_id.as_deref().unwrap_or(""))} }
                                },
                            }
                        }
                    }
                }
            }
        }
    }
}
