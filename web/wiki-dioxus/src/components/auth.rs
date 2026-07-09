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
                                }),
                                access_token: Some(session.access_token),
                                refresh_token: Some(session.refresh_token),
                                node_id: None,
                            };
                            save_session(&new_session);
                            *SESSION.write() = new_session;
                            nav.push(Route::HomeApp {});
                        }
                        Err(err) => {
                            if err.error.as_deref() == Some("unverified-user") {
                                error_email.set(t("auth.emailNotVerified"));
                            } else {
                                error_email.set(t("auth.wrongCredentials"));
                                error_password.set(t("auth.wrongCredentials"));
                            }
                        }
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
                    if pw2.is_empty() || pw != pw2 {
                        error_password_repeat.set(t("auth.passwordMismatch"));
                        loading.set(false);
                        return;
                    }
                    match nhost::sign_up(&em, &pw, &nm).await {
                        Ok(()) => {
                            nav.push(Route::Unverified {});
                        }
                        Err(err) => match err.error.as_deref() {
                            Some("invalid-email") => error_email.set(t("auth.invalidEmail")),
                            Some("email-already-in-use") => {
                                error_email.set(t("auth.emailAlreadyInUse"))
                            }
                            _ => error_email.set(err.to_string()),
                        },
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
                            nav.push(Route::SetPassword {});
                        }
                        Err(err) => match err.error.as_deref() {
                            Some("invalid-email") => error_email.set(t("auth.invalidEmail")),
                            Some("user-not-found") => error_email.set(t("auth.userNotFound")),
                            _ => error_email.set(err.to_string()),
                        },
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
                    if pw2.is_empty() || pw != pw2 {
                        error_password_repeat.set(t("auth.passwordMismatch"));
                        loading.set(false);
                        return;
                    }
                    let token = SESSION.read().access_token.clone().unwrap_or_default();
                    match nhost::change_password(&token, &pw).await {
                        Ok(()) => {
                            nav.push(Route::HomeApp {});
                        }
                        Err(err) => {
                            error_password.set(err.to_string());
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
                div { class: "avatar", span { class: "material-icons", "{icon}" } }
                h2 { class: "title-large", "{title}" }

                // Name field (register only)
                if mode == AuthMode::Register {
                    div { class: if error_name.read().is_empty() { "text-field" } else { "text-field error" },
                        label { "{t(\"auth.fullName\")}" }
                        input {
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
                        label { "{t(\"auth.email\")}" }
                        input {
                            r#type: "email",
                            name: "email",
                            autocomplete: "username",
                            value: "{email}",
                            oninput: move |evt| {
                                email.set(evt.value());
                                if !evt.value().is_empty() {
                                    error_email.set(String::new());
                                }
                            },
                        }
                        if !error_email.read().is_empty() {
                            div { class: "helper-text", "{error_email}" }
                        }
                    }
                }

                // Password field (not for reset-password)
                if mode != AuthMode::ResetPassword {
                    div { class: if error_password.read().is_empty() { "text-field" } else { "text-field error" },
                        label {
                            if mode == AuthMode::SetPassword {
                                "{t(\"auth.newPassword\")}"
                            } else {
                                "{t(\"auth.password\")}"
                            }
                        }
                        input {
                            r#type: "password",
                            name: "password",
                            autocomplete: "current-password",
                            value: "{password}",
                            oninput: move |evt| {
                                password.set(evt.value());
                                error_password.set(String::new());
                            },
                        }
                        if !error_password.read().is_empty() {
                            div { class: "helper-text", "{error_password}" }
                        }
                    }
                }

                // Repeat password (register and set-password)
                if mode == AuthMode::Register || mode == AuthMode::SetPassword {
                    div { class: if error_password_repeat.read().is_empty() { "text-field" } else { "text-field error" },
                        label { "{t(\"auth.repeatPassword\")}" }
                        input {
                            r#type: "password",
                            name: "password-repeat",
                            value: "{password_repeat}",
                            oninput: move |evt| {
                                password_repeat.set(evt.value());
                                error_password_repeat.set(String::new());
                            },
                        }
                        if !error_password_repeat.read().is_empty() {
                            div { class: "helper-text", "{error_password_repeat}" }
                        }
                    }
                }

                // Submit button
                div { style: "position: relative; width: 100%;",
                    button {
                        class: "btn btn-primary btn-full",
                        r#type: "submit",
                        disabled: *loading.read() || has_errors,
                        "{title}"
                    }
                    if *loading.read() {
                        div { style: "position: absolute; top: 50%; left: 50%; transform: translate(-50%, -50%);",
                            div { class: "spinner" }
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

#[component]
pub fn Unverified() -> Element {
    rsx! {
        div { class: "auth-container",
            div { class: "auth-form",
                div { class: "avatar", span { class: "material-icons", "mail" } }
                h2 { class: "title-large", "{t(\"auth.verifyEmail\")}" }
                p { class: "body-large", "{t(\"auth.verificationEmailSent\")}" }
                p { class: "body-large", "{t(\"auth.useToActivate\")}" }
                p { class: "body-medium", "{t(\"auth.checkSpam\")}" }
            }
        }
    }
}
