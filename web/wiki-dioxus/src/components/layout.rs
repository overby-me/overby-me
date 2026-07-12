use dioxus::prelude::*;

use super::ui::switch::Switch;
use crate::graphql::{self, NodeFields};
use crate::i18n::{t, Lang, LANG};
use crate::route::Route;
use crate::session::{save_session, use_session, SESSION};
use crate::theme::{apply_theme, use_theme, ThemeMode, THEME};

/// Per-navigation chrome state, resolved once in [`Layout`]: the breadcrumb
/// crumbs for the current path and the current context depth (how many leading
/// segments belong to the nearest group/event, per
/// [`graphql::deepest_context_depth`]). The breadcrumbs, drawer and app rail all
/// read these so they agree on the context without each re-querying the path.
static NAV_CRUMBS: GlobalSignal<Vec<graphql::Crumb>> = Signal::global(Vec::new);
static CONTEXT_DEPTH: GlobalSignal<usize> = Signal::global(|| 0);
/// The signed-in user's pending-invitation count, for the Home nav badge. Set by
/// [`Layout`] (once per session, refreshed on mutations via the data version).
static PENDING_INVITES: GlobalSignal<usize> = Signal::global(|| 0);

/// The current context's key-path: the leading `CONTEXT_DEPTH` segments (or the
/// first segment as a fallback until the context resolves).
fn context_path(segments: &[String]) -> Vec<String> {
    let depth = CONTEXT_DEPTH().max(1);
    segments.iter().take(depth).cloned().collect()
}

thread_local! {
    /// A password-reset `refreshToken` captured from the launch URL before the
    /// router normalized the query away (see [`capture_reset_token`]).
    static PENDING_RESET: std::cell::RefCell<Option<String>> =
        const { std::cell::RefCell::new(None) };
}

/// Stash the `refreshToken` from a password-reset deep link
/// (`/?type=passwordReset&refreshToken=...`) so [`Layout`] can act on it. Called
/// from `main` *before* the router mounts: the router renders home for `/` and
/// rewrites the query, which would otherwise drop these params before any
/// component reads them.
pub fn capture_reset_token() {
    let Some(search) = web_sys::window().and_then(|w| w.location().search().ok()) else {
        return;
    };
    let Some(raw) = parse_reset_token(&search) else {
        return;
    };
    // The token is usually URL-safe, but decode any percent-encoding to be safe.
    let token = js_sys::decode_uri_component(&raw)
        .ok()
        .map(|s| String::from(&s))
        .unwrap_or(raw);
    PENDING_RESET.with(|c| *c.borrow_mut() = Some(token));
}

/// Take the captured password-reset token (consuming it), if any.
fn take_reset_token() -> Option<String> {
    PENDING_RESET.with(|c| c.borrow_mut().take())
}

/// Extract the `refreshToken` value from a `?type=passwordReset&refreshToken=...`
/// query string, or `None` when it is not a password-reset link. Pure (no
/// percent-decoding) so it is unit-testable off the wasm target.
fn parse_reset_token(search: &str) -> Option<String> {
    let query = search.strip_prefix('?').unwrap_or(search);
    let mut is_reset = false;
    let mut token: Option<String> = None;
    for pair in query.split('&') {
        let Some((k, v)) = pair.split_once('=') else {
            continue;
        };
        match k {
            "type" if v == "passwordReset" => is_reset = true,
            "refreshToken" => token = Some(v.to_string()),
            _ => {}
        }
    }
    if is_reset {
        token
    } else {
        None
    }
}

