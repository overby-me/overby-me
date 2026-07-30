use dioxus::prelude::*;

use crate::graphql::{self};
use crate::i18n::t;
use crate::model::NodeWithChildren;
use crate::route::Route;
use crate::session::use_session;

use super::content::ContentApp;
use super::editor::EditorApp;
use super::file::FileApp;
use super::folder::FolderApp;
use super::home::HomeApp;
use super::member::MemberApp;
use super::node::NodeApp;
use super::sort::SortApp;
use super::speak::SpeakApp;
use super::vote::{PolicyApp, PollApp, PositionApp, VoteApp};

/// The catch-all path page. Re-keys the resolver on the full path so navigating
/// between two `PathPage` routes remounts it and re-runs the query: `use_resource`
/// only re-runs for reactive reads inside its closure, not for a changed prop, so
/// without this the view would keep showing the previously resolved node.
#[component]
pub fn PathPage(segments: Vec<String>, app: Option<String>) -> Element {
    // Read the route directly (like the breadcrumbs and app rail) so this
    // re-renders on EVERY navigation. Relying only on the router handing us new
    // props is unreliable when moving between two `PathPage` routes (e.g. a
    // breadcrumb click going up a level): some renders keep the old props, so
    // the URL changed but the view stayed put. Subscribing via `use_route`
    // guarantees the re-render, after which the key below remounts the resolver.
    let route = use_route::<Route>();
    let (segments, app) = match route {
        Route::PathPage { segments, app } => (segments, app),
        _ => (segments, app),
    };

    // Re-key the resolver on the path (not the app) so navigating between two
    // paths remounts and refetches, while switching apps at the same path just
    // re-renders and swaps the view without a redundant query. Join with a
    // separator that cannot appear in node keys (not "/", which a lint mistakes
    // for filesystem path joining) so the key is unique per path.
    let key = segments.join("\u{1f}");
    rsx! {
        PathResolver { key: "{key}", segments, app }
    }
}

/// `/`: its own route (Dioxus 0.7 can't serialize the empty catch-all to a
/// usable URL), sharing the same [`PathResolver`] as every other path. Without
/// an app it renders the welcome; `?app=editor` opens the owner-only root editor.
#[component]
pub fn Home(app: Option<String>) -> Element {
    rsx! {
        PathResolver { segments: Vec::<String>::new(), app }
    }
}

/// Whether the signed-in user owns the currently resolved page's context.
/// `None` while the page is still resolving. Written by [`PathResolver`]; read
/// by the navigation surfaces (`layout::breadcrumbs::context_apps`) so the
/// owner-only apps (members, console) only show for context owners — and never
/// flash before ownership is known (they appear once it resolves, animated by
/// the rail/bar item entry animation).
pub(crate) static CTX_IS_OWNER: GlobalSignal<Option<bool>> = Signal::global(|| None);

/// Resolves a path to a node and renders the matching app. The query re-runs
/// whenever the path (or token) changes.
#[component]
fn PathResolver(segments: Vec<String>, app: Option<String>) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();

    // Depend reactively on `segments` so the query re-runs when the path
    // changes, WITHOUT relying on a keyed remount: changing a single child's
    // `key` does not reliably force a remount in the web renderer (it does in
    // Servo, which is why this only showed up in real browsers), so a
    // path-change navigation would otherwise keep the previously resolved node.
    // Also depend on the global data version so a mutation (e.g. saving the
    // editor and returning to this same path) forces a refetch: the path and
    // token are unchanged, so without this the resolver would serve the stale
    // pre-edit node until a full reload.
    let segs = segments.clone();
    let node_future = crate::use_data_resource!(|(segs, access_token)| async move {
        // Empty segments is `/?app=editor` (the root editor): the root has no path
        // row, so `resolve_path(&[])` is `None`; fetch it by id instead. The
        // plain `/` welcome below does not depend on this succeeding.
        if segs.is_empty() {
            graphql::query_root_node(access_token.as_deref()).await
        } else {
            graphql::resolve_path(access_token.as_deref(), &segs).await
        }
    });

    // Publish whether the user owns this page's context, for the nav surfaces
    // (owner-only rail apps). An effect rather than a render-time write. While
    // a navigation is still resolving the state is UNKNOWN (`None`): the owner
    // apps stay hidden rather than flashing the previous context's set and
    // then disappearing.
    use_effect(move || {
        let owner = match &*node_future.read() {
            Some(Ok(Some(node))) => Some(node.is_context_owner.unwrap_or(false)),
            Some(_) => Some(false),
            None => None,
        };
        if *CTX_IS_OWNER.peek() != owner {
            *CTX_IS_OWNER.write() = owner;
        }
    });

    // Node-independent apps (profile, parent) take no node, so render them
    // directly — otherwise the empty-path HomeApp short-circuit below would
    // swallow `/?app=profile` and show the home page instead.
    match app.as_deref() {
        // No `profile` arm: a person is shown at /profile/:id, your own id
        // included, so there is one profile page rather than two.
        Some("parent") => return rsx! { super::parent::ParentApp {} },
        Some("feedback") => return rsx! { super::feedback_app::FeedbackApp {} },
        _ => {}
    }

    // `/` (empty path, no app) is the welcome page. Render HomeApp directly: it
    // fetches the root itself and still renders when logged out (or when the root
    // isn't readable), whereas resolving the root here would 404 for an anonymous
    // visitor and hide the welcome card + login links. `/?app=editor` falls
    // through to the resolver below to open the owner editor on the root node.
    if segments.is_empty() && app.as_deref() != Some("editor") {
        return rsx! { HomeApp {} };
    }

    let result = node_future.read().clone();
    match result {
        Some(Ok(Some(node))) => {
            // The active app comes from the route's `?app=` query.
            match app.as_deref() {
                Some("feed") => rsx! { super::feed::FeedApp { node } },
                Some("vote") => rsx! { VoteApp { node } },
                Some("speak") => rsx! {
                    SpeakApp { node, mode: super::speak::SpeakMode::Full }
                },
                Some("member") => rsx! { MemberApp { node } },
                Some("editor") => rsx! { EditorApp { node } },
                Some("sort") => rsx! { SortApp { node } },
                Some("screen") => rsx! { super::screen::ScreenApp { node } },
                Some("follow") => rsx! { super::screen::FollowApp { node } },
                Some("admin") => rsx! { super::admin::AdminApp { node } },
                Some("perm") => rsx! { super::perm::PermApp { node } },
                Some("map") => rsx! { super::map::MapApp { node } },
                Some("graph") => rsx! { super::graph::GraphApp { node, path: segments.clone() } },
                Some("program") => {
                    rsx! { super::program::ProgramApp { node, path: segments.clone() } }
                }
                // profile / parent are handled before node resolution (they need
                // no node), so they don't appear here.
                Some("redirect") => rsx! { super::redirect::RedirectApp { node } },
                Some("social") => rsx! { super::social::SocialApp { node } },
                Some("cow") => rsx! { super::cow::CowApp { node } },
                _ => rsx! { MimeLoader { node, path: segments.clone() } },
            }
        }
        Some(Ok(None)) => {
            rsx! { NodeNotFound {} }
        }
        Some(Err(e)) => {
            // Log the detail; show a friendly state, never a raw debug dump.
            log::error!("resolve node: {e}");
            rsx! {
                div { class: "card accent-error",
                    super::widgets::ErrorState { title: t("error.somethingWentWrong") }
                }
            }
        }
        None => {
            rsx! { ContentSkeleton {} }
        }
    }
}

