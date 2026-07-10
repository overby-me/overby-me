use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::nhost::storage_url;
use crate::route::Route;
use crate::session::use_session;

use super::loader::{icon_el, mime_icon};
use super::ui::alert_dialog::{
    AlertDialog, AlertDialogAction, AlertDialogActions, AlertDialogCancel, AlertDialogDescription,
    AlertDialogTitle,
};

#[component]
pub fn ContentApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let node_id = node.id.0.clone();
    let context_id = node.context_id.as_ref().map(|u| u.0.clone());
    let mut confirm_open = use_signal(|| false);
    let name = node.name.clone();
    let members = node.members.clone();
    let data = node.data.map(|d| d.0);
    // Owner-only actions (mirrors the React ContentToolbar gating): a node/context
    // owner may delete; editing also requires the node to still be mutable.
    let can_manage = node.is_owner.unwrap_or(false) || node.is_context_owner.unwrap_or(false);
    let can_edit = can_manage && node.mutable;

    // Optional inline image (a `data.image` file id), mirroring React's Content.
    let image_url = data
        .as_ref()
        .and_then(|d| d.get("image"))
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|file_id| {
            let token = session.read().access_token.clone().unwrap_or_default();
            format!("{}/files/{file_id}?token={token}", storage_url())
        });

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("wiki/document")} }
                h3 { class: "title-medium", "{name}" }
                div { class: "flex-grow" }
                // Export this document (and any nested content) to an .odt file.
                button {
                    class: "btn-icon",
                    title: "{t(\"folder.export\")}",
                    onclick: {
                        let export_name = name.clone();
                        let export_id = node_id.clone();
                        move |_| {
                            let token = session.read().access_token.clone();
                            let id = export_id.clone();
                            let name = export_name.clone();
                            spawn(async move {
                                crate::export::export_tree(token, id, name).await;
                            });
                        }
                    },
                    span { class: "material-icons", "download" }
                }
                if can_edit && !segments.is_empty() {
                    Link {
                        to: Route::PathPage {
                            segments: segments.clone(),
                            app: Some("editor".to_string()),
                        },
                        class: "btn-icon",
                        title: "{t(\"mime.editor\")}",
                        {icon_el("app/editor")}
                    }
                }
                if can_manage && !segments.is_empty() {
                    // Delete via an accessible modal confirm dialog.
                    button {
                        class: "btn-icon",
                        title: "{t(\"common.delete\")}",
                        onclick: move |_| confirm_open.set(true),
                        span { class: "material-icons", "delete" }
                    }
                    AlertDialog {
                        open: Some(confirm_open()),
                        on_open_change: move |v| confirm_open.set(v),
                        AlertDialogTitle { "{t(\"content.confirmDelete\")}" }
                        AlertDialogDescription { "{name}" }
                        AlertDialogActions {
                            AlertDialogCancel { "{t(\"common.cancel\")}" }
                            AlertDialogAction {
                                on_click: {
                                    let node_id = node_id.clone();
                                    let parent = segments[..segments.len() - 1].to_vec();
                                    move |_| {
                                        let token = session.read().access_token.clone();
                                        let node_id = node_id.clone();
                                        let parent = parent.clone();
                                        confirm_open.set(false);
                                        spawn(async move {
                                            if graphql::delete_node(token.as_deref(), &node_id)
                                                .await
                                                .unwrap_or(false)
                                            {
                                                crate::session::bump_data_version();
                                                nav.push(Route::PathPage {
                                                    segments: parent,
                                                    app: None,
                                                });
                                            }
                                        });
                                    }
                                },
                                "{t(\"common.delete\")}"
                            }
                        }
                    }
                }
            }
            if let Some(url) = image_url {
                div { class: "card-content",
                    super::widgets::ZoomableImage { src: url.clone(), alt: t("content.imageAlt") }
                }
            }
            // Author chips (the document's members), mirroring MemberChips.
            if !members.is_empty() {
                div { class: "chip-row", style: "padding: 0 16px 8px;",
                    for member in members.iter() {
                        super::widgets::Chip {
                            key: "{member.id.0}",
                            icon: mime_icon(member.node.as_ref().and_then(|n| n.mime_id.as_deref()).unwrap_or("wiki/user")).to_string(),
                            label: member.label(),
                            title: t("member.author"),
                        }
                    }
                }
            }
            div { class: "card-content",
                SlateRenderer { data: data.clone() }
            }
        }
        super::comments::CommentSection { node_id: node_id.clone(), context_id: context_id.clone() }
    }
}

