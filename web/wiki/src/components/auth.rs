use dioxus::prelude::*;

use crate::i18n::t;
use crate::nhost;
use crate::route::Route;
use crate::session::{expires_at_from, save_session, Session, User, SESSION};

#[derive(Clone, PartialEq)]
enum AuthMode {
    Login,
    Register,
    ResetPassword,
    SetPassword,
}

/// Shortest value the sign-up schema accepts for a name or a password.
///
/// Checked here so a value below it is answered under the box it belongs to, in
/// the user's language, without a round trip. The service enforces it too, but
/// as `schema-validation-error` carrying "minimum string length is 3" and naming
/// no field, which is not something a form can point at.
const MIN_FIELD_LEN: usize = 3;

/// A localized message for an auth failure.
///
/// The service answers in English with a machine code beside it. The CODE is
/// what carries the meaning, so translate that and never show the service's own
/// sentence: an unmapped one used to reach the screen verbatim, which is how a
/// Danish user got "Password is too short" under a Danish label.
///
/// An unmapped code shows the app's generic failure line and logs the original,
/// so the wording stays localized and the detail is still recoverable.
fn auth_error_message(err: &nhost::NhostError) -> String {
    match err.error.as_deref() {
        Some("invalid-email") => t("auth.invalidEmail"),
        Some("email-already-in-use") => t("auth.emailAlreadyInUse"),
        Some("unverified-user") => t("auth.emailNotVerified"),
        Some("user-not-found") => t("auth.userNotFound"),
        Some("invalid-email-password") => t("auth.wrongCredentials"),
        Some("password-too-short") => t("auth.passwordTooShort"),
        Some("password-in-hibp") => t("auth.passwordCompromised"),
        Some("disabled-user") => t("auth.userDisabled"),
        // A value broke the request schema — a length or format rule. The
        // detail ("minimum string length is 3") names no field, so the message
        // has to ask the user to look rather than point.
        Some("schema-validation-error") => t("auth.invalidInput"),
        // A password-reset or verification link that has expired or been used.
        Some("unauthenticated-user") | Some("invalid-refresh-token") => t("auth.linkExpired"),
        _ => {
            log::error!("unmapped auth error: {err:?}");
            t("error.somethingWentWrong")
        }
    }
}

/// Whether a failure is about the password rather than the address, so its
/// message lands under the box it is actually about. Everything else a sign-up
/// or a reset rejects is about the address.
fn is_password_error(err: &nhost::NhostError) -> bool {
    if matches!(
        err.error.as_deref(),
        Some("password-too-short") | Some("password-in-hibp")
    ) {
        return true;
    }
    // A schema rejection sometimes names its field in the detail. That text is
    // the service's English and is never shown; it is only read here to decide
    // which box the localized message belongs under.
    matches!(err.error.as_deref(), Some("schema-validation-error"))
        && err
            .message
            .as_deref()
            .is_some_and(|m| m.to_ascii_lowercase().contains("password"))
}

/// Clear the OTHER field's error when it holds `shared`, the message a PAIRED
/// failure put on both boxes.
///
/// Two failures here blame a pair rather than a field: a rejected sign-in, which
/// cannot say which of email and password was wrong, and a mismatch, which is
/// the two password boxes disagreeing. Either way editing one box answers the
/// complaint, so the twin has to leave its error state too. On its own it went
/// on claiming the pair was wrong while the user was busy correcting it.
///
/// Matching on the message keeps this to the paired case: a field's own
/// validation error is a different string and stays that field's to clear.
fn clear_paired_error(mut other: Signal<String>, shared: &str) {
    // Compare first, then write: the read guard would still be alive inside an
    // `if *other.read() == ...` body and the write would panic on the borrow.
    let is_paired = *other.read() == shared;
    if is_paired {
        other.set(String::new());
    }
}

