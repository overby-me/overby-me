//! Remote logging to Better Stack (Logtail).
//!
//! Mirrors console output and ships `warn`/`error` records (plus a rolling
//! breadcrumb trail of DOM interactions and any wasm panic) as structured JSON,
//! so the logs can be filtered and grouped by user/session in the Logtail
//! dashboard. Gated behind the `remote-logging` cargo feature; the source token
//! is read from the `BETTERSTACK_SOURCE_TOKEN` env var at BUILD time, so nothing
//! secret is committed. With no token it is inert (console only), and the token
//! it does embed is a write-only ingestion token (it cannot read your logs).
//!
//! Enable with, e.g.:
//! ```sh
//! BETTERSTACK_SOURCE_TOKEN=xxxxx BETTERSTACK_INGEST_HOST=sN.betterstackdata.com \
//!   dx build --release --features remote-logging
//! ```

use std::cell::RefCell;

use log::{Level, LevelFilter, Log, Metadata, Record};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

/// Set at build (any value) when this build should ship logs; `None` leaves
/// shipping off (console only). Presence, not the value, is the switch — the
/// actual ingest token lives on the backend, which does the Better Stack call
/// (see `backend/src/logs.rs`), so no secret is baked into the wasm bundle.
const SOURCE_TOKEN: Option<&str> = option_env!("BETTERSTACK_SOURCE_TOKEN");
const FLUSH_INTERVAL_MS: u32 = 5000;

thread_local! {
    /// Log entries waiting to be shipped on the next flush.
    static PENDING: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    /// The rolling trail of recent DOM interactions.
    /// A random id identifying this browser tab/session.
    static SESSION_ID: RefCell<String> = const { RefCell::new(String::new()) };
    /// Which build the service worker serving this page came from, once it has
    /// been asked. See [`ask_the_worker_which_build`].
    static SW_BUILD: RefCell<Option<String>> = const { RefCell::new(None) };
}

/// The backend log-ingest proxy (it forwards to Better Stack server-side). Going
/// through our own origin keeps the ship off Better Stack's CORS (whose
/// `Allow-Headers: *` excludes `Authorization`) and the token out of the client.
fn ingest_url() -> String {
    format!("{}/log", crate::backend_api::BACKEND_URL)
}

/// The wall-clock now as an RFC 3339 string (Logtail's `dt`).
fn now_iso() -> String {
    js_sys::Date::new_0()
        .to_iso_string()
        .as_string()
        .unwrap_or_default()
}

fn current_path() -> String {
    web_sys::window()
        .and_then(|w| w.location().pathname().ok())
        .unwrap_or_default()
}

/// The current user's `(id, name)`, read from the persisted session in
/// localStorage so the logger stays decoupled from the Dioxus runtime (it runs
/// from log macros and timers that are not always inside a render).
fn current_user() -> (Option<String>, Option<String>) {
    let read = || -> Option<(Option<String>, Option<String>)> {
        let raw = web_sys::window()?
            .local_storage()
            .ok()??
            .get_item("wiki_session")
            .ok()??;
        let v: Value = serde_json::from_str(&raw).ok()?;
        let user = v.get("user")?;
        let id = user.get("id").and_then(|x| x.as_str()).map(String::from);
        let name = user
            .get("display_name")
            .and_then(|x| x.as_str())
            .map(String::from);
        Some((id, name))
    };
    read().unwrap_or((None, None))
}

