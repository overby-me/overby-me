//! In-app feedback / bug report / feature request. The dialog is opened from
//! the user menu; on submit it POSTs to the backend `/feedback` endpoint, which
//! ships the report to the team's observability sink (BetterStack, the same
//! sink app errors go to). The current path, app version and user agent are
//! attached automatically, and the backend captures the sender when signed in.
//!
//! The dialog's open state is a GLOBAL signal: the trigger is a user-menu item
//! inside the drawer pane, but that pane is transformed (slide animation) and
//! overflow-clipped, which would trap and clip the dialog's fixed scrim. The
//! dialog itself is therefore rendered at the app-shell root (see
//! `layout::Layout`), the same escape hatch the TOC popover uses.

use dioxus::prelude::*;

use crate::components::widgets::Dialog;
use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// Matches the backend's message cap, so a long paste is trimmed before send.
const FEEDBACK_MAXLEN: usize = 4000;

/// Gates the send-feedback user-menu item. Enabled: the backend `/feedback`
/// endpoint ships to a configured Better Stack sink (the container carries
/// `BETTERSTACK_SOURCE_TOKEN` + `BETTERSTACK_INGEST_HOST`). Set to `false` if the
/// sink is ever removed, so reports aren't silently dropped (see
/// `backend/src/feedback.rs`).
pub const FEEDBACK_ENABLED: bool = true;

/// Open state for [`FeedbackDialog`]. Global so the trigger (a user-menu item
/// inside the transformed drawer pane) and the dialog (at the app-shell root,
/// where its fixed scrim can cover the viewport) can live in different subtrees.
pub static FEEDBACK_OPEN: GlobalSignal<bool> = Signal::global(|| false);

/// The feedback dialog. Opened by setting [`FEEDBACK_OPEN`]; rendered once at
/// the app-shell root.
#[component]
pub fn FeedbackDialog() -> Element {
    let session = use_session();
    let mut kind = use_signal(|| "bug".to_string());
    let mut message = use_signal(String::new);
    let mut busy = use_signal(|| false);

    let submit = move |_| {
        let msg = message.read().trim().to_string();
        if msg.is_empty() || *busy.read() {
            return;
        }
        let k = kind.read().clone();
        let token = session.read().access_token.clone();
        let path = web_sys::window()
            .and_then(|w| w.location().pathname().ok())
            .unwrap_or_default();
        let ua = web_sys::window()
            .map(|w| w.navigator().user_agent().unwrap_or_default())
            .unwrap_or_default();
        busy.set(true);
        spawn(async move {
            let res = crate::backend_api::submit_feedback(
                token.as_deref(),
                &k,
                &msg,
                &path,
                env!("CARGO_PKG_VERSION"),
                &ua,
            )
            .await;
            busy.set(false);
            match res {
                Ok(()) => {
                    *FEEDBACK_OPEN.write() = false;
                    message.set(String::new());
                    show_snackbar(&t("feedback.sent"));
                }
                Err(e) => {
                    log::error!("feedback submit failed: {e}");
                    show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    // (value, material-icon, label) for the type toggle.
    let types = [
        ("bug", "bug_report", t("feedback.bug")),
        ("feature", "lightbulb", t("feedback.feature")),
        ("other", "chat", t("feedback.other")),
    ];

    rsx! {
        Dialog {
            open: FEEDBACK_OPEN(),
            on_dismiss: move |_| *FEEDBACK_OPEN.write() = false,
            headline: t("feedback.title"),
            icon: "feedback".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| *FEEDBACK_OPEN.write() = false,
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    disabled: message.read().trim().is_empty() || *busy.read(),
                    onclick: submit,
                    "{t(\"feedback.send\")}"
                }
            },
            // Type selector (bug / feature / other): a labelled toggle row.
            div { class: "stack stack-h feedback-types", style: "margin-bottom: var(--md-sys-spacing-3);",
                for (val , icon , label) in types {
                    button {
                        key: "{val}",
                        r#type: "button",
                        class: if kind.read().as_str() == val { "btn btn-primary" } else { "btn btn-outlined" },
                        onclick: move |_| kind.set(val.to_string()),
                        span { class: "material-icons", "{icon}" }
                        span { class: "feedback-type-label", "{label}" }
                    }
                }
            }
            div { class: "text-field",
                label { "{t(\"feedback.messageLabel\")}" }
                textarea {
                    rows: "5",
                    maxlength: "{FEEDBACK_MAXLEN}",
                    placeholder: t("feedback.placeholder"),
                    value: "{message}",
                    oninput: move |e| message.set(e.value()),
                }
            }
        }
    }
}
