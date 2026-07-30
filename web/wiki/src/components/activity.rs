//! The activity surface: what is new for the signed-in user (pending
//! invitations, then everything recently posted they may see), shown as an
//! overlay over wherever they already are.
//!
//! Checking what happened is not a place, so it is not a page. A delegate
//! reading a vote can open this, accept an invitation, close it and still be on
//! the same vote at the same scroll position. That is why it is ephemeral UI
//! state (like the search field) rather than a route.

use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

/// Whether the activity sheet is open. Global because the trigger lives in the
/// top app bar while the sheet itself is rendered at the app-shell root, so its
/// scrim escapes the bar's stacking context.
pub static ACTIVITY_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// Close the sheet and put focus back where it came from. The trigger lives in
/// the drawer, which on compact has itself closed by the time the sheet opens,
/// so focus falls back to the menu button that leads there. Found by class
/// rather than stashed in a signal, since both are single and always present.
/// Public so a feed row can dismiss the sheet it was followed from.
pub fn close_activity() {
    // Called by feed rows, which also live on the feed page: with nothing open
    // there is nothing to close, and the focus move below would be a jump for no
    // reason.
    if !ACTIVITY_OPEN() {
        return;
    }
    *ACTIVITY_OPEN.write() = false;
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    for sel in [".activity-trigger", ".menu-trigger"] {
        if let Some(el) = doc
            .query_selector(sel)
            .ok()
            .flatten()
            .and_then(|e| wasm_bindgen::JsCast::dyn_into::<web_sys::HtmlElement>(e).ok())
        {
            let _ = el.focus();
            return;
        }
    }
}

/// The drawer's activity row, pinned above the account menu.
///
/// It sits with the drawer's other standing utilities rather than in the top app
/// bar: the bar is at its most cramped on a phone, where it also carries the
/// breadcrumb trail. What has to be visible everywhere is not this row but the
/// count on it, and that travels to whichever menu button opens the drawer (see
/// [`NavBadge`]), so a waiting invitation still announces itself from every page.
#[component]
pub fn ActivityRow() -> Element {
    let session = use_session();
    if !session.read().is_authenticated() {
        return rsx! {};
    }
    let pending = crate::components::layout::PENDING_INVITES();
    let label = t("common.activity");
    rsx! {
        div { class: "drawer-utility",
            button {
                class: "drawer-account-trigger activity-trigger state-layer",
                aria_label: "{label}",
                title: "{label}",
                "aria-haspopup": "dialog",
                "aria-expanded": if ACTIVITY_OPEN() { "true" } else { "false" },
                // Deliberately NOT stop_propagation, unlike the account trigger
                // below: on compact this click should dismiss the drawer, because
                // the sheet it opens is what takes its place.
                onclick: move |_| {
                    let now = !ACTIVITY_OPEN();
                    if now {
                        *ACTIVITY_OPEN.write() = true;
                    } else {
                        close_activity();
                    }
                },
                span { class: "avatar small badged-icon",
                    span { class: "material-icons", "notifications" }
                    if pending > 0 {
                        crate::components::widgets::Badge { count: Some(pending) }
                    }
                }
                span { class: "drawer-account-name", "{label}" }
            }
        }
    }
}

/// The pending-invitation count, for the menu button that opens the drawer the
/// activity row lives in. Wraps the caller's icon, so the badge overlays it.
///
/// Nothing to answer means no badge at all, rather than a zero: the point is to
/// be noticed when it appears.
#[component]
pub fn NavBadge(children: Element) -> Element {
    let pending = crate::components::layout::PENDING_INVITES();
    rsx! {
        span { class: "badged-icon",
            {children}
            if pending > 0 {
                crate::components::widgets::Badge { count: Some(pending) }
            }
        }
    }
}

/// The activity sheet itself: a bottom sheet on compact, a right side sheet on
/// medium+ (the app's standard sheet geometry). Rendered by `Layout` at the
/// app-shell root.
#[component]
pub fn ActivitySheet() -> Element {
    let session = use_session();
    if !session.read().is_authenticated() {
        return rsx! {};
    }
    let open = ACTIVITY_OPEN();

    rsx! {
        div {
            class: if open { "sheet-scrim open" } else { "sheet-scrim" },
            role: "presentation",
            onclick: move |_| close_activity(),
        }
        aside {
            class: if open { "tool-sheet activity-sheet open" } else { "tool-sheet activity-sheet" },
            role: "dialog",
            "aria-modal": "true",
            "aria-label": t("common.activity"),
            tabindex: "-1",
            onkeydown: move |e| {
                match e.key() {
                    Key::Escape => close_activity(),
                    Key::Tab
                        if crate::components::widgets::trap_tab_focus(
                            ".activity-sheet.open",
                            e.modifiers().shift(),
                        ) =>
                    {
                        e.prevent_default();
                    }
                    _ => {}
                }
            },
            // Focus sentinel: pulls focus into the sheet when it opens, so Escape
            // and Tab work without a click first.
            if open {
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
                span { class: "tool-sheet-icon material-icons", "notifications" }
                h3 { class: "title-medium", "{t(\"common.activity\")}" }
                button {
                    class: "btn-icon state-layer",
                    aria_label: t("common.close"),
                    onclick: move |_| close_activity(),
                    span { class: "material-icons", "close" }
                }
            }
            div { class: "tool-sheet-body activity-body",
                // Mounted only while open: the feed is a real query, and a closed
                // sheet has no business running it. Reopening therefore also
                // refreshes, which is what "what is new" should do.
                if open {
                    PendingInvitations {}
                    div { class: "activity-section",
                        p { class: "title-small list-subheader", "{t(\"layout.feed\")}" }
                        // A peek, not a feed: the newest few across everything the
                        // reader may see, enough to answer "did I miss anything".
                        // Reading properly happens in a full column, on the home
                        // page (everywhere) or a context's own `?app=feed`.
                        crate::components::feed::FeedList { peek: true }
                        Link {
                            to: Route::Home { app: None },
                            class: "btn btn-text",
                            onclick: move |_| close_activity(),
                            "{t(\"layout.allActivity\")}"
                        }
                    }
                }
            }
        }
    }
}

/// Invitations waiting for an answer, at the top of the sheet because they are
/// the only part of it that asks the reader to do something. Accepting or
/// declining happens in place (the same row the groups/events lists use).
#[component]
fn PendingInvitations() -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let email = session
        .read()
        .user
        .as_ref()
        .map(|u| u.email.clone())
        .unwrap_or_default();

    let invites = crate::use_data_resource!(|(token, user_id, email)| async move {
        let Some(uid) = user_id else {
            return Vec::new();
        };
        graphql::query_invitations(token.as_deref(), &uid, &email)
            .await
            .unwrap_or_default()
    });

    let list = invites.read().clone().unwrap_or_default();
    if list.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "activity-section",
            p { class: "title-small list-subheader", "{t(\"invite.invitations\")}" }
            div { class: "list",
                for invite in list.iter() {
                    crate::components::layout::InvitedContextItem {
                        key: "{invite.id.0}",
                        invite: invite.clone(),
                    }
                }
            }
        }
    }
}
