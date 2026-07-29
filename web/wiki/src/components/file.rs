use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;

use super::loader::node_icon_el;

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

/// The state of the backend-signed link the Office viewer fetches on. Kept
/// apart from "no URL yet" so a refusal shows the no-preview state instead of a
/// spinner that never stops.
#[derive(Clone, PartialEq)]
enum OfficeLink {
    Pending,
    Ready(String),
    Refused,
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
    // Deleting walks the file's comment subtree a request at a time.
    let mut deleting = use_signal(|| false);

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
    // DESIGN (format-aware file card): retint the card accent by format
    // (video/audio -> magenta tertiary, docs -> secondary, else primary) and show
    // a short type chip. Reuses the .card --card-accent system.
    let accent = if file_mime.starts_with("video") || file_mime.starts_with("audio") {
        "accent-tertiary"
    } else if file_mime.contains("pdf")
        || file_mime.contains("word")
        || file_mime.contains("sheet")
        || file_mime.contains("presentation")
        || file_mime.contains("officedocument")
    {
        "accent-secondary"
    } else {
        ""
    };
    let type_label = file_mime
        .rsplit('/')
        .next()
        .unwrap_or("")
        .split('+')
        .next()
        .unwrap_or("")
        .to_uppercase();

    // Presigned, not a bare storage URL: this feeds an <iframe>, <video>, <audio>
    // and a download <a>, none of which can send the Authorization header the
    // storage service reads. Empty until it resolves, which the branches below
    // already treat as "no preview yet".
    let file_url = super::loader::use_presigned_url(file_id.to_string()).unwrap_or_default();

    // The Office viewer needs its own link (see the branch that uses it), fetched
    // only for the mimes that go through it.
    let mut office_embed = use_signal(|| OfficeLink::Pending);
    {
        let id = file_id.to_string();
        let wanted = is_office_mime(file_mime);
        let token = session.read().access_token.clone();
        use_effect(use_reactive!(|(id, wanted, token)| {
            office_embed.set(OfficeLink::Pending);
            if !wanted || id.is_empty() {
                return;
            }
            let Some(token) = token.clone() else {
                office_embed.set(OfficeLink::Refused);
                return;
            };
            spawn(async move {
                office_embed.set(
                    match crate::backend_api::office_embed_url(&id, &token).await {
                        Some(url) => OfficeLink::Ready(url),
                        None => OfficeLink::Refused,
                    },
                );
            });
        }));
    }

