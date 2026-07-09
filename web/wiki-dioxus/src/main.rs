mod components;
#[allow(dead_code)]
mod export;
#[allow(dead_code)]
mod graphql;
#[allow(dead_code)]
mod i18n;
#[allow(dead_code)]
mod nhost;
mod route;
#[allow(dead_code)]
mod session;
pub mod snackbar;
#[allow(dead_code)]
mod subscription;
#[allow(dead_code)]
mod theme;

use dioxus::prelude::*;
use route::Route;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::JsCast;

const STYLE_CSS: Asset = asset!("/assets/style.css");
// The dioxus-components (shadcn-style) theme tokens, loaded before our own CSS
// so app styles can build on / override them as screens migrate to primitives.
const DX_THEME_CSS: Asset = asset!("/assets/dx-components-theme.css");

fn main() {
    // Print real panic messages (with a JS stack trace) to the console instead
    // of a bare `unreachable executed` wasm trap — the single highest-value
    // change for debugging authenticated-load traps in Servo.
    console_error_panic_hook::set_once();
    wasm_logger::init(wasm_logger::Config::default());
    log::info!("RadikalWiki starting...");

    dioxus::launch(App);
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
        document::Stylesheet { href: STYLE_CSS }
        Router::<Route> {}
        snackbar::Snackbar {}
    }
}
