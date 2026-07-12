use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren, PollSummaryFields};
use crate::i18n::t;
use crate::session::use_session;

use super::loader::icon_el;

/// AdminApp — the results overview (`?app=admin`): every poll in the context
/// with its per-option tallies. React shipped this as a stubbed data grid; here
/// it is a live table.
#[component]
pub fn AdminApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    let polls = crate::use_data_resource!(|(context_id, access_token)| async move {
        graphql::query_context_polls(access_token.as_deref(), &context_id)
            .await
            .unwrap_or_default()
    });
    let polls = polls.read().clone().unwrap_or_default();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("vote/poll")} }
                h3 { class: "title-medium", "{t(\"mime.vote\")}" }
            }
            if polls.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                super::widgets::DataTable {
                    columns: vec![t("admin.poll"), t("admin.results"), t("admin.votes")],
                    for poll in polls.iter() {
                        AdminPollRow { key: "{poll.id.0}", poll: poll.clone() }
                    }
                }
            }
        }
    }
}

/// One poll's row: its name and each option's vote count (live).
#[component]
fn AdminPollRow(poll: PollSummaryFields) -> Element {
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

    rsx! {
        tr {
            td {
                div { class: "list-item-primary", "{poll.name}" }
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
            }
        }
    }
}