/// The JS call stack at the point of logging (from a throwaway `Error`), so an
/// error entry pinpoints where it originated. `None` if unavailable.
fn current_stack() -> Option<String> {
    let err = js_sys::Error::new("");
    js_sys::Reflect::get(&err, &JsValue::from_str("stack"))
        .ok()
        .and_then(|v| v.as_string())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// The `stack` property of a thrown JavaScript value, if it has one.
///
/// For an uncaught error this is the ONLY true stack: [`current_stack`] runs in
/// the event handler and describes the handler, not the throw.
fn stack_of(value: &JsValue) -> Option<String> {
    js_sys::Reflect::get(value, &JsValue::from_str("stack"))
        .ok()
        .and_then(|v| v.as_string())
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
}

/// `name: message` for a thrown Error, which JSON cannot serialise.
///
/// `JSON.stringify(new Error("boom"))` is `{}` — `name`, `message` and `stack`
/// are non-enumerable — so a rejected Error logged as "UNHANDLED REJECTION:"
/// with nothing after the colon. Read the properties instead.
fn error_text(value: &JsValue) -> Option<String> {
    let prop = |key: &str| {
        js_sys::Reflect::get(value, &JsValue::from_str(key))
            .ok()
            .and_then(|v| v.as_string())
            .filter(|s| !s.is_empty())
    };
    join_error_text(prop("name"), prop("message"))
}

/// The `name`/`message` pair as one line, however much of it exists.
fn join_error_text(name: Option<String>, message: Option<String>) -> Option<String> {
    match (name, message) {
        (Some(name), Some(message)) => Some(format!("{name}: {message}")),
        (None, Some(message)) => Some(message),
        (Some(name), None) => Some(name),
        (None, None) => None,
    }
}

/// A stack as one frame per element rather than one string full of `\n`.
///
/// Logtail shows a JSON array as a list; a newline-joined string arrives as a
/// single unreadable line, which is what these reports looked like.
fn stack_frames(stack: Option<String>) -> Value {
    match stack {
        Some(s) => Value::Array(
            s.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(|l| Value::String(l.to_string()))
                .collect(),
        ),
        None => Value::Null,
    }
}

/// One structured log entry with the standard enrichment, so Logtail can filter
/// and group by any field:
/// - who: `user_id` / `user_name`, plus a per-tab `session_id`
/// - where: the current URL `path`, the app `app_version` + `commit`, and a JS
///   `stack`
/// - what they did: `breadcrumbs` (the recent navigation + click/change/submit
///   trail, newest last)
fn make_entry(level: &str, message: String) -> Value {
    make_entry_with_stack(level, message, current_stack())
}

/// [`make_entry`], but with the stack supplied rather than sampled here.
///
/// An uncaught error must pass the stack of the THROW. Sampling one in the
/// handler produces a trace of the logger, which is worse than none: it names
/// real functions that had nothing to do with the failure.
fn make_entry_with_stack(level: &str, message: String, stack: Option<String>) -> Value {
    let (user_id, user_name) = current_user();
    let session_id = SESSION_ID.with(|s| s.borrow().clone());
    json!({
        "dt": now_iso(),
        "level": level,
        "message": message,
        "user_id": user_id,
        "user_name": user_name,
        "session_id": session_id,
        "path": current_path(),
        "app_version": env!("CARGO_PKG_VERSION"),
        // Which build, so a stack can be read against the code that produced it
        // — and so an error from a bundle nobody runs any more is recognisable.
        "commit": crate::build_info::COMMIT,
        "stack": stack_frames(stack),
        "user_agent": web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok()),
        "breadcrumbs": crate::breadcrumbs::trail(),
        // The state of the thing it happened IN. Every one of these has been
        // the answer to a real report: the window's size decides where a page
        // control thinks the reader is, whether a service worker is serving the
        // page decides whether the build running is the build deployed, and the
        // connection is the first question anyone asks about a failure in a hall
        // of six hundred people on one access point.
        "viewport": viewport(),
        "dpr": web_sys::window().map(|w| w.device_pixel_ratio()),
        "installed": display_mode_standalone(),
        "sw_controlled": service_worker_controlling(),
        // And WHICH build that worker is. A worker outlives the page that
        // installed it, so a visitor can be running one deploy in the tab while
        // an older one serves the files -- which is what "I still see the old
        // version" is, and why a stack can disagree with the code. `null` with
        // `sw_controlled` true is a worker from before this was asked of them.
        "sw_build": SW_BUILD.with(|b| b.borrow().clone()),
        "online": web_sys::window().map(|w| w.navigator().on_line()),
        "connection": connection_kind(),
        "language": web_sys::window().and_then(|w| w.navigator().language()),
        "storage": storage_works(),
        // How long the tab had been open. A fault at boot and a fault after an
        // hour of use are different faults, and the timestamp alone cannot tell
        // them apart.
        "up_ms": web_sys::window()
            .and_then(|w| w.performance())
            .map(|p| p.now().round()),
    })
}

/// The window, as `1400x1000`. Not the screen: the window is what the layout is
/// laid out in.
fn viewport() -> Option<String> {
    let w = web_sys::window()?;
    let width = w.inner_width().ok()?.as_f64()?;
    let height = w.inner_height().ok()?.as_f64()?;
    Some(format!("{}x{}", width.round(), height.round()))
}

