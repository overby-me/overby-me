mod components;
#[allow(dead_code)]
mod export;
#[allow(dead_code)]
mod graphql;
#[allow(dead_code)]
mod i18n;
#[cfg(feature = "remote-logging")]
mod logging;
#[allow(dead_code)]
mod nhost;
mod pwa;
#[allow(dead_code)]
mod roster;
mod route;
#[allow(dead_code)]
mod session;
pub mod snackbar;
#[allow(dead_code)]
mod subscription;
#[allow(dead_code)]
mod theme;
#[allow(dead_code)]
mod window_size;

use dioxus::prelude::*;
use route::Route;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

const STYLE_CSS: Asset = asset!("/assets/style.css");
// The dioxus-components (shadcn-style) theme tokens, loaded before our own CSS
// so app styles can build on / override them as screens migrate to primitives.
const DX_THEME_CSS: Asset = asset!("/assets/dx-components-theme.css");
// The Material Design 3 colour scheme (generated from the Radikale brand seeds
// by scripts/gen-theme.ts). Defines the canonical --md-sys-color-* roles that
// style.css's --md-* tokens alias, so the whole app re-skins from one file.
const M3_THEME_CSS: Asset = asset!("/assets/m3-theme.css");
// Non-colour M3 system tokens (type scale, shape/corner scale, elevation,
// state-layer opacities, motion) — hand-authored from the M3 spec.
const M3_TOKENS_CSS: Asset = asset!("/assets/m3-tokens.css");

fn main() {
    // Print real panic messages (with a JS stack trace) to the console instead
    // of a bare `unreachable executed` wasm trap — the single highest-value
    // change for debugging authenticated-load traps in Servo.
    console_error_panic_hook::set_once();
    // With `remote-logging`, ship warn/error (plus breadcrumbs + panics) to
    // Logtail and mirror to the console; otherwise just log to the console.
    #[cfg(feature = "remote-logging")]
    logging::init();
    #[cfg(not(feature = "remote-logging"))]
    wasm_logger::init(wasm_logger::Config::default());
    log::info!("RadikalWiki starting...");

    // Clean the stray trailing "?" the router emits for the optional `app` query
    // before the first navigation writes it to the address bar.
    install_history_query_shim();

    // Capture a password-reset deep link (`/?type=passwordReset&refreshToken=...`)
    // now, before the router mounts and rewrites `/`'s query away; Layout then
    // exchanges the token and shows the set-password form.
    components::layout::capture_reset_token();

    // PWA: install the manifest / icon / theme-color head tags and register the
    // service worker (offline where the SW controls the root).
    pwa::setup();

    dioxus::launch(App);
}

/// Strip the stray trailing "?" the Dioxus router writes for the optional `app`
/// query (a route with no `app=` serializes to e.g. "/group?"). Wrap
/// `history.pushState` / `history.replaceState` so any URL ending in a bare "?"
/// is cleaned at the source, before it reaches the address bar. This is
/// race-free, unlike stripping after the fact in an effect: the router rewrites
/// the URL on its own schedule and would re-add the "?" after such a strip.
fn install_history_query_shim() {
    use wasm_bindgen::JsValue;
    let Some(win) = web_sys::window() else { return };
    let Ok(history) = js_sys::Reflect::get(&win, &JsValue::from_str("history")) else {
        return;
    };
    for method in ["pushState", "replaceState"] {
        let key = JsValue::from_str(method);
        let Ok(orig) = js_sys::Reflect::get(&history, &key) else {
            continue;
        };
        let Ok(orig_fn) = orig.dyn_into::<js_sys::Function>() else {
            continue;
        };
        let this = history.clone();
        let wrapper = Closure::wrap(
            Box::new(move |state: JsValue, title: JsValue, url: JsValue| {
                let url = match url.as_string() {
                    Some(s) if s.ends_with('?') => JsValue::from_str(s.strip_suffix('?').unwrap()),
                    _ => url,
                };
                let args = js_sys::Array::of3(&state, &title, &url);
                let _ = js_sys::Reflect::apply(&orig_fn, &this, &args);
            }) as Box<dyn FnMut(JsValue, JsValue, JsValue)>,
        );
        let _ = js_sys::Reflect::set(&history, &key, wrapper.as_ref().unchecked_ref());
        // Leak the closure so the wrapped method lives for the app's lifetime.
        wrapper.forget();
    }
}

#[component]
fn App() -> Element {
    // Initialize global state once, *inside* the Dioxus runtime. Writing to a
    // `GlobalSignal` (SESSION / LANG) from `main` before `launch` panics with
    // "Must be called from inside a Dioxus runtime" — and did so non-obviously
    // only when localStorage already held a session (so `load_session` wrote),
    // which is exactly the flaky authenticated-load trap from PLAN.md issue 1.
    use_hook(|| {
        // Load persisted session from localStorage.
        session::load_session();

        // Detect browser language for i18n.
        if let Some(window) = web_sys::window() {
            if let Some(lang) = window.navigator().language() {
                if lang.starts_with("da") {
                    *i18n::LANG.write() = i18n::Lang::Da;
                }
            }
        }

        // Load the persisted theme, then apply it to the document element.
        theme::load_theme();
        theme::apply_theme(&theme::THEME.read());
        // Load any user-picked M3 seed colours and inject the override scheme.
        theme::load_seeds();

        // Nudge the token-refresh loop whenever the tab becomes visible again:
        // a backgrounded tab throttles timers, so the access token can lapse
        // while it sits stale. Refreshing on return keeps the session alive.
        if let Some(document) = web_sys::window().and_then(|w| w.document()) {
            let doc = document.clone();
            let closure = Closure::wrap(Box::new(move || {
                if !doc.hidden() {
                    session::nudge_refresh();
                }
            }) as Box<dyn FnMut()>);
            let _ = document.add_event_listener_with_callback(
                "visibilitychange",
                closure.as_ref().unchecked_ref(),
            );
            // Leak the closure so the listener lives for the app's lifetime.
            closure.forget();
        }
    });

    // Keep the NHost access token fresh (renew before expiry / on return).
    use_future(session::run_token_refresh);

    rsx! {
        document::Stylesheet { href: DX_THEME_CSS }
        document::Stylesheet { href: M3_THEME_CSS }
        document::Stylesheet { href: M3_TOKENS_CSS }
        document::Stylesheet { href: STYLE_CSS }
        Router::<Route> {}
        snackbar::Snackbar {}
    }
}
