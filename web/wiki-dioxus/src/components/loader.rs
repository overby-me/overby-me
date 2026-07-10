use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren};
use crate::i18n::t;
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
use super::vote::{PolicyApp, PollApp, VoteApp};

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
    let version = crate::session::DATA_VERSION();
    let node_future = use_resource(use_reactive!(|(segs, access_token, version)| async move {
        let _ = version;
        graphql::resolve_path(access_token.as_deref(), &segs).await
    }));

    let result = node_future.read().clone();
    match result {
        Some(Ok(Some(node))) => {
            // The active app comes from the route's `?app=` query.
            match app.as_deref() {
                Some("vote") => rsx! { VoteApp { node } },
                Some("speak") => rsx! {
                    SpeakApp { node, mode: super::speak::SpeakMode::Full }
                },
                Some("member") => rsx! { MemberApp { node } },
                Some("editor") => rsx! { EditorApp { node } },
                Some("sort") => rsx! { SortApp { node } },
                Some("screen") => rsx! { super::screen::ScreenApp { node } },
                Some("admin") => rsx! { super::admin::AdminApp { node } },
                Some("perm") => rsx! { super::perm::PermApp { node } },
                Some("map") => rsx! { super::map::MapApp { node } },
                Some("graph") => rsx! { super::graph::GraphApp { node, path: segments.clone() } },
                Some("program") => {
                    rsx! { super::program::ProgramApp { node, path: segments.clone() } }
                }
                Some("profile") => rsx! { super::profile::ProfileApp {} },
                Some("parent") => rsx! { super::parent::ParentApp {} },
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
            rsx! {
                div { class: "card",
                    div { class: "card-content",
                        p { class: "body-large", "{t(\"error.somethingWentWrong\")}" }
                        pre { class: "error-fallback", "{e}" }
                    }
                }
            }
        }
        None => {
            rsx! { super::widgets::Spinner {} }
        }
    }
}

/// Routes a node to the appropriate app based on its MIME type
#[component]
pub fn MimeLoader(node: NodeWithChildren, path: Vec<String>) -> Element {
    let mime_id = node.mime_id.as_deref().unwrap_or("");

    match mime_id {
        "wiki/folder" => rsx! { FolderApp { node: node.clone(), parent_path: path } },
        "wiki/document" => rsx! { ContentApp { node: node.clone() } },
        "wiki/file" => rsx! { FileApp { node: node.clone() } },
        "wiki/home" => rsx! { HomeApp {} },
        "wiki/group" | "wiki/event" => {
            rsx! { FolderApp { node: node.clone(), parent_path: path } }
        }
        "vote/policy" | "vote/change" => {
            rsx! { PolicyApp { node: node.clone(), path } }
        }
        "vote/position" => {
            rsx! { NodeApp { node: node.clone(), title: t("mime.position") } }
        }
        "vote/candidate" => {
            rsx! { NodeApp { node: node.clone(), title: t("mime.candidate") } }
        }
        "vote/poll" => rsx! { PollApp { node: node.clone() } },
        "map/map" => rsx! { super::map::MapApp { node: node.clone() } },
        _ => rsx! { NodeApp { node: node.clone(), title: t("mime.unknown") } },
    }
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
        "vote/change" => "rate_review",
        "vote/poll" => "poll",
        // speak / apps (old wiki: speak/list=InterpreterMode, app/speak=RecordVoiceOver)
        "speak/list" => "interpreter_mode",
        "app/speak" => "record_voice_over",
        "app/editor" => "edit",
        "app/sort" => "low_priority",
        "app/vote" => "how_to_vote",
        "app/screen" => "connected_tv",
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
) -> Element {
    let cls = if small { "avatar small" } else { "avatar" };
    rsx! {
        div { class: "avatar-badged",
            div { class: "{cls}", {node_avatar(&mime, &name, ordinal)} }
            if mutable {
                span { class: "avatar-badge", title: "{t(\"layout.notSubmitted\")}",
                    span { class: "material-icons", "lock_open" }
                }
            }
        }
    }
}

/// For each child, its ordinal among preceding siblings of the SAME lettered /
/// numbered mime (policies, changes). Others get `None`. Feeds `node_avatar` so
/// the A/B/C and 1/2/3 labels count within their own type, like the old wiki.
pub fn sibling_ordinals(children: &[graphql::ChildNodeFields]) -> Vec<Option<usize>> {
    let mut policies = 0usize;
    let mut changes = 0usize;
    children
        .iter()
        .map(|c| match c.mime_id.as_deref() {
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
fn slug_base(name: &str) -> String {
    let mut base = String::new();
    let mut prev_dash = false;
    for c in name.trim().to_lowercase().chars() {
        if c.is_alphanumeric() {
            base.push(c);
            prev_dash = false;
        } else if !prev_dash {
            base.push('-');
            prev_dash = true;
        }
    }
    base.trim_matches('-').to_string()
}

/// Build a URL-safe node key from a display name plus a short unique suffix, so
/// a freshly created child does not collide with a sibling's key.
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
pub fn visible_sorted(children: &[graphql::ChildNodeFields]) -> Vec<graphql::ChildNodeFields> {
    let mut out: Vec<graphql::ChildNodeFields> = children
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
    use crate::graphql::{ChildNodeFields, MimeFields, Timestamptz, Uuid};

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
        assert_eq!(super::slug_base("Hello, World! 123"), "hello-world-123");
        assert_eq!(super::slug_base("  Trim -- Me  "), "trim-me");
        assert_eq!(super::slug_base("!!!"), "");
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
}

#[component]
fn NodeNotFound() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", span { class: "material-icons", "warning" } }
                h3 { class: "headline-small", "{t(\"node.documentUnavailable\")}" }
            }
            div { class: "card-content",
                p { class: "body-large mb-1", "{t(\"node.notFoundOrNoAccess\")}" }
                if !is_auth {
                    p { class: "body-large mb-2", "{t(\"node.maybeLoginForAccess\")}" }
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
