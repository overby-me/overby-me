use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::{t, t_with};
use crate::route::Route;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

use super::content::ContentApp;
use super::loader::{icon_el, mime_icon, visible_sorted};
use super::ui::checkbox::Checkbox;
use super::ui::radio_group::{RadioGroup, RadioItem};
use super::ui::switch::Switch;
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

    let no_vote = rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("vote/poll")} }
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
    let content = match state {
        Some(Some(active)) if active.mime_id.as_deref() == Some("vote/poll") => {
            rsx! { PollApp { node: active } }
        }
        Some(_) => no_vote,
        None => rsx! {
            div { class: "spinner-overlay",
                div { class: "spinner" }
            }
        },
    };

    rsx! {
        // Voting-rights indicator (React VoteApp's canVote status).
        if is_auth {
            div { class: "card mb-1",
                div { class: "card-content stack stack-h", style: "align-items: center; gap: 8px;",
                    span {
                        class: "material-icons",
                        style: if can_vote { "color: var(--md-primary);" } else { "color: var(--md-error, #b3261e);" },
                        if can_vote { "how_to_reg" } else { "do_not_disturb" }
                    }
                    span { class: "body-medium",
                        if can_vote { "{t(\"vote.hasVotingRight\")}" } else { "{t(\"vote.noVotingRight\")}" }
                    }
                }
            }
        }
        {content}
    }
}

