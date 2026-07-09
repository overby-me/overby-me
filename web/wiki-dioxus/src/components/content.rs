use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::nhost::storage_url;
use crate::route::Route;
use crate::session::use_session;

use super::loader::mime_icon;
use super::ui::alert_dialog::{
    AlertDialog, AlertDialogAction, AlertDialogActions, AlertDialogCancel, AlertDialogDescription,
    AlertDialogTitle,
};

#[component]
pub fn ContentApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let is_auth = session.read().is_authenticated();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let node_id = node.id.0.clone();
    let mut confirm_open = use_signal(|| false);
    let name = node.name.clone();
    let members = node.members.clone();
    let data = node.data.map(|d| d.0);

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
                div { class: "avatar", "{mime_icon(\"wiki/document\")}" }
                h3 { class: "title-medium", "{name}" }
                if is_auth && !segments.is_empty() {
                    div { class: "flex-grow" }
                    Link {
                        to: Route::PathPage {
                            segments: segments.clone(),
                            app: Some("editor".to_string()),
                        },
                        class: "btn-icon",
                        title: "{t(\"mime.editor\")}",
                        "{mime_icon(\"app/editor\")}"
                    }
                    // Delete via an accessible modal confirm dialog.
                    button {
                        class: "btn-icon",
                        title: "{t(\"common.delete\")}",
                        onclick: move |_| confirm_open.set(true),
                        "\u{1F5D1}\u{FE0F}"
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
                    img {
                        src: "{url}",
                        alt: "{t(\"content.imageAlt\")}",
                        style: "max-width: 100%; border-radius: 8px;",
                    }
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
                SlateRenderer { data }
            }
        }
    }
}

/// Renders Slate.js JSON content as HTML
#[component]
fn SlateRenderer(data: Option<serde_json::Value>) -> Element {
    let content = data
        .as_ref()
        .and_then(|d| d.get("content"))
        .or(data.as_ref());

    match content {
        Some(serde_json::Value::Array(blocks)) => {
            rsx! {
                div { class: "slate-content",
                    for (i , block) in blocks.iter().enumerate() {
                        SlateBlock { key: "{i}", block: block.clone() }
                    }
                }
            }
        }
        _ => {
            rsx! {
                div { class: "slate-content",
                    p { class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        ""
                    }
                }
            }
        }
    }
}

#[component]
fn SlateBlock(block: serde_json::Value) -> Element {
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

    match block_type {
        "heading-one" | "h1" => rsx! { h1 { {rendered_children} } },
        "heading-two" | "h2" => rsx! { h2 { {rendered_children} } },
        "heading-three" | "h3" => rsx! { h3 { {rendered_children} } },
        "heading-four" | "h4" => rsx! { h4 { {rendered_children} } },
        "heading-five" | "h5" => rsx! { h5 { {rendered_children} } },
        "heading-six" | "h6" => rsx! { h6 { {rendered_children} } },
        "block-quote" => rsx! { blockquote { {rendered_children} } },
        "block-pre" | "code" => rsx! { pre { {rendered_children} } },
        "bulleted-list" | "ul" => rsx! { ul { {rendered_children} } },
        "numbered-list" | "ol" => rsx! { ol { {rendered_children} } },
        "list-item" | "li" => rsx! { li { {rendered_children} } },
        "image" => {
            let url = block.get("url").and_then(|u| u.as_str()).unwrap_or("");
            rsx! {
                img { src: "{url}", alt: "content image" }
            }
        }
        _ => rsx! { p { {rendered_children} } },
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

        if code {
            return rsx! {
                code { "{text}" }
            };
        }

        if style.is_empty() {
            return rsx! { "{text}" };
        }

        return rsx! {
            span { style: "{style}", "{text}" }
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
