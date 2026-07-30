use crate::model;
use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::{t, t_with};
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

use crate::components::loader::icon_el;
use crate::components::ui::checkbox::Checkbox;
use crate::components::ui::radio_group::{RadioGroup, RadioItem};
use crate::components::ui::switch::Switch;
use dioxus_primitives::checkbox::CheckboxState;

/// A small vote-count badge for a poll row: the number of `vote/vote` children
/// the viewer can see, fetched via the nodes aggregate.
#[component]
pub(super) fn PollVoteBadge(poll_id: String) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let count = crate::use_data_resource!(|(poll_id, token)| async move {
        graphql::poll_vote_count(token.as_deref(), &poll_id)
            .await
            .unwrap_or(0)
    });
    let n = (*count.read()).unwrap_or(0);
    rsx! {
        if n > 0 {
            span { class: "count-badge", title: "{t(\"vote.voteCount\")}", "{n}" }
        }
    }
}

/// Owner action on a poll row: delete the poll (behind a confirm), restoring the
/// old wiki's PollList delete so a stray or mistaken poll can be removed.
#[component]
pub(super) fn DeletePollButton(poll_id: String) -> Element {
    let session = use_session();
    let mut confirm = use_signal(|| false);
    let mut busy = use_signal(|| false);
    rsx! {
        button {
            class: "btn-icon",
            aria_label: t("common.delete"),
            title: t("common.delete"),
            // Sibling of the row Link, but stop propagation so a click never also
            // navigates into the poll.
            onclick: move |e: Event<MouseData>| {
                e.stop_propagation();
                confirm.set(true);
            },
            span { class: "material-icons", "delete" }
        }
        super::super::widgets::Dialog {
            open: confirm(),
            on_dismiss: move |_| confirm.set(false),
            headline: t("content.confirmDeleteBin"),
            icon: "delete".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| confirm.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: busy(),
                    onclick: {
                        let poll_id = poll_id.clone();
                        move |_| {
                            let token = session.read().access_token.clone();
                            let actor = session.read().user.as_ref().map(|u| u.id.clone());
                            let poll_id = poll_id.clone();
                            busy.set(true);
                            spawn(async move {
                                // The ballots cast on this poll are its children, so
                                // they go with it rather than lingering unreachable —
                                // and to the bin, so a poll deleted mid-meeting comes
                                // back with the votes already cast on it.
                                match graphql::bin_node(token.as_deref(), &poll_id, None, actor.as_deref()).await {
                                    Ok(_) => {
                                        crate::session::bump_data_version();
                                        confirm.set(false);
                                    }
                                    _ => show_snackbar(&t("error.somethingWentWrong")),
                                }
                                busy.set(false);
                            });
                        }
                    },
                    if busy() {
                        div { class: "spinner spinner-xs" }
                    }
                    "{t(\"common.delete\")}"
                }
            },
            p { class: "body-medium text-muted", "{t(\"content.deleteRecoverableTree\")}" }
        }
    }
}

/// The options / min / max a poll's `data` describes.
struct PollConfig {
    options: Vec<String>,
    min_vote: usize,
    max_vote: usize,
}

