use dioxus::prelude::*;

use super::*;
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

/// App-axis navigation rail: a fixed icon + label strip (medium+). Its menu button
/// opens/closes the groups/events tree pane to the rail's right — the app icons
/// never move. One primary-container pill tracks the active `?app=` destination.
#[component]
pub(super) fn NavigationRail(tree_open: bool, on_toggle: EventHandler<()>) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let route = use_route::<Route>();
    let apps = context_apps(&route, is_auth);

    rsx! {
        nav { class: "nav-rail",
            // Header: the menu button opens / closes the context tree pane.
            div { class: "nav-rail-header",
                button {
                    class: "btn-icon menu-trigger state-layer",
                    aria_label: t("common.menu"),
                    "aria-expanded": if tree_open { "true" } else { "false" },
                    onclick: move |_| on_toggle.call(()),
                    // Badge only while the pane is shut. Open, the activity row it
                    // stands for is on screen carrying its own count, and two of
                    // them would just be two.
                    if tree_open {
                        span { class: "material-icons", "menu_open" }
                    } else {
                        crate::components::activity::NavBadge {
                            span { class: "material-icons", "menu" }
                        }
                    }
                }
            }
            // App destinations (icon over label).
            div { class: "nav-rail-destinations",
                for (mime_id , label , to , active) in apps.into_iter() {
                    Link {
                        key: "{mime_id}",
                        to,
                        class: if active { "nav-rail-item active state-layer" } else { "nav-rail-item state-layer" },
                        "aria-current": if active { "page" } else { "false" },
                        title: "{label}",
                        span { class: "nav-rail-indicator",
                            span { {crate::components::loader::icon_el(mime_id)} }
                        }
                        span {
                            class: if active { "nav-rail-label md-label-medium-emphasized" } else { "nav-rail-label md-label-medium" },
                            "{label}"
                        }
                    }
                }
            }
        }
    }
}

/// Bottom navigation bar (APP axis) on compact — the same context destinations as
/// the rail, with the secondary-container pill indicator.
#[component]
pub(super) fn NavigationBar() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let route = use_route::<Route>();
    let apps = context_apps(&route, is_auth);
    if apps.is_empty() {
        return rsx! {};
    }

    // M3 bottom navigation shows up to 5 destinations. Beyond that the bar does
    // not scale, so the surplus moves into an "apps" switcher sheet: the bar keeps
    // the first four destinations plus a trailing More button. With <=5 apps every
    // destination stays inline (the current case), so this only kicks in as more
    // apps are surfaced.
    const MAX_INLINE: usize = 5;
    let overflow = apps.len() > MAX_INLINE;
    let inline = if overflow { MAX_INLINE - 1 } else { apps.len() };

    rsx! {
        nav { class: "nav-bar",
            for (mime_id , label , to , active) in apps.iter().take(inline).cloned() {
                Link {
                    key: "{mime_id}",
                    to,
                    class: if active { "nav-bar-item active state-layer" } else { "nav-bar-item state-layer" },
                    "aria-current": if active { "page" } else { "false" },
                    title: "{label}",
                    aria_label: "{label}",
                    span { class: "nav-bar-indicator",
                        span { {crate::components::loader::icon_el(mime_id)} }
                    }
                    span { class: "nav-bar-label md-label-medium", "{label}" }
                }
            }
            if overflow {
                AppSwitcher { apps: apps.clone() }
            }
        }
    }
}

