use crate::model;
use dioxus::prelude::*;

use super::*;
use crate::graphql::{self};
use crate::i18n::t;
use crate::route::Route;
use crate::session::use_session;

/// Drawer content — shows navigation tree
#[component]
pub(super) fn DrawerContent() -> Element {
    let session = use_session();
    let route = use_route::<Route>();
    let segments: Vec<String> = match &route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let is_auth = session.read().is_authenticated();

    // The drawer's top entry switches from Home to the current context (the
    // nearest group/event, linking to its root) once inside one — mirroring the
    // old wiki's drawer, whose title switched Home → the selected context.
    let ctx_path = context_path(&segments);
    let crumbs = NAV_CRUMBS();
    let ctx_crumb = crumbs.get(ctx_path.len().saturating_sub(1));
    let ctx_name = ctx_crumb
        .map(|c| c.name.clone())
        .filter(|n| !n.is_empty())
        .unwrap_or_else(|| ctx_path.last().cloned().unwrap_or_default());
    let ctx_mime = ctx_crumb
        .and_then(|c| c.mime_id.clone())
        .unwrap_or_default();

    rsx! {
        div { class: "drawer-content",
            div { class: "drawer-scroll",
            div { class: "list",
                if segments.is_empty() {
                    // At the home route: Home, styled as a bar like the top panel.
                    Link {
                        to: Route::Home { app: None },
                        class: "bar drawer-context-bar",
                        div { class: "avatar small", span { class: "material-icons", "home" } }
                        span { class: "drawer-context-name", "{t(\"common.home\")}" }
                    }
                } else {
                    // Inside a context: the current context, styled as a bar like
                    // the top panel (the old wiki's drawer Bar/Title), linking to
                    // its root.
                    Link {
                        to: Route::PathPage { segments: ctx_path.clone(), app: None },
                        class: "bar drawer-context-bar",
                        div { class: "avatar small", {crate::components::loader::icon_el(&ctx_mime)} }
                        span { class: "drawer-context-name", "{ctx_name}" }
                    }
                }
            }

            // In-context navigation: at the home route show the groups/events
            // home list; once inside a context show its lazy child tree (the
            // React app's MenuList).
            if is_auth {
                if segments.is_empty() {
                    HomeList {}
                } else {
                    // Key on the context so switching contexts remounts and
                    // re-resolves (use_resource does not re-run for prop changes).
                    MenuList {
                        key: "{segments.first().cloned().unwrap_or_default()}",
                        segments: segments.clone(),
                    }
                }
            }
            }
            // Account menu, pinned at the bottom of the drawer.
            UserMenu {}
        }
    }
}

/// MenuList: the in-context drawer tree. Resolves the current context (the
/// nearest group/event, which may be nested below the top path segment), then
/// renders its children lazily and expandably, mirroring the React
/// `MenuList`/`DrawerList`/`DrawerElement` trio.
#[component]
pub(super) fn MenuList(segments: Vec<String>) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    // Root the tree at the context (deepest group/event) rather than the first
    // segment, so a nested event shows its own contents.
    let ctx_path = context_path(&segments);

    // Re-resolve reactively when the context path changes. Deliberately a plain
    // `use_resource` (not `use_data_resource!`): this only maps the path to the
    // context node id, which is stable across data changes, and refetching it on
    // every refresh would briefly blank and remount the whole drawer tree. The
    // drawer's actual contents refresh via `DrawerLevel` instead.
    let cpath = ctx_path.clone();
    let context = use_resource(use_reactive!(|(cpath, access_token)| async move {
        graphql::resolve_path(access_token.as_deref(), &cpath)
            .await
            .ok()
            .flatten()
            .map(|n| n.id.0)
    }));

    let ctx = context.read().clone();
    match ctx {
        Some(Some(context_id)) => rsx! {
            div { class: "list mt-1",
                DrawerLevel {
                    parent_id: context_id,
                    path_prefix: ctx_path.clone(),
                    current_path: segments.clone(),
                    depth: 0,
                }
            }
        },
        Some(None) => rsx! {},
        None => rsx! {
            p { class: "body-medium list-subheader", "…" }
        },
    }
}