/// Whether the app is running as an installed app rather than in a browser tab.
fn display_mode_standalone() -> Option<bool> {
    use wasm_bindgen::JsCast;
    let w = web_sys::window()?;
    // Through Reflect: `matchMedia` is not in the web-sys Window binding this
    // build uses, and asking JavaScript directly is cheaper than a feature.
    let ask = js_sys::Reflect::get(&w, &"matchMedia".into())
        .ok()?
        .dyn_into::<js_sys::Function>()
        .ok()?;
    let list = ask.call1(&w, &"(display-mode: standalone)".into()).ok()?;
    Some(
        js_sys::Reflect::get(&list, &"matches".into())
            .ok()
            .and_then(|m| m.as_bool())
            .unwrap_or(false),
    )
}

/// Whether a service worker is serving this page -- which decides whether the
/// code running is the code deployed, and whether there is one at all.
fn service_worker_controlling() -> Option<bool> {
    if !crate::pwa::service_worker_available() {
        return Some(false);
    }
    let w = web_sys::window()?;
    let container = js_sys::Reflect::get(w.navigator().as_ref(), &"serviceWorker".into()).ok()?;
    let controller = js_sys::Reflect::get(&container, &"controller".into()).ok()?;
    Some(!controller.is_null() && !controller.is_undefined())
}

/// What the browser calls this connection (`4g`, `slow-2g`), where it says.
fn connection_kind() -> Option<String> {
    let w = web_sys::window()?;
    let connection = js_sys::Reflect::get(w.navigator().as_ref(), &"connection".into()).ok()?;
    js_sys::Reflect::get(&connection, &"effectiveType".into())
        .ok()?
        .as_string()
}

/// Whether this browser will let the app store anything. Private-mode Safari
/// THROWS on `localStorage`, which is how a session silently fails to persist.
fn storage_works() -> Option<bool> {
    let w = web_sys::window()?;
    Some(matches!(w.local_storage(), Ok(Some(_))))
}

/// Ask the service worker serving this page which build it is, and remember the
/// answer for every entry after it.
///
/// A worker outlives the page that installed it, so a visitor can be running one
/// deploy in the tab while an older one serves the files. There is no other way
/// to ask: the worker's build is in the worker's own code, and `version.json` is
/// deliberately never cached (see sw.js).
///
/// Nothing waits for the reply. The listener goes on first and the question
/// after it, so an answer lands when it lands -- and a worker from a deploy
/// before this question existed simply never sends one, which is why the field
/// can be absent while `sw_controlled` is true.
fn ask_the_worker_which_build() {
    use wasm_bindgen::{closure::Closure, JsCast};

    let Some(window) = web_sys::window() else {
        return;
    };
    if !crate::pwa::service_worker_available() {
        return;
    }
    let Ok(container) = js_sys::Reflect::get(window.navigator().as_ref(), &"serviceWorker".into())
    else {
        return;
    };
    let Ok(target) = container.clone().dyn_into::<web_sys::EventTarget>() else {
        return;
    };

    let heard =
        Closure::<dyn FnMut(web_sys::MessageEvent)>::new(move |ev: web_sys::MessageEvent| {
            let build = js_sys::Reflect::get(&ev.data(), &"build".into())
                .ok()
                .and_then(|v| v.as_string())
                .filter(|b| !b.is_empty() && b != "__WIKI_BUILD__");
            if build.is_some() {
                SW_BUILD.with(|b| *b.borrow_mut() = build);
            }
        });
    let _ = target.add_event_listener_with_callback("message", heard.as_ref().unchecked_ref());
    heard.forget();
    // And START the delivery. A container queues its messages until either an
    // `onmessage` property is assigned or this is called; a listener added with
    // `addEventListener` does NOT start it, and the answer would sit in the
    // queue unread.
    if let Ok(start) = js_sys::Reflect::get(&container, &"startMessages".into()) {
        if let Ok(start) = start.dyn_into::<js_sys::Function>() {
            let _ = start.call0(&container);
        }
    }

    // Now, if a worker is already serving this page -- and again when one takes
    // over, which is what happens on the visit that installs it: the page is
    // loaded uncontrolled and claimed a moment later, so asking only at boot
    // would leave that visit unable to say what is serving it.
    ask_the_controller(&container);
    let container_again = container.clone();
    let taken_over = Closure::<dyn FnMut(web_sys::Event)>::new(move |_: web_sys::Event| {
        ask_the_controller(&container_again);
    });
    let _ = target
        .add_event_listener_with_callback("controllerchange", taken_over.as_ref().unchecked_ref());
    taken_over.forget();
}

