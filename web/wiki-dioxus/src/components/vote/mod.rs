//! Voting screens. The context-level dispatcher `VoteApp` lives here; the
//! per-kind sub-apps are split by seam into `policy`, `position`, and `poll`
//! (which also carries the ballot mechanics).
use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::session::use_session;

mod policy;
mod poll;
mod position;

pub use policy::PolicyApp;
pub use poll::PollApp;
pub use position::PositionApp;

// Re-exported so the sub-apps reach each other's buttons/badge through
// `use super::*` (PolicyApp/PositionApp render StartPollButton, PollVoteBadge,
// AddChangeButton).
use poll::*;
use position::*;

/// VoteApp — the context-level vote screen (`?app=vote`). Resolves the context's
/// "active" relation to the currently open node; when that is a poll it shows
/// the ballot (via PollApp), mirroring the React VoteApp's `get("active")`.
#[component]
pub fn VoteApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let access_token = session.read().access_token.clone();
    let context_id = node.context_id.clone().map(|c| c.0).unwrap_or(node.id.0);

    // Voting rights: whether the user is an active member of this context (the
    // port's approximation of React's canVote), for the rights card.
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let cv_ctx = context_id.clone();
    let cv_token = access_token.clone();
    let cv_user = user_id.clone();
    let can_vote_res = crate::use_data_resource!(|(cv_ctx, cv_token, cv_user)| async move {
        match cv_user {
            Some(uid) => graphql::is_active_member(cv_token.as_deref(), &cv_ctx, &uid).await,
            None => false,
        }
    });
    let can_vote = (*can_vote_res.read()).unwrap_or(false);

    // Live-update when a poll opens/closes: subscribe to the context `active`
    // relation so a freshly-opened ballot appears without a reload.
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

    // DESIGN: an expressive empty state (floating ballot orb) instead of a dull
    // text card when there is no active vote.
    let no_vote = rsx! {
        div { class: "card",
            div { class: "empty-state",
                div { class: "empty-state-orb",
                    span { class: "material-icons", "how_to_vote" }
                }
                h3 { class: "empty-state-title", "{t(\"mime.vote\")}" }
                p { class: "empty-state-body",
                    if is_auth { "{t(\"vote.noVoteNow\")}" } else { "{t(\"vote.noVotingRight\")}" }
                }
            }
        }
    };

    let state = active.read().clone();

    // Notify when a poll opens — the transition from "no active vote" to an active
    // vote/poll — for members who may vote. The narrow window to cast a ballot is
    // the highest-stakes assembly event, and the subscription above already fires.
    let active_poll_id = match &state {
        Some(Some(a)) if a.mime_id.as_deref() == Some("vote/poll") => Some(a.id.0.clone()),
        _ => None,
    };
    let mut seen_poll = use_signal(|| None::<Option<String>>);
    {
        let apid = active_poll_id.clone();
        use_effect(use_reactive!(|(apid, can_vote)| {
            let previous = seen_poll.peek().clone();
            match previous {
                // First render: remember the current poll without notifying.
                None => seen_poll.set(Some(apid)),
                Some(prev) => {
                    if can_vote && apid.is_some() && apid != prev {
                        crate::pwa::notify(&t("vote.pollOpenTitle"), &t("vote.pollOpenBody"));
                    }
                    seen_poll.set(Some(apid));
                }
            }
        }));
    }

    let content = match state {
        Some(Some(active)) if active.mime_id.as_deref() == Some("vote/poll") => {
            rsx! { PollApp { node: active } }
        }
        Some(_) => no_vote,
        // Wrap the loading spinner in the same card the empty/poll states use, so
        // the app does not visibly jump from a bare overlay to a card on load.
        None => rsx! {
            div { class: "card",
                div { class: "spinner-overlay",
                    div { class: "spinner" }
                }
            }
        },
    };

    rsx! {
        // Voting-rights indicator (React VoteApp's canVote status) — a tonal status
        // banner rather than a plain card (DESIGN).
        if is_auth {
            div {
                class: if can_vote { "vote-rights-banner can-vote" } else { "vote-rights-banner cannot-vote" },
                span { class: "material-icons",
                    if can_vote { "how_to_reg" } else { "do_not_disturb" }
                }
                span { class: "body-medium",
                    if can_vote { "{t(\"vote.hasVotingRight\")}" } else { "{t(\"vote.noVotingRight\")}" }
                }
            }
        }
        {content}
    }
}
