//! The standard loading / error / empty (LEE) feedback states, so every async
//! surface renders the four states the same way. The data + loading states are
//! screen-specific (a screen picks its skeleton and renders its own rows); these
//! two cover the shared empty and error presentations. Domain-free: the caller
//! passes localized strings.
//!
//! The reference pattern for a `use_data_resource!` returning `Result<T, _>`:
//!
//! ```ignore
//! match &*resource.read() {
//!     Some(Ok(data)) if !data.is_empty() => rsx! { /* rows */ },
//!     Some(Ok(_))    => rsx! { div { class: "card", EmptyState { icon: "inbox", message: t("..."), small: true } } },
//!     Some(Err(e))   => { log::error!("{e}"); rsx! { div { class: "card accent-error", ErrorState { title: t("error.somethingWentWrong"), small: true } } } },
//!     None           => rsx! { Spinner {} },  // or a shape-matched skeleton
//! }
//! ```

use dioxus::prelude::*;

/// A standard empty-state block: an expressive orb + a message. Wrap it in a
/// `.card` for the carded form. Use for `Some(Ok(empty))`.
#[component]
pub fn EmptyState(icon: String, message: String, #[props(default)] small: bool) -> Element {
    rsx! {
        div { class: if small { "empty-state empty-state-sm" } else { "empty-state" },
            div { class: if small { "empty-state-orb empty-state-orb-sm" } else { "empty-state-orb" },
                span { class: "material-icons", "{icon}" }
            }
            p { class: "empty-state-body", "{message}" }
        }
    }
}

/// A standard error-state block: an error-tinted orb + a friendly title. Never
/// renders raw error text (log the detail separately). Wrap in
/// `.card.accent-error` for the carded form. Use for `Some(Err(_))`.
#[component]
pub fn ErrorState(title: String, #[props(default)] small: bool) -> Element {
    rsx! {
        div { class: if small { "empty-state empty-state-sm" } else { "empty-state" },
            div { class: if small { "empty-state-orb empty-state-orb-sm error-orb" } else { "empty-state-orb error-orb" },
                span { class: "material-icons", "error_outline" }
            }
            h3 { class: "empty-state-title", "{title}" }
        }
    }
}