/// Put the question to whichever worker is serving the page right now.
fn ask_the_controller(container: &wasm_bindgen::JsValue) {
    use wasm_bindgen::{JsCast, JsValue};

    let Ok(controller) = js_sys::Reflect::get(container, &"controller".into()) else {
        return;
    };
    if controller.is_null() || controller.is_undefined() {
        // The visit that INSTALLS a worker is not served by one, so there is
        // nobody to ask yet. Said out loud because a report with no `sw_build`
        // is otherwise ambiguous between this and a worker too old to answer.
        log::info!("no service worker controls this page yet, so none was asked which build it is");
        return;
    }
    let Ok(post) = js_sys::Reflect::get(&controller, &"postMessage".into()) else {
        return;
    };
    let Ok(post) = post.dyn_into::<js_sys::Function>() else {
        return;
    };
    match post.call1(&controller, &JsValue::from_str("which build?")) {
        Ok(_) => log::info!("asked the service worker which build it is"),
        Err(e) => log::info!("could not ask the service worker which build it is: {e:?}"),
    }
}

fn level_str(l: Level) -> &'static str {
    match l {
        Level::Error => "error",
        Level::Warn => "warn",
        Level::Info => "info",
        Level::Debug => "debug",
        Level::Trace => "trace",
    }
}

fn console_out(level: Level, line: &str) {
    let v = JsValue::from_str(line);
    match level {
        Level::Error => web_sys::console::error_1(&v),
        Level::Warn => web_sys::console::warn_1(&v),
        _ => web_sys::console::log_1(&v),
    }
}

/// A `log::Log` that mirrors to the console and queues warn/error for shipping.
struct RemoteLogger;

impl Log for RemoteLogger {
    fn enabled(&self, _: &Metadata) -> bool {
        true
    }

    fn log(&self, record: &Record) {
        let msg = record.args().to_string();
        console_out(
            record.level(),
            &format!("[{}] {}: {}", record.level(), record.target(), msg),
        );
        // Ship only warn/error, and only when a token is configured.
        //
        // Minus one message. A PDF trailer's `/Size` must be one more than the
        // highest object number, and producers get it wrong by one often enough
        // that lopdf simply counts the entries itself and says so. It has the
        // right answer before it warns, and the document renders. Shipping it
        // put a named user's report in Better Stack for every such file, which
        // reads as a fault in this app and is not one.
        //
        // Deliberately this message and not the whole crate: a PDF library's
        // complaints are worth seeing in an app that renders PDFs, and this is
        // the one that is provably self-correcting. A wording change upstream
        // fails open, back to noise rather than to silence.
        let benign = record.target().starts_with("lopdf")
            && msg.contains("Size entry of trailer dictionary");
        if record.level() <= Level::Warn && !benign && SOURCE_TOKEN.is_some() {
            // A stack for an error, none for a warning.
            //
            // `make_entry_with_stack` says why above: a stack sampled in the
            // handler is a trace of the logger. For a `log::` call it is worse
            // than that, because almost every one of them runs inside a spawned
            // task, and on wasm a task's stack begins at the microtask that
            // resumed it -- every `.await` before that has already erased the
            // caller. What arrives is the executor's resume path, expanded by
            // fat LTO into inlined generics from across the tree, so a refused
            // ballot came back naming the docx renderer and the colour library.
            // Checked against a real report: the symbols resolve correctly and
            // the frames are honest, they are just not the ones that led here.
            //
            // Warnings are the routine ones -- a session expiring, a server
            // refusing something -- where the message and the breadcrumb trail
            // already carry it, and 200 lines of plumbing per report cost
            // bandwidth and read as evidence. Errors keep theirs: the frames are
            // no better, but they are rare enough to be worth having when the
            // message alone does not explain it.
            //
            // A panic is untouched (`ship_sync` below): its hook runs on the
            // failing stack, synchronously, which is the case all of this was
            // built for and the one where the frames are the answer.
            let level = level_str(record.level());
            let message = format!("{}: {msg}", record.target());
            let entry = match record.level() {
                Level::Error => make_entry(level, message),
                _ => make_entry_with_stack(level, message, None),
            };
            PENDING.with(|p| p.borrow_mut().push(entry));
        }
    }

    fn flush(&self) {}
}

