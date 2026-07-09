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

/// Flatten Slate content to editable text, re-emitting inline marks as markdown
/// (`**bold**`, `*italic*`, `` `code` ``) — one line per top-level block.
fn slate_to_text(content: &serde_json::Value) -> String {
    fn leaf_to_md(leaf: &serde_json::Value) -> Option<String> {
        let text = leaf.get("text").and_then(|t| t.as_str())?;
        let flag = |k: &str| leaf.get(k).and_then(|b| b.as_bool()).unwrap_or(false);
        Some(if flag("bold") {
            format!("**{text}**")
        } else if flag("italic") {
            format!("*{text}*")
        } else if flag("code") {
            format!("`{text}`")
        } else {
            text.to_string()
        })
    }
    fn block_text(block: &serde_json::Value) -> String {
        match block.get("children").and_then(|c| c.as_array()) {
            Some(children) => children
                .iter()
                .map(|leaf| leaf_to_md(leaf).unwrap_or_else(|| block_text(leaf)))
                .collect(),
            None => String::new(),
        }
    }
    match content.as_array() {
        Some(blocks) => blocks.iter().map(block_text).collect::<Vec<_>>().join("\n"),
        None => String::new(),
    }
}

fn leaf(text: &str, mark: Option<&str>) -> serde_json::Value {
    let mut obj = serde_json::Map::new();
    obj.insert("text".to_string(), serde_json::Value::from(text));
    if let Some(m) = mark {
        obj.insert(m.to_string(), serde_json::Value::Bool(true));
    }
    serde_json::Value::Object(obj)
}

/// Parse one line of markdown-ish inline syntax into Slate leaves. Supports
/// non-nested `**bold**`, `*italic*` and `` `code` ``.
fn parse_inline(line: &str) -> Vec<serde_json::Value> {
    let chars: Vec<char> = line.chars().collect();
    let mut out: Vec<serde_json::Value> = Vec::new();
    let mut plain = String::new();
    let mut i = 0;

    let find = |from: usize, pat: &[char]| -> Option<usize> {
        let mut j = from;
        while j + pat.len() <= chars.len() {
            if chars[j..j + pat.len()] == *pat {
                return Some(j);
            }
            j += 1;
        }
        None
    };

    while i < chars.len() {
        // Try each marker: (delimiter chars, mark name).
        let markers: [(&[char], &str); 3] =
            [(&['*', '*'], "bold"), (&['*'], "italic"), (&['`'], "code")];
        let mut matched = false;
        for (delim, mark) in markers {
            if i + delim.len() <= chars.len() && chars[i..i + delim.len()] == *delim {
                if let Some(close) = find(i + delim.len(), delim) {
                    if close > i + delim.len() {
                        if !plain.is_empty() {
                            out.push(leaf(&plain, None));
                            plain.clear();
                        }
                        let text: String = chars[i + delim.len()..close].iter().collect();
                        out.push(leaf(&text, Some(mark)));
                        i = close + delim.len();
                        matched = true;
                        break;
                    }
                }
            }
        }
        if !matched {
            plain.push(chars[i]);
            i += 1;
        }
    }
    if !plain.is_empty() {
        out.push(leaf(&plain, None));
    }
    if out.is_empty() {
        out.push(leaf("", None));
    }
    out
}

/// Convert editable text into Slate blocks: one paragraph per line, each parsed
/// for inline markdown marks.
fn build_slate_content(text: &str) -> serde_json::Value {
    let paragraphs: Vec<serde_json::Value> = text
        .split('\n')
        .map(|line| {
            serde_json::json!({
                "type": "paragraph",
                "children": parse_inline(line),
            })
        })
        .collect();

    if paragraphs.is_empty() {
        serde_json::json!([{ "type": "paragraph", "children": [{"text": ""}] }])
    } else {
        serde_json::Value::Array(paragraphs)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn slate_to_text_reemits_marks_as_markdown() {
        let content = serde_json::json!([
            {"type": "paragraph", "children": [{"text": "Hello "}, {"text": "world", "bold": true}]},
            {"type": "heading-one", "children": [{"text": "Title"}]}
        ]);
        assert_eq!(slate_to_text(&content), "Hello **world**\nTitle");
    }

    #[test]
    fn build_slate_content_makes_one_paragraph_per_line() {
        let v = build_slate_content("a\nb");
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["children"][0]["text"], "a");
        assert_eq!(arr[1]["children"][0]["text"], "b");
    }

    #[test]
    fn inline_markdown_round_trips() {
        let text = "plain **b** and *i* and `c`";
        let content = build_slate_content(text);
        // Bold/italic/code leaves are produced with the right marks.
        let leaves = content[0]["children"].as_array().unwrap();
        assert!(leaves.iter().any(|l| l["text"] == "b" && l["bold"] == true));
        assert!(leaves
            .iter()
            .any(|l| l["text"] == "i" && l["italic"] == true));
        assert!(leaves.iter().any(|l| l["text"] == "c" && l["code"] == true));
        // And the text survives a round trip.
        assert_eq!(slate_to_text(&content), text);
    }
}
