//! Slate JSON -> HTML rendering (seeds the contenteditable editor and the
//! ODT export). Pure and portable: no browser APIs, unit-tested on the host.
//!
//! The DOM -> Slate direction and the `execCommand`/selection plumbing live in
//! the parent [`super`] module, which is browser-only.

use serde_json::Value;

/// Escape text for use as HTML character data.
pub(super) fn html_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    out
}

/// Escape text for use inside a double-quoted HTML attribute.
fn attr_escape(s: &str) -> String {
    html_escape(s).replace('"', "&quot;")
}

/// The `text-align` style attribute for a block's `align`, or empty for none.
fn align_style(block: &Value) -> String {
    match block.get("align").and_then(|a| a.as_str()) {
        Some(a @ ("center" | "right" | "justify" | "left")) => {
            format!(" style=\"text-align:{a}\"")
        }
        _ => String::new(),
    }
}

/// One Slate text leaf as HTML, wrapping the text in the tags for its marks.
/// Soft breaks (`\n`) become `<br>`.
fn leaf_to_html(leaf: &Value) -> String {
    // A nested block sitting in inline position (unusual) is rendered as a block.
    if leaf.get("type").is_some() {
        return block_to_html(leaf);
    }
    let text = leaf.get("text").and_then(|t| t.as_str()).unwrap_or("");
    let mut s = html_escape(text).replace('\n', "<br>");
    let flag = |k: &str| leaf.get(k).and_then(|b| b.as_bool()).unwrap_or(false);
    if flag("bold") {
        s = format!("<strong>{s}</strong>");
    }
    if flag("italic") {
        s = format!("<em>{s}</em>");
    }
    if flag("underline") {
        s = format!("<u>{s}</u>");
    }
    if flag("strikethrough") {
        s = format!("<s>{s}</s>");
    }
    if flag("code") {
        s = format!("<code>{s}</code>");
    }
    if let Some(link) = leaf
        .get("link")
        .and_then(|l| l.as_str())
        .filter(|l| !l.is_empty())
    {
        s = format!("<a href=\"{}\">{s}</a>", attr_escape(link));
    }
    s
}

/// The concatenated inline HTML of a block's children.
fn inline_html(block: &Value) -> String {
    block
        .get("children")
        .and_then(|c| c.as_array())
        .map(|children| children.iter().map(leaf_to_html).collect())
        .unwrap_or_default()
}

/// Render one Slate block to HTML.
fn block_to_html(block: &Value) -> String {
    let ty = block
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("paragraph");

    if ty == "image" {
        let url = block.get("url").and_then(|u| u.as_str()).unwrap_or("");
        return format!("<img src=\"{}\" alt=\"\">", attr_escape(url));
    }

    let tag = match ty {
        "heading-one" => "h1",
        "heading-two" => "h2",
        "heading-three" => "h3",
        "heading-four" => "h4",
        "heading-five" => "h5",
        "heading-six" => "h6",
        "block-quote" => "blockquote",
        "block-pre" => "pre",
        "bulleted-list" => "ul",
        "numbered-list" => "ol",
        "list-item" => "li",
        _ => "p",
    };

    let inner = if ty == "bulleted-list" || ty == "numbered-list" {
        // Children are list items.
        block
            .get("children")
            .and_then(|c| c.as_array())
            .map(|items| items.iter().map(block_to_html).collect())
            .unwrap_or_default()
    } else {
        let h = inline_html(block);
        // An empty block needs a <br> so the caret can enter it.
        if h.is_empty() {
            "<br>".to_string()
        } else {
            h
        }
    };

    format!("<{tag}{}>{inner}</{tag}>", align_style(block))
}

/// Render a whole Slate `content` array to the HTML that seeds the editor.
pub fn slate_to_html(content: &Value) -> String {
    match content.as_array() {
        Some(blocks) if !blocks.is_empty() => blocks.iter().map(block_to_html).collect(),
        _ => "<p><br></p>".to_string(),
    }
}

/// Whether a Slate block is an empty paragraph (its only text is blank).
fn is_empty_paragraph(block: &Value) -> bool {
    block.get("type").and_then(|t| t.as_str()) == Some("paragraph")
        && block
            .get("children")
            .and_then(|c| c.as_array())
            .map(|ch| {
                ch.iter().all(|leaf| {
                    leaf.get("text")
                        .and_then(|t| t.as_str())
                        .unwrap_or("x")
                        .is_empty()
                })
            })
            .unwrap_or(false)
}

