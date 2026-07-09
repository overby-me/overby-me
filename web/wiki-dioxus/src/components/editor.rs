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

    // Extract the existing content as editable plain text (one line per block).
    // Loading the raw Slate JSON here would show markup and, on save, round-trip
    // that JSON back into paragraphs.
    let initial_content = node
        .data
        .as_ref()
        .and_then(|d| d.0.get("content"))
        .map(slate_to_text)
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

                // Content editor — a plain-text area, one paragraph per line.
                textarea {
                    class: "editor-area",
                    style: "width: 100%; min-height: 240px; resize: vertical;",
                    value: "{content_html}",
                    oninput: move |evt| {
                        content_html.set(evt.value());
                    },
                }
            }
        }
    }
}

/// Flatten Slate content JSON to editable plain text: one line per top-level
/// block, concatenating each block's leaf `text` runs.
fn slate_to_text(content: &serde_json::Value) -> String {
    fn collect_text(node: &serde_json::Value, out: &mut String) {
        if let Some(t) = node.get("text").and_then(|t| t.as_str()) {
            out.push_str(t);
        } else if let Some(children) = node.get("children").and_then(|c| c.as_array()) {
            for child in children {
                collect_text(child, out);
            }
        }
    }
    match content.as_array() {
        Some(blocks) => blocks
            .iter()
            .map(|block| {
                let mut line = String::new();
                collect_text(block, &mut line);
                line
            })
            .collect::<Vec<_>>()
            .join("\n"),
        None => String::new(),
    }
}

/// Convert plain text into Slate-compatible JSON blocks (one paragraph per line)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slate_to_text_flattens_blocks_and_marks() {
        let content = serde_json::json!([
            {"type": "paragraph", "children": [{"text": "Hello "}, {"text": "world", "bold": true}]},
            {"type": "heading-one", "children": [{"text": "Title"}]}
        ]);
        assert_eq!(slate_to_text(&content), "Hello world\nTitle");
    }

    #[test]
    fn build_slate_content_makes_one_paragraph_per_line() {
        let v = build_slate_content("a\nb");
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["children"][0]["text"], "a");
        assert_eq!(arr[1]["children"][0]["text"], "b");
    }
}
