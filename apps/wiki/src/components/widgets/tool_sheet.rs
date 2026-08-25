//! The M3 tools/actions sheet: a permanent docked side sheet on extra-large
//! screens, otherwise a modal sheet opened from a bottom-right FAB.

use dioxus::prelude::*;

use super::focus::{active_html_element, close_modal, trap_tab_focus};

/// Set while a [`ToolSheet`] is mounted in its docked (permanent) form, so the
/// shell can reserve room for it on the right of the content pane. Released when
/// the sheet unmounts (e.g. navigating to a view with no tools).
pub static TOOLS_DOCKED: GlobalSignal<bool> = Signal::global(|| false);

/// Bumped by each docked sheet as it mounts, so a release can tell "nothing has
/// tools any more" from "the next view's sheet has already taken over".
static DOCK_GEN: GlobalSignal<u64> = Signal::global(|| 0);

/// How long the shell keeps the right gutter after a docked sheet unmounts.
///
/// Moving between two nodes tears the old view down and only mounts the new
/// one's sheet once its data arrives. Releasing in that gap animates the whole
/// content pane and app bar out to the window edge and straight back, which
/// reads as the panel flickering; waiting through it keeps the pane still.
const DOCK_RELEASE_MS: u32 = 700;

/// A copy-link segment of the sheet's quick-action group. [`ToolSheet`] renders
/// this itself as the group's first segment, so copy-link is available on all
/// content and always sits at the top; call sites pass only their own segments.
/// Copies a shareable link to the current page, keeping Unicode (æøå) literal
/// (`decodeURI`) rather than percent-encoded — modern browsers handle it fine.
#[component]
pub fn CopyLinkAction() -> Element {
    let label = crate::i18n::t("common.copyLink");
    rsx! {
        button {
            class: "sheet-quick-action",
            r#type: "button",
            title: "{label}",
            aria_label: "{label}",
            onclick: move |_| {
                if let Some(win) = web_sys::window() {
                    if let Ok(href) = win.location().href() {
                        let link = js_sys::decode_uri(&href)
                            .ok()
                            .map(String::from)
                            .unwrap_or(href);
                        let _ = win.navigator().clipboard().write_text(&link);
                        crate::snackbar::show_snackbar(&crate::i18n::t("common.linkCopied"));
                    }
                }
            },
            span { class: "material-icons", "link" }
        }
    }
}

/// One labelled group of action rows inside a [`ToolSheet`].
///
/// A sheet that lists everything a node can do reaches ten identical rows on a
/// group (and is permanently on screen once the sheet docks on extra-large
/// windows). Grouping gives the eye a category to land on first: the rows are
/// unchanged, but they arrive in twos and threes under a quiet subheader.
/// `danger` sets the destructive group apart, below a rule and error-tinted.
///
/// Gate the group on the same condition as its rows: an empty group would still
/// draw its subheader.
#[component]
pub fn SheetGroup(
    #[props(default)] title: String,
    #[props(default)] danger: bool,
    children: Element,
) -> Element {
    rsx! {
        div {
            class: if danger { "sheet-group sheet-group-danger" } else { "sheet-group" },
            if !title.is_empty() {
                div { class: "sheet-group-title", "{title}" }
            }
            {children}
        }
    }
}

/// An export-to-ODT tools-sheet action with a built-in busy state. Generating the
/// ODT (fetching the whole content subtree plus embedded images) can take a
/// moment, so while it runs the action shows a spinner and disables, and a
/// "generating" snackbar appears — clear feedback that an export is in progress.
#[component]
pub fn ExportAction(node_id: String, name: String) -> Element {
    let session = crate::session::use_session();
    let mut exporting = use_signal(|| false);
    let label = crate::i18n::t("folder.download");
    rsx! {
        button {
            class: "sheet-quick-action",
            r#type: "button",
            title: "{label}",
            aria_label: "{label}",
            disabled: *exporting.read(),
            onclick: move |_| {
                if *exporting.read() {
                    return;
                }
                let token = session.read().access_token.clone();
                let who = session.read().identity();
                let id = node_id.clone();
                let name = name.clone();
                exporting.set(true);
                crate::snackbar::show_snackbar(&crate::i18n::t("folder.downloading"));
                spawn(async move {
                    crate::export::export_tree(token, id, name, who).await;
                    exporting.set(false);
                });
            },
            if *exporting.read() {
                div { class: "spinner spinner-xs" }
            } else {
                span { class: "material-icons", "download" }
            }
        }
    }
}

