use dioxus::prelude::*;

use crate::graphql::{self, NodeFields};
use crate::i18n::{t, Lang, LANG};
use crate::route::Route;
use crate::session::{save_session, use_session, SESSION};
use crate::theme::{apply_theme, use_theme, ThemeMode, THEME};

use super::ui::dropdown_menu::{
    DropdownMenu, DropdownMenuContent, DropdownMenuItem, DropdownMenuTrigger,
};

/// Per-navigation chrome state, resolved once in [`Layout`]: the breadcrumb
/// crumbs for the current path and the current context depth (how many leading
/// segments belong to the nearest group/event, per
/// [`graphql::deepest_context_depth`]). The breadcrumbs, drawer and app rail all
/// read these so they agree on the context without each re-querying the path.
static NAV_CRUMBS: GlobalSignal<Vec<graphql::Crumb>> = Signal::global(Vec::new);
static CONTEXT_DEPTH: GlobalSignal<usize> = Signal::global(|| 0);

/// The current context's key-path: the leading `CONTEXT_DEPTH` segments (or the
/// first segment as a fallback until the context resolves).
fn context_path(segments: &[String]) -> Vec<String> {
    let depth = CONTEXT_DEPTH().max(1);
    segments.iter().take(depth).cloned().collect()
}

#[component]
pub fn Layout() -> Element {
    let mut open_drawer = use_signal(|| false);
    let mut search_mode = use_signal(|| false);
    let mut search_input = use_signal(String::new);
    let mut search_results = use_signal(Vec::<NodeFields>::new);
    let menu_open = use_signal(|| false);

    let route = use_route::<Route>();

    // Resolve the path once for the whole chrome. The breadcrumbs, drawer and app
    // rail all key off the current context (the nearest group/event), so
    // resolving it here keeps them consistent and avoids each re-querying.
    {
        let segments = match &route {
            Route::PathPage { segments, .. } => segments.clone(),
            _ => vec![],
        };
        let token = SESSION.read().access_token.clone();
        use_resource(use_reactive!(|(segments, token)| async move {
            let crumbs = graphql::path_crumbs(token.as_deref(), &segments)
                .await
                .unwrap_or_default();
            *CONTEXT_DEPTH.write() = graphql::deepest_context_depth(&crumbs);
            *NAV_CRUMBS.write() = crumbs;
        }));
    }

    // Dioxus renders a lone "?" for the optional `app` query even when it is
    // None (e.g. "/group?"); strip it from the address bar on each navigation so
    // URLs read cleanly. Cosmetic only — the router's own state is untouched.
    {
        let route_dep = route.clone();
        use_effect(use_reactive!(|(route_dep,)| {
            let _ = route_dep;
            if let Some(w) = web_sys::window() {
                let bare = w.location().search().map(|s| s == "?").unwrap_or(false);
                if bare {
                    if let (Ok(path), Ok(history)) = (w.location().pathname(), w.history()) {
                        let _ = history.replace_state_with_url(
                            &wasm_bindgen::JsValue::NULL,
                            "",
                            Some(&path),
                        );
                    }
                }
            }
        }));
    }

    let is_auth_page = matches!(
        route,
        Route::Login {}
            | Route::Register {}
            | Route::ResetPassword {}
            | Route::SetPassword {}
            | Route::Unverified {}
    );

    if is_auth_page {
        return rsx! {
            Outlet::<Route> {}
        };
    }

    rsx! {
        div { class: "app-shell",
            // Main content area
            div { class: "main-content",
                Outlet::<Route> {}
            }

            // Desktop drawer (sidebar)
            div { class: "drawer",
                div { class: "drawer-inner",
                    DrawerContent {}
                }
            }

            // App rail (desktop only — right of drawer)
            div { class: "app-rail",
                AppRail {}
            }

            // Bottom/top bar
            div { class: "bottom-bar",
                div { class: "bar",
                    // Menu button (mobile)
                    button {
                        class: "btn-icon mobile-only",
                        onclick: move |_| {
                            open_drawer.set(true);
                        },
                        span { class: "material-icons", "menu" }
                    }

                    // Search or breadcrumbs
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
                        Breadcrumbs {}
                        button {
                            class: "btn-icon",
                            onclick: move |_| search_mode.set(true),
                            span { class: "material-icons", "search" }
                        }
                    }

                    // User menu
                    UserMenu { menu_open }
                }
            }

            // Spacer for bottom bar
            div { class: "bar-spacer" }
        }

        // Mobile drawer overlay
        div {
            class: if *open_drawer.read() { "mobile-drawer" } else { "mobile-drawer hidden" },
            div { style: "padding: 8px;",
                div { class: "bar",
                    div { class: "breadcrumbs", "{t(\"common.home\")}" }
                    button {
                        class: "btn-icon",
                        onclick: move |_| {
                            open_drawer.set(false);
                        },
                        span { class: "material-icons", "close" }
                    }
                }
            }
            DrawerContent {}
        }
    }
}

