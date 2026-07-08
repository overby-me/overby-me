mod graph;
mod pages;

use dioxus::prelude::*;
use pages::{Index, Search, X, Yt};

const MAIN_CSS: Asset = asset!("/assets/main.css");

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
        document::Stylesheet { href: MAIN_CSS }
        Router::<Route> {}
    }
}
