mod graph;
mod pages;

use dioxus::prelude::*;
use pages::{Index, Search, X, Yt};

const MAIN_CSS: Asset = asset!("/assets/main.css");
// Self-hosted Space Grotesk (latin subset, variable 300–700). The @font-face is
// injected inline so its src points at the hashed, dx-bundled asset URL (dx does
// not rewrite url() inside plain CSS files).
const SPACE_GROTESK: Asset = asset!("/assets/fonts/space-grotesk-latin.woff2");

#[derive(Routable, Clone, PartialEq, Debug)]
enum Route {
    #[route("/")]
    Index {},
    #[route("/search?:url")]
    Search { url: String },
    #[route("/x?:url")]
    X { url: String },
    #[route("/yt?:url")]
    Yt { url: String },
}

fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Style {
            {format!(
                "@font-face{{font-family:'Space Grotesk';font-style:normal;font-weight:300 700;font-display:swap;src:url({SPACE_GROTESK}) format('woff2')}}"
            )}
        }
        document::Stylesheet { href: MAIN_CSS }
        Router::<Route> {}
    }
}
