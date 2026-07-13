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
                Some(t) => crate::nhost::atproto_status(&t).await,
                None => crate::nhost::AtprotoLink::default(),
            }
        }
    });

    // The user's groups + events (same query the home list uses).
    let memberships = crate::use_data_resource!(move || {
        let token = access_token.clone();
        let uid = user_id.clone();
        async move {
            let uid = uid?;
            let mut out = Vec::new();
            for mime in ["wiki/group", "wiki/event"] {
                if let Ok(nodes) = graphql::query_contexts(token.as_deref(), &uid, mime).await {
                    out.extend(nodes);
                }
            }
            Some(out)
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

    let contexts = memberships.read().clone().flatten().unwrap_or_default();
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
                div { class: "avatar small", span { class: "material-icons", "link" } }
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
                                if crate::nhost::atproto_unlink(&tok).await {
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
                            value: "{bsky_handle}",
                            oninput: move |e| bsky_handle.set(e.value()),
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
                                let url = format!(
                                    "{}/atproto/start?handle={handle}&token={token}",
                                    crate::nhost::BACKEND_URL
                                );
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

        div { class: "card",
            div { class: "card-header",
                h3 { class: "title-medium", "{t(\"profile.memberships\")}" }
            }
            if contexts.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.noContent\")}"
                    }
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
