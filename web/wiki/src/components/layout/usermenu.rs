use dioxus::prelude::*;

use crate::components::ui::switch::Switch;
use crate::i18n::{t, Lang, LANG};
use crate::route::Route;
use crate::session::{save_session, use_session, SESSION};
use crate::theme::{apply_theme, use_theme, ThemeMode, THEME};

/// Account menu, shown at the bottom of the navigation drawer (the tree pane on
/// medium+, the modal drawer on compact). The trigger is an avatar + name row;
/// clicking it opens the account popover (theme, language, sign out). Owns its
/// own open state.
#[component]
pub(super) fn UserMenu() -> Element {
    let session = use_session();
    let nav = use_navigator();
    let is_auth = session.read().is_authenticated();
    let theme = use_theme();
    let mut menu_open = use_signal(|| false);

    let initial = session
        .read()
        .user
        .as_ref()
        .map(|u| u.display_name.chars().next().unwrap_or('?').to_string())
        .unwrap_or_else(|| "?".to_string());
    // The user's avatar image (e.g. their linked Bluesky picture), if any.
    let avatar_url = session
        .read()
        .user
        .as_ref()
        .map(|u| u.avatar_url.clone())
        .unwrap_or_default();

    let dark = *theme.read() == ThemeMode::Dark;
    let display_name = session
        .read()
        .user
        .as_ref()
        .map(|u| u.display_name.clone())
        .unwrap_or_default();
    let email = session
        .read()
        .user
        .as_ref()
        .map(|u| u.email.clone())
        .unwrap_or_default();
    // Your own id, so the identity card can open your profile at /profile/:id.
    let my_id = session.read().user.as_ref().map(|u| u.id.clone());

    rsx! {
        div { class: "user-menu in-drawer",
            // Account row: avatar + name. Opens the account popover upward.
            // stop_propagation so the click does not also dismiss the drawer.
            button {
                class: "drawer-account-trigger state-layer",
                aria_label: "{t(\"layout.userMenu\")}",
                aria_haspopup: "menu",
                aria_expanded: "{menu_open()}",
                onclick: move |evt| {
                    evt.stop_propagation();
                    let v = menu_open();
                    menu_open.set(!v);
                },
                onkeydown: move |evt| {
                    if evt.key() == Key::Escape {
                        menu_open.set(false);
                    }
                },
                if is_auth {
                    span { class: "avatar small secondary",
                        {crate::components::loader::user_avatar(&avatar_url, rsx! { "{initial}" })}
                    }
                } else {
                    span { class: "avatar small", span { class: "material-icons", "person" } }
                }
                span { class: "drawer-account-name",
                    if is_auth {
                        "{display_name}"
                    } else {
                        "{t(\"layout.account\")}"
                    }
                }
            }
            if menu_open() {
                // Full-viewport click-catcher so a click anywhere else closes it.
                div { class: "menu-backdrop", onclick: move |_| menu_open.set(false) }
                div { class: "user-menu-dropdown",
                    // Signed-in identity header (the old wiki had no user card in
                    // the sidebar; this belongs with the account menu instead).
                    if is_auth {
                        // The identity card IS the link to your profile — the one
                        // route for everyone, your own id included.
                        button {
                            class: "user-menu-header",
                            onclick: {
                                let my_id = my_id.clone();
                                move |_| {
                                    menu_open.set(false);
                                    if let Some(id) = my_id.clone() {
                                        nav.push(Route::UserProfile { id });
                                    }
                                }
                            },
                            span { class: "avatar secondary",
                                {crate::components::loader::user_avatar(&avatar_url, rsx! { "{initial}" })}
                            }
                            div { class: "user-menu-identity",
                                div { class: "user-menu-name", "{display_name}" }
                                div { class: "user-menu-email", "{email}" }
                            }
                            span { class: "material-icons user-menu-header-chevron", "chevron_right" }
                        }
                    }
                    // Dark-mode toggle, as an accessible on/off switch (the menu
                    // stays open so the flip is visible).
                    div { class: "list-item switch-row",
                        span { class: "material-icons",
                            {if dark { "dark_mode" } else { "light_mode" }}
                        }
                        span { class: "switch-row-label", "{t(\"layout.darkMode\")}" }
                        Switch {
                            checked: Some(dark),
                            aria_label: t("layout.darkMode"),
                            on_checked_change: move |on: bool| {
                                let new_theme = if on { ThemeMode::Dark } else { ThemeMode::Light };
                                apply_theme(&new_theme);
                                crate::theme::save_theme(&new_theme);
                                *THEME.write() = new_theme;
                            },
                        }
                    }
                    // DESIGN (functional): compact / comfortable UI density.
                    div { class: "list-item switch-row",
                        span { class: "material-icons", "density_medium" }
                        span { class: "switch-row-label", "{t(\"layout.compactDensity\")}" }
                        Switch {
                            checked: Some(crate::density::COMPACT_DENSITY()),
                            aria_label: t("layout.compactDensity"),
                            on_checked_change: move |on: bool| {
                                crate::density::set_compact(on);
                            },
                        }
                    }
                    // Theme colours: pick the M3 primary + accent seeds. Changing
                    // either regenerates the tonal scheme and re-skins the whole
                    // app at runtime (see crate::theme::apply_seeds).
                    div { class: "menu-color-section",
                        span { class: "menu-color-title", "{t(\"layout.themeColor\")}" }
                        // Five preset swatches; the sixth circle in each row is the
                        // freeform custom picker. The first swatch is the brand
                        // default (selecting it clears the override).
                        crate::components::widgets::ColorPicker {
                            label: t("layout.primaryColor"),
                            value: crate::theme::effective_primary(),
                            custom_title: t("layout.customColor"),
                            swatches: vec![
                                crate::theme::BRAND_PRIMARY.to_string(),
                                "#1565C0".to_string(),
                                "#6750A4".to_string(),
                                "#00796B".to_string(),
                                "#C62828".to_string(),
                            ],
                            on_change: move |hex: String| crate::theme::set_primary_seed(hex),
                        }
                        crate::components::widgets::ColorPicker {
                            label: t("layout.accentColor"),
                            value: crate::theme::effective_accent(),
                            custom_title: t("layout.customColor"),
                            swatches: vec![
                                crate::theme::BRAND_ACCENT.to_string(),
                                "#7B1FA2".to_string(),
                                "#0097A7".to_string(),
                                "#F9A825".to_string(),
                                "#2E7D32".to_string(),
                            ],
                            on_change: move |hex: String| crate::theme::set_accent_seed(hex),
                        }
                    }
                    // Language toggle
                    button {
                        class: "list-item",
                        onclick: move |_| {
                            let new_lang = match *LANG.read() {
                                Lang::En => Lang::Da,
                                Lang::Da => Lang::En,
                            };
                            crate::i18n::apply_lang(&new_lang);
                            *LANG.write() = new_lang;
                            menu_open.set(false);
                        },
                        span { class: "material-icons", "language" }
                        {match *LANG.read() { Lang::En => " Dansk", Lang::Da => " English" }}
                    }
                    // Send feedback / report a bug (available signed in or out).
                    // The dialog itself renders at the app-shell root (see
                    // `feedback::FEEDBACK_OPEN`) — inside this drawer pane its
                    // fixed scrim would be trapped by the pane's transform.
                    if crate::components::feedback::FEEDBACK_ENABLED {
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                *crate::components::feedback::FEEDBACK_OPEN.write() = true;
                            },
                            span { class: "material-icons", "feedback" }
                            " {t(\"feedback.menu\")}"
                        }
                        // Browse feedback: everyone sees their own submissions; home
                        // context owners see all (enforced server-side).
                        if is_auth {
                            button {
                                class: "list-item",
                                onclick: move |_| {
                                    menu_open.set(false);
                                    nav.push(Route::Home { app: Some("feedback".to_string()) });
                                },
                                span { class: "material-icons", "forum" }
                                " {t(\"feedback.view\")}"
                            }
                        }
                    }
                    if is_auth {
                        // No separate "Profile" row: the identity card at the top
                        // of this menu is the way to your profile, and two links to
                        // one page is one too many.
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                nav.push(Route::SetPassword {});
                            },
                            span { class: "material-icons", "lock" }
                            " {t(\"auth.setPassword\")}"
                        }
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                crate::nhost::sign_out();
                                *SESSION.write() = Default::default();
                                save_session(&Default::default());
                                // Drop cached data + the folder paste clipboard so
                                // nothing from the old session lingers (React clears
                                // the GraphQL cache here).
                                crate::components::folder::clear_selection();
                                crate::session::bump_data_version();
                                nav.push(Route::Home { app: None });
                            },
                            span { class: "material-icons", "logout" }
                            " {t(\"auth.logout\")}"
                        }
                    } else {
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                nav.push(Route::Login {});
                            },
                            span { class: "material-icons", "login" }
                            " {t(\"common.logIn\")}"
                        }
                        button {
                            class: "list-item",
                            onclick: move |_| {
                                menu_open.set(false);
                                nav.push(Route::Register {});
                            },
                            span { class: "material-icons", "person_add" }
                            " {t(\"auth.register\")}"
                        }
                    }
                }
            }
        }
    }
}
