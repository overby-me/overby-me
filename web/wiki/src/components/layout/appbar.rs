use dioxus::prelude::*;

use super::*;
use crate::i18n::t;
use crate::model::NodeFields;

/// M3 top app bar over the content region: a leading menu button (compact, opens
/// the tree drawer), the breadcrumb trail as the headline, and trailing search +
/// user menu. The docked search bar expands in place.
#[component]
pub(super) fn TopAppBar(
    search_mode: Signal<bool>,
    search_input: Signal<String>,
    search_results: Signal<Vec<NodeFields>>,
    open_drawer: Signal<bool>,
) -> Element {
    let size = crate::window_size::WINDOW_SIZE();

    rsx! {
        header { class: "top-app-bar",
            // Cap the bar content at the reading-column width and centre it so the
            // search bar and breadcrumbs line up with the content column below.
            div { class: "top-app-bar-inner",
                if *search_mode.read() {
                    SearchBar {
                        input: search_input,
                        results: search_results,
                        on_close: move |_| {
                            search_mode.set(false);
                            search_input.set(String::new());
                            search_results.set(vec![]);
                        },
                    }
                } else {
                    // M3 Expressive docked search: a rounded container carrying the
                    // breadcrumb trail. On compact the drawer button is integrated as
                    // the leading affordance; the search button trails on the right and
                    // opens full search. The crumbs stay tappable for up-navigation.
                    div { class: "expressive-search",
                        if size.is_compact() {
                            button {
                                // A pending invitation badges the button that leads
                                // to it (drawer, then the place picker holding the
                                // invitation itself), so it is visible from every
                                // page without the bar spending a slot on it next
                                // to the crumbs.
                                class: "expressive-search-btn menu-trigger state-layer",
                                aria_label: t("common.menu"),
                                onclick: move |_| open_drawer.set(true),
                                NavBadge {
                                    span { class: "material-icons", "menu" }
                                }
                            }
                        }
                        div { class: "expressive-search-crumbs", Breadcrumbs {} }
                        button {
                            class: "expressive-search-btn state-layer",
                            aria_label: t("common.search"),
                            onclick: move |_| search_mode.set(true),
                            span { class: "material-icons", "search" }
                        }
                    }
                }
            }
        }
    }
}