/// One lazily-loaded level of the drawer tree: the visible children of
/// `parent_id`, ordered like the folder view.
#[component]
pub(super) fn DrawerLevel(
    parent_id: String,
    path_prefix: Vec<String>,
    current_path: Vec<String>,
    depth: usize,
) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let parent = parent_id.clone();

    let children = crate::use_data_resource!(|(parent, access_token, user_id)| async move {
        let Some(user_id) = user_id else {
            return Vec::new();
        };
        graphql::query_drawer_children(access_token.as_deref(), &parent, &user_id)
            .await
            .unwrap_or_default()
    });

    let items = children.read().clone();
    match items {
        Some(items) => {
            let ordinals = crate::components::loader::sibling_ordinals(&items);
            rsx! {
                for (child , ordinal) in items.iter().zip(ordinals) {
                    DrawerNodeItem {
                        key: "{child.id.0}",
                        node: child.clone(),
                        path_prefix: path_prefix.clone(),
                        current_path: current_path.clone(),
                        depth,
                        ordinal,
                    }
                }
            }
        }
        None => rsx! {},
    }
}

/// A single row in the drawer tree: icon, name, "not submitted" badge, and an
/// expander that lazily reveals the node's own children. Rows on the current
/// path start expanded and the active node is highlighted.
#[component]
pub(super) fn DrawerNodeItem(
    node: model::DrawerChildFields,
    path_prefix: Vec<String>,
    current_path: Vec<String>,
    depth: usize,
    ordinal: Option<usize>,
) -> Element {
    let nav = use_navigator();

    let mut full_path = path_prefix.clone();
    full_path.push(node.key.clone());

    // This row is on the active path if its full path is a prefix of the URL.
    let on_path = full_path.len() <= current_path.len()
        && full_path
            .iter()
            .zip(current_path.iter())
            .all(|(a, b)| a == b);
    let selected = full_path == current_path;

    let mime_id = node.mime_id.clone().unwrap_or_default();
    // Show the expander only when the node actually has children the user can
    // see (per-row `children_aggregate` count), not merely because its mime type
    // *could* have children. Mirrors the React DrawerElement gate.
    let expandable = node.has_children();
    let node_name = node.name.clone();

    // Auto-expand ancestors of the current node; let the user toggle the rest.
    let mut expanded = use_signal(|| on_path && !selected);
    // Re-expand when a later navigation brings this node onto the active path
    // (the initial value above only applies on first mount).
    use_effect(use_reactive!(|(on_path, selected)| {
        if on_path && !selected {
            expanded.set(true);
        }
    }));

    let node_id = node.id.0.clone();
    // Indent by depth from the spacing scale's base (so a global density change
    // retunes it) plus a fixed per-level step, rather than a hard-coded pixel sum.
    let indent =
        format!("padding-left: calc(var(--md-sys-spacing-3) + {depth} * var(--nav-indent-step));");
    let nav_path = full_path.clone();

    rsx! {
        div {
            // The selected node gets the full active indicator; its ancestors on
            // the active path get a subtler tint (M3-style emphasis levels).
            class: if selected {
                "list-item selected"
            } else if on_path {
                "list-item ancestor"
            } else {
                "list-item"
            },
            style: "{indent}",
            onclick: move |_| {
                nav.push(Route::PathPage {
                    segments: nav_path.clone(),
                    app: None,
                });
            },
            crate::components::loader::NodeAvatar {
                mime: crate::components::loader::node_icon_mime_id(&mime_id, node.data.as_ref().map(|d| &d.0)),
                name: node_name.clone(),
                ordinal,
                mutable: node.mutable,
                small: true,
                // The drawer tree reads as a plain icon list (like the old wiki),
                // not a column of coloured avatar circles.
                bare: true,
            }
            div { class: "list-item-text",
                div { class: "list-item-primary", "{node.name}" }
            }
            if expandable {
                button {
                    class: "btn-icon list-item-trailing",
                    aria_label: "{t(\"common.expand\")}",
                    onclick: move |evt| {
                        evt.stop_propagation();
                        let now = *expanded.read();
                        expanded.set(!now);
                    },
                    if *expanded.read() {
                        span { class: "material-icons", "expand_more" }
                    } else {
                        span { class: "material-icons", "chevron_right" }
                    }
                }
            }
        }
        if expandable && *expanded.read() {
            DrawerLevel {
                parent_id: node_id,
                path_prefix: full_path.clone(),
                current_path: current_path.clone(),
                depth: depth + 1,
            }
        }
    }
}
