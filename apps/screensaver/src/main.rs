// Canvas geometry and random picks cast counts to f64; everything is far below
// 2^52, so the casts are exact. The lint stays on where truncation is real
// (decoders, wire formats).
#![allow(clippy::cast_precision_loss)]
mod atproto;
mod glyphs;
mod images;
mod pages;
mod text;
mod tiles;
mod url;

use dioxus::prelude::*;
use pages::{Screensaver, ScreensaverRandom};

// No `asset!` in this app: with the savers split into chunks, every `asset!`
// resolves to "" (measured: `<link href="">`, an empty font `url()`). The
// stylesheet is inlined and the justfile copies the font to a fixed path.
const MAIN_CSS: &str = include_str!("../assets/main.css");
// Self-hosted Space Grotesk (latin subset, variable 300–700).
const FONT_FACE: &str = "@font-face{font-family:'Space Grotesk';font-style:normal;font-weight:300 700;font-display:swap;src:url(/fonts/space-grotesk-latin.woff2) format('woff2')}";

#[derive(Routable, Clone, PartialEq, Debug)]
pub enum Route {
    #[route("/")]
    ScreensaverRandom {},
    #[route("/:name")]
    Screensaver { name: String },
}

fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    url::capture_url_query();
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Style { {FONT_FACE} }
        document::Style { {MAIN_CSS} }
        Router::<Route> {}
    }
}
