use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;
use crate::i18n::t;
use crate::session::use_session;

use super::content::ContentApp;
use super::loader::{mime_icon, visible_sorted};

/// VoteApp — voting interface for active polls
#[component]
pub fn VoteApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let children = &node.children;

    // Find active poll among children
    let polls: Vec<_> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("vote/poll"))
        .collect();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "{mime_icon(\"vote/poll\")}" }
                h3 { class: "title-medium", "{t(\"mime.vote\")}" }
            }
            div { class: "card-content",
                if !is_auth {
                    p { class: "body-large", "{t(\"vote.noVotingRight\")}" }
                } else if polls.is_empty() {
                    p { class: "body-large", "{t(\"vote.noVoteNow\")}" }
                } else {
                    // Show polls
                    for poll in polls.iter() {
                        div { class: "list-item", key: "{poll.id.0}",
                            div { class: "avatar small", "{mime_icon(\"vote/poll\")}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{poll.name}" }
                            }
                        }
                    }
                    p { class: "body-medium mt-1",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"vote.castVote\")}"
                    }
                }
            }
        }
    }
}

/// PolicyApp — document with comments, changes, and polls
#[component]
pub fn PolicyApp(node: NodeWithChildren) -> Element {
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

    let comments: Vec<_> = children
        .iter()
        .filter(|c| {
            !matches!(
                c.mime_id.as_deref(),
                Some("vote/poll") | Some("vote/change")
            )
        })
        .collect();

    rsx! {
        // Main content
        ContentApp { node: node.clone() }

        // Amendments
        if !amendments.is_empty() {
            div { class: "card mt-1",
                div { class: "card-header",
                    div { class: "avatar", "\u{1F4DD}" }
                    h3 { class: "title-medium", "{t(\"vote.amendments\")}" }
                }
                div { class: "list",
                    for item in amendments.iter() {
                        div { class: "list-item", key: "{item.id.0}",
                            div { class: "avatar small", "{mime_icon(\"vote/change\")}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{item.name}" }
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
                    div { class: "avatar", "{mime_icon(\"vote/poll\")}" }
                    h3 { class: "title-medium", "{t(\"mime.vote\")}" }
                }
                div { class: "list",
                    for poll in polls.iter() {
                        div { class: "list-item", key: "{poll.id.0}",
                            div { class: "avatar small", "{mime_icon(\"vote/poll\")}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{poll.name}" }
                            }
                        }
                    }
                }
            }
        }

        // Other children (comments, questions)
        if !comments.is_empty() {
            div { class: "card mt-1",
                div { class: "list",
                    for child in comments.iter() {
                        div { class: "list-item", key: "{child.id.0}",
                            div { class: "avatar small",
                                "{mime_icon(child.mime_id.as_deref().unwrap_or(\"\"))}"
                            }
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

/// PollApp — poll administration and result viewing
#[component]
pub fn PollApp(node: NodeWithChildren) -> Element {
    let name = node.name.as_str();
    let children = &node.children;
    let data = node
        .data
        .as_ref()
        .and_then(|d| {
            if let serde_json::Value::Object(map) = &d.0 {
                Some(map.clone())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let options: Vec<String> = data
        .get("options")
        .and_then(|o| o.as_array())
        .map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(String::from))
                .collect()
        })
        .unwrap_or_default();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "{mime_icon(\"vote/poll\")}" }
                div {
                    h3 { class: "title-medium", "{name}" }
                    p { class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"poll.managePoll\")}"
                    }
                }
            }
            if options.is_empty() {
                div { class: "card-content",
                    p { class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                div { class: "card-content",
                    div { class: "list",
                        for (i , option) in options.iter().enumerate() {
                            div { class: "list-item", key: "{i}",
                                div { class: "avatar small", "{i + 1}" }
                                div { class: "list-item-text",
                                    div { class: "list-item-primary", "{option}" }
                                }
                            }
                        }
                    }
                }
            }

            // Vote results (children are individual votes)
            if !children.is_empty() {
                div { class: "card-content",
                    p { class: "body-medium",
                        "{t(\"vote.voteCount\")}: {children.len()}"
                    }
                }
            }
        }
    }
}
