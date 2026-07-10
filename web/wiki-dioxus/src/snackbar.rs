use dioxus::prelude::*;

#[derive(Clone, Debug, PartialEq)]
pub struct SnackbarMessage {
    pub text: String,
    pub id: u64,
}

static SNACKBAR_COUNTER: GlobalSignal<u64> = Signal::global(|| 0);
pub static SNACKBAR: GlobalSignal<Option<SnackbarMessage>> = Signal::global(|| None);

/// Show a snackbar message that auto-dismisses after 3 seconds
pub fn show_snackbar(text: &str) {
    let id = *SNACKBAR_COUNTER.read() + 1;
    *SNACKBAR_COUNTER.write() = id;
    *SNACKBAR.write() = Some(SnackbarMessage {
        text: text.to_string(),
        id,
    });

    // Auto-dismiss after 3 seconds
    spawn(async move {
        gloo_timers::future::TimeoutFuture::new(3000).await;
        let current = SNACKBAR.read().clone();
        if let Some(msg) = &current {
            if msg.id == id {
                *SNACKBAR.write() = None;
            }
        }
    });
}

/// Snackbar component — render at the root level
#[component]
pub fn Snackbar() -> Element {
    let message = SNACKBAR.read().clone();

    match message {
        Some(msg) => {
            rsx! {
                // role="status" + aria-live so assistive tech announces the
                // message (it appears and auto-dismisses without focus moving).
                div {
                    class: "snackbar",
                    role: "status",
                    aria_live: "polite",
                    aria_atomic: "true",
                    "{msg.text}"
                }
            }
        }
        None => rsx! {},
    }
}
