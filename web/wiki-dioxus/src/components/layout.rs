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
                                    nav.push(Route::PathPage { segments: vec![key.clone()] });
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
        Route::PathPage { segments } => segments.clone(),
        _ => vec![],
    };

    rsx! {
        div { class: "breadcrumbs",
            Link { to: Route::HomeApp {}, "\u{1F3E0}" }
            for (i , segment) in segments.iter().enumerate() {
                span { class: "separator", " / " }
                if i == segments.len() - 1 {
                    span { "{segment}" }
                } else {
                    Link {
                        to: Route::PathPage {
                            segments: segments[..=i].to_vec(),
                        },
                        "{segment}"
                    }
                }
            }
        }
    }
}

/// App rail — vertical icon navigation for large screens
#[component]
fn AppRail() -> Element {
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments } => segments.clone(),
        _ => vec![],
    };

    if segments.is_empty() {
        return rsx! {};
    }

    let apps = [
        ("member", "\u{1F465}", t("mime.members")),
        ("speak", "\u{1F3A4}", t("mime.speak")),
        ("vote", "\u{1F4CA}", t("mime.vote")),
    ];

    rsx! {
        for (_app_id , icon , label) in apps.iter() {
            Link {
                to: Route::PathPage { segments: segments.clone() },
                class: "btn-icon",
                style: "flex-direction: column; gap: 2px; width: 56px; height: 56px;",
                title: "{label}",
                span { style: "font-size: 20px;", "{icon}" }
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

            // HomeList — load groups and events from GraphQL
            if is_auth {
                HomeList {}
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
                            nav.push(Route::PathPage { segments });
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
