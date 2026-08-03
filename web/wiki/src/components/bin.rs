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
//!
//! Anyone signed in can put back what they deleted or what was theirs; only an
//! owner of the context can empty it for good. Recovering is the reason the bin
//! exists and costs nothing if it was a mistake, so it needs no special standing;
//! the irreversible half does.

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
    let here = node.id.0.clone();
    let items = crate::use_data_resource!(|(ctx, here, token)| async move {
        graphql::query_deleted(token.as_deref(), &ctx, &here).await
    });
    // A bin that failed to load must not read as a bin with nothing in it: the
    // reader came here to find something they deleted, and "nothing deleted
    // here" is the one answer that would send them away.
    let load = items.read().clone();
    let failed = matches!(load, Some(Err(_)));
    if let Some(Err(e)) = &load {
        crate::errors::log_handled("bin load failed", e);
    }
    let loading = load.is_none();
    let items: Vec<graphql::DeletedNodeFields> = load.and_then(|r| r.ok()).unwrap_or_default();
    // Emptying the bin is the one action here that cannot be taken back, so it
    // belongs to whoever answers for the context rather than to whoever deleted.
    let can_purge = node.is_context_owner.unwrap_or(false) || node.is_owner.unwrap_or(false);

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "restore_from_trash" } }
                h3 { class: "title-medium", "{t(\"bin.title\")}" }
            }
            if loading {
                super::widgets::Spinner {}
            } else if failed {
                super::widgets::ErrorState {
                    title: t("error.couldNotLoad"),
                    small: true,
                    on_retry: move |_| crate::session::bump_data_version(),
                }
            } else if items.is_empty() {
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
                        BinRow {
                            key: "{item.id.as_ref().map(|i| i.0.clone()).unwrap_or_default()}",
                            item: item.clone(),
                            can_purge,
                        }
                    }
                }
            }
        }
    }
}

/// One binned item: what it was, where it was, when it went, and a way back —
/// plus, for an owner, the way out that is not a way back.
#[component]
fn BinRow(item: graphql::DeletedNodeFields, can_purge: bool) -> Element {
    let session = use_session();
    let mut busy = use_signal(|| false);
    let mut purge_confirm = use_signal(|| false);
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
            if can_purge {
                button {
                    class: "btn-icon",
                    disabled: busy(),
                    aria_label: t("bin.purge"),
                    title: t("bin.purge"),
                    onclick: move |_| purge_confirm.set(true),
                    span { class: "material-icons", "delete_forever" }
                }
            }
        }
        if can_purge {
            // Named for what it does. "Delete" would read as the same delete that
            // put this row here, which is exactly the thing it is not.
            super::widgets::Dialog {
                open: purge_confirm(),
                on_dismiss: move |_| purge_confirm.set(false),
                headline: t("bin.purgeHeadline"),
                icon: "delete_forever".to_string(),
                actions: rsx! {
                    button {
                        class: "btn btn-outlined",
                        onclick: move |_| purge_confirm.set(false),
                        "{t(\"common.cancel\")}"
                    }
                    button {
                        class: "btn btn-primary",
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
                                    match graphql::purge_node(token.as_deref(), &id).await {
                                        Ok(n) if n > 0 => {
                                            purge_confirm.set(false);
                                            crate::snackbar::show_snackbar(&t("bin.purged"));
                                            crate::session::bump_data_version();
                                        }
                                        other => {
                                            busy.set(false);
                                            log::error!("purge failed: {other:?}");
                                            crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                                        }
                                    }
                                });
                            }
                        },
                        if busy() {
                            div { class: "spinner spinner-xs" }
                        }
                        "{t(\"bin.purge\")}"
                    }
                },
                p { class: "body-medium", "{item.name.clone().unwrap_or_default()}" }
                p { class: "body-medium text-muted", "{t(\"bin.purgeWarning\")}" }
            }
        }
    }
}
