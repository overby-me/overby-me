use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::{t, t_with};
use crate::model::{self, NodeFields};
use crate::route::Route;
use crate::session::{use_session, SESSION};

mod appbar;
mod breadcrumbs;
mod drawer;
mod home_list;
mod navigation;
mod public_places;
mod search;
mod usermenu;

use appbar::*;
use breadcrumbs::*;
use drawer::*;
pub use home_list::HomeList;
use navigation::*;
pub use public_places::PublicPlaces;
use search::*;
use usermenu::*;

/// Per-navigation chrome state, resolved once in [`Layout`]: the breadcrumb
/// crumbs for the current path and the current context depth (how many leading
/// segments belong to the nearest group/event, per
/// [`model::deepest_context_depth`]). The breadcrumbs, drawer and app rail all
/// read these so they agree on the context without each re-querying the path.
pub(super) static NAV_CRUMBS: GlobalSignal<Vec<model::Crumb>> = Signal::global(Vec::new);

/// Whether the path is currently being resolved into crumbs.
///
/// [`NAV_CRUMBS`] holds the PREVIOUS route's crumbs until the new ones arrive, so
/// a deeper path has indices with nothing behind them for a moment. Without
/// knowing that a resolution is in flight, the breadcrumbs cannot tell "not
/// loaded yet" from "does not resolve", and showed a question mark for both.
pub(super) static NAV_CRUMBS_LOADING: GlobalSignal<bool> = Signal::global(|| false);

pub(super) static CONTEXT_DEPTH: GlobalSignal<usize> = Signal::global(|| 0);

/// The signed-in user's pending invitations, for the Home nav badge and the
/// prompt shown inside a context they have not joined ([`InviteToJoin`]). Set by
/// [`Layout`] (once per session, refreshed on mutations via the data version).
///
/// The whole list rather than a count, because the prompt has to ask "is one of
/// these for the place being read", and the poll that fills this already paid
/// for the rows. Asking again per page would undo the reason this is polled at
/// all (see `INVITE_POLL_MS`).
pub static PENDING_INVITE_LIST: GlobalSignal<Vec<model::InvitationFields>> =
    Signal::global(Vec::new);

/// How many of the above there are, for the Home nav badge.
pub fn pending_invites() -> usize {
    PENDING_INVITE_LIST.read().len()
}

/// Invitations the reader has waved away this session, by member id, so moving
/// between pages of a context does not ask again on every step. Deliberately
/// not persisted: a new session is a fair time to mention it once more.
static INVITES_DISMISSED: GlobalSignal<std::collections::HashSet<String>> =
    Signal::global(std::collections::HashSet::new);

/// The reader's unaccepted invitation to `ctx`, unless they have already waved
/// it away this session. An invitation whose context did not come back (the
/// place is gone, or unreadable) matches nothing.
fn invite_for_context(
    invites: &[model::InvitationFields],
    ctx: &str,
    dismissed: &std::collections::HashSet<String>,
) -> Option<model::InvitationFields> {
    invites
        .iter()
        .find(|i| i.parent.as_ref().is_some_and(|p| p.id.0 == ctx) && !dismissed.contains(&i.id.0))
        .cloned()
}

