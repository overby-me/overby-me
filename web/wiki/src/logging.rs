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
use std::collections::VecDeque;

use log::{Level, LevelFilter, Log, Metadata, Record};
use serde_json::{json, Value};
use wasm_bindgen::prelude::*;

/// Set at build (any value) when this build should ship logs; `None` leaves
/// shipping off (console only). Presence, not the value, is the switch — the
/// actual ingest token lives on the backend, which does the Better Stack call
/// (see `backend/src/logs.rs`), so no secret is baked into the wasm bundle.
const SOURCE_TOKEN: Option<&str> = option_env!("BETTERSTACK_SOURCE_TOKEN");
const MAX_BREADCRUMBS: usize = 50;
const FLUSH_INTERVAL_MS: u32 = 5000;

thread_local! {
    /// Log entries waiting to be shipped on the next flush.
    static PENDING: RefCell<Vec<Value>> = const { RefCell::new(Vec::new()) };
    /// The rolling trail of recent DOM interactions.
    static BREADCRUMBS: RefCell<VecDeque<String>> = const { RefCell::new(VecDeque::new()) };
    /// A random id identifying this browser tab/session.
    static SESSION_ID: RefCell<String> = const { RefCell::new(String::new()) };
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

fn push_breadcrumb(text: String) {
    BREADCRUMBS.with(|b| {
        let mut b = b.borrow_mut();
        if b.len() >= MAX_BREADCRUMBS {
            b.pop_front();
        }
        b.push_back(text);
    });
}

fn breadcrumbs() -> Vec<String> {
    BREADCRUMBS.with(|b| b.borrow().iter().cloned().collect())
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
        "breadcrumbs": breadcrumbs(),
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
    let list = ask
        .call1(&w, &"(display-mode: standalone)".into())
        .ok()?;
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
        if record.level() <= Level::Warn && SOURCE_TOKEN.is_some() {
            let entry = make_entry(
                level_str(record.level()),
                format!("{}: {msg}", record.target()),
            );
            PENDING.with(|p| p.borrow_mut().push(entry));
        }
    }

    fn flush(&self) {}
}

/// A compact description of an element for a breadcrumb: `tag#id.class "label"`,
/// resolved to the nearest interactive ancestor. Never includes input values.
fn describe(el: &web_sys::Element) -> String {
    let target = el
        .closest("button, a, input, textarea, select, [role=button], .btn, .btn-icon, .list-item, .folder-item")
        .ok()
        .flatten()
        .unwrap_or_else(|| el.clone());
    let tag = target.tag_name().to_lowercase();
    let id = target.id();
    let id_part = if id.is_empty() {
        String::new()
    } else {
        format!("#{id}")
    };
    let class = target.get_attribute("class").unwrap_or_default();
    let class_part = class
        .split_whitespace()
        .next()
        .map(|c| format!(".{c}"))
        .unwrap_or_default();
    // Field inputs: identity only, never the value; hide password fields.
    if matches!(tag.as_str(), "input" | "textarea" | "select") {
        if target.get_attribute("type").as_deref() == Some("password") {
            return format!("{tag}{id_part} [password]");
        }
        let name = target
            .get_attribute("name")
            .map(|n| format!("[name={n}]"))
            .unwrap_or_default();
        return format!("{tag}{id_part}{name}");
    }
    let label = target
        .get_attribute("aria-label")
        .or_else(|| target.get_attribute("title"))
        .or_else(|| {
            let t = target.text_content().unwrap_or_default();
            let t = t.trim();
            if t.is_empty() {
                None
            } else {
                Some(t.chars().take(40).collect())
            }
        })
        .map(|l| format!(" \"{}\"", l.replace('"', "'")))
        .unwrap_or_default();
    // For a link, also record where it points (the destination is the relevant
    // context for a click that navigates). A same-origin href is trimmed to its
    // path so the trail reads as in-app routes.
    let href = if tag == "a" {
        target
            .get_attribute("href")
            .filter(|h| !h.is_empty())
            .map(|h| format!(" -> {h}"))
            .unwrap_or_default()
    } else {
        String::new()
    };
    format!("{tag}{id_part}{class_part}{label}{href}")
}

fn target_element(ev: &web_sys::Event) -> Option<web_sys::Element> {
    ev.target()?.dyn_into::<web_sys::Element>().ok()
}

fn add_listener<F: FnMut(&web_sys::Event) + 'static>(
    target: &web_sys::EventTarget,
    event: &str,
    mut f: F,
) {
    let closure = Closure::<dyn FnMut(web_sys::Event)>::new(move |ev: web_sys::Event| f(&ev));
    let _ = target.add_event_listener_with_callback(event, closure.as_ref().unchecked_ref());
    // Leak the closure so the listener lives for the app's lifetime.
    closure.forget();
}

/// Record clicks, field edits and form submissions as breadcrumbs (no values).
fn setup_breadcrumbs() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let et: &web_sys::EventTarget = doc.as_ref();
    add_listener(et, "click", |ev| {
        if let Some(el) = target_element(ev) {
            push_breadcrumb(format!("click {}", describe(&el)));
        }
    });
    add_listener(et, "change", |ev| {
        if let Some(el) = target_element(ev) {
            push_breadcrumb(format!("change {}", describe(&el)));
        }
    });
    add_listener(et, "submit", |ev| {
        if let Some(el) = target_element(ev) {
            push_breadcrumb(format!("submit {}", describe(&el)));
        }
    });
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
    add_listener(et, "error", |ev| {
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
            // And say it at warn. Naming it opaque while still filing it as an
            // error was having it both ways: the reader of the report is told in
            // the same breath that something failed and that nothing can be
            // learned about it, and it lands in the same list as a real crash.
            // Warn keeps the record (a burst of these is still worth seeing)
            // without claiming the app broke, which nothing here establishes.
            let opaque = stack.is_none() && ee.filename().is_empty() && ee.lineno() == 0;
            let (level, message) = if opaque {
                (
                    "warn",
                    format!("UNCAUGHT (opaque, no detail from the browser): {msg}"),
                )
            } else {
                let at = format!("{}:{}:{}", ee.filename(), ee.lineno(), ee.colno());
                ("error", format!("UNCAUGHT: {msg} @ {at}"))
            };
            queue(make_entry_with_stack(level, message, stack));
        }
    });
    add_listener(et, "unhandledrejection", |ev| {
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

thread_local! {
    /// The last path recorded as a navigation breadcrumb, to dedupe the router's
    /// initial + repeated route effects.
    static LAST_NAV: RefCell<String> = const { RefCell::new(String::new()) };
}

/// Record a client-side navigation as a breadcrumb. Called by the router on each
/// route change (see `layout::Layout`), so an error's trail shows the pages the
/// user moved through, not just the URL they were on when it broke.
pub fn record_navigation(path: &str) {
    let changed = LAST_NAV.with(|l| {
        let mut l = l.borrow_mut();
        if *l == path {
            false
        } else {
            *l = path.to_string();
            true
        }
    });
    if changed {
        push_breadcrumb(format!("navigate {path}"));
    }
}

/// Install the remote logger, breadcrumb listeners, panic hook and flush loop.
pub fn init() {
    let sid = format!("{:x}", (js_sys::Math::random() * 1e18) as u64);
    SESSION_ID.with(|s| *s.borrow_mut() = sid);

    let _ = log::set_boxed_logger(Box::new(RemoteLogger));
    log::set_max_level(LevelFilter::Info);

    setup_breadcrumbs();
    setup_panic_hook();
    setup_global_error_handlers();
    start_flush_loop();

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