/// Whether a node's `data` carries non-empty rich-text content (so a folder or
/// context can decide whether to show its description).
pub fn has_rich_content(data: Option<&serde_json::Value>) -> bool {
    data.and_then(|d| d.get("content"))
        .and_then(|c| c.as_array())
        .map(|blocks| {
            blocks
                .iter()
                .any(|b| !block_plain_text(b).trim().is_empty())
        })
        .unwrap_or(false)
}

/// Renders Slate.js JSON content as HTML
#[component]
pub fn SlateRenderer(data: Option<serde_json::Value>) -> Element {
    let content = data
        .as_ref()
        .and_then(|d| d.get("content"))
        .or(data.as_ref());

    match content {
        Some(serde_json::Value::Array(blocks)) => {
            // Table of contents (#117): heading blocks become anchored links.
            let headings = extract_headings(blocks);
            rsx! {
                div { class: "slate-content",
                    if headings.len() >= 2 {
                        nav { class: "toc", aria_label: "{t(\"content.tableOfContents\")}",
                            for (level , text , anchor) in headings.iter() {
                                a { class: "toc-item toc-l{level}", href: "#{anchor}", "{text}" }
                            }
                        }
                    }
                    for (i , block) in blocks.iter().enumerate() {
                        SlateBlock { key: "{i}", index: i, block: block.clone() }
                    }
                }
            }
        }
        _ => {
            rsx! {
                div { class: "slate-content",
                    p { class: "body-medium",
                        class: "text-muted",
                        ""
                    }
                }
            }
        }
    }
}

/// The heading blocks of a Slate document as `(level, text, anchor)`, for the
/// table of contents. The anchor matches the `id` `SlateBlock` renders.
fn extract_headings(blocks: &[serde_json::Value]) -> Vec<(u8, String, String)> {
    blocks
        .iter()
        .enumerate()
        .filter_map(|(i, block)| {
            let ty = block.get("type").and_then(|t| t.as_str()).unwrap_or("");
            let level = heading_level(ty)?;
            let text = block_plain_text(block);
            if text.trim().is_empty() {
                return None;
            }
            let anchor = heading_anchor(i, &text);
            Some((level, text, anchor))
        })
        .collect()
}

/// Heading depth (1..6) for a Slate block type, or None if it is not a heading.
fn heading_level(block_type: &str) -> Option<u8> {
    match block_type {
        "heading-one" | "h1" => Some(1),
        "heading-two" | "h2" => Some(2),
        "heading-three" | "h3" => Some(3),
        "heading-four" | "h4" => Some(4),
        "heading-five" | "h5" => Some(5),
        "heading-six" | "h6" => Some(6),
        _ => None,
    }
}

/// All leaf text of a block, concatenated (for the TOC label / anchor).
fn block_plain_text(block: &serde_json::Value) -> String {
    fn collect(v: &serde_json::Value, out: &mut String) {
        if let Some(t) = v.get("text").and_then(|t| t.as_str()) {
            out.push_str(t);
        }
        if let Some(children) = v.get("children").and_then(|c| c.as_array()) {
            for c in children {
                collect(c, out);
            }
        }
    }
    let mut s = String::new();
    collect(block, &mut s);
    s
}

/// A unique, stable anchor id for the heading at block `index` — slug of its
/// text prefixed with the index so duplicate headings do not collide.
fn heading_anchor(index: usize, text: &str) -> String {
    let mut slug = String::new();
    let mut prev_dash = false;
    for c in text.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            slug.push(c);
            prev_dash = false;
        } else if !prev_dash {
            slug.push('-');
            prev_dash = true;
        }
    }
    let slug = slug.trim_matches('-');
    format!("h{index}-{slug}")
}