#[component]
fn AuthForm(mode: AuthMode) -> Element {
    let nav = use_navigator();
    let mut loading = use_signal(|| false);
    let mut name = use_signal(String::new);
    let mut email = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut password_repeat = use_signal(String::new);
    let mut error_name = use_signal(String::new);
    let mut error_email = use_signal(String::new);
    let mut error_password = use_signal(String::new);
    let mut error_password_repeat = use_signal(String::new);
    // Set when a sign-in fails because the account is unverified, so the login
    // screen can offer to re-send the verification email.
    let mut unverified = use_signal(|| false);

    let title = match mode {
        AuthMode::Login => t("auth.login"),
        AuthMode::Register => t("auth.register"),
        AuthMode::ResetPassword => t("auth.resetPassword"),
        AuthMode::SetPassword => t("auth.setPassword"),
    };

    let icon = match mode {
        AuthMode::Login => "login",
        AuthMode::Register => "person_add",
        AuthMode::ResetPassword => "mail",
        AuthMode::SetPassword => "lock",
    };

    let mode_clone = mode.clone();
    let on_submit = move |evt: FormEvent| {
        evt.prevent_default();
        let mode = mode_clone.clone();
        let nav = nav;
        spawn(async move {
            loading.set(true);

            match mode {
                AuthMode::Login => {
                    let em = email.read().clone();
                    let pw = password.read().clone();
                    if em.is_empty() {
                        error_email.set(t("auth.missingEmail"));
                        loading.set(false);
                        return;
                    }
                    if pw.is_empty() {
                        error_password.set(t("auth.missingPassword"));
                        loading.set(false);
                        return;
                    }
                    match nhost::sign_in(&em, &pw).await {
                        Ok(session) => {
                            let new_session = Session {
                                access_token_expires_at: expires_at_from(
                                    session.access_token_expires_in,
                                ),
                                user: session.user.map(|u| User {
                                    id: u.id,
                                    email: u.email.unwrap_or_default(),
                                    display_name: u.display_name.unwrap_or_default(),
                                    avatar_url: u.avatar_url.unwrap_or_default(),
                                }),
                                access_token: Some(session.access_token),
                                refresh_token: Some(session.refresh_token),
                                node_id: None,
                            };
                            save_session(&new_session);
                            *SESSION.write() = new_session;
                            // Refetch everything for the new session so no data
                            // from a previous one lingers (React clears its cache).
                            crate::session::bump_data_version();
                            nav.push(Route::Home { app: None });
                        }
                        Err(err) => match err.error.as_deref() {
                            Some("unverified-user") => {
                                error_email.set(t("auth.emailNotVerified"));
                                unverified.set(true);
                            }
                            // Not about the credentials typed, so they say so on
                            // their own rather than reddening both boxes.
                            Some("disabled-user") | Some("network_error") | Some("parse_error") => {
                                error_email.set(auth_error_message(&err));
                            }
                            // Anything else a sign-in can fail with is a rejected
                            // pair, and the service will not say which half was
                            // wrong, so both boxes carry the same message.
                            _ => {
                                error_email.set(t("auth.wrongCredentials"));
                                error_password.set(t("auth.wrongCredentials"));
                            }
                        },
                    }
                }
                AuthMode::Register => {
                    let nm = name.read().clone();
                    let em = email.read().clone();
                    let pw = password.read().clone();
                    let pw2 = password_repeat.read().clone();
                    if nm.is_empty() {
                        error_name.set(t("auth.missingName"));
                        loading.set(false);
                        return;
                    }
                    if nm.chars().count() < MIN_FIELD_LEN {
                        error_name.set(t("auth.nameTooShort"));
                        loading.set(false);
                        return;
                    }
                    if em.is_empty() {
                        error_email.set(t("auth.missingEmail"));
                        loading.set(false);
                        return;
                    }
                    if pw.is_empty() {
                        error_password.set(t("auth.missingPassword"));
                        loading.set(false);
                        return;
                    }
                    if pw.chars().count() < MIN_FIELD_LEN {
                        error_password.set(t("auth.passwordTooShort"));
                        loading.set(false);
                        return;
                    }
                    if pw2.is_empty() || pw != pw2 {
                        // The two boxes disagree, so neither is the wrong one:
                        // both carry it, and editing either clears both.
                        error_password.set(t("auth.passwordMismatch"));
                        error_password_repeat.set(t("auth.passwordMismatch"));
                        loading.set(false);
                        return;
                    }
                    match nhost::sign_up(&em, &pw, &nm).await {
                        Ok(()) => {
                            nav.push(Route::Unverified {});
                        }
                        Err(err) => {
                            // Under the box it is about: a rejected password
                            // is not a complaint about the address.
                            let msg = auth_error_message(&err);
                            if is_password_error(&err) {
                                error_password.set(msg);
                            } else {
                                error_email.set(msg);
                            }
                        }
                    }
                }
                AuthMode::ResetPassword => {
                    let em = email.read().clone();
                    if em.is_empty() {
                        error_email.set(t("auth.missingEmail"));
                        loading.set(false);
                        return;
                    }
                    match nhost::reset_password(&em).await {
                        Ok(()) => {
                            // Confirm the mail went out — do NOT jump to
                            // set-password. That screen changes the password of
                            // the CURRENT session, and asking for a reset does
                            // not create one; reaching it from here showed a
                            // form that could not work. The emailed link is what
                            // lands there, carrying the session with it.
                            nav.push(Route::CheckEmail {});
                        }
                        // This screen is the address alone: it renders no password
                        // box for a message to land under.
                        Err(err) => error_email.set(auth_error_message(&err)),
                    }
                }
                AuthMode::SetPassword => {
                    let pw = password.read().clone();
                    let pw2 = password_repeat.read().clone();
                    if pw.is_empty() {
                        error_password.set(t("auth.missingPassword"));
                        loading.set(false);
                        return;
                    }
                    if pw.chars().count() < MIN_FIELD_LEN {
                        error_password.set(t("auth.passwordTooShort"));
                        loading.set(false);
                        return;
                    }
                    if pw2.is_empty() || pw != pw2 {
                        // The two boxes disagree, so neither is the wrong one:
                        // both carry it, and editing either clears both.
                        error_password.set(t("auth.passwordMismatch"));
                        error_password_repeat.set(t("auth.passwordMismatch"));
                        loading.set(false);
                        return;
                    }
                    let token = SESSION.read().access_token.clone().unwrap_or_default();
                    match nhost::change_password(&token, &pw).await {
                        Ok(()) => {
                            nav.push(Route::Home { app: None });
                        }
                        Err(err) => {
                            error_password.set(auth_error_message(&err));
                        }
                    }
                }
            }

            loading.set(false);
        });
    };

    let has_errors = !error_name.read().is_empty()
        || !error_email.read().is_empty()
        || !error_password.read().is_empty()
        || !error_password_repeat.read().is_empty();

    rsx! {
        div { class: "auth-container",
            form { class: "auth-form", onsubmit: on_submit,
                // DESIGN: a hero icon badge (matching the home/profile heroes).
                div { class: "auth-hero-icon", span { class: "material-icons", "{icon}" } }
                h2 { class: "headline-small auth-title", "{title}" }

                // Name field (register only)
                if mode == AuthMode::Register {
                    div { class: if error_name.read().is_empty() { "text-field" } else { "text-field error" },
                        label { r#for: "auth-fullname", "{t(\"auth.fullName\")}" }
                        input {
                            id: "auth-fullname",
                            r#type: "text",
                            name: "fullname",
                            value: "{name}",
                            oninput: move |evt| {
                                name.set(evt.value());
                                if !evt.value().is_empty() {
                                    error_name.set(String::new());
                                }
                            },
                        }
                        if !error_name.read().is_empty() {
                            div { class: "helper-text", "{error_name}" }
                        }
                    }
                }

                // Email field (not for set-password)
                if mode != AuthMode::SetPassword {
                    div { class: if error_email.read().is_empty() { "text-field" } else { "text-field error" },
                        label { r#for: "auth-email", "{t(\"auth.email\")}" }
                        input {
                            id: "auth-email",
                            r#type: "email",
                            name: "email",
                            autocomplete: "username",
                            value: "{email}",
                            oninput: move |evt| {
                                email.set(evt.value());
                                if !evt.value().is_empty() {
                                    error_email.set(String::new());
                                }
                                // A wrong sign-in marked the password too.
                                clear_paired_error(error_password, &t("auth.wrongCredentials"));
                            },
                        }
                        if !error_email.read().is_empty() {
                            div { class: "helper-text", "{error_email}" }
                        }
                    }
                    // Offer to re-send the verification email when a sign-in was
                    // rejected because the account is not verified yet.
                    if mode == AuthMode::Login && *unverified.read() {
                        button {
                            r#type: "button",
                            class: "btn btn-text mt-1",
                            onclick: move |_| {
                                let em = email.read().trim().to_lowercase();
                                if em.is_empty() {
                                    return;
                                }
                                spawn(async move {
                                    match nhost::send_verification_email(&em).await {
                                        Ok(()) => crate::snackbar::show_snackbar(&t("auth.verificationResent")),
                                        Err(_) => crate::snackbar::show_snackbar(&t("error.somethingWentWrong")),
                                    }
                                });
                            },
                            "{t(\"auth.resendVerification\")}"
                        }
                    }
                }

                // Password field (not for reset-password)
                if mode != AuthMode::ResetPassword {
                    div { class: if error_password.read().is_empty() { "text-field" } else { "text-field error" },
                        label {
                            r#for: "auth-password",
                            if mode == AuthMode::SetPassword {
                                "{t(\"auth.newPassword\")}"
                            } else {
                                "{t(\"auth.password\")}"
                            }
                        }
                        div { class: "pw-field",
                        input {
                            id: "auth-password",
                            class: "pw-input",
                            r#type: "password",
                            name: "password",
                            autocomplete: "current-password",
                            value: "{password}",
                            oninput: move |evt| {
                                password.set(evt.value());
                                error_password.set(String::new());
                                // ...and the email, unless its error is its own
                                // (an unverified account keeps its message and
                                // the resend button that goes with it).
                                clear_paired_error(error_email, &t("auth.wrongCredentials"));
                                // ...and the repeat box, which a mismatch marked
                                // alongside this one.
                                clear_paired_error(
                                    error_password_repeat,
                                    &t("auth.passwordMismatch"),
                                );
                            },
                        }
                        super::widgets::PasswordDots { len: password.read().chars().count() }
                        }
                        if !error_password.read().is_empty() {
                            div { class: "helper-text", "{error_password}" }
                        }
                    }
                }

                // Repeat password (register and set-password)
                if mode == AuthMode::Register || mode == AuthMode::SetPassword {
                    div { class: if error_password_repeat.read().is_empty() { "text-field" } else { "text-field error" },
                        label { r#for: "auth-password-repeat", "{t(\"auth.repeatPassword\")}" }
                        div { class: "pw-field",
                        input {
                            id: "auth-password-repeat",
                            class: "pw-input",
                            r#type: "password",
                            name: "password-repeat",
                            value: "{password_repeat}",
                            oninput: move |evt| {
                                password_repeat.set(evt.value());
                                error_password_repeat.set(String::new());
                                // A mismatch marked the first box as well.
                                clear_paired_error(error_password, &t("auth.passwordMismatch"));
                            },
                        }
                        super::widgets::PasswordDots {
                            len: password_repeat.read().chars().count(),
                        }
                        }
                        if !error_password_repeat.read().is_empty() {
                            div { class: "helper-text", "{error_password_repeat}" }
                        }
                    }
                }

                // Submit button
                div { class: "btn-busy",
                    button {
                        class: "btn btn-primary btn-full",
                        r#type: "submit",
                        disabled: *loading.read() || has_errors,
                        "{title}"
                    }
                    if *loading.read() {
                        div { class: "btn-busy-spinner",
                            div { class: "spinner spinner-sm" }
                        }
                    }
                }

                // Extra buttons for login page
                if mode == AuthMode::Login {
                    button {
                        class: "btn btn-secondary btn-full",
                        r#type: "button",
                        onclick: move |_| { nav.push(Route::Register {}); },
                        "{t(\"auth.register\")}"
                    }
                    button {
                        class: "btn btn-secondary btn-full",
                        r#type: "button",
                        onclick: move |_| { nav.push(Route::ResetPassword {}); },
                        "{t(\"auth.resetPassword\")}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn Login() -> Element {
    rsx! { AuthForm { mode: AuthMode::Login } }
}

#[component]
pub fn Register() -> Element {
    rsx! { AuthForm { mode: AuthMode::Register } }
}

#[component]
pub fn ResetPassword() -> Element {
    rsx! { AuthForm { mode: AuthMode::ResetPassword } }
}

#[component]
pub fn SetPassword() -> Element {
    rsx! { AuthForm { mode: AuthMode::SetPassword } }
}

/// Shown after asking for a password-reset link: the mail is out, and the next
/// step is in the inbox. Mirrors [`Unverified`], the same shape of "we sent you
/// something, go and look" screen.
#[component]
pub fn CheckEmail() -> Element {
    rsx! {
        div { class: "auth-container",
            div { class: "auth-form",
                div { class: "auth-hero-icon", span { class: "material-icons", "mark_email_read" } }
                h2 { class: "headline-small auth-title", "{t(\"auth.checkEmail\")}" }
                p { class: "body-large", "{t(\"auth.passwordResetSent\")}" }
                p { class: "body-large", "{t(\"auth.useToResetPassword\")}" }
                p { class: "body-medium", "{t(\"auth.checkSpam\")}" }
            }
        }
    }
}

#[component]
pub fn Unverified() -> Element {
    rsx! {
        div { class: "auth-container",
            div { class: "auth-form",
                div { class: "auth-hero-icon", span { class: "material-icons", "mail" } }
                h2 { class: "headline-small auth-title", "{t(\"auth.verifyEmail\")}" }
                p { class: "body-large", "{t(\"auth.verificationEmailSent\")}" }
                p { class: "body-large", "{t(\"auth.useToActivate\")}" }
                p { class: "body-medium", "{t(\"auth.checkSpam\")}" }
            }
        }
    }
}
