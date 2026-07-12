//! Small app-agnostic UI primitives that dioxus-components does not ship. Kept
//! wiki-free (no GraphQL / domain knowledge) so they could move to a shared
//! crate or be upstreamed. Screen components compose these plus the
//! dioxus-primitives in `components::ui`.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

/// The currently focused element — captured when a modal opens so focus can be
/// returned to the trigger on close (keyboard accessibility).
pub(crate) fn active_html_element() -> Option<web_sys::HtmlElement> {
    web_sys::window()?
        .document()?
        .active_element()?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()
}

/// Close a modal surface (sheet/dialog) and return focus to whatever was focused
/// when it opened (`return_focus`).
pub(crate) fn close_modal(
    mut open: Signal<bool>,
    return_focus: Signal<Option<web_sys::HtmlElement>>,
) {
    open.set(false);
    if let Some(el) = return_focus.read().clone() {
        let _ = el.focus();
    }
}

/// Dismiss a controlled dialog (via its `on_dismiss` handler) and return focus to
/// whatever was focused when it opened.
pub(crate) fn dialog_dismiss(
    on_dismiss: EventHandler<()>,
    return_focus: Signal<Option<web_sys::HtmlElement>>,
) {
    on_dismiss.call(());
    if let Some(el) = return_focus.read().clone() {
        let _ = el.focus();
    }
}

/// Trap Tab focus within the open modal matched by `container_sel`. Called from a
/// Tab keydown handler; returns true when it wrapped focus (the caller should then
/// `prevent_default`), keeping keyboard focus inside the modal.
pub(crate) fn trap_tab_focus(container_sel: &str, shift: bool) -> bool {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return false;
    };
    let Some(container) = doc.query_selector(container_sel).ok().flatten() else {
        return false;
    };
    let Ok(nodes) = container.query_selector_all(
        "a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]",
    ) else {
        return false;
    };
    let mut items: Vec<web_sys::HtmlElement> = Vec::new();
    for i in 0..nodes.length() {
        let Some(el) = nodes
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            continue;
        };
        // Skip the off-screen focus sentinel and hidden controls.
        if el.get_attribute("tabindex").as_deref() == Some("-1") || el.offset_parent().is_none() {
            continue;
        }
        items.push(el);
    }
    let (Some(first), Some(last)) = (items.first(), items.last()) else {
        return false;
    };
    let active_html = doc
        .active_element()
        .and_then(|a| a.dyn_into::<web_sys::HtmlElement>().ok());
    let no_active = active_html.is_none();
    let matches = |t: &web_sys::HtmlElement| active_html.as_ref() == Some(t);
    if shift {
        if matches(first) || no_active {
            let _ = last.focus();
            return true;
        }
    } else if matches(last) || no_active {
        let _ = first.focus();
        return true;
    }
    false
}

/// A centred loading spinner overlay.
#[component]
pub fn Spinner() -> Element {
    rsx! {
        div { class: "spinner-overlay",
            div { class: "spinner" }
        }
    }
}

/// A small outlined chip matching the old wiki's MUI outlined-secondary Chip: an
/// optional leading Material icon (the mime icon) plus a label.
#[component]
pub fn Chip(icon: Option<String>, label: String, title: Option<String>) -> Element {
    rsx! {
        span { class: "chip", title: title.unwrap_or_default(),
            if let Some(icon) = icon {
                span { class: "material-icons", "{icon}" }
            }
            span { class: "chip-label", "{label}" }
        }
    }
}

/// M3 badge overlaying the top-trailing corner of a positioned parent: a small
/// dot (`count` `None`/`0`) or a large numeric pill (`count > 0`, capped 999+).
/// The parent must establish a positioning context (e.g. `position: relative`).
#[component]
pub fn Badge(count: Option<usize>) -> Element {
    match count {
        Some(n) if n > 0 => {
            let label = if n > 999 {
                "999+".to_string()
            } else {
                n.to_string()
            };
            rsx! {
                span { class: "md-badge", "aria-label": "{label}", "{label}" }
            }
        }
        _ => rsx! {
            span { class: "md-badge md-badge-dot" }
        },
    }
}

/// A slot-based Material 3 list item: an optional leading element (avatar/icon),
/// a headline, an optional supporting line, and an optional trailing element
/// (badge/action). Carries the `.list-item` state layer; pass `selected` for the
/// M3 selected treatment. Wrap in a `Link` (or add an `onclick`) for navigation.
#[component]
pub fn ListItem(
    headline: String,
    leading: Option<Element>,
    supporting: Option<String>,
    trailing: Option<Element>,
    #[props(default)] selected: bool,
) -> Element {
    rsx! {
        div { class: if selected { "list-item selected" } else { "list-item" },
            if let Some(leading) = leading {
                {leading}
            }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{headline}" }
                if let Some(supporting) = supporting {
                    div { class: "list-item-secondary", "{supporting}" }
                }
            }
            if let Some(trailing) = trailing {
                div { class: "list-item-trailing", {trailing} }
            }
        }
    }
}

