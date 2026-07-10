use dioxus::prelude::*;

use crate::graph::Graph;
use crate::graph::data::GraphData;

#[component]
pub fn Index() -> Element {
    rsx! {
        a { rel: "me", href: "https://mas.to/@niclasoverby" }
        Graph { data: GraphData::personal() }
    }
}
