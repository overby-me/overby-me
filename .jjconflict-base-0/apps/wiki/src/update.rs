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
//! the reader comes back to the page — switching tabs, refocusing the window
//! from another application, or a back/forward-cache restore. That last group
//! carries most of the weight: someone coming back to a tab left open since this
//! morning finds out then, rather than whenever the timer next comes round.

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

/// The closest together two checks may fall. Only the return triggers can get
/// near it, and they overlap by design (see [`install_return_checks`]); the
/// timer above is far slower.
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
    install_return_checks();
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
    // `spawn_local`, not Dioxus's `spawn_forever`: this is also called from raw
    // DOM listeners (see `install_return_checks`), where no Dioxus scope is
    // being rendered, and asking Dioxus to attach a task to a scope from outside
    // one is not something to rely on in a path that runs every time the reader
    // comes back.
    //
    // The write below then needs the runtime handed back explicitly. This is a
    // future rather than a callback, so it resumes after an await on the
    // microtask queue — later than, and unprotected by, whatever guard the
    // caller held.
    wasm_bindgen_futures::spawn_local(async move {
        let Some(deployed) = deployed_commit().await else {
            return;
        };
        if deployed != crate::build_info::COMMIT {
            log::info!(
                "running {} but {deployed} is deployed",
                crate::build_info::COMMIT
            );
            crate::runtime::enter(|| *UPDATE_AVAILABLE.write() = true);
        }
    });
}

/// Check again when the reader comes back to the page, not only when the timer
/// comes round.
///
/// Coming back to a tab left open for hours is exactly the moment being on old
/// code matters, and on the timer alone a reader could sit there for another
/// quarter of an hour before being told.
///
/// Three events, because "coming back" happens in three ways and only one of
/// them is a tab switch:
///
/// - `visibilitychange` covers switching tabs, and minimising on the platforms
///   that report it. It does NOT fire when the reader switches to another
///   application and the browser window stays visible, which is the common case
///   on a desktop and was the gap this used to have.
/// - `focus` covers exactly that: returning to the browser from something else,
///   or from another browser window.
/// - `pageshow` covers a restore from the back/forward cache, which on iOS
///   frequently does not fire `visibilitychange` at all.
///
/// They overlap, and that costs nothing: `check_now` will not check twice
/// within [`MIN_CHECK_GAP_MS`], nor once an update is already known.
fn install_return_checks() {
    let Some(win) = web_sys::window() else {
        return;
    };
    let Some(doc) = win.document() else {
        return;
    };
    let on_return = wasm_bindgen::closure::Closure::<dyn FnMut()>::new(|| {
        // A hidden page is not being looked at, whichever event woke us.
        let hidden = web_sys::window()
            .and_then(|w| w.document())
            .is_some_and(|d| d.hidden());
        if !hidden {
            // The browser calls this, so the runtime has to be put back before
            // `check_now` reads `UPDATE_AVAILABLE` (see `crate::runtime`).
            crate::runtime::enter(check_now);
        }
    });
    let handler = on_return.as_ref().unchecked_ref();
    let _ = doc.add_event_listener_with_callback("visibilitychange", handler);
    let _ = win.add_event_listener_with_callback("focus", handler);
    let _ = win.add_event_listener_with_callback("pageshow", handler);
    // Lives as long as the page; nothing ever removes these listeners.
    on_return.forget();
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
/// The shell is cached under every URL it was ever served at, not just `/`. The
/// host answers any deep link with `index.html` (see `assets/_redirects`), the
/// worker files each response under the URL that was asked for, and it looks
/// them up the same way. So a reader standing on an agenda item had a stale copy
/// under that whole path, this deleted `/` and `/index.html`, and the reload was
/// served the build it was meant to replace. Only the second press worked, by
/// which time the background revalidate had quietly fixed that one path. Hence:
/// every cached entry goes, keyed by what the cache actually holds.
///
/// Except `/assets/`, which is content-hashed. A changed file is a changed URL,
/// so the new shell asks for what it needs by a name that cannot be stale, and
/// dropping the rest would only re-download fonts and icons that were still
/// correct. That matters here: the reader on venue wifi is the one being asked
/// to reload.
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
        let Ok(entries) = JsFuture::from(cache.keys()).await else {
            continue;
        };
        let Some(entries) = entries.dyn_ref::<js_sys::Array>() else {
            continue;
        };
        for entry in entries.iter() {
            // The entries are `Request`s. Reading `.url` reflectively keeps the
            // `Request` binding (and its feature) out of a path that only wants
            // a string to delete by.
            let Some(url) = js_sys::Reflect::get(&entry, &"url".into())
                .ok()
                .and_then(|v| v.as_string())
            else {
                continue;
            };
            if names_its_own_content(&url) {
                continue;
            }
            let _ = JsFuture::from(cache.delete_with_str(&url)).await;
        }
    }
}