/// DESIGN (functional): a content-shaped shimmer placeholder shown while a
/// node loads, instead of a bare spinner — better perceived performance because
/// the layout appears immediately in roughly its final shape.
#[component]
fn ContentSkeleton() -> Element {
    rsx! {
        div { class: "card",
            div { class: "skeleton-card",
                div { class: "skeleton-row",
                    div { class: "skeleton skeleton-avatar" }
                    div { class: "flex-grow",
                        // Only the shape (the share of the line each placeholder
                        // fills) is per-instance; every other value is the
                        // .skeleton-line treatment.
                        div { class: "skeleton skeleton-line", style: "width: 45%;" }
                        div { class: "skeleton skeleton-line", style: "width: 25%;" }
                    }
                }
                div { class: "skeleton skeleton-line", style: "width: 92%;" }
                div { class: "skeleton skeleton-line", style: "width: 97%;" }
                div { class: "skeleton skeleton-line", style: "width: 83%;" }
                div { class: "skeleton skeleton-line", style: "width: 89%;" }
            }
        }
    }
}

/// Routes a node to the appropriate app based on its MIME type. `projector` is set
/// only by the Screen/projector view, where a poll shows its live tally (not the
/// ballot) and the room-facing CSS strips interactive chrome.
#[component]
pub fn MimeLoader(
    node: NodeWithChildren,
    path: Vec<String>,
    #[props(default)] projector: bool,
) -> Element {
    let mime_id = node.mime_id.as_deref().unwrap_or("");

    match mime_id {
        "wiki/folder" => rsx! { FolderApp { node: node.clone(), parent_path: path, projector } },
        "wiki/document" if projector => rsx! { ContentApp { node: node.clone() } },
        "wiki/document" => rsx! {
            super::widgets::SupportingPaneLayout {
                primary: rsx! {
                    ContentApp { node: node.clone() }
                },
                supporting: rsx! {
                    super::comments::CommentSection {
                        node_id: node.id.0.clone(),
                        context_id: node.context_id.as_ref().map(|u| u.0.clone()),
                    }
                },
            }
        },
        "wiki/file" => rsx! { FileApp { node: node.clone() } },
        "wiki/home" => rsx! { HomeApp {} },
        "wiki/group" | "wiki/event" => {
            rsx! { FolderApp { node: node.clone(), parent_path: path, projector } }
        }
        "vote/policy" | "vote/change" => {
            rsx! { PolicyApp { node: node.clone(), path } }
        }
        "vote/position" => {
            rsx! { PositionApp { node: node.clone(), path } }
        }
        // A candidate reads as content (its photo is `data.image`, description is
        // the content); React hides members, which the port already omits here.
        "vote/candidate" if projector => rsx! { ContentApp { node: node.clone() } },
        "vote/candidate" => {
            rsx! {
                super::widgets::SupportingPaneLayout {
                    primary: rsx! {
                        ContentApp { node: node.clone() }
                    },
                    supporting: rsx! {
                        super::comments::CommentSection {
                            node_id: node.id.0.clone(),
                            context_id: node.context_id.as_ref().map(|u| u.0.clone()),
                        }
                    },
                }
            }
        }
        "vote/poll" => rsx! { PollApp { node: node.clone(), projector } },
        "map/map" => rsx! { super::map::MapApp { node: node.clone() } },
        // Leaf text nodes (a plain note, a Q&A question, a single comment) carry
        // their body in `data.text` / `data.content`; without an arm they fell to
        // NodeApp, which dropped the text entirely — bad on the projector.
        "text/plain" | "vote/question" | "vote/comment" => {
            rsx! { TextNode { node: node.clone() } }
        }
        _ => rsx! { NodeApp { node: node.clone(), title: t("mime.unknown") } },
    }
}