/// An M3 tools/actions sheet holding the current view's actions and admin tools.
/// Two modes only:
///
/// - Extra-large: a *permanent* docked side sheet — no trigger, always visible —
///   since there is room to stand it beside the content (M3 standard side sheet).
/// - Anything smaller: a modal sheet (bottom sheet on compact, right-anchored side
///   sheet on medium/large) opened from a bottom-right FAB — never an in-header
///   button on the content card.
///
/// Pass the action rows as `children`, grouped in [`SheetGroup`]s.
///
/// `quick` holds the always-available, icon-legible actions (export, share,
/// download) as segments of one M3 Expressive button group pinned at the top of
/// the sheet: three such rows cost three lines of a list that was already too
/// long, while as connected segments they cost one and anchor the sheet.
/// [`CopyLinkAction`] is the group's first segment, rendered by the sheet itself,
/// so copy-link leads every sheet rather than sitting wherever each call site
/// happened to place it.
#[component]
pub fn ToolSheet(
    title: String,
    #[props(default)] quick: Option<Element>,
    children: Element,
) -> Element {
    let mut open = use_signal(|| false);
    // Remember the trigger so focus returns to it when the sheet closes (a11y).
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);

    // Extra-large screens dock the tools as a permanent standing side sheet.
    let docked = use_memo(move || crate::window_size::WINDOW_SIZE().is_extra_large());
    // Reserve/release the shell's right gutter as this sheet docks or unmounts.
    use_effect(move || {
        if docked() {
            *DOCK_GEN.write() += 1;
        }
        *TOOLS_DOCKED.write() = docked();
    });
    use_drop(move || {
        // Detached: a task spawned on a scope being dropped dies with it, and
        // the release has to outlive this sheet to see whether another docks.
        let generation = DOCK_GEN();
        wasm_bindgen_futures::spawn_local(async move {
            gloo_timers::future::TimeoutFuture::new(DOCK_RELEASE_MS).await;
            if DOCK_GEN() == generation {
                *TOOLS_DOCKED.write() = false;
            }
        });
    });

    if docked() {
        return rsx! {
            aside {
                class: "tool-sheet docked",
                role: "complementary",
                "aria-label": "{title}",
                div { class: "tool-sheet-header",
                    span { class: "tool-sheet-icon material-icons", "bolt" }
                    h3 { class: "title-medium", "{title}" }
                }
                div { class: "tool-sheet-body",
                    div { class: "sheet-quick-group",
                        CopyLinkAction {}
                        if let Some(quick) = quick.clone() {
                            {quick}
                        }
                    }
                    {children}
                }
            }
        };
    }

    rsx! {
        // The trigger is always a bottom-right FAB (fixed) whenever the sheet is not
        // docked — the same focal affordance at every non-docked size, never an
        // in-header button on the content card.
        button {
            class: "fab",
            aria_label: "{title}",
            title: "{title}",
            onclick: move |_| {
                return_focus.set(active_html_element());
                open.set(true);
            },
            span { class: "material-icons", "bolt" }
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
                span { class: "tool-sheet-icon material-icons", "bolt" }
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
                div { class: "sheet-quick-group",
                    CopyLinkAction {}
                    if let Some(quick) = quick.clone() {
                        {quick}
                    }
                }
                {children}
            }
        }
    }
}
