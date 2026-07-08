mod components;
#[allow(dead_code)]
mod graphql;
#[allow(dead_code)]
mod i18n;
#[allow(dead_code)]
mod nhost;
mod route;
#[allow(dead_code)]
mod session;
pub mod snackbar;
#[allow(dead_code)]
mod theme;

use dioxus::prelude::*;
use route::Route;

const STYLE_CSS: Asset = asset!("/assets/style.css");

fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    log::info!("RadikalWiki starting...");

    // Load persisted session from localStorage
    session::load_session();

    // Detect browser language for i18n
    if let Some(window) = web_sys::window() {
        if let Some(lang) = window.navigator().language() {
            if lang.starts_with("da") {
                *i18n::LANG.write() = i18n::Lang::Da;
            }
        }
    }

    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: STYLE_CSS }
        Router::<Route> {}
        snackbar::Snackbar {}
    }
}
