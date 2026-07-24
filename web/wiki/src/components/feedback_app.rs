//! The feedback app (`/?app=feedback`): browse feedback submissions. A home-
//! context owner sees ALL feedback (with the submitter) and may delete items; a
//! plain member sees only their own. NOTE: the own-only filtering is COSMETIC —
//! the `nodes` select rule is open to any authenticated user, so a member could
//! still read others' feedback via a raw query. It's a low-sensitivity report
//! inbox, so this is a deliberate simplification (no restrictive select rule).
//! Feedback is composed from the user-menu dialog
//! ([`super::feedback::FeedbackDialog`]), which creates the `wiki/feedback` nodes.

use dioxus::prelude::*;

use crate::components::widgets::Dialog;
use crate::graphql::{self, FeedbackItem};
use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// The material icon + label key for a feedback kind.
fn kind_glyph(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "bug" => ("bug_report", "feedback.bug"),
        "feature" => ("lightbulb", "feedback.feature"),
        _ => ("chat", "feedback.other"),
    }
}

#[component]
pub fn FeedbackApp() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let my_id = session.read().user.as_ref().map(|u| u.id.clone());

    // Confirm-delete state: the id of the item awaiting confirmation.
    let mut confirm_delete = use_signal(|| None::<String>);

    // Whether the caller owns the home context (→ sees all feedback + can delete).
    let owner_token = session.read().access_token.clone();
    let owner_res = crate::use_data_resource!(|(owner_token)| async move {
        graphql::query_root_node(owner_token.as_deref())
            .await
            .ok()
            .flatten()
            .and_then(|n| n.is_context_owner)
            .unwrap_or(false)
    });
    let is_owner = (*owner_res.read()).unwrap_or(false);

    let feed_token = session.read().access_token.clone();
    let items_res = crate::use_data_resource!(|(feed_token)| async move {
        graphql::query_feedback(feed_token.as_deref())
            .await
            .unwrap_or_default()
    });
    let loading = items_res.read().is_none();
    let mut items = items_res.read().clone().unwrap_or_default();
    // Members see only their own (cosmetic — see the module doc); owners see all.
    if !is_owner {
        items.retain(|it| it.owner_id.is_some() && it.owner_id == my_id);
    }

    if !is_auth {
        return rsx! {
            div { class: "card",
                div { class: "empty-state",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "lock" }
                    }
                    p { class: "empty-state-body", "{t(\"node.documentUnavailable\")}" }
                }
            }
        };
    }

    let delete_item = move |id: String| {
        let token = session.read().access_token.clone();
        spawn(async move {
            match graphql::delete_node(token.as_deref(), &id).await {
                Ok(_) => crate::session::bump_data_version(),
                Err(e) => {
                    log::error!("delete feedback failed: {e}");
                    show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "feedback" } }
                h3 { class: "title-medium",
                    if is_owner { "{t(\"feedback.all\")}" } else { "{t(\"feedback.yours\")}" }
                }
            }
            div { class: "card-content",
                if loading {
                    div { class: "stack stack-h", style: "align-items: center; gap: 8px;",
                        div { class: "spinner" }
                    }
                } else if items.is_empty() {
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "feedback" }
                        }
                        p { class: "empty-state-body",
                            if is_owner { "{t(\"feedback.empty\")}" } else { "{t(\"feedback.emptyMine\")}" }
                        }
                    }
                } else {
                    div { class: "list",
                        for item in items.iter() {
                            FeedbackRow {
                                key: "{item.id}",
                                item: item.clone(),
                                show_owner: is_owner,
                                can_delete: is_owner,
                                on_delete: move |id| confirm_delete.set(Some(id)),
                            }
                        }
                    }
                }
            }
        }

        // Delete confirmation.
        Dialog {
            open: confirm_delete.read().is_some(),
            on_dismiss: move |_| confirm_delete.set(None),
            headline: t("common.delete"),
            icon: "delete".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| confirm_delete.set(None),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    onclick: move |_| {
                        if let Some(id) = confirm_delete.write().take() {
                            delete_item(id);
                        }
                    },
                    "{t(\"common.delete\")}"
                }
            },
            p { class: "body-medium", "{t(\"feedback.deleteConfirm\")}" }
        }
    }
}

/// One feedback submission row.
#[component]
fn FeedbackRow(
    item: FeedbackItem,
    show_owner: bool,
    can_delete: bool,
    on_delete: EventHandler<String>,
) -> Element {
    let (icon, label_key) = kind_glyph(&item.kind);
    let date: String = item.created_at.chars().take(10).collect();
    let screenshot = super::loader::use_file_object_url(item.image.clone().unwrap_or_default());

    rsx! {
        div { class: "feedback-item",
            div { class: "stack stack-h", style: "align-items: center; gap: 8px;",
                div { class: "avatar small", span { class: "material-icons", "{icon}" } }
                span { class: "chip", span { class: "material-icons", "{icon}" }
                    span { class: "chip-label", "{t(label_key)}" }
                }
                div { class: "flex-grow" }
                if !date.is_empty() {
                    span { class: "body-small text-muted", "{date}" }
                }
                if can_delete {
                    button {
                        class: "btn-icon",
                        title: "{t(\"common.delete\")}",
                        onclick: {
                            let id = item.id.clone();
                            move |_| on_delete.call(id.clone())
                        },
                        span { class: "material-icons", "delete" }
                    }
                }
            }
            p { class: "body-medium", style: "white-space: pre-wrap; margin-top: var(--md-sys-spacing-2);",
                "{item.message}"
            }
            if let Some(url) = screenshot {
                a { href: "{url}", target: "_blank", rel: "noopener",
                    img {
                        class: "zoomable",
                        src: "{url}",
                        alt: t("feedback.screenshot"),
                        loading: "lazy",
                        style: "margin-top: var(--md-sys-spacing-2);",
                    }
                }
            }
            div { class: "stack stack-h", style: "align-items: center; gap: 8px; margin-top: var(--md-sys-spacing-2);",
                if show_owner {
                    super::loader::UserPopover {
                        name: if item.owner_name.is_empty() { t("feedback.anonymous") } else { item.owner_name.clone() },
                        avatar_url: item.owner_avatar.clone(),
                        user_id: item.owner_id.clone(),
                        span { class: "chip",
                            span { class: "material-icons", "person" }
                            span { class: "chip-label",
                                "{t(\"feedback.submittedBy\")}: "
                                if item.owner_name.is_empty() { "{t(\"feedback.anonymous\")}" } else { "{item.owner_name}" }
                            }
                        }
                    }
                }
                if !item.path.is_empty() {
                    span { class: "body-small text-muted", "{item.path}" }
                }
            }
        }
    }
}
