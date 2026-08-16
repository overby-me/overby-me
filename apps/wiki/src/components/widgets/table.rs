//! M3 data tables: a semantic `<table>` scaffold and a generic server-paginated
//! table (search + filter chips + prev/next). Both domain-free.

use dioxus::prelude::*;

/// An M3 data table: a horizontally scrollable semantic `<table>` with a quiet
/// surface-container header row and tonal row dividers. `columns` are the header
/// labels; pass the `<tr>` body rows as `children`.
#[component]
pub fn DataTable(columns: Vec<String>, children: Element) -> Element {
    rsx! {
        div { class: "m3-data-table-wrap",
            table { class: "m3-data-table",
                thead {
                    tr {
                        for col in columns {
                            th { key: "{col}", "{col}" }
                        }
                    }
                }
                tbody { {children} }
            }
        }
    }
}

/// A search field and a row of single-select filter chips.
///
/// Extracted from [`PaginatedTable`] so a list can filter the same way a table
/// does. It was worth doing the moment a second screen needed filtering: the two
/// would otherwise have looked alike only for as long as someone kept them so.
///
/// Owns nothing — the parent binds `search` and `filter` and decides what they
/// mean. `on_change` fires after either, which is how the table resets its page
/// without this knowing pages exist. `trailing` takes anything else the screen
/// needs in the bar, such as a date range.
#[component]
pub fn FilterToolbar(
    search: Signal<String>,
    filter: Signal<String>,
    #[props(default)] filters: Vec<(String, String)>,
    search_placeholder: String,
    #[props(default)] on_change: EventHandler<()>,
    #[props(default)] trailing: Option<Element>,
) -> Element {
    let mut search = search;
    let mut filter = filter;
    let active = filter.read().clone();
    rsx! {
        div { class: "paginated-table-toolbar",
            label { class: "search-field",
                span { class: "material-icons", "search" }
                input {
                    r#type: "text",
                    placeholder: "{search_placeholder}",
                    value: "{search}",
                    oninput: move |e| {
                        search.set(e.value());
                        on_change.call(());
                    },
                }
            }
            if !filters.is_empty() {
                div { class: "filter-chips", role: "group",
                    for (value , label) in filters.iter().cloned() {
                        {
                            let selected = value == active;
                            let v = value.clone();
                            rsx! {
                                button {
                                    key: "{value}",
                                    r#type: "button",
                                    class: if selected { "m3-filter-chip selected" } else { "m3-filter-chip" },
                                    "aria-pressed": if selected { "true" } else { "false" },
                                    onclick: move |_| {
                                        filter.set(v.clone());
                                        on_change.call(());
                                    },
                                    if selected {
                                        span { class: "material-icons", "check" }
                                    }
                                    "{label}"
                                }
                            }
                        }
                    }
                }
            }
            if let Some(trailing) = trailing {
                {trailing}
            }
        }
    }
}

/// A generic, wiki-agnostic paginated data table: a search field, an optional row
/// of single-select filter chips, a [`DataTable`] body, and a footer with a range
/// label and prev/next controls. It owns no data — the parent passes the current
/// page's rows (`children`), the `total` matching count, and the bound `search` /
/// `filter` / `page` signals, and (re)fetches server-side when they change. The
/// search field and each filter chip reset `page` to 0. Reusable for any
/// server-paginated list.
#[component]
pub fn PaginatedTable(
    columns: Vec<String>,
    #[props(default)] filters: Vec<(String, String)>,
    search: Signal<String>,
    filter: Signal<String>,
    page: Signal<usize>,
    page_size: usize,
    total: usize,
    search_placeholder: String,
    prev_label: String,
    next_label: String,
    children: Element,
) -> Element {
    let mut page = page;
    let page_count = total.div_ceil(page_size).max(1);
    let cur = (*page.read()).min(page_count - 1);
    let first = if total == 0 { 0 } else { cur * page_size + 1 };
    let last = ((cur + 1) * page_size).min(total);
    rsx! {
        div { class: "paginated-table",
            FilterToolbar {
                search,
                filter,
                filters,
                search_placeholder,
                // Any change starts the results again from the first page; the
                // toolbar itself knows nothing about paging.
                on_change: move |_| page.set(0),
            }
            DataTable { columns, {children} }
            div { class: "paginated-table-footer",
                span { class: "paginated-count body-medium", "{first}-{last} / {total}" }
                div { class: "pagination-controls",
                    button {
                        class: "btn-icon",
                        r#type: "button",
                        disabled: cur == 0,
                        title: "{prev_label}",
                        aria_label: "{prev_label}",
                        onclick: move |_| page.set(cur.saturating_sub(1)),
                        span { class: "material-icons", "chevron_left" }
                    }
                    span { class: "body-medium page-indicator", "{cur + 1} / {page_count}" }
                    button {
                        class: "btn-icon",
                        r#type: "button",
                        disabled: cur + 1 >= page_count,
                        title: "{next_label}",
                        aria_label: "{next_label}",
                        onclick: move |_| page.set((cur + 1).min(page_count - 1)),
                        span { class: "material-icons", "chevron_right" }
                    }
                }
            }
        }
    }
}
