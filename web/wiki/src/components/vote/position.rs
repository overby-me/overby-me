use crate::model;
use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;

use crate::components::content::ContentApp;
use crate::components::loader::{icon_el, visible_sorted};

use super::*;

/// A candidate shown optimistically the instant it is added, before the insert is
/// confirmed. Reconciled by `key` against the fetched candidates. It carries no
/// photo: one is attached later, in the editor that adding a candidate opens.
#[derive(Clone, PartialEq)]
struct PendingCandidate {
    key: String,
    name: String,
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
    let is_ctx_owner = node.is_context_owner.unwrap_or(false);
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
    // Optimistic questions: (key, text) rows shown at once, reconciled by key.
    let mut pending_q = use_signal(Vec::<(String, String)>::new);
    // Optimistic candidates (owned here; AddCandidateButton pushes into it), shown in
    // the carousel at once and reconciled by key against the fetched candidates.
    let pending_cand = use_signal(Vec::<PendingCandidate>::new);
    let cand_keys: std::collections::HashSet<String> =
        candidates.iter().map(|c| c.key.clone()).collect();
    let pending_cand_shown = crate::components::optimistic::reconcile_by_key(
        &pending_cand.read(),
        |p| p.key.as_str(),
        &cand_keys,
    );
    // Drop optimistic questions once the real node (same key) has come back.
    let q_keys: std::collections::HashSet<String> =
        questions.iter().map(|q| q.key.clone()).collect();
    let pending_q_shown = crate::components::optimistic::reconcile_by_key(
        &pending_q.read(),
        |p| p.0.as_str(),
        &q_keys,
    );

