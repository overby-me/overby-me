// Canvas geometry and random picks cast counts to f64; everything is far below
// 2^52, so the casts are exact.
#![allow(clippy::cast_precision_loss)]
mod cardioid;
mod ui;
mod url;

use dioxus::prelude::*;

const MAIN_CSS: Asset = asset!("/assets/main.css");
// Self-hosted Space Grotesk (latin subset, variable 300–700). The @font-face is
// injected inline so its src points at the hashed, dx-bundled asset URL (dx does
// not rewrite url() inside plain CSS files).
const SPACE_GROTESK: Asset = asset!("/assets/fonts/space-grotesk-latin.woff2");

fn main() {
    url::capture_url_query();
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
        cardioid::Cardioid {}
    }
}
