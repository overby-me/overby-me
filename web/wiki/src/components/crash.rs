//! A hidden debug/QA route (`/crash`), ported from the old wiki's
//! `pages/crash.tsx`. The button triggers a Rust panic so the crash-reporting
//! pipeline can be verified end to end: `console_error_panic_hook` prints a
//! readable stack trace in the console, and in a `remote-logging` build the
//! panic hook in `logging.rs` ships a `PANIC:` entry to BetterStack (the old
//! wiki did the same via Bugfender). Not linked from any nav: reach it by
//! typing `/crash`.

use dioxus::prelude::*;

/// Panic on demand. A named `()`-returning fn (rather than an inline closure
/// body) so the diverging `panic!` does not trip edition-2024 never-type
/// fallback in the event-handler closure.
fn trigger_crash() {
    panic!("Triggered Crash");
}

#[component]
pub fn Crash() -> Element {
    rsx! {
        div { class: "card",
            div { class: "empty-state",
                div { class: "empty-state-orb",
                    span { class: "material-icons", "bug_report" }
                }
                p { class: "empty-state-body",
                    "Debug / QA: trigger a Rust panic to verify crash reporting "
                    "(a console stack trace, plus a BetterStack event in a remote-logging build)."
                }
                button {
                    class: "btn btn-primary mt-2",
                    onclick: move |_| trigger_crash(),
                    span { class: "material-icons", "warning" }
                    " Trigger Crash"
                }
            }
        }
    }
}