/// A minimal renderer for leaf text nodes (plain notes, questions, comments):
/// the name as a heading and the body (`data.content` rich text or `data.text`).
#[component]
fn TextNode(node: NodeWithChildren) -> Element {
    let mime = node
        .mime_id
        .clone()
        .unwrap_or_else(|| "text/plain".to_string());
    let name = node.name.clone();
    let data = node.data.as_ref().map(|d| d.0.clone());
    let has_rich = super::content::has_rich_content(data.as_ref());
    let text = data
        .as_ref()
        .and_then(|d| d.get("text"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el(&mime)} }
                h3 { class: "title-medium", "{name}" }
            }
            div { class: "card-content",
                if has_rich {
                    super::content::SlateRenderer { data: data.clone() }
                } else if !text.is_empty() {
                    p { class: "body-large text-preserve-breaks", "{text}" }
                } else {
                    p { class: "body-medium text-muted", "{t(\"common.noContent\")}" }
                }
            }
        }
    }
}

/// A localized relative-time sentence ("3 hours ago" / "for 3 timer siden") for
/// an ISO timestamp, via the browser's `Intl.RelativeTimeFormat` in the current
/// UI language. Mirrors the old wiki's date-fns `formatDistance`; the precise
/// datetime is the tooltip (see [`full_datetime`]). Shared by the comment thread
/// and the content/file "created" subtitles.
pub fn relative_time(iso: &str) -> String {
    let then = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(iso)).get_time();
    if then.is_nan() {
        return String::new();
    }
    let secs = ((js_sys::Date::now() - then) / 1000.0).max(0.0);
    // Largest sensible unit; the value is negative because it is in the past
    // (the sign `Intl.RelativeTimeFormat` expects).
    let (value, unit) = if secs < 60.0 {
        (-(secs as i64), "second")
    } else if secs < 3600.0 {
        (-((secs / 60.0) as i64), "minute")
    } else if secs < 86_400.0 {
        (-((secs / 3600.0) as i64), "hour")
    } else if secs < 2_592_000.0 {
        (-((secs / 86_400.0) as i64), "day")
    } else if secs < 31_536_000.0 {
        (-((secs / 2_592_000.0) as i64), "month")
    } else {
        (-((secs / 31_536_000.0) as i64), "year")
    };
    intl_relative_format(value, unit, crate::i18n::current_locale())
        // Fallback to a compact form if Intl.RelativeTimeFormat is unavailable.
        .unwrap_or_else(|| format!("{}{}", -value, unit.chars().next().unwrap_or('s')))
}

/// `new Intl.RelativeTimeFormat(locale, {numeric:'auto'}).format(value, unit)`
/// via reflection, so no extra wasm-bindgen binding is needed. Returns `None`
/// when the API is unavailable.
fn intl_relative_format(value: i64, unit: &str, locale: &str) -> Option<String> {
    use wasm_bindgen::{JsCast, JsValue};
    let intl = js_sys::Reflect::get(&js_sys::global(), &JsValue::from_str("Intl")).ok()?;
    let ctor: js_sys::Function =
        js_sys::Reflect::get(&intl, &JsValue::from_str("RelativeTimeFormat"))
            .ok()?
            .dyn_into()
            .ok()?;
    let opts = js_sys::Object::new();
    js_sys::Reflect::set(
        &opts,
        &JsValue::from_str("numeric"),
        &JsValue::from_str("auto"),
    )
    .ok()?;
    let args = js_sys::Array::of2(&JsValue::from_str(locale), &opts);
    let instance = js_sys::Reflect::construct(&ctor, &args).ok()?;
    let format_fn: js_sys::Function = js_sys::Reflect::get(&instance, &JsValue::from_str("format"))
        .ok()?
        .dyn_into()
        .ok()?;
    format_fn
        .call2(
            &instance,
            &JsValue::from_f64(value as f64),
            &JsValue::from_str(unit),
        )
        .ok()?
        .as_string()
}

/// The absolute, localised date/time for an ISO timestamp — used as the tooltip
/// behind a compact relative time.
pub fn full_datetime(iso: &str) -> String {
    let d = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(iso));
    if d.get_time().is_nan() {
        return String::new();
    }
    String::from(&d.to_locale_string(
        crate::i18n::current_locale(),
        &wasm_bindgen::JsValue::UNDEFINED,
    ))
}

