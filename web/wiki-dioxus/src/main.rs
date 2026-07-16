mod components;
mod density;
mod export;
// cynic query-result structs must select fields the code doesn't read (e.g. an
// insert's returned `id`, a delete's `affected_rows`), which reads as dead code.
#[allow(dead_code)]
mod graphql;
mod i18n;
#[cfg(feature = "remote-logging")]
mod logging;
mod nhost;
mod pwa;
mod route;
mod session;
pub mod snackbar;
mod subscription;
mod theme;
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

// Self-hosted fonts (privacy + offline + sovereignty: no Google Fonts CDN). The
// body face (Atkinson Hyperlegible, latin subset) and the Material Icons ligature
// font are bundled via asset!() and declared with @font-face at mount, replacing
// the two CDN @imports style.css used to carry. Bundling them means the service
// worker's cache-first /assets rule also caches them, so they work offline.
const FONT_ATK_400: Asset = asset!("/assets/fonts/atkinson-400.woff2");
const FONT_ATK_700: Asset = asset!("/assets/fonts/atkinson-700.woff2");
const FONT_ATK_400I: Asset = asset!("/assets/fonts/atkinson-400i.woff2");
const FONT_ATK_700I: Asset = asset!("/assets/fonts/atkinson-700i.woff2");
const FONT_MATERIAL: Asset = asset!("/assets/fonts/material-icons.woff2");

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

/// Extract the `claim` query-param value from a `location.search` string
/// (e.g. `?claim=abc&x=1` → `abc`). The token is a uuid, so no decoding needed.
fn claim_token_from_search(search: &str) -> Option<String> {
    search
        .trim_start_matches('?')
        .split('&')
        .find_map(|pair| pair.strip_prefix("claim="))
        .map(str::to_string)
        .filter(|v| !v.is_empty())
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

        // If we just came back from the atproto (Bluesky) linking flow, surface the
        // outcome and drop the ?linked query so it does not re-fire on refresh.
        if let Some(win) = web_sys::window() {
            if let Ok(search) = win.location().search() {
                let msg = if search.contains("linked=bluesky") {
                    Some(i18n::t("profile.linkedOk"))
                } else if search.contains("linked=error") {
                    Some(i18n::t("profile.linkedErr"))
                } else {
                    None
                };
                if let Some(msg) = msg {
                    snackbar::show_snackbar(&msg);
                    if let Ok(history) = win.history() {
                        let _ = history.replace_state_with_url(
                            &wasm_bindgen::JsValue::NULL,
                            "",
                            Some("/"),
                        );
                    }
                }

                // A `?claim=<token>` invitation link: stash the token (so it
                // survives a login) and strip it from the URL so the secret token
                // isn't left in history. Consumed by the effect below once signed in.
                if let Some(tok) = claim_token_from_search(&search) {
                    *session::PENDING_CLAIM.write() = Some(tok);
                    if let Ok(history) = win.history() {
                        let _ = history.replace_state_with_url(
                            &wasm_bindgen::JsValue::NULL,
                            "",
                            Some("/"),
                        );
                    }
                }
            }
        }

        // Detect browser language for i18n.
        if let Some(window) = web_sys::window() {
            if let Some(lang) = window.navigator().language() {
                if lang.starts_with("da") {
                    *i18n::LANG.write() = i18n::Lang::Da;
                }
            }
        }
        // Reflect the language on <html lang> so `hyphens: auto` works.
        i18n::apply_lang(&i18n::LANG.read());

        // Load the persisted theme, then apply it to the document element.
        theme::load_theme();
        theme::apply_theme(&theme::THEME.read());
        // Load any user-picked M3 seed colours and inject the override scheme.
        theme::load_seeds();
        // DESIGN: load the persisted UI density preference.
        density::load_density();

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

    // Consume a pending `?claim=` invitation once signed in: bind the membership
    // to this account (regardless of the roster email) and land on the context.
    let session_sig = session::use_session();
    use_effect(move || {
        let authed = session_sig.read().is_authenticated();
        let pending = session::PENDING_CLAIM();
        if authed {
            if let Some(claim) = pending {
                *session::PENDING_CLAIM.write() = None;
                let token = session_sig.read().access_token.clone();
                spawn(async move {
                    let Some(token) = token else { return };
                    match nhost::claim_membership(&token, &claim).await {
                        Ok(context) => {
                            session::bump_data_version();
                            snackbar::show_snackbar(&i18n::t("invite.claimedOk"));
                            if let Ok(segments) =
                                graphql::path_from_id(Some(&token), &context).await
                            {
                                if !segments.is_empty() {
                                    if let Some(win) = web_sys::window() {
                                        let _ = win
                                            .location()
                                            .set_href(&format!("/{}", segments.join("/")));
                                    }
                                }
                            }
                        }
                        Err(_) => snackbar::show_snackbar(&i18n::t("invite.claimErr")),
                    }
                });
            }
        }
    });

    // Keep the NHost access token fresh (renew before expiry / on return).
    use_future(session::run_token_refresh);

    // @font-face for the self-hosted (asset-bundled) fonts. Built here so the src
    // URLs are the content-hashed asset paths, avoiding any CSS url() rewriting
    // dependency. `block` display for icons prevents a flash of ligature text.
    let font_face = format!(
        concat!(
            "@font-face{{font-family:'Atkinson Hyperlegible';font-style:normal;font-weight:400;font-display:swap;src:url({}) format('woff2')}}",
            "@font-face{{font-family:'Atkinson Hyperlegible';font-style:normal;font-weight:700;font-display:swap;src:url({}) format('woff2')}}",
            "@font-face{{font-family:'Atkinson Hyperlegible';font-style:italic;font-weight:400;font-display:swap;src:url({}) format('woff2')}}",
            "@font-face{{font-family:'Atkinson Hyperlegible';font-style:italic;font-weight:700;font-display:swap;src:url({}) format('woff2')}}",
            "@font-face{{font-family:'Material Icons';font-style:normal;font-weight:400;font-display:block;src:url({}) format('woff2')}}",
        ),
        FONT_ATK_400, FONT_ATK_700, FONT_ATK_400I, FONT_ATK_700I, FONT_MATERIAL
    );

    rsx! {
        style { dangerous_inner_html: font_face }
        document::Stylesheet { href: DX_THEME_CSS }
        document::Stylesheet { href: M3_THEME_CSS }
        document::Stylesheet { href: M3_TOKENS_CSS }
        document::Stylesheet { href: STYLE_CSS }
        Router::<Route> {}
        snackbar::Snackbar {}
    }
}
