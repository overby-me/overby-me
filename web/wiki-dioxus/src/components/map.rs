use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;

use super::loader::icon_el;

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
                div { class: "avatar", {icon_el("map/map")} }
                h3 { class: "title-medium", "{name}" }
            }
            // EXPERIMENT (framed viewport): a tonal rounded frame around the embed.
            div { class: "viewport-frame",
                iframe {
                    src: "{src}",
                    style: "width: 100%; height: 78vh; border: 0; border-radius: var(--md-sys-shape-corner-medium); display: block;",
                    title: "{name}",
                }
            }
        }
    }
}