/// Mime type to a **Material Icons ligature name**, matching the reference React
/// app's `IconId` (core/mime.tsx), which renders `@mui/icons-material` (filled
/// Material Icons). Render via `icon_el` (or a `.material-icons` span) so the
/// drawer, folder list, headers and app rail show the exact same icons.
pub fn mime_icon(mime_id: &str) -> &'static str {
    match mime_id {
        // wiki/*
        "wiki/search" | "app/search" => "search",
        "wiki/home" | "app/home" => "home",
        "wiki/group" | "app/member" => "group",
        "wiki/event" => "event",
        "wiki/folder" | "app/folder" => "folder",
        "wiki/document" => "article",
        "wiki/file" => "upload_file",
        "wiki/user" => "person",
        "text/plain" => "subject",
        // vote/*
        "vote/policy" => "gavel",
        "vote/position" => "how_to_reg",
        "vote/candidate" => "face",
        "vote/question" => "question_mark",
        "vote/comment" => "add_comment",
        "vote/reaction" => "add_reaction",
        "vote/change" => "rate_review",
        "vote/poll" => "poll",
        // speak / apps (old wiki: speak/list=InterpreterMode, app/speak=RecordVoiceOver)
        "speak/list" => "interpreter_mode",
        "app/feed" => "view_agenda",
        "app/speak" => "record_voice_over",
        "app/editor" => "edit",
        "app/sort" => "low_priority",
        "app/vote" => "how_to_vote",
        "app/screen" => "connected_tv",
        "app/follow" => "sensors",
        "app/admin" => "co_present",
        "application/pdf" => "picture_as_pdf",
        "app/map" | "map/map" => "map",
        // Apps the old wiki did not have — closest Material icons.
        "app/graph" => "hub",
        "app/program" => "calendar_month",
        "app/profile" => "account_circle",
        "app/social" => "public",
        "app/redirect" => "open_in_new",
        "app/cow" => "pets",
        "app/parent" => "link_off",
        "wiki/feedback" | "app/feedback" => "feedback",
        _ => mime_icon_by_prefix(mime_id),
    }
}

/// An icon element for a mime type: a `.material-icons` span holding the ligature
/// so the Material Icons webfont renders it. Use in place of the old emoji text.
pub fn icon_el(mime_id: &str) -> Element {
    let name = mime_icon(mime_id);
    rsx! {
        span { class: "material-icons", "{name}" }
    }
}

/// Content for a user's `.avatar`: their profile image (e.g. the linked Bluesky
/// picture, from `users.avatarUrl`) if set, otherwise `fallback` (an icon or
/// initial). Place the result inside an `.avatar` element.
pub fn user_avatar(avatar_url: &str, fallback: Element) -> Element {
    // NHost gives every user a gravatar URL (with `default=blank`, i.e. usually a
    // blank image), so only a non-gravatar URL — e.g. the linked Bluesky picture —
    // counts as a real avatar; otherwise fall back to the icon/initial.
    if avatar_url.is_empty() || avatar_url.contains("gravatar") {
        fallback
    } else {
        rsx! {
            img { class: "avatar-img", src: "{avatar_url}", alt: "", loading: "lazy" }
        }
    }
}

/// A presigned URL for a protected file, for the `src`/`href` attributes that
/// cannot carry an `Authorization` header: `<iframe>`, `<video>`, `<audio>` and
/// a download link.
///
/// Prefer [`use_file_object_url`] for images, which costs one request instead of
/// two (presign, then load) — but never for a large or streamed file, since a
/// blob URL has to buffer the whole thing before anything appears.
///
/// Reactive on `file_id` and the token, so a sibling navigation re-presigns
/// rather than serving the previous node's file. Empty `file_id` yields None.
pub fn use_presigned_url(file_id: String) -> Option<String> {
    let session = use_session();
    let token = session.read().access_token.clone();
    let mut url = use_signal(|| None::<String>);
    use_effect(use_reactive!(|(file_id, token)| {
        url.set(None);
        if file_id.is_empty() {
            return;
        }
        let Some(token) = token.clone() else { return };
        spawn(async move {
            if let Some(signed) = crate::backend_api::presigned_file_url(&file_id, &token).await {
                url.set(Some(signed));
            }
        });
    }));
    let current = url.read().clone();
    current
}

/// Fetch a protected nhost file with the session token in the `Authorization`
/// header (never the URL) and expose it as a `blob:` object URL, so the JWT never
/// enters the DOM as an `src`/`href` attribute. Empty `file_id` yields None;
/// the object URL is revoked when the component unmounts.
pub fn use_file_object_url(file_id: String) -> Option<String> {
    let session = use_session();
    let token = session.read().access_token.clone();
    let mut blob_url = use_signal(|| None::<String>);
    // Reactive on `file_id` (and the token), NOT a one-shot `use_hook`: this
    // component is reused across sibling navigations (e.g. switching between two
    // candidates), where only the `file_id` prop changes and the component never
    // remounts — a `use_hook` would keep serving the first node's image. On each
    // change, revoke the previous blob before fetching the new one.
    use_effect(use_reactive!(|(file_id, token)| {
        let previous = blob_url.peek().clone();
        if let Some(old) = previous {
            let _ = web_sys::Url::revoke_object_url(&old);
            blob_url.set(None);
        }
        if file_id.is_empty() {
            return;
        }
        let Some(token) = token.clone() else { return };
        spawn(async move {
            // The file URL is built through the one seam (backend_api), so the
            // cutover blob-path swap is a change there. The token goes in the
            // Authorization header, which is the only place the storage service
            // reads it — as this function's own description always claimed.
            let url = crate::backend_api::file_url(&file_id);
            let Ok(resp) = reqwest::Client::new()
                .get(&url)
                .bearer_auth(&token)
                .send()
                .await
            else {
                return;
            };
            if !resp.status().is_success() {
                return;
            }
            let Ok(bytes) = resp.bytes().await else {
                return;
            };
            let arr = js_sys::Uint8Array::new_with_length(bytes.len() as u32);
            arr.copy_from(&bytes);
            let parts = js_sys::Array::of1(&arr);
            if let Ok(blob) = web_sys::Blob::new_with_u8_array_sequence(parts.as_ref()) {
                if let Ok(obj) = web_sys::Url::create_object_url_with_blob(&blob) {
                    blob_url.set(Some(obj));
                }
            }
        });
    }));
    use_drop(move || {
        if let Some(u) = blob_url.peek().clone() {
            let _ = web_sys::Url::revoke_object_url(&u);
        }
    });
    let current = blob_url.read().clone();
    current
}

