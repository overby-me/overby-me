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
                                        {crate::components::loader::node_avatar("vote/change", &item.name, Some(n))}
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
