use dioxus::prelude::*;

use super::*;
use crate::i18n::t;
use crate::route::Route;

/// Breadcrumb navigation based on the current route. Mirrors the old wiki: a row
/// of mime avatars (each path node); only the current node's name is shown, and
/// hovering a crumb reveals its name (the whole bar resets on mouse-leave). The
/// trail STARTS at the current context (the nearest group/event) rather than the
/// root, so it begins with the selected event/group. The open app is shown as a
/// badge on the current node's avatar.
#[component]
pub(super) fn Breadcrumbs() -> Element {
    let route = use_route::<Route>();
    let (segments, app) = match &route {
        Route::PathPage { segments, app } => (segments.clone(), app.clone()),
        _ => (vec![], None),
    };

    // Resolved once by `Layout`; read reactively so crumbs update on navigation.
    let crumbs = NAV_CRUMBS();
    let depth = CONTEXT_DEPTH();
    let total = segments.len();

    // Begin at the context (deepest group/event). With no context in the path
    // (e.g. the home route) fall back to showing Home plus the full path.
    let (show_home, start) = if depth >= 1 {
        (false, depth - 1)
    } else {
        (true, 0)
    };

    // The deepest crumb is open (its name shown) by default: the app view when one
    // is open — that is the current location — otherwise the last path node. Hover
    // to reveal any other crumb's name is done in pure CSS (`.crumb:hover`), so it
    // works in every browser without a JS reactivity round-trip.
    let last_id = if app.is_some() {
        total + 1
    } else if total > 0 {
        total
    } else {
        0
    };

    rsx! {
        div {
            class: "breadcrumbs",
            if show_home {
                BreadcrumbCrumb {
                    to: Route::Home { app: None },
                    mime: "app/home".to_string(),
                    name: t("common.home"),
                    ordinal: None,
                    open: last_id == 0,
                }
            }
            for i in start..total {
                {
                    let info = crumbs.get(i);
                    let name = info
                        .map(|c| c.name.clone())
                        .filter(|n| !n.is_empty())
                        .unwrap_or_else(|| segments[i].clone());
                    let mime = info
                        .map(|c| {
                            crate::components::loader::node_icon_mime_id(
                                c.mime_id.as_deref().unwrap_or(""),
                                c.data.as_ref().map(|d| &d.0),
                            )
                        })
                        .unwrap_or_default();
                    let ordinal = info.and_then(|c| c.ordinal);
                    rsx! {
                        BreadcrumbCrumb {
                            key: "{i}",
                            to: Route::PathPage { segments: segments[..=i].to_vec(), app: None },
                            mime,
                            name,
                            ordinal,
                            open: last_id == i + 1,
                        }
                    }
                }
            }
            // The open app (vote / speak / members / editor / …) as its own trailing
            // crumb — a labelled, clickable step rather than a badge on the node.
            if let Some(a) = app.clone() {
                BreadcrumbCrumb {
                    to: Route::PathPage { segments: segments.clone(), app: Some(a.clone()) },
                    mime: format!("app/{a}"),
                    name: app_crumb_label(&a),
                    ordinal: None,
                    open: last_id == total + 1,
                    app_crumb: true,
                }
            }
        }
    }
}

/// Human label for an `?app=` view, shown as the trailing breadcrumb. Mirrors the
/// app-rail labels; hidden/URL-only apps fall back to their key.
pub(super) fn app_crumb_label(app: &str) -> String {
    match app {
        "folder" => t("mime.folder"),
        "speak" => t("mime.speak"),
        "vote" => t("mime.vote"),
        "member" => t("common.members"),
        "editor" => t("mime.editor"),
        "sort" => t("mime.sort"),
        "screen" => t("mime.screen"),
        "follow" => t("mime.follow"),
        "admin" => t("console.title"),
        other => other.to_string(),
    }
}

