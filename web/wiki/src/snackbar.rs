use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct SnackbarMessage {
    pub text: String,
    pub id: u64,
}

static SNACKBAR_COUNTER: GlobalSignal<u64> = Signal::global(|| 0);
/// The stack of visible snackbars (newest last), capped at [`MAX_SNACK`].
pub static SNACKBAR: GlobalSignal<Vec<SnackbarMessage>> = Signal::global(Vec::new);

/// How many snackbars stack at once before the oldest is dropped (Notistack's
/// `maxSnack`).
const MAX_SNACK: usize = 3;

/// Show a snackbar that auto-dismisses after 3 seconds. Stacks up to
/// [`MAX_SNACK`] at once, and skips a message identical to the newest one still
/// showing (Notistack's `preventDuplicate`).
pub fn show_snackbar(text: &str) {
    // A generic-failure toast shown to the user is also recorded for the logger
    // (shipped in remote-logging builds), so "something went wrong" leaves a trace
    // even at the many call sites that discard the underlying error. The specific
    // detail, when there is one, is logged separately (e.g. the GraphQL layer).
    if text == crate::i18n::t("error.somethingWentWrong") {
        log::warn!("error toast shown to user: {text}");
    }
    // preventDuplicate: don't queue the same text twice in a row.
    if SNACKBAR.read().last().is_some_and(|m| m.text == text) {
        return;
    }
    let id = *SNACKBAR_COUNTER.read() + 1;
    *SNACKBAR_COUNTER.write() = id;
    {
        let mut q = SNACKBAR.write();
        q.push(SnackbarMessage {
            text: text.to_string(),
            id,
        });
        // maxSnack: drop the oldest beyond the cap.
        while q.len() > MAX_SNACK {
            q.remove(0);
        }
    }

    // Auto-dismiss this specific message after 3 seconds.
    spawn(async move {
        gloo_timers::future::TimeoutFuture::new(3000).await;
        SNACKBAR.write().retain(|m| m.id != id);
    });
}

/// Snackbar component — render at the root level. Renders the whole stack.
#[component]
pub fn Snackbar() -> Element {
    let messages = SNACKBAR.read().clone();
    if messages.is_empty() {
        return rsx! {};
    }
    rsx! {
        div { class: "snackbar-stack",
            for msg in messages.iter() {
                // role="status" + aria-live so assistive tech announces the
                // message (it appears and auto-dismisses without focus moving).
                div {
                    class: "snackbar",
                    key: "{msg.id}",
                    role: "status",
                    aria_live: "polite",
                    aria_atomic: "true",
                    "{msg.text}"
                }
            }
        }
    }
}
