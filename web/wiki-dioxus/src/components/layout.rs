use dioxus::prelude::*;

use crate::graphql::{self, NodeFields};
use crate::i18n::{t, Lang, LANG};
use crate::route::Route;
use crate::session::{save_session, use_session, SESSION};
use crate::theme::{apply_theme, use_theme, ThemeMode, THEME};

#[component]
pub fn Layout() -> Element {
    let mut open_drawer = use_signal(|| false);
    let mut search_mode = use_signal(|| false);
    let mut search_input = use_signal(String::new);
    let mut search_results = use_signal(Vec::<NodeFields>::new);
    let menu_open = use_signal(|| false);

    let route = use_route::<Route>();
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
                        span { class: "avatar small", "\u{2630}" }
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
                            span { class: "avatar small", "\u{1F50D}" }
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
                        span { class: "avatar small", "\u{2715}" }
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
                    let token = session.read().access_token.clone();
                    spawn(async move {
                        match graphql::search_nodes(token.as_deref(), &value).await {
                            Ok(nodes) => results.set(nodes),
                            Err(_) => results.set(vec![]),
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
                                let key = node.key.clone();
                                let on_close = on_close;
                                move |_| {
                                    nav.push(Route::PathPage {
                                        segments: vec![key.clone()],
                                        app: None,
                                    });
                                    on_close.call(());
                                }
                            },
                            div { class: "avatar small",
                                "{super::loader::mime_icon(node.mime_id.as_deref().unwrap_or(\"\"))}"
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
            span { class: "avatar small", "\u{2715}" }
        }
    }
}

/// Breadcrumb navigation based on current route
#[component]
fn Breadcrumbs() -> Element {
    let route = use_route::<Route>();

    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };

    let key = segments.join("\u{1f}");
    rsx! {
        div { class: "breadcrumbs",
            Link { to: Route::HomeApp {}, "\u{1F3E0}" }
            if !segments.is_empty() {
                BreadcrumbTrail { key: "{key}", segments: segments.clone() }
            }
        }
    }
}