/// The Bluesky butterfly (the official mark), inline so it needs no asset fetch
/// and scales crisply. Sized like an inline icon via the `bsky-logo` class.
///
/// Painted in `currentColor`, not the brand blue: it sits where the Material
/// icons sit (inside an avatar circle, inside a button) and those all inherit
/// their surrounding text colour, so a fixed blue was the one mark on screen
/// ignoring the theme, and it fought the green circle it sat in.
pub fn bsky_logo() -> Element {
    rsx! {
        svg {
            class: "bsky-logo",
            view_box: "0 0 568 501",
            "aria-hidden": "true",
            path {
                fill: "currentColor",
                d: "M123.121 33.664C188.241 82.553 258.281 181.68 284 234.873c25.719-53.192 95.759-152.32 160.879-201.21C491.866-1.611 568-28.906 568 57.947c0 17.346-9.945 145.713-15.778 166.555-20.275 72.453-94.155 90.933-159.875 79.748 114.875 19.551 144.097 84.311 80.986 149.071-119.86 122.992-172.272-30.859-185.702-70.281-2.462-7.227-3.614-10.608-3.631-7.733-.017-2.875-1.169.506-3.631 7.733-13.43 39.422-65.842 193.273-185.702 70.281-63.111-64.76-33.889-129.52 80.986-149.071-65.72 11.185-139.6-7.295-159.875-79.748C9.945 203.659 0 75.291 0 57.946 0-28.906 76.135-1.612 123.121 33.664Z",
            }
        }
    }
}

/// Inline position for the identity popover, anchored to the click point (which
/// sits on the trigger chip/avatar): horizontally centred on it, clamped into
/// the viewport, opening below the trigger when it is in the top half of the
/// screen and above it otherwise. Empty (the CSS centring fallback) when the
/// window is unavailable.
fn popover_anchor_style(x: f64, y: f64) -> String {
    let Some(win) = web_sys::window() else {
        return String::new();
    };
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
    // Half the card's max width (320px) + a margin keeps the translateX(-50%)
    // card fully on-screen after clamping.
    let half = 168.0_f64;
    let x = x.clamp(half.min(vw / 2.0), (vw - half).max(vw / 2.0));
    if y < vh / 2.0 {
        let top = y + 12.0;
        format!("left: {x:.0}px; top: {top:.0}px; transform: translateX(-50%);")
    } else {
        let bottom = vh - y + 12.0;
        format!("left: {x:.0}px; top: auto; bottom: {bottom:.0}px; transform: translateX(-50%);")
    }
}

/// A click-triggered identity popover for any user representation. Wrap the
/// trigger markup (an avatar, a name, a chip) as `children`; clicking it opens a
/// small card showing a larger avatar, the display name, an optional role line,
/// and a "View profile" link (to the signed-in user's own profile when it's
/// them, otherwise the person's `/profile/:id` page). `user_id = None` hides the
/// profile link (e.g. a free-text author with no account).
#[component]
pub fn UserPopover(
    name: String,
    avatar_url: String,
    user_id: Option<String>,
    #[props(default)] role: Option<String>,
    children: Element,
) -> Element {
    let mut open = use_signal(|| false);
    // Anchors the card to the trigger (set from the opening click's position).
    let mut anchor_style = use_signal(String::new);
    let nav = use_navigator();
    // A linked Bluesky account is recognisable from its avatar URL: the bsky CDN
    // path embeds the account's DID, and bsky.app resolves profile URLs by DID —
    // so the popover can link to their Bluesky profile with no extra lookup.
    let bsky_did = avatar_url
        .contains("cdn.bsky.app/")
        .then(|| avatar_url.split('/').find(|seg| seg.starts_with("did:")))
        .flatten()
        .map(str::to_string);

    rsx! {
        button {
            class: "user-pop-trigger",
            aria_label: "{name}",
            aria_haspopup: "dialog",
            aria_expanded: "{open()}",
            onclick: move |e| {
                e.stop_propagation();
                let v = open();
                if !v {
                    let p = e.client_coordinates();
                    anchor_style.set(popover_anchor_style(p.x, p.y));
                }
                open.set(!v);
            },
            onkeydown: move |e| {
                if e.key() == Key::Escape {
                    open.set(false);
                }
            },
            {children}
        }
        if open() {
            // Every interactive part stops propagation, so the popover is safe even
            // when its trigger sits inside a clickable parent (e.g. a recent-item
            // that navigates): dismissing or tapping "View profile" must not also
            // fire the ancestor's onclick.
            div {
                class: "menu-backdrop",
                onclick: move |e| {
                    e.stop_propagation();
                    open.set(false);
                },
            }
            div {
                class: "user-pop-card",
                style: "{anchor_style}",
                role: "dialog",
                aria_modal: "true",
                // Name the popover by the user it describes, so a screen reader
                // announces whose card this is rather than a bare "dialog".
                aria_label: "{name}",
                onclick: move |e| e.stop_propagation(),
                div { class: "user-pop-head",
                    div { class: "avatar",
                        {user_avatar(&avatar_url, rsx! { span { class: "material-icons", "person" } })}
                    }
                    div {
                        div { class: "user-pop-name", "{name}" }
                        if let Some(r) = role.clone() {
                            div { class: "user-pop-role", "{r}" }
                        }
                    }
                }
                if let Some(uid) = user_id.clone() {
                    button {
                        class: "btn btn-primary btn-full",
                        onclick: move |e| {
                            e.stop_propagation();
                            open.set(false);
                            // One destination for everyone, yourself included —
                            // /profile/:id renders the self view for your own id.
                            nav.push(Route::UserProfile { id: uid.clone() });
                        },
                        span { class: "material-icons", "person" }
                        " {t(\"profile.viewProfile\")}"
                    }
                }
                if let Some(did) = bsky_did.clone() {
                    a {
                        class: "btn btn-outlined btn-full mt-1",
                        href: "https://bsky.app/profile/{did}",
                        target: "_blank",
                        rel: "noopener",
                        onclick: move |e| e.stop_propagation(),
                        {bsky_logo()}
                        " {t(\"profile.blueskyAccount\")}"
                    }
                }
            }
        }
    }
}