/// Search bar with live GraphQL results
#[component]
fn SearchBar(
    input: Signal<String>,
    results: Signal<Vec<NodeFields>>,
    on_close: EventHandler,
) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let mut input = input;
    let mut results = results;
    // Monotonic request id so out-of-order responses don't clobber newer ones
    // (typing fires a query per keystroke; the last issued must win, not the
    // last to return).
    let mut seq = use_signal(|| 0u32);

    rsx! {
        div { style: "flex: 1; position: relative;",
            input {
                class: "breadcrumbs",
                style: "background: transparent; border: none; color: white; outline: none; font-size: 14px; width: 100%;",
                placeholder: "{t(\"common.search\")}",
                value: "{input}",
                oninput: move |evt| {
                    let value = evt.value();
                    input.set(value.clone());
                    let my = *seq.read() + 1;
                    seq.set(my);
                    if value.trim().is_empty() {
                        results.set(vec![]);
                        return;
                    }
                    let token = session.read().access_token.clone();
                    spawn(async move {
                        let nodes = graphql::search_nodes(token.as_deref(), &value)
                            .await
                            .unwrap_or_default();
                        // Only apply if this is still the latest query.
                        if *seq.read() == my {
                            results.set(nodes);
                        }
                    });
                },
                onkeydown: move |evt| {
                    if evt.key() == Key::Escape {
                        on_close.call(());
                    }
                },
            }
            // Search results dropdown
            if !results.read().is_empty() {
                div { class: "search-results",
                    for node in results.read().iter() {
                        div {
                            class: "list-item",
                            key: "{node.id.0}",
                            onclick: {
                                // A search hit can live anywhere in the tree, so
                                // resolve its full ancestor path (root excluded)
                                // rather than treating the key as a top-level
                                // segment. Fall back to the bare key if the walk
                                // yields nothing.
                                let node_id = node.id.0.clone();
                                let key = node.key.clone();
                                let on_close = on_close;
                                move |_| {
                                    let node_id = node_id.clone();
                                    let key = key.clone();
                                    let token = session.read().access_token.clone();
                                    // Resolve first, THEN navigate and close: closing
                                    // unmounts the SearchBar, which would cancel this
                                    // task before the async path lookup finished.
                                    spawn(async move {
                                        let mut segments = graphql::path_from_id(
                                                token.as_deref(),
                                                &node_id,
                                            )
                                            .await
                                            .unwrap_or_default();
                                        if segments.is_empty() {
                                            segments = vec![key];
                                        }
                                        nav.push(Route::PathPage { segments, app: None });
                                        on_close.call(());
                                    });
                                }
                            },
                            div { class: "avatar small",
                                {super::loader::icon_el(node.mime_id.as_deref().unwrap_or(""))}
                            }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{node.name}" }
                            }
                        }
                    }
                }
            }
        }
        button {
            class: "btn-icon",
            onclick: move |_| on_close.call(()),
            span { class: "material-icons", "close" }
        }
    }
}

