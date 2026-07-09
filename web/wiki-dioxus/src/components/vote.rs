use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::{t, t_with};
use crate::session::use_session;
use crate::snackbar::show_snackbar;

use super::content::ContentApp;
use super::loader::{mime_icon, visible_sorted};
use super::ui::checkbox::Checkbox;
use super::ui::radio_group::{RadioGroup, RadioItem};
use dioxus_primitives::checkbox::CheckboxState;

/// VoteApp — the context-level vote screen (`?app=vote`). Resolves the context's
/// "active" relation to the currently open node; when that is a poll it shows
/// the ballot (via PollApp), mirroring the React VoteApp's `get("active")`.
#[component]
pub fn VoteApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let access_token = session.read().access_token.clone();
    let context_id = node.context_id.clone().map(|c| c.0).unwrap_or(node.id.0);

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

    let no_vote = rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "{mime_icon(\"vote/poll\")}" }
                h3 { class: "title-medium", "{t(\"mime.vote\")}" }
            }
            div { class: "card-content",
                p { class: "body-large",
                    if is_auth { "{t(\"vote.noVoteNow\")}" } else { "{t(\"vote.noVotingRight\")}" }
                }
            }
        }
    };

    let state = active.read().clone();
    match state {
        Some(Some(active)) if active.mime_id.as_deref() == Some("vote/poll") => {
            rsx! { PollApp { node: active } }
        }
        Some(_) => no_vote,
        None => rsx! {
            div { class: "spinner-overlay",
                div { class: "spinner" }
            }
        },
    }
}