/// The breadcrumb segments after home, showing resolved node names (not URL
/// slugs). Keyed on the path so it remounts and re-resolves on navigation.
#[component]
fn BreadcrumbTrail(segments: Vec<String>) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let segs = segments.clone();
    let names = use_resource(move || {
        let token = token.clone();
        let segs = segs.clone();
        async move {
            graphql::path_names(token.as_deref(), &segs)
                .await
                .unwrap_or_default()
        }
    });
    let names = names.read().clone().unwrap_or_default();

    rsx! {
        for (i , segment) in segments.iter().enumerate() {
            span { class: "separator", " / " }
            {
                let label = names
                    .get(i)
                    .filter(|s| !s.is_empty())
                    .cloned()
                    .unwrap_or_else(|| segment.clone());
                if i == segments.len() - 1 {
                    rsx! {
                        span { "{label}" }
                    }
                } else {
                    rsx! {
                        Link {
                            to: Route::PathPage {
                                segments: segments[..=i].to_vec(),
                                app: None,
                            },
                            "{label}"
                        }
                    }
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

    // The apps operate on the context (the first path segment), mirroring the
    // React `useApps`: Home, Folder, and — when signed in — Speak and Vote. The
    // app is part of the route's query, so these navigate client-side and the
    // resolver swaps the view without a reload.
    let context = segments.first().cloned().unwrap_or_default();

    let mut apps: Vec<(&str, String, Route, bool)> = vec![
        ("app/home", t("common.home"), Route::HomeApp {}, false),
        (
            "app/folder",
            t("mime.folder"),
            Route::PathPage {
                segments: vec![context.clone()],
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
                segments: vec![context.clone()],
                app: Some("speak".to_string()),
            },
            current_app.as_deref() == Some("speak"),
        ));
        apps.push((
            "app/vote",
            t("mime.vote"),
            Route::PathPage {
                segments: vec![context.clone()],
                app: Some("vote".to_string()),
            },
            current_app.as_deref() == Some("vote"),
        ));
    }

    rsx! {
        for (mime_id , label , to , active) in apps.into_iter() {
            Link {
                to,
                class: if active { "btn-icon active" } else { "btn-icon" },
                style: "flex-direction: column; gap: 2px; width: 56px; height: 56px;",
                title: "{label}",
                span { style: "font-size: 20px;", "{super::loader::mime_icon(mime_id)}" }
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

    rsx! {
        div { style: "position: relative;",
            button {
                class: "btn-icon",
                onclick: move |_| {
                    let current = *menu_open.read();
                    menu_open.set(!current);
                },
                if is_auth {
                    span { class: "avatar small secondary", "{initial}" }
                } else {
                    span { class: "avatar small", "\u{1F464}" }
                }
            }

            // Dropdown menu
            if *menu_open.read() {
                div { class: "user-menu-dropdown",
                    // Theme toggle
                    div {
                        class: "list-item",
                        onclick: move |_| {
                            let new_theme = theme.read().toggle();
                            apply_theme(&new_theme);
                            *THEME.write() = new_theme;
                        },
                        span { style: "font-size: 18px; width: 24px; text-align: center;",
                            if *theme.read() == ThemeMode::Dark {
                                "\u{2600}"
                            } else {
                                "\u{1F319}"
                            }
                        }
                        div { class: "list-item-text",
                            div { class: "list-item-primary",
                                if *theme.read() == ThemeMode::Dark {
                                    "{t(\"layout.light\")}"
                                } else {
                                    "{t(\"layout.dark\")}"
                                }
                            }
                        }
                    }

                    // Language toggle
                    div {
                        class: "list-item",
                        onclick: move |_| {
                            let new_lang = match *LANG.read() {
                                Lang::En => Lang::Da,
                                Lang::Da => Lang::En,
                            };
                            *LANG.write() = new_lang;
                        },
                        span { style: "font-size: 18px; width: 24px; text-align: center;", "\u{1F310}" }
                        div { class: "list-item-text",
                            div { class: "list-item-primary",
                                {match *LANG.read() {
                                    Lang::En => "Dansk",
                                    Lang::Da => "English",
                                }}
                            }
                        }
                    }

                    if is_auth {
                        // Set password
                        div {
                            class: "list-item",
                            onclick: move |_| {
                                nav.push(Route::SetPassword {});
                                menu_open.set(false);
                            },
                            span { style: "font-size: 18px; width: 24px; text-align: center;", "\u{1F512}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{t(\"auth.setPassword\")}" }
                            }
                        }

                        // Logout
                        div {
                            class: "list-item",
                            onclick: move |_| {
                                crate::nhost::sign_out();
                                *SESSION.write() = Default::default();
                                save_session(&Default::default());
                                menu_open.set(false);
                                nav.push(Route::HomeApp {});
                            },
                            span { style: "font-size: 18px; width: 24px; text-align: center;", "\u{1F6AA}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{t(\"auth.logout\")}" }
                            }
                        }
                    } else {
                        // Login
                        div {
                            class: "list-item",
                            onclick: move |_| {
                                nav.push(Route::Login {});
                                menu_open.set(false);
                            },
                            span { style: "font-size: 18px; width: 24px; text-align: center;", "\u{1F511}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{t(\"common.logIn\")}" }
                            }
                        }

                        // Register
                        div {
                            class: "list-item",
                            onclick: move |_| {
                                nav.push(Route::Register {});
                                menu_open.set(false);
                            },
                            span { style: "font-size: 18px; width: 24px; text-align: center;", "\u{1F464}" }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{t(\"auth.register\")}" }
                            }
                        }
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
                        div { class: "avatar", "\u{1F464}" }
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
                    div { class: "avatar small", "\u{1F3E0}" }
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

/// MenuList — the in-context drawer tree. Resolves the context (first path
/// segment) node, then renders its children lazily and expandably, mirroring
/// the React `MenuList`/`DrawerList`/`DrawerElement` trio.
#[component]
fn MenuList(segments: Vec<String>) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_key = segments.first().cloned().unwrap_or_default();

    let context = use_resource(move || {
        let token = access_token.clone();
        let key = context_key.clone();
        async move {
            graphql::resolve_path(token.as_deref(), &[key])
                .await
                .ok()
                .flatten()
                .map(|n| n.id.0)
        }
    });

    let hint_style = "padding: 4px 16px; color: var(--md-on-surface-variant);";
    let ctx = context.read().clone();
    match ctx {
        Some(Some(context_id)) => rsx! {
            div { class: "list", style: "margin-top: 8px;",
                DrawerLevel {
                    parent_id: context_id,
                    path_prefix: segments[..1].to_vec(),
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

    let children = use_resource(move || {
        let token = access_token.clone();
        let parent = parent.clone();
        let user_id = user_id.clone();
        async move {
            let Some(user_id) = user_id else {
                return Vec::new();
            };
            graphql::query_children(token.as_deref(), &parent, &user_id)
                .await
                .unwrap_or_default()
        }
    });

    let items = children.read().clone();
    match items {
        Some(items) => rsx! {
            for child in items.iter() {
                DrawerNodeItem {
                    key: "{child.id.0}",
                    node: child.clone(),
                    path_prefix: path_prefix.clone(),
                    current_path: current_path.clone(),
                    depth,
                }
            }
        },
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
    let icon = super::loader::mime_icon(&mime_id);

    // Auto-expand ancestors of the current node; let the user toggle the rest.
    let mut expanded = use_signal(|| on_path && !selected);

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
            div { class: "avatar small", "{icon}" }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{node.name}" }
                if node.mutable {
                    div { class: "list-item-secondary",
                        "\u{1F513} {t(\"layout.notSubmitted\")}"
                    }
                }
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
                    if *expanded.read() { "\u{25BE}" } else { "\u{25B8}" }
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

    let contexts = use_resource(move || {
        let token = access_token.clone();
        let user_id = user_id.clone();
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