/// Breadcrumb navigation based on the current route. Mirrors the old wiki: a row
/// of mime avatars (each path node); only the current node's name is shown, and
/// hovering a crumb reveals its name (the whole bar resets on mouse-leave). The
/// trail STARTS at the current context (the nearest group/event) rather than the
/// root, so it begins with the selected event/group. The open app is shown as a
/// badge on the current node's avatar.
#[component]
fn Breadcrumbs() -> Element {
    let route = use_route::<Route>();
    let (segments, app) = match &route {
        Route::PathPage { segments, app } => (segments.clone(), app.clone()),
        _ => (vec![], None),
    };

    // Resolved once by `Layout`; read reactively so crumbs update on navigation.
    let crumbs = NAV_CRUMBS();
    let depth = CONTEXT_DEPTH();
    let total = segments.len();

    let mut hovered = use_signal(|| None::<usize>);

    // Begin at the context (deepest group/event). With no context in the path
    // (e.g. the home route) fall back to showing Home plus the full path.
    let (show_home, start) = if depth >= 1 {
        (false, depth - 1)
    } else {
        (true, 0)
    };

    // The default (unhovered) open crumb is the deepest one.
    let last_id = if total > 0 { total } else { 0 };
    let hov = *hovered.read();
    let is_open = move |c: usize| match hov {
        Some(h) => h == c,
        None => c == last_id,
    };

    // The open app, badged onto the current (last) crumb's avatar.
    let app_badge = app.map(|a| format!("app/{a}"));

    rsx! {
        div {
            class: "breadcrumbs",
            onmouseleave: move |_| hovered.set(None),
            if show_home {
                BreadcrumbCrumb {
                    to: Route::HomeApp {},
                    mime: "app/home".to_string(),
                    name: t("common.home"),
                    ordinal: None,
                    open: is_open(0),
                    crumb_id: 0,
                    hovered,
                    app_badge: None,
                }
            }
            for i in start..total {
                {
                    let info = crumbs.get(i);
                    let name = info
                        .map(|c| c.name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| segments[i].clone());
                    let mime = info.and_then(|c| c.mime_id.clone()).unwrap_or_default();
                    let ordinal = info.and_then(|c| c.ordinal);
                    let badge = if i + 1 == total { app_badge.clone() } else { None };
                    rsx! {
                        BreadcrumbCrumb {
                            key: "{i}",
                            to: Route::PathPage { segments: segments[..=i].to_vec(), app: None },
                            mime,
                            name,
                            ordinal,
                            open: is_open(i + 1),
                            crumb_id: i + 1,
                            hovered,
                            app_badge: badge,
                        }
                    }
                }
            }
        }
    }
}

/// A single breadcrumb: an always-visible mime avatar and a name that expands on
/// hover (horizontal collapse), matching the old wiki's `BreadcrumbsLink`.
#[component]
fn BreadcrumbCrumb(
    to: Route,
    mime: String,
    name: String,
    ordinal: Option<usize>,
    open: bool,
    crumb_id: usize,
    hovered: Signal<Option<usize>>,
    app_badge: Option<String>,
) -> Element {
    let mut hovered = hovered;
    rsx! {
        div {
            class: "crumb",
            onmouseenter: move |_| hovered.set(Some(crumb_id)),
            Link { to, class: "crumb-link",
                div { class: "avatar small crumb-avatar",
                    {super::loader::node_avatar(&mime, &name, ordinal)}
                    // The open app (e.g. vote, editor) badged onto the avatar.
                    if let Some(badge) = app_badge {
                        span { class: "crumb-app-badge", {super::loader::icon_el(&badge)} }
                    }
                }
                span {
                    class: if open { "crumb-name open" } else { "crumb-name" },
                    "{name}"
                }
            }
        }
    }
}

/// App rail — vertical icon navigation for large screens
#[component]
fn AppRail() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };

    let current_app = match &route {
        Route::PathPage { app, .. } => app.clone(),
        _ => None,
    };

    if segments.is_empty() {
        return rsx! {};
    }

    // The apps operate on the current context (the nearest group/event), mirroring
    // the React `useApps`. The app is part of the route's query, so these navigate
    // client-side and the resolver swaps the view without a reload.
    let ctx_path = context_path(&segments);

    let mut apps: Vec<(&str, String, Route, bool)> = vec![
        ("app/home", t("common.home"), Route::HomeApp {}, false),
        (
            "app/folder",
            t("mime.folder"),
            Route::PathPage {
                segments: ctx_path.clone(),
                app: None,
            },
            current_app.is_none(),
        ),
    ];
    if is_auth {
        apps.push((
            "app/speak",
            t("mime.speak"),
            Route::PathPage {
                segments: ctx_path.clone(),
                app: Some("speak".to_string()),
            },
            current_app.as_deref() == Some("speak"),
        ));
        apps.push((
            "app/vote",
            t("mime.vote"),
            Route::PathPage {
                segments: ctx_path.clone(),
                app: Some("vote".to_string()),
            },
            current_app.as_deref() == Some("vote"),
        ));
        // The other apps (screen, admin, program, graph, social, map, profile,
        // perm, parent) are still reachable via their `?app=` URL but hidden
        // from the rail until they are ready to show.
    }

    rsx! {
        for (mime_id , label , to , active) in apps.into_iter() {
            Link {
                to,
                class: if active { "btn-icon active" } else { "btn-icon" },
                style: "flex-direction: column; gap: 2px; width: 56px; height: 56px;",
                title: "{label}",
                span { style: "font-size: 20px;", {super::loader::icon_el(mime_id)} }
                span { style: "font-size: 10px; color: var(--md-on-surface-variant);", "{label}" }
            }
        }
    }
}

