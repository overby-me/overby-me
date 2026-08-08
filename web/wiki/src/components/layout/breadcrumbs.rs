use dioxus::prelude::*;
use wasm_bindgen::JsCast;

use super::*;
use crate::i18n::t;
use crate::route::Route;

/// A heading found on the page for the table of contents: (element id, text, TOC
/// depth, icon). Depth 0 is a hard header (a section/card title or a top-level
/// document heading — shown bold, flush-left); depth 1+ are nested subheaders,
/// indented. The icon is a `material-icons` ligature, or a single letter prefixed
/// with `@` for a lettered avatar (a policy's A/B, a folder's initial).
type Heading = (String, String, u8, String);

/// The icon/avatar for a TOC entry, reusing the card header's own glyph so the
/// list mirrors the page. A heading that titles a card/section takes that card's
/// avatar icon (or its letter avatar); an in-body document heading falls back to a
/// generic glyph (weightier for hard headers).
fn heading_icon(hdr: Option<&web_sys::Element>, depth: u8) -> String {
    if let Some(hdr) = hdr {
        // Prefer the avatar's icon over any incidental icon in the header (e.g. the
        // small "schedule" date icon that sits next to the title).
        for sel in [".avatar .material-icons", ".material-icons"] {
            if let Ok(Some(mi)) = hdr.query_selector(sel) {
                let lig = mi.text_content().unwrap_or_default().trim().to_string();
                if !lig.is_empty() {
                    return lig;
                }
            }
        }
        if let Ok(Some(lbl)) = hdr.query_selector(".avatar-label, .folder-letter") {
            if let Some(c) = lbl.text_content().unwrap_or_default().trim().chars().next() {
                return format!("@{c}");
            }
        }
    }
    if depth == 0 { "segment" } else { "subject" }.to_string()
}

/// Scan the content pane for every heading (h1–h6) — document headers AND the
/// section/card headers rendered by components (amendments, polls, comments, …).
/// Ensures each has an id (assigning one when missing) so the TOC can scroll to
/// it, and returns them in document order. Skipped: decorative dividers (dashes,
/// rules — no letters or digits); UI chrome (the tools/"Actions" sheet, dialogs);
/// and the page's own title, which just repeats the current breadcrumb.
fn collect_headings() -> Vec<Heading> {
    let mut out = Vec::new();
    let Some(doc) = web_sys::window().and_then(|w| w.document()) else {
        return out;
    };
    // The open breadcrumb already names the current page; drop a heading that only
    // repeats it (the document's own title card).
    let crumb = doc
        .query_selector(".breadcrumbs .crumb-name.open")
        .ok()
        .flatten()
        .and_then(|e| e.text_content())
        .unwrap_or_default()
        .trim()
        .to_string();
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
        // Skip UI chrome (the tools/"Actions" sheet, dialogs), not page content.
        if matches!(el.closest(".tool-sheet, .m3-dialog"), Ok(Some(_))) {
            continue;
        }
        let text = el.text_content().unwrap_or_default().trim().to_string();
        // Skip empty headings and decorative dividers (dashes/rules with no label).
        if text.is_empty() || !text.chars().any(char::is_alphanumeric) {
            continue;
        }
        let dom_level = el
            .tag_name()
            .to_lowercase()
            .strip_prefix('h')
            .and_then(|n| n.parse::<u8>().ok())
            .unwrap_or(3);
        // A section/card header (Comments, a poll, …) is a hard header regardless of
        // its DOM tag, as is a top-level document heading (h1/h2). Deeper document
        // headings nest by their level.
        let hdr = el
            .closest(".card-header, .content-hero-veil")
            .ok()
            .flatten();
        // Skip the page's own TITLE card when it just repeats the current breadcrumb.
        // Only a card/hero TITLE is dropped — a document body heading that happens to
        // match the node's name is real content and stays.
        if hdr.is_some() && !crumb.is_empty() && text == crumb {
            continue;
        }
        let depth = if hdr.is_some() || dom_level <= 2 {
            0
        } else {
            (dom_level - 2).min(3)
        };
        let mut id = el.id();
        if id.is_empty() {
            id = format!("toc-h{i}");
            el.set_id(&id);
        }
        let icon = heading_icon(hdr.as_ref(), depth);
        out.push((id, text, depth, icon));
    }
    out
}

