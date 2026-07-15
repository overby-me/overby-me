use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use super::*;
use crate::i18n::t;
use crate::route::Route;

/// A heading found on the page for the table of contents: (element id, text,
/// level 1–6).
type Heading = (String, String, u8);

/// Scan the content pane for every heading (h1–h6) — document headers AND the
/// section/card headers rendered by components (amendments, polls, comments, …).
/// Ensures each has an id (assigning one when missing) so the TOC can scroll to
/// it, and returns them in document order.
fn collect_headings() -> Vec<Heading> {
    let mut out = Vec::new();
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return out;
    };
    let sel = "#main-content h1, #main-content h2, #main-content h3, \
        #main-content h4, #main-content h5, #main-content h6";
    let Ok(nodes) = doc.query_selector_all(sel) else {
        return out;
    };
    for i in 0..nodes.length() {
        let Some(el) = nodes
            .item(i)
            .and_then(|n| n.dyn_into::<web_sys::Element>().ok())
        else {
            continue;
        };
        let text = el.text_content().unwrap_or_default().trim().to_string();
        if text.is_empty() {
            continue;
        }
        let level = el
            .tag_name()
            .to_lowercase()
            .strip_prefix('h')
            .and_then(|n| n.parse::<u8>().ok())
            .unwrap_or(3);
        let mut id = el.id();
        if id.is_empty() {
            id = format!("toc-h{i}");
            el.set_id(&id);
        }
        out.push((id, text, level));
    }
    out
}

/// Smooth-scroll the element with `id` into view (for a TOC click).
fn scroll_to_id(id: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    {
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_behavior(web_sys::ScrollBehavior::Smooth);
        opts.set_block(web_sys::ScrollLogicalPosition::Start);
        el.scroll_into_view_with_scroll_into_view_options(&opts);
    }
}

/// Global TOC popover state. Rendered by `Layout` at the app-shell level, NOT
/// inside the breadcrumbs bar — the bar has `overflow` + a `transform`, either of
/// which would clip/trap the popover. The current-crumb trigger just sets these.
pub(super) static TOC_OPEN: GlobalSignal<bool> = Signal::global(|| false);
static TOC_ITEMS: GlobalSignal<Vec<Heading>> = Signal::global(Vec::new);

/// The page table-of-contents popover, rendered outside the (clipped, transformed)
/// breadcrumbs bar so it is actually visible. Opened by clicking the current crumb.
#[component]
pub(super) fn TocPopover() -> Element {
    if !TOC_OPEN() {
        return rsx! {};
    }
    let items = TOC_ITEMS();
    rsx! {
        div { class: "toc-scrim", onclick: move |_| { *TOC_OPEN.write() = false; } }
        nav { class: "toc-popover", "aria-label": t("toc.title"),
            if items.is_empty() {
                div { class: "toc-empty body-medium text-muted", "{t(\"toc.empty\")}" }
            } else {
                for (id , text , level) in items.iter() {
                    button {
                        key: "{id}",
                        class: "toc-item toc-level-{level}",
                        onclick: {
                            let id = id.clone();
                            move |_| {
                                scroll_to_id(&id);
                                *TOC_OPEN.write() = false;
                            }
                        },
                        "{text}"
                    }
                }
            }
        }
    }
}

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
                            // The current node's crumb doubles as the page TOC trigger.
                            toc_trigger: last_id == i + 1,
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
    /// This (current) crumb doubles as the page table-of-contents trigger: clicking
    /// it opens a popover of every heading on the page, each scrolling to it.
    #[props(default)]
    toc_trigger: bool,
) -> Element {
    if toc_trigger {
        // The current crumb toggles the page TOC; the popover itself is rendered by
        // `Layout` (via `TocPopover`) OUTSIDE this overflow-clipped, transformed bar.
        return rsx! {
            div { class: if app_crumb { "crumb app-crumb crumb-toc" } else { "crumb crumb-toc" },
                div {
                    class: "crumb-link",
                    style: "cursor: pointer;",
                    onclick: move |_| {
                        let now = !TOC_OPEN();
                        if now {
                            *TOC_ITEMS.write() = collect_headings();
                        }
                        *TOC_OPEN.write() = now;
                    },
                    div { class: "avatar small crumb-avatar",
                        {crate::components::loader::node_avatar(&mime, &name, ordinal)}
                    }
                    span { class: "crumb-name open", "{name}" }
                }
            }
        };
    }

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
