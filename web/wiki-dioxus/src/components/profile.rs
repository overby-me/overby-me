use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, user_avatar};

/// UserApp — a public profile for a `wiki/user` node: the person's name plus the
/// groups and events they belong to (React UserApp). Reached by navigating to a
/// user node, unlike self-only [`ProfileApp`].
#[component]
pub fn UserApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let uid = node.id.0.clone();
    let name = node.name.clone();

    // The groups + events this user belongs to (owner or accepted member).
    let memberships = crate::use_data_resource!(|(uid, access_token)| async move {
        let mut out = Vec::new();
        for mime in ["wiki/group", "wiki/event"] {
            if let Ok(nodes) = graphql::query_contexts(access_token.as_deref(), &uid, mime).await {
                out.extend(nodes);
            }
        }
        out
    });
    let contexts = memberships.read().clone().unwrap_or_default();

    rsx! {
        // EXPERIMENT (profile hero): a bold identity header with a large tonal
        // avatar and a membership-count chip.
        div { class: "card",
            div { class: "profile-hero",
                div { class: "profile-hero-avatar", {icon_el("wiki/user")} }
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
                h3 { class: "title-medium", "{t(\"profile.memberships\")}" }
            }
            if contexts.is_empty() {
                // EXPERIMENT: orb empty state for a user with no memberships.
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

        // Link a Bluesky (atproto) account: hand off to the backend OAuth flow with
        // the handle + current NHost access token; it redirects back to APP_ORIGIN
        // with ?linked=success|error (surfaced in a snackbar by App on load).
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "link" } }
                h3 { class: "title-medium", "{t(\"profile.linkBluesky\")}" }
            }
            div { class: "card-content",
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