/// Drop a leading empty paragraph from a serialized content array (a common
/// contenteditable artefact), unless it is the only block. Stops a document from
/// gaining a blank first line on every save.
pub fn strip_leading_empty_paragraph(content: Value) -> Value {
    match content {
        Value::Array(mut blocks) if blocks.len() > 1 && is_empty_paragraph(&blocks[0]) => {
            blocks.remove(0);
            Value::Array(blocks)
        }
        other => other,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn attr_escape_covers_quotes_and_html_metachars() {
        // Inside a double-quoted attribute, `"` must become `&quot;` (else an
        // attacker's href/src could break out of the attribute), on top of the
        // usual `&`/`<`/`>` escaping html_escape already does.
        assert_eq!(attr_escape(r#"a"b"#), "a&quot;b");
        assert_eq!(attr_escape("a&b<c>"), "a&amp;b&lt;c&gt;");
        assert_eq!(
            attr_escape(r#"" onmouseover="x"#),
            "&quot; onmouseover=&quot;x"
        );
        assert_eq!(attr_escape("plain"), "plain");
    }

    #[test]
    fn strips_a_leading_blank_paragraph_but_keeps_a_sole_one() {
        let empty = json!({"type": "paragraph", "children": [{"text": ""}]});
        let para = json!({"type": "paragraph", "children": [{"text": "hi"}]});
        // Leading blank before real content is dropped.
        assert_eq!(
            strip_leading_empty_paragraph(json!([empty, para])),
            json!([{"type": "paragraph", "children": [{"text": "hi"}]}])
        );
        // A sole blank paragraph is kept (an empty document needs a block).
        assert_eq!(
            strip_leading_empty_paragraph(json!([empty])),
            json!([{"type": "paragraph", "children": [{"text": ""}]}])
        );
        // A leading non-empty paragraph is untouched.
        assert_eq!(
            strip_leading_empty_paragraph(json!([para, para])),
            json!([para, para])
        );
    }

    #[test]
    fn paragraph_and_heading_round_to_html() {
        let content = json!([
            {"type":"paragraph","children":[{"text":"Hello "},{"text":"world","bold":true}]},
            {"type":"heading-two","children":[{"text":"Title"}]},
        ]);
        let html = slate_to_html(&content);
        assert_eq!(html, "<p>Hello <strong>world</strong></p><h2>Title</h2>");
    }

    #[test]
    fn lists_render_nested_items() {
        let content = json!([
            {"type":"bulleted-list","children":[
                {"type":"list-item","children":[{"text":"a"}]},
                {"type":"list-item","children":[{"text":"b"}]},
            ]},
            {"type":"numbered-list","children":[
                {"type":"list-item","children":[{"text":"one"}]},
            ]},
        ]);
        let html = slate_to_html(&content);
        assert_eq!(html, "<ul><li>a</li><li>b</li></ul><ol><li>one</li></ol>");
    }

    #[test]
    fn marks_alignment_link_and_softbreak() {
        let content = json!([
            {"type":"paragraph","align":"center","children":[
                {"text":"i","italic":true},
                {"text":"u","underline":true},
                {"text":"s","strikethrough":true},
                {"text":"c","code":true},
                {"text":"x"},
                {"text":"\n"},
                {"text":"link","link":"https://example.org/a?b=1&c"},
            ]},
        ]);
        let html = slate_to_html(&content);
        assert!(html.contains("<p style=\"text-align:center\">"));
        assert!(html.contains("<em>i</em>"));
        assert!(html.contains("<u>u</u>"));
        assert!(html.contains("<s>s</s>"));
        assert!(html.contains("<code>c</code>"));
        assert!(html.contains("x<br>"));
        assert!(html.contains("<a href=\"https://example.org/a?b=1&amp;c\">link</a>"));
    }

    #[test]
    fn block_pre_and_image_and_empty() {
        let content = json!([
            {"type":"block-pre","children":[{"text":"code()"}]},
            {"type":"image","url":"https://example.org/p.png","children":[{"text":""}]},
            {"type":"paragraph","children":[{"text":""}]},
        ]);
        let html = slate_to_html(&content);
        assert!(html.contains("<pre>code()</pre>"));
        assert!(html.contains("<img src=\"https://example.org/p.png\" alt=\"\">"));
        // Empty paragraph carries a <br> so the caret can enter it.
        assert!(html.contains("<p><br></p>"));
    }

    #[test]
    fn empty_content_is_one_empty_paragraph() {
        assert_eq!(slate_to_html(&json!([])), "<p><br></p>");
        assert_eq!(slate_to_html(&Value::Null), "<p><br></p>");
    }

    #[test]
    fn html_escaping_of_text_and_attributes() {
        let content = json!([
            {"type":"paragraph","children":[{"text":"a < b & c > d"}]},
        ]);
        assert_eq!(slate_to_html(&content), "<p>a &lt; b &amp; c &gt; d</p>");
    }
}