    rsx! {
        super::widgets::SupportingPaneLayout {
            // Primary pane: the file's identity header above the file itself, so the
            // title / date / tools sit atop the content rather than below it.
            primary: rsx! {
                div { class: "card file-card {accent}",
                    div { class: "card-header",
                        div { class: "avatar", {node_icon_el("wiki/file", data.as_ref())} }
                        div {
                            h3 { class: "title-medium", "{name}" }
                            div { class: "file-meta-chips",
                                if !type_label.is_empty() {
                                    span { class: "file-chip",
                                        span { class: "material-icons", "description" }
                                        "{type_label}"
                                    }
                                }
                                if let Some(iso) = created.as_ref() {
                                    span {
                                        class: "file-chip",
                                        title: "{super::loader::full_datetime(iso)}",
                                        span { class: "material-icons", "schedule" }
                                        "{super::loader::relative_time(iso)}"
                                    }
                                }
                            }
                        }
                        div { class: "flex-grow" }
                        // File actions in the M3 tools sheet.
                        if !file_url.is_empty() || (can_manage && !segments.is_empty()) {
                            super::widgets::ToolSheet {
                                title: t("common.tools"),
                                // Pinned quick group: copy link (the sheet's own
                                // first segment) and downloading the file itself.
                                quick: rsx! {
                                    if !file_url.is_empty() {
                                        a {
                                            href: "{file_url}",
                                            target: "_blank",
                                            download: "{name}",
                                            "referrerpolicy": "no-referrer",
                                            class: "sheet-quick-action",
                                            title: "{t(\"common.download\")}",
                                            aria_label: "{t(\"common.download\")}",
                                            span { class: "material-icons", "download" }
                                        }
                                    }
                                },
                                if can_manage && !segments.is_empty() {
                                    super::widgets::SheetGroup { danger: true,
                                        button {
                                            class: "sheet-action danger",
                                            onclick: move |_| confirm_open.set(true),
                                            span { class: "material-icons", "delete" }
                                            "{t(\"common.delete\")}"
                                        }
                                    }
                                }
                            }
                        }
                        // Delete confirm dialog (owner-only).
                        if can_manage && !segments.is_empty() {
                            super::widgets::Dialog {
                                open: confirm_open(),
                                on_dismiss: move |_| confirm_open.set(false),
                                headline: t("content.confirmDelete"),
                                icon: "delete".to_string(),
                                actions: rsx! {
                                    button {
                                        class: "btn btn-outlined",
                                        onclick: move |_| confirm_open.set(false),
                                        "{t(\"common.cancel\")}"
                                    }
                                    button {
                                        class: "btn btn-primary",
                                        disabled: deleting(),
                                        onclick: {
                                            let node_id = node_id.clone();
                                            let parent = segments[..segments.len() - 1].to_vec();
                                            move |_| {
                                                if deleting() {
                                                    return;
                                                }
                                                let token = session.read().access_token.clone();
                                                let node_id = node_id.clone();
                                                let parent = parent.clone();
                                                // Dialog stays open so the spinner has somewhere
                                                // to be while the subtree is walked.
                                                deleting.set(true);
                                                spawn(async move {
                                                    // Subtree and member rows together — nothing
                                                    // cascades in the database, so a comment left
                                                    // under a deleted file is unreachable forever.
                                                    match graphql::delete_node_deep(token, node_id).await {
                                                        Ok(()) => {
                                                            crate::session::bump_data_version();
                                                            deleting.set(false);
                                                            confirm_open.set(false);
                                                            nav.push(Route::PathPage { segments: parent, app: None });
                                                        }
                                                        other => {
                                                            log::error!("delete_node failed: {other:?}");
                                                            deleting.set(false);
                                                            crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                                        }
                                                    }
                                                });
                                            }
                                        },
                                        if deleting() {
                                            div { class: "spinner spinner-xs" }
                                        }
                                        "{t(\"common.delete\")}"
                                    }
                                },
                                p { class: "body-medium", "{name}" }
                            }
                        }
                    }
                    div { class: "file-viewer",
                        if file_url.is_empty() {
                            p { class: "body-medium", "{t(\"common.noContent\")}" }
                        } else if file_mime.starts_with("image/") {
                            super::widgets::ZoomableImage { src: file_url.clone(), alt: name.to_string() }
                        } else if file_mime.starts_with("video/") {
                            video {
                                controls: true,
                                "referrerpolicy": "no-referrer",
                                src: "{file_url}",
                            }
                        } else if file_mime.starts_with("audio/") {
                            audio { controls: true, "referrerpolicy": "no-referrer", src: "{file_url}" }
                        } else if file_mime == "application/pdf" {
                            // DESIGN: frame document previews like the map/graph.
                            div { class: "viewport-frame",
                                iframe { src: "{file_url}", title: "{name}", "referrerpolicy": "no-referrer" }
                            }
                        } else if is_office_mime(file_mime) {
                            // Word/Excel/PowerPoint through Microsoft's hosted
                            // viewer, which fetches the document from ITS servers.
                            // So it gets a backend link, not a storage URL: the
                            // backend checked the caller may read the file and
                            // serves the bytes for a couple of hours. A presigned
                            // storage URL would be dead 30 seconds later.
                            match office_embed() {
                                OfficeLink::Pending => rsx! {
                                    div { class: "empty-state empty-state-sm",
                                        div { class: "spinner spinner-sm" }
                                    }
                                },
                                OfficeLink::Ready(url) => {
                                    let encoded = String::from(&js_sys::encode_uri_component(&url));
                                    rsx! {
                                        div { class: "viewport-frame",
                                            iframe {
                                                src: "https://view.officeapps.live.com/op/embed.aspx?src={encoded}",
                                                title: "{name}",
                                            }
                                        }
                                    }
                                }
                                OfficeLink::Refused => rsx! {
                                    p { class: "body-medium", "{t(\"common.noPreview\")}" }
                                },
                            }
                        } else {
                            // DESIGN: a rich "no preview" state (format orb +
                            // prominent download) instead of a bare button.
                            div { class: "empty-state empty-state-sm",
                                div { class: "empty-state-orb empty-state-orb-sm",
                                    {node_icon_el("wiki/file", data.as_ref())}
                                }
                                p { class: "empty-state-body", "{t(\"common.noPreview\")}" }
                                a {
                                    href: "{file_url}",
                                    target: "_blank",
                                    "referrerpolicy": "no-referrer",
                                    class: "btn btn-primary",
                                    span { class: "material-icons", "download" }
                                    " {t(\"common.download\")}"
                                }
                            }
                        }
                    }
                }
            },
            // Supporting pane: the discussion.
            supporting: rsx! {
                super::comments::CommentSection {
                    node_id: node_id.clone(),
                    context_id: context_id.clone(),
                }
            },
        }
    }
}