/// Whether this cached URL is safe to keep across an update, because its content
/// hash is part of its path and a different build would ask for a different URL.
fn names_its_own_content(url: &str) -> bool {
    let path = cached_path(url);
    // `/symbols/` is hashed the same way. No reader fetches it, so this is only
    // for completeness.
    path.starts_with("/assets/") || path.starts_with("/symbols/")
}

/// The path of a cache key, which the Cache API stores as an absolute URL.
///
/// Matching on the path rather than the whole string is what keeps a document at
/// `/x/assets/y` from being mistaken for a hashed asset.
fn cached_path(url: &str) -> &str {
    let Some((_scheme, rest)) = url.split_once("://") else {
        return url; // already a path, or something unparseable: treat it as one
    };
    match rest.find('/') {
        Some(start) => &rest[start..],
        None => "/",
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

#[cfg(test)]
mod tests {
    use super::names_its_own_content as kept;

    /// Hashed URLs survive the purge: a new build asks for new ones, and
    /// re-downloading a 125 KB icon font over venue wifi helps nobody.
    #[test]
    fn a_hashed_asset_is_kept() {
        assert!(kept("https://radikal.wiki/assets/style-dxh43d5a6.css"));
        assert!(kept(
            "https://radikal.wiki/assets/material-icons-dxhd3c.woff2"
        ));
        assert!(kept("https://radikal.wiki/symbols/wiki-dioxus-dxh27f.wasm"));
    }

    /// Everything the shell is served as goes, whatever URL it was cached under.
    /// The deep path is the one that used to survive and cost a second press.
    #[test]
    fn every_copy_of_the_shell_is_dropped() {
        for url in [
            "https://radikal.wiki/",
            "https://radikal.wiki/index.html",
            "https://radikal.wiki/radikal_ungdom/hb5/dagsorden_1.0",
            "https://radikal.wiki/radikal_ungdom/hb5/dagsorden_1.0?tab=2",
            "https://radikal.wiki/?",
            "https://radikal.wiki/user/login",
        ] {
            assert!(!kept(url), "{url} should have been dropped");
        }
    }

    /// A page whose own path happens to contain `/assets/` is still a page. This
    /// is why the check is anchored to the path and not run over the whole URL.
    #[test]
    fn a_page_that_merely_mentions_assets_is_not_an_asset() {
        assert!(!kept("https://radikal.wiki/radikal_ungdom/assets/plan"));
        assert!(!kept("https://radikal.wiki/x?next=/assets/style.css"));
    }

    /// Odd shapes must not panic, and must not be mistaken for assets.
    #[test]
    fn a_malformed_key_is_dropped_rather_than_kept() {
        assert!(!kept("https://radikal.wiki"));
        assert!(!kept(""));
        assert!(!kept("nonsense"));
        // A bare path, should the Cache API ever hand one back.
        assert!(kept("/assets/style-dxh43d5a6.css"));
        assert!(!kept("/index.html"));
    }
}
