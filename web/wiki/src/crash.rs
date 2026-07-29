//! What the reader sees when the app dies.
//!
//! A panic in wasm aborts — there is no unwinding, so no `ErrorBoundary` can
//! catch it, and the Dioxus runtime is left in an undefined state. The page does
//! not close or reload; it simply stops responding, which looks identical to a
//! slow network. This puts a plain statement and a reload button over it, and
//! files the crash as a `wiki/feedback` node of kind `crash` on the way.
//!
//! Reporting is not offered, it just happens. Asking a reader to press a button
//! meant the crashes worth hearing about were the ones least likely to be
//! reported, and there is nothing for them to decide: the report says what broke
//! and where, which is the same thing the page in front of them already said.
//! All the overlay owes them is whether it got through.
//!
//! Two rules shape everything here.
//!
//! **Raw DOM, never Dioxus.** The runtime that would render a component has just
//! aborted, so anything routed through it — including reading a signal, which is
//! why the text comes from [`crate::i18n::t_static`] — risks panicking inside the
//! panic hook and turning a reportable crash into a bare wasm trap.
//!
//! **The interactive parts are plain JavaScript, not Rust closures.** Once the
//! panic has trapped the wasm instance, calls back into it may not run at all —
//! so anything wired to a `Closure` could be dead on arrival, which is the one
//! thing this overlay must not be. Everything they need (the endpoint, the token,
//! the panic message) is baked in while wasm is still alive, and what runs
//! afterwards only touches `location`, `fetch` and the DOM.
//!
//! It also lives outside the `remote-logging` feature, since a dead page needs
//! saying so whether or not anyone is collecting the report.

use std::cell::RefCell;

const OVERLAY_ID: &str = "wiki-crash-overlay";
/// The line that says whether the report got through.
const STATUS_ID: &str = "wiki-crash-status";

/// How much of the panic message + stack the report carries. See
/// [`report_fetch_js`]: it goes in a URL, and symbolication expands it further
/// on the far side (the backend's own cap is 4000 chars, applied after).
const MAX_REPORT_CHARS: usize = 2000;

/// How long the same crash stays suppressed before it is worth reporting again.
/// Long enough that a reload loop files one node, short enough that a crash
/// someone is still hitting later says so.
const REPORT_AGAIN_MS: u64 = 30 * 60 * 1000;

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

/// The `fetch(…)` expression that files the crash report, with everything it
/// needs baked in while wasm is still alive.
///
/// The backend reads its fields from the query string, resolves the stack's wasm
/// offsets to source lines, and files the result as a `wiki/feedback` node — the
/// same node the in-app dialog creates, so the report lands in the feedback app
/// (`backend/src/feedback.rs`). It takes the caller from the bearer token when
/// there is one; a crash from a logged-out reader still reports, anonymously.
fn report_fetch_js(message: &str) -> String {
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

    // The whole report travels in the query string, and percent-encoding a stack
    // (newlines, colons, slashes) can triple its length — so cap the raw text
    // well under any server's URL limit, or the one report a reader chose to send
    // comes back 414. Cutting the TAIL is right: the panic's own message leads,
    // and the frames after it are ordered innermost first.
    let message: String = message.chars().take(MAX_REPORT_CHARS).collect();

    let url = format!(
        "{}/feedback?kind=crash&message={}&path={}&app={}&commit={}&ua={}",
        crate::backend_api::BACKEND_URL,
        js_sys::encode_uri_component(&message),
        js_sys::encode_uri_component(&path),
        js_sys::encode_uri_component(env!("CARGO_PKG_VERSION")),
        // Which build crashed. The offsets in the stack only mean anything
        // against the binary they came from, and the backend already keys its
        // symbol lookup on the bundle hash in the stack — this says the same in
        // terms a person can act on.
        js_sys::encode_uri_component(crate::build_info::COMMIT),
        js_sys::encode_uri_component(&ua),
    );

    // `keepalive` is the load-bearing option. This overlay puts a Reload button
    // in front of someone whose app has just died, and pressing it is the
    // obvious thing to do — but the report is a request in flight from a page
    // that is then torn down, and an ordinary fetch dies with it. The first
    // report for a build takes a couple of seconds (the backend fetches that
    // build's symbols), which is exactly long enough to lose the race. With
    // keepalive the browser owns the request and finishes it regardless.
    format!(
        "fetch({url},{{method:'POST',keepalive:true,headers:{headers}}})",
        url = js_string(&url),
        headers = if token.is_empty() {
            "{}".to_string()
        } else {
            format!("{{Authorization:'Bearer '+{}}}", js_string(&token))
        },
    )
}