fn poll_config(data: Option<&serde_json::Value>) -> PollConfig {
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
pub fn PollApp(node: NodeWithChildren, #[props(default)] projector: bool) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let PollConfig {
        options,
        min_vote,
        max_vote,
    } = poll_config(node.data.as_ref().map(|d| &d.0));
    let name = node.name.clone();
    let poll_id = node.id.0.clone();
    let context_id = node.context_id.clone().map(|c| c.0);
    let open = node.mutable;
    let single = max_vote == 1 && min_vote == 1;
    // hideResult (`data.hidden`): a hide-result poll reveals tallies only to the
    // context owner; other viewers see the options without any counts.
    let poll_hidden = node
        .data
        .as_ref()
        .and_then(|d| d.0.get("hidden"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let show_results = !poll_hidden || node.is_context_owner.unwrap_or(false);
    // Secret ballot (`data.secret`): casts route through the backend so the vote
    // node carries no owner_id, and the has-voted check comes from the backend.
    let poll_secret = node
        .data
        .as_ref()
        .and_then(|d| d.0.get("secret"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);

    let options_len = options.len();
    let mut selected = use_signal(|| vec![false; options_len]);
    let mut error = use_signal(String::new);
    let mut refresh = use_signal(|| 0u32);
    // Optimistic cast: the chosen option indices, shown as voted + counted at once
    // and dropped once the refetch confirms (`voted` flips true) or on error.
    let mut cast_pending = use_signal(|| None::<Vec<usize>>);
    // In-flight guard so a rapid double-click cannot fire two casts (the second
    // would fail the one-vote uniqueness check).
    let mut casting = use_signal(|| false);
    // Optimistic close: flip the ballot to "results" at once when the owner stops
    // the poll; reverted on error. The refetch confirms mutable=false.
    let mut closed_opt = use_signal(|| false);
    // Randomise the ballot order (#27; Blank stays last). Reactive on the poll id
    // and option count — NOT a one-shot `use_hook` — because this component is
    // reused across sibling navigations without remounting: a stale order carries
    // the previous poll's indices, and against a SHORTER ballot `opts[ri]` below
    // would be out of bounds (a panic). A fresh poll re-shuffles for its own set.
    let order_memo = use_memo(use_reactive!(|(options_len)| ballot_order(
        options_len,
        js_sys::Math::random
    )));
    let order: Vec<usize> = order_memo.read().clone();
    // The selection vector must likewise track the current ballot: reset to a
    // correctly sized, cleared set when the poll (or its option count) changes,
    // so a navigation to a different poll never carries stale checks or length.
    let poll_id_dep = poll_id.clone();
    use_effect(use_reactive!(|(poll_id_dep, options_len)| {
        let _ = &poll_id_dep;
        selected.set(vec![false; options_len]);
    }));

    // Live results: any vote cast on this poll re-runs the tally / voted checks.
    let sub_poll = crate::graphql::gql_escape(&poll_id);
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{sub_poll}\" }}, mimeId: {{ _eq: \"vote/vote\" }} }}) {{ id }} }}"
        ),
        refresh,
    );
    // React to the poll being opened/closed on its own counter — selecting
    // `mutable` so the live query fires on that update — so a member's ballot hides
    // promptly when the chair closes the poll, without refetching on every vote.
    let poll_refresh = use_signal(|| 0u32);
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ id: {{ _eq: \"{sub_poll}\" }} }}) {{ mutable }} }}"
        ),
        poll_refresh,
    );

    // Live results depend on the poll (node) id and the refresh counter; use
    // use_reactive so they re-run when navigating to a different poll, not only
    // via a keyed remount (unreliable in the web renderer).
    let rev = *refresh.read();
    let n_opts = options.len();

    // Whether the current user has already voted (own votes are visible to them).
    let av_poll = poll_id.clone();
    let av_token = session.read().access_token.clone();
    let av_user = user_id.clone();
    let av_secret = poll_secret;
    let already_voted =
        crate::use_data_resource!(|(av_poll, av_token, av_user, av_secret, rev)| async move {
            let _ = rev;
            if av_secret {
                // Anonymous votes have no owner_id, so ask the backend's has-voted marker.
                match &av_token {
                    Some(t) => crate::backend_api::vote_status(t, &av_poll).await,
                    None => false,
                }
            } else {
                let Some(uid) = av_user else { return false };
                graphql::count_user_votes(av_token.as_deref(), &av_poll, &uid)
                    .await
                    .map(|n| n > 0)
                    .unwrap_or(false)
            }
        });
    let voted = already_voted.read().unwrap_or(false);

    // Tally of the votes visible to this user (all of them for the poll owner /
    // an admin; just their own otherwise). Counts per option index.
    let ty_poll = poll_id.clone();
    let ty_token = session.read().access_token.clone();
    let tally = crate::use_data_resource!(|(ty_poll, ty_token, n_opts, rev)| async move {
        let _ = rev;
        let votes = graphql::query_poll_votes(ty_token.as_deref(), &ty_poll)
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
    let (counts, total_votes) = tally.read().clone().unwrap_or((vec![], 0));
    // Fold the optimistic cast into the tally + voted state until the refetch
    // reflects it (`voted` true), then drop the optimistic add so it is not counted
    // twice. On error `cast_pending` is cleared, snapping the ballot back.
    let show_opt = cast_pending.read().is_some() && !voted;
    let counts = if show_opt {
        let mut c = counts;
        if let Some(chosen) = cast_pending.read().as_ref() {
            for &i in chosen {
                if let Some(x) = c.get_mut(i) {
                    *x += 1;
                }
            }
        }
        c
    } else {
        counts
    };
    let total_votes = if show_opt {
        total_votes + 1
    } else {
        total_votes
    };
    let voted = voted || cast_pending.read().is_some();
    // Eligible voters = active members of the poll's context, for the turnout /
    // quorum line the room reads off the projector.
    let el_ctx = context_id.clone();
    let el_token = session.read().access_token.clone();
    let eligible = crate::use_data_resource!(|(el_ctx, el_token, rev)| async move {
        let _ = rev;
        match el_ctx {
            Some(ctx) => graphql::count_active_members(el_token.as_deref(), &ctx).await,
            None => 0,
        }
    });
    let eligible_count = (*eligible.read()).unwrap_or(0);
    let turnout_pct = (total_votes * 100).checked_div(eligible_count).unwrap_or(0);

    // Live poll-open state, so the ballot disappears when the chair closes the
    // poll (shadows the prop value; the server-side gate for late votes is separate).
    let po_poll = poll_id.clone();
    let po_token = session.read().access_token.clone();
    let po_rev = *poll_refresh.read();
    let poll_open_res = crate::use_data_resource!(|(po_poll, po_token, po_rev)| async move {
        let _ = po_rev;
        graphql::query_node_by_id(po_token.as_deref(), &po_poll)
            .await
            .ok()
            .flatten()
            .map(|n| n.mutable)
    });
    let open = (*poll_open_res.read()).flatten().unwrap_or(open) && !closed_opt();
    // The trailing option is always the "Blank" abstention (see StartPollButton /
    // ballot_order): it is shown as a distinct muted row and excluded from the
    // winner, and the For/Imod split is computed on the non-blank cast votes.
    let blank_idx = counts.len().saturating_sub(1);
    let has_blank = counts.len() > 1;
    let is_abstention = move |i: usize| i == blank_idx && has_blank;
    let max_count = counts
        .iter()
        .enumerate()
        .filter(|(i, _)| !is_abstention(*i))
        .map(|(_, c)| *c)
        .max()
        .unwrap_or(0);
    // Ballots that expressed a preference (excludes blanks), the base for the
    // decisive percentages on a single-choice motion.
    let cast_votes: usize = counts
        .iter()
        .enumerate()
        .filter(|(i, _)| !is_abstention(*i))
        .map(|(_, c)| *c)
        .sum();
    // Multi-select ballots contribute several selections, so "share of ballots"
    // (count / ballots) is the honest base; single-choice divides by cast votes.
    let multi_select = max_vote > 1;

    let opts = options.clone();

    // A single strict maximum is the winner (trophy + verdict); two or more options
    // sharing the top count is a tie — no trophy, shown as "Uafgjort". A tie on a
    // For/Imod motion means it is not carried, which the verdict line makes explicit.
    let n_at_max = counts
        .iter()
        .enumerate()
        .filter(|(i, &c)| !is_abstention(*i) && c == max_count)
        .count();
    let has_single_winner = show_results && max_count > 0 && n_at_max == 1;
    let is_tie = show_results && max_count > 0 && n_at_max > 1;
    let winning_option = if has_single_winner {
        counts
            .iter()
            .enumerate()
            .find(|(i, &c)| !is_abstention(*i) && c == max_count)
            .and_then(|(i, _)| opts.get(i).cloned())
    } else {
        None
    };

    // A CSV snapshot of the tally for the minutes/archive, plus a print action.
    // Parity with the old wiki (which exported only the member roster): the
    // results are now exportable too. Headers stay English (spreadsheet
    // convention, like the roster export); the button labels are localized.
    let results_csv = {
        let mut s = String::new();
        if !name.is_empty() {
            s.push_str(&crate::export::csv_field(&name));
            s.push_str("\n\n");
        }
        s.push_str("Option,Votes,Percent\n");
        for (i, opt) in opts.iter().enumerate() {
            let count = counts.get(i).copied().unwrap_or(0);
            let base = if multi_select || is_abstention(i) {
                total_votes
            } else {
                cast_votes
            };
            let pct = (count * 100).checked_div(base).unwrap_or(0);
            s.push_str(&crate::export::csv_field(opt));
            s.push_str(&format!(",{count},{pct}%\n"));
        }
        s.push('\n');
        s.push_str(&format!("Total votes,{total_votes}\n"));
        if eligible_count > 0 {
            s.push_str(&format!("Eligible,{eligible_count}\n"));
            s.push_str(&format!("Turnout,{turnout_pct}%\n"));
        }
        if let Some(win) = &winning_option {
            s.push_str(&format!("Result,{}\n", crate::export::csv_field(win)));
        } else if is_tie {
            s.push_str("Result,Tie\n");
        }
        s
    };
    let results_filename = format!("{}-results.csv", crate::export::sanitize_filename(&name));

    let submit = {
        let token = session.read().access_token.clone();
        let poll = poll_id.clone();
        let ctx = context_id.clone();
        let opts = options.clone();
        let min = min_vote;
        let max = max_vote;
        let uid = user_id.clone();
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
            let uid = uid.clone();
            // Optimistic: show the ballot as cast and move the tally bars at once.
            casting.set(true);
            cast_pending.set(Some(chosen.clone()));
            spawn(async move {
                // Key the vote by the voter, so a second cast collides on the nodes
                // (parent_id, key) UNIQUE constraint — the DB enforces one vote per
                // member, not just the client-side check. Falls back to a timestamp
                // only if somehow unauthenticated (which the normal path rejects).
                // Secret ballots use the backend's has-voted dedup instead.
                let suffix = uid.clone().unwrap_or_else(|| format!("{:.0}", now_ms()));
                // A secret ballot goes through the backend (anonymous insert +
                // has-voted marker); a normal cast inserts under the user's token.
                let result = if poll_secret {
                    match token.as_deref() {
                        Some(t) => {
                            crate::backend_api::vote_cast_secret(t, &poll, ctx.as_deref(), &chosen)
                                .await
                        }
                        None => Err("not signed in".to_string()),
                    }
                } else {
                    graphql::cast_vote(token.as_deref(), &poll, ctx.as_deref(), &chosen, &suffix)
                        .await
                        .map(|_| ())
                };
                casting.set(false);
                match result {
                    Ok(()) => {
                        show_snackbar(&t("vote.hasVoted"));
                        // Leave cast_pending: `voted` will flip true on refetch and the
                        // optimistic add drops itself (show_opt gates on !voted).
                        refresh += 1;
                    }
                    // "already voted" is the backend's secret-ballot signal; a
                    // uniqueness violation is the normal path's DB-enforced
                    // one-vote-per-member. Both mean the same to the voter.
                    Err(e)
                        if e == "already voted"
                            || e.contains("niqueness")
                            || e.contains("nodes_parent_id_namespace_key") =>
                    {
                        // The voter had already voted: keep the "voted" state (it is
                        // true on the server), just drop the double-count.
                        cast_pending.set(None);
                        error.set(t("vote.hasVoted"))
                    }
                    _ => {
                        // Genuine failure: roll the ballot back so they can retry.
                        cast_pending.set(None);
                        error.set(t("error.somethingWentWrong"))
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("vote/poll")} }
                div {
                    h3 { class: "title-medium", "{name}" }
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        if !open { "{t(\"vote.noVoteNow\")}" } else if voted { "{t(\"vote.hasVoted\")}" } else { "{t(\"poll.managePoll\")}" }
                    }
                }
                div { class: "flex-grow" }
                // Owner-only: close the poll (mutable:false) so results show. Never
                // on the projector — the room-facing screen carries no controls.
                if open && node.is_context_owner.unwrap_or(false) && !projector {
                    button {
                        class: "btn-icon",
                        aria_label: "{t(\"poll.stopPoll\")}",
                        title: "{t(\"poll.stopPoll\")}",
                        onclick: {
                            let poll_id = poll_id.clone();
                            move |_| {
                                let token = session.read().access_token.clone();
                                let poll_id = poll_id.clone();
                                // Optimistic: show results at once; revert on error.
                                closed_opt.set(true);
                                spawn(async move {
                                    match graphql::update_node(
                                        token.as_deref(),
                                        &poll_id,
                                        model::NodesSetInput {
                                            mutable: Some(false),
                                            ..Default::default()
                                        },
                                    )
                                    .await
                                    {
                                        Ok(true) => crate::session::bump_data_version(),
                                        other => {
                                            closed_opt.set(false);
                                            log::error!("stop poll failed: {other:?}");
                                            show_snackbar(&t("error.somethingWentWrong"));
                                        }
                                    }
                                });
                            }
                        },
                        span { class: "material-icons", "stop" }
                    }
                }
            }

            div { class: "card-content",
                if options.is_empty() {
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.noContent\")}"
                    }
                } else if is_auth && open && !voted && !projector {
                    // The ballot: single-choice uses an accessible RadioGroup,
                    // multi-choice uses Checkbox per option. On the projector the
                    // ballot is never shown (casting happens on personal devices) —
                    // the room sees the live tally via the read-only branch below.
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
                                                div {
                                                    class: if selected.read().get(ri).copied().unwrap_or(false) { "list-item ballot-option selected" } else { "list-item ballot-option" },
                                                    key: "{ri}",
                                                    // DESIGN (functional): the whole option card selects, not
                                                    // just the small radio. Idempotent with the RadioItem for
                                                    // single-choice, so clicking either works.
                                                    onclick: move |_| {
                                                        let mut cur = vec![false; len];
                                                        cur[ri] = true;
                                                        selected.set(cur);
                                                        error.set(String::new());
                                                    },
                                                    RadioItem { value: "{ri}", index: dp, aria_label: "{option}" }
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
                                        div {
                                            class: if selected.read().get(ri).copied().unwrap_or(false) { "list-item ballot-option selected" } else { "list-item ballot-option" },
                                            key: "{ri}",
                                            Checkbox {
                                                checked: Some(if selected.read().get(ri).copied().unwrap_or(false) {
                                                    CheckboxState::Checked
                                                } else {
                                                    CheckboxState::Unchecked
                                                }),
                                                aria_label: "{option}",
                                                on_checked_change: move |_| apply_toggle(selected, error, ri, false, max_vote),
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
                        p { class: "body-medium text-error", "{error}" }
                    }
                    // The civic focal action: casting a vote is a democratic act,
                    // so it gets the magenta tertiary emphasis, not plain primary.
                    button {
                        class: "btn btn-cast mt-1",
                        disabled: casting(),
                        onclick: submit,
                        span { class: "material-icons", "how_to_vote" }
                        " {t(\"vote.castVote\")}"
                    }
                } else {
                    // Read-only option list with per-option tallies (closed poll,
                    // already voted, or logged out).
                    // Closed poll: announce the outcome prominently (the room follows
                    // this on the projector), distinct from an open ballot.
                    if !open && show_results {
                        div { class: "ballot-verdict",
                            span { class: "material-icons", if is_tie { "balance" } else { "emoji_events" } }
                            if let Some(win) = winning_option.clone() {
                                span { "{t(\"vote.resultWinner\")}: {win}" }
                            } else if is_tie {
                                span { "{t(\"vote.resultTie\")}" }
                            } else {
                                span { "{t(\"vote.resultNone\")}" }
                            }
                        }
                    }
                    div { class: "list",
                        for (i , option) in opts.iter().enumerate() {
                            div {
                                class: if is_abstention(i) {
                                    "list-item ballot-abstention"
                                } else if has_single_winner && counts.get(i).copied().unwrap_or(0) == max_count {
                                    "list-item ballot-winner"
                                } else {
                                    "list-item"
                                },
                                key: "{i}",
                                div { class: "avatar small", "{i + 1}" }
                                div { class: "list-item-text",
                                    div { class: "list-item-primary",
                                        "{option}"
                                        if has_single_winner && !is_abstention(i) && counts.get(i).copied().unwrap_or(0) == max_count {
                                            span { class: "winner-badge material-icons", "emoji_events" }
                                        }
                                    }
                                    {
                                        if show_results {
                                            let count = counts.get(i).copied().unwrap_or(0);
                                            // Single-choice non-blank options divide by cast (non-blank)
                                            // votes so the For/Imod split is decisive; blanks and
                                            // multi-select ballots divide by the ballot count.
                                            let base = if multi_select || is_abstention(i) { total_votes } else { cast_votes };
                                            let pct = (count * 100).checked_div(base).unwrap_or(0);
                                            let fraction = count as f64 / base.max(1) as f64;
                                            rsx! {
                                                crate::components::widgets::Bar { fraction }
                                                div { class: "list-item-secondary", "{count} ({pct}%)" }
                                            }
                                        } else {
                                            rsx! {}
                                        }
                                    }
                                }
                            }
                        }
                    }
                    p { class: "body-medium mt-1",
                        if voted { "{t(\"vote.hasVoted\")} · " }
                        if show_results {
                            "{t(\"vote.voteCount\")}: {total_votes}"
                            if eligible_count > 0 {
                                " · {t(\"vote.turnout\")}: {total_votes}/{eligible_count} ({turnout_pct}%)"
                            }
                        } else {
                            "{t(\"poll.resultsHidden\")}"
                        }
                    }
                    // Export/print the tally for the minutes (hidden on the projector,
                    // which is a clean display, and only when there are results to save).
                    if show_results && !projector {
                        div { class: "results-actions mt-1",
                            button {
                                class: "btn-text",
                                onclick: {
                                    let csv = results_csv.clone();
                                    let file = results_filename.clone();
                                    move |_| {
                                        crate::export::download_bytes(
                                            &file,
                                            "text/csv;charset=utf-8",
                                            csv.as_bytes(),
                                        )
                                    }
                                },
                                span { class: "material-icons", "download" }
                                " {t(\"vote.exportCsv\")}"
                            }
                            button {
                                class: "btn-text",
                                onclick: move |_| crate::export::print_page(),
                                span { class: "material-icons", "print" }
                                " {t(\"vote.print\")}"
                            }
                        }
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
    max: usize,
) {
    let mut cur = selected.read().clone();
    if idx >= cur.len() {
        return;
    }
    if single {
        cur = vec![false; cur.len()];
        cur[idx] = true;
    } else {
        // Block over-selection as it happens (not only at submit): refuse a new
        // check once `max` options are already selected.
        if !cur[idx] && cur.iter().filter(|&&b| b).count() >= max {
            error.set(t_with("vote.selectAtMost", &[("count", &max.to_string())]));
            return;
        }
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

/// Owner-only control to open a poll on a policy / change / position: a "start"
/// button and a small dialog (hide-result toggle, plus a vote-range for a
/// position with more than two candidates). Mirrors React's PollDialog — it
/// closes any prior active poll, inserts a `vote/poll`, sets the context
/// `active` relation, and navigates to the new ballot.
#[component]
pub(super) fn StartPollButton(node: NodeWithChildren, path: Vec<String>) -> Element {
    let mime = node.mime_id.clone().unwrap_or_default();
    let is_position = mime == "vote/position";
    let options: Vec<String> = if is_position {
        let mut o: Vec<String> = node
            .children
            .iter()
            .filter(|c| c.mime_id.as_deref() == Some("vote/candidate"))
            .map(|c| c.name.clone())
            .collect();
        o.push("Blank".to_string());
        o
    } else {
        vec!["For".to_string(), "Imod".to_string(), "Blank".to_string()]
    };
    let opt_count = options.len();
    let max_range = opt_count.saturating_sub(1).max(1);

    let session = use_session();
    let nav = use_navigator();
    let mut open = use_signal(|| false);
    let mut hidden = use_signal(|| is_position);
    // Secret ballot: casts route through the backend so vote nodes carry no
    // owner_id (untraceable). Defaults on for candidate elections (positions).
    let mut secret = use_signal(|| is_position);
    let mut min_vote = use_signal(|| 1usize);
    let mut max_vote = use_signal(|| 1usize);

    // Non-owners get nothing (hooks above run unconditionally).
    if !node.is_context_owner.unwrap_or(false) {
        return rsx! {};
    }

    let node_id = node.id.0.clone();
    let context_id = node.context_id.clone().map(|c| c.0);
    let name = node.name.clone();
    let range_label = t("poll.voteRange");

    rsx! {
        div { class: "card mt-1",
            div { class: "card-header",
                // `small`, like every other section card on the pages this sits
                // among (candidates, questions, amendments, polls): a full-size
                // avatar belongs to a page header, not to a section of one.
                div { class: "avatar small", {icon_el("vote/poll")} }
                h3 { class: "title-medium", "{t(\"poll.newPoll\")}" }
                div { class: "flex-grow" }
                button {
                    class: "btn-icon add-action state-layer",
                    aria_label: "{t(\"poll.newPoll\")}",
                    title: "{t(\"poll.newPoll\")}",
                    onclick: move |_| open.set(true),
                    span { class: "material-icons", "play_arrow" }
                }
            }
        }
        crate::components::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: t("poll.newPoll"),
            icon: "ballot".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-text",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    onclick: {
                        let node_id = node_id.clone();
                        let context_id = context_id.clone();
                        let name = name.clone();
                        let options = options.clone();
                        let path = path.clone();
                        move |_| {
                            let token = session.read().access_token.clone();
                            let Some(context_id) = context_id.clone() else {
                                return;
                            };
                            let parent_id = node_id.clone();
                            let name = name.clone();
                            let options = options.clone();
                            let hidden = hidden();
                            let secret = secret();
                            let mn = min_vote();
                            let mx = max_vote().max(mn);
                            let mut poll_path = path.clone();
                            spawn(async move {
                                let key = format!("poll{}", js_sys::Date::now() as u64);
                                match graphql::create_poll(
                                    token.as_deref(),
                                    &parent_id,
                                    &context_id,
                                    &name,
                                    &key,
                                    &options,
                                    mn,
                                    mx,
                                    model::BallotRules {
                                        hide_tally: hidden,
                                        secret,
                                    },
                                )
                                .await
                                {
                                    Ok(inserted) => {
                                        crate::session::bump_data_version();
                                        open.set(false);
                                        poll_path.push(inserted.key);
                                        // Best-effort background push to the group's members
                                        // ("a vote has opened"); the backend gates this on the
                                        // caller owning the context, so non-owners just no-op.
                                        if let Some(tok) = token.clone() {
                                            let ctx = context_id.clone();
                                            let title = t("vote.pollOpenTitle");
                                            let body = if name.trim().is_empty() {
                                                t("vote.pollOpenBody")
                                            } else {
                                                name.clone()
                                            };
                                            let link = format!("/{}", poll_path.join("/"));
                                            spawn(async move {
                                                let _ = crate::backend_api::push_notify(
                                                    &tok, &ctx, &title, &body, &link,
                                                )
                                                .await;
                                            });
                                        }
                                        nav.push(Route::PathPage {
                                            segments: poll_path,
                                            app: None,
                                        });
                                    }
                                    Err(e) => {
                                        open.set(false);
                                        show_snackbar(&e);
                                    }
                                }
                            });
                        }
                    },
                    "{t(\"poll.start\")}"
                }
            },
            if is_position && opt_count > 2 {
                div { class: "range-field",
                    div { class: "body-medium range-field-label",
                        "{range_label}: {min_vote} to {max_vote}"
                    }
                    input {
                        r#type: "range",
                        min: "1",
                        max: "{max_range}",
                        value: "{min_vote}",
                        aria_label: t("poll.minVotesLabel"),
                        aria_valuetext: "{min_vote}",
                        oninput: move |e| {
                            let v: usize = e.value().parse().unwrap_or(1);
                            min_vote.set(v);
                            if max_vote() < v {
                                max_vote.set(v);
                            }
                        },
                    }
                    input {
                        r#type: "range",
                        min: "1",
                        max: "{max_range}",
                        value: "{max_vote}",
                        aria_label: t("poll.maxVotesLabel"),
                        aria_valuetext: "{max_vote}",
                        oninput: move |e| {
                            let v: usize = e.value().parse().unwrap_or(1);
                            max_vote.set(v.max(min_vote()));
                        },
                    }
                }
            }
            div { class: "list-item switch-row",
                span { class: "switch-row-label", "{t(\"poll.hideResult\")}" }
                Switch {
                    checked: Some(hidden()),
                    aria_label: t("poll.hideResult"),
                    on_checked_change: move |v: bool| hidden.set(v),
                }
            }
            div { class: "list-item switch-row",
                span { class: "switch-row-label", "{t(\"poll.secretBallot\")}" }
                Switch {
                    checked: Some(secret()),
                    aria_label: t("poll.secretBallot"),
                    on_checked_change: move |v: bool| secret.set(v),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ballot_order, poll_config};
    use serde_json::json;

    #[test]
    fn poll_config_reads_options_and_bounds() {
        let data = json!({
            "options": ["Yes", "No", "Blank"],
            "minVote": 1,
            "maxVote": 2,
        });
        let cfg = poll_config(Some(&data));
        assert_eq!(cfg.options, vec!["Yes", "No", "Blank"]);
        assert_eq!(cfg.min_vote, 1);
        assert_eq!(cfg.max_vote, 2);
    }

    #[test]
    fn poll_config_defaults_when_absent_or_malformed() {
        // No data at all -> no options, single-choice defaults (min=max=1).
        let cfg = poll_config(None);
        assert!(cfg.options.is_empty());
        assert_eq!((cfg.min_vote, cfg.max_vote), (1, 1));

        // Non-string option entries are dropped; missing bounds fall back to 1.
        let data = json!({ "options": ["Yes", 42, null, "No"] });
        let cfg = poll_config(Some(&data));
        assert_eq!(cfg.options, vec!["Yes", "No"]);
        assert_eq!((cfg.min_vote, cfg.max_vote), (1, 1));

        // A negative/!u64 bound is ignored (falls back to the default).
        let data = json!({ "options": [], "minVote": -3, "maxVote": 5 });
        let cfg = poll_config(Some(&data));
        assert_eq!((cfg.min_vote, cfg.max_vote), (1, 5));
    }

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
