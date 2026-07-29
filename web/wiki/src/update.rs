//! Noticing that the running bundle is out of date, and offering a way out.
//!
//! Two things conspire to leave a reader on old code for a long time. The app is
//! a single page, so a tab open across a deploy never re-fetches anything; and
//! the service worker serves the app shell from its cache first (stale-while-
//! revalidate, `assets/sw.js`), so even a reload can hand back the build that was
//! current an hour ago. Neither is wrong — both are why the app opens instantly
//! on venue wifi — but together they mean "just tell them to refresh" does not
//! work.
//!
//! So the build writes its commit to `/version.json` (see the justfile), and this
//! compares that against the commit compiled into the bundle
//! ([`crate::build_info::COMMIT`]). A difference means a newer build is deployed
//! and this tab is not running it.
//!
//! Checks happen shortly after load, every quarter hour after that, and whenever
//! the tab is looked at again. That last one carries most of the weight: someone
//! coming back to a tab left open since this morning finds out then, rather than
//! whenever the timer next comes round.

use std::cell::Cell;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use crate::i18n::t;

/// Set once a newer build is known to be live. Drives [`UpdateBanner`].
pub static UPDATE_AVAILABLE: GlobalSignal<bool> = Signal::global(|| false);

thread_local! {
    /// When the last check went out. Returning to a tab triggers one, and
    /// without this, flicking between two tabs would mean a request per flick.
    static LAST_CHECK_MS: Cell<f64> = const { Cell::new(0.0) };
    /// Set by "Not now". Being told once is informative; being told again every
    /// time the tab regains focus is nagging, and they already answered.
    static DISMISSED: Cell<bool> = const { Cell::new(false) };
}

/// Wait this long before the first check. The page has just loaded, but from the
/// service worker's cache it may already be stale, so this is short.
const FIRST_CHECK_MS: u32 = 5_000;

/// And this long between checks after that. A deploy during a long-open tab is
/// what this catches.
const CHECK_INTERVAL_MS: u32 = 15 * 60 * 1000;

/// The closest together two checks may fall. Only the visibility trigger can get
/// near it; the timer above is far slower.
const MIN_CHECK_GAP_MS: f64 = 60_000.0;

/// Start watching for a newer build. Call once, from inside the Dioxus runtime.
///
/// Does nothing when this build has no commit to compare (`unknown`, from a
/// build made outside the deploy path): every check would report a difference
/// and the banner would never go away.
pub fn spawn_update_check() {
    if crate::build_info::COMMIT == "unknown" {
        return;
    }
    install_visibility_check();
    dioxus::core::spawn_forever(async move {
        gloo_timers::future::TimeoutFuture::new(FIRST_CHECK_MS).await;
        loop {
            if *UPDATE_AVAILABLE.peek() || DISMISSED.with(Cell::get) {
                return;
            }
            check_now();
            gloo_timers::future::TimeoutFuture::new(CHECK_INTERVAL_MS).await;
        }
    });
}

/// Check unless it would be pointless (already known, already declined) or too
/// soon. Spawns; the caller does not wait.
fn check_now() {
    if *UPDATE_AVAILABLE.peek() || DISMISSED.with(Cell::get) {
        return;
    }
    let now = js_sys::Date::now();
    if now - LAST_CHECK_MS.with(Cell::get) < MIN_CHECK_GAP_MS {
        return;
    }
    LAST_CHECK_MS.with(|t| t.set(now));
    // `spawn_local`, not Dioxus's `spawn_forever`: this is also called from a
    // `visibilitychange` callback, where no Dioxus scope is being rendered.
    // Writing a `GlobalSignal` from there is fine (the runtime outlives the
    // whole page), but asking Dioxus to attach a task to a scope from outside
    // one is not something to rely on in a path that runs on every tab switch.
    wasm_bindgen_futures::spawn_local(async move {
        let Some(deployed) = deployed_commit().await else {
            return;
        };
        if deployed != crate::build_info::COMMIT {
            log::info!(
                "running {} but {deployed} is deployed",
                crate::build_info::COMMIT
            );
            *UPDATE_AVAILABLE.write() = true;
        }
    });
}

