use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// Maximum length for a node's display name (#111). Applied as `maxlength` on
/// the name inputs (editor title, add-content form).
pub const NODE_NAME_MAXLEN: usize = 120;

/// EditorApp — rich text content editor
#[component]
pub fn EditorApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let node_id = node.id.0.clone();
    let nav = use_navigator();
    // The node's own path, so saving/publishing returns to the rendered node
    // (its non-app view) rather than leaving the user in the editor.
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };

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
        let segments = segments.clone();
        move |mutable: bool| {
            let token = token.clone();
            let node_id = node_id.clone();
            let segments = segments.clone();
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
                    Ok(true) => {
                        show_snackbar(&t("common.save"));
                        // Return to the node's rendered (non-app) view.
                        nav.push(Route::PathPage {
                            segments: segments.clone(),
                            app: None,
                        });
                    }
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
                // Title field. maxlength caps the node name length (#111).
                div { class: "text-field mb-2",
                    label { "{t(\"common.title\")}" }
                    input {
                        r#type: "text",
                        maxlength: "{NODE_NAME_MAXLEN}",
                        value: "{title}",
                        oninput: move |evt| title.set(evt.value()),
                    }
                }

                // Sticky toolbar (#94): action buttons + formatting controls
                // stay pinned while scrolling a long document.
                div { class: "editor-toolbar",
                    // Action buttons
                    div { class: "stack stack-h mb-1",
                        button {
                            class: "btn btn-primary",
                            disabled: *saving.read(),
                            onclick: {
                                let save = handle_save.clone();
                                move |_| save(true)
                            },
                            span { class: "material-icons", "save" }
                            " {t(\"common.save\")}"
                        }
                        if node.mutable {
                            button {
                                class: "btn btn-secondary",
                                disabled: *saving.read(),
                                onclick: {
                                    let save = handle_save.clone();
                                    move |_| save(false)
                                },
                                span { class: "material-icons", "publish" }
                                " {t(\"content.submit\")}"
                            }
                        }
                        if *saving.read() {
                            div { class: "spinner" }
                        }
                    }

                    // Formatting toolbar — wraps the current selection in the
                    // markdown markers that map to Slate marks.
                    div { class: "stack stack-h mb-1", style: "gap: 4px;",
                        button {
                            class: "btn-icon",
                            style: "font-weight: bold;",
                            title: "Bold",
                            onclick: move |_| wrap_selection("**", content_html),
                            "B"
                        }
                        button {
                            class: "btn-icon",
                            style: "font-style: italic;",
                            title: "Italic",
                            onclick: move |_| wrap_selection("*", content_html),
                            "I"
                        }
                        button {
                            class: "btn-icon",
                            style: "font-family: monospace;",
                            title: "Code",
                            onclick: move |_| wrap_selection("`", content_html),
                            "<>"
                        }
                    }
                }

                // Content editor — a plain-text area, one paragraph per line.
                textarea {
                    id: "editor-textarea",
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

/// Wrap the editor textarea's current selection in `marker` (e.g. `**`) and push
/// the result back into the content signal. Selection indices are UTF-16 units,
/// which match Rust `char` indices for the BMP text this app handles.
fn wrap_selection(marker: &str, mut content: Signal<String>) {
    use wasm_bindgen::JsCast;
    let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id("editor-textarea"))
    else {
        return;
    };
    let Ok(ta) = el.dyn_into::<web_sys::HtmlTextAreaElement>() else {
        return;
    };
    let value = ta.value();
    let chars: Vec<char> = value.chars().collect();
    let start = ta.selection_start().ok().flatten().unwrap_or(0) as usize;
    let end = ta.selection_end().ok().flatten().unwrap_or(0) as usize;
    let (start, end) = (start.min(chars.len()), end.min(chars.len()));
    if start >= end {
        return; // nothing selected
    }
    let before: String = chars[..start].iter().collect();
    let sel: String = chars[start..end].iter().collect();
    let after: String = chars[end..].iter().collect();
    let new = format!("{before}{marker}{sel}{marker}{after}");
    ta.set_value(&new);
    content.set(new);
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
    // Blocks are paragraphs separated by a blank line; soft breaks inside a
    // paragraph are "\n" leaves that `block_text` already re-emits verbatim.
    match content.as_array() {
        Some(blocks) => blocks
            .iter()
            .map(block_text)
            .collect::<Vec<_>>()
            .join("\n\n"),
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

/// Group the editable text into paragraphs of soft-break lines, using the
/// Markdown convention: a blank line starts a new paragraph, a single newline is
/// a soft line break within the current paragraph (#92).
fn split_paragraphs(text: &str) -> Vec<Vec<&str>> {
    let mut paras: Vec<Vec<&str>> = Vec::new();
    let mut cur: Vec<&str> = Vec::new();
    for line in text.split('\n') {
        if line.trim().is_empty() {
            if !cur.is_empty() {
                paras.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(line);
        }
    }
    if !cur.is_empty() {
        paras.push(cur);
    }
    paras
}

/// Convert editable text into Slate blocks: one paragraph per blank-line group,
/// soft breaks as "\n" leaves, each line parsed for inline markdown marks.
fn build_slate_content(text: &str) -> serde_json::Value {
    let paragraphs: Vec<serde_json::Value> = split_paragraphs(text)
        .into_iter()
        .map(|lines| {
            let mut children: Vec<serde_json::Value> = Vec::new();
            for (i, line) in lines.iter().enumerate() {
                if i > 0 {
                    children.push(leaf("\n", None));
                }
                children.extend(parse_inline(line));
            }
            if children.is_empty() {
                children.push(leaf("", None));
            }
            serde_json::json!({ "type": "paragraph", "children": children })
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
        // Separate blocks re-emit as separate paragraphs (blank line between).
        assert_eq!(slate_to_text(&content), "Hello **world**\n\nTitle");
    }

    #[test]
    fn blank_line_separates_paragraphs_single_newline_is_soft_break() {
        // Two blank-line-separated groups → two paragraphs.
        let v = build_slate_content("a\n\nb");
        let arr = v.as_array().expect("array");
        assert_eq!(arr.len(), 2);
        assert_eq!(arr[0]["children"][0]["text"], "a");
        assert_eq!(arr[1]["children"][0]["text"], "b");

        // A single newline is a soft break inside one paragraph: [a, \n, c].
        let soft = build_slate_content("a\nc");
        let sarr = soft.as_array().expect("array");
        assert_eq!(sarr.len(), 1);
        let kids = sarr[0]["children"].as_array().unwrap();
        assert_eq!(kids[0]["text"], "a");
        assert_eq!(kids[1]["text"], "\n");
        assert_eq!(kids[2]["text"], "c");

        // Round-trips back to the same text.
        assert_eq!(slate_to_text(&soft), "a\nc");
        assert_eq!(slate_to_text(&v), "a\n\nb");
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