/// User menu with popover
#[component]
fn UserMenu(menu_open: Signal<bool>) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let is_auth = session.read().is_authenticated();
    let theme = use_theme();
    let mut menu_open = menu_open;

    let initial = session
        .read()
        .user
        .as_ref()
        .map(|u| u.display_name.chars().next().unwrap_or('?').to_string())
        .unwrap_or_else(|| "?".to_string());

    let dark = *theme.read() == ThemeMode::Dark;

    rsx! {
        DropdownMenu {
            open: Some(menu_open()),
            on_open_change: move |v| menu_open.set(v),
            DropdownMenuTrigger {
                class: "btn-icon",
                if is_auth {
                    span { class: "avatar small secondary", "{initial}" }
                } else {
                    span { class: "avatar small", span { class: "material-icons", "person" } }
                }
            }
            DropdownMenuContent {
                // Theme toggle
                DropdownMenuItem::<String> {
                    value: "theme".to_string(),
                    index: 0usize,
                    on_select: move |_| {
                        let new_theme = theme.read().toggle();
                        apply_theme(&new_theme);
                        crate::theme::save_theme(&new_theme);
                        *THEME.write() = new_theme;
                    },
                    if dark {
                        span { class: "material-icons", "light_mode" }
                        " {t(\"layout.light\")}"
                    } else {
                        span { class: "material-icons", "dark_mode" }
                        " {t(\"layout.dark\")}"
                    }
                }
                // Language toggle
                DropdownMenuItem::<String> {
                    value: "lang".to_string(),
                    index: 1usize,
                    on_select: move |_| {
                        let new_lang = match *LANG.read() {
                            Lang::En => Lang::Da,
                            Lang::Da => Lang::En,
                        };
                        *LANG.write() = new_lang;
                    },
                    span { class: "material-icons", "language" }
                    {match *LANG.read() { Lang::En => " Dansk", Lang::Da => " English" }}
                }
                if is_auth {
                    DropdownMenuItem::<String> {
                        value: "setpw".to_string(),
                        index: 2usize,
                        on_select: move |_| { nav.push(Route::SetPassword {}); },
                        span { class: "material-icons", "lock" }
                        " {t(\"auth.setPassword\")}"
                    }
                    DropdownMenuItem::<String> {
                        value: "logout".to_string(),
                        index: 3usize,
                        on_select: move |_| {
                            crate::nhost::sign_out();
                            *SESSION.write() = Default::default();
                            save_session(&Default::default());
                            nav.push(Route::HomeApp {});
                        },
                        span { class: "material-icons", "logout" }
                        " {t(\"auth.logout\")}"
                    }
                } else {
                    DropdownMenuItem::<String> {
                        value: "login".to_string(),
                        index: 2usize,
                        on_select: move |_| { nav.push(Route::Login {}); },
                        span { class: "material-icons", "login" }
                        " {t(\"common.logIn\")}"
                    }
                    DropdownMenuItem::<String> {
                        value: "register".to_string(),
                        index: 3usize,
                        on_select: move |_| { nav.push(Route::Register {}); },
                        span { class: "material-icons", "person_add" }
                        " {t(\"auth.register\")}"
                    }
                }
            }
        }
    }
}