#[component]
fn SlateBlock(block: serde_json::Value, index: usize) -> Element {
    let block_type = block
        .get("type")
        .and_then(|t| t.as_str())
        .unwrap_or("paragraph");
    let children = block
        .get("children")
        .and_then(|c| c.as_array())
        .cloned()
        .unwrap_or_default();

    let rendered_children = rsx! {
        for (i , child) in children.iter().enumerate() {
            SlateInline { key: "{i}", node: child.clone() }
        }
    };

    // Heading anchor id so the table of contents can link to it (#117).
    let hid = heading_level(block_type)
        .map(|_| heading_anchor(index, &block_plain_text(&block)))
        .unwrap_or_default();

    // Per-block alignment set in the editor.
    let astyle = match block.get("align").and_then(|a| a.as_str()) {
        Some(a @ ("center" | "right" | "justify" | "left")) => format!("text-align:{a}"),
        _ => String::new(),
    };

    match block_type {
        "heading-one" | "h1" => rsx! { h1 { id: "{hid}", style: "{astyle}", {rendered_children} } },
        "heading-two" | "h2" => rsx! { h2 { id: "{hid}", style: "{astyle}", {rendered_children} } },
        "heading-three" | "h3" => {
            rsx! { h3 { id: "{hid}", style: "{astyle}", {rendered_children} } }
        }
        "heading-four" | "h4" => {
            rsx! { h4 { id: "{hid}", style: "{astyle}", {rendered_children} } }
        }
        "heading-five" | "h5" => {
            rsx! { h5 { id: "{hid}", style: "{astyle}", {rendered_children} } }
        }
        "heading-six" | "h6" => rsx! { h6 { id: "{hid}", style: "{astyle}", {rendered_children} } },
        "block-quote" => rsx! { blockquote { style: "{astyle}", {rendered_children} } },
        "block-pre" | "code" => rsx! { pre { style: "{astyle}", {rendered_children} } },
        "bulleted-list" | "ul" => rsx! { ul { style: "{astyle}", {rendered_children} } },
        "numbered-list" | "ol" => rsx! { ol { style: "{astyle}", {rendered_children} } },
        "list-item" | "li" => rsx! { li { style: "{astyle}", {rendered_children} } },
        "image" => {
            let url = block.get("url").and_then(|u| u.as_str()).unwrap_or("");
            rsx! {
                super::widgets::ZoomableImage { src: url.to_string(), alt: "content image".to_string() }
            }
        }
        _ => rsx! { p { style: "{astyle}", {rendered_children} } },
    }
}

#[component]
fn SlateInline(node: serde_json::Value) -> Element {
    // Leaf text node
    if let Some(text) = node.get("text").and_then(|t| t.as_str()) {
        let bold = node.get("bold").and_then(|b| b.as_bool()).unwrap_or(false);
        let italic = node
            .get("italic")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let underline = node
            .get("underline")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let strikethrough = node
            .get("strikethrough")
            .and_then(|b| b.as_bool())
            .unwrap_or(false);
        let code = node.get("code").and_then(|b| b.as_bool()).unwrap_or(false);

        let mut style_parts = Vec::new();
        if bold {
            style_parts.push("font-weight: bold");
        }
        if italic {
            style_parts.push("font-style: italic");
        }
        if underline && strikethrough {
            style_parts.push("text-decoration: underline line-through");
        } else if underline {
            style_parts.push("text-decoration: underline");
        } else if strikethrough {
            style_parts.push("text-decoration: line-through");
        }

        let style = style_parts.join("; ");

        // An explicit link mark turns the whole leaf into an anchor (matching the
        // editor, which stores links as a leaf mark rather than an element).
        if let Some(url) = node
            .get("link")
            .and_then(|l| l.as_str())
            .filter(|l| !l.is_empty())
        {
            return rsx! {
                a { href: "{url}", target: "_blank", rel: "noopener",
                    if code {
                        code { "{text}" }
                    } else if style.is_empty() {
                        "{text}"
                    } else {
                        span { style: "{style}", "{text}" }
                    }
                }
            };
        }

        if code {
            return rsx! {
                code { "{text}" }
            };
        }

        if style.is_empty() {
            return rsx! {
                AutoLinked { text: text.to_string() }
            };
        }

        return rsx! {
            span { style: "{style}", AutoLinked { text: text.to_string() } }
        };
    }

    // Inline element (link, etc.)
    if let Some(element_type) = node.get("type").and_then(|t| t.as_str()) {
        let children = node
            .get("children")
            .and_then(|c| c.as_array())
            .cloned()
            .unwrap_or_default();

        match element_type {
            "link" => {
                let url = node.get("url").and_then(|u| u.as_str()).unwrap_or("#");
                return rsx! {
                    a { href: "{url}", target: "_blank", rel: "noopener",
                        for (i , child) in children.iter().enumerate() {
                            SlateInline { key: "{i}", node: child.clone() }
                        }
                    }
                };
            }
            "list-item" | "li" => {
                return rsx! {
                    li {
                        for (i , child) in children.iter().enumerate() {
                            SlateInline { key: "{i}", node: child.clone() }
                        }
                    }
                };
            }
            _ => {
                return rsx! {
                    span {
                        for (i , child) in children.iter().enumerate() {
                            SlateInline { key: "{i}", node: child.clone() }
                        }
                    }
                };
            }
        }
    }

    rsx! {}
}