/// Ship any queued entries via a batched HTTP POST to the backend proxy (browser
/// fetch via reqwest). No auth header: the request is same-origin-friendly and
/// the backend holds the ingest token.
async fn flush() {
    if SOURCE_TOKEN.is_none() {
        return;
    }
    let batch: Vec<Value> = PENDING.with(|p| std::mem::take(&mut *p.borrow_mut()));
    if batch.is_empty() {
        return;
    }
    // Best-effort: on failure the batch is dropped rather than growing unbounded.
    let _ = reqwest::Client::new()
        .post(ingest_url())
        .json(&batch)
        .send()
        .await;
}

fn start_flush_loop() {
    wasm_bindgen_futures::spawn_local(async {
        loop {
            gloo_timers::future::TimeoutFuture::new(FLUSH_INTERVAL_MS).await;
            flush().await;
        }
    });
}

/// Ship a single entry synchronously (used from the panic hook, where the app is
/// about to abort and an async flush could not complete). A blocking XHR is
/// deliberate here: it guarantees the panic reaches the server.
fn ship_sync(entry: Value) {
    if SOURCE_TOKEN.is_none() {
        return;
    }
    let body = Value::Array(vec![entry]).to_string();
    let Ok(xhr) = web_sys::XmlHttpRequest::new() else {
        return;
    };
    if xhr.open_with_async("POST", &ingest_url(), false).is_err() {
        return;
    }
    let _ = xhr.set_request_header("Content-Type", "application/json");
    let _ = xhr.send_with_opt_str(Some(&body));
}

/// Set once the app has panicked, after which the global handlers stay quiet.
///
/// A panic aborts, which traps the wasm instance, and every JavaScript call back
/// into it then throws — "Unreachable code should not be executed", or a bare
/// "Script error." when the throw is cross-origin. Those arrive as uncaught
/// errors and were logged as if they were new failures, so one panic produced
/// three entries and the two extra ones described the corpse rather than the
/// cause.
static PANICKED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn panicked() -> bool {
    PANICKED.load(std::sync::atomic::Ordering::Relaxed)
}

fn setup_panic_hook() {
    // The console output and the crash overlay come from `crash`; this only adds
    // the shipping. Synchronous, so the report is away before the app tears down.
    crate::crash::install_hook(|info| {
        // Only the FIRST panic is the cause; anything after it describes the
        // corpse. Nothing unwinds here — `panic = "abort"` — so a borrow held
        // when the panic struck is never released, and the next thing to touch
        // the runtime panics again with "RefCell already borrowed" from inside a
        // dependency. Reported second, that reads like the failure and buries the
        // line that actually broke.
        if PANICKED.swap(true, std::sync::atomic::Ordering::Relaxed) {
            return;
        }
        ship_sync(make_entry("error", format!("PANIC: {info}")));
    });
}

/// Catch errors that never flow through `log::` or a Rust panic: uncaught JS
/// exceptions (e.g. thrown from web-sys glue) and unhandled promise rejections
/// (a `spawn`ed future that errored with no handler). Both ship with the same
/// enrichment (stack, breadcrumbs, user) as any other entry.
fn setup_global_error_handlers() {
    let Some(win) = web_sys::window() else {
        return;
    };
    let et: &web_sys::EventTarget = win.as_ref();
    crate::breadcrumbs::add_listener(et, "error", |ev| {
        // Nothing after a panic is news: the instance is trapped and every call
        // into it throws. The panic itself was already shipped.
        if panicked() {
            return;
        }
        // Only genuine script errors: an `ErrorEvent` with a message. Resource
        // load failures (img/script 404) also fire "error" but carry none.
        if let Some(ee) = ev.dyn_ref::<web_sys::ErrorEvent>() {
            let msg = ee.message();
            if msg.trim().is_empty() {
                return;
            }
            // The thrown value, when the browser lets us see it. Its `stack` is
            // the only true one here.
            let thrown = ee.error();
            let stack = stack_of(&thrown);
            // "Script error." with no file, line or column is the browser
            // REFUSING to describe an error it considers cross-origin — often
            // not our code at all, but an extension or a browser-injected
            // script. Say so, rather than leaving a report that looks like a
            // failure in the app and cannot be chased.
            //
            // And do not file it at all. It was an error, then a warning, and
            // both were having it both ways: the report says in one breath that
            // something failed and that nothing can be learned about it. There
            // is no file, no line, no stack and no message -- "Script error." is
            // the whole of it -- so there is nothing to act on and nothing to
            // fix, and the thing that threw is as likely to be an extension or
            // the browser's own injected script as this app. A record nobody can
            // work from is not a record, it is a queue to be triaged and closed.
            //
            // It stays on the console, where whoever is actually debugging that
            // device can see it happened.
            let opaque = stack.is_none() && ee.filename().is_empty() && ee.lineno() == 0;
            if opaque {
                log::info!("uncaught, opaque (no detail from the browser): {msg}");
                return;
            }
            let at = format!("{}:{}:{}", ee.filename(), ee.lineno(), ee.colno());
            queue(make_entry_with_stack(
                "error",
                format!("UNCAUGHT: {msg} @ {at}"),
                stack,
            ));
        }
    });
    crate::breadcrumbs::add_listener(et, "unhandledrejection", |ev| {
        if panicked() {
            return;
        }
        if let Some(pre) = ev.dyn_ref::<web_sys::PromiseRejectionEvent>() {
            let reason = pre.reason();
            let text = reason
                .as_string()
                // An Error is the common case and the one that used to come out
                // blank: `JSON.stringify(new Error("x"))` is `{}`, because its
                // fields are not enumerable. Read them directly.
                .or_else(|| error_text(&reason))
                .or_else(|| {
                    js_sys::JSON::stringify(&reason)
                        .ok()
                        .and_then(|s| s.as_string())
                        .filter(|s| s != "{}")
                })
                .unwrap_or_else(|| "unhandled rejection (no reason given)".to_string());
            // A rejected Error carries where it was thrown; the handler does not.
            let stack = stack_of(&reason).or_else(current_stack);
            queue(make_entry_with_stack(
                "error",
                format!("UNHANDLED REJECTION: {text}"),
                stack,
            ));
        }
    });
}

