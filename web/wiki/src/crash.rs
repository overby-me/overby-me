//! What the reader sees when the app dies.
//!
//! A panic in wasm aborts — there is no unwinding, so no `ErrorBoundary` can
//! catch it, and the Dioxus runtime is left in an undefined state. The page does
//! not close or reload; it simply stops responding, which looks identical to a
//! slow network. This puts a plain statement, a reload and a report button over
//! it.
//!
//! Two rules shape everything here.
//!
//! **Raw DOM, never Dioxus.** The runtime that would render a component has just
//! aborted, so anything routed through it — including reading a signal, which is
//! why the text comes from [`crate::i18n::t_static`] — risks panicking inside the
//! panic hook and turning a reportable crash into a bare wasm trap.
//!
//! **The buttons are plain JavaScript, not Rust closures.** Once the panic has
//! trapped the wasm instance, calls back into it may not run at all — so a button
//! wired to a `Closure` could be dead on arrival, which is the one thing this
//! overlay must not be. Everything the buttons need (the endpoint, the token, the
//! panic message) is baked into their handlers while wasm is still alive, and the
//! handlers themselves only touch `location` and `fetch`.
//!
//! It also lives outside the `remote-logging` feature, since a dead page needs
//! saying so whether or not anyone is collecting the report.

use std::cell::RefCell;

const OVERLAY_ID: &str = "wiki-crash-overlay";

thread_local! {
    /// What panicked, captured in the hook so the report can carry it. Read when
    /// building the overlay, which happens in the same call.
    static PANIC_MESSAGE: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Install the panic hook. One installer for every build, so the order cannot
/// drift between them: console first (so the message is there even if what
/// follows fails), then `report` — which is where `logging` ships the panic, and
/// nothing in builds without it — then the overlay last, once the crash has been
/// recorded.
pub fn install_hook(report: fn(&std::panic::PanicHookInfo<'_>)) {
    std::panic::set_hook(Box::new(move |info| {
        console_error_panic_hook::hook(info);
        report(info);
        // Message AND stack. The panic's own location is in the message, but for
        // the crash that prompted all this it was `dioxus-core/src/diff/…` — a
        // dependency. Which of OUR components was rendering is only in the stack,
        // and the backend resolves its wasm offsets to source lines on the way to
        // Better Stack (see backend/src/symbolicate.rs).
        PANIC_MESSAGE.with(|m| {
            *m.borrow_mut() = match js_stack() {
                Some(stack) => format!("{info}\n{stack}"),
                None => info.to_string(),
            }
        });
        show_overlay();
    }));
}

/// The JS call stack at this moment, from a throwaway `Error`. Its wasm frames
/// carry the module URL and a code offset, which is what the backend needs to
/// resolve them to source lines.
fn js_stack() -> Option<String> {
    let err = js_sys::Error::new("");
    js_sys::Reflect::get(&err, &wasm_bindgen::JsValue::from_str("stack"))
        .ok()
        .and_then(|v| v.as_string())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// A JS string literal for `text`, quoted and escaped. Via `serde_json` because
/// it is exactly JSON string syntax, and a panic message is arbitrary text that
/// will contain quotes and newlines.
fn js_string(text: &str) -> String {
    serde_json::Value::String(text.to_string()).to_string()
}

/// The `onclick` handler for the report button: send the crash to the feedback
/// endpoint, then say so in the button itself.
///
/// The backend reads its fields from the query string (`backend/src/feedback.rs`),
/// and takes the caller from the bearer token when there is one — a crash from a
/// logged-out reader still reports, anonymously.
fn report_handler_js(message: &str, sent_label: &str, failed_label: &str) -> String {
    let token = web_sys::window()
        .and_then(|w| w.local_storage().ok().flatten())
        .and_then(|s| s.get_item("wiki_session").ok().flatten())
        .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
        .and_then(|v| {
            v.get("access_token")
                .and_then(|t| t.as_str())
                .map(str::to_string)
        })
        .unwrap_or_default();

    let path = web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_default();
    let ua = web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .unwrap_or_default();

    let url = format!(
        "{}/feedback?kind=bug&message={}&path={}&app={}&ua={}",
        crate::backend_api::BACKEND_URL,
        js_sys::encode_uri_component(&format!("CRASH: {message}")),
        js_sys::encode_uri_component(&path),
        js_sys::encode_uri_component(env!("CARGO_PKG_VERSION")),
        js_sys::encode_uri_component(&ua),
    );

    // `this` is the button. Disable it first so a second tap cannot double-send,
    // and report the outcome in place rather than with an alert.
    format!(
        "var b=this;b.disabled=true;fetch({url},{{method:'POST',headers:{headers}}})\
         .then(function(r){{b.textContent=r.ok?{sent}:{failed};}})\
         .catch(function(){{b.textContent={failed};}});",
        url = js_string(&url),
        headers = if token.is_empty() {
            "{}".to_string()
        } else {
            format!("{{Authorization:'Bearer '+{}}}", js_string(&token))
        },
        sent = js_string(sent_label),
        failed = js_string(failed_label),
    )
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

    if let Ok(actions) = doc.create_element("div") {
        let _ = actions.set_attribute("class", "crash-actions");

        if let Ok(report) = doc.create_element("button") {
            let _ = report.set_attribute("class", "btn btn-outlined");
            report.set_text_content(Some(&crate::i18n::t_static("error.crashReport")));
            let message = PANIC_MESSAGE.with(|m| m.borrow().clone());
            let _ = report.set_attribute(
                "onclick",
                &report_handler_js(
                    &message,
                    &crate::i18n::t_static("error.crashReported"),
                    &crate::i18n::t_static("error.crashReportFailed"),
                ),
            );
            let _ = actions.append_child(report.as_ref());
        }

        if let Ok(reload) = doc.create_element("button") {
            let _ = reload.set_attribute("class", "btn btn-primary");
            reload.set_text_content(Some(&crate::i18n::t_static("error.crashReload")));
            let _ = reload.set_attribute("onclick", "location.reload()");
            let _ = actions.append_child(reload.as_ref());
        }

        let _ = card.append_child(actions.as_ref());
    }

    let _ = overlay.append_child(card.as_ref());
    let _ = body.append_child(overlay.as_ref());
}