/// The mime id that should drive a NODE's icon: for a `wiki/file` it is the
/// file's own content type (`data.type`) so uploads show format-specific icons
/// (pdf, Word, Excel, PowerPoint, image, audio, video), mirroring the old wiki's
/// `type ?? mimeId`. Any other node returns its own mime. The result feeds
/// [`icon_el`] / [`node_avatar`], whose prefix matching maps the office/media
/// content types to the right ligature.
pub fn node_icon_mime_id(mime_id: &str, data: Option<&serde_json::Value>) -> String {
    if mime_id == "wiki/file" {
        if let Some(t) = data
            .and_then(|d| d.get("type"))
            .and_then(|t| t.as_str())
            .filter(|t| !t.is_empty())
        {
            return t.to_string();
        }
    }
    mime_id.to_string()
}

/// Like [`icon_el`] but node-aware: files get a format-specific icon from their
/// content type. Pass the node's `mime_id` and its `data` json.
pub fn node_icon_el(mime_id: &str, data: Option<&serde_json::Value>) -> Element {
    icon_el(&node_icon_mime_id(mime_id, data))
}

/// The spreadsheet-style letter for an index (0→A, 1→B, …, 25→Z, 26→AA, …),
/// ported from the React `getLetter`. Used to label policy proposals.
pub fn index_letter(index: usize) -> String {
    let last = (b'A' + (index % 26) as u8) as char;
    if index >= 26 {
        let first = (b'@' + (index / 26) as u8) as char;
        format!("{first}{last}")
    } else {
        last.to_string()
    }
}

/// The avatar content for a node, matching the reference app's `IconId`:
/// policies get a **letter** (A, B, …) and change proposals a **number**
/// (1, 2, …) by their `ordinal` among same-type siblings; folders show the
/// folder icon with their name's first letter; everything else shows its mime
/// icon. `ordinal` is None when there is no meaningful position (falls back to
/// the gavel / rate-review icon).
pub fn node_avatar(mime_id: &str, name: &str, ordinal: Option<usize>) -> Element {
    match (mime_id, ordinal) {
        ("vote/policy", Some(i)) => {
            let label = index_letter(i);
            rsx! {
                span { class: "avatar-label", "{label}" }
            }
        }
        ("vote/change", Some(i)) => {
            let n = i + 1;
            rsx! {
                span { class: "avatar-label", "{n}" }
            }
        }
        ("wiki/folder", _) => match name.chars().next() {
            Some(first) => rsx! {
                span { class: "folder-avatar",
                    span { class: "material-icons", "folder" }
                    span { class: "folder-letter", "{first}" }
                }
            },
            None => icon_el(mime_id),
        },
        _ => icon_el(mime_id),
    }
}

/// A node's avatar with a "not submitted" badge overlaid when it is still a
/// mutable draft, mirroring the old wiki's `MimeAvatarNode` (a MUI Badge with a
/// LockOpen icon). Use in the folder list / drawer tree / headers.
#[component]
pub fn NodeAvatar(
    mime: String,
    name: String,
    ordinal: Option<usize>,
    mutable: bool,
    small: bool,
    /// Render without the coloured avatar circle — just the node's representation
    /// (icon, or the policy/change letter, or the folder+letter) on a bare icon
    /// footprint, as the old wiki's node tree did.
    #[props(default)]
    bare: bool,
) -> Element {
    // Always the shared node representation (icon / letter / folder+letter), so the
    // icon-vs-letter-vs-folder logic lives in ONE place ([`node_avatar`]) for every
    // avatar in the app; `bare` only changes the container, not the content.
    let inner = node_avatar(&mime, &name, ordinal);
    let cls = if bare {
        "node-icon"
    } else if small {
        "avatar small"
    } else {
        "avatar"
    };
    rsx! {
        AvatarBadged { mutable,
            div { class: "{cls}", {inner} }
        }
    }
}

/// Wraps any avatar in the "not submitted yet" mark a mutable node carries: the
/// small `lock_open` badge on its corner. Kept as its own component so a list
/// avatar ([`NodeAvatar`]) and a page's header avatar wear the same mark from one
/// definition, instead of each place restating what the badge looks like.
#[component]
pub fn AvatarBadged(mutable: bool, children: Element) -> Element {
    rsx! {
        div { class: "avatar-badged",
            {children}
            if mutable {
                span { class: "avatar-badge", title: "{t(\"layout.notSubmitted\")}",
                    span { class: "material-icons", "lock_open" }
                }
            }
        }
    }
}

