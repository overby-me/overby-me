//! Browser side of the WebXR Wayland compositor.
//!
//! Scaffold: a static status page. The WebSocket session, window canvases
//! and input capture come next.

use dioxus::prelude::*;
use webxr_compositor_protocol as protocol;

const MAIN_CSS: Asset = asset!("/assets/main.css");

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: MAIN_CSS }
        main {
            h1 { "webxr-compositor" }
            p { class: "tagline", "Wayland apps in the browser: flat now, XR later." }
            p { class: "status", "protocol v{protocol::VERSION}, host link not wired yet" }
        }
    }
}
