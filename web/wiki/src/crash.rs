//! What the reader sees when the app dies.
//!
//! A panic in wasm aborts — there is no unwinding, so no `ErrorBoundary` can
//! catch it, and the Dioxus runtime is left in an undefined state. The page does
//! not close or reload; it simply stops responding, which looks identical to a
//! slow network. This puts a plain statement and a reload button over it.
//!
//! Everything here is raw DOM on purpose. The runtime that would render a
//! component has just aborted, so anything routed through Dioxus — including
//! reading a signal, which is why the text comes from [`crate::i18n::t_static`]
//! — risks panicking inside the panic hook and turning a reportable crash into
//! a bare wasm trap. It also lives outside the `remote-logging` feature, since a
//! dead page needs saying so whether or not anyone is collecting the report.

use wasm_bindgen::JsCast;

const OVERLAY_ID: &str = "wiki-crash-overlay";

/// Install the panic hook. One installer for every build, so the order cannot
/// drift between them: console first (so the message is there even if what
/// follows fails), then `report` — which is where `logging` ships the panic, and
/// nothing in builds without it — then the overlay last, once the crash has been
/// recorded.
pub fn install_hook(report: fn(&std::panic::PanicHookInfo<'_>)) {
    std::panic::set_hook(Box::new(move |info| {
        console_error_panic_hook::hook(info);
        report(info);
        show_overlay();
    }));
}

/// Cover the dead app with an explanation and a way out. Safe to call more than
/// once; only the first has any effect.
pub fn show_overlay() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    // A panic often arrives in a burst (a failed render, retried); the first is
    // the one worth showing, and re-inserting would discard a pending click.
    if doc.get_element_by_id(OVERLAY_ID).is_some() {
        return;
    }
    let Some(body) = doc.body() else { return };
    let Ok(overlay) = doc.create_element("div") else {
        return;
    };
    let _ = overlay.set_attribute("id", OVERLAY_ID);
    let _ = overlay.set_attribute("class", "crash-overlay");
    let _ = overlay.set_attribute("role", "alertdialog");
    let _ = overlay.set_attribute("aria-modal", "true");

    let Ok(card) = doc.create_element("div") else {
        return;
    };
    let _ = card.set_attribute("class", "crash-card");

    // Text through the DOM, never innerHTML. The strings are ours, but a crash
    // path is the last place to leave an injection seam.
    if let Ok(heading) = doc.create_element("h2") {
        heading.set_text_content(Some(&crate::i18n::t_static("error.crashTitle")));
        let _ = card.append_child(heading.as_ref());
    }
    if let Ok(text) = doc.create_element("p") {
        text.set_text_content(Some(&crate::i18n::t_static("error.crashBody")));
        let _ = card.append_child(text.as_ref());
    }
    if let Ok(button) = doc.create_element("button") {
        let _ = button.set_attribute("class", "btn btn-primary");
        button.set_text_content(Some(&crate::i18n::t_static("error.crashReload")));
        let reload = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(|| {
            if let Some(win) = web_sys::window() {
                let _ = win.location().reload();
            }
        });
        let _ = button.add_event_listener_with_callback("click", reload.as_ref().unchecked_ref());
        // Leaked deliberately: the page is going away on click, and there is no
        // longer a component whose lifetime this could belong to.
        reload.forget();
        let _ = card.append_child(button.as_ref());
    }

    let _ = overlay.append_child(card.as_ref());
    let _ = body.append_child(overlay.as_ref());
}