/// Smooth-scroll the element with `id` into view (for a TOC click). When the
/// heading titles a card/section, scroll the whole card so its top (the avatar and
/// header padding *above* the title) is what lands under the sticky bar — not the
/// title text, which would leave the card's head cut off. `scroll-margin-top` on
/// the target (set in CSS) clears the sticky top bar.
fn scroll_to_id(id: &str) {
    if let Some(el) = web_sys::window()
        .and_then(|w| w.document())
        .and_then(|d| d.get_element_by_id(id))
    {
        let target = if matches!(el.closest(".card-header, .content-hero-veil"), Ok(Some(_))) {
            el.closest(".card, .content-hero")
                .ok()
                .flatten()
                .unwrap_or_else(|| el.clone())
        } else {
            el.clone()
        };
        let opts = web_sys::ScrollIntoViewOptions::new();
        opts.set_behavior(web_sys::ScrollBehavior::Smooth);
        opts.set_block(web_sys::ScrollLogicalPosition::Start);
        target.scroll_into_view_with_scroll_into_view_options(&opts);
    }
}

/// Global TOC popover state. Rendered by `Layout` at the app-shell level, NOT
/// inside the breadcrumbs bar — the bar has `overflow` + a `transform`, either of
/// which would clip/trap the popover. The current-crumb trigger just sets these.
pub(super) static TOC_OPEN: GlobalSignal<bool> = Signal::global(|| false);
static TOC_ITEMS: GlobalSignal<Vec<Heading>> = Signal::global(Vec::new);
/// Inline position for the popover, computed from the trigger crumb's on-screen
/// box when it opens (see `toc_anchor_style`). Overrides the CSS fallback so the
/// popover sits under the crumb wherever the breadcrumbs bar is — not pinned to
/// the far left, which is wrong once the nav rail + tree pane push the bar right.
static TOC_STYLE: GlobalSignal<String> = Signal::global(String::new);

