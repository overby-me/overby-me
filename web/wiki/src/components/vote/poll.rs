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

    // Whether this reader is an active member of the poll's context, to warn
    // BEFORE they fill in a ballot the server will refuse. VoteApp has shown this
    // for a while; a poll opened from its own page (which is where a link to a
    // ballot lands) showed nothing, so the first news of having no vote was the
    // refusal after casting one.
    //
    // A warning, not a gate. Membership is one of four things the insert rule
    // wants (see graphql::is_active_member), so a ballot can still be refused
    // with this saying yes, and the check is narrower than the rule even on its
    // own arm. Hiding the ballot on it would take the vote away from people the
    // server would have accepted, and the honest answer for the rest is the one
    // the server gives.
    let rights_ctx = context_id.clone();
    let rights_token = session.read().access_token.clone();
    let rights_user = session.read().user.as_ref().map(|u| u.id.clone());
    let may_vote_res =
        crate::use_data_resource!(|(rights_ctx, rights_token, rights_user)| async move {
            match (rights_ctx, rights_user) {
                (Some(ctx), Some(uid)) => {
                    graphql::is_active_member(rights_token.as_deref(), &ctx, &uid).await
                }
                _ => None,
            }
        });
    let may_vote: Option<bool> = (*may_vote_res.read()).flatten();

    let options_len = options.len();
    let mut selected = use_signal(|| vec![false; options_len]);
    let mut error = use_signal(String::new);
    let mut refresh = use_signal(|| 0u32);
    // Optimistic cast: the chosen option indices AND the tally total as it stood
    // when they were cast, shown as voted + counted at once and dropped once the
    // tally itself passes that total (see `show_opt`) or on error.
    let mut cast_pending = use_signal(|| None::<(Vec<usize>, usize)>);
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
        crate::graphql::nodes_changed_typed(crate::graphql::children_of_mime(
            &sub_poll,
            "vote/vote",
        )),
        refresh,
    );
    // The chair opening or closing the poll, STREAMED: the pushed row carries the
    // new `mutable`, so the ballot hides the moment the poll closes without
    // anything being fetched to find out. It used to be a change token that
    // triggered a whole node-with-children query to read one boolean.
    let mut live_open = use_signal(|| node.mutable);
    let poll_since = use_hook(crate::session::server_now_iso);
    let poll_stream = crate::subscription::use_graphql_subscription(graphql::state_stream(
        graphql::node_is(&sub_poll),
        &poll_since,
    ));
    use_effect(move || {
        let Some(payload) = poll_stream.read().clone() else {
            return;
        };
        // The newest row wins: a batch can carry several changes to one node.
        if let Some(mutable) = payload
            .get("nodes_stream")
            .and_then(|r| r.as_array())
            .and_then(|rows| rows.iter().rev().find_map(|r| r.get("mutable")?.as_bool()))
        {
            live_open.set(mutable);
        }
    });

    // Live results depend on the poll (node) id and the refresh counter; use
    // use_reactive so they re-run when navigating to a different poll, not only
    // via a keyed remount (unreliable in the web renderer).
    let rev = *refresh.read();
    let n_opts = options.len();

    // Tally of the votes visible to this user (all of them for the poll owner /
    // an admin; just their own otherwise). Counts per option index.
    //
    // Ahead of the has-voted check because, on a normal poll, it ANSWERS it: the
    // voter's own ballots are one more aggregate in the same request. Casting used
    // to refresh two, this and a whole-node query counted with `.len()`, and two
    // requests on one trigger land at different moments, which is what moved the
    // result bars a second time.
    let ty_poll = poll_id.clone();
    let ty_token = session.read().access_token.clone();
    // Counted by the server. A poll that does not show its results still needs
    // the turnout total for the quorum line, so it asks for that alone rather
    // than for counts it would not draw.
    let ty_show = show_results;
    // A secret ballot has no `ownerId` to count by, so there is nothing to ask for
    // and the backend's marker answers below instead.
    let ty_own = if poll_secret { None } else { user_id.clone() };
    let tally = crate::use_data_resource!(
        |(ty_poll, ty_token, n_opts, ty_show, ty_own, rev)| async move {
            let _ = rev;
            let wanted = if ty_show { n_opts } else { 0 };
            let (counts, total, own) =
                graphql::poll_tally(ty_token.as_deref(), &ty_poll, wanted, ty_own.as_deref())
                    .await
                    .unwrap_or_else(|_| (Vec::new(), 0, 0));
            let mut counts = counts;
            counts.resize(n_opts, 0);
            (counts, total, own)
        }
    );
    let (counts, fetched_total, own_votes) = tally.read().clone().unwrap_or((vec![], 0, 0));

    // Whether the current user has already voted.
    //
    // A secret poll only. Its votes are anonymous by construction, so the count
    // above cannot see them and our own backend holds the has-voted marker.
    let av_poll = poll_id.clone();
    let av_token = session.read().access_token.clone();
    let av_secret = poll_secret;
    // Having voted is final, so stop asking. Without this every device re-ran the
    // check on every refresh for the rest of the vote — and that check is a call to
    // our own backend rather than to Hasura, so it is the one piece of the ballot
    // that lands on a single small server. A delegate votes early and then
    // watches; this makes those minutes free.
    let mut voted_latch = use_signal(|| false);
    let secret_voted =
        crate::use_data_resource!(|(av_poll, av_token, av_secret, rev)| async move {
            let _ = rev;
            if !av_secret {
                return false;
            }
            if voted_latch() {
                return true;
            }
            match &av_token {
                Some(t) => crate::backend_api::vote_status(t, &av_poll).await,
                None => false,
            }
        });
    let voted = if poll_secret {
        secret_voted.read().unwrap_or(false)
    } else {
        own_votes > 0
    };
    use_effect(move || {
        if voted && !voted_latch() {
            voted_latch.set(true);
        }
    });
    // Fold the optimistic cast into the tally until the TALLY has caught up, then
    // drop it so the ballot is not counted twice.
    //
    // This used to hold the optimistic add until `voted` turned true, and `voted`
    // is a different query from the tally. Both re-run on the same `rev` bump and
    // resolve independently, so whichever landed first moved the bars a second
    // time: `voted` first dropped the +1 back onto a tally that had not refreshed
    // yet, and the bars fell and then rose; the tally first counted the ballot
    // twice, and they overshot and came back. Either way the bars animated, sat,
    // and animated again a moment after the vote.
    //
    // The honest test is the tally's own number. `cast_pending` remembers the
    // total as it stood when the ballot was cast, and the optimistic add stands
    // until the server reports more than that. The displayed total therefore only
    // ever rises, and it rises once: it goes from fetched+1 to a fetched value
    // that already includes the ballot, which is the same number and so no
    // transition at all.
    //
    // Another voter arriving in the same moment can satisfy this early. That
    // costs nothing: the count is still going up, and the tally is the authority
    // either way.
    let show_opt = match cast_pending.read().as_ref() {
        Some((_, total_at_cast)) => fetched_total <= *total_at_cast,
        None => false,
    };
    let counts = if show_opt {
        let mut c = counts;
        if let Some((chosen, _)) = cast_pending.read().as_ref() {
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
        fetched_total + 1
    } else {
        fetched_total
    };
    let voted = voted || cast_pending.read().is_some();
    // Eligible voters = active members of the poll's context, for the turnout /
    // quorum line the room reads off the projector.
    let el_ctx = context_id.clone();
    let el_token = session.read().access_token.clone();
    // NOT keyed on `rev`: the electorate is the context's active members, which a
    // vote does not change. Re-counting it on every ballot cast was one query per
    // device per vote — 250,000 of them across a 500-person poll — to re-learn a
    // number that only moves when someone joins or leaves. It still refreshes on
    // navigation and on focus, which is when membership can have changed.
    let eligible = crate::use_data_resource!(|(el_ctx, el_token)| async move {
        match el_ctx {
            Some(ctx) => graphql::count_active_members(el_token.as_deref(), &ctx).await,
            None => 0,
        }
    });
    let eligible_count = (*eligible.read()).unwrap_or(0);
    let turnout_pct = (total_votes * 100).checked_div(eligible_count).unwrap_or(0);

    // The ballot disappears when the chair closes the poll. The value comes from
    // the stream above (shadowing the prop); the server-side gate for late votes
    // is separate and does not trust this.
    let open = live_open() && !closed_opt();
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
            // The total is stamped as it stands NOW, which is what tells the tally
            // apart from itself-plus-this-ballot when it comes back.
            casting.set(true);
            cast_pending.set(Some((chosen.clone(), total_votes)));
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
                        // Leave cast_pending: it keeps the ballot showing as cast,
                        // and the optimistic add drops itself once the refetched
                        // tally passes the total stamped into it (see `show_opt`).
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
                    Err(e) => {
                        // Genuine failure: roll the ballot back so they can retry.
                        cast_pending.set(None);
                        // Say which failure. The backend refuses a ballot for
                        // reasons the voter can act on ("not a member of this
                        // context" means ask the chair for a seat), and answering
                        // all of them with "something went wrong" turns a fixable
                        // situation into a mystery.
                        // "check constraint of an insert/update permission has
                        // failed" is how Hasura says it refused the ballot under
                        // `insert_with_email_invites` (migrations/0015). Every
                        // arm of that rule is a variant of "you may not vote
                        // here": no vote permission in this context, the poll
                        // closed under you, the poll's parent locked. The voter
                        // cannot act on which, only on the fact, and answering
                        // with "something went wrong" hides that this is a
                        // decision rather than a fault. Which arm it was is in
                        // the log line below.
                        let refused = e.contains("permission has failed")
                            || e == "not a member of this context";
                        error.set(if refused {
                            t("vote.noVotingRight")
                        } else if e == "poll closed" {
                            t("vote.closed")
                        } else {
                            t("error.somethingWentWrong")
                        });
                        // And report it. The remote log ships from `log::` calls
                        // (snackbar.rs is where most of them come from); this arm
                        // wrote to an inline error signal instead and so reached
                        // Better Stack from nowhere. A refused ballot is exactly
                        // the event worth seeing from the outside.
                        log::error!("cast ballot failed on poll {poll}: {e}");
                    }
                }
            });
        }
    };

    rsx! {
        div { class: "card app-card",
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
                // Only the negative, and only when it is known and would matter:
                // someone about to fill in a ballot. A positive banner on every
                // poll is noise, and saying nothing while the answer is still
                // unknown beats guessing at it.
                if is_auth && open && !voted && !projector && may_vote == Some(false) {
                    div { class: "status-banner is-negative",
                        span { class: "material-icons", "do_not_disturb" }
                        span { class: "body-medium", "{t(\"vote.noVotingRight\")}" }
                    }
                }
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
                                    // How you voted is not a breadcrumb. Every
                                    // option here carries its text as both label
                                    // and aria-label, which is exactly what the
                                    // trail records, so without this a click on
                                    // "Imod" is filed under the voter's name.
                                    "data-private": "true",
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
                        // Same reason as the single-choice branch above.
                        div { class: "list", "data-private": "true",
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
                            // `btn` carries the layout (inline-flex, centring, the
                            // icon/label gap, the 40px pill); `btn-text` only
                            // colours it. Without the base these were bare native
                            // buttons: icon on the text baseline, no gap, no pill,
                            // no label typography. The leading space in the label
                            // was standing in for the gap, so it goes too.
                            button {
                                class: "btn btn-text",
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
                                "{t(\"vote.exportCsv\")}"
                            }
                            button {
                                class: "btn btn-text",
                                onclick: move |_| crate::export::print_page(),
                                span { class: "material-icons", "print" }
                                "{t(\"vote.print\")}"
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

/// The chair's row that opens [`StartPollDialog`], for the tools sheet's Meeting
/// group. Split from the dialog because the two cannot live in the same place:
/// the modal sheet is `transform`ed and `overflow: hidden`, and a transform makes
/// an element the containing block for its `position: fixed` descendants, so a
/// dialog rendered inside the sheet is positioned against the SHEET and clipped
/// by it. `content.rs` splits its delete confirmation for the same reason.
///
/// `open` is therefore owned by the caller, which renders the row here and the
/// dialog out in the page body.
#[component]
pub(super) fn StartPollButton(node: NodeWithChildren, open: Signal<bool>) -> Element {
    let mut open = open;
    // Non-owners get nothing. The sheet group is owner-gated too; this is the
    // component's own guarantee, not a reliance on where it is rendered.
    if !node.is_context_owner.unwrap_or(false) {
        return rsx! {};
    }
    rsx! {
        // A row, not a card. As a card this was a permanent empty header reading
        // "New poll" above the list of polls it creates, which announced a poll
        // section on pages that had none and repeated the heading on pages that
        // did. Opening a poll is a chair's action, so it sits with the chair's
        // other actions; the polls themselves are the list, which hides when
        // there are none.
        button {
            class: "sheet-action",
            onclick: move |_| open.set(true),
            span { class: "material-icons", "ballot" }
            "{t(\"poll.newPoll\")}"
        }
    }
}

/// Owner-only dialog to open a poll on a policy / change / position (hide-result
/// toggle, plus a vote-range for a position with more than two candidates).
/// Mirrors React's PollDialog — it closes any prior active poll, inserts a
/// `vote/poll`, sets the context `active` relation, and navigates to the new
/// ballot. Its trigger is [`StartPollButton`].
#[component]
pub(super) fn StartPollDialog(
    node: NodeWithChildren,
    path: Vec<String>,
    open: Signal<bool>,
) -> Element {
    let mut open = open;
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
            // The M3 two-line list item every other switch in this app uses
            // (usermenu's dark mode, perm's open-to-all): leading state icon,
            // label over a supporting line, control at the end. These two rows
            // had a bare label and no icon, so they sat flush against the dialog
            // edge with nothing to align to, and each carried its explanation as
            // a parenthetical in the label — which is the supporting line, set as
            // the title. The icon tracks the state, so the row reads as on or off
            // from across a table.
            div { class: "list-item switch-row",
                span { class: "material-icons",
                    {if hidden() { "visibility_off" } else { "visibility" }}
                }
                div { class: "list-item-text",
                    div { class: "list-item-primary", "{t(\"poll.hideResult\")}" }
                    div { class: "list-item-secondary", "{t(\"poll.hideResultHint\")}" }
                }
                Switch {
                    checked: Some(hidden()),
                    aria_label: t("poll.hideResult"),
                    on_checked_change: move |v: bool| hidden.set(v),
                }
            }
            div { class: "list-item switch-row",
                span { class: "material-icons",
                    {if secret() { "lock" } else { "lock_open" }}
                }
                div { class: "list-item-text",
                    div { class: "list-item-primary", "{t(\"poll.secretBallot\")}" }
                    div { class: "list-item-secondary", "{t(\"poll.secretBallotHint\")}" }
                }
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