#[component]
pub fn Layout() -> Element {
    let open_drawer = use_signal(|| false);
    let mut search_mode = use_signal(|| false);
    let search_input = use_signal(String::new);
    let search_results = use_signal(Vec::<NodeFields>::new);
    let menu_open = use_signal(|| false);

    let route = use_route::<Route>();
    let nav = use_navigator();
    // Reactive M3 window size class (adaptive nav + panes). Called before the
    // early returns below so the hook order stays stable across auth/screen pages.
    let size_class = crate::window_size::use_window_size();

    // The app rail stays a fixed icon rail; the groups/events context tree lives in
    // a pane to its RIGHT that the rail's menu button animates open/closed (the
    // icons never move). Open by default on large/xl (persistent, pushes content);
    // on medium/expanded it opens as a modal overlay (scrim). Compact keeps a modal
    // drawer.
    let is_large = use_memo(move || crate::window_size::WINDOW_SIZE().is_expanded_rail());
    let mut tree_open = use_signal(|| crate::window_size::WINDOW_SIZE().is_expanded_rail());
    // Reset the tree to its per-size default (open on large/xl, closed below) only
    // when the large threshold is crossed — a manual toggle persists otherwise.
    use_effect(move || tree_open.set(is_large()));
    let modal_tree = tree_open() && !is_large() && !size_class.is_compact();

    // NHost password-reset emails link to `/?type=passwordReset&refreshToken=...`,
    // which the router renders as home. The token was stashed by
    // `capture_reset_token` in `main` (before the router dropped the query); act
    // on it once here: exchange it for a session and route to set-password.
    use_hook(move || {
        if let Some(rt) = take_reset_token() {
            spawn(async move {
                if crate::session::establish_from_refresh_token(&rt).await {
                    nav.replace(Route::SetPassword {});
                }
            });
        }
    });

    // Resolve the path once for the whole chrome. The breadcrumbs, drawer and app
    // rail all key off the current context (the nearest group/event), so
    // resolving it here keeps them consistent and avoids each re-querying.
    {
        let segments = match &route {
            Route::PathPage { segments, .. } => segments.clone(),
            _ => vec![],
        };
        let token = SESSION.read().access_token.clone();
        crate::use_data_resource!(|(segments, token)| async move {
            let crumbs = graphql::path_crumbs(token.as_deref(), &segments)
                .await
                .unwrap_or_default();
            *CONTEXT_DEPTH.write() = graphql::deepest_context_depth(&crumbs);
            // Reflect the current node in the browser tab title.
            let title = match crumbs.last() {
                Some(c) => format!("{} · RadikalWiki", c.name),
                None => "RadikalWiki".to_string(),
            };
            if let Some(doc) = web_sys::window().and_then(|w| w.document()) {
                doc.set_title(&title);
            }
            *NAV_CRUMBS.write() = crumbs;
        });
    }

    // Pending-invitation count for the Home nav badge. Session-stable deps, so
    // this runs once per session and refreshes on mutations (data version).
    {
        let uid = SESSION.read().user.as_ref().map(|u| u.id.clone());
        let email = SESSION.read().user.as_ref().map(|u| u.email.clone());
        let token = SESSION.read().access_token.clone();
        crate::use_data_resource!(|(uid, email, token)| async move {
            if let (Some(uid), Some(email)) = (uid, email) {
                let list = graphql::query_invitations(token.as_deref(), &uid, &email)
                    .await
                    .unwrap_or_default();
                *PENDING_INVITES.write() = list.len();
            } else {
                *PENDING_INVITES.write() = 0;
            }
        });
    }

    // The stray trailing "?" the router emits for the optional `app` query is
    // cleaned globally at the source by `install_history_query_shim` (main.rs),
    // which wraps history.pushState/replaceState. No per-navigation strip here.

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

    // The projector view (`?app=screen`) is full-screen: no drawer / rail / bar,
    // just the active content + speaker list (React renders it chrome-less).
    let is_screen = matches!(&route, Route::PathPage { app: Some(a), .. } if a == "screen");
    if is_screen {
        return rsx! {
            div { class: "screen-full",
                Outlet::<Route> {}
            }
        };
    }

    rsx! {
        div {
            class: "app-shell",
            "data-size-class": "{size_class.as_str()}",
            "data-tree-open": if tree_open() { "true" } else { "false" },
            // Set when a view mounts a permanent (docked) tools side sheet, so the
            // content pane reserves room for it on the right.
            "data-tools-docked": if super::widgets::TOOLS_DOCKED() { "true" } else { "false" },
            // Ctrl/Cmd+K opens search (a common shortcut). Catches keydowns that
            // bubble up from any focused element in the app.
            onkeydown: move |evt| {
                let m = evt.modifiers();
                if (m.ctrl() || m.meta()) && evt.key() == Key::Character("k".to_string()) {
                    search_mode.set(true);
                    evt.prevent_default();
                }
            },
            // Pull-to-refresh spinner (fixed overlay; listens on the window).
            super::pull_refresh::PullToRefresh {}

            // APP axis (medium+): a fixed icon rail. Its menu button toggles the
            // context tree pane. A bottom bar carries the apps on compact (below).
            if !size_class.is_compact() {
                NavigationRail {
                    tree_open: tree_open(),
                    on_toggle: move |_| {
                        let v = !tree_open();
                        tree_open.set(v);
                    },
                }
            }

            // PLACE axis (medium+): the groups/events tree in a pane to the RIGHT of
            // the app rail. It slides open/closed (the rail icons stay put), pushing
            // the content on large/xl and overlaying it (with a scrim) on medium.
            if !size_class.is_compact() {
                aside {
                    class: if tree_open() { "nav-tree-pane open" } else { "nav-tree-pane" },
                    div {
                        class: "nav-rail-tree",
                        // On the modal (overlay) tree, a click inside collapses it
                        // after navigating.
                        onclick: move |_| {
                            if modal_tree {
                                tree_open.set(false);
                            }
                        },
                        DrawerContent {}
                    }
                }
            }

            // Scrim behind the tree pane when it opens as a modal overlay (medium).
            if modal_tree {
                div {
                    class: "nav-rail-scrim open",
                    role: "presentation",
                    onclick: move |_| tree_open.set(false),
                }
            }

            // Top app bar spanning the content region.
            TopAppBar { search_mode, search_input, search_results, open_drawer, menu_open }

            // Content pane (the resolved ?app= view). An inner measure caps the
            // reading column at ~A4 and centres it, so content stays legible on
            // wide screens instead of stretching edge to edge.
            main { class: "content-pane",
                div { class: "content-measure",
                    Outlet::<Route> {}
                    div { class: "bar-spacer" }
                }
            }

            if size_class.is_compact() {
                NavigationBar {}
            }
        }

        // Compact keeps a modal navigation drawer for the tree (M3 uses a modal
        // drawer on phones); medium+ uses the expandable rail above.
        if size_class.is_compact() {
            NavigationDrawer { open: open_drawer }
        }
    }
}

