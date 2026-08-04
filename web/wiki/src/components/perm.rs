use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::session::use_session;

use super::loader::icon_el;
use crate::components::ui::switch::Switch;

/// PermApp — a context's permission rows (`?app=perm`): which mime types each
/// role may insert / select / delete, and, for an owner, the one switch that
/// matters to people rather than to the model: whether this place is open to
/// everyone.
///
/// The table is still read-only. Editing individual rows is a different tool for
/// a different question; "can a stranger read this" is the one an owner actually
/// asks, and until now the only way to answer it was to edit the database.
#[component]
pub fn PermApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let context_id = node
        .context_id
        .clone()
        .map(|c| c.0)
        .unwrap_or_else(|| node.id.0.clone());
    let is_owner = node.is_context_owner.unwrap_or(false);
    // Held in signals so the closure below captures only `Copy` values and is
    // itself `Copy`: the switch and the confirm dialog both call it.
    let ctx_sig = use_signal(|| context_id.clone());
    // The mime the public rows must cover for the place itself to be openable.
    let mime_sig = use_signal(|| {
        node.mime_id
            .clone()
            .unwrap_or_else(|| "wiki/group".to_string())
    });

    let perms = crate::use_data_resource!(|(context_id, access_token)| async move {
        graphql::query_permissions(access_token.as_deref(), &context_id).await
    });
    // "No content" and "the query failed" are opposite answers on a screen about
    // who is allowed to do what, and they used to look the same.
    let load = perms.read().clone();
    let failed = matches!(load, Some(Err(_)));
    if let Some(Err(e)) = &load {
        crate::errors::log_handled("permissions load failed", e);
    }
    let mut perms = load.and_then(|r| r.ok()).unwrap_or_default();
    perms.sort_by(|a, b| {
        (a.role.as_str(), a.mime_id.as_deref().unwrap_or(""))
            .cmp(&(b.role.as_str(), b.mime_id.as_deref().unwrap_or("")))
    });

    // Whether this place is currently open, read off the same rows the table
    // shows, so the switch and the table can never disagree.
    let is_public = perms
        .iter()
        .any(|p| p.role == "public" && p.select && p.active);
    // Turning it ON asks first. A page that has been read cannot be unread, so
    // this is the kind of thing to be sure about; turning it off does not ask,
    // because closing a door is not the risky direction.
    let mut confirm_open = use_signal(|| false);
    let mut saving = use_signal(|| false);

    let mut apply = move |on: bool| {
        let ctx = ctx_sig.read().clone();
        let mime = mime_sig.read().clone();
        let token = session.read().access_token.clone();
        saving.set(true);
        spawn(async move {
            let result = graphql::set_context_public(token.as_deref(), &ctx, &mime, on).await;
            saving.set(false);
            match result {
                Ok(()) => {
                    crate::snackbar::show_snackbar(&t(if on {
                        "perm.nowPublic"
                    } else {
                        "perm.nowPrivate"
                    }));
                    crate::session::bump_data_version();
                }
                Err(e) => {
                    crate::errors::log_handled("set context public", &e);
                    crate::snackbar::show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "lock" } }
                h3 { class: "title-medium", "{node.name}" }
            }
            // The owner's one real control. Above the table, because it is the
            // question being asked; the table below is the answer in detail.
            if is_owner {
                div { class: "card-content",
                    div { class: "list-item switch-row",
                        span { class: "material-icons",
                            {if is_public { "public" } else { "lock" }}
                        }
                        div { class: "list-item-text",
                            div { class: "list-item-primary", "{t(\"perm.openToAll\")}" }
                            div { class: "list-item-secondary",
                                {if is_public { t("perm.openToAllOn") } else { t("perm.openToAllOff") }}
                            }
                        }
                        Switch {
                            checked: Some(is_public),
                            disabled: saving(),
                            aria_label: t("perm.openToAll"),
                            on_checked_change: move |on: bool| {
                                if on {
                                    confirm_open.set(true);
                                } else {
                                    apply(false);
                                }
                            },
                        }
                    }
                }
                super::widgets::Dialog {
                    open: confirm_open(),
                    on_dismiss: move |_| confirm_open.set(false),
                    headline: t("perm.openToAll"),
                    icon: "public".to_string(),
                    actions: rsx! {
                        button {
                            class: "btn btn-outlined",
                            onclick: move |_| confirm_open.set(false),
                            "{t(\"common.cancel\")}"
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: move |_| {
                                confirm_open.set(false);
                                apply(true);
                            },
                            "{t(\"perm.openConfirm\")}"
                        }
                    },
                    p { class: "body-medium", "{t(\"perm.openWarning\")}" }
                }
            }
            if failed {
                super::widgets::ErrorState {
                    title: t("error.couldNotLoad"),
                    small: true,
                    on_retry: move |_| crate::session::bump_data_version(),
                }
            } else if perms.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"common.noContent\")}"
                    }
                }
            } else {
                super::widgets::DataTable {
                    columns: vec![
                        t("perm.type"),
                        t("perm.role"),
                        t("perm.insert"),
                        t("perm.select"),
                        t("perm.delete"),
                        t("perm.active"),
                    ],
                    for perm in perms.iter() {
                        {
                            let mime = perm.mime_id.clone().unwrap_or_default();
                            rsx! {
                                tr { key: "{perm.id.0}",
                                    td {
                                        span { class: "m3-cell-icon",
                                            {icon_el(&mime)}
                                            span { "{mime}" }
                                        }
                                    }
                                    td { "{perm.role}" }
                                    td { {flag_cell(perm.insert)} }
                                    td { {flag_cell(perm.select)} }
                                    td { {flag_cell(perm.delete)} }
                                    td { {flag_cell(perm.active)} }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// A boolean permission cell: a filled primary check when granted, else a muted
/// minus glyph. The glyph is exposed to assistive tech as "granted"/"denied" so
/// screen readers don't announce the raw icon ligature ("check"/"remove").
fn flag_cell(on: bool) -> Element {
    if on {
        rsx! {
            span {
                class: "material-icons m3-flag-on",
                role: "img",
                aria_label: t("perm.granted"),
                "check"
            }
        }
    } else {
        rsx! {
            span {
                class: "material-icons m3-flag-off",
                role: "img",
                aria_label: t("perm.denied"),
                "remove"
            }
        }
    }
}
