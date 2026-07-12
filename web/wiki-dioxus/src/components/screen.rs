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

    // Live projector: subscribe to the context `active` relation so switching the
    // active node (remotely, from the admin) updates the projected pane without a
    // reload — the whole point of the screen view (React ScreenApp's useSubsGet).
    let refresh = use_signal(|| 0u32);
    let sub_ctx = context_id.clone();
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
        // Projector: a chromeless, room-distance 2-pane layout — a dominant hero
        // (the active content the room is looking at) + a supporting rail (the
        // current + on-deck speaker). Tonal, overscan-safe (see `.projector`).
        div { class: "projector",
            div { class: "projector-hero",
                match active {
                    Some(n) => rsx! { MimeLoader { node: n, path: Vec::new() } },
                    None => rsx! {
                        div { class: "card",
                            div { class: "card-content",
                                p { class: "md-display-small text-muted", "…" }
                            }
                        }
                    },
                }
            }
            div { class: "projector-rail",
                SpeakApp { node: node.clone(), mode: SpeakMode::Screen }
            }
        }
    }
}
