use dioxus::prelude::*;

use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;

use crate::components::content::ContentApp;
use crate::components::loader::{icon_el, mime_icon, visible_sorted};

use super::*;

/// PolicyApp — document with comments, changes, and polls. Sub-changes form a
/// tree: each `vote/change` row links into its own PolicyApp, so the whole
/// amendment tree is browsable (#112).
#[component]
pub fn PolicyApp(node: NodeWithChildren, path: Vec<String>) -> Element {
    /// The amendment "Show changes" word-diff is hidden for now: against the
    /// current motion bodies it produces noisy, unhelpful diffs. Flip to
    /// re-enable the toggle — the diff view and plumbing stay wired.
    const AMENDMENT_DIFF_ENABLED: bool = false;
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
    let is_ctx_owner = node.is_context_owner.unwrap_or(false);

    // The motion's own body as plain text, to diff each amendment against. Which
    // amendment (if any) currently shows its diff, keyed by id.
    let motion_text = node
        .data
        .as_ref()
        .map(|d| crate::components::content::slate_plain_text(&d.0))
        .unwrap_or_default();
    let mut diff_open = use_signal(|| Option::<String>::None);
    // Owned here so the sheet row and the dialog, which must render in different
    // parts of the tree, still open and close together.
    let poll_open = use_signal(|| false);

    rsx! {
        // Main content. The comment thread renders at the end, below the
        // amendments and polls.
        // Opening a poll on this motion is a chair's action, so it rides in the
        // tools sheet's Meeting group rather than standing as its own card. The
        // polls it makes are the section further down, which shows itself only
        // when there is something in it.
        //
        // Row and dialog are separated on purpose: the sheet is transformed, so
        // anything `position: fixed` inside it is clipped to the sheet (see
        // `StartPollButton`). The dialog therefore renders out here.
        ContentApp {
            node: node.clone(),
            meeting_actions: rsx! {
                StartPollButton { node: node.clone(), open: poll_open }
            },
        }
        StartPollDialog { node: node.clone(), path: path.clone(), open: poll_open }

        // Amendments — always shown so its create action (in the header) has a
        // home; the body shows an empty state until the first amendment lands.
        div { class: "card app-card mt-1",
            div { class: "card-header",
                div { class: "avatar small", {icon_el("vote/change")} }
                h3 { class: "title-medium", "{t(\"vote.amendments\")}" }
                div { class: "flex-grow" }
                // Propose a new amendment (redirects to its editor).
                AddChangeButton { node: node.clone(), path: path.clone() }
            }
            if amendments.is_empty() {
                // DESIGN: the expressive orb empty state, matching the other
                // "no X" states, instead of a plain muted line.
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "difference" }
                    }
                    p { class: "empty-state-body", "{t(\"vote.noAmendments\")}" }
                }
            } else {
                div { class: "list",
                    for (n , item) in amendments.iter().enumerate() {
                        {
                            let mut full = path.clone();
                            full.push(item.key.clone());
                            // Author byline: the creating user (a blank display name
                            // means free-text, so it is treated as no identity).
                            let owner = item.owner.as_ref().filter(|o| !o.display_name.is_empty());
                            let author = owner.map(|o| o.display_name.clone());
                            let author_id = owner.map(|o| o.id.0.clone());
                            let author_avatar = owner.map(|o| o.avatar_url.clone()).unwrap_or_default();
                            // Inline body preview, so an amendment can be read (and
                            // its author seen) without opening it — matching the old
                            // wiki's expandable ChangeList row.
                            let body = item.data.as_ref().map(|d| d.0.clone());
                            let has_body = crate::components::content::has_rich_content(body.as_ref());
                            // The amendment as plain text, for the diff against the
                            // motion. Diff is offered only when both sides have text
                            // and neither is too long (see `diffable`).
                            let amendment_text = body
                                .as_ref()
                                .map(crate::components::content::slate_plain_text)
                                .unwrap_or_default();
                            let can_diff =
                                AMENDMENT_DIFF_ENABLED && diffable(&motion_text, &amendment_text);
                            let this_id = item.id.0.clone();
                            let is_open = can_diff && diff_open() == Some(this_id.clone());
                            rsx! {
                                div { key: "{item.id.0}", class: "amendment-item",
                                    div {
                                        class: "stack stack-h",
                                        div { class: "avatar small",
                                            {crate::components::loader::node_avatar("vote/change", &item.name, Some(n))}
                                        }
                                        div { class: "list-item-text flex-grow",
                                            Link {
                                                to: Route::PathPage { segments: full, app: None },
                                                div { class: "list-item-primary", "{item.name}" }
                                            }
                                            if let Some(a) = author.clone() {
                                                crate::components::loader::UserPopover {
                                                    name: a.clone(),
                                                    avatar_url: author_avatar.clone(),
                                                    user_id: author_id.clone(),
                                                    div { class: "list-item-secondary", "{a}" }
                                                }
                                            }
                                        }
                                        // Toggle a word-level diff against the motion.
                                        if can_diff {
                                            button {
                                                class: "btn btn-text btn-sm amendment-diff-toggle",
                                                onclick: {
                                                    let this_id = this_id.clone();
                                                    move |_| {
                                                        let open = diff_open() == Some(this_id.clone());
                                                        diff_open.set(if open { None } else { Some(this_id.clone()) });
                                                    }
                                                },
                                                span { class: "material-icons", "difference" }
                                                if is_open {
                                                    " {t(\"vote.hideDiff\")}"
                                                } else {
                                                    " {t(\"vote.showDiff\")}"
                                                }
                                            }
                                        }
                                    }
                                    if is_open {
                                        div { class: "amendment-preview",
                                            AmendmentDiffView {
                                                original: motion_text.clone(),
                                                proposed: amendment_text.clone(),
                                            }
                                        }
                                    } else if has_body {
                                        div { class: "amendment-preview",
                                            crate::components::content::SlateRenderer { data: body }
                                        }
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

        // Discussion thread for the policy/change, below its amendments/polls.
        crate::components::comments::CommentSection { node_id: node_id.clone(), context_id: context_id.clone() }

        // Other children (comments, questions)
        if !comments.is_empty() {
            div { class: "card app-card mt-1",
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

/// One token of a word-level diff.
#[derive(Clone, Copy, PartialEq)]
enum DiffTag {
    Equal,
    Insert,
    Delete,
}

/// A hand-rolled word-level LCS diff (no diff crate, to avoid churning the Nix
/// dependency hashes). Returns the amendment's words tagged Equal / Insert (added
/// vs the motion) / Delete (removed). Whitespace-delimited tokens: coarse but
/// enough to make an amendment's change legible. Bounded by the caller, which
/// only diffs reasonably-sized bodies (`MAX_DIFF_WORDS`).
fn word_diff(old: &str, new: &str) -> Vec<(DiffTag, String)> {
    let a: Vec<&str> = old.split_whitespace().collect();
    let b: Vec<&str> = new.split_whitespace().collect();
    let (n, m) = (a.len(), b.len());
    // LCS-length table (row-major, (n+1) x (m+1)).
    let mut lcs = vec![vec![0usize; m + 1]; n + 1];
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            lcs[i][j] = if a[i] == b[j] {
                lcs[i + 1][j + 1] + 1
            } else {
                lcs[i + 1][j].max(lcs[i][j + 1])
            };
        }
    }
    // Backtrack into a Delete/Insert/Equal token stream.
    let mut out = Vec::new();
    let (mut i, mut j) = (0, 0);
    while i < n && j < m {
        if a[i] == b[j] {
            out.push((DiffTag::Equal, a[i].to_string()));
            i += 1;
            j += 1;
        } else if lcs[i + 1][j] >= lcs[i][j + 1] {
            out.push((DiffTag::Delete, a[i].to_string()));
            i += 1;
        } else {
            out.push((DiffTag::Insert, b[j].to_string()));
            j += 1;
        }
    }
    while i < n {
        out.push((DiffTag::Delete, a[i].to_string()));
        i += 1;
    }
    while j < m {
        out.push((DiffTag::Insert, b[j].to_string()));
        j += 1;
    }
    out
}

/// The largest body pair (words on either side) the O(n*m) diff will run on, so a
/// very long motion cannot wedge the tab. Past this the toggle is not offered.
pub(crate) const MAX_DIFF_WORDS: usize = 3000;

/// Whether a motion/amendment text pair is small enough to diff.
pub(crate) fn diffable(original: &str, proposed: &str) -> bool {
    !original.trim().is_empty()
        && !proposed.trim().is_empty()
        && original.split_whitespace().count() <= MAX_DIFF_WORDS
        && proposed.split_whitespace().count() <= MAX_DIFF_WORDS
}

/// A word-level diff of an amendment against the original motion text: inserted
/// words highlighted, removed words struck through, so what an amendment changes
/// is legible inline without opening it and reading both side by side.
#[component]
fn AmendmentDiffView(original: String, proposed: String) -> Element {
    let tokens = word_diff(&original, &proposed);
    rsx! {
        p { class: "amendment-diff-text",
            for (i , (tag , word)) in tokens.iter().enumerate() {
                span {
                    key: "{i}",
                    class: match tag {
                        DiffTag::Insert => "diff-ins",
                        DiffTag::Delete => "diff-del",
                        DiffTag::Equal => "diff-eq",
                    },
                    "{word} "
                }
            }
        }
    }
}