/// An adaptive M3 supporting-pane / list-detail scaffold: the `primary` pane and
/// a `supporting` pane stack into one column on compact/medium and sit side by
/// side (roughly 2:1) on large+ — a pure-CSS responsive grid off the window
/// width, so foldables/split-window resolve for free.
#[component]
pub fn SupportingPaneLayout(primary: Element, supporting: Element) -> Element {
    rsx! {
        div { class: "m3-supporting-pane",
            div { class: "m3-pane-primary", {primary} }
            div { class: "m3-pane-supporting", {supporting} }
        }
    }
}

/// An M3 single-select segmented button: a connected row of icon segments with
/// the selected one filled (secondary-container). Emits the chosen value through
/// `on_select`. `segments` is a list of `(value, material-icon)` pairs.
#[component]
pub fn SegmentedButton(
    segments: Vec<(String, String)>,
    selected: String,
    on_select: EventHandler<String>,
) -> Element {
    rsx! {
        div { class: "m3-segmented", role: "group",
            for (value , icon) in segments {
                {
                    let is_sel = value == selected;
                    let chosen = value.clone();
                    rsx! {
                        button {
                            key: "{value}",
                            r#type: "button",
                            class: if is_sel { "m3-segment selected state-layer" } else { "m3-segment state-layer" },
                            "aria-pressed": if is_sel { "true" } else { "false" },
                            onclick: move |_| on_select.call(chosen.clone()),
                            if is_sel {
                                span { class: "material-icons m3-segment-check", "check" }
                            }
                            span { class: "material-icons", "{icon}" }
                        }
                    }
                }
            }
        }
    }
}

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
    let mut search = search;
    let mut filter = filter;
    let mut page = page;
    let page_count = total.div_ceil(page_size).max(1);
    let cur = (*page.read()).min(page_count - 1);
    let first = if total == 0 { 0 } else { cur * page_size + 1 };
    let last = ((cur + 1) * page_size).min(total);
    let active = filter.read().clone();
    rsx! {
        div { class: "paginated-table",
            div { class: "paginated-table-toolbar",
                label { class: "search-field",
                    span { class: "material-icons", "search" }
                    input {
                        r#type: "text",
                        placeholder: "{search_placeholder}",
                        value: "{search}",
                        oninput: move |e| {
                            search.set(e.value());
                            page.set(0);
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
                                            page.set(0);
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

/// An M3 tools/actions sheet: a trigger icon button that opens a sheet holding
/// the current view's actions and admin tools. The sheet is a bottom sheet on
/// compact screens and a right-anchored side sheet on medium+ (pure CSS off the
/// viewport width). Pass the action rows (`.sheet-action`) as `children`.
#[component]
pub fn ToolSheet(title: String, children: Element) -> Element {
    let mut open = use_signal(|| false);
    // Remember the trigger so focus returns to it when the sheet closes (a11y).
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);
    rsx! {
        button {
            class: "btn-icon state-layer",
            aria_label: "{title}",
            title: "{title}",
            onclick: move |_| {
                return_focus.set(active_html_element());
                open.set(true);
            },
            span { class: "material-icons", "tune" }
        }
        div {
            class: if open() { "sheet-scrim open" } else { "sheet-scrim" },
            role: "presentation",
            onclick: move |_| close_modal(open, return_focus),
        }
        aside {
            class: if open() { "tool-sheet open" } else { "tool-sheet" },
            role: "dialog",
            "aria-modal": "true",
            "aria-label": "{title}",
            tabindex: "-1",
            onkeydown: move |e| {
                match e.key() {
                    Key::Escape => close_modal(open, return_focus),
                    Key::Tab if trap_tab_focus(".tool-sheet.open", e.modifiers().shift()) => {
                        e.prevent_default();
                    }
                    _ => {}
                }
            },
            // Focus sentinel: mounts when the sheet opens and pulls focus into it
            // so Escape/Tab work and keyboard users land inside the sheet.
            if open() {
                div {
                    class: "sheet-focus-sentinel",
                    tabindex: "-1",
                    onmounted: move |e| {
                        spawn(async move {
                            let _ = e.set_focus(true).await;
                        });
                    },
                }
            }
            div { class: "tool-sheet-header",
                div { class: "sheet-handle" }
                h3 { class: "title-medium", "{title}" }
                button {
                    class: "btn-icon state-layer",
                    aria_label: "close",
                    onclick: move |_| close_modal(open, return_focus),
                    span { class: "material-icons", "close" }
                }
            }
            div {
                class: "tool-sheet-body",
                // Dismiss the sheet when an action inside it is chosen.
                onclick: move |_| close_modal(open, return_focus),
                {children}
            }
        }
    }
}

/// A reusable Material Design 3 basic dialog: a scrim over a surface-container-
/// high card (extra-large corners, level-3 elevation, spring rise-in) with an
/// optional leading icon, a headline, the passed `children` as content, and an
/// `actions` row. Dismisses on scrim click. Retires the ad-hoc
/// `.modal-backdrop`/`.modal-card` markup across screens.
#[component]
pub fn Dialog(
    open: bool,
    on_dismiss: EventHandler<()>,
    headline: String,
    actions: Element,
    icon: Option<String>,
    children: Element,
) -> Element {
    // Remember the trigger so focus returns to it on dismiss (a11y). Declared
    // before the early return so the hook order stays stable.
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);
    if !open {
        return rsx! {};
    }
    rsx! {
        div {
            class: "m3-dialog-scrim",
            role: "presentation",
            onclick: move |_| dialog_dismiss(on_dismiss, return_focus),
            div {
                class: "m3-dialog",
                role: "dialog",
                "aria-modal": "true",
                tabindex: "-1",
                onkeydown: move |e| {
                    match e.key() {
                        Key::Escape => dialog_dismiss(on_dismiss, return_focus),
                        Key::Tab if trap_tab_focus(".m3-dialog", e.modifiers().shift()) => {
                            e.prevent_default();
                        }
                        _ => {}
                    }
                },
                onclick: move |e| e.stop_propagation(),
                // Remember the trigger, then pull focus into the dialog on open (it
                // mounts only when open).
                div {
                    class: "sheet-focus-sentinel",
                    tabindex: "-1",
                    onmounted: move |e| {
                        return_focus.set(active_html_element());
                        spawn(async move {
                            let _ = e.set_focus(true).await;
                        });
                    },
                }
                if let Some(icon) = icon {
                    span { class: "m3-dialog-icon material-icons", "{icon}" }
                }
                h2 { class: "m3-dialog-headline", "{headline}" }
                div { class: "m3-dialog-content", {children} }
                div { class: "m3-dialog-actions", {actions} }
            }
        }
    }
}

/// A reusable Material Design 3 colour-swatch row: a label, preset swatches, and
/// a trailing custom "any colour" chip ([`CustomColorSwatch`]) as the last circle.
/// Emits the chosen `#rrggbb` hex through `on_change`; `value` is the active
/// colour (it highlights a matching swatch). App-agnostic: labels are passed in.
#[component]
pub fn ColorPicker(
    label: String,
    value: String,
    swatches: Vec<String>,
    on_change: EventHandler<String>,
    custom_title: Option<String>,
) -> Element {
    rsx! {
        div { class: "color-picker",
            span { class: "color-picker-label", "{label}" }
            div { class: "color-swatches",
                for swatch in swatches.iter().cloned() {
                    {
                        let is_active = swatch.eq_ignore_ascii_case(&value);
                        let picked = swatch.clone();
                        rsx! {
                            button {
                                key: "{swatch}",
                                r#type: "button",
                                class: if is_active { "color-swatch active" } else { "color-swatch" },
                                style: "background-color: {swatch};",
                                title: "{swatch}",
                                onclick: move |_| on_change.call(picked.clone()),
                                if is_active {
                                    span { class: "material-icons", "check" }
                                }
                            }
                        }
                    }
                }
                // The last circle is the freeform custom picker.
                CustomColorSwatch { value: value.clone(), on_change, title: custom_title.clone() }
            }
        }
    }
}

/// The "any colour" chip: a rainbow swatch wrapping a native OS colour input.
/// Emits the chosen `#rrggbb` through `on_change`. Pairs with [`ColorPicker`].
#[component]
pub fn CustomColorSwatch(
    value: String,
    on_change: EventHandler<String>,
    title: Option<String>,
) -> Element {
    rsx! {
        label {
            class: "color-swatch color-swatch-custom",
            title: title.clone().unwrap_or_default(),
            span { class: "material-icons", "colorize" }
            input {
                r#type: "color",
                class: "color-input-native",
                value: "{value}",
                oninput: move |e| on_change.call(e.value()),
            }
        }
    }
}

/// An image that opens full-screen on click (#114). Click the overlay (or press
/// its close affordance) to dismiss. Keeps its own open/closed state.
#[component]
pub fn ZoomableImage(src: String, alt: String) -> Element {
    let mut zoomed = use_signal(|| false);
    let mut errored = use_signal(|| false);
    rsx! {
        if *errored.read() {
            // Error state: a broken-image placeholder instead of a dead <img>.
            div { class: "image-error", title: "{alt}",
                span { class: "material-icons", "broken_image" }
            }
        } else {
            img {
                src: "{src}",
                alt: "{alt}",
                // `.zoomable` fades the image in on mount (see CSS).
                class: "zoomable",
                onerror: move |_| errored.set(true),
                onclick: move |_| zoomed.set(true),
            }
        }
        if *zoomed.read() {
            div {
                class: "image-lightbox",
                role: "dialog",
                onclick: move |_| zoomed.set(false),
                img { src: "{src}", alt: "{alt}" }
            }
        }
    }
}

/// A horizontal proportion bar (e.g. a poll option's share of the vote).
#[component]
pub fn Bar(fraction: f64) -> Element {
    let pct = (fraction.clamp(0.0, 1.0) * 100.0).round() as i64;
    rsx! {
        div { class: "vote-bar",
            div { class: "vote-bar-fill", style: "width: {pct}%;" }
        }
    }
}
