//! The ballot: opening a poll, casting on it, and reading how it stands.
//!
//! To the components a poll is a node whose `data` says what is asked and whose
//! `mutable` says whether it is still open, with a ballot as a child row under
//! it. The AppView keeps a poll's state and its ballots in tables of their own,
//! so a poll is read there and dressed as that node.

use super::seen::saw_node;
use super::{ask, ask_quiet, client, reported};
use crate::model::{BallotRules, InsertedNode, Jsonb, PollSummaryFields, Timestamptz, Uuid};
use appview_client::{cast_open_ballot, close_poll, defs, get_poll, list_polls, open_poll};

/// A poll's `data` as the interim keeps it, which is what its components read.
pub(crate) fn poll_data(poll: &defs::PollView) -> serde_json::Value {
    serde_json::json!({
        "options": poll.options,
        "minVote": poll.min,
        "maxVote": poll.max,
        "hidden": poll.hide_tally,
        "secret": poll.secret,
        "nodeId": poll.parent_id,
    })
}

/// One poll, if the reader may see it.
pub(crate) async fn read_poll(access_token: Option<&str>, id: &str) -> Option<defs::PollView> {
    let client = client(access_token);
    let params = get_poll::Params { id: id.to_string() };
    ask_quiet(true, || client.get_poll(&params)).await.ok()
}

/// The polls on a node, by id: what a page's poll rows are filled in from.
pub(crate) async fn polls_on(access_token: Option<&str>, parent_id: &str) -> Vec<defs::PollView> {
    let client = client(access_token);
    let params = list_polls::Params {
        parent: Some(parent_id.to_string()),
        ..Default::default()
    };
    ask_quiet(true, || client.list_polls(&params))
        .await
        .map(|listed| listed.polls)
        .unwrap_or_default()
}

/// Open a poll on a motion, close the one the room had open before it, and put
/// the new one on the screen, as the interim's dialog does.
#[allow(clippy::too_many_arguments)]
pub async fn create_poll(
    access_token: Option<&str>,
    parent_id: &str,
    context_id: &str,
    name: &str,
    _key: &str,
    options: &[String],
    min_vote: usize,
    max_vote: usize,
    rules: BallotRules,
) -> Result<InsertedNode, String> {
    let client = client(access_token);
    if let Ok(Some(prior)) = super::active_node_id(access_token, context_id).await {
        if read_poll(access_token, &prior)
            .await
            .is_some_and(|p| p.open)
        {
            let close = close_poll::Input { id: prior };
            let _ = ask_quiet(false, || client.close_poll(&close)).await;
        }
    }
    let open = open_poll::Input {
        parent_id: parent_id.to_string(),
        title: name.to_string(),
        options: options.to_vec(),
        min: i64::try_from(min_vote).ok(),
        max: i64::try_from(max_vote).ok(),
        // The interim's ballots end in the abstention, which the dialog appends.
        blank: Some(options.last().is_some_and(|last| last == "blank")),
        secret: Some(rules.secret),
        hide_tally: Some(rules.hide_tally),
        ..Default::default()
    };
    let poll = ask("openPoll", false, || client.open_poll(&open)).await?;
    saw_node(&poll.id, "document", "poll");
    super::set_active_relation(access_token, context_id, Some(&poll.id)).await?;
    Ok(InsertedNode {
        id: Uuid(poll.id),
        key: poll.path.rsplit('/').next().unwrap_or_default().to_string(),
    })
}

/// Cast an open ballot: a show of hands, where who chose what is on record.
pub async fn cast_vote(
    access_token: Option<&str>,
    poll_id: &str,
    _context_id: Option<&str>,
    selected: &[usize],
    _key_suffix: &str,
) -> Result<bool, String> {
    let client = client(access_token);
    let ballot = cast_open_ballot::Input {
        poll: poll_id.to_string(),
        choices: selected
            .iter()
            .filter_map(|choice| i64::try_from(*choice).ok())
            .collect(),
    };
    match ask_quiet(false, || client.cast_open_ballot(&ballot)).await {
        Ok(_) => Ok(true),
        // In the words the poll screen takes to mean the ballot is in.
        Err(error) if error.name() == Some("AlreadyVoted") => Err("already voted".to_string()),
        Err(error) => Err(reported("castOpenBallot", &error)),
    }
}

/// How a poll stands: the count per option, the ballots cast, and whether the
/// caller's is among them (1 or 0). Where the counts are hidden from the caller
/// they read as zero, and the turnout is still said.
pub async fn poll_tally(
    access_token: Option<&str>,
    poll_id: &str,
    options: usize,
    own_of: Option<&str>,
) -> Result<(Vec<usize>, usize, usize), String> {
    let client = client(access_token);
    let params = get_poll::Params {
        id: poll_id.to_string(),
    };
    let poll = ask("getPoll", true, || client.get_poll(&params)).await?;
    let count = |n: i64| usize::try_from(n).unwrap_or(0);
    let mut counts: Vec<usize> = poll
        .counts
        .unwrap_or_default()
        .into_iter()
        .map(count)
        .collect();
    counts.resize(options, 0);
    let voted = poll
        .viewer
        .as_ref()
        .is_some_and(|viewer| match poll.secret {
            // Collected, which is all the server can know of a secret ballot.
            true => viewer.issued,
            false => viewer.choices.is_some(),
        });
    let own = usize::from(own_of.is_some() && voted);
    Ok((counts, count(poll.ballots), own))
}

pub async fn poll_vote_count(access_token: Option<&str>, poll_id: &str) -> Result<usize, String> {
    Ok(poll_tally(access_token, poll_id, 0, None).await?.1)
}

/// A poll's ballots, each as the options it chose, for a screen that tallies
/// them itself. The AppView hands out counts and no ballots, so these are made
/// up to say the same: as many as were cast, with each option chosen as often
/// as it was. Which ballot chose what together is not something it tells.
pub async fn query_poll_votes(
    access_token: Option<&str>,
    poll_id: &str,
) -> Result<Vec<Vec<usize>>, String> {
    let client = client(access_token);
    let params = get_poll::Params {
        id: poll_id.to_string(),
    };
    let poll = ask("getPoll", true, || client.get_poll(&params)).await?;
    let counts = poll.counts.unwrap_or_default();
    Ok((0..poll.ballots)
        .map(|ballot| {
            counts
                .iter()
                .enumerate()
                .filter(|(_, chosen)| ballot < **chosen)
                .map(|(option, _)| option)
                .collect()
        })
        .collect())
}

/// Every poll of a context, newest first, for the chair's overview.
pub async fn query_context_polls(
    access_token: Option<&str>,
    context_id: &str,
) -> Result<Vec<PollSummaryFields>, String> {
    let client = client(access_token);
    let params = list_polls::Params {
        context: Some(context_id.to_string()),
        ..Default::default()
    };
    let mut polls = ask("listPolls", true, || client.list_polls(&params))
        .await?
        .polls;
    polls.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(polls
        .iter()
        .map(|poll| {
            saw_node(&poll.id, "document", "poll");
            PollSummaryFields {
                id: Uuid(poll.id.clone()),
                name: poll.question.clone(),
                data: Some(Jsonb(poll_data(poll))),
                created_at: Some(Timestamptz(poll.created_at.clone())),
                mutable: poll.open,
            }
        })
        .collect())
}
