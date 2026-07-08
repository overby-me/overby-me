use dioxus::prelude::*;

use crate::graphql::{self, ChildNodeFields, Jsonb, NodeWithChildren, NodesInsertInput, Uuid};
use crate::i18n::t;
use crate::session::use_session;

/// SpeakApp — the speaker queue for a context.
///
/// A context holds a hidden `speak/list` child ("talerliste"); the queue itself
/// is that node's `speak/speak` children. This mirrors the React SpeakApp, which
/// reads `node.get("speakerlist")` and shows its entries.
#[component]
pub fn SpeakApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();

    // The speaker list is the context's (hidden) speak/list child.
    let list_id = node
        .children
        .iter()
        .find(|c| c.mime_id.as_deref() == Some("speak/list"))
        .map(|c| c.id.0.clone());
    let context_id = node.context_id.clone().unwrap_or_else(|| node.id.clone());

    // Re-fetch the queue after every join/remove by bumping this counter.
    let mut refresh = use_signal(|| 0u32);
    let access_token = session.read().access_token.clone();
    let list_for_query = list_id.clone();
    let queue = use_resource(move || {
        let token = access_token.clone();
        let list = list_for_query.clone();
        let _ = refresh.read();
        async move {
            let Some(list) = list else {
                return Vec::new();
            };
            match graphql::query_node_by_id(token.as_deref(), &list).await {
                Ok(Some(n)) => {
                    let mut kids = n.children;
                    // Queue order is arrival order (oldest first).
                    kids.sort_by(|a, b| {
                        let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                        let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                        a_ts.cmp(b_ts)
                    });
                    kids
                }
                _ => Vec::new(),
            }
        }
    });

    let speakers: Vec<ChildNodeFields> = queue.read().clone().unwrap_or_default();

    rsx! {
        div { class: "grid grid-2",
            // Speaker list card
            div {
                div { class: "card",
                    div { class: "card-header",
                        div { class: "avatar secondary", "{super::loader::mime_icon(\"app/speak\")}" }
                        div {
                            h3 { class: "title-medium", "{node.name}" }
                            p {
                                class: "body-medium",
                                style: "color: var(--md-on-surface-variant);",
                                "{t(\"speak.speakerList\")}"
                            }
                        }
                    }
                    if list_id.is_none() {
                        div { class: "card-content",
                            p {
                                class: "body-medium",
                                style: "color: var(--md-on-surface-variant);",
                                "{t(\"speak.emptyList\")}"
                            }
                        }
                    } else if speakers.is_empty() {
                        div { class: "card-content",
                            p {
                                class: "body-medium",
                                style: "color: var(--md-on-surface-variant);",
                                "{t(\"speak.emptyList\")}"
                            }
                        }
                    } else {
                        div { class: "list",
                            for (i , speaker) in speakers.iter().enumerate() {
                                div { class: "list-item", key: "{speaker.id.0}",
                                    div { class: "avatar small secondary", "{i + 1}" }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{speaker.name}" }
                                    }
                                    if is_auth {
                                        {
                                            let speaker_id = speaker.id.0.clone();
                                            let token = session.read().access_token.clone();
                                            rsx! {
                                                button {
                                                    class: "btn-icon",
                                                    title: "{t(\"speak.removeFromList\")}",
                                                    onclick: move |_| {
                                                        let token = token.clone();
                                                        let id = speaker_id.clone();
                                                        spawn(async move {
                                                            let _ = graphql::delete_node(token.as_deref(), &id).await;
                                                            refresh += 1;
                                                        });
                                                    },
                                                    "\u{2715}"
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

            // Join panel — insert a speak/speak entry under the speaker list.
            div {
                if is_auth {
                    if let Some(list) = list_id.clone() {
                        div { class: "card",
                            div { class: "card-header",
                                h3 { class: "title-medium", "{t(\"speak.joinSpeakerList\")}" }
                            }
                            div { class: "card-content",
                                div { class: "stack stack-v",
                                    {
                                        let speak_types = [
                                            ("0", t("speak.talk")),
                                            ("1", t("speak.question")),
                                            ("2", t("speak.clarify")),
                                            ("3", t("speak.procedure")),
                                        ];
                                        rsx! {
                                            for (type_key , label) in speak_types {
                                                {
                                                    let list = list.clone();
                                                    let context_id = context_id.clone();
                                                    let display_name = session.read().user.as_ref().map(|u| u.display_name.clone()).unwrap_or_default();
                                                    let token = session.read().access_token.clone();
                                                    rsx! {
                                                        button {
                                                            class: "btn btn-outlined",
                                                            onclick: move |_| {
                                                                let name = display_name.clone();
                                                                let key = format!("{}-{}", name.to_lowercase().replace(' ', "-"), now_ms());
                                                                let parent = list.clone();
                                                                let ctx = context_id.clone();
                                                                let token = token.clone();
                                                                let type_val = type_key.to_string();
                                                                spawn(async move {
                                                                    let _ = graphql::insert_node(
                                                                        token.as_deref(),
                                                                        NodesInsertInput {
                                                                            name: Some(name),
                                                                            key: Some(key),
                                                                            mime_id: Some("speak/speak".to_string()),
                                                                            parent_id: Some(Uuid(parent)),
                                                                            context_id: Some(ctx),
                                                                            data: Some(Jsonb(serde_json::Value::String(type_val))),
                                                                            mutable: None,
                                                                            index: None,
                                                                        },
                                                                    ).await;
                                                                    refresh += 1;
                                                                });
                                                            },
                                                            "{label}"
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
                }
            }
        }
    }
}

/// A millisecond timestamp for generating unique node keys.
fn now_ms() -> String {
    let window = web_sys::window().unwrap();
    let performance = window.performance().unwrap();
    format!("{:.0}", performance.now())
}
