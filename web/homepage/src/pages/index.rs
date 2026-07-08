use dioxus::prelude::*;

use crate::graph::Graph;

#[component]
pub fn Index() -> Element {
    rsx! {
        a { rel: "me", href: "https://mas.to/@niclasoverby" }
        Graph {}
    }
}