/// PolicyApp — document with comments, changes, and polls
#[component]
pub fn PolicyApp(node: NodeWithChildren) -> Element {
    let children = visible_sorted(&node.children);
    let children = &children;

    let polls: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/poll"))
        .collect();

    let amendments: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/change"))
        .collect();

    let comments: Vec<_> = children
        .iter()
        .filter(|c| {
            !matches!(
                c.mime_id.as_deref(),
                Some("vote/poll") | Some("vote/change")
            )
        })
        .collect();

    rsx! {
        // Main content
        ContentApp { node: node.clone() }

        // Amendments
        if !amendments.is_empty() {
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar", "\u{1F4DD}" }
                    h3 { class: "title-medium", "{t(\"vote.amendments\")}" }
                }
                div { class: "list",
                    for item in amendments.iter() {
                        div { class: "list-item", key: "{item.id.0}",
                            div { class: "avatar small", "{mime_icon(\"vote/change\")}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{item.name}" }
                            }
                        }
                    }
                }
            }
        }

        // Polls
        if !polls.is_empty() {
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar", "{mime_icon(\"vote/poll\")}" }
                    h3 { class: "title-medium", "{t(\"mime.vote\")}" }
                }
                div { class: "list",
                    for poll in polls.iter() {
                        div { class: "list-item", key: "{poll.id.0}",
                            div { class: "avatar small", "{mime_icon(\"vote/poll\")}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{poll.name}" }
                            }
                        }
                    }
                }
            }
        }

        // Other children (comments, questions)
        if !comments.is_empty() {
            div { class: "card mt-1",
                div { class: "list",
                    for child in comments.iter() {
                        div { class: "list-item", key: "{child.id.0}",
                            div { class: "avatar small",
                                "{mime_icon(child.mime_id.as_deref().unwrap_or(\"\"))}"
                            }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{child.name}" }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// The options / min / max a poll's `data` describes.
struct PollConfig {
    options: Vec<String>,
    min_vote: usize,
    max_vote: usize,
}

fn poll_config(node: &NodeWithChildren) -> PollConfig {
    let data = node.data.as_ref().map(|d| &d.0);
    let options = data
        .and_then(|d| d.get("options"))
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();
    let get_num = |k: &str, default: usize| {
        data.and_then(|d| d.get(k))
            .and_then(|v| v.as_u64())
            .and_then(|n| usize::try_from(n).ok())
            .unwrap_or(default)
    };
    PollConfig {
        options,
        min_vote: get_num("minVote", 1),
        max_vote: get_num("maxVote", 1),
    }
}

/// PollApp — cast a vote on an open poll, or show that you have voted / the poll
/// is closed. Mirrors the React VoteApp ballot: radio for single-choice, else
/// checkboxes; the last option ("Blank") can only be chosen alone.
#[component]
pub fn PollApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let PollConfig {
        options,
        min_vote,
        max_vote,
    } = poll_config(&node);
    let name = node.name.clone();
    let poll_id = node.id.0.clone();
    let context_id = node.context_id.clone().map(|c| c.0);
    let open = node.mutable;
    let single = max_vote == 1 && min_vote == 1;

    let mut selected = use_signal(|| vec![false; options.len()]);
    let mut error = use_signal(String::new);
    let mut refresh = use_signal(|| 0u32);
    // Randomise the ballot order once per mount (#27); Blank stays last.
    let order = use_hook(|| ballot_order(options.len(), js_sys::Math::random));

    // Live results: any vote cast on this poll re-runs the tally / voted checks.
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{poll_id}\" }}, mimeId: {{ _eq: \"vote/vote\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    // Whether the current user has already voted (own votes are visible to them).
    let already_voted = use_resource({
        let token = session.read().access_token.clone();
        let poll = poll_id.clone();
        let uid = user_id.clone();
        move || {
            let token = token.clone();
            let poll = poll.clone();
            let uid = uid.clone();
            let _ = refresh.read();
            async move {
                let Some(uid) = uid else { return false };
                graphql::count_user_votes(token.as_deref(), &poll, &uid)
                    .await
                    .map(|n| n > 0)
                    .unwrap_or(false)
            }
        }
    });
    let voted = already_voted.read().unwrap_or(false);

    // Tally of the votes visible to this user (all of them for the poll owner /
    // an admin; just their own otherwise). Counts per option index.
    let tally = use_resource({
        let token = session.read().access_token.clone();
        let poll = poll_id.clone();
        let n = options.len();
        move || {
            let token = token.clone();
            let poll = poll.clone();
            let _ = refresh.read();
            async move {
                let votes = graphql::query_poll_votes(token.as_deref(), &poll)
                    .await
                    .unwrap_or_default();
                let mut counts = vec![0usize; n];
                for vote in &votes {
                    for &i in vote {
                        if let Some(c) = counts.get_mut(i) {
                            *c += 1;
                        }
                    }
                }
                (counts, votes.len())
            }
        }
    });
    let (counts, total_votes) = tally.read().clone().unwrap_or((vec![], 0));

    let opts = options.clone();

    let submit = {
        let token = session.read().access_token.clone();
        let poll = poll_id.clone();
        let ctx = context_id.clone();
        let opts = options.clone();
        let min = min_vote;
        let max = max_vote;
        move |_| {
            let cur = selected.read().clone();
            let chosen: Vec<usize> = cur
                .iter()
                .enumerate()
                .filter_map(|(i, v)| v.then_some(i))
                .collect();
            // "Blank" (last option) can only be selected alone.
            let blank = opts.len().saturating_sub(1);
            if chosen.len() > 1 && chosen.contains(&blank) {
                error.set(t("vote.blankOnlyAlone"));
                return;
            }
            let blank_alone = chosen.len() == 1 && chosen[0] == blank;
            if !blank_alone && chosen.len() < min {
                error.set(t_with("vote.selectAtLeast", &[("count", &min.to_string())]));
                return;
            }
            if chosen.len() > max {
                error.set(t_with("vote.selectAtMost", &[("count", &max.to_string())]));
                return;
            }
            let token = token.clone();
            let poll = poll.clone();
            let ctx = ctx.clone();
            spawn(async move {
                let suffix = format!("{:.0}", now_ms());
                match graphql::cast_vote(token.as_deref(), &poll, ctx.as_deref(), &chosen, &suffix)
                    .await
                {
                    Ok(true) => {
                        show_snackbar(&t("vote.hasVoted"));
                        refresh += 1;
                    }
                    _ => error.set(t("error.somethingWentWrong")),
                }
            });
        }
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "{mime_icon(\"vote/poll\")}" }
                div {
                    h3 { class: "title-medium", "{name}" }
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        if !open { "{t(\"vote.noVoteNow\")}" } else if voted { "{t(\"vote.hasVoted\")}" } else { "{t(\"poll.managePoll\")}" }
                    }
                }
            }

            div { class: "card-content",
                if options.is_empty() {
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.noContent\")}"
                    }
                } else if is_auth && open && !voted {
                    // The ballot: single-choice uses an accessible RadioGroup,
                    // multi-choice uses Checkbox per option.
                    if single {
                        {
                            let current = selected.read().iter().position(|&b| b).map(|i| i.to_string());
                            let len = opts.len();
                            rsx! {
                                RadioGroup {
                                    value: current,
                                    on_value_change: move |v: String| {
                                        if let Ok(idx) = v.parse::<usize>() {
                                            let mut cur = vec![false; len];
                                            if idx < cur.len() {
                                                cur[idx] = true;
                                            }
                                            selected.set(cur);
                                            error.set(String::new());
                                        }
                                    },
                                    for (dp , ri) in order.iter().enumerate() {
                                        {
                                            let ri = *ri;
                                            let option = opts[ri].clone();
                                            rsx! {
                                                div { class: "list-item", key: "{ri}", style: "gap: 8px;",
                                                    RadioItem { value: "{ri}", index: dp }
                                                    div { class: "list-item-text",
                                                        div { class: "list-item-primary", "{option}" }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    } else {
                        div { class: "list",
                            for ri in order.iter() {
                                {
                                    let ri = *ri;
                                    let option = opts[ri].clone();
                                    rsx! {
                                        div { class: "list-item", key: "{ri}", style: "gap: 8px;",
                                            Checkbox {
                                                checked: Some(if selected.read().get(ri).copied().unwrap_or(false) {
                                                    CheckboxState::Checked
                                                } else {
                                                    CheckboxState::Unchecked
                                                }),
                                                on_checked_change: move |_| apply_toggle(selected, error, ri, false),
                                            }
                                            div { class: "list-item-text",
                                                div { class: "list-item-primary", "{option}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    if !error.read().is_empty() {
                        p { class: "body-medium", style: "color: var(--md-error, #b3261e);", "{error}" }
                    }
                    button {
                        class: "btn btn-primary mt-1",
                        onclick: submit,
                        "\u{1F5F3}\u{FE0F} {t(\"vote.castVote\")}"
                    }
                } else {
                    // Read-only option list with per-option tallies (closed poll,
                    // already voted, or logged out).
                    div { class: "list",
                        for (i , option) in opts.iter().enumerate() {
                            div { class: "list-item", key: "{i}",
                                div { class: "avatar small", "{i + 1}" }
                                div { class: "list-item-text",
                                    div { class: "list-item-primary", "{option}" }
                                    {
                                        let count = counts.get(i).copied().unwrap_or(0);
                                        let pct = (count * 100).checked_div(total_votes).unwrap_or(0);
                                        let fraction = count as f64 / total_votes.max(1) as f64;
                                        rsx! {
                                            super::widgets::Bar { fraction }
                                            div { class: "list-item-secondary", "{count} ({pct}%)" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    p { class: "body-medium mt-1",
                        if voted { "{t(\"vote.hasVoted\")} · " }
                        "{t(\"vote.voteCount\")}: {total_votes}"
                    }
                }
            }
        }
    }
}

fn now_ms() -> f64 {
    web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| p.now())
        .unwrap_or(0.0)
}

/// Toggle option `idx` in a poll ballot. Single-choice polls (radio) clear the
/// other options; multi-choice (checkbox) flip just this one.
fn apply_toggle(
    mut selected: Signal<Vec<bool>>,
    mut error: Signal<String>,
    idx: usize,
    single: bool,
) {
    let mut cur = selected.read().clone();
    if idx >= cur.len() {
        return;
    }
    if single {
        cur = vec![false; cur.len()];
        cur[idx] = true;
    } else {
        cur[idx] = !cur[idx];
    }
    selected.set(cur);
    error.set(String::new());
}

/// A randomised display order for a ballot's `n` options (#27), to remove
/// first-listed bias. The final option ("Blank") is kept last (it can only be
/// chosen alone); the rest are Fisher-Yates shuffled with `rand` in `[0, 1)`.
/// Returns real option indices in display order, so callers still address
/// `selected`/`counts` by the returned index.
fn ballot_order(n: usize, mut rand: impl FnMut() -> f64) -> Vec<usize> {
    if n <= 2 {
        return (0..n).collect();
    }
    let mut order: Vec<usize> = (0..n - 1).collect();
    for i in (1..order.len()).rev() {
        let j = ((rand() * (i as f64 + 1.0)).floor() as usize).min(i);
        order.swap(i, j);
    }
    order.push(n - 1);
    order
}

#[cfg(test)]
mod tests {
    use super::ballot_order;

    #[test]
    fn ballot_order_keeps_blank_last_and_is_a_permutation() {
        // Deterministic "random" that always picks index 0 on each step.
        let order = ballot_order(5, || 0.0);
        assert_eq!(order.len(), 5);
        assert_eq!(*order.last().unwrap(), 4, "Blank (last) stays last");
        let mut sorted = order.clone();
        sorted.sort();
        assert_eq!(sorted, vec![0, 1, 2, 3, 4], "every option appears once");

        // Small ballots are returned unchanged.
        assert_eq!(ballot_order(2, || 0.5), vec![0, 1]);
        assert_eq!(ballot_order(0, || 0.5), Vec::<usize>::new());
    }
}
