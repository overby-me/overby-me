use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// EditorApp — rich text content editor
#[component]
pub fn EditorApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let node_id = node.id.0.clone();

    let mut title = use_signal(|| node.name.clone());
    let mut saving = use_signal(|| false);

    // Extract existing content from node data
    let initial_content = node
        .data
        .as_ref()
        .and_then(|d| d.0.get("content"))
        .and_then(|c| serde_json::to_string_pretty(c).ok())
        .unwrap_or_default();

    let mut content_html = use_signal(|| initial_content);

    let handle_save = {
        let token = session.read().access_token.clone();
        let node_id = node_id.clone();
        move |mutable: bool| {
            let token = token.clone();
            let node_id = node_id.clone();
            let title_val = title.read().clone();
            let content_val = content_html.read().clone();
            spawn(async move {
                saving.set(true);

                // Build content as Slate-compatible JSON.
                let content_json = build_slate_content(&content_val);
                let data = serde_json::json!({ "content": content_json });

                let set = graphql::NodesSetInput {
                    name: Some(title_val),
                    data: Some(graphql::Jsonb(data)),
                    mutable: Some(mutable),
                    ..Default::default()
                };

                match graphql::update_node(token.as_deref(), &node_id, set).await {
                    Ok(true) => show_snackbar(&t("common.save")),
                    Ok(false) => show_snackbar(&t("error.somethingWentWrong")),
                    Err(e) => {
                        log::error!("Save failed: {e}");
                        show_snackbar(&t("error.somethingWentWrong"));
                    }
                }

                saving.set(false);
            });
        }
    };

    if !is_auth {
        return rsx! {
            div { class: "card",
                div { class: "card-content",
                    p { class: "body-large", "{t(\"node.documentUnavailable\")}" }
                }
            }
        };
    }

    rsx! {
        div { class: "card",
            div { class: "card-content",
                // Title field
                div { class: "text-field mb-2",
                    label { "{t(\"common.title\")}" }
                    input {
                        r#type: "text",
                        value: "{title}",
                        oninput: move |evt| title.set(evt.value()),
                    }
                }

                // Action buttons
                div { class: "stack stack-h mb-2",
                    button {
                        class: "btn btn-primary",
                        disabled: *saving.read(),
                        onclick: {
                            let save = handle_save.clone();
                            move |_| save(true)
                        },
                        "\u{1F4BE} {t(\"common.save\")}"
                    }
                    if node.mutable {
                        button {
                            class: "btn btn-secondary",
                            disabled: *saving.read(),
                            onclick: {
                                let save = handle_save.clone();
                                move |_| save(false)
                            },
                            "\u{1F4E4} {t(\"content.submit\")}"
                        }
                    }
                    if *saving.read() {
                        div { class: "spinner" }
                    }
                }

                // Content editor — uses contenteditable div
                div {
                    class: "slate-content editor-area",
                    contenteditable: "true",
                    oninput: move |evt| {
                        content_html.set(evt.value());
                    },
                    dangerous_inner_html: "{content_html}",
                }
            }
        }
    }
}

/// Convert plain text/HTML content into Slate-compatible JSON blocks
fn build_slate_content(html: &str) -> serde_json::Value {
    let paragraphs: Vec<serde_json::Value> = html
        .split('\n')
        .filter(|line| !line.is_empty())
        .map(|line| {
            serde_json::json!({
                "type": "paragraph",
                "children": [{"text": line.trim()}]
            })
        })
        .collect();

    if paragraphs.is_empty() {
        serde_json::json!([{
            "type": "paragraph",
            "children": [{"text": ""}]
        }])
    } else {
        serde_json::Value::Array(paragraphs)
    }
}
