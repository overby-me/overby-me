use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;
use crate::i18n::t;
use crate::route::Route;

use crate::components::content::ContentApp;
use crate::components::loader::{icon_el, mime_icon, visible_sorted};

use super::*;

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
    let is_ctx_owner = node.is_context_owner.unwrap_or(false);

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
                            rsx! {
                                div { key: "{item.id.0}", class: "amendment-item",
                                    div {
                                        class: "stack stack-h",
                                        style: "align-items: center; gap: 8px;",
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
                                    }
                                    if has_body {
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
                                    style: "align-items: center;",
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