/// Issue a search for `value` with the given scope (`scoped` = a context id to
/// restrict to, or `None` for site-wide), applying the result only if it is
/// still the latest request. Signals are `Copy`, so callers pass them by value.
fn search_run(
    value: String,
    mut results: Signal<Vec<NodeFields>>,
    mut seq: Signal<u32>,
    token: Option<String>,
    scoped: Option<String>,
) {
    if value.trim().is_empty() {
        results.set(vec![]);
        return;
    }
    let my = seq() + 1;
    seq.set(my);
    spawn(async move {
        let nodes = graphql::search_nodes(token.as_deref(), &value, scoped.as_deref())
            .await
            .unwrap_or_default();
        if seq() == my {
            results.set(nodes);
        }
    });
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
    // Monotonic request id so out-of-order responses don't clobber newer ones
    // (typing fires a query per keystroke; the last issued must win, not the
    // last to return). `search_run` owns the writes, so these stay immutable here.
    let seq = use_signal(|| 0u32);
    // Keyboard-highlighted result (arrow keys move it, Enter opens it).
    let mut selected = use_signal(|| 0usize);

    // Resolve the current context (nearest group/event) so the search can be
    // scoped to it. `in_context` toggles between context-only and site-wide.
    let route = use_route::<Route>();
    let segments = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => Vec::new(),
    };
    let cp = context_path(&segments);
    let ctx_token = session.read().access_token.clone();
    let context = use_resource(use_reactive!(|(cp, ctx_token)| async move {
        if cp.is_empty() {
            return None;
        }
        graphql::resolve_path(ctx_token.as_deref(), &cp)
            .await
            .ok()
            .flatten()
            .map(|n| n.id.0)
    }));
    let has_context = context.read().clone().flatten().is_some();
    let mut in_context = use_signal(|| false);

    rsx! {
        div { style: "flex: 1; position: relative; display: flex; align-items: center; gap: 2px;",
            if has_context {
                button {
                    class: "btn-icon",
                    title: if in_context() { t("common.searchEverywhere") } else { t("common.searchInSection") },
                    onclick: move |_| {
                        let now = in_context();
                        in_context.set(!now);
                        let scoped = if !now { context.read().clone().flatten() } else { None };
                        let token = session.read().access_token.clone();
                        search_run(input.read().clone(), results, seq, token, scoped);
                    },
                    span { class: "material-icons", {if in_context() { "folder" } else { "public" }} }
                }
            }
            input {
                class: "breadcrumbs",
                style: "background: transparent; border: none; color: white; outline: none; font-size: 14px; flex: 1; min-width: 0;",
                placeholder: "{t(\"common.search\")}",
                value: "{input}",
                oninput: move |evt| {
                    let value = evt.value();
                    input.set(value.clone());
                    selected.set(0);
                    let scoped = if in_context() { context.read().clone().flatten() } else { None };
                    let token = session.read().access_token.clone();
                    search_run(value, results, seq, token, scoped);
                },
                onkeydown: move |evt| {
                    let len = results.read().len();
                    match evt.key() {
                        Key::Escape => on_close.call(()),
                        Key::ArrowDown if len > 0 => {
                            let s = (*selected.read() + 1).min(len - 1);
                            selected.set(s);
                            evt.prevent_default();
                        }
                        Key::ArrowUp if len > 0 => {
                            let s = selected.read().saturating_sub(1);
                            selected.set(s);
                            evt.prevent_default();
                        }
                        Key::Enter => {
                            let idx = *selected.read();
                            if let Some(node) = results.read().get(idx) {
                                let node_id = node.id.0.clone();
                                let key = node.key.clone();
                                let token = session.read().access_token.clone();
                                spawn(async move {
                                    let segments = resolve_result_path(node_id, key, token).await;
                                    nav.push(Route::PathPage { segments, app: None });
                                    on_close.call(());
                                });
                            }
                        }
                        _ => {}
                    }
                },
            }
            // Search results dropdown
            if !results.read().is_empty() {
                div { class: "search-results",
                    for (idx , node) in results.read().iter().enumerate() {
                        div {
                            class: if idx == *selected.read() { "list-item selected" } else { "list-item" },
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
                                    // Resolve first, THEN navigate + close: closing
                                    // unmounts the SearchBar, cancelling the task.
                                    spawn(async move {
                                        let segments = resolve_result_path(node_id, key, token).await;
                                        nav.push(Route::PathPage { segments, app: None });
                                        on_close.call(());
                                    });
                                }
                            },
                            div { class: "avatar small",
                                {super::loader::node_icon_el(node.mime_id.as_deref().unwrap_or(""), node.data.as_ref().map(|d| &d.0))}
                            }
                            div { class: "list-item-text",
                                div { class: "list-item-primary", "{node.name}" }
                                if let Some(parent) = node.parent.as_ref() {
                                    div { class: "list-item-secondary", "{parent.name}" }
                                }
                            }
                        }
                    }
                }
            }
        }
        button {
            class: "btn-icon",
            aria_label: "{t(\"common.close\")}",
            onclick: move |_| on_close.call(()),
            span { class: "material-icons", "close" }
        }
    }
}

