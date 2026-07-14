use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::session::use_session;

use super::loader::MimeLoader;
use super::speak::{SpeakApp, SpeakMode};

/// ScreenApp — the projector/presentation view (`?app=screen`). Shows the
/// context's currently active node next to the speaker list, mirroring the
/// React ScreenApp. Live: the active relation is re-resolved on each poll.
#[component]
pub fn ScreenApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    // Live projector: subscribe to the context's `active` (what to show) and
    // `screenComments` (whether the owner opted to show comments) relations so
    // remote changes update the projected pane without a reload (React's useSubsGet).
    let refresh = use_signal(|| 0u32);
    let sub_ctx = crate::graphql::gql_escape(&context_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ relations(where: {{ parentId: {{ _eq: \"{sub_ctx}\" }}, name: {{ _in: [\"active\", \"screenComments\"] }} }}) {{ nodeId name }} }}"
        ),
        refresh,
    );
    let rev = *refresh.read();

    let active_ctx = context_id.clone();
    let active_token = access_token.clone();
    let active = crate::use_data_resource!(|(active_ctx, active_token, rev)| async move {
        let _ = rev;
        let id = graphql::active_node_id(active_token.as_deref(), &active_ctx)
            .await
            .ok()
            .flatten()?;
        graphql::query_node_by_id(active_token.as_deref(), &id)
            .await
            .ok()?
    });
    let active = active.read().clone().flatten();

    let comments_ctx = context_id.clone();
    let comments_token = session.read().access_token.clone();
    let show_comments =
        crate::use_data_resource!(|(comments_ctx, comments_token, rev)| async move {
            let _ = rev;
            graphql::screen_comments_on(comments_token.as_deref(), &comments_ctx)
                .await
                .unwrap_or(false)
        });
    let show_comments = show_comments.read().unwrap_or(false);

    rsx! {
        // Projector: a chromeless, room-distance 2-pane layout — a dominant hero
        // (the active content the room is looking at) + a supporting rail (the
        // current + on-deck speaker). Tonal, overscan-safe (see `.projector`).
        div { class: "projector",
            div { class: "projector-hero",
                match active.clone() {
                    Some(n) => rsx! { MimeLoader { key: "{n.id.0}", node: n, path: Vec::new(), projector: true } },
                    None => rsx! {
                        // EXPERIMENT: an expressive idle state instead of a bare "…".
                        div { class: "card",
                            div { class: "empty-state",
                                div { class: "empty-state-orb",
                                    span { class: "material-icons", "cast" }
                                }
                                p { class: "empty-state-body", "{crate::i18n::t(\"common.noContent\")}" }
                            }
                        }
                    },
                }
            }
            div { class: "projector-rail",
                SpeakApp { node: node.clone(), mode: SpeakMode::Screen }
                // Comments only when the owner opted in (screenComments relation).
                // Read-only on the room screen — the composer is hidden via CSS.
                if show_comments {
                    if let Some(n) = active.as_ref() {
                        div { class: "projector-comments",
                            super::comments::CommentSection {
                                node_id: n.id.0.clone(),
                                context_id: n.context_id.as_ref().map(|u| u.0.clone()),
                            }
                        }
                    }
                }
            }
        }
    }
}

/// FollowApp — a personal-device "follow the room" view (`?app=follow`). Tracks the
/// context's `active` node (the item the chair projected) live and renders it
/// INTERACTIVELY, so a member reads the motion or casts a vote as focus moves,
/// auto-updating without a reload. The device-side mirror of [`ScreenApp`], which
/// shows the same active node room-facing.
#[component]
pub fn FollowApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    let refresh = use_signal(|| 0u32);
    let sub_ctx = crate::graphql::gql_escape(&context_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ relations(where: {{ parentId: {{ _eq: \"{sub_ctx}\" }}, name: {{ _eq: \"active\" }} }}) {{ nodeId }} }}"
        ),
        refresh,
    );
    let rev = *refresh.read();

    let active = crate::use_data_resource!(|(context_id, access_token, rev)| async move {
        let _ = rev;
        let id = graphql::active_node_id(access_token.as_deref(), &context_id)
            .await
            .ok()
            .flatten()?;
        graphql::query_node_by_id(access_token.as_deref(), &id)
            .await
            .ok()?
    });
    let active = active.read().clone().flatten();

    rsx! {
        div { class: "follow-view",
            div { class: "follow-banner",
                span { class: "material-icons follow-pulse", "sensors" }
                span { "{crate::i18n::t(\"follow.live\")}" }
            }
            match active {
                Some(n) => rsx! { MimeLoader { key: "{n.id.0}", node: n, path: Vec::new() } },
                None => rsx! {
                    div { class: "card",
                        div { class: "empty-state",
                            div { class: "empty-state-orb",
                                span { class: "material-icons", "sensors_off" }
                            }
                            p { class: "empty-state-body", "{crate::i18n::t(\"follow.idle\")}" }
                        }
                    }
                },
            }
        }
    }
}