/// A single breadcrumb: an always-visible mime avatar and a name that expands on
/// hover (horizontal collapse), matching the old wiki's `BreadcrumbsLink`.
#[component]
pub(super) fn BreadcrumbCrumb(
    to: Route,
    mime: String,
    name: String,
    ordinal: Option<usize>,
    /// Whether this is the deepest (current) crumb, whose name is shown by default;
    /// every other crumb reveals its name on hover via CSS (`.crumb:hover`).
    open: bool,
    /// The open-app crumb: a different axis (a view of the node, not a path step),
    /// so it is tinted with the accent instead of the node/path colour.
    #[props(default)]
    app_crumb: bool,
) -> Element {
    rsx! {
        div {
            class: if app_crumb { "crumb app-crumb" } else { "crumb" },
            // Clicking a crumb (navigating to an ancestor, or re-clicking the
            // current node) scrolls the content back to the top.
            onclick: move |_| {
                if let Some(win) = web_sys::window() {
                    win.scroll_to_with_x_and_y(0.0, 0.0);
                }
            },
            Link { to, class: "crumb-link",
                div { class: "avatar small crumb-avatar",
                    {crate::components::loader::node_avatar(&mime, &name, ordinal)}
                }
                span {
                    class: if open { "crumb-name open" } else { "crumb-name" },
                    "{name}"
                }
            }
        }
    }
}

/// App rail — vertical icon navigation for large screens
/// The context apps for the current route (home, folder, and for authed users
/// speak/vote/member), each as `(mime, label, route, is-active)`. Shared by the
/// desktop rail and the mobile app bar, mirroring React's `useApps`. Empty off a
/// context (`segments` empty), which hides both nav surfaces.
pub(super) fn context_apps(
    route: &Route,
    is_auth: bool,
) -> Vec<(&'static str, String, Route, bool)> {
    let segments: Vec<String> = match route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    if segments.is_empty() {
        // Home: the folder/speak/vote/member apps all need a context, but show
        // the Home destination (active) so the rail isn't blank on the landing
        // page.
        return vec![(
            "app/home",
            t("common.home"),
            Route::Home { app: None },
            true,
        )];
    }
    let current_app = match route {
        Route::PathPage { app, .. } => app.clone(),
        _ => None,
    };
    // The app is part of the route's query, so these navigate client-side and the
    // resolver swaps the view without a reload.
    let ctx_path = context_path(&segments);

    let mut apps: Vec<(&str, String, Route, bool)> = vec![
        (
            "app/home",
            t("common.home"),
            Route::Home { app: None },
            false,
        ),
        (
            "app/folder",
            t("mime.folder"),
            Route::PathPage {
                segments: ctx_path.clone(),
                app: None,
            },
            // The editor / sort sub-apps operate on folder content, so the folder
            // rail item stays highlighted while they are open.
            current_app.is_none()
                || matches!(current_app.as_deref(), Some("editor") | Some("sort")),
        ),
    ];
    if is_auth {
        for (app, label) in [
            ("speak", t("mime.speak")),
            ("vote", t("mime.vote")),
            // Members: React only surfaces this to owners, but MemberApp gates
            // its admin controls itself, so the entry is safe for any authed user.
            ("member", t("common.members")),
        ] {
            apps.push((
                match app {
                    "speak" => "app/speak",
                    "vote" => "app/vote",
                    _ => "app/member",
                },
                label,
                Route::PathPage {
                    segments: ctx_path.clone(),
                    app: Some(app.to_string()),
                },
                current_app.as_deref() == Some(app),
            ));
        }
        // Follow the room: a member's device tracks the context's active node
        // (what the chair projected) and shows it live, to read/vote in step with
        // the room. Sits after the per-item apps as a live-session destination.
        apps.push((
            "app/follow",
            t("mime.follow"),
            Route::PathPage {
                segments: ctx_path.clone(),
                app: Some("follow".to_string()),
            },
            current_app.as_deref() == Some("follow"),
        ));
        // The chair's run-the-meeting console (agenda + project + results). Owner
        // actions gate themselves inside; members see the agenda/results read-only.
        apps.push((
            "app/admin",
            t("console.title"),
            Route::PathPage {
                segments: ctx_path.clone(),
                app: Some("admin".to_string()),
            },
            current_app.as_deref() == Some("admin"),
        ));
        // The other apps (screen, admin, program, graph, social, map, profile,
        // perm, parent) are still reachable via their `?app=` URL but hidden
        // from these nav surfaces until they are ready to show.
    }
    apps
}