/// Drawer content — shows navigation tree
#[component]
fn DrawerContent() -> Element {
    let session = use_session();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let is_auth = session.read().is_authenticated();
    let display_name = session
        .read()
        .user
        .as_ref()
        .map(|u| u.display_name.clone())
        .unwrap_or_default();
    let email = session
        .read()
        .user
        .as_ref()
        .map(|u| u.email.clone())
        .unwrap_or_default();

    rsx! {
        div { style: "padding: 16px;",
            if is_auth {
                div { class: "card",
                    div { class: "card-header",
                        div { class: "avatar", span { class: "material-icons", "person" } }
                        div {
                            h3 { class: "title-medium", "{display_name}" }
                            p { class: "body-medium",
                                style: "color: var(--md-on-surface-variant);",
                                "{email}"
                            }
                        }
                    }
                }
            }

            div { class: "list", style: "margin-top: 8px;",
                Link {
                    to: Route::HomeApp {},
                    class: "list-item",
                    div { class: "avatar small", span { class: "material-icons", "home" } }
                    div { class: "list-item-text",
                        div { class: "list-item-primary", "{t(\"common.home\")}" }
                    }
                }
            }

            // In-context navigation: at the home route show the groups/events
            // home list; once inside a context show its lazy child tree (the
            // React app's MenuList).
            if is_auth {
                if segments.is_empty() {
                    HomeList {}
                } else {
                    // Key on the context so switching contexts remounts and
                    // re-resolves (use_resource does not re-run for prop changes).
                    MenuList {
                        key: "{segments.first().cloned().unwrap_or_default()}",
                        segments: segments.clone(),
                    }
                }
            }
        }
    }
}

/// MenuList: the in-context drawer tree. Resolves the current context (the
/// nearest group/event, which may be nested below the top path segment), then
/// renders its children lazily and expandably, mirroring the React
/// `MenuList`/`DrawerList`/`DrawerElement` trio.
#[component]
fn MenuList(segments: Vec<String>) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    // Root the tree at the context (deepest group/event) rather than the first
    // segment, so a nested event shows its own contents.
    let ctx_path = context_path(&segments);

    // Re-resolve reactively when the context path changes.
    let cpath = ctx_path.clone();
    let context = use_resource(use_reactive!(|(cpath, access_token)| async move {
        graphql::resolve_path(access_token.as_deref(), &cpath)
            .await
            .ok()
            .flatten()
            .map(|n| n.id.0)
    }));

    let hint_style = "padding: 4px 16px; color: var(--md-on-surface-variant);";
    let ctx = context.read().clone();
    match ctx {
        Some(Some(context_id)) => rsx! {
            div { class: "list", style: "margin-top: 8px;",
                DrawerLevel {
                    parent_id: context_id,
                    path_prefix: ctx_path.clone(),
                    current_path: segments.clone(),
                    depth: 0,
                }
            }
        },
        Some(None) => rsx! {},
        None => rsx! {
            p { class: "body-medium", style: "{hint_style}", "…" }
        },
    }
}

/// Whether a mime type can hold children (so its drawer row gets an expander).
/// Leaves (documents, files, maps) never do.
fn mime_has_children(mime_id: &str) -> bool {
    !matches!(mime_id, "wiki/document" | "wiki/file" | "map/map")
}

/// One lazily-loaded level of the drawer tree: the visible children of
/// `parent_id`, ordered like the folder view.
#[component]
fn DrawerLevel(
    parent_id: String,
    path_prefix: Vec<String>,
    current_path: Vec<String>,
    depth: usize,
) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let parent = parent_id.clone();

    let children = use_resource(use_reactive!(
        |(parent, access_token, user_id)| async move {
            let Some(user_id) = user_id else {
                return Vec::new();
            };
            graphql::query_children(access_token.as_deref(), &parent, &user_id)
                .await
                .unwrap_or_default()
        }
    ));

    let items = children.read().clone();
    match items {
        Some(items) => {
            let ordinals = super::loader::sibling_ordinals(&items);
            rsx! {
                for (child , ordinal) in items.iter().zip(ordinals) {
                    DrawerNodeItem {
                        key: "{child.id.0}",
                        node: child.clone(),
                        path_prefix: path_prefix.clone(),
                        current_path: current_path.clone(),
                        depth,
                        ordinal,
                    }
                }
            }
        }
        None => rsx! {},
    }
}