/// A plain-text run with bare URLs and email addresses turned into links (#97).
#[component]
fn AutoLinked(text: String) -> Element {
    rsx! {
        for (i , token) in autolink_tokens(&text).into_iter().enumerate() {
            match token {
                LinkToken::Text(s) => rsx! { "{s}" },
                LinkToken::Url(url, trail) => rsx! {
                    a { key: "{i}", href: "{url}", target: "_blank", rel: "noopener", "{url}" }
                    "{trail}"
                },
                LinkToken::Email(addr, trail) => rsx! {
                    a { key: "{i}", href: "mailto:{addr}", "{addr}" }
                    "{trail}"
                },
            }
        }
    }
}

#[derive(Debug, PartialEq)]
enum LinkToken {
    Text(String),
    Url(String, String),
    Email(String, String),
}

/// Split a text run into plain / URL / email tokens, keeping spacing. URLs must
/// start http(s)://; emails are `local@domain.tld`. Trailing punctuation is kept
/// out of the link target.
fn autolink_tokens(text: &str) -> Vec<LinkToken> {
    let mut out = Vec::new();
    for (i, word) in text.split(' ').enumerate() {
        if i > 0 {
            out.push(LinkToken::Text(" ".to_string()));
        }
        if word.is_empty() {
            continue;
        }
        let end = word.trim_end_matches(|c: char| ".,;:!?)]}'\"".contains(c));
        let trail = word[end.len()..].to_string();
        if end.starts_with("http://") || end.starts_with("https://") {
            out.push(LinkToken::Url(end.to_string(), trail));
        } else if is_email(end) {
            out.push(LinkToken::Email(end.to_string(), trail));
        } else {
            out.push(LinkToken::Text(word.to_string()));
        }
    }
    out
}

fn is_email(w: &str) -> bool {
    let mut parts = w.splitn(2, '@');
    match (parts.next(), parts.next()) {
        (Some(local), Some(domain)) => {
            !local.is_empty()
                && domain.contains('.')
                && !domain.starts_with('.')
                && !domain.ends_with('.')
                && !domain.contains('@')
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn headings_extracted_with_stable_anchors() {
        let blocks = vec![
            serde_json::json!({ "type": "heading-one", "children": [{"text": "Intro"}] }),
            serde_json::json!({ "type": "paragraph", "children": [{"text": "body"}] }),
            serde_json::json!({ "type": "heading-two", "children": [{"text": "Intro"}] }),
            serde_json::json!({ "type": "heading-three", "children": [{"text": "  "}] }),
        ];
        let hs = extract_headings(&blocks);
        // Two non-empty headings; the blank one is skipped.
        assert_eq!(hs.len(), 2);
        assert_eq!(hs[0], (1, "Intro".to_string(), "h0-intro".to_string()));
        // Duplicate text gets a distinct anchor via the block index.
        assert_eq!(hs[1], (2, "Intro".to_string(), "h2-intro".to_string()));
        // The block renderer computes the same anchor for the same index/text.
        assert_eq!(heading_anchor(0, "Intro"), "h0-intro");
    }

    #[test]
    fn autolink_detects_url_and_email() {
        let toks = autolink_tokens("see https://x.org, mail me@x.dk!");
        assert!(toks.contains(&LinkToken::Url("https://x.org".into(), ",".into())));
        assert!(toks.contains(&LinkToken::Email("me@x.dk".into(), "!".into())));
        // Plain words stay text.
        assert!(toks.contains(&LinkToken::Text("see".into())));
        assert!(!is_email("not-an-email"));
        assert!(!is_email("a@b"));
        assert!(is_email("a@b.dk"));
    }
}
