mod components;
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

const STYLE_CSS: Asset = asset!("/assets/style.css");

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
    });

    rsx! {
        document::Stylesheet { href: STYLE_CSS }
        Router::<Route> {}
        snackbar::Snackbar {}
    }
}
