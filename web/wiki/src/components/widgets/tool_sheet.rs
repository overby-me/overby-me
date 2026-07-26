//! The M3 tools/actions sheet: a permanent docked side sheet on extra-large
//! screens, otherwise a modal sheet opened from a bottom-right FAB.

use dioxus::prelude::*;

use super::focus::{active_html_element, close_modal, trap_tab_focus};

/// Set while a [`ToolSheet`] is mounted in its docked (permanent) form, so the
/// shell can reserve room for it on the right of the content pane. Released when
/// the sheet unmounts (e.g. navigating to a view with no tools).
pub static TOOLS_DOCKED: GlobalSignal<bool> = Signal::global(|| false);

/// A copy-link tools-sheet action, shared by every content view's [`ToolSheet`]
/// so copy-link is available on all content. Copies a shareable link to the
/// current page, keeping Unicode (æøå) literal (`decodeURI`) rather than
/// percent-encoded — modern browsers handle it fine.
#[component]
pub fn CopyLinkAction() -> Element {
    rsx! {
        button {
            class: "sheet-action",
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
            "{crate::i18n::t(\"common.copyLink\")}"
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
    rsx! {
        button {
            class: "sheet-action",
            disabled: *exporting.read(),
            onclick: move |_| {
                if *exporting.read() {
                    return;
                }
                let token = session.read().access_token.clone();
                let id = node_id.clone();
                let name = name.clone();
                exporting.set(true);
                crate::snackbar::show_snackbar(&crate::i18n::t("folder.exporting"));
                spawn(async move {
                    crate::export::export_tree(token, id, name).await;
                    exporting.set(false);
                });
            },
            if *exporting.read() {
                div { class: "spinner spinner-xs" }
            } else {
                span { class: "material-icons", "download" }
            }
            "{crate::i18n::t(\"folder.export\")}"
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
/// Pass the action rows (`.sheet-action`) as `children`.
#[component]
pub fn ToolSheet(title: String, children: Element) -> Element {
    let mut open = use_signal(|| false);
    // Remember the trigger so focus returns to it when the sheet closes (a11y).
    let mut return_focus = use_signal(|| None::<web_sys::HtmlElement>);

    // Extra-large screens dock the tools as a permanent standing side sheet.
    let docked = use_memo(move || crate::window_size::WINDOW_SIZE().is_extra_large());
    // Reserve/release the shell's right gutter as this sheet docks or unmounts.
    use_effect(move || {
        *TOOLS_DOCKED.write() = docked();
    });
    use_drop(move || {
        *TOOLS_DOCKED.write() = false;
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
                div { class: "tool-sheet-body", {children} }
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
                {children}
            }
        }
    }
}
