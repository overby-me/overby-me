use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;

use super::loader::mime_icon;

/// MapApp — a full-height OpenStreetMap view (`?app=map`), mirroring the React
/// MapApp's MapLibre map centred on Denmark. Uses OSM's embed rather than
/// pulling a WebGL map library into the wasm bundle.
#[component]
pub fn MapApp(node: NodeWithChildren) -> Element {
    let name = node.name.clone();
    // Bounding box roughly covering Denmark (the React map centres near 10.2E,
    // 56.2N at zoom 6.5).
    let src = "https://www.openstreetmap.org/export/embed.html?bbox=7.0%2C54.5%2C15.5%2C57.9&layer=mapnik";

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", "{mime_icon(\"map/map\")}" }
                h3 { class: "title-medium", "{name}" }
            }
            iframe {
                src: "{src}",
                style: "width: 100%; height: 80vh; border: 0; border-radius: 0 0 12px 12px;",
                title: "{name}",
            }
        }
    }
}
