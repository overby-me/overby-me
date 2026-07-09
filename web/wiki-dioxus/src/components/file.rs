use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;
use crate::nhost::storage_url;
use crate::session::use_session;

use super::loader::icon_el;

#[component]
pub fn FileApp(node: NodeWithChildren) -> Element {
    let name = node.name.as_str();
    let session = use_session();

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
                div { class: "avatar", {icon_el("wiki/file")} }
                h3 { class: "title-medium", "{name}" }
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
                } else {
                    a {
                        href: "{file_url}",
                        target: "_blank",
                        class: "btn btn-outlined",
                        "\u{1F4E5} Download {name}"
                    }
                }
            }
        }
    }
}
