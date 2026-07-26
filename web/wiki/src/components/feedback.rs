//! In-app feedback / bug report / feature request. The dialog is opened from the
//! user menu; on submit it creates a `wiki/feedback` node under the root node (in
//! the root context), owned by the submitter. A signed-in member may post; the
//! server-side `nodes` select rule then limits reads (home-context owners see all
//! feedback, members only their own). Submissions are browsable in the feedback
//! app ([`super::feedback_app::FeedbackApp`], `/?app=feedback`). The current path,
//! app version and user agent are attached automatically, and an optional
//! screenshot is uploaded to storage and referenced by `data.image`.
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

/// Cap on an attached screenshot, so a huge paste doesn't stall the upload.
const MAX_SCREENSHOT_BYTES: usize = 8 * 1024 * 1024;

/// Gates the send-feedback user-menu item. Enabled now that feedback is stored
/// in the database (a `wiki/feedback` node) and viewable in the feedback app.
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
    // Optional screenshot: the uploaded file's id + display name, and an in-flight
    // flag while it streams to storage. Mirrors the editor's cover-image uploader.
    let mut image_id = use_signal(|| Option::<String>::None);
    let mut image_name = use_signal(String::new);
    let mut image_uploading = use_signal(|| false);

    let on_pick_image = move |evt: FormEvent| {
        let files = evt.files();
        let Some(fd) = files.into_iter().next() else {
            return;
        };
        let name = fd.name();
        let ctype = fd.content_type().unwrap_or_default();
        let token = session.read().access_token.clone();
        image_uploading.set(true);
        spawn(async move {
            match fd.read_bytes().await {
                Ok(bytes) => {
                    if bytes.len() > MAX_SCREENSHOT_BYTES {
                        show_snackbar(&t("editor.imageTooLarge"));
                    } else {
                        match crate::nhost::upload_file(
                            token.as_deref(),
                            bytes.to_vec(),
                            &name,
                            &ctype,
                        )
                        .await
                        {
                            Ok(up) => {
                                image_id.set(Some(up.id));
                                image_name.set(name);
                            }
                            Err(e) => {
                                log::error!("feedback screenshot upload failed: {e}");
                                show_snackbar(&t("error.somethingWentWrong"));
                            }
                        }
                    }
                }
                Err(_) => show_snackbar(&t("error.somethingWentWrong")),
            }
            image_uploading.set(false);
        });
    };

    let submit = move |_| {
        let msg = message.read().trim().to_string();
        if msg.is_empty() || *busy.read() || *image_uploading.read() {
            return;
        }
        let k = kind.read().clone();
        let token = session.read().access_token.clone();
        let image = image_id.read().clone();
        let path = web_sys::window()
            .and_then(|w| w.location().pathname().ok())
            .unwrap_or_default();
        let ua = web_sys::window()
            .map(|w| w.navigator().user_agent().unwrap_or_default())
            .unwrap_or_default();
        busy.set(true);
        spawn(async move {
            let res = crate::graphql::insert_feedback(
                token.as_deref(),
                &k,
                &msg,
                image.as_deref(),
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
                    image_id.set(None);
                    image_name.set(String::new());
                    crate::session::bump_data_version();
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
                    disabled: message.read().trim().is_empty() || *busy.read() || *image_uploading.read(),
                    onclick: submit,
                    "{t(\"feedback.send\")}"
                }
            },
            // Type selector (bug / feature / other): a labelled toggle row.
            div { class: "stack stack-h feedback-types",
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
            // Optional screenshot.
            div { class: "mt-2",
                div { class: "file-upload-label", "{t(\"feedback.screenshot\")}" }
                label { class: "file-upload",
                    input {
                        r#type: "file",
                        accept: "image/*",
                        class: "file-upload-input",
                        onchange: on_pick_image,
                    }
                    span { class: "material-icons", "image" }
                    span { class: "file-upload-text", "{t(\"feedback.addScreenshot\")}" }
                }
                if *image_uploading.read() {
                    div {
                        class: "stack stack-h mt-1",
                        div { class: "spinner spinner-sm" }
                        span { class: "body-small text-muted", "{t(\"feedback.addScreenshot\")}\u{2026}" }
                    }
                } else if !image_name.read().is_empty() {
                    div {
                        class: "file-upload-done",
                        span { class: "material-icons", "check_circle" }
                        span { class: "flex-grow", "{image_name}" }
                        button {
                            class: "btn btn-text",
                            onclick: move |_| {
                                image_id.set(None);
                                image_name.set(String::new());
                            },
                            "{t(\"feedback.removeScreenshot\")}"
                        }
                    }
                }
            }
        }
    }
}
