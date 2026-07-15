use dioxus::prelude::*;

use crate::graphql::{self, NodeFields};
use crate::i18n::t;
use crate::route::Route;
use crate::session::SESSION;

mod appbar;
mod breadcrumbs;
mod drawer;
mod home_list;
mod navigation;
mod search;
mod usermenu;

use appbar::*;
use breadcrumbs::*;
use drawer::*;
pub use home_list::HomeList;
use navigation::*;
use search::*;
use usermenu::*;

/// Per-navigation chrome state, resolved once in [`Layout`]: the breadcrumb
/// crumbs for the current path and the current context depth (how many leading
/// segments belong to the nearest group/event, per
/// [`graphql::deepest_context_depth`]). The breadcrumbs, drawer and app rail all
/// read these so they agree on the context without each re-querying the path.
pub(super) static NAV_CRUMBS: GlobalSignal<Vec<graphql::Crumb>> = Signal::global(Vec::new);

pub(super) static CONTEXT_DEPTH: GlobalSignal<usize> = Signal::global(|| 0);

/// The signed-in user's pending-invitation count, for the Home nav badge. Set by
/// [`Layout`] (once per session, refreshed on mutations via the data version).
pub(super) static PENDING_INVITES: GlobalSignal<usize> = Signal::global(|| 0);

