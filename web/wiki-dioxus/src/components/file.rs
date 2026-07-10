use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
use crate::nhost::storage_url;
use crate::route::Route;
use crate::session::use_session;

use super::loader::node_icon_el;
use super::ui::alert_dialog::{
    AlertDialog, AlertDialogAction, AlertDialogActions, AlertDialogCancel, AlertDialogDescription,
    AlertDialogTitle,
};

/// Office document mimes (legacy + OpenXML) previewable via the MS Office viewer:
/// Word, Excel and PowerPoint.
fn is_office_mime(mime: &str) -> bool {
    matches!(
        mime,
        "application/msword"
            | "application/vnd.openxmlformats-officedocument.wordprocessingml.document"
            | "application/vnd.ms-excel"
            | "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet"
            | "application/vnd.ms-powerpoint"
            | "application/vnd.openxmlformats-officedocument.presentationml.presentation"
    )
}

#[cfg(test)]
mod tests {
    use super::is_office_mime;

    #[test]
    fn office_mimes_are_previewable() {
        for m in [
            "application/msword",
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
            "application/vnd.ms-excel",
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
            "application/vnd.ms-powerpoint",
            "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        ] {
            assert!(
                is_office_mime(m),
                "{m} should preview via the office viewer"
            );
        }
    }

    #[test]
    fn non_office_mimes_are_not() {
        for m in ["application/pdf", "image/png", "text/plain", ""] {
            assert!(!is_office_mime(m));
        }
    }
}

#[component]
pub fn FileApp(node: NodeWithChildren) -> Element {
    let name = node.name.as_str();
    let session = use_session();
    let nav = use_navigator();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let created = node.created_at.as_ref().map(|t| t.0.clone());
    let node_id = node.id.0.clone();
    let context_id = node.context_id.clone().map(|c| c.0);
    // Owners may delete the file (node/context owner); mirrors ContentApp gating.
    let can_manage = node.is_owner.unwrap_or(false) || node.is_context_owner.unwrap_or(false);
    let mut confirm_open = use_signal(|| false);

    let data = node.data.map(|d| d.0);

    let file_id = data
        .as_ref()
        .and_then(|d| d.get("fileId"))
        .and_then(|f| f.as_str())
        .unwrap_or("");
    let file_mime = data
        .as_ref()
        .and_then(|d| d.get("type"))
        .and_then(|t| t.as_str())
        .unwrap_or("");

    let file_url = if !file_id.is_empty() {
        let token = session.read().access_token.clone().unwrap_or_default();
        format!("{}/files/{file_id}?token={token}", storage_url())
    } else {
        String::new()
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {node_icon_el("wiki/file", data.as_ref())} }
                div {
                    h3 { class: "title-medium", "{name}" }
                    if let Some(iso) = created.as_ref() {
                        p {
                            class: "body-small",
                            class: "text-muted",
                            title: "{super::loader::full_datetime(iso)}",
                            span { class: "material-icons", style: "font-size: 13px; vertical-align: middle;", "schedule" }
                            " {super::loader::relative_time(iso)}"
                        }
                    }
                }
                div { class: "flex-grow" }
                // Download the raw file (any previewable type), not just view it.
                if !file_url.is_empty() {
                    a {
                        href: "{file_url}",
                        target: "_blank",
                        download: "{name}",
                        class: "btn-icon",
                        title: "{t(\"common.download\")}",
                        span { class: "material-icons", "download" }
                    }
                }
                // Delete the file (owner-only), removing member rows first.
                if can_manage && !segments.is_empty() {
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
                                            let _ = graphql::delete_node_members(token.as_deref(), &node_id).await;
                                            if graphql::delete_node(token.as_deref(), &node_id).await.unwrap_or(false) {
                                                crate::session::bump_data_version();
                                                nav.push(Route::PathPage { segments: parent, app: None });
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
            div { class: "file-viewer",
                if file_url.is_empty() {
                    p { class: "body-medium", "No file attached" }
                } else if file_mime.starts_with("image/") {
                    super::widgets::ZoomableImage { src: file_url.clone(), alt: name.to_string() }
                } else if file_mime.starts_with("video/") {
                    video { controls: true, src: "{file_url}" }
                } else if file_mime.starts_with("audio/") {
                    audio { controls: true, src: "{file_url}" }
                } else if file_mime == "application/pdf" {
                    iframe { src: "{file_url}", title: "{name}" }
                } else if is_office_mime(file_mime) {
                    // Preview Word/Excel/PowerPoint via Microsoft's hosted viewer,
                    // which fetches the file URL server-side (mirrors the old wiki).
                    // The whole tokenised URL is percent-encoded into `src`.
                    {
                        let encoded = String::from(&js_sys::encode_uri_component(&file_url));
                        rsx! {
                            iframe {
                                src: "https://view.officeapps.live.com/op/embed.aspx?src={encoded}",
                                title: "{name}",
                            }
                        }
                    }
                } else {
                    a {
                        href: "{file_url}",
                        target: "_blank",
                        class: "btn btn-outlined",
                        span { class: "material-icons", "download" }
                        " Download {name}"
                    }
                }
            }
        }
        // A file can be discussed too (React FileApp carries a discussion below
        // the viewer).
        super::comments::CommentSection {
            node_id: node_id.clone(),
            context_id: context_id.clone(),
        }
    }
}