/// A single row in the drawer tree: icon, name, "not submitted" badge, and an
/// expander that lazily reveals the node's own children. Rows on the current
/// path start expanded and the active node is highlighted.
#[component]
fn DrawerNodeItem(
    node: graphql::ChildNodeFields,
    path_prefix: Vec<String>,
    current_path: Vec<String>,
    depth: usize,
    ordinal: Option<usize>,
) -> Element {
    let nav = use_navigator();

    let mut full_path = path_prefix.clone();
    full_path.push(node.key.clone());

    // This row is on the active path if its full path is a prefix of the URL.
    let on_path = full_path.len() <= current_path.len()
        && full_path
            .iter()
            .zip(current_path.iter())
            .all(|(a, b)| a == b);
    let selected = full_path == current_path;

    let mime_id = node.mime_id.clone().unwrap_or_default();
    let expandable = mime_has_children(&mime_id);
    let node_name = node.name.clone();

    // Auto-expand ancestors of the current node; let the user toggle the rest.
    let mut expanded = use_signal(|| on_path && !selected);
    // Re-expand when a later navigation brings this node onto the active path
    // (the initial value above only applies on first mount).
    use_effect(use_reactive!(|(on_path, selected)| {
        if on_path && !selected {
            expanded.set(true);
        }
    }));

    let node_id = node.id.0.clone();
    let indent = format!("padding-left: {}px;", 12 + depth * 14);
    let nav_path = full_path.clone();

    rsx! {
        div {
            class: if selected { "list-item selected" } else { "list-item" },
            style: "cursor: pointer; {indent}",
            onclick: move |_| {
                nav.push(Route::PathPage {
                    segments: nav_path.clone(),
                    app: None,
                });
            },
            super::loader::NodeAvatar {
                mime: mime_id.clone(),
                name: node_name.clone(),
                ordinal,
                mutable: node.mutable,
                small: true,
            }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{node.name}" }
            }
            if expandable {
                button {
                    class: "btn-icon",
                    style: "margin-left: auto;",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        let now = *expanded.read();
                        expanded.set(!now);
                    },
                    if *expanded.read() {
                        span { class: "material-icons", "expand_more" }
                    } else {
                        span { class: "material-icons", "chevron_right" }
                    }
                }
            }
        }
        if expandable && *expanded.read() {
            DrawerLevel {
                parent_id: node_id,
                path_prefix: full_path.clone(),
                current_path: current_path.clone(),
                depth: depth + 1,
            }
        }
    }
}

/// The result of loading the user's contexts: (groups, events).
type ContextLists = (
    Vec<graphql::ContextNodeFields>,
    Vec<graphql::ContextNodeFields>,
);

/// HomeList — shows the user's groups and events, loaded from GraphQL.
#[component]
fn HomeList() -> Element {
    let session = use_session();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let access_token = session.read().access_token.clone();

    // Live home list: accepting an invitation or a membership change re-fetches
    // the user's groups and events.
    let refresh = use_signal(|| 0u32);
    let sub_uid = user_id
        .clone()
        .unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".to_string());
    crate::subscription::use_live(
        format!(
            "subscription {{ members(where: {{ nodeId: {{ _eq: \"{sub_uid}\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    let contexts = use_resource(move || {
        let token = access_token.clone();
        let user_id = user_id.clone();
        let _ = refresh.read();
        async move {
            let Some(user_id) = user_id else {
                return Ok::<ContextLists, String>((Vec::new(), Vec::new()));
            };
            let groups = graphql::query_contexts(token.as_deref(), &user_id, "wiki/group").await?;
            let events = graphql::query_contexts(token.as_deref(), &user_id, "wiki/event").await?;
            Ok((groups, events))
        }
    });

    let state = contexts.read().clone();
    let hint_style = "padding: 4px 16px; color: var(--md-on-surface-variant);";

    rsx! {
        div { style: "margin-top: 16px;",
            // Groups
            h4 { class: "title-small", style: "padding: 8px 16px; color: var(--md-on-surface-variant);",
                "{t(\"layout.groups\")}"
            }
            {match &state {
                None => rsx! {
                    p { class: "body-medium", style: "{hint_style}", "…" }
                },
                Some(Err(e)) => rsx! {
                    p { class: "body-medium", style: "{hint_style}", "{e}" }
                },
                Some(Ok((groups, _))) if groups.is_empty() => rsx! {
                    p { class: "body-medium", style: "{hint_style}", "{t(\"layout.noGroups\")}" }
                },
                Some(Ok((groups, _))) => rsx! {
                    div { class: "list",
                        for node in groups.iter() {
                            ContextItem { key: "{node.id.0}", node: node.clone() }
                        }
                    }
                },
            }}

            // Events, grouped by year (newest first)
            h4 { class: "title-small", style: "padding: 8px 16px; margin-top: 8px; color: var(--md-on-surface-variant);",
                "{t(\"layout.events\")}"
            }
            {match &state {
                None => rsx! {
                    p { class: "body-medium", style: "{hint_style}", "…" }
                },
                Some(Err(e)) => rsx! {
                    p { class: "body-medium", style: "{hint_style}", "{e}" }
                },
                Some(Ok((_, events))) if events.is_empty() => rsx! {
                    p { class: "body-medium", style: "{hint_style}", "{t(\"layout.noEvents\")}" }
                },
                Some(Ok((_, events))) => rsx! {
                    for (year , items) in group_by_year(events) {
                        div { key: "{year}",
                            p { class: "label-medium",
                                style: "padding: 4px 16px; font-weight: 600; color: var(--md-on-surface-variant);",
                                "{year}"
                            }
                            div { class: "list",
                                for node in items.iter() {
                                    ContextItem { key: "{node.id.0}", node: node.clone() }
                                }
                            }
                        }
                    }
                },
            }}
        }
    }
}

