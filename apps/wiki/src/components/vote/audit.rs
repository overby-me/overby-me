//! A voter's check on a secret poll: that their ballot is on the board as they
//! cast it, and that the board adds up to what was announced.
//!
//! Only on a backend that keeps a ballot stub and serves a board
//! (`graphql::BALLOT_RECEIPTS`). Both checks run on this device: the stub never
//! leaves it, and the recount is the code the result was counted with.

use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::{t, t_with};
use crate::model::{BallotStanding, Recounted};
use crate::session::use_session;

#[component]
pub fn BallotAudit(poll_id: String, open: bool, show_results: bool, rev: u32) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let mut recount = use_signal(|| None::<Result<Recounted, String>>);
    let mut counting = use_signal(|| false);

    let (mine_poll, mine_token) = (poll_id.clone(), token.clone());
    let mine = crate::use_data_resource!(|(mine_poll, mine_token, rev)| async move {
        let _ = rev;
        match mine_token {
            Some(token) => graphql::my_ballots(&token, &mine_poll).await,
            None => Vec::new(),
        }
    });
    let standings = mine.read().clone().unwrap_or_default();

    let on_recount = move |_| {
        let (poll, token) = (poll_id.clone(), token.clone());
        spawn(async move {
            counting.set(true);
            recount.set(Some(graphql::recount_poll(token.as_deref(), &poll).await));
            counting.set(false);
        });
    };

    if standings.is_empty() && (open || !show_results) {
        return rsx! {};
    }
    rsx! {
        div { class: "ballot-audit mt-1",
            for (i, standing) in standings.iter().enumerate() {
                p { key: "{i}", class: "body-medium",
                    match standing {
                        BallotStanding::Counted { position } => rsx! {
                            span { class: "material-icons", "verified" }
                            " {t_with(\"vote.ballotCounted\", &[(\"position\", (position + 1).to_string().as_str())])}"
                        },
                        BallotStanding::RecordedDifferently => rsx! {
                            span { class: "material-icons", "report" }
                            " {t(\"vote.ballotDifferent\")}"
                        },
                        BallotStanding::NotOnTheBoard => rsx! {
                            span { class: "material-icons", "help_outline" }
                            " {t(\"vote.ballotMissing\")}"
                        },
                    }
                }
            }
            if !open && show_results {
                button {
                    class: "btn btn-text",
                    disabled: counting(),
                    onclick: on_recount,
                    span { class: "material-icons", "calculate" }
                    if counting() { "{t(\"vote.recounting\")}" } else { "{t(\"vote.recount\")}" }
                }
                match recount.read().clone() {
                    Some(Ok(counted)) if counted.problems.is_empty() => rsx! {
                        p { class: "body-medium recount-ok",
                            "{t_with(\"vote.recountMatches\", &[(\"count\", counted.ballots.to_string().as_str())])}"
                        }
                    },
                    Some(Ok(counted)) => rsx! {
                        p { class: "body-medium recount-disputed", "{t(\"vote.recountDisputed\")}" }
                        ul {
                            for (i, problem) in counted.problems.iter().enumerate() {
                                li { key: "{i}", class: "body-small", "{problem}" }
                            }
                        }
                    },
                    Some(Err(why)) => rsx! { p { class: "body-small text-muted", "{why}" } },
                    None => rsx! {},
                }
            }
        }
    }
}
