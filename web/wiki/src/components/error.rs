//! A hidden debug/QA route (`/error`, formerly `/crash`) for verifying the
//! error-reporting pipeline end to end. Each button triggers a different kind of
//! failure so you can confirm it reaches the console and — in a `remote-logging`
//! build — Better Stack (via the backend `/log` proxy). It triggers a Rust panic
//! (the panic hook ships a `PANIC:` entry), a GraphQL error (logged centrally in
//! `graphql::execute*`), the generic "something went wrong" toast (logged at the
//! snackbar seam), or a direct `log::error!` / `log::warn!`. Not linked from any
//! nav: reach it by typing `/error`.

use dioxus::prelude::*;

use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// Panic on demand. A named `()`-returning fn (rather than an inline closure
/// body) so the diverging `panic!` does not trip edition-2024 never-type
/// fallback in the event-handler closure.
fn trigger_panic() {
    panic!("Triggered test panic from /error");
}

#[component]
pub fn ErrorPage() -> Element {
    let session = use_session();

    rsx! {
        div { class: "card",
            div { class: "empty-state",
                div { class: "empty-state-orb",
                    span { class: "material-icons", "bug_report" }
                }
                p { class: "empty-state-body",
                    "Debug / QA: trigger each kind of error to verify reporting "
                    "(a console message, plus a Better Stack event in a remote-logging build)."
                }
                div {
                    class: "stack stack-v mt-2",
                    style: "gap: var(--md-sys-spacing-2); align-items: stretch; width: 100%; max-width: 320px;",

                    // A Rust panic → console stack trace + a shipped `PANIC:` entry.
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| trigger_panic(),
                        span { class: "material-icons", "warning" }
                        " Trigger panic"
                    }

                    // A real GraphQL failure → logged centrally in `execute_raw`,
                    // then the generic toast (also logged) surfaces to the user.
                    button {
                        class: "btn btn-secondary",
                        onclick: move |_| {
                            let token = session.read().access_token.clone();
                            spawn(async move {
                                let _ = crate::graphql::execute_raw(
                                        token.as_deref(),
                                        "query { __definitely_not_a_field }",
                                    )
                                    .await;
                                show_snackbar(&t("error.somethingWentWrong"));
                            });
                        },
                        span { class: "material-icons", "cloud_off" }
                        " Trigger GraphQL error"
                    }

                    // The generic "something went wrong" toast on its own.
                    button {
                        class: "btn btn-secondary",
                        onclick: move |_| show_snackbar(&t("error.somethingWentWrong")),
                        span { class: "material-icons", "error_outline" }
                        " Show \"something went wrong\""
                    }

                    // A direct error / warning log line (no user-facing effect).
                    button {
                        class: "btn btn-outlined",
                        onclick: move |_| {
                            log::error!("Triggered test error from /error");
                            show_snackbar("Logged a test error");
                        },
                        span { class: "material-icons", "report" }
                        " Log an error"
                    }
                    button {
                        class: "btn btn-outlined",
                        onclick: move |_| {
                            log::warn!("Triggered test warning from /error");
                            show_snackbar("Logged a test warning");
                        },
                        span { class: "material-icons", "info" }
                        " Log a warning"
                    }
                }
            }
        }
    }
}