/// Asks the reader to join the context they are reading, when they have an
/// invitation to it they have never accepted.
///
/// An invitation arrives as a link to a page, not to the invitation. Following
/// it lands you INSIDE the place, reading it, with the offer to join sitting on
/// a home screen you have no reason to visit, so people read a group for weeks
/// while still counting as invited. This carries the offer to wherever they
/// actually are.
///
/// It costs no query: the context comes from the page's own resolve
/// (`loader::CTX_ID`) and the invitations from the poll that already feeds the
/// nav badge. Only a match between the two opens anything.
#[component]
pub(super) fn InviteToJoin() -> Element {
    let session = use_session();
    let mut dismissed_open = use_signal(|| false);

    let Some(ctx) = crate::components::loader::CTX_ID() else {
        return rsx! {};
    };
    let invite = invite_for_context(&PENDING_INVITE_LIST.read(), &ctx, &INVITES_DISMISSED.read());
    let Some(invite) = invite else {
        return rsx! {};
    };
    if dismissed_open() {
        return rsx! {};
    }
    let member_id = invite.id.0.clone();
    let place = invite
        .parent
        .as_ref()
        .map_or_else(String::new, |p| p.name.clone());

    let accept = {
        let member_id = member_id.clone();
        move |_| {
            let token = session.read().access_token.clone();
            let Some(uid) = session.read().user.as_ref().map(|u| u.id.clone()) else {
                return;
            };
            let (member_id, ctx) = (member_id.clone(), ctx.clone());
            // Optimistic: the prompt goes now, and the poll confirms it.
            INVITES_DISMISSED.write().insert(member_id.clone());
            spawn(async move {
                // Already a bound member of this place (the invitation is a
                // leftover): accept THAT row and drop the invitation, which is
                // what the home list does for the same reason.
                let already =
                    graphql::accept_existing_member(token.as_deref(), &ctx, &uid, &member_id)
                        .await
                        .unwrap_or(false);
                let ok = if already {
                    let _ = graphql::decline_invitation(token.as_deref(), &member_id).await;
                    true
                } else {
                    graphql::accept_invitation(token.as_deref(), &member_id, &uid)
                        .await
                        .unwrap_or(false)
                };
                if ok {
                    PENDING_INVITE_LIST.write().retain(|i| i.id.0 != member_id);
                    crate::session::bump_data_version();
                } else {
                    INVITES_DISMISSED.write().remove(&member_id);
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            });
        }
    };

    // Dismissing is "not now", not "no": declining is a decision, and it
    // belongs where the invitation is listed rather than behind a scrim tap in
    // the middle of reading.
    let not_now = {
        let member_id = member_id.clone();
        move || {
            dismissed_open.set(true);
            INVITES_DISMISSED.write().insert(member_id.clone());
        }
    };

    rsx! {
        crate::components::widgets::Dialog {
            open: true,
            on_dismiss: {
                let mut not_now = not_now.clone();
                move |()| not_now()
            },
            headline: t_with("invite.joinHeadline", &[("name", &place)]),
            icon: Some("group_add".to_string()),
            actions: rsx! {
                button {
                    class: "btn btn-text",
                    onclick: {
                        let mut not_now = not_now.clone();
                        move |_| not_now()
                    },
                    "{t(\"invite.notNow\")}"
                }
                button { class: "btn btn-primary", onclick: accept, "{t(\"invite.join\")}" }
            },
            p { "{t_with(\"invite.joinBody\", &[(\"name\", &place)])}" }
        }
    }
}

/// Wraps an icon in the pending-invitation badge.
///
/// An invitation is a place you have been offered, so it lives in the drawer's
/// place picker (inline in the groups/events list it would join). The badge is
/// how it reaches you from anywhere: it marks each surface on the way in, the
/// menu button that opens the drawer and the bar that opens the picker, and
/// stops at whichever of them is already showing what it points at.
///
/// Nothing waiting means no badge at all, rather than a zero: the point is to be
/// noticed when it appears.
#[component]
pub(super) fn NavBadge(children: Element) -> Element {
    let pending = pending_invites();
    rsx! {
        span { class: "badged-icon",
            {children}
            if pending > 0 {
                crate::components::widgets::Badge { count: Some(pending) }
            }
        }
    }
}

/// Put the reader back where they were, once there is page enough to do it.
///
/// The content arrives after the route does, so the document is still short at
/// this moment and scrolling now would just land at the top. Poll until it is
/// tall enough, then jump; give up after two seconds. Any scrolling the reader
/// does themselves cancels it, so at worst this does nothing.
fn restore_scroll(target: f64) {
    const POLL_MS: u32 = 50;
    const ATTEMPTS: u32 = 40;
    spawn(async move {
        // Start at the top, the way an unremembered page would.
        crate::scroll_host::scroll_to(0.0);
        for _ in 0..ATTEMPTS {
            gloo_timers::future::TimeoutFuture::new(POLL_MS).await;
            // Anything but where we left it means the reader took over.
            if crate::scroll_host::scroll_top().abs() > 2.0 {
                return;
            }
            let reachable =
                crate::scroll_host::scroll_height() - crate::scroll_host::client_height();
            if reachable >= target {
                crate::scroll_host::scroll_to(target);
                // File it as well as perform it. Getting here means the throttled
                // trail still holds whatever tiny value was written on the way in
                // (this function starts at the top), and a reload before the
                // reader scrolls again would honour that instead of where they
                // just landed.
                if let Some(url) = crate::nav_memory::current_url() {
                    crate::nav_memory::note_scroll(&url, target);
                    crate::nav_memory::stash_scroll(&url, target);
                    // Then hold it: the jump above generates scroll events of
                    // its own, and the throttled writer would file an early
                    // frame of them over what was just filed.
                    crate::nav_memory::hold_trail(700.0);
                }
                return;
            }
        }
    });
}

/// The current context's key-path: the leading `CONTEXT_DEPTH` segments (or the
/// first segment as a fallback until the context resolves).
pub(super) fn context_path(segments: &[String]) -> Vec<String> {
    let depth = CONTEXT_DEPTH().max(1);
    segments.iter().take(depth).cloned().collect()
}

/// [`context_path`] without subscribing, for the navigation effect.
///
/// The depth is rewritten as each route's crumbs resolve, so an effect that
/// subscribed to it would run a second time per navigation and redo its scroll
/// handling. It is also still the OUTGOING route's depth when the effect fires,
/// which is exactly what recording the page being left wants; for the arriving
/// one it is right whenever both are in the same context, and a miss only costs
/// a fall back to the top of the page.
fn context_path_peek(segments: &[String]) -> Vec<String> {
    let depth = (*CONTEXT_DEPTH.peek()).max(1);
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
    let (cur_segments, cur_app) = match &route {
        Route::PathPage { segments, app } => (segments.clone(), app.clone()),
        Route::Home { app } => (Vec::new(), app.clone()),
        _ => (Vec::new(), None),
    };
    let path_key = cur_segments.join("/");
    let app_key = cur_app.clone().unwrap_or_default();
    // The route we are leaving: its path, its app, and the URL its scroll is
    // filed under.
    let mut previous = use_signal(|| Option::<(Vec<String>, Option<String>, String)>::None);
    use_effect(use_reactive!(|(path_key, app_key)| {
        let leaving = previous.peek().clone();
        if let Some((segments, app, url)) = &leaving {
            crate::nav_memory::remember(&context_path_peek(segments), app.as_deref(), segments);
            // Where the reader actually was, from the listener's per-event note,
            // NOT from the window. This effect runs after the new route has been
            // committed to the DOM, so if that view is shorter the browser has
            // already pulled the scroll down to the new maximum and the window
            // reports a zero the reader never chose. Reading it filed that zero
            // over the position of the page being left, which is what made
            // coming back to a context land at the top.
            let leaving_at =
                crate::nav_memory::last_scroll(url).unwrap_or_else(crate::scroll_host::scroll_top);
            crate::nav_memory::stash_scroll(url, leaving_at);
        }

        let segments: Vec<String> = if path_key.is_empty() {
            Vec::new()
        } else {
            path_key.split('/').map(str::to_string).collect()
        };
        let app = (!app_key.is_empty()).then(|| app_key.clone());
        let same_page = leaving
            .as_ref()
            .is_some_and(|(was, _, _)| was.join("/") == path_key);
        let url = crate::nav_memory::current_url().unwrap_or_default();
        previous.set(Some((segments.clone(), app.clone(), url.clone())));

        // Where this URL was left, if it was: the app rail bringing us back, the
        // browser's own back and forward, and a reload all key off the same
        // thing. Otherwise a different node starts at the top.
        //
        // A remembered position wins even when only `?app=` changed. That case
        // used to return early on the grounds that switching app in place leaves
        // the page under it alone and the reader keeps their scroll, which holds
        // for the scroll but not for the memory: the app view REPLACES the
        // content, so the browser clamps the scroll to the shorter document, and
        // coming back had nothing to undo that with. AT A CONTEXT ROOT this is
        // every rail tap, since the rail targets the context root and only the
        // query changes, which is why the root was the one place that never came
        // back. The early return survives for the case it was actually about:
        // nothing remembered, so leave the scroll where it is rather than
        // yanking a reader to the top for changing tab.
        match crate::nav_memory::stashed_scroll(&url) {
            Some(y) if y > 1.0 => restore_scroll(y),
            _ if same_page => {}
            _ => crate::scroll_host::scroll_to(0.0),
        }
    }));
    // Record each navigation as a diagnostics breadcrumb (remote-logging builds),
    // so an error's trail shows the route path the user moved through. Keyed on
    // the full URL string so an in-place app-view switch (?app=) is captured too.
    {
        let url = route.to_string();
        use_effect(use_reactive!(|(url,)| {
            crate::breadcrumbs::record_navigation(&url);
        }));
    }
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
        // Returns the crumbs rather than writing them, so the read cache has
        // something worth holding. Delivered by side effect they were invisible
        // to it, and every navigation showed the OUTGOING path's trail for a
        // round trip, even for a page visited a minute ago.
        let resolved = crate::use_data_resource!(|(segments, token)| async move {
            graphql::path_crumbs(token.as_deref(), &segments)
                .await
                .unwrap_or_default()
        });
        // Publish whatever is current for this path, cached or fresh.
        use_effect(move || {
            let crumbs = resolved.read().clone();
            // Nothing known for this path YET, which is what the bar reads as
            // "still resolving" rather than "does not resolve". A cached trail
            // counts as known: it is this path's, not the last one's.
            let waiting = crumbs.is_none();
            if *NAV_CRUMBS_LOADING.peek() != waiting {
                *NAV_CRUMBS_LOADING.write() = waiting;
            }
            let Some(crumbs) = crumbs else {
                return;
            };
            let depth = model::deepest_context_depth(&crumbs);
            if *CONTEXT_DEPTH.peek() != depth {
                *CONTEXT_DEPTH.write() = depth;
            }
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

    // Pending-invitation count for the nav badge.
    //
    // Still not session-stable: the badge is the ONLY way an invitation
    // announces itself, and one arriving while the tab sits open used to stay
    // invisible until a reload or an unrelated mutation.
    //
    // ON A TIMER, NOT A SUBSCRIPTION, and that is about the hall rather than
    // about this badge. Hasura batches live queries into cohorts by query AND
    // variables, so subscribers asking the same question share one evaluation
    // per poll. This one asks about the reader's OWN member rows, which puts
    // every reader in a cohort of one: at a congress of three hundred that is
    // three hundred evaluations per poll, where a shared question would be one.
    // Each of those runs the row-level permission check over every membership
    // the reader has, and one real reader with 206 of them measured at 8.1
    // SECONDS for a single evaluation before migration 0018.
    //
    // So the cost of the badge scaled with the number of people in the room
    // rather than with anything happening. A minute and a half late is not late
    // for an invitation, and a poll costs one query per reader per interval
    // instead of one evaluation per reader per second.
    const INVITE_POLL_MS: u32 = 90_000;
    {
        let uid = SESSION.read().user.as_ref().map(|u| u.id.clone());
        let email = SESSION.read().user.as_ref().map(|u| u.email.clone());
        let token = SESSION.read().access_token.clone();
        let mut invite_refresh = use_signal(|| 0u32);
        use_future(move || async move {
            loop {
                gloo_timers::future::TimeoutFuture::new(INVITE_POLL_MS).await;
                invite_refresh += 1;
            }
        });
        let rev = *invite_refresh.read();
        // Delivered by side effect on purpose, unlike the crumbs above: this
        // lives in the Layout, which never remounts, and its dependencies do
        // not change within a session, so returning it to be cached would buy
        // nothing.
        crate::use_data_resource!(|(uid, email, token, rev)| async move {
            let _ = rev;
            if let (Some(uid), Some(email)) = (uid, email) {
                let list = graphql::query_invitations(token.as_deref(), &uid, &email)
                    .await
                    .unwrap_or_default();
                if *PENDING_INVITE_LIST.peek() != list {
                    *PENDING_INVITE_LIST.write() = list;
                }
            } else if !PENDING_INVITE_LIST.peek().is_empty() {
                PENDING_INVITE_LIST.write().clear();
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

    // Where the reader is, in case they sign in from here. Recorded for every
    // page that is not an auth screen, which is the one place that covers all
    // the ways into them: the user menu, the buttons on an empty state, and the
    // app sending a signed-out reader there itself.
    if let Some(url) = crate::nav_memory::current_url() {
        crate::nav_memory::note_way_back(&url);
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

            // Feedback dialog (opened from the user menu). Also rendered at the
            // app-shell root: inside the drawer pane its fixed scrim would be
            // trapped by the pane's slide transform and overflow clip.
            super::feedback::FeedbackDialog {}

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
                // The whole dock (both tiers) hides on scroll-down and returns on
                // scroll-up, so it does not permanently eat two rows on a phone.
                div {
                    class: if super::back_to_top::dock_hidden() { "compact-dock dock-hidden" } else { "compact-dock" },
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
                    // A view that fails to render takes only itself down, not the
                    // chrome around it: the rail, the drawer and the breadcrumbs
                    // survive, so there is always a way out of a broken page.
                    // Without this the whole shell unmounts and the reader is left
                    // with a blank screen and no navigation.
                    ErrorBoundary {
                        handle_error: |error: ErrorContext| {
                            // The detail goes to the log (and to Better Stack in a
                            // remote-logging build); the reader gets the card, and
                            // one throttled toast in case they were scrolled away
                            // from where it broke.
                            log::error!("view failed to render: {error:?}");
                            crate::errors::report(crate::errors::Failure::Broken);
                            rsx! {
                                div { class: "card accent-error",
                                    crate::components::widgets::ErrorState {
                                        title: t("error.somethingWentWrong"),
                                    }
                                }
                            }
                        },
                        Outlet::<Route> {}
                    }
                    // Inside the shell, so it reaches every node page; it opens
                    // only where the reader has an unaccepted invitation.
                    InviteToJoin {}
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
    use super::{invite_for_context, parse_reset_token};
    use crate::model::{InvitationFields, ParentNodeFields, Uuid};
    use std::collections::HashSet;

    fn invite(member: &str, ctx: Option<&str>) -> InvitationFields {
        InvitationFields {
            id: Uuid(member.to_string()),
            parent: ctx.map(|c| ParentNodeFields {
                id: Uuid(c.to_string()),
                name: "Landsmøde".to_string(),
                key: "lm".to_string(),
                mime_id: Some("wiki/group".to_string()),
                data: None,
                author_avatar: None,
                parent: None,
            }),
        }
    }

    /// The prompt exists because an invitation link lands you inside the place,
    /// not on the invitation, so it must fire on the context being READ.
    #[test]
    fn an_invitation_is_offered_on_the_context_it_is_for() {
        let invites = [invite("m1", Some("ctx-a")), invite("m2", Some("ctx-b"))];
        let none = HashSet::new();

        let hit = invite_for_context(&invites, "ctx-b", &none).expect("matched");
        assert_eq!(hit.id.0, "m2", "the invitation for THIS place");
        assert!(
            invite_for_context(&invites, "ctx-elsewhere", &none).is_none(),
            "a place they were not invited to asks nothing"
        );
    }

    #[test]
    fn waving_it_away_stops_it_asking_again() {
        let invites = [invite("m1", Some("ctx-a"))];
        let dismissed: HashSet<String> = ["m1".to_string()].into_iter().collect();
        assert!(invite_for_context(&invites, "ctx-a", &dismissed).is_none());
        // Still offered to someone who has not dismissed it.
        assert!(invite_for_context(&invites, "ctx-a", &HashSet::new()).is_some());
    }

    /// A context that did not come back cannot be matched or named, so it must
    /// not open a dialog headlined after nothing.
    #[test]
    fn an_invitation_without_its_context_is_ignored() {
        let invites = [invite("m1", None)];
        assert!(invite_for_context(&invites, "ctx-a", &HashSet::new()).is_none());
    }

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
