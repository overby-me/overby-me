//! Shared modal-accessibility helpers used by the overlay components (Dialog,
//! ToolSheet, ZoomableImage) and the layout's AppSwitcher: capture/return focus
//! to the trigger, dismiss, and a Tab focus-trap. Domain-free.

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

/// The currently focused element — captured when a modal opens so focus can be
/// returned to the trigger on close (keyboard accessibility).
pub(crate) fn active_html_element() -> Option<web_sys::HtmlElement> {
    web_sys::window()?
        .document()?
        .active_element()?
        .dyn_into::<web_sys::HtmlElement>()
        .ok()
}

/// Close a modal surface (sheet/dialog) and return focus to whatever was focused
/// when it opened (`return_focus`).
pub(crate) fn close_modal(
    mut open: Signal<bool>,
    return_focus: Signal<Option<web_sys::HtmlElement>>,
) {
    open.set(false);
    if let Some(el) = return_focus.read().clone() {
        let _ = el.focus();
    }
}

/// Dismiss a controlled dialog (via its `on_dismiss` handler) and return focus to
/// whatever was focused when it opened.
pub(crate) fn dialog_dismiss(
    on_dismiss: EventHandler<()>,
    return_focus: Signal<Option<web_sys::HtmlElement>>,
) {
    on_dismiss.call(());
    if let Some(el) = return_focus.read().clone() {
        let _ = el.focus();
    }
}

/// Trap Tab focus within the open modal matched by `container_sel`. Called from a
/// Tab keydown handler; returns true when it wrapped focus (the caller should then
/// `prevent_default`), keeping keyboard focus inside the modal.
pub(crate) fn trap_tab_focus(container_sel: &str, shift: bool) -> bool {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return false;
    };
    let Some(container) = doc.query_selector(container_sel).ok().flatten() else {
        return false;
    };
    let Ok(nodes) = container.query_selector_all(
        "a[href], button:not([disabled]), input:not([disabled]), textarea:not([disabled]), select:not([disabled]), [tabindex]",
    ) else {
        return false;
    };
    let mut items: Vec<web_sys::HtmlElement> = Vec::new();
    for i in 0..nodes.length() {
        let Some(el) = nodes
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::HtmlElement>().ok())
        else {
            continue;
        };
        // Skip the off-screen focus sentinel and hidden controls.
        if el.get_attribute("tabindex").as_deref() == Some("-1") || el.offset_parent().is_none() {
            continue;
        }
        items.push(el);
    }
    let (Some(first), Some(last)) = (items.first(), items.last()) else {
        return false;
    };
    let active_html = doc
        .active_element()
        .and_then(|a| a.dyn_into::<web_sys::HtmlElement>().ok());
    let no_active = active_html.is_none();
    let matches = |t: &web_sys::HtmlElement| active_html.as_ref() == Some(t);
    if shift {
        if matches(first) || no_active {
            let _ = last.focus();
            return true;
        }
    } else if matches(last) || no_active {
        let _ = first.focus();
        return true;
    }
    false
}