/// Check again when the tab is looked at, not only when the timer comes round.
///
/// Coming back to a tab left open for hours is exactly the moment being on old
/// code matters, and on the timer alone a reader could sit there for another
/// quarter of an hour before being told.
fn install_visibility_check() {
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return;
    };
    let on_visibility = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(|| {
        let hidden = web_sys::window()
            .and_then(|w| w.document())
            .is_some_and(|d| d.hidden());
        if !hidden {
            check_now();
        }
    });
    let _ = doc.add_event_listener_with_callback(
        "visibilitychange",
        on_visibility.as_ref().unchecked_ref(),
    );
    // Lives as long as the page; nothing ever removes this listener.
    on_visibility.forget();
}

/// The commit the site is currently serving, or `None` if that cannot be
/// established — an older deploy with no `version.json`, or simply being offline.
/// Silence is the right answer to both: a failed check must never be read as
/// "you are out of date".
async fn deployed_commit() -> Option<String> {
    let origin = web_sys::window()?.location().origin().ok()?;
    // The cache-busting parameter matters. The service worker answers same-origin
    // GETs from its cache first, so a plain fetch here could be served the very
    // build being compared against — the check would then always agree with
    // itself. `sw.js` skips this path outright, but an already-installed worker
    // stays in control until it updates, and this covers that window.
    let url = format!("{origin}/version.json?t={}", js_sys::Date::now() as u64);
    let response = reqwest::Client::new().get(&url).send().await.ok()?;
    if !response.status().is_success() {
        return None;
    }
    let body: serde_json::Value = response.json().await.ok()?;
    body.get("commit")?.as_str().map(str::to_string)
}

/// Drop the cached app shell, so the reload that follows actually fetches the new
/// build.
///
/// Without this the reload is close to useless: the service worker would answer
/// the navigation from its cache — the shell it stored when this tab opened,
/// which names the OLD asset hashes — and only refresh it in the background. The
/// reader would land back on the same build and be told again 15 minutes later.
///
/// Only the shell is dropped. Everything under `/assets/` is content-hashed, so
/// what the new shell needs it asks for by a new URL anyway, and what it does not
/// need costs nothing to keep.
async fn drop_cached_shell() {
    use wasm_bindgen::JsCast;
    use wasm_bindgen_futures::JsFuture;

    let Some(caches) = web_sys::window().and_then(|w| w.caches().ok()) else {
        return;
    };
    let Ok(keys) = JsFuture::from(caches.keys()).await else {
        return;
    };
    let Some(keys) = keys.dyn_ref::<js_sys::Array>() else {
        return;
    };
    for key in keys.iter() {
        let Some(name) = key.as_string() else {
            continue;
        };
        let Ok(cache) = JsFuture::from(caches.open(&name)).await else {
            continue;
        };
        let Some(cache) = cache.dyn_ref::<web_sys::Cache>() else {
            continue;
        };
        for entry in ["/", "/index.html"] {
            let _ = JsFuture::from(cache.delete_with_str(entry)).await;
        }
    }
}

/// The offer to reload, at the app-shell root beside the snackbar.
#[component]
pub fn UpdateBanner() -> Element {
    if !UPDATE_AVAILABLE() {
        return rsx! {};
    }
    rsx! {
        div { class: "update-banner", role: "status", aria_live: "polite",
            span { class: "update-banner-text", "{t(\"update.available\")}" }
            button {
                class: "btn btn-text",
                onclick: move |_| {
                    DISMISSED.with(|d| d.set(true));
                    *UPDATE_AVAILABLE.write() = false;
                },
                "{t(\"update.dismiss\")}"
            }
            button {
                class: "btn btn-primary",
                onclick: move |_| {
                    spawn(async move {
                        drop_cached_shell().await;
                        if let Some(win) = web_sys::window() {
                            let _ = win.location().reload();
                        }
                    });
                },
                "{t(\"update.reload\")}"
            }
        }
    }
}
