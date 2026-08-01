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

/// Who renders a Word, Excel or PowerPoint file.
///
/// Neither is us: this app cannot render OpenXML, so a preview means handing the
/// document to somebody who can. Both fetch it from the signed backend link, so
/// the choice is only WHICH third party sees it — worth being able to answer,
/// since the answer used to be "Microsoft, always".
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum OfficeViewer {
    Microsoft,
    Google,
}

impl OfficeViewer {
    fn key(self) -> &'static str {
        match self {
            OfficeViewer::Microsoft => "microsoft",
            OfficeViewer::Google => "google",
        }
    }

    fn from_key(key: &str) -> Self {
        match key {
            "google" => OfficeViewer::Google,
            // Microsoft is what everybody had before the choice existed, so an
            // unset or unreadable preference keeps what they were used to.
            _ => OfficeViewer::Microsoft,
        }
    }
}

/// Where the chosen viewer is asked to render `encoded_src`, which is the signed
/// backend link, URI-component encoded.
///
/// Pure and separate from the component so the two URL shapes can be tested; a
/// wrong parameter name here is a viewer that shows an error page, and neither
/// service tells you which parameter it wanted.
pub fn viewer_embed_url(viewer: OfficeViewer, encoded_src: &str) -> String {
    match viewer {
        OfficeViewer::Microsoft => {
            format!("https://view.officeapps.live.com/op/embed.aspx?src={encoded_src}")
        }
        // `gview` is the free one, and the reason this choice exists. It wants
        // `url`, not `src`, and `embedded=true` or it serves a full page with
        // Google chrome around it.
        OfficeViewer::Google => {
            format!("https://docs.google.com/gview?embedded=true&url={encoded_src}")
        }
    }
}

/// The chosen viewer, remembered per device.
///
/// A device preference rather than an account one: it is about which service you
/// are willing to send a document to on the machine in front of you, and storing
/// it on the account would need a schema and a migration to say less.
pub static OFFICE_VIEWER: GlobalSignal<OfficeViewer> = Signal::global(|| {
    web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("wiki_office_viewer").ok().flatten())
        .map(|v| OfficeViewer::from_key(&v))
        .unwrap_or(OfficeViewer::Microsoft)
});

/// Choose a viewer, and remember it.
pub fn set_office_viewer(viewer: OfficeViewer) {
    *OFFICE_VIEWER.write() = viewer;
    if let Some(storage) = web_sys::window().and_then(|w| w.local_storage().ok().flatten()) {
        let _ = storage.set_item("wiki_office_viewer", viewer.key());
    }
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

    /// Both viewers, spelled the way each service actually wants. Microsoft
    /// takes `src`; Google takes `url` and needs `embedded=true` or it returns a
    /// whole Google page rather than the document.
    #[test]
    fn each_viewer_gets_the_parameters_it_wants() {
        use super::{viewer_embed_url, OfficeViewer};
        let src = "https%3A%2F%2Fapi.example%2Foffice%2Ffile%3Ff%3D1%26s%3Dabc";
        let ms = viewer_embed_url(OfficeViewer::Microsoft, src);
        assert!(
            ms.starts_with("https://view.officeapps.live.com/op/embed.aspx?src="),
            "{ms}"
        );
        assert!(ms.ends_with(src), "the signed link travels whole: {ms}");

        let g = viewer_embed_url(OfficeViewer::Google, src);
        assert!(g.starts_with("https://docs.google.com/gview?"), "{g}");
        assert!(g.contains("embedded=true"), "or it is not an embed: {g}");
        assert!(g.ends_with(&format!("url={src}")), "{g}");
        assert!(
            !g.contains("src="),
            "gview ignores src, which would render nothing: {g}"
        );
    }

    /// The stored preference round-trips, and anything else is Microsoft — which
    /// is what every reader had before this choice existed, so an unset or
    /// corrupt value must not change what they see.
    #[test]
    fn an_unknown_preference_falls_back_to_what_people_had() {
        use super::OfficeViewer;
        assert_eq!(
            OfficeViewer::from_key(OfficeViewer::Google.key()),
            OfficeViewer::Google
        );
        assert_eq!(
            OfficeViewer::from_key(OfficeViewer::Microsoft.key()),
            OfficeViewer::Microsoft
        );
        for junk in ["", "libreoffice", "GOOGLE", "null"] {
            assert_eq!(
                OfficeViewer::from_key(junk),
                OfficeViewer::Microsoft,
                "{junk:?}"
            );
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
    // What binning needs: the path stamps the subtree, the actor records who.
    let node_path = node.path.clone();
    let actor = session.read().user.as_ref().map(|u| u.id.clone());
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
                                // Who renders it. Only for the mimes that go
                                // through a third party at all — offering it on
                                // a PDF or an image would be offering a choice
                                // that changes nothing.
                                if is_office_mime(file_mime) {
                                    super::widgets::SheetGroup {
                                        div { class: "sheet-label", "{t(\"file.renderedBy\")}" }
                                        for (viewer , label) in [
                                            (OfficeViewer::Microsoft, t("file.viewerMicrosoft")),
                                            (OfficeViewer::Google, t("file.viewerGoogle")),
                                        ] {
                                            button {
                                                key: "{viewer.key()}",
                                                class: if OFFICE_VIEWER() == viewer { "sheet-action selected" } else { "sheet-action" },
                                                "aria-pressed": if OFFICE_VIEWER() == viewer { "true" } else { "false" },
                                                onclick: move |_| set_office_viewer(viewer),
                                                span { class: "material-icons",
                                                    if OFFICE_VIEWER() == viewer { "radio_button_checked" } else { "radio_button_unchecked" }
                                                }
                                                "{label}"
                                            }
                                        }
                                    }
                                }
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
                                headline: t("content.confirmDeleteBin"),
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
                                            let node_path = node_path.clone();
                                            let actor = actor.clone();
                                            move |_| {
                                                if deleting() {
                                                    return;
                                                }
                                                let token = session.read().access_token.clone();
                                                let node_id = node_id.clone();
                                                let parent = parent.clone();
                                                let node_path = node_path.clone();
                                                let actor = actor.clone();
                                                // Dialog stays open so the spinner has somewhere
                                                // to be while the statement runs.
                                                deleting.set(true);
                                                spawn(async move {
                                                    // To the bin, like a document or a folder:
                                                    // deleting a file from its own page used to be
                                                    // final while deleting the folder around it was
                                                    // recoverable, which made the promise depend on
                                                    // which page you happened to be looking at.
                                                    // The stored object is untouched either way, so
                                                    // a restored file is a working file.
                                                    match graphql::bin_node(
                                                        token.as_deref(),
                                                        &node_id,
                                                        node_path.as_deref(),
                                                        actor.as_deref(),
                                                    )
                                                    .await
                                                    {
                                                        Ok(_) => {
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
                                p { class: "body-medium text-muted", "{t(\"content.deleteRecoverable\")}" }
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
                                    let embed = viewer_embed_url(OFFICE_VIEWER(), &encoded);
                                    rsx! {
                                        div { class: "viewport-frame",
                                            iframe {
                                                // Keyed on the viewer: swapping
                                                // service must reload the frame,
                                                // and a changed `src` alone does
                                                // not reliably do that.
                                                key: "{OFFICE_VIEWER().key()}",
                                                src: "{embed}",
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
