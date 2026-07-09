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

    let active = use_resource(move || {
        let token = access_token.clone();
        let ctx = context_id.clone();
        async move {
            let id = graphql::active_node_id(token.as_deref(), &ctx)
                .await
                .ok()
                .flatten()?;
            graphql::query_node_by_id(token.as_deref(), &id)
                .await
                .ok()?
        }
    });
    let active = active.read().clone().flatten();

    rsx! {
        div { class: "grid grid-3", style: "gap: 8px;",
            // Active content (spans two columns on wide screens).
            div { style: "grid-column: span 2;",
                match active {
                    Some(n) => rsx! { MimeLoader { node: n, path: Vec::new() } },
                    None => rsx! {
                        div { class: "card",
                            div { class: "card-content",
                                p { class: "body-large", "…" }
                            }
                        }
                    },
                }
            }
            // Speaker list alongside it.
            div {
                SpeakApp { node: node.clone(), mode: SpeakMode::Screen }
            }
        }
    }
}