/// A single group/event entry. Clicking resolves the node's path and navigates.
#[component]
fn ContextItem(node: graphql::ContextNodeFields) -> Element {
    let session = use_session();
    let nav = use_navigator();
    let name = node.name.clone();
    let node_id = node.id.0.clone();
    let abbr = abbrev_context_name(&name);

    rsx! {
        div {
            class: "list-item",
            style: "cursor: pointer;",
            onclick: move |_| {
                let node_id = node_id.clone();
                let token = session.read().access_token.clone();
                spawn(async move {
                    if let Ok(segments) = graphql::path_from_id(token.as_deref(), &node_id).await {
                        if !segments.is_empty() {
                            nav.push(Route::PathPage { segments, app: None });
                        }
                    }
                });
            },
            div { class: "avatar small secondary", "{abbr}" }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{name}" }
            }
        }
    }
}

/// Group events into (year, events) buckets, preserving the input order. Since
/// events arrive newest-first, buckets come out in descending-year order.
fn group_by_year(
    events: &[graphql::ContextNodeFields],
) -> Vec<(String, Vec<graphql::ContextNodeFields>)> {
    let mut out: Vec<(String, Vec<graphql::ContextNodeFields>)> = Vec::new();
    for event in events {
        let year = event
            .created_at
            .as_ref()
            .and_then(|t| t.0.get(0..4))
            .unwrap_or("")
            .to_string();
        match out.last_mut() {
            Some((last_year, items)) if *last_year == year => items.push(event.clone()),
            _ => out.push((year, vec![event.clone()])),
        }
    }
    out
}

/// Abbreviate a context name into a short avatar badge (ported from the React
/// `abrivContextName`): keep capitalised words, collapse each to its acronym or
/// initial, and join at most three of them.
fn abbrev_context_name(name: &str) -> String {
    fn upper_count(word: &str) -> usize {
        word.chars().filter(|c| c.is_uppercase()).count()
    }

    let words: Vec<String> = name
        .trim()
        .split(' ')
        .filter(|w| !w.is_empty())
        .filter(|w| {
            let first = w.chars().next().unwrap();
            let has_digit = w.chars().any(|c| c.is_ascii_digit());
            (first.is_uppercase() && !(has_digit && w.chars().count() > 1)) || upper_count(w) > 1
        })
        .map(|w| match w {
            "Hovedbestyrelsesmøde" => "HB".to_string(),
            "Landsmøde" => "LM".to_string(),
            _ if upper_count(w) > 1 => w.to_string(),
            _ => w.chars().next().unwrap().to_string(),
        })
        .collect();

    match words.len() {
        1..=3 => words.concat(),
        _ => String::new(),
    }
}