/// Nodes that expose a mime id, so `sibling_ordinals` can run over either the
/// full `ChildNodeFields` (folder view, export) or the lean `DrawerChildFields`
/// (drawer tree).
pub trait HasMimeId {
    fn mime_id_str(&self) -> Option<&str>;
}

impl HasMimeId for crate::model::ChildNodeFields {
    fn mime_id_str(&self) -> Option<&str> {
        self.mime_id.as_deref()
    }
}

impl HasMimeId for crate::model::DrawerChildFields {
    fn mime_id_str(&self) -> Option<&str> {
        self.mime_id.as_deref()
    }
}

/// For each child, its ordinal among preceding siblings of the SAME lettered /
/// numbered mime (policies, changes). Others get `None`. Feeds `node_avatar` so
/// the A/B/C and 1/2/3 labels count within their own type, like the old wiki.
pub fn sibling_ordinals<T: HasMimeId>(children: &[T]) -> Vec<Option<usize>> {
    let mut policies = 0usize;
    let mut changes = 0usize;
    children
        .iter()
        .map(|c| match c.mime_id_str() {
            Some("vote/policy") => {
                let o = policies;
                policies += 1;
                Some(o)
            }
            Some("vote/change") => {
                let o = changes;
                changes += 1;
                Some(o)
            }
            _ => None,
        })
        .collect()
}

/// Fallback icons for the media / office families the React app matches by
/// substring (image/, audio/, video/, spreadsheet, presentation, document).
fn mime_icon_by_prefix(mime_id: &str) -> &'static str {
    if mime_id.contains("image/") {
        "image"
    } else if mime_id.contains("audio/") {
        "music_note"
    } else if mime_id.contains("video/") {
        "movie"
    } else if mime_id.contains("spreadsheet") {
        "table_chart"
    } else if mime_id.contains("presentation") {
        "slideshow"
    } else if mime_id.contains("document") {
        "description"
    } else {
        "question_mark"
    }
}

/// The URL-safe base of a node key: lowercase, non-alphanumerics collapsed to
/// single dashes, trimmed. Pure (no browser globals) so it is unit-testable.
pub fn slug_base(name: &str) -> String {
    // Lowered once, as chars, so a hyphen can look at what follows it.
    let lowered = name.trim().to_lowercase();
    let chars: Vec<char> = lowered.chars().collect();
    let mut base = String::new();
    let mut prev_sep = true; // nothing yet, so a leading separator is dropped
    for (i, &c) in chars.iter().enumerate() {
        // Letters and digits, kept as they are. `is_alphanumeric` is Unicode-aware,
        // so ø, æ and å survive — the wiki's keys have always carried them
        // (`landsmøde_2026`), and stripping them now would slug a name differently
        // than it has been slugged for years.
        if c.is_alphanumeric() {
            base.push(c);
            prev_sep = false;
            continue;
        }
        // A hyphen BETWEEN two letters belongs to the word, not to the spacing:
        // `Saint-Laguë`, `to-statsløsningen`. 415 of the wiki's 3729 navigable
        // keys carry one for that reason, so folding it into a separator would
        // spell the same convention a third way. A hyphen used as punctuation —
        // `Klima-, og …`, `fra - en …`, `Trim -- Me` — is spacing, and becomes
        // one separator like any other.
        let joins_words =
            c == '-' && !prev_sep && chars.get(i + 1).is_some_and(|next| next.is_alphanumeric());
        if joins_words {
            base.push('-');
            prev_sep = false;
        } else if !prev_sep {
            // Everything else becomes an UNDERSCORE, which is what every key in
            // the wiki uses. This emitted a hyphen, so a new node read
            // `asger-holm-ørskov` beside the existing `asger_holm_ørskov`: one
            // convention, two spellings, forever.
            base.push('_');
            prev_sep = true;
        }
    }
    // A trailing separator would leave the `-2` suffix sitting behind punctuation.
    base.trim_end_matches('_').to_string()
}

/// Build a URL-safe node key from a display name plus a short unique suffix.
///
/// The suffix makes a collision practically impossible, at the cost of a number
/// in every URL forever. Prefer [`crate::graphql::insert_node_named`], which
/// spends the clean key first and only falls back to something like this when
/// the name is genuinely taken. This remains for callers with no parent to check
/// against, and as that fallback.
pub fn slugify(name: &str) -> String {
    let base = slug_base(name);
    let suffix = web_sys::window()
        .and_then(|w| w.performance())
        .map(|p| format!("{:.0}", p.now()))
        .unwrap_or_default();
    let tail = &suffix[suffix.len().saturating_sub(4)..];
    if base.is_empty() {
        format!("n-{tail}")
    } else {
        format!("{base}-{tail}")
    }
}