/// Resolve a search hit's full ancestor path (root excluded), falling back to the
/// bare key. A hit can live anywhere in the tree, so we resolve its ancestors
/// rather than treating the key as a top-level segment. Shared by a result click
/// and the Enter key.
async fn resolve_result_path(node_id: String, key: String, token: Option<String>) -> Vec<String> {
    let mut segments = graphql::path_from_id(token.as_deref(), &node_id)
        .await
        .unwrap_or_default();
    if segments.is_empty() {
        segments = vec![key];
    }
    segments
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
                    to: Route::Home { app: None },
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
                    let mime = info
                        .map(|c| {
                            super::loader::node_icon_mime_id(
                                c.mime_id.as_deref().unwrap_or(""),
                                c.data.as_ref().map(|d| &d.0),
                            )
                        })
                        .unwrap_or_default();
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
            // Clicking a crumb (navigating to an ancestor, or re-clicking the
            // current node) scrolls the content back to the top.
            onclick: move |_| {
                if let Some(win) = web_sys::window() {
                    win.scroll_to_with_x_and_y(0.0, 0.0);
                }
            },
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
/// The context apps for the current route (home, folder, and for authed users
/// speak/vote/member), each as `(mime, label, route, is-active)`. Shared by the
/// desktop rail and the mobile app bar, mirroring React's `useApps`. Empty off a
/// context (`segments` empty), which hides both nav surfaces.
fn context_apps(route: &Route, is_auth: bool) -> Vec<(&'static str, String, Route, bool)> {
    let segments: Vec<String> = match route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    if segments.is_empty() {
        // Home: the folder/speak/vote/member apps all need a context, but show
        // the Home destination (active) so the rail isn't blank on the landing
        // page.
        return vec![(
            "app/home",
            t("common.home"),
            Route::Home { app: None },
            true,
        )];
    }
    let current_app = match route {
        Route::PathPage { app, .. } => app.clone(),
        _ => None,
    };
    // The app is part of the route's query, so these navigate client-side and the
    // resolver swaps the view without a reload.
    let ctx_path = context_path(&segments);

    let mut apps: Vec<(&str, String, Route, bool)> = vec![
        (
            "app/home",
            t("common.home"),
            Route::Home { app: None },
            false,
        ),
        (
            "app/folder",
            t("mime.folder"),
            Route::PathPage {
                segments: ctx_path.clone(),
                app: None,
            },
            // The editor / sort sub-apps operate on folder content, so the folder
            // rail item stays highlighted while they are open.
            current_app.is_none()
                || matches!(current_app.as_deref(), Some("editor") | Some("sort")),
        ),
    ];
    if is_auth {
        for (app, label) in [
            ("speak", t("mime.speak")),
            ("vote", t("mime.vote")),
            // Members: React only surfaces this to owners, but MemberApp gates
            // its admin controls itself, so the entry is safe for any authed user.
            ("member", t("common.members")),
        ] {
            apps.push((
                match app {
                    "speak" => "app/speak",
                    "vote" => "app/vote",
                    _ => "app/member",
                },
                label,
                Route::PathPage {
                    segments: ctx_path.clone(),
                    app: Some(app.to_string()),
                },
                current_app.as_deref() == Some(app),
            ));
        }
        // The other apps (screen, admin, program, graph, social, map, profile,
        // perm, parent) are still reachable via their `?app=` URL but hidden
        // from these nav surfaces until they are ready to show.
    }
    apps
}

/// App-axis navigation rail: a fixed icon + label strip (medium+). Its menu button
/// opens/closes the groups/events tree pane to the rail's right — the app icons
/// never move. One primary-container pill tracks the active `?app=` destination.
#[component]
fn NavigationRail(tree_open: bool, on_toggle: EventHandler<()>) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let route = use_route::<Route>();
    let apps = context_apps(&route, is_auth);
    let pending = PENDING_INVITES();

    rsx! {
        nav { class: "nav-rail",
            // Header: the menu button opens / closes the context tree pane.
            div { class: "nav-rail-header",
                button {
                    class: "btn-icon state-layer",
                    aria_label: t("common.menu"),
                    "aria-expanded": if tree_open { "true" } else { "false" },
                    onclick: move |_| on_toggle.call(()),
                    span { class: "material-icons",
                        if tree_open { "menu_open" } else { "menu" }
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
                        title: "{label}",
                        span { class: "nav-rail-indicator",
                            span { class: "app-rail-icon", {super::loader::icon_el(mime_id)} }
                            if mime_id == "app/home" && pending > 0 {
                                super::widgets::Badge { count: Some(pending) }
                            }
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
/// the rail, with the secondary-container pill indicator and the Home badge.
#[component]
fn NavigationBar() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let route = use_route::<Route>();
    let apps = context_apps(&route, is_auth);
    if apps.is_empty() {
        return rsx! {};
    }
    let pending = PENDING_INVITES();

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
                    title: "{label}",
                    aria_label: "{label}",
                    span { class: "nav-bar-indicator",
                        span { class: "app-rail-icon", {super::loader::icon_el(mime_id)} }
                        if mime_id == "app/home" && pending > 0 {
                            super::widgets::Badge { count: Some(pending) }
                        }
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
fn AppSwitcher(apps: Vec<(&'static str, String, Route, bool)>) -> Element {
    let mut open = use_signal(|| false);
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);
    rsx! {
        button {
            class: "nav-bar-item state-layer",
            r#type: "button",
            aria_label: t("common.apps"),
            onclick: move |_| {
                return_focus.set(super::widgets::active_html_element());
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
            onclick: move |_| super::widgets::close_modal(open, return_focus),
        }
        aside {
            class: if open() { "tool-sheet open" } else { "tool-sheet" },
            role: "dialog",
            "aria-modal": "true",
            "aria-label": t("common.apps"),
            tabindex: "-1",
            onkeydown: move |e| {
                match e.key() {
                    Key::Escape => super::widgets::close_modal(open, return_focus),
                    Key::Tab if super::widgets::trap_tab_focus(".tool-sheet.open", e.modifiers().shift()) => {
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
                    onclick: move |_| super::widgets::close_modal(open, return_focus),
                    span { class: "material-icons", "close" }
                }
            }
            div {
                class: "tool-sheet-body",
                onclick: move |_| super::widgets::close_modal(open, return_focus),
                for (mime_id , label , to , active) in apps.into_iter() {
                    Link {
                        key: "{mime_id}",
                        to,
                        class: if active { "sheet-action selected" } else { "sheet-action" },
                        {super::loader::icon_el(mime_id)}
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
fn NavigationDrawer(open: Signal<bool>) -> Element {
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

    rsx! {
        div {
            class: if open() { "nav-drawer-scrim open" } else { "nav-drawer-scrim" },
            onclick: move |_| open.set(false),
        }
        aside { class: if open() { "nav-drawer open" } else { "nav-drawer" },
            div { class: "nav-drawer-header",
                if segments.is_empty() {
                    Link {
                        to: Route::Home { app: None },
                        class: "bar drawer-context-bar",
                        onclick: move |_| open.set(false),
                        div { class: "avatar small",
                            span { class: "material-icons", "home" }
                        }
                        span { class: "drawer-context-name", "{t(\"common.home\")}" }
                    }
                } else {
                    Link {
                        to: Route::PathPage { segments: ctx_path.clone(), app: None },
                        class: "bar drawer-context-bar",
                        onclick: move |_| open.set(false),
                        div { class: "avatar small", {super::loader::icon_el(&ctx_mime)} }
                        span { class: "drawer-context-name", "{ctx_name}" }
                    }
                }
                button {
                    class: "btn-icon state-layer",
                    aria_label: t("common.close"),
                    onclick: move |_| open.set(false),
                    span { class: "material-icons", "close" }
                }
            }
            div { onclick: move |_| open.set(false),
                DrawerContent {}
            }
        }
    }
}

/// M3 top app bar over the content region: a leading menu button (compact, opens
/// the tree drawer), the breadcrumb trail as the headline, and trailing search +
/// user menu. The docked search bar expands in place.
#[component]
fn TopAppBar(
    search_mode: Signal<bool>,
    search_input: Signal<String>,
    search_results: Signal<Vec<NodeFields>>,
    open_drawer: Signal<bool>,
    menu_open: Signal<bool>,
) -> Element {
    let size = crate::window_size::WINDOW_SIZE();

    rsx! {
        header { class: "top-app-bar",
            if size.is_compact() {
                button {
                    class: "btn-icon state-layer",
                    aria_label: t("common.menu"),
                    onclick: move |_| open_drawer.set(true),
                    span { class: "material-icons", "menu" }
                }
            }
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
                // M3 Expressive docked search: a rounded container holding the
                // breadcrumb trail behind a leading search affordance. Tapping the
                // icon opens full search; the crumbs stay tappable for up-navigation.
                div { class: "expressive-search",
                    button {
                        class: "expressive-search-btn state-layer",
                        aria_label: t("common.search"),
                        onclick: move |_| search_mode.set(true),
                        span { class: "material-icons", "search" }
                    }
                    div { class: "expressive-search-crumbs", Breadcrumbs {} }
                }
                UserMenu { menu_open }
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
        div { class: "user-menu",
            button {
                class: "btn-icon",
                aria_label: "{t(\"layout.userMenu\")}",
                onclick: move |_| {
                    let v = menu_open();
                    menu_open.set(!v);
                },
                if is_auth {
                    span { class: "avatar small secondary", "{initial}" }
                } else {
                    span { class: "avatar small", span { class: "material-icons", "person" } }
                }
            }
            if menu_open() {
                // Full-viewport click-catcher so a click anywhere else closes it.
                div { class: "menu-backdrop", onclick: move |_| menu_open.set(false) }
                div { class: "user-menu-dropdown",
                    // Signed-in identity header (the old wiki had no user card in
                    // the sidebar; this belongs with the account menu instead).
                    if is_auth {
                        div { class: "user-menu-header",
                            span { class: "avatar secondary", "{initial}" }
                            div { class: "user-menu-identity",
                                div { class: "user-menu-name", "{display_name}" }
                                div { class: "user-menu-email", "{email}" }
                            }
                        }
                    }
                    // Dark-mode toggle, as an accessible on/off switch (the menu
                    // stays open so the flip is visible).
                    div { class: "list-item switch-row",
                        span { class: "material-icons",
                            {if dark { "dark_mode" } else { "light_mode" }}
                        }
                        span { class: "switch-row-label", "{t(\"layout.darkMode\")}" }
                        Switch {
                            checked: Some(dark),
                            on_checked_change: move |on: bool| {
                                let new_theme = if on { ThemeMode::Dark } else { ThemeMode::Light };
                                apply_theme(&new_theme);
                                crate::theme::save_theme(&new_theme);
                                *THEME.write() = new_theme;
                            },
                        }
                    }
                    // Theme colours: pick the M3 primary + accent seeds. Changing
                    // either regenerates the tonal scheme and re-skins the whole
                    // app at runtime (see crate::theme::apply_seeds).
                    div { class: "menu-color-section",
                        span { class: "menu-color-title", "{t(\"layout.themeColor\")}" }
                        // Five preset swatches; the sixth circle in each row is the
                        // freeform custom picker. The first swatch is the brand
                        // default (selecting it clears the override).
                        super::widgets::ColorPicker {
                            label: t("layout.primaryColor"),
                            value: crate::theme::effective_primary(),
                            custom_title: t("layout.customColor"),
                            swatches: vec![
                                crate::theme::BRAND_PRIMARY.to_string(),
                                "#1565C0".to_string(),
                                "#6750A4".to_string(),
                                "#00796B".to_string(),
                                "#C62828".to_string(),
                            ],
                            on_change: move |hex: String| crate::theme::set_primary_seed(hex),
                        }
                        super::widgets::ColorPicker {
                            label: t("layout.accentColor"),
                            value: crate::theme::effective_accent(),
                            custom_title: t("layout.customColor"),
                            swatches: vec![
                                crate::theme::BRAND_ACCENT.to_string(),
                                "#7B1FA2".to_string(),
                                "#0097A7".to_string(),
                                "#F9A825".to_string(),
                                "#2E7D32".to_string(),
                            ],
                            on_change: move |hex: String| crate::theme::set_accent_seed(hex),
                        }
                    }
                    // Language toggle
                    button {
                        class: "list-item",
                        onclick: move |_| {
                            let new_lang = match *LANG.read() {
                                Lang::En => Lang::Da,
                                Lang::Da => Lang::En,
                            };
                            *LANG.write() = new_lang;
                            menu_open.set(false);
                        },
                        span { class: "material-icons", "language" }
                        {match *LANG.read() { Lang::En => " Dansk", Lang::Da => " English" }}
                    }
                    if is_auth {
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                nav.push(Route::SetPassword {});
                            },
                            span { class: "material-icons", "lock" }
                            " {t(\"auth.setPassword\")}"
                        }
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                crate::nhost::sign_out();
                                *SESSION.write() = Default::default();
                                save_session(&Default::default());
                                // Drop cached data so nothing from the old session
                                // lingers (React clears the GraphQL cache here).
                                crate::session::bump_data_version();
                                nav.push(Route::Home { app: None });
                            },
                            span { class: "material-icons", "logout" }
                            " {t(\"auth.logout\")}"
                        }
                    } else {
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                nav.push(Route::Login {});
                            },
                            span { class: "material-icons", "login" }
                            " {t(\"common.logIn\")}"
                        }
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                nav.push(Route::Register {});
                            },
                            span { class: "material-icons", "person_add" }
                            " {t(\"auth.register\")}"
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

    // The drawer's top entry switches from Home to the current context (the
    // nearest group/event, linking to its root) once inside one — mirroring the
    // old wiki's drawer, whose title switched Home → the selected context.
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

    rsx! {
        div { style: "padding: 0 16px 16px;",
            div { class: "list",
                if segments.is_empty() {
                    // At the home route: Home, styled as a bar like the top panel.
                    Link {
                        to: Route::Home { app: None },
                        class: "bar drawer-context-bar",
                        div { class: "avatar small", span { class: "material-icons", "home" } }
                        span { class: "drawer-context-name", "{t(\"common.home\")}" }
                    }
                } else {
                    // Inside a context: the current context, styled as a bar like
                    // the top panel (the old wiki's drawer Bar/Title), linking to
                    // its root.
                    Link {
                        to: Route::PathPage { segments: ctx_path.clone(), app: None },
                        class: "bar drawer-context-bar",
                        div { class: "avatar small", {super::loader::icon_el(&ctx_mime)} }
                        span { class: "drawer-context-name", "{ctx_name}" }
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

    // Re-resolve reactively when the context path changes. Deliberately a plain
    // `use_resource` (not `use_data_resource!`): this only maps the path to the
    // context node id, which is stable across data changes, and refetching it on
    // every refresh would briefly blank and remount the whole drawer tree. The
    // drawer's actual contents refresh via `DrawerLevel` instead.
    let cpath = ctx_path.clone();
    let context = use_resource(use_reactive!(|(cpath, access_token)| async move {
        graphql::resolve_path(access_token.as_deref(), &cpath)
            .await
            .ok()
            .flatten()
            .map(|n| n.id.0)
    }));

    let hint_style = "padding: 4px 16px;";
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
            p { class: "body-medium text-muted", style: "{hint_style}", "…" }
        },
    }
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

    let children = crate::use_data_resource!(|(parent, access_token, user_id)| async move {
        let Some(user_id) = user_id else {
            return Vec::new();
        };
        graphql::query_drawer_children(access_token.as_deref(), &parent, &user_id)
            .await
            .unwrap_or_default()
    });

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
    node: graphql::DrawerChildFields,
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
    // Show the expander only when the node actually has children the user can
    // see (per-row `children_aggregate` count), not merely because its mime type
    // *could* have children. Mirrors the React DrawerElement gate.
    let expandable = node.has_children();
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
            // The selected node gets the full active indicator; its ancestors on
            // the active path get a subtler tint (M3-style emphasis levels).
            class: if selected {
                "list-item selected"
            } else if on_path {
                "list-item ancestor"
            } else {
                "list-item"
            },
            style: "cursor: pointer; {indent}",
            onclick: move |_| {
                nav.push(Route::PathPage {
                    segments: nav_path.clone(),
                    app: None,
                });
            },
            super::loader::NodeAvatar {
                mime: super::loader::node_icon_mime_id(&mime_id, node.data.as_ref().map(|d| &d.0)),
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
                    aria_label: "{t(\"common.expand\")}",
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
pub fn HomeList() -> Element {
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

    let contexts = crate::use_data_resource!(move || {
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
    let hint_style = "padding: 4px 16px;";

    rsx! {
        div { style: "margin-top: 16px;",
            // Groups
            h4 { class: "title-small", class: "text-muted", style: "padding: 8px 16px;",
                "{t(\"layout.groups\")}"
            }
            {match &state {
                None => rsx! {
                    p { class: "body-medium text-muted", style: "{hint_style}", "…" }
                },
                Some(Err(e)) => rsx! {
                    p { class: "body-medium text-muted", style: "{hint_style}", "{e}" }
                },
                Some(Ok((groups, _))) if groups.is_empty() => rsx! {
                    p { class: "body-medium text-muted", style: "{hint_style}", "{t(\"layout.noGroups\")}" }
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
            h4 { class: "title-small", class: "text-muted", style: "padding: 8px 16px; margin-top: 8px;",
                "{t(\"layout.events\")}"
            }
            {match &state {
                None => rsx! {
                    p { class: "body-medium text-muted", style: "{hint_style}", "…" }
                },
                Some(Err(e)) => rsx! {
                    p { class: "body-medium text-muted", style: "{hint_style}", "{e}" }
                },
                Some(Ok((_, events))) if events.is_empty() => rsx! {
                    p { class: "body-medium text-muted", style: "{hint_style}", "{t(\"layout.noEvents\")}" }
                },
                Some(Ok((_, events))) => rsx! {
                    for (year , items) in group_by_year(events) {
                        div { key: "{year}",
                            p { class: "label-medium",
                                class: "text-muted", style: "padding: 4px 16px; font-weight: 600;",
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
            div { class: "avatar small secondary avatar-abbr", "{abbr}" }
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
            // An acronym (e.g. "EU-"): keep only its uppercase letters, so
            // trailing punctuation like a hyphen cannot break onto a new line.
            _ if upper_count(w) > 1 => w.chars().filter(|c| c.is_uppercase()).collect(),
            _ => w.chars().next().unwrap().to_string(),
        })
        .collect();

    match words.len() {
        // Two characters sit comfortably in the avatar circle (e.g. "EU", "KM",
        // "HB"); more than that reads as crammed.
        1..=3 => words.concat().chars().take(2).collect(),
        _ => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::parse_reset_token;

    #[test]
    fn extracts_reset_token() {
        assert_eq!(
            parse_reset_token("?type=passwordReset&refreshToken=abc123"),
            Some("abc123".to_string())
        );
        // Order-independent.
        assert_eq!(
            parse_reset_token("?refreshToken=xyz&type=passwordReset"),
            Some("xyz".to_string())
        );
    }

    #[test]
    fn ignores_non_reset_links() {
        assert_eq!(parse_reset_token(""), None);
        assert_eq!(parse_reset_token("?app=vote"), None);
        // A refresh token without the reset type is not a reset link.
        assert_eq!(parse_reset_token("?refreshToken=abc"), None);
        // The reset type without a token yields nothing to exchange.
        assert_eq!(parse_reset_token("?type=passwordReset"), None);
    }
}