    // Anyone who is part of this group/event (a member) may add a candidature —
    // vote/candidate is member-insertable. Ask the permission resolver what the
    // current user can actually insert here, so the button shows to members (and
    // owners), not to signed-in non-members (mirrors the comment composer's gate).
    let cand_nid = node_id.clone();
    let cand_tok = session.read().access_token.clone();
    let can_add_candidate_res = crate::use_data_resource!(|(cand_nid, cand_tok)| async move {
        if cand_tok.is_none() {
            return false;
        }
        graphql::node_insert_mimes(cand_tok.as_deref(), &cand_nid)
            .await
            .iter()
            .any(|m| m == "vote/candidate")
    });
    let can_add_candidate = (*can_add_candidate_res.read()).unwrap_or(false);

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
            let key = format!("q{}", js_sys::Date::now() as u64);
            // Optimistic: show the question now (a muted pending row) and clear the
            // input; reconciled by key, restored on error.
            pending_q.write().push((key.clone(), text.clone()));
            q_text.set(String::new());
            spawn(async move {
                let input = model::NodesInsertInput {
                    name: author,
                    key: Some(key.clone()),
                    mime_id: Some("vote/question".to_string()),
                    parent_id: Some(model::Uuid(node_id)),
                    context_id: context_id.map(model::Uuid),
                    data: Some(model::Jsonb(serde_json::json!({ "text": text }))),
                    mutable: Some(false),
                    index: None,
                };
                match graphql::insert_node(token.as_deref(), input).await {
                    Ok(_) => crate::session::bump_data_version(),
                    Err(e) => {
                        // Roll back the optimistic row and restore the typed text.
                        pending_q.write().retain(|p| p.0 != key);
                        q_text.set(text);
                        log::error!("add question failed: {e}");
                        crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                    }
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

        // Candidate gallery (photos from `data.image`). Shown, with an empty state,
        // to members who can add a candidature so they can add the first one.
        if !candidates.is_empty() || !pending_cand_shown.is_empty() || can_add_candidate {
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar small", {icon_el("vote/candidate")} }
                    h3 { class: "title-medium", "{t(\"vote.candidates\")}" }
                    div { class: "flex-grow" }
                    if can_add_candidate {
                        AddCandidateButton {
                            parent_id: node_id.clone(),
                            context_id: context_id.clone(),
                            path: path.clone(),
                            pending: pending_cand,
                        }
                    }
                }
                if candidates.is_empty() && pending_cand_shown.is_empty() {
                    p { class: "body-medium list-subheader", "{t(\"vote.noCandidates\")}" }
                }
                // Candidates in an M3 carousel: a snapping, horizontally scrollable
                // strip of rounded photo tiles with the name overlaid.
                if !candidates.is_empty() || !pending_cand_shown.is_empty() {
                    crate::components::widgets::Carousel { label: t("vote.candidates"),
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
                                    crate::backend_api::file_url(
                                        fid,
                                        &token.clone().unwrap_or_default(),
                                    )
                                });
                            rsx! {
                                Link {
                                    key: "{cand.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "m3-carousel-item",
                                    if let Some(src) = photo {
                                        img {
                                            class: "m3-carousel-img",
                                            src: "{src}",
                                            alt: "{cand.name}",
                                            loading: "lazy",
                                            decoding: "async",
                                            // The src carries the nhost ?token=; keep it out of the Referer.
                                            referrerpolicy: "no-referrer",
                                        }
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
                    // Optimistic candidate cards (muted), dropped once confirmed.
                    // Always the placeholder: a photo is added afterwards, in the
                    // editor, so a just-added candidate never has one yet.
                    for p in pending_cand_shown.iter() {
                        div { key: "{p.key}", class: "m3-carousel-item is-pending",
                            div { class: "m3-carousel-placeholder",
                                {icon_el("vote/candidate")}
                            }
                            div { class: "m3-carousel-label", "{p.name}" }
                        }
                    }
                    }
                }
            }
        }

        // Questions (numbered), with add + owner/author delete.
        div { class: "card mt-1",
            div { class: "card-header",
                div { class: "avatar small", {icon_el("vote/question")} }
                h3 { class: "title-medium", "{t(\"vote.questions\")}" }
            }
            if questions.is_empty() && pending_q_shown.is_empty() {
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
                            // Treat an owner with a blank display name as free-text
                            // (no identity), so the shown name and the linked profile
                            // never diverge; a real free-text author has no owner.
                            let owner = q.owner.as_ref().filter(|o| !o.display_name.is_empty());
                            let author = owner
                                .map(|o| o.display_name.clone())
                                .unwrap_or_else(|| q.name.clone());
                            let author_id = owner.map(|o| o.id.0.clone());
                            let author_avatar =
                                owner.map(|o| o.avatar_url.clone()).unwrap_or_default();
                            // Questions are part of the record and carry no delete
                            // affordance (comments are the deletable discussion type).
                            rsx! {
                                div { class: "list-item", key: "{q.id.0}",
                                    div { class: "avatar small", "{n + 1}" }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{text}" }
                                        if !author.is_empty() {
                                            crate::components::loader::UserPopover {
                                                name: author.clone(),
                                                avatar_url: author_avatar.clone(),
                                                user_id: author_id.clone(),
                                                div { class: "list-item-secondary", "{author}" }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    // Optimistic questions (muted "sending" rows), dropped on confirm.
                    for (i , p) in pending_q_shown.iter().enumerate() {
                        div { key: "{p.0}", class: "list-item is-pending",
                            div { class: "avatar small", "{questions.len() + i + 1}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{p.1}" }
                                div { class: "list-item-secondary", "{t(\"vote.sending\")}" }
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
                    div { class: "avatar small", {icon_el("vote/poll")} }
                    h3 { class: "title-medium", "{t(\"mime.vote\")}" }
                }
                div { class: "list",
                    for poll in polls.iter() {
                        {
                            let mut full = path.clone();
                            full.push(poll.key.clone());
                            rsx! {
                                div {
                                    key: "{poll.id.0}",
                                    class: "stack stack-h",
                                    Link {
                                        to: Route::PathPage { segments: full, app: None },
                                        class: "folder-item flex-grow",
                                        div { class: "avatar small", {icon_el("vote/poll")} }
                                        div { class: "list-item-text",
                                            div { class: "list-item-primary", "{poll.name}" }
                                        }
                                        PollVoteBadge { poll_id: poll.id.0.clone() }
                                    }
                                    if is_ctx_owner {
                                        DeletePollButton { poll_id: poll.id.0.clone() }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Discussion thread for the position, below the candidate gallery.
        crate::components::comments::CommentSection { node_id: node_id.clone(), context_id: context_id.clone() }
    }
}

/// Owner control to add a candidate (`vote/candidate`) to an election: a name and
/// an optional photo (uploaded to NHost storage, stored as `data.image`). This is
/// the "inline add-candidate" the position view was missing (the photo upload it
/// depended on already exists as `nhost::upload_file`).
///
/// Adding one lands in the new candidate's editor, so its text can be written
/// straight away, the same way [`AddChangeButton`] opens a new amendment.
#[component]
fn AddCandidateButton(
    parent_id: String,
    context_id: Option<String>,
    /// The position's own path; the new candidate's key is appended to it to
    /// route into its editor.
    path: Vec<String>,
    /// Optimistic candidates owned by PositionApp: this button pushes the new
    /// candidate here (shown at once), reconciled/rolled back there and here.
    mut pending: Signal<Vec<PendingCandidate>>,
) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let mut open = use_signal(|| false);
    let mut name = use_signal(String::new);

    let submit = {
        let parent_id = parent_id.clone();
        let context_id = context_id.clone();
        let path = path.clone();
        move |_| {
            let cname = name.read().trim().to_string();
            if cname.is_empty() {
                return;
            }
            let token = session.read().access_token.clone();
            let parent_id = parent_id.clone();
            let context_id = context_id.clone();
            let path = path.clone();
            let key = format!(
                "{}-{}",
                crate::components::loader::slugify(&cname),
                js_sys::Date::now() as u64
            );
            // Optimistic: show the candidate card now and close the dialog; reconciled
            // by key against the fetched candidates, removed on error.
            pending.write().push(PendingCandidate {
                key: key.clone(),
                name: cname.clone(),
            });
            open.set(false);
            name.set(String::new());
            spawn(async move {
                let input = model::NodesInsertInput {
                    name: Some(cname),
                    key: Some(key.clone()),
                    mime_id: Some("vote/candidate".to_string()),
                    parent_id: Some(model::Uuid(parent_id)),
                    context_id: context_id.map(model::Uuid),
                    // The photo is set in the editor this lands in (`data.image`).
                    data: None,
                    mutable: Some(true),
                    index: None,
                };
                match graphql::insert_node(token.as_deref(), input).await {
                    Ok(_) => {
                        crate::session::bump_data_version();
                        // Land in the new candidate's editor so its text can be
                        // written now, as adding an amendment does.
                        let mut full = path.clone();
                        full.push(key);
                        nav.push(Route::PathPage {
                            segments: full,
                            app: Some("editor".to_string()),
                        });
                    }
                    Err(e) => {
                        pending.write().retain(|p| p.key != key);
                        log::error!("add candidate failed: {e}");
                        crate::snackbar::show_snackbar(&e);
                    }
                }
            });
        }
    };

    rsx! {
        button {
            class: "btn-icon add-action state-layer",
            title: "{t(\"vote.addCandidate\")}",
            aria_label: "{t(\"vote.addCandidate\")}",
            onclick: move |_| open.set(true),
            span { class: "material-icons", "add" }
        }
        crate::components::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: t("vote.addCandidate"),
            icon: "person".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| open.set(false),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: name.read().trim().is_empty(),
                    onclick: submit,
                    "{t(\"common.add\")}"
                }
            },
            // Only the name: the photo (and the text) are set in the editor this
            // opens, so a candidate is never half-created behind a failed upload.
            div { class: "text-field",
                label { "{t(\"member.name\")}" }
                input {
                    r#type: "text",
                    maxlength: "{crate::components::editor::NODE_NAME_MAXLEN}",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                }
            }
        }
    }
}

/// Control to propose an amendment (`vote/change`) on a policy or change: names
/// it, inserts the node under the parent, and jumps to its editor. Mirrors React
/// AddChangeButton (insert + redirect to `?app=editor`).
#[component]
pub(super) fn AddChangeButton(node: NodeWithChildren, path: Vec<String>) -> Element {
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
                let input = model::NodesInsertInput {
                    name: Some(name),
                    key: Some(key.clone()),
                    mime_id: Some("vote/change".to_string()),
                    parent_id: Some(model::Uuid(node_id)),
                    context_id: context_id.map(model::Uuid),
                    data: None,
                    mutable: Some(true),
                    index: None,
                };
                match graphql::insert_node(token.as_deref(), input).await {
                    Ok(_) => {
                        crate::session::bump_data_version();
                        // Redirect to the new amendment's editor to write its body.
                        let mut full = path.clone();
                        full.push(key);
                        nav.push(Route::PathPage {
                            segments: full,
                            app: Some("editor".to_string()),
                        });
                    }
                    Err(e) => {
                        // Close the dialog and surface the error instead of leaving
                        // the user staring at an open dialog with no feedback.
                        log::error!("add amendment failed: {e}");
                        open.set(false);
                        crate::snackbar::show_snackbar(&e);
                    }
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
        crate::components::widgets::Dialog {
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