/// Anchor the popover to the current-crumb trigger: align its left edge to the
/// crumb (clamped into the viewport), and open below the crumb when it sits in the
/// top half of the screen, or above it when the bar is docked at the bottom
/// (compact). Returns an inline `style` string, empty if the trigger isn't found.
fn toc_anchor_style() -> String {
    let Some(win) = web_sys::window() else {
        return String::new();
    };
    let Some(el) = win
        .document()
        .and_then(|d| d.query_selector(".crumb-toc").ok().flatten())
    else {
        return String::new();
    };
    let rect = el.get_bounding_client_rect();
    let vw = win
        .inner_width()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let vh = win
        .inner_height()
        .ok()
        .and_then(|v| v.as_f64())
        .unwrap_or(0.0);
    let gap = 8.0;
    // Keep the popover (max-width 340) fully on-screen with a 12px margin.
    let width = 340.0_f64;
    let mut left = rect.left();
    if left + width + 12.0 > vw {
        left = (vw - width - 12.0).max(12.0);
    }
    if rect.top() < vh / 2.0 {
        format!(
            "left: {left:.0}px; top: {:.0}px; bottom: auto;",
            rect.bottom() + gap
        )
    } else {
        format!(
            "left: {left:.0}px; bottom: {:.0}px; top: auto;",
            vh - rect.top() + gap
        )
    }
}

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
        nav { class: "toc-popover", style: "{TOC_STYLE()}", "aria-label": t("toc.title"),
            if items.is_empty() {
                div { class: "toc-empty body-medium text-muted", "{t(\"toc.empty\")}" }
            } else {
                for (id , text , level , icon) in items.iter() {
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
                        if let Some(letter) = icon.strip_prefix('@') {
                            span { class: "toc-letter", "{letter}" }
                        } else {
                            span { class: "material-icons toc-icon", "{icon}" }
                        }
                        span { class: "toc-text", "{text}" }
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
        // The root takes apps too (`/?app=member`), so its trail ends in the same
        // app crumb every other page's does.
        Route::Home { app } => (vec![], app.clone()),
        _ => (vec![], None),
    };

    // Resolved once by `Layout`; read reactively so crumbs update on navigation.
    let crumbs = NAV_CRUMBS();
    let resolving = super::NAV_CRUMBS_LOADING();
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
                    // A crumb counts only if it resolved from THIS segment.
                    // Otherwise it belongs to the path we came from — following a
                    // link deeper into the wiki, or a search result landing
                    // somewhere else entirely, would show the old names in the new
                    // place until the resolution caught up. Matching on the
                    // segment also means navigating between siblings does not
                    // shimmer the crumbs that did not change.
                    let info = crumbs.get(i).filter(|c| c.key == segments[i]);
                    // Nothing resolved for this step yet. Showing the URL slug
                    // under a question-mark icon guessed at content and looked
                    // like an error; a shimmer says "coming" and takes the space
                    // the crumb will occupy, so the rail does not jump when the
                    // name lands.
                    if info.is_none() && resolving {
                        rsx! {
                            span {
                                key: "{i}-{segments[i]}",
                                class: "crumb crumb-pending skeleton",
                                style: "--crumb-i: {i - start + usize::from(show_home)}",
                            }
                        }
                    } else {
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
                            key: "{i}-{segments[i]}",
                            to: Route::PathPage { segments: segments[..=i].to_vec(), app: None },
                            mime,
                            name,
                            ordinal,
                            // Position in the trail, for the staggered entrance.
                            step: i - start + usize::from(show_home),
                            open: last_id == i + 1,
                            // The current node's crumb doubles as the page TOC trigger.
                            toc_trigger: last_id == i + 1,
                        }
                    }
                    }
                }
            }
            // The open app (vote / speak / members / editor / …) as its own trailing
            // crumb — a labelled, clickable step rather than a badge on the node.
            if let Some(a) = app.clone() {
                BreadcrumbCrumb {
                    // `/` has its own route variant, so an app on the root cannot
                    // be addressed as an empty-segment PathPage.
                    to: if segments.is_empty() {
                        Route::Home { app: Some(a.clone()) }
                    } else {
                        Route::PathPage { segments: segments.clone(), app: Some(a.clone()) }
                    },
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
        "feed" => t("layout.feed"),
        "bin" => t("bin.title"),
        "feedback" => t("feedback.view"),
        "folder" => t("mime.folder"),
        "speak" => t("mime.speak"),
        "vote" => t("mime.vote"),
        "member" => t("common.members"),
        "editor" => t("mime.editor"),
        "sort" => t("mime.sort"),
        "screen" => t("mime.screen"),
        // `pixel` too: the app was called that before it was renamed.
        "canvas" | "pixel" => t("mime.canvas"),
        "follow" => t("mime.follow"),
        "admin" => t("console.title"),
        // The remaining URL-only apps: still labelled so a deep link shows a name
        // rather than its raw key.
        "program" => t("mime.program"),
        "graph" => t("mime.graph"),
        "social" => t("mime.social"),
        "map" => t("mime.map"),
        "perm" => t("mime.permissions"),
        "parent" => t("mime.parent"),
        "redirect" => t("mime.redirect"),
        "cow" => t("mime.cow"),
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
    /// Position in the trail, left to right. The trail used to draw itself as
    /// each segment resolved, a query at a time; it now arrives in one, so the
    /// unfolding is staggered deliberately instead of by latency.
    #[props(default)]
    step: usize,
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
            div {
                class: if app_crumb { "crumb app-crumb crumb-toc" } else { "crumb crumb-toc" },
                style: "--crumb-i: {step}",
                div {
                    class: "crumb-link",
                    onclick: move |_| {
                        let now = !TOC_OPEN();
                        if now {
                            *TOC_ITEMS.write() = collect_headings();
                            *TOC_STYLE.write() = toc_anchor_style();
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
            style: "--crumb-i: {step}",
            // Clicking a crumb (navigating to an ancestor, or re-clicking the
            // current node) scrolls the content back to the top.
            onclick: move |_| crate::scroll_host::scroll_to(0.0),
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

/// What a context's own front page is called, and the glyph that stands for it:
/// Home at the root of the wiki, otherwise the group, event or folder you are
/// standing in.
///
/// This is the first rail item, and it is named after the PLACE rather than
/// after the view, because that is what it opens: the entry always points at the
/// context root, never at whatever sub-folder you happen to be in, so "Folder"
/// described neither where it goes nor, inside an event, what it goes to.
///
/// Read from the crumbs the chrome already resolved (`NAV_CRUMBS` with
/// `CONTEXT_DEPTH`), so naming the rail costs no query. During a navigation
/// those still hold the PREVIOUS route's crumbs, which is why the label changes
/// with the page rather than blanking between two of them.
pub(super) fn context_home(segments: &[String]) -> (&'static str, String) {
    // Indexed off the context PATH rather than the raw depth, the way the drawer
    // names the same place: the depth is carried over from the previous route
    // while the crumbs load, and on a shorter path it would otherwise point past
    // the context at whatever was there before.
    let ctx_path = context_path(segments);
    let mime = NAV_CRUMBS()
        .get(ctx_path.len().saturating_sub(1))
        .and_then(|c| c.mime_id.clone());
    let (icon, key) = place_name(segments.is_empty(), mime.as_deref());
    (icon, t(key))
}

/// The glyph and translation key for the front page of a context of this mime.
///
/// Whether this is the root is a separate argument from the mime on purpose. A
/// missing mime is NOT the root: the crumbs are empty on every navigation until
/// they load, and permanently for a signed-out visitor to a context they cannot
/// read — and inferring Home from that put "Home" on a group's own front page,
/// under a link that goes to the group. Unknown falls back to a folder, which is
/// the one name that is never a lie about where the link goes.
///
/// Separate from [`context_home`] so the naming is decided by a pure function:
/// the label a person reads is one lookup away from a key that does not exist,
/// and `t` renders a missing key as the key itself.
pub(super) fn place_name(
    at_root: bool,
    context_mime: Option<&str>,
) -> (&'static str, &'static str) {
    if at_root {
        return ("app/home", "common.home");
    }
    match context_mime {
        Some("wiki/group") => ("wiki/group", "mime.group"),
        Some("wiki/event") => ("wiki/event", "mime.event"),
        Some("wiki/site") => ("wiki/site", "mime.site"),
        // A path with no group or event above it, or crumbs that have not
        // arrived: the context is a plain folder, and so is the name.
        _ => ("wiki/folder", "mime.folder"),
    }
}

/// App rail — vertical icon navigation for large screens
/// The context apps for the current route, each as `(mime, label, route,
/// is-active)`. The first is always the context's own front page, named after
/// the place (see [`context_home`]); then the feed, and for authed users
/// speak/vote/canvas/member and the rest. Shared by the desktop rail and the
/// mobile app bar, mirroring React's `useApps`.
pub(super) fn context_apps(
    route: &Route,
    is_auth: bool,
) -> Vec<(&'static str, String, Route, bool)> {
    let segments: Vec<String> = match route {
        Route::PathPage { segments, .. } => segments.clone(),
        _ => vec![],
    };
    let current_app = match route {
        Route::PathPage { app, .. } | Route::Home { app } => app.clone(),
        _ => None,
    };
    if segments.is_empty() {
        // The root has a rail too, holding its own front page plus the apps that
        // mean something at the top of the wiki. The rest act on content, which
        // the root has none of. Signed out it stays empty: a lone Home item
        // pointing at the page you are already on is furniture, and the welcome
        // page has its own way in.
        let mut root_apps: Vec<(&str, String, Route, bool)> = Vec::new();
        if !is_auth {
            return root_apps;
        }
        // Home: the root's own front page, the same first slot every other
        // context gets. The root was the one place the pattern broke — its rail
        // opened on the feed with no way back to the list of groups and events
        // except the logo.
        let (home_icon, home_label) = context_home(&segments);
        root_apps.push((
            home_icon,
            home_label,
            Route::Home { app: None },
            current_app.is_none(),
        ));
        // The feed, which at the root means every group and event you belong to
        // (see `FeedApp`) rather than one context's own.
        root_apps.push((
            "app/feed",
            t("layout.feed"),
            Route::Home {
                app: Some("feed".to_string()),
            },
            current_app.as_deref() == Some("feed"),
        ));
        // The root's members are its owners: the people who may create groups and
        // events. It is managed by the same app every other context uses, and
        // this entry is the only way in.
        if crate::components::loader::CTX_IS_OWNER().unwrap_or(false) {
            root_apps.push((
                "app/member",
                t("common.members"),
                Route::Home {
                    app: Some("member".to_string()),
                },
                current_app.as_deref() == Some("member"),
            ));
        }
        // The root's own bin: a binned group or event comes back from here, and
        // only from here — a context is its own context, so a deleted group's
        // own bin would be inside the thing that is gone. That makes this the
        // recovery path for whoever owned the group, who is rarely an owner of
        // the root, so it cannot be a root-owner surface. The view shows each
        // person only what was theirs.
        if is_auth {
            root_apps.push((
                "app/bin",
                t("bin.title"),
                Route::Home {
                    app: Some("bin".to_string()),
                },
                current_app.as_deref() == Some("bin"),
            ));
        }
        // What people have reported. It has been an app all along, but the only
        // way in was the account menu inside the drawer, two lids down from
        // anywhere, which is no way to invite anyone to say what is wrong.
        if crate::components::feedback::FEEDBACK_ENABLED {
            root_apps.push((
                "app/feedback",
                t("feedback.view"),
                Route::Home {
                    app: Some("feedback".to_string()),
                },
                current_app.as_deref() == Some("feedback"),
            ));
        }
        return root_apps;
    }
    // The app is part of the route's query, so these navigate client-side and the
    // resolver swaps the view without a reload.
    let ctx_path = context_path(&segments);

    // Still no site Home here. The rail is the axis of THIS context, and a site
    // Home item dropped the context and emptied the rail of everything else.
    // Changing place is the drawer's job (see `ContextSwitchBar`).
    let mut apps: Vec<(&str, String, Route, bool)> = Vec::new();
    // The context's own front page, first and named after the place: Group,
    // Event, or Folder. A rail is a list of views of somewhere, so the somewhere
    // belongs at the top of it — and unlike the site Home that used to sit here,
    // this one keeps you where you are.
    let (home_icon, home_label) = context_home(&segments);
    apps.push((
        home_icon,
        home_label,
        // Back to the page they were reading, not the top of the group. Falls
        // back to the context root the first time, and whenever the reader has
        // not been in this app yet (see nav_memory).
        Route::PathPage {
            segments: crate::nav_memory::destination(&ctx_path, None),
            app: None,
        },
        // The editor / sort sub-apps operate on this content, so the entry stays
        // highlighted while they are open.
        current_app.is_none() || matches!(current_app.as_deref(), Some("editor") | Some("sort")),
    ));
    // What has happened here, newest first. Signed in only: the feed is what YOU
    // may see, and a signed-out visitor would get an empty list that reads as
    // "nothing happened" rather than "not for you".
    if is_auth {
        apps.push((
            "app/feed",
            t("layout.feed"),
            Route::PathPage {
                segments: crate::nav_memory::destination(&ctx_path, Some("feed")),
                app: Some("feed".to_string()),
            },
            current_app.as_deref() == Some("feed"),
        ));
    }
    if is_auth {
        // Members and the chair console are owner surfaces: only context owners
        // get their rail/bar entries (written by the path resolver as pages
        // load; unknown-while-resolving counts as not-owner, so the entries
        // never show and then vanish — they animate in once ownership is
        // confirmed). Their deep links still resolve for everyone; the apps
        // gate their admin controls themselves.
        let is_ctx_owner = crate::components::loader::CTX_IS_OWNER().unwrap_or(false);
        for (app, icon, label) in [
            ("speak", "app/speak", t("mime.speak")),
            ("vote", "app/vote", t("mime.vote")),
            // A canvas is reached through its app, like a speaker list, rather
            // than sitting in the folder listing among the documents.
            ("canvas", "app/canvas", t("mime.canvas")),
        ] {
            apps.push((
                icon,
                label,
                Route::PathPage {
                    segments: crate::nav_memory::destination(&ctx_path, Some(app)),
                    app: Some(app.to_string()),
                },
                current_app.as_deref() == Some(app),
            ));
        }
        if is_ctx_owner {
            apps.push((
                "app/member",
                t("common.members"),
                Route::PathPage {
                    segments: crate::nav_memory::destination(&ctx_path, Some("member")),
                    app: Some("member".to_string()),
                },
                current_app.as_deref() == Some("member"),
            ));
        }
        // Follow the room: a member's device tracks the context's active node
        // (what the chair projected) and shows it live, to read/vote in step with
        // the room. Sits after the per-item apps as a live-session destination.
        apps.push((
            "app/follow",
            t("mime.follow"),
            Route::PathPage {
                segments: crate::nav_memory::destination(&ctx_path, Some("follow")),
                app: Some("follow".to_string()),
            },
            current_app.as_deref() == Some("follow"),
        ));
        // The chair's run-the-meeting console (agenda + project + results).
        if is_ctx_owner {
            apps.push((
                "app/admin",
                t("console.title"),
                Route::PathPage {
                    segments: crate::nav_memory::destination(&ctx_path, Some("admin")),
                    app: Some("admin".to_string()),
                },
                current_app.as_deref() == Some("admin"),
            ));
        }
        // The other apps (screen, program, graph, social, map, profile, perm,
        // parent) are still reachable via their `?app=` URL but hidden from these
        // nav surfaces until they are ready to show. (admin IS shown, just above.)

        // What was deleted here, and the way back. Not an owner surface: anyone
        // who can delete something can undo it, and the view shows each person
        // what was theirs (owners see the context's whole bin). Hiding it from
        // the people most likely to need it would make the bin a thing you have
        // to ask an owner for.
        if is_auth {
            apps.push((
                "app/bin",
                t("bin.title"),
                Route::PathPage {
                    segments: crate::nav_memory::destination(&ctx_path, Some("bin")),
                    app: Some("bin".to_string()),
                },
                current_app.as_deref() == Some("bin"),
            ));
        }
        // Feedback, always last: it is a tool rather than a view of this context,
        // so it sits after everything the context is actually about, and is the
        // first thing the bottom bar pushes into its overflow sheet.
        //
        // Addressed at the context you are in, not at the site root. The app
        // needs no node (the resolver dispatches it before resolution), so the
        // path costs nothing and buys everything: the rail keeps this context's
        // apps while you write, and one tap on the crumb puts you back.
        if is_auth && crate::components::feedback::FEEDBACK_ENABLED {
            apps.push((
                "app/feedback",
                t("feedback.view"),
                Route::PathPage {
                    segments: crate::nav_memory::destination(&ctx_path, Some("feedback")),
                    app: Some("feedback".to_string()),
                },
                current_app.as_deref() == Some("feedback"),
            ));
        }
    }
    apps
}

#[cfg(test)]
mod place_tests {
    use super::place_name;

    /// A context is named after what it is, so the first rail item reads as the
    /// place rather than as a view of it.
    #[test]
    fn a_context_is_named_after_what_it_is() {
        assert_eq!(place_name(true, None), ("app/home", "common.home"));
        assert_eq!(
            place_name(false, Some("wiki/group")),
            ("wiki/group", "mime.group")
        );
        assert_eq!(
            place_name(false, Some("wiki/event")),
            ("wiki/event", "mime.event")
        );
    }

    /// The fallback is a folder, not an empty label: a path with no group or
    /// event above it still has a front page, and a crumb whose mime has not
    /// arrived yet must not render a blank rail item.
    #[test]
    fn anything_else_falls_back_to_a_folder() {
        assert_eq!(
            place_name(false, Some("wiki/folder")),
            ("wiki/folder", "mime.folder")
        );
        assert_eq!(place_name(false, Some("")), ("wiki/folder", "mime.folder"));
        assert_eq!(
            place_name(false, Some("wiki/document")),
            ("wiki/folder", "mime.folder")
        );
    }

    /// Crumbs that have not arrived are not the root. They are empty on every
    /// navigation until they load, and stay empty for a signed-out visitor to a
    /// context they cannot read — and calling that Home put "Home" on a group's
    /// own front page, on a link that goes to the group. Caught in a browser,
    /// not here, which is why it is now pinned here.
    #[test]
    fn a_missing_mime_is_not_the_root() {
        assert_eq!(place_name(false, None), ("wiki/folder", "mime.folder"));
    }

    /// Every glyph the first rail item can ask for must resolve to a real icon,
    /// or the item renders an empty box where the place should be.
    #[test]
    fn every_place_has_a_glyph() {
        for (root, mime) in [
            (true, None),
            (false, Some("wiki/group")),
            (false, Some("wiki/event")),
            (false, None),
        ] {
            let (icon, _) = place_name(root, mime);
            let glyph = crate::components::loader::mime_icon(icon);
            assert!(
                !glyph.is_empty() && glyph != "insert_drive_file",
                "{icon} has no glyph of its own"
            );
        }
    }
}