/// A short, stable digest of the crash text, used to recognise the same crash
/// across reloads. FNV-1a: tiny, deterministic, and nothing here needs it to
/// resist anything.
fn digest(text: &str) -> String {
    let mut hash: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in text.as_bytes() {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x100_0000_01b3);
    }
    format!("{hash:016x}")
}

/// A JS function literal `function(ok){…}` that writes the outcome into the
/// overlay's status line and remembers a successful report under `key`.
///
/// By id rather than by closure, because it is a string evaluated after the wasm
/// instance has trapped.
fn settle_js(key: &str, sent_label: &str, failed_label: &str) -> String {
    format!(
        "function(ok){{\
           if(ok){{try{{sessionStorage.setItem({key},String(Date.now()));}}catch(e){{}}}}\
           var s=document.getElementById({status});if(s)s.textContent=ok?{sent}:{failed};\
         }}",
        key = js_string(key),
        status = js_string(STATUS_ID),
        sent = js_string(sent_label),
        failed = js_string(failed_label),
    )
}

/// Send the report as soon as the overlay is up, without waiting for a click.
///
/// The reader has already lost their work; making the report conditional on them
/// noticing a button meant the crashes worth hearing about were the ones least
/// likely to be sent. `fetch` is safe to start here even though the instance is
/// about to trap: the request now belongs to the browser, and the callback only
/// touches the DOM.
///
/// The same crash is not re-sent for [`REPORT_AGAIN_MS`]. A crash that survives a
/// reload — a bad record on the page being opened, say — would otherwise file a
/// node every time the reader tried again, burying the report under its own
/// copies.
///
/// The suppression EXPIRES, and it says so when it applies. It used to last the
/// whole tab and still display "Reported — thank you", so triggering the same
/// crash twice looked like reporting working once and then silently failing —
/// flaky, when it was doing exactly what it was told. A report is worth
/// refreshing eventually anyway: a crash still happening half an hour later is
/// news about the present, not a duplicate.
///
/// Only a success is remembered, so a failed send retries on the next load, and
/// storage being unavailable (private browsing) sends rather than skips: a
/// duplicate is the cheaper mistake.
fn auto_report_js(
    message: &str,
    sent_label: &str,
    failed_label: &str,
    already_label: &str,
) -> String {
    let key = format!("wiki-crash-{}", digest(message));
    format!(
        "(function(){{var d={settle};\
           try{{var t=Number(sessionStorage.getItem({key}));\
             if(t>0&&Date.now()-t<{window}){{\
               var s=document.getElementById({status});if(s)s.textContent={already};return;}}\
           }}catch(e){{}}\
           {fetch}.then(function(r){{d(r.ok);}}).catch(function(){{d(false);}});}})();",
        settle = settle_js(&key, sent_label, failed_label),
        key = js_string(&key),
        window = REPORT_AGAIN_MS,
        status = js_string(STATUS_ID),
        already = js_string(already_label),
        fetch = report_fetch_js(message),
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

    // Whether the automatic report got through. Empty until the fetch settles,
    // which takes a moment — the alternative is claiming success before there is
    // any.
    if let Ok(status) = doc.create_element("p") {
        let _ = status.set_attribute("id", STATUS_ID);
        let _ = status.set_attribute("class", "crash-status");
        let _ = status.set_attribute("aria-live", "polite");
        let _ = card.append_child(status.as_ref());
    }

    let message = PANIC_MESSAGE.with(|m| m.borrow().clone());
    let sent = crate::i18n::t_static("error.crashReported");
    let failed = crate::i18n::t_static("error.crashReportFailed");

    if let Ok(actions) = doc.create_element("div") {
        let _ = actions.set_attribute("class", "crash-actions");

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

    // File the report now the status line it writes into exists. Once per page:
    // the early return above means a burst of panics produces one overlay, and
    // so one report.
    let _ = js_sys::eval(&auto_report_js(
        &message,
        &sent,
        &failed,
        &crate::i18n::t_static("error.crashAlreadyReported"),
    ));
}