/// The scalable "apps" overflow: a bottom-nav destination that opens a sheet
/// listing every context app (bottom sheet on compact). Keeps the bottom bar
/// within the M3 five-destination limit however many apps a context surfaces.
#[component]
pub(super) fn AppSwitcher(apps: Vec<(&'static str, String, Route, bool)>) -> Element {
    let mut open = use_signal(|| false);
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);
    rsx! {
        button {
            class: "nav-bar-item state-layer",
            r#type: "button",
            aria_label: t("common.apps"),
            onclick: move |_| {
                return_focus.set(crate::components::widgets::active_html_element());
                open.set(true);
            },
            span { class: "nav-bar-indicator",
                span { class: "material-icons", "apps" }
            }
            span { class: "nav-bar-label md-label-medium", "{t(\"common.apps\")}" }
        }
        div {
            class: if open() { "sheet-scrim open" } else { "sheet-scrim" },
            role: "presentation",
            onclick: move |_| crate::components::widgets::close_modal(open, return_focus),
        }
        aside {
            class: if open() { "tool-sheet open" } else { "tool-sheet" },
            role: "dialog",
            "aria-modal": "true",
            "aria-label": t("common.apps"),
            tabindex: "-1",
            onkeydown: move |e| {
                match e.key() {
                    Key::Escape => crate::components::widgets::close_modal(open, return_focus),
                    Key::Tab if crate::components::widgets::trap_tab_focus(".tool-sheet.open", e.modifiers().shift()) => {
                        e.prevent_default();
                    }
                    _ => {}
                }
            },
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
                h3 { class: "title-medium", "{t(\"common.apps\")}" }
                button {
                    class: "btn-icon state-layer",
                    aria_label: t("common.close"),
                    onclick: move |_| crate::components::widgets::close_modal(open, return_focus),
                    span { class: "material-icons", "close" }
                }
            }
            div {
                class: "tool-sheet-body",
                onclick: move |_| crate::components::widgets::close_modal(open, return_focus),
                for (mime_id , label , to , active) in apps.into_iter() {
                    Link {
                        key: "{mime_id}",
                        to,
                        class: if active { "sheet-action selected" } else { "sheet-action" },
                        {crate::components::loader::icon_el(mime_id)}
                        "{label}"
                    }
                }
            }
        }
    }
}

/// Modal navigation drawer hosting the groups/events tree (PLACE axis) on
/// compact/medium/expanded. Scrim + spring slide; auto-closes on navigation
/// (any click inside the tree bubbles to the wrapper), fixing the old
/// manual-close bug.
#[component]
pub(super) fn NavigationDrawer(open: Signal<bool>) -> Element {
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    // The current context (nearest group/event) heads the drawer next to the close
    // icon — the same bar the tree pane shows, so it is hidden inside the drawer's
    // body to avoid repeating it.
    let ctx_path = context_path(&segments);
    let crumbs = NAV_CRUMBS();
    let ctx_crumb = crumbs.get(ctx_path.len().saturating_sub(1));
    let ctx_name = ctx_crumb
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| ctx_path.last().cloned().unwrap_or_default());
    let ctx_mime = ctx_crumb
        .and_then(|c| c.mime_id.clone())
        .unwrap_or_default();

    // Return focus to the menu trigger when the drawer closes (a11y), captured
    // when the focus sentinel mounts on open.
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);

    rsx! {
        div {
            class: if open() { "nav-drawer-scrim open" } else { "nav-drawer-scrim" },
            role: "presentation",
            onclick: move |_| crate::components::widgets::close_modal(open, return_focus),
        }
        aside {
            class: if open() { "nav-drawer open" } else { "nav-drawer" },
            // The compact drawer is the primary phone navigation: give it the same
            // modal a11y (name, focus trap, Escape, return-focus) as the app's other
            // overlays, which it previously lacked.
            role: "dialog",
            "aria-modal": "true",
            "aria-label": t("common.menu"),
            tabindex: "-1",
            onkeydown: move |e| {
                match e.key() {
                    Key::Escape => crate::components::widgets::close_modal(open, return_focus),
                    Key::Tab if crate::components::widgets::trap_tab_focus(".nav-drawer.open", e.modifiers().shift()) => {
                        e.prevent_default();
                    }
                    _ => {}
                }
            },
            // Focus sentinel: capture the trigger + pull focus into the drawer on open.
            if open() {
                div {
                    class: "sheet-focus-sentinel",
                    tabindex: "-1",
                    onmounted: move |e| {
                        return_focus.set(crate::components::widgets::active_html_element());
                        spawn(async move {
                            let _ = e.set_focus(true).await;
                        });
                    },
                }
            }
            div { class: "nav-drawer-header",
                // The same place bar the tree pane shows: where you are, and the
                // way to somewhere else. It opens the picker inside the drawer
                // rather than navigating, so the drawer stays open while you look.
                ContextSwitchBar {
                    name: if segments.is_empty() { t("common.home") } else { ctx_name.clone() },
                    mime: ctx_mime.clone(),
                    at_home: segments.is_empty(),
                }
                button {
                    class: "btn-icon state-layer",
                    aria_label: t("common.close"),
                    onclick: move |_| crate::components::widgets::close_modal(open, return_focus),
                    span { class: "material-icons", "close" }
                }
            }
            div { class: "nav-drawer-body", onclick: move |_| open.set(false),
                DrawerContent {}
            }
        }
    }
}