/// PolicyApp — document with comments, changes, and polls. Sub-changes form a
/// tree: each `vote/change` row links into its own PolicyApp, so the whole
/// amendment tree is browsable (#112).
#[component]
pub fn PolicyApp(node: NodeWithChildren, path: Vec<String>) -> Element {
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

    // Other children (questions etc.); comments now render in the nested
    // CommentSection (via ContentApp), so they are excluded here.
    let comments: Vec<_> = children
        .iter()
        .filter(|c| {
            !matches!(
                c.mime_id.as_deref(),
                Some("vote/poll") | Some("vote/change") | Some("vote/comment")
            )
        })
        .collect();

    let node_id = node.id.0.clone();
    let context_id = node.context_id.as_ref().map(|u| u.0.clone());

    rsx! {
        // Main content. The comment thread renders at the end, below the
        // amendments and polls.
        ContentApp { node: node.clone() }

        // Owner-only: open a poll on this policy/change.
        StartPollButton { node: node.clone(), path: path.clone() }

        // Amendments — always shown so its create action (in the header) has a
        // home; the body shows an empty state until the first amendment lands.
        div { class: "card mt-1",
            div { class: "card-header",
                div { class: "avatar", {icon_el("vote/change")} }
                h3 { class: "title-medium", "{t(\"vote.amendments\")}" }
                div { class: "flex-grow" }
                // Propose a new amendment (redirects to its editor).
                AddChangeButton { node: node.clone(), path: path.clone() }
            }
            if amendments.is_empty() {
                div { class: "card-content",
                    p { class: "body-medium", class: "text-muted", "{t(\"vote.noAmendments\")}" }
                }
            } else {
                div { class: "list",
                    for (n , item) in amendments.iter().enumerate() {
                        {
                            let mut full = path.clone();
                            full.push(item.key.clone());
                            rsx! {
                                Link {
                                    key: "{item.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "folder-item",
                                    div { class: "avatar small",
                                        {super::loader::node_avatar("vote/change", &item.name, Some(n))}
                                    }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{item.name}" }
                                    }
                                }
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
                    div { class: "avatar", {icon_el("vote/poll")} }
                    h3 { class: "title-medium", "{t(\"mime.vote\")}" }
                }
                div { class: "list",
                    for poll in polls.iter() {
                        {
                            let mut full = path.clone();
                            full.push(poll.key.clone());
                            rsx! {
                                Link {
                                    key: "{poll.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "folder-item",
                                    div { class: "avatar small", {icon_el("vote/poll")} }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{poll.name}" }
                                    }
                                    PollVoteBadge { poll_id: poll.id.0.clone() }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Discussion thread for the policy/change, below its amendments/polls.
        super::comments::CommentSection { node_id: node_id.clone(), context_id: context_id.clone() }

        // Other children (comments, questions)
        if !comments.is_empty() {
            div { class: "card mt-1",
                div { class: "list",
                    for child in comments.iter() {
                        {
                            let mut full = path.clone();
                            full.push(child.key.clone());
                            let icon = mime_icon(child.mime_id.as_deref().unwrap_or(""));
                            rsx! {
                                Link {
                                    key: "{child.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "folder-item",
                                    div { class: "avatar small", span { class: "material-icons", "{icon}" } }
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
    }
}

/// PositionApp — a `vote/position` (candidate election): the position text (with
/// its edit/delete affordances), a candidate photo gallery, the numbered
/// questions list (add + owner delete), and any polls. Mirrors React PositionApp
/// (ContentApp + CandidateList + QuestionList + PollList).
#[component]
pub fn PositionApp(node: NodeWithChildren, path: Vec<String>) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let token = session.read().access_token.clone();
    let children = visible_sorted(&node.children);
    let children = &children;

    let candidates: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/candidate"))
        .collect();
    let questions: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/question"))
        .collect();
    let polls: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/poll"))
        .collect();

    let node_id = node.id.0.clone();
    let context_id = node.context_id.clone().map(|c| c.0);
    let mut q_text = use_signal(String::new);

    // Add a question (a `vote/question` child carrying `data.text`), mirroring
    // React AddQuestionButton. The node is immutable; its name records the author.
    let add_question = {
        let node_id = node_id.clone();
        let context_id = context_id.clone();
        move |_| {
            let text = q_text.read().trim().to_string();
            if text.is_empty() {
                return;
            }
            let token = session.read().access_token.clone();
            let node_id = node_id.clone();
            let context_id = context_id.clone();
            let author = session.read().user.as_ref().map(|u| u.display_name.clone());
            q_text.set(String::new());
            spawn(async move {
                let input = graphql::NodesInsertInput {
                    name: author,
                    key: Some(format!("q{}", js_sys::Date::now() as u64)),
                    mime_id: Some("vote/question".to_string()),
                    parent_id: Some(graphql::Uuid(node_id)),
                    context_id: context_id.map(graphql::Uuid),
                    data: Some(graphql::Jsonb(serde_json::json!({ "text": text }))),
                    mutable: Some(false),
                    index: None,
                };
                if graphql::insert_node(token.as_deref(), input).await.is_ok() {
                    crate::session::bump_data_version();
                }
            });
        }
    };

    rsx! {
        // Position text + edit / delete. The comment thread renders at the very
        // end, below the candidate gallery.
        ContentApp { node: node.clone() }
        // Owner-only: open a poll whose options are the candidates.
        StartPollButton { node: node.clone(), path: path.clone() }

        // Candidate gallery (photos from `data.image`).
        if !candidates.is_empty() {
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar", {icon_el("vote/candidate")} }
                    h3 { class: "title-medium", "{t(\"vote.candidates\")}" }
                }
                // Candidates in an M3 carousel: a snapping, horizontally scrollable
                // strip of rounded photo tiles with the name overlaid.
                super::widgets::Carousel { label: t("vote.candidates"),
                    for cand in candidates.iter() {
                        {
                            let mut full = path.clone();
                            full.push(cand.key.clone());
                            let photo = cand
                                .data
                                .as_ref()
                                .and_then(|d| d.0.get("image"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .map(|fid| {
                                    format!(
                                        "{}/files/{fid}?token={}",
                                        crate::nhost::storage_url(),
                                        token.clone().unwrap_or_default()
                                    )
                                });
                            rsx! {
                                Link {
                                    key: "{cand.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "m3-carousel-item",
                                    if let Some(src) = photo {
                                        img { class: "m3-carousel-img", src: "{src}", alt: "{cand.name}" }
                                    } else {
                                        div { class: "m3-carousel-placeholder",
                                            {icon_el("vote/candidate")}
                                        }
                                    }
                                    div { class: "m3-carousel-label", "{cand.name}" }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Questions (numbered), with add + owner/author delete.
        div { class: "card mt-1",
            div { class: "card-header",
                div { class: "avatar", {icon_el("vote/question")} }
                h3 { class: "title-medium", "{t(\"vote.questions\")}" }
            }
            if questions.is_empty() {
                div { class: "card-content",
                    p { class: "body-medium", class: "text-muted", "{t(\"vote.noQuestions\")}" }
                }
            } else {
                div { class: "list",
                    for (n , q) in questions.iter().enumerate() {
                        {
                            let text = q
                                .data
                                .as_ref()
                                .and_then(|d| d.0.get("text"))
                                .and_then(|v| v.as_str())
                                .unwrap_or("")
                                .to_string();
                            let author = q
                                .owner
                                .as_ref()
                                .map(|o| o.display_name.clone())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| q.name.clone());
                            let can_del =
                                q.is_owner.unwrap_or(false) || q.is_context_owner.unwrap_or(false);
                            let qid = q.id.0.clone();
                            rsx! {
                                div { class: "list-item", key: "{q.id.0}",
                                    div { class: "avatar small", "{n + 1}" }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{text}" }
                                        if !author.is_empty() {
                                            div { class: "list-item-secondary", "{author}" }
                                        }
                                    }
                                    if can_del {
                                        button {
                                            class: "btn-icon",
                                            title: "{t(\"common.delete\")}",
                                            onclick: move |_| {
                                                let token = session.read().access_token.clone();
                                                let qid = qid.clone();
                                                spawn(async move {
                                                    if graphql::delete_node(token.as_deref(), &qid)
                                                        .await
                                                        .unwrap_or(false)
                                                    {
                                                        crate::session::bump_data_version();
                                                    }
                                                });
                                            },
                                            span { class: "material-icons", "delete" }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
            if is_auth {
                div { class: "card-content",
                    div { class: "text-field",
                        label { "{t(\"vote.question\")}" }
                        input {
                            r#type: "text",
                            value: "{q_text}",
                            oninput: move |e| q_text.set(e.value()),
                        }
                    }
                    button {
                        class: "btn btn-primary mt-1",
                        disabled: q_text.read().trim().is_empty(),
                        onclick: add_question,
                        "{t(\"common.add\")}"
                    }
                }
            }
        }

        // Polls opened on this position.
        if !polls.is_empty() {
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar", {icon_el("vote/poll")} }
                    h3 { class: "title-medium", "{t(\"mime.vote\")}" }
                }
                div { class: "list",
                    for poll in polls.iter() {
                        {
                            let mut full = path.clone();
                            full.push(poll.key.clone());
                            rsx! {
                                Link {
                                    key: "{poll.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "folder-item",
                                    div { class: "avatar small", {icon_el("vote/poll")} }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{poll.name}" }
                                    }
                                    PollVoteBadge { poll_id: poll.id.0.clone() }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Discussion thread for the position, below the candidate gallery.
        super::comments::CommentSection { node_id: node_id.clone(), context_id: context_id.clone() }
    }
}

/// Control to propose an amendment (`vote/change`) on a policy or change: names
/// it, inserts the node under the parent, and jumps to its editor. Mirrors React
/// AddChangeButton (insert + redirect to `?app=editor`).
#[component]
fn AddChangeButton(node: NodeWithChildren, path: Vec<String>) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let nav = use_navigator();
    let mut open = use_signal(|| false);
    let mut title = use_signal(String::new);
    let node_id = node.id.0.clone();
    let context_id = node.context_id.clone().map(|c| c.0);

    // Proposing an amendment is a member action; the backend enforces who may.
    if !is_auth {
        return rsx! {};
    }

    let submit = {
        let path = path.clone();
        move |_| {
            let name = title.read().trim().to_string();
            if name.is_empty() {
                return;
            }
            let token = session.read().access_token.clone();
            let node_id = node_id.clone();
            let context_id = context_id.clone();
            let path = path.clone();
            spawn(async move {
                let key = crate::components::loader::slugify(&name);
                let input = graphql::NodesInsertInput {
                    name: Some(name),
                    key: Some(key.clone()),
                    mime_id: Some("vote/change".to_string()),
                    parent_id: Some(graphql::Uuid(node_id)),
                    context_id: context_id.map(graphql::Uuid),
                    data: None,
                    mutable: Some(true),
                    index: None,
                };
                if graphql::insert_node(token.as_deref(), input).await.is_ok() {
                    crate::session::bump_data_version();
                    // Redirect to the new amendment's editor to write its body.
                    let mut full = path.clone();
                    full.push(key);
                    nav.push(Route::PathPage {
                        segments: full,
                        app: Some("editor".to_string()),
                    });
                }
            });
        }
    };

    rsx! {
        button {
            class: "btn-icon add-action state-layer",
            title: "{t(\"vote.newAmendment\")}",
            aria_label: "{t(\"vote.newAmendment\")}",
            onclick: move |_| open.set(true),
            span { class: "material-icons", "add" }
        }
        super::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: t("vote.newAmendment"),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: title.read().trim().is_empty(),
                    onclick: submit,
                    "{t(\"common.add\")}"
                }
            },
            div { class: "text-field",
                label { "{t(\"common.title\")}" }
                input {
                    r#type: "text",
                    value: "{title}",
                    oninput: move |e| title.set(e.value()),
                }
            }
        }
    }
}

/// A small vote-count badge for a poll row: the number of `vote/vote` children
/// the viewer can see, fetched via the nodes aggregate.
#[component]
fn PollVoteBadge(poll_id: String) -> Element {
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
    // hideResult (`data.hidden`): a hide-result poll reveals tallies only to the
    // context owner; other viewers see the options without any counts.
    let poll_hidden = node
        .data
        .as_ref()
        .and_then(|d| d.0.get("hidden"))
        .and_then(|v| v.as_bool())
        .unwrap_or(false);
    let show_results = !poll_hidden || node.is_context_owner.unwrap_or(false);

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

    // Live results depend on the poll (node) id and the refresh counter; use
    // use_reactive so they re-run when navigating to a different poll, not only
    // via a keyed remount (unreliable in the web renderer).
    let rev = *refresh.read();
    let n_opts = options.len();

    // Whether the current user has already voted (own votes are visible to them).
    let av_poll = poll_id.clone();
    let av_token = session.read().access_token.clone();
    let av_user = user_id.clone();
    let already_voted = crate::use_data_resource!(|(av_poll, av_token, av_user, rev)| async move {
        let _ = rev;
        let Some(uid) = av_user else { return false };
        graphql::count_user_votes(av_token.as_deref(), &av_poll, &uid)
            .await
            .map(|n| n > 0)
            .unwrap_or(false)
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
                // Owner-only: close the poll (mutable:false) so results show.
                if open && node.is_context_owner.unwrap_or(false) {
                    button {
                        class: "btn-icon",
                        aria_label: "{t(\"poll.stopPoll\")}",
                        title: "{t(\"poll.stopPoll\")}",
                        onclick: {
                            let poll_id = poll_id.clone();
                            move |_| {
                                let token = session.read().access_token.clone();
                                let poll_id = poll_id.clone();
                                spawn(async move {
                                    let _ = graphql::update_node(
                                        token.as_deref(),
                                        &poll_id,
                                        graphql::NodesSetInput {
                                            mutable: Some(false),
                                            ..Default::default()
                                        },
                                    )
                                    .await;
                                    crate::session::bump_data_version();
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
                        p { class: "body-medium", style: "color: var(--md-error, #b3261e);", "{error}" }
                    }
                    // The civic focal action: casting a vote is a democratic act,
                    // so it gets the magenta tertiary emphasis, not plain primary.
                    button {
                        class: "btn btn-cast mt-1",
                        onclick: submit,
                        span { class: "material-icons", "how_to_vote" }
                        " {t(\"vote.castVote\")}"
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
                                        if show_results {
                                            let count = counts.get(i).copied().unwrap_or(0);
                                            let pct = (count * 100).checked_div(total_votes).unwrap_or(0);
                                            let fraction = count as f64 / total_votes.max(1) as f64;
                                            rsx! {
                                                super::widgets::Bar { fraction }
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
                        } else {
                            "{t(\"poll.resultsHidden\")}"
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
fn StartPollButton(node: NodeWithChildren, path: Vec<String>) -> Element {
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
                div { class: "avatar", {icon_el("vote/poll")} }
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
        super::widgets::Dialog {
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
                                    hidden,
                                )
                                .await
                                {
                                    Ok(inserted) => {
                                        crate::session::bump_data_version();
                                        open.set(false);
                                        poll_path.push(inserted.key);
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
                div { class: "body-medium", style: "margin-bottom: 4px;",
                    "{range_label}: {min_vote} to {max_vote}"
                }
                input {
                    r#type: "range",
                    min: "1",
                    max: "{max_range}",
                    value: "{min_vote}",
                    style: "width: 100%;",
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
                    style: "width: 100%; margin-bottom: 8px;",
                    oninput: move |e| {
                        let v: usize = e.value().parse().unwrap_or(1);
                        max_vote.set(v.max(min_vote()));
                    },
                }
            }
            div { class: "list-item switch-row",
                span { class: "switch-row-label", "{t(\"poll.hideResult\")}" }
                Switch {
                    checked: Some(hidden()),
                    on_checked_change: move |v: bool| hidden.set(v),
                }
            }
        }
    }
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