/// Queue an entry for the next flush (used by the global handlers, which — unlike
/// a panic — do not tear the app down, so an async ship is fine).
fn queue(entry: Value) {
    PENDING.with(|p| p.borrow_mut().push(entry));
}

/// Install the remote logger, breadcrumb listeners, panic hook and flush loop.
pub fn init() {
    let sid = format!("{:x}", (js_sys::Math::random() * 1e18) as u64);
    SESSION_ID.with(|s| *s.borrow_mut() = sid);

    let _ = log::set_boxed_logger(Box::new(RemoteLogger));
    log::set_max_level(LevelFilter::Info);

    setup_panic_hook();
    setup_global_error_handlers();
    start_flush_loop();
    ask_the_worker_which_build();

    if SOURCE_TOKEN.is_none() {
        log::info!("remote logging built in, but BETTERSTACK_SOURCE_TOKEN is unset: console only");
    } else {
        log::info!("remote logging active");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A stack ships as one frame per element, not as one line with `\n` in it.
    ///
    /// Logtail renders an array as a list and a string as a single line, so a
    /// newline-joined stack arrived as an unreadable wall of text.
    #[test]
    fn a_stack_is_a_list_of_frames() {
        let raw = "  at foo (src/a.rs:1)\nat bar (src/b.rs:2)\n\n  \n at baz (src/c.rs:3)  ";
        let frames = stack_frames(Some(raw.to_string()));
        assert_eq!(
            frames,
            serde_json::json!([
                "at foo (src/a.rs:1)",
                "at bar (src/b.rs:2)",
                "at baz (src/c.rs:3)",
            ]),
            "blank lines dropped, each frame trimmed"
        );
    }

    /// A rejected Error must not log as an empty line.
    ///
    /// `JSON.stringify(new Error("boom"))` is `{}` — its fields are not
    /// enumerable — which is how a real report arrived as "UNHANDLED REJECTION:"
    /// with nothing after the colon, from Googlebot rejecting a service worker
    /// registration.
    #[test]
    fn an_error_reason_reads_as_name_and_message() {
        assert_eq!(
            join_error_text(Some("SecurityError".into()), Some("Rejected".into())).as_deref(),
            Some("SecurityError: Rejected")
        );
        assert_eq!(
            join_error_text(None, Some("Rejected".into())).as_deref(),
            Some("Rejected")
        );
        assert_eq!(
            join_error_text(Some("Error".into()), None).as_deref(),
            Some("Error")
        );
        assert_eq!(join_error_text(None, None), None);
    }

    /// No stack is null, not an empty list: "we never got one" and "it had no
    /// frames" are different things to read in a log.
    #[test]
    fn a_missing_stack_stays_missing() {
        assert_eq!(stack_frames(None), serde_json::Value::Null);
        assert_eq!(
            stack_frames(Some("   \n  ".to_string())),
            serde_json::json!([])
        );
    }
}
