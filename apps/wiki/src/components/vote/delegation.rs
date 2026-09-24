//! Giving one's vote to another member, and taking it back.
//!
//! Shown on the vote screen to someone who holds a vote, on a backend that has
//! delegation (`graphql::DELEGATION`). It stands for polls opened from then on:
//! a poll freezes who casts what as it opens.

use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::{t, t_with};
use crate::model::MemberPageFilter;
use crate::session::use_session;

/// How many members a search offers. A name narrows it faster than a scroll.
const OFFERED: usize = 6;

#[component]
pub fn DelegationCard(context_id: String) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let me = session.read().identity();
    let mut refresh = use_signal(|| 0u32);
    let mut search = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let rev = refresh();
    let (st_ctx, st_token, st_me) = (context_id.clone(), token.clone(), me.clone());
    let standing = crate::use_data_resource!(|(st_ctx, st_token, st_me, rev)| async move {
        let _ = rev;
        graphql::query_delegations(st_token.as_deref(), &st_ctx, &st_me)
            .await
            .ok()
    });

    let (fd_ctx, fd_token, fd_me) = (context_id.clone(), token.clone(), me.clone());
    let found = use_resource(move || {
        let (ctx, token, me) = (fd_ctx.clone(), fd_token.clone(), fd_me.clone());
        let wanted = search.read().trim().to_string();
        async move {
            if wanted.len() < 2 {
                return Vec::new();
            }
            // Wait out the typing: each keystroke restarts this.
            gloo_timers::future::TimeoutFuture::new(220).await;
            let voters = MemberPageFilter {
                active: Some(true),
                accepted: Some(true),
                search: wanted,
                ..Default::default()
            };
            graphql::query_members_page(token.as_deref(), &ctx, &voters, OFFERED, 0)
                .await
                .map(|(rows, _)| rows)
                .unwrap_or_default()
                .into_iter()
                // Someone who has signed in, and not oneself.
                .filter(|m| {
                    m.node_id
                        .as_ref()
                        .is_some_and(|id| id.0.starts_with("did:") && id.0 != me)
                })
                .collect::<Vec<_>>()
        }
    });

    let give = move |to: Option<String>| {
        let (ctx, token) = (context_id.clone(), token.clone());
        spawn(async move {
            busy.set(true);
            match graphql::set_delegation(token.as_deref(), &ctx, to.as_deref()).await {
                Ok(()) => {
                    search.set(String::new());
                    refresh += 1;
                }
                Err(why) => crate::snackbar::show_snackbar(&why),
            }
            busy.set(false);
        });
    };
    let take_back = give.clone();

    let Some(Some(standing)) = standing.read().clone() else {
        return rsx! {};
    };
    let carried: Vec<String> = standing
        .received_from
        .iter()
        .map(|person| person.display_name.clone())
        .collect();

    rsx! {
        div { class: "card app-card delegation-card",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "how_to_vote" } }
                h3 { class: "title-medium", "{t(\"vote.yourVote\")}" }
            }
            div { class: "card-content",
                if let Some(to) = &standing.given_to {
                    p { class: "body-medium mb-1",
                        "{t_with(\"vote.givenTo\", &[(\"name\", to.display_name.as_str())])}"
                    }
                    button {
                        class: "btn btn-secondary",
                        disabled: busy(),
                        onclick: move |_| take_back(None),
                        span { class: "material-icons", "undo" }
                        " {t(\"vote.takeBack\")}"
                    }
                } else {
                    p { class: "body-medium mb-1", "{t(\"vote.youCastIt\")}" }
                    if !carried.is_empty() {
                        p { class: "body-medium mb-1",
                            "{t_with(\"vote.youCarry\", &[(\"names\", carried.join(\", \").as_str())])}"
                        }
                    }
                    div { class: "text-field",
                        label { r#for: "delegate-search", "{t(\"vote.giveTo\")}" }
                        input {
                            id: "delegate-search",
                            r#type: "text",
                            autocomplete: "off",
                            placeholder: "{t(\"vote.searchMember\")}",
                            value: "{search}",
                            oninput: move |evt| search.set(evt.value()),
                        }
                    }
                    for member in found.read().clone().unwrap_or_default() {
                        button {
                            key: "{member.id.0}",
                            class: "list-item state-layer delegate-option",
                            disabled: busy(),
                            onclick: {
                                let give = give.clone();
                                let to = member.node_id.as_ref().map(|id| id.0.clone());
                                move |_| give(to.clone())
                            },
                            span { class: "material-icons", "person" }
                            span { class: "list-item-text", "{member.label()}" }
                        }
                    }
                }
                p { class: "body-small text-muted mt-1", "{t(\"vote.delegationNote\")}" }
            }
        }
    }
}