/// Return a node's children the way the React folder/list views show them:
/// hidden-mime entries dropped, ordered by `index` then creation time. Row-level
/// permissions are already applied by Hasura, so only the hidden filter and the
/// ordering need to happen client-side.
pub fn visible_sorted(
    children: &[crate::model::ChildNodeFields],
) -> Vec<crate::model::ChildNodeFields> {
    let mut out: Vec<crate::model::ChildNodeFields> = children
        .iter()
        .filter(|c| c.mime.as_ref().map(|m| !m.hidden).unwrap_or(true))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        a.index.cmp(&b.index).then_with(|| {
            let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
            let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
            a_ts.cmp(b_ts)
        })
    });
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{ChildNodeFields, MimeFields, Timestamptz, Uuid};

    fn child(name: &str, index: i32, hidden: bool, created: &str) -> ChildNodeFields {
        ChildNodeFields {
            id: Uuid(name.to_string()),
            name: name.to_string(),
            key: name.to_string(),
            mime_id: Some("wiki/folder".to_string()),
            mutable: false,
            index,
            created_at: Some(Timestamptz(created.to_string())),
            owner_id: None,
            is_owner: None,
            is_context_owner: None,
            owner: None,
            author_name: None,
            author_avatar: None,
            parent: None,
            data: None,
            mime: Some(MimeFields {
                id: "wiki/folder".to_string(),
                icon: "folder".to_string(),
                hidden,
                context: false,
            }),
        }
    }

    #[test]
    fn slug_base_is_url_safe() {
        // Underscores, like every key already in the wiki.
        assert_eq!(super::slug_base("Hello, World! 123"), "hello_world_123");
        assert_eq!(super::slug_base("  Trim -- Me  "), "trim_me");
        assert_eq!(super::slug_base("!!!"), "");
        // Danish letters are kept, as the existing keys keep them.
        assert_eq!(super::slug_base("Landsmøde 2026"), "landsmøde_2026");
        assert_eq!(super::slug_base("Asger Holm Ørskov"), "asger_holm_ørskov");
        // A hyphen JOINING two letters is part of the word and stays.
        assert_eq!(
            super::slug_base("Saint-Laguës metode"),
            "saint-laguës_metode"
        );
        assert_eq!(super::slug_base("To-statsløsningen"), "to-statsløsningen");
        // A hyphen used as punctuation is spacing, and collapses like any other.
        assert_eq!(
            super::slug_base("Klima-, og Miljøudvalget"),
            "klima_og_miljøudvalget"
        );
        assert_eq!(
            super::slug_base("EU- og Udenrigsudvalget"),
            "eu_og_udenrigsudvalget"
        );
        assert_eq!(
            super::slug_base("Hvor pengene kommer fra - en finansplan"),
            "hvor_pengene_kommer_fra_en_finansplan"
        );
        // Nothing is left dangling for the `-2` suffix to sit behind.
        assert_eq!(super::slug_base("Klima- "), "klima");
        assert_eq!(super::slug_base("--"), "");
        assert_eq!(super::slug_base("- Leading"), "leading");
    }

    #[test]
    fn index_letter_matches_react_getletter() {
        assert_eq!(super::index_letter(0), "A");
        assert_eq!(super::index_letter(1), "B");
        assert_eq!(super::index_letter(25), "Z");
        assert_eq!(super::index_letter(26), "AA");
        assert_eq!(super::index_letter(27), "AB");
    }

    #[test]
    fn visible_sorted_drops_hidden_and_orders_by_index_then_time() {
        let children = vec![
            child("b", 1, false, "2024-01-01"),
            child("secret", 0, true, "2024-01-01"),
            child("a", 0, false, "2024-01-02"),
            child("a-older", 0, false, "2024-01-01"),
        ];
        let out = visible_sorted(&children);
        let names: Vec<&str> = out.iter().map(|c| c.name.as_str()).collect();
        // hidden dropped; index 0 before index 1; within index 0 older first.
        assert_eq!(names, vec!["a-older", "a", "b"]);
    }

    #[test]
    fn files_get_format_specific_icons_from_their_type() {
        let cases = [
            ("application/pdf", "picture_as_pdf"),
            (
                "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
                "description",
            ),
            (
                "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
                "table_chart",
            ),
            (
                "application/vnd.openxmlformats-officedocument.presentationml.presentation",
                "slideshow",
            ),
            ("image/png", "image"),
            ("audio/ogg", "music_note"),
            ("video/mp4", "movie"),
        ];
        for (ty, icon) in cases {
            let data = serde_json::json!({ "type": ty });
            let eff = node_icon_mime_id("wiki/file", Some(&data));
            assert_eq!(eff, ty);
            assert_eq!(mime_icon(&eff), icon, "wrong icon for {ty}");
        }
    }

    #[test]
    fn files_without_a_type_and_non_files_fall_back() {
        // No data, or an empty type, keeps the generic file icon.
        assert_eq!(node_icon_mime_id("wiki/file", None), "wiki/file");
        assert_eq!(mime_icon("wiki/file"), "upload_file");
        let empty = serde_json::json!({ "type": "" });
        assert_eq!(node_icon_mime_id("wiki/file", Some(&empty)), "wiki/file");
        // Non-file nodes ignore any `type` and keep their own mime.
        let data = serde_json::json!({ "type": "application/pdf" });
        assert_eq!(node_icon_mime_id("vote/policy", Some(&data)), "vote/policy");
    }
}

#[component]
fn NodeNotFound() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();

    rsx! {
        div { class: "card",
            // DESIGN (expressive empty state): a centred "void portal" — a big
            // floating, morphing tonal orb holding the icon, a bold title, actions.
            div { class: "empty-state",
                div { class: "empty-state-orb",
                    span { class: "material-icons", "search_off" }
                }
                h3 { class: "empty-state-title", "{t(\"node.documentUnavailable\")}" }
                p { class: "empty-state-body", "{t(\"node.notFoundOrNoAccess\")}" }
                if !is_auth {
                    p { class: "empty-state-body", "{t(\"node.maybeLoginForAccess\")}" }
                    Link {
                        to: Route::Login {},
                        class: "btn btn-primary",
                        "{t(\"common.logIn\")}"
                    }
                }
            }
        }
    }
}