/// The current context's key-path: the leading `CONTEXT_DEPTH` segments (or the
/// first segment as a fallback until the context resolves).
pub(super) fn context_path(segments: &[String]) -> Vec<String> {
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
    // DESIGN (functional): a keyboard-shortcuts help overlay, opened with "?".
    let mut shortcuts_open = use_signal(|| false);
    let search_input = use_signal(String::new);
    let search_results = use_signal(Vec::<NodeFields>::new);

    let route = use_route::<Route>();
    let nav = use_navigator();

    // DESIGN (functional): scroll to the top when navigating to a DIFFERENT
    // node, so each page starts at the top rather than wherever the previous one
    // was left scrolled. Keyed on the path (not the ?app= view), so switching
    // apps in place keeps your scroll position.
    let path_key = match &route {
        Route::PathPage { segments, .. } => segments.join("/"),
        _ => String::new(),
    };
    use_effect(use_reactive!(|(path_key,)| {
        let _ = &path_key;
        if let Some(win) = web_sys::window() {
            win.scroll_to_with_x_and_y(0.0, 0.0);
        }
    }));
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
    // Return focus to the rail's menu trigger when the modal tree overlay closes.
    let mut tree_return_focus = use_signal(|| None::<web_sys::HtmlElement>);

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
            // DESIGN (functional): make the shell focusable and grab focus once on
            // mount, so keyboard shortcuts (Ctrl+K, /, ?) work immediately rather than
            // only after the user first clicks something inside the app.
            tabindex: "-1",
            onmounted: move |e| {
                spawn(async move {
                    let _ = e.set_focus(true).await;
                });
            },
            "data-size-class": "{size_class.as_str()}",
            "data-tree-open": if tree_open() { "true" } else { "false" },
            // Set when a view mounts a permanent (docked) tools side sheet, so the
            // content pane reserves room for it on the right.
            "data-tools-docked": if super::widgets::TOOLS_DOCKED() { "true" } else { "false" },
            // DESIGN (functional): reflect the UI density preference.
            "data-density": if crate::density::COMPACT_DENSITY() { "compact" } else { "comfortable" },
            // Ctrl/Cmd+K opens search (a common shortcut). Catches keydowns that
            // bubble up from any focused element in the app.
            onkeydown: move |evt| {
                let m = evt.modifiers();
                // Whether focus is in a text field / the editor — bare-key shortcuts
                // must not fire while typing.
                let typing = web_sys::window()
                    .and_then(|w| w.document())
                    .and_then(|d| d.active_element())
                    .map(|e| {
                        let tag = e.tag_name().to_lowercase();
                        tag == "input" || tag == "textarea" || e.has_attribute("contenteditable")
                    })
                    .unwrap_or(false);
                if (m.ctrl() || m.meta()) && evt.key() == Key::Character("k".to_string()) {
                    search_mode.set(true);
                    evt.prevent_default();
                } else if !typing && !m.ctrl() && !m.meta()
                    && evt.key() == Key::Character("/".to_string())
                {
                    // DESIGN (functional): bare "/" opens search too.
                    search_mode.set(true);
                    evt.prevent_default();
                } else if !typing && evt.key() == Key::Character("?".to_string()) {
                    // DESIGN (functional): "?" opens the keyboard-shortcuts help.
                    shortcuts_open.set(true);
                    evt.prevent_default();
                }
            },
            // DESIGN (functional a11y): a skip link — the first focusable
            // element, visually hidden until focused — jumps keyboard users past
            // the chrome straight to the content.
            a { class: "skip-link", href: "#main-content", "{t(\"common.skipToContent\")}" }

            // DESIGN (functional): keyboard-shortcuts help overlay (opened with ?).
            super::widgets::Dialog {
                open: shortcuts_open(),
                on_dismiss: move |_| shortcuts_open.set(false),
                headline: t("common.keyboardShortcuts"),
                icon: "keyboard".to_string(),
                actions: rsx! {
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| shortcuts_open.set(false),
                        "{t(\"common.close\")}"
                    }
                },
                div { class: "shortcut-list",
                    div { class: "shortcut-row",
                        span { class: "shortcut-keys",
                            span { class: "kbd", "Ctrl" }
                            span { class: "kbd", "K" }
                        }
                        span { "{t(\"common.search\")}" }
                    }
                    div { class: "shortcut-row",
                        span { class: "shortcut-keys", span { class: "kbd", "/" } }
                        span { "{t(\"common.search\")}" }
                    }
                    div { class: "shortcut-row",
                        span { class: "shortcut-keys", span { class: "kbd", "?" } }
                        span { "{t(\"common.shortcutHelp\")}" }
                    }
                    div { class: "shortcut-row",
                        span { class: "shortcut-keys", span { class: "kbd", "Esc" } }
                        span { "{t(\"common.close\")}" }
                    }
                }
            }

            // Page table-of-contents popover (opened by clicking the current crumb).
            // Rendered here, at the app-shell root, so it escapes the breadcrumbs bar's
            // overflow clip and transform containing-block.
            breadcrumbs::TocPopover {}

            // Pull-to-refresh spinner (fixed overlay; listens on the window).
            super::pull_refresh::PullToRefresh {}

            // A back-to-top button + a reading-progress bar.
            super::back_to_top::ReadingProgress {}
            super::back_to_top::BackToTop {}

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
                    // On medium the tree opens as a modal overlay (scrim): give it
                    // the same modal a11y as the other overlays (name, focus trap,
                    // Escape, return-focus). On large/xl it is a persistent
                    // complementary landmark, so none of that applies.
                    role: if modal_tree { "dialog" } else { "complementary" },
                    "aria-modal": if modal_tree { "true" } else { "false" },
                    "aria-label": t("common.menu"),
                    tabindex: if modal_tree { "-1" } else { "" },
                    onkeydown: move |e| {
                        if !modal_tree {
                            return;
                        }
                        match e.key() {
                            Key::Escape => crate::components::widgets::close_modal(tree_open, tree_return_focus),
                            Key::Tab
                                if crate::components::widgets::trap_tab_focus(".nav-tree-pane.open", e.modifiers().shift()) =>
                            {
                                e.prevent_default();
                            }
                            _ => {}
                        }
                    },
                    // Focus sentinel: capture the trigger + pull focus into the
                    // overlay when it opens modally.
                    if modal_tree {
                        div {
                            class: "sheet-focus-sentinel",
                            tabindex: "-1",
                            onmounted: move |ev| {
                                tree_return_focus.set(crate::components::widgets::active_html_element());
                                spawn(async move {
                                    let _ = ev.set_focus(true).await;
                                });
                            },
                        }
                    }
                    div {
                        class: "nav-rail-tree",
                        // On the modal (overlay) tree, a click inside collapses it
                        // after navigating.
                        onclick: move |_| {
                            if modal_tree {
                                crate::components::widgets::close_modal(tree_open, tree_return_focus);
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
                    onclick: move |_| crate::components::widgets::close_modal(tree_open, tree_return_focus),
                }
            }

            // Top app bar. DESIGN: on compact it joins the navigation bar in a
            // single unified bottom dock (one elevated surface) to reclaim the
            // vertical space a second free-floating bar would cost; on medium+ it is
            // the top bar as before.
            if size_class.is_compact() {
                div { class: "compact-dock",
                    TopAppBar { search_mode, search_input, search_results, open_drawer }
                    NavigationBar {}
                }
            } else {
                TopAppBar { search_mode, search_input, search_results, open_drawer }
            }

            // Content pane (the resolved ?app= view). An inner measure caps the
            // reading column at ~A4 and centres it, so content stays legible on
            // wide screens instead of stretching edge to edge.
            main { class: "content-pane", id: "main-content",
                div { class: "content-measure",
                    Outlet::<Route> {}
                    div { class: "bar-spacer" }
                }
            }

        }

        // Compact keeps a modal navigation drawer for the tree (M3 uses a modal
        // drawer on phones); medium+ uses the expandable rail above.
        if size_class.is_compact() {
            NavigationDrawer { open: open_drawer }
        }
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
