//! The sign-in screens on the AppView, where an account is an atproto one.
//!
//! There is one thing to type: a handle. The browser is sent to the account's
//! own provider to approve the sign-in and comes back with a one-time code
//! (`#code=`), which [`finish_sign_in`] spends on a session. The wiki never
//! sees a password, so the interim's register, reset and set-password screens
//! have nothing to do here: they keep their routes and say where that happens.

use dioxus::prelude::*;

use crate::i18n::t;
use crate::route::Route;
use crate::session::{expires_at_from, save_session, Session, User, SESSION};

/// Where an account is made, for someone who has none. Bluesky runs the
/// largest provider; any atproto account signs in here all the same.
const MAKE_AN_ACCOUNT: &str = "https://bsky.app";

/// Put the reader back where they were before they signed in: they came to
/// sign in so that they could see THAT page.
fn back_to_where_they_were() {
    let nav = navigator();
    match crate::nav_memory::way_back().and_then(|url| url.parse::<Route>().ok()) {
        Some(back) => nav.push(back),
        None => nav.push(Route::Home { app: None }),
    };
}

thread_local! {
    /// The code a sign-in came back with, from [`capture_returned_code`] until
    /// [`finish_sign_in`] spends it.
    static RETURNED: std::cell::RefCell<Option<String>> = const { std::cell::RefCell::new(None) };
}

/// Called from `main`, BEFORE the router mounts: the router rewrites the address
/// as it starts and drops the fragment, and the code with it, which left a
/// person who had just signed in looking at the sign-in form again.
pub fn capture_returned_code() {
    RETURNED.with(|kept| *kept.borrow_mut() = returned_code());
}

/// The one-time code a sign-in came back with, taken out of the address bar so
/// that it is not left in the history to be spent again by a reload.
fn returned_code() -> Option<String> {
    let win = web_sys::window()?;
    let location = win.location();
    let code = location
        .hash()
        .ok()?
        .strip_prefix("#code=")
        .map(str::to_string)?;
    let here = format!("{}{}", location.pathname().ok()?, location.search().ok()?);
    if let Ok(history) = win.history() {
        let _ = history.replace_state_with_url(&wasm_bindgen::JsValue::NULL, "", Some(&here));
    }
    Some(code)
}

/// Finish a sign-in that the account's provider has just sent back here. Run
/// once at startup; does nothing on an ordinary page load.
pub async fn finish_sign_in() {
    let Some(code) = RETURNED.with(|kept| kept.borrow_mut().take()) else {
        return;
    };
    match crate::nhost::sign_in_with_code(&code).await {
        Ok(new) => {
            let session = Session {
                access_token_expires_at: expires_at_from(new.access_token_expires_in),
                access_token: Some(new.access_token),
                refresh_token: Some(new.refresh_token),
                user: new.user.map(|user| User {
                    id: user.id,
                    email: user.email.unwrap_or_default(),
                    display_name: user.display_name.unwrap_or_default(),
                    avatar_url: user.avatar_url.unwrap_or_default(),
                }),
                node_id: None,
            };
            *SESSION.write() = session.clone();
            save_session(&session);
            crate::session::bump_data_version();
            back_to_where_they_were();
        }
        Err(e) => {
            crate::errors::log_handled("sign-in code was not accepted", &e);
            crate::snackbar::show_snackbar(&t("auth.signInFailed"));
        }
    }
}

#[component]
fn HandleForm(title: String, icon: String, note: String) -> Element {
    let mut handle = use_signal(String::new);
    let mut error = use_signal(String::new);
    let mut leaving = use_signal(|| false);

    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        // A pasted `@alice.example` is the handle `alice.example`.
        let typed = handle.read().trim().trim_start_matches('@').to_string();
        if typed.is_empty() {
            error.set(t("auth.missingHandle"));
            return;
        }
        leaving.set(true);
        if let Some(win) = web_sys::window() {
            let _ = win.location().set_href(&crate::graphql::login_url(&typed));
        }
    };

    rsx! {
        div { class: "auth-container",
            form { class: "auth-form", onsubmit: on_submit,
                div { class: "auth-hero-icon", span { class: "material-icons", "{icon}" } }
                h2 { class: "headline-small auth-title", "{title}" }
                p { class: "body-medium text-muted", "{note}" }
                div { class: if error.read().is_empty() { "text-field" } else { "text-field error" },
                    label { r#for: "auth-handle", "{t(\"auth.handle\")}" }
                    input {
                        id: "auth-handle",
                        r#type: "text",
                        name: "handle",
                        placeholder: "alice.bsky.social",
                        autocomplete: "username",
                        autocapitalize: "none",
                        spellcheck: "false",
                        value: "{handle}",
                        oninput: move |evt| {
                            handle.set(evt.value());
                            error.set(String::new());
                        },
                    }
                    if !error.read().is_empty() {
                        div { class: "helper-text", "{error}" }
                    }
                }
                div { class: "btn-busy",
                    button {
                        class: "btn btn-primary btn-full",
                        r#type: "submit",
                        disabled: *leaving.read(),
                        "{t(\"auth.login\")}"
                    }
                    if *leaving.read() {
                        div { class: "btn-busy-spinner", div { class: "spinner spinner-sm" } }
                    }
                }
                a {
                    class: "btn btn-secondary btn-full",
                    href: MAKE_AN_ACCOUNT,
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "{t(\"auth.createAccount\")}"
                }
            }
        }
    }
}

#[component]
pub fn Login() -> Element {
    rsx! {
        HandleForm { title: t("auth.login"), icon: "login", note: t("auth.handleHint") }
    }
}

#[component]
pub fn Register() -> Element {
    rsx! {
        HandleForm { title: t("auth.register"), icon: "person_add", note: t("auth.noAccountYet") }
    }
}

/// A password is the provider's to reset, so every route that was about one
/// says so, and offers the sign-in.
#[component]
fn PasswordIsElsewhere() -> Element {
    rsx! {
        HandleForm { title: t("auth.login"), icon: "lock", note: t("auth.passwordAtProvider") }
    }
}

#[component]
pub fn ResetPassword() -> Element {
    rsx! { PasswordIsElsewhere {} }
}

#[component]
pub fn SetPassword() -> Element {
    rsx! { PasswordIsElsewhere {} }
}

#[component]
pub fn CheckEmail() -> Element {
    rsx! { PasswordIsElsewhere {} }
}

#[component]
pub fn Unverified() -> Element {
    rsx! { PasswordIsElsewhere {} }
}
