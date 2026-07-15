use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

use crate::components::content::ContentApp;
use crate::components::loader::{icon_el, visible_sorted};

use super::*;

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
                // Only clear the field once the question is actually stored, so a
                // failed insert does not silently discard the typed text.
                match graphql::insert_node(token.as_deref(), input).await {
                    Ok(_) => {
                        q_text.set(String::new());
                        crate::session::bump_data_version();
                    }
                    Err(e) => {
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

        // Candidate gallery (photos from `data.image`). Shown to authed users even
        // when empty, so a member can add the first candidate (mirrors add-question,
        // which is likewise member-insertable).
        if !candidates.is_empty() || is_auth {
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar small", {icon_el("vote/candidate")} }
                    h3 { class: "title-medium", "{t(\"vote.candidates\")}" }
                    div { class: "flex-grow" }
                    if is_auth {
                        AddCandidateButton {
                            parent_id: node_id.clone(),
                            context_id: context_id.clone(),
                        }
                    }
                }
                if candidates.is_empty() {
                    p { class: "body-medium text-muted", style: "padding: 0 16px 12px;",
                        "{t(\"vote.noCandidates\")}"
                    }
                }
                // Candidates in an M3 carousel: a snapping, horizontally scrollable
                // strip of rounded photo tiles with the name overlaid.
                if !candidates.is_empty() {
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
        crate::components::comments::CommentSection { node_id: node_id.clone(), context_id: context_id.clone() }
    }
}

/// Owner control to add a candidate (`vote/candidate`) to an election: a name and
/// an optional photo (uploaded to NHost storage, stored as `data.image`). This is
/// the "inline add-candidate" the position view was missing (the photo upload it
/// depended on already exists as `nhost::upload_file`).
#[component]
fn AddCandidateButton(parent_id: String, context_id: Option<String>) -> Element {
    let session = use_session();
    let mut open = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut photo_id = use_signal(|| Option::<String>::None);
    let mut uploading = use_signal(|| false);
    let mut busy = use_signal(|| false);

    // Upload the chosen photo immediately; the Add button attaches its id.
    let on_pick = move |evt: FormEvent| {
        let Some(fd) = evt.files().into_iter().next() else {
            return;
        };
        let fname = fd.name();
        let ctype = fd.content_type().unwrap_or_default();
        let token = session.read().access_token.clone();
        uploading.set(true);
        photo_id.set(None);
        spawn(async move {
            match fd.read_bytes().await {
                Ok(bytes) => {
                    match crate::nhost::upload_file(
                        token.as_deref(),
                        bytes.to_vec(),
                        &fname,
                        &ctype,
                    )
                    .await
                    {
                        Ok(up) => photo_id.set(Some(up.id)),
                        Err(e) => crate::snackbar::show_snackbar(&format!(
                            "{}: {e}",
                            t("error.somethingWentWrong")
                        )),
                    }
                }
                Err(_) => crate::snackbar::show_snackbar(&t("error.somethingWentWrong")),
            }
            uploading.set(false);
        });
    };

    let submit = {
        let parent_id = parent_id.clone();
        let context_id = context_id.clone();
        move |_| {
            let cname = name.read().trim().to_string();
            if cname.is_empty() || *busy.read() || *uploading.read() {
                return;
            }
            let token = session.read().access_token.clone();
            let parent_id = parent_id.clone();
            let context_id = context_id.clone();
            let img = photo_id.read().clone();
            busy.set(true);
            spawn(async move {
                let key = format!(
                    "{}-{}",
                    crate::components::loader::slugify(&cname),
                    js_sys::Date::now() as u64
                );
                let data = img.map(|fid| graphql::Jsonb(serde_json::json!({ "image": fid })));
                let input = graphql::NodesInsertInput {
                    name: Some(cname),
                    key: Some(key),
                    mime_id: Some("vote/candidate".to_string()),
                    parent_id: Some(graphql::Uuid(parent_id)),
                    context_id: context_id.map(graphql::Uuid),
                    data,
                    mutable: Some(true),
                    index: None,
                };
                match graphql::insert_node(token.as_deref(), input).await {
                    Ok(_) => {
                        crate::session::bump_data_version();
                        busy.set(false);
                        open.set(false);
                        name.set(String::new());
                        photo_id.set(None);
                    }
                    Err(e) => {
                        busy.set(false);
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
                    disabled: name.read().trim().is_empty() || *busy.read() || *uploading.read(),
                    onclick: submit,
                    "{t(\"common.add\")}"
                }
            },
            div { class: "text-field",
                label { "{t(\"member.name\")}" }
                input {
                    r#type: "text",
                    maxlength: "{crate::components::editor::NODE_NAME_MAXLEN}",
                    value: "{name}",
                    oninput: move |e| name.set(e.value()),
                }
            }
            div { class: "text-field mt-2",
                label { "{t(\"vote.candidatePhoto\")}" }
                input { r#type: "file", accept: "image/*", onchange: on_pick }
                if *uploading.read() {
                    span { class: "body-small text-muted", "{t(\"vote.sending\")}" }
                } else if photo_id.read().is_some() {
                    span { class: "body-small", "{t(\"vote.photoReady\")}" }
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
