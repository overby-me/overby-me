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

/// One structured log entry with the standard enrichment, so Logtail can filter
/// and group by any field:
/// - who: `user_id` / `user_name`, plus a per-tab `session_id`
/// - where: the current URL `path`, the app `app_version`, and a JS `stack`
/// - what they did: `breadcrumbs` (the recent navigation + click/change/submit
///   trail, newest last)
fn make_entry(level: &str, message: String) -> Value {
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
        "stack": current_stack(),
        "user_agent": web_sys::window()
            .and_then(|w| w.navigator().user_agent().ok()),
        "breadcrumbs": breadcrumbs(),
    })
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

fn setup_panic_hook() {
    // The console output and the crash overlay come from `crash`; this only adds
    // the shipping. Synchronous, so the report is away before the app tears down.
    crate::crash::install_hook(|info| {
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
        // Only genuine script errors: an `ErrorEvent` with a message. Resource
        // load failures (img/script 404) also fire "error" but carry none.
        if let Some(ee) = ev.dyn_ref::<web_sys::ErrorEvent>() {
            let msg = ee.message();
            if msg.trim().is_empty() {
                return;
            }
            let at = format!("{}:{}:{}", ee.filename(), ee.lineno(), ee.colno());
            queue(make_entry("error", format!("UNCAUGHT: {msg} @ {at}")));
        }
    });
    add_listener(et, "unhandledrejection", |ev| {
        if let Some(pre) = ev.dyn_ref::<web_sys::PromiseRejectionEvent>() {
            let reason = pre.reason();
            let text = reason
                .as_string()
                .or_else(|| {
                    js_sys::JSON::stringify(&reason)
                        .ok()
                        .and_then(|s| s.as_string())
                })
                .unwrap_or_else(|| "unhandled rejection".to_string());
            queue(make_entry("error", format!("UNHANDLED REJECTION: {text}")));
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
