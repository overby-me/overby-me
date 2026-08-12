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

/// One candidate's photo in the carousel, or the mime placeholder when there is
/// none (or while it loads).
///
/// A component rather than markup inlined in the loop, because fetching the photo
/// takes a hook and a hook cannot be called per iteration. It goes through
/// `use_file_object_url`, which sends the session token as a header and hands
/// back a `blob:` URL: an `<img src>` cannot send that header itself, and the
/// storage service reads it nowhere else.
#[component]
fn CandidatePhoto(file_id: String, name: String) -> Element {
    let photo = crate::components::loader::use_file_object_url(file_id);
    rsx! {
        if let Some(src) = photo {
            img {
                class: "m3-carousel-img",
                src: "{src}",
                alt: "{name}",
                loading: "lazy",
                decoding: "async",
            }
        } else {
            div { class: "m3-carousel-placeholder", {icon_el("vote/candidate")} }
        }
    }
}

/// A candidate shown optimistically the instant it is added, before the insert is
/// confirmed. Reconciled by `key` against the fetched candidates. It carries no
/// photo: one is attached later, in the editor that adding a candidate opens.
#[derive(Clone, PartialEq)]
struct PendingCandidate {
    key: String,
    name: String,
}

/// PositionApp — a `vote/position` (candidate election): the position text (with
/// its edit/delete affordances), a candidate photo gallery, and any polls.
///
/// React's PositionApp also carried a QuestionList; this one does not. Existing
/// `vote/question` nodes still render on their own (loader's TextNode arm), they
/// are simply no longer listed or created here.
#[component]
pub fn PositionApp(node: NodeWithChildren, path: Vec<String>) -> Element {
    let session = use_session();
    let is_ctx_owner = node.is_context_owner.unwrap_or(false);
    let children = visible_sorted(&node.children);
    let children = &children;

    let candidates: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/candidate"))
        .collect();
    // Owned here so the sheet row and the dialog, which must render in different
    // parts of the tree, still open and close together.
    let poll_open = use_signal(|| false);

    let polls: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/poll"))
        .collect();

    let node_id = node.id.0.clone();
    let context_id = node.context_id.clone().map(|c| c.0);
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
    // ...and only while candidature is open: `attachable` is the owner's lock on
    // adding children, which ContentApp's tools sheet toggles for a position the
    // same way FolderApp's does for a folder's motions. Not for the chair who set
    // it: the insert rule exempts a context owner (`migrations/0015`) precisely so
    // a late candidature can be entered by hand without reopening the position.
    let can_add_candidate =
        (*can_add_candidate_res.read()).unwrap_or(false) && (node.attachable || is_ctx_owner);

    // The candidate gallery (photos from `data.image`), rendered INSIDE the
    // position's own card rather than as a second one below it. A position rarely
    // carries any text, so two cards meant an empty card announcing "no content"
    // stacked above the only thing on the page. Shown, with an empty state, to
    // members who can add a candidature so they can add the first one.
    let candidate_section = rsx! {
        if !candidates.is_empty() || !pending_cand_shown.is_empty() || can_add_candidate {
            div { class: "card-header card-header-section",
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
                    // The orb empty state every other card uses (policy's own
                    // "no amendments" sits right beside this one). It had a bare
                    // subheader line, which belongs to the drawer and home lists,
                    // not inside a card.
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            {icon_el("vote/candidate")}
                        }
                        p { class: "empty-state-body", "{t(\"vote.noCandidates\")}" }
                    }
                }
                // Candidates in an M3 carousel: a snapping, horizontally scrollable
                // strip of rounded photo tiles with the name overlaid.
                if !candidates.is_empty() || !pending_cand_shown.is_empty() {
                    crate::components::widgets::Carousel { label: t("vote.candidates"),
                    for cand in candidates.iter() {
                        {
                            let mut full = path.clone();
                            full.push(cand.key.clone());
                            let photo_id = cand
                                .data
                                .as_ref()
                                .and_then(|d| d.0.get("image"))
                                .and_then(|v| v.as_str())
                                .filter(|s| !s.is_empty())
                                .unwrap_or_default()
                                .to_string();
                            rsx! {
                                Link {
                                    key: "{cand.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "m3-carousel-item",
                                    CandidatePhoto { file_id: photo_id, name: cand.name.clone() }
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
    };

    rsx! {
        // The position's text and its candidates as ONE card (see
        // `candidate_section` above). The comment thread renders at the very end,
        // below the polls.
        // Opening a poll is a chair's action and rides in the tools sheet's
        // Meeting group. It used to be a card between the candidate gallery and
        // the polls, which put a permanent "New poll" heading on every position
        // whether or not one had ever been opened.
        //
        // Row and dialog are separated on purpose: the sheet is transformed, so
        // anything `position: fixed` inside it is clipped to the sheet (see
        // `StartPollButton`). The dialog therefore renders out here.
        ContentApp {
            node: node.clone(),
            extra: candidate_section,
            meeting_actions: rsx! {
                StartPollButton { node: node.clone(), open: poll_open }
            },
        }
        StartPollDialog { node: node.clone(), path: path.clone(), open: poll_open }

        // Polls opened on this position.
        if !polls.is_empty() {
            div { class: "card app-card mt-1",
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
            // The optimistic card is keyed on the plain slug, which is what the
            // insert asks for first; they differ only if the name is taken, and
            // the navigation below then uses the key the server assigned.
            let key = crate::components::loader::slug_base(&cname);
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
                    name: Some(cname.clone()),
                    // Assigned by insert_node_named.
                    key: None,
                    mime_id: Some("vote/candidate".to_string()),
                    parent_id: Some(model::Uuid(parent_id)),
                    context_id: context_id.map(model::Uuid),
                    // The photo is set in the editor this lands in (`data.image`).
                    data: None,
                    mutable: Some(true),
                    index: None,
                    created_at: None,
                };
                match graphql::insert_node_named(token.as_deref(), input, &cname).await {
                    Ok(inserted) => {
                        crate::session::bump_data_version();
                        // Land in the new candidate's editor so its text can be
                        // written now, as adding an amendment does.
                        let mut full = path.clone();
                        full.push(inserted.map(|n| n.key).unwrap_or(key));
                        nav.push(Route::PathPage {
                            segments: full,
                            app: Some("editor".to_string()),
                        });
                    }
                    Err(e) => {
                        pending.write().retain(|p| p.key != key);
                        crate::errors::log_handled("add candidate failed", &e);
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
            onclick: move |_| {
                // Standing for a position is normally standing yourself, so open
                // with the signed-in member's display name already filled in. Seeded
                // here rather than at mount, so a session that resolves late still
                // prefills, and only when the field is empty, so a name typed and
                // cancelled survives reopening.
                let blank = name.read().trim().is_empty();
                if blank {
                    let display_name = session
                        .read()
                        .user
                        .as_ref()
                        .map(|u| u.display_name.clone())
                        .unwrap_or_default();
                    name.set(display_name);
                }
                open.set(true);
            },
            span { class: "material-icons", "add" }
        }
        crate::components::widgets::Dialog {
            open: open(),
            on_dismiss: move |_| open.set(false),
            headline: t("vote.addCandidate"),
            // A form, so it takes the screen on a phone (see widgets::Dialog).
            form: true,
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
    // `attachable` is the owner's lock on adding children, which ContentApp's
    // tools sheet toggles for a motion the same way FolderApp does for a folder:
    // once amendments are closed, the affordance goes with them. For members. A
    // context owner keeps it, the way the insert rule does (`migrations/0015`).
    if !is_auth || (!node.attachable && !node.is_context_owner.unwrap_or(false)) {
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
                let input = model::NodesInsertInput {
                    name: Some(name.clone()),
                    key: None,
                    mime_id: Some("vote/change".to_string()),
                    parent_id: Some(model::Uuid(node_id)),
                    context_id: context_id.map(model::Uuid),
                    data: None,
                    mutable: Some(true),
                    index: None,
                    created_at: None,
                };
                match graphql::insert_node_named(token.as_deref(), input, &name).await {
                    Ok(inserted) => {
                        crate::session::bump_data_version();
                        // Redirect to the new amendment's editor to write its body.
                        // The key comes back from the insert, being whatever was free.
                        let Some(inserted) = inserted else { return };
                        let mut full = path.clone();
                        full.push(inserted.key);
                        nav.push(Route::PathPage {
                            segments: full,
                            app: Some("editor".to_string()),
                        });
                    }
                    Err(e) => {
                        // Close the dialog and surface the error instead of leaving
                        // the user staring at an open dialog with no feedback.
                        crate::errors::log_handled("add amendment failed", &e);
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
            // A form, so it takes the screen on a phone (see widgets::Dialog).
            form: true,
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
