use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use super::loader::icon_el;

/// ProfileApp — the signed-in user's profile (#78): who they are plus the groups
/// and events they belong to. Reachable via `?app=profile`.
#[component]
pub fn ProfileApp() -> Element {
    let session = use_session();
    let user = session.read().user.clone();
    let access_token = session.read().access_token.clone();
    let user_id = user.as_ref().map(|u| u.id.clone());

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
                div { class: "avatar", {icon_el("app/profile")} }
                div {
                    h3 { class: "title-medium", "{user.display_name}" }
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"profile.signedInAs\")} {user.email}"
                    }
                }
            }
            div { class: "card-content",
                p {
                    class: "body-small",
                    style: "color: var(--md-on-surface-variant);",
                    "{t(\"profile.userId\")}: {user.id}"
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
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                div { class: "list",
                    for ctx in contexts.iter() {
                        Link {
                            key: "{ctx.id.0}",
                            to: Route::PathPage { segments: vec![ctx.key.clone()], app: None },
                            class: "folder-item",
                            div { class: "avatar small", {icon_el(ctx.mime_id.as_deref().unwrap_or(""))} }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{ctx.name}" }
                            }
                        }
                    }
                }
            }
        }
    }
}
