//! The bin: what was deleted in this context, and the way back.
//!
//! Deleting content stamps it rather than removing it (`graphql::bin_node`), and
//! the row-level rules hide anything stamped from every client, so a binned node
//! is gone from the app in every way that matters while still being there. This
//! is the one view that can see into that, through a database view whose own
//! permission limits it to people who own the context.
//!
//! Only the tops are listed: deleting a folder stamps everything under it, but
//! what the reader asked for was the folder, and that is what they get back.

use dioxus::prelude::*;

use crate::graphql;
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::session::use_session;

use super::loader::{icon_el, relative_time};

/// BinApp — a context's deleted items (`?app=bin`), owner-only.
#[component]
pub fn BinApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());

    let ctx = context_id.clone();
    let items = crate::use_data_resource!(|(ctx, token)| async move {
        graphql::query_deleted(token.as_deref(), &ctx)
            .await
            .unwrap_or_default()
    });
    let items = items.read().clone().unwrap_or_default();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "restore_from_trash" } }
                h3 { class: "title-medium", "{t(\"bin.title\")}" }
            }
            if items.is_empty() {
                div { class: "empty-state empty-state-sm",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "restore_from_trash" }
                    }
                    p { class: "empty-state-body", "{t(\"bin.empty\")}" }
                }
            } else {
                // Deleting is recoverable but not free: the list says what will
                // come back and where it was, so restoring is a decision rather
                // than a guess.
                div { class: "card-content",
                    p { class: "body-medium text-muted", "{t(\"bin.explain\")}" }
                }
                div { class: "list",
                    for item in items.iter() {
                        BinRow { key: "{item.id.as_ref().map(|i| i.0.clone()).unwrap_or_default()}", item: item.clone() }
                    }
                }
            }
        }
    }
}

/// One binned item: what it was, where it was, when it went, and a way back.
#[component]
fn BinRow(item: graphql::DeletedNodeFields) -> Element {
    let session = use_session();
    let mut busy = use_signal(|| false);
    let id = item.id.as_ref().map(|i| i.0.clone()).unwrap_or_default();
    let when = item
        .deleted_at
        .as_ref()
        .map(|t| relative_time(&t.0))
        .unwrap_or_default();
    // The path it came from, minus its own key: where it will go back to.
    let whence = item
        .path
        .as_deref()
        .and_then(|p| p.rsplit_once('/'))
        .map(|(parent, _)| parent.replace('/', " / "))
        .unwrap_or_default();

    rsx! {
        div { class: "list-item",
            div { class: "avatar small",
                {icon_el(item.mime_id.as_deref().unwrap_or("wiki/document"))}
            }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{item.name.clone().unwrap_or_default()}" }
                div { class: "list-item-secondary",
                    if !whence.is_empty() {
                        "{whence}"
                    }
                    if !whence.is_empty() && !when.is_empty() {
                        " · "
                    }
                    "{when}"
                }
            }
            button {
                class: "btn btn-tonal btn-sm",
                disabled: busy(),
                onclick: {
                    let id = id.clone();
                    move |_| {
                        if busy() {
                            return;
                        }
                        let id = id.clone();
                        let token = session.read().access_token.clone();
                        busy.set(true);
                        spawn(async move {
                            match graphql::restore_node(token.as_deref(), &id).await {
                                Ok(n) if n > 0 => {
                                    crate::snackbar::show_snackbar(&t("bin.restored"));
                                    crate::session::bump_data_version();
                                }
                                other => {
                                    busy.set(false);
                                    log::error!("restore failed: {other:?}");
                                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                }
                            }
                        });
                    }
                },
                span { class: "material-icons", "restore" }
                "{t(\"bin.restore\")}"
            }
        }
    }
}
