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

/// Resolves a path to a node and renders the matching app. Remounted per path.
#[component]
fn PathResolver(segments: Vec<String>, app: Option<String>) -> Element {
    let session = use_session();
    let access_token = session.read().access_token.clone();
    let segments_clone = segments.clone();

    let node_future = use_resource(move || {
        let token = access_token.clone();
        let segs = segments_clone.clone();
        async move { graphql::resolve_path(token.as_deref(), &segs).await }
    });

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

/// Mime type to icon glyph. Mirrors the React `IconId` (core/mime.tsx) mapping
/// from mime id to a Material icon, using the closest emoji equivalent so the
/// drawer, folder list and headers show the same icon the reference app does.
pub fn mime_icon(mime_id: &str) -> &'static str {
    match mime_id {
        // wiki/*
        "wiki/search" => "\u{1F50D}",                // Search 🔍
        "wiki/home" | "app/home" => "\u{1F3E0}",     // Home 🏠
        "wiki/group" | "app/member" => "\u{1F465}",  // Group 👥
        "wiki/event" => "\u{1F4C5}",                 // Event 📅
        "wiki/folder" | "app/folder" => "\u{1F4C1}", // Folder 📁
        "wiki/document" => "\u{1F4C4}",              // Article 📄
        "wiki/file" => "\u{1F4CE}",                  // UploadFile 📎
        "wiki/user" => "\u{1F464}",                  // Person 👤
        "text/plain" => "\u{1F4C3}",                 // Subject 📃
        // vote/*
        "vote/policy" => "\u{2696}\u{FE0F}", // Gavel ⚖️
        "vote/position" => "\u{1F64B}",      // HowToReg 🙋
        "vote/candidate" => "\u{1F642}",     // Face 🙂
        "vote/question" => "\u{2753}",       // QuestionMark ❓
        "vote/comment" => "\u{1F4AC}",       // AddComment 💬
        "vote/change" => "\u{1F4DD}",        // RateReview 📝
        "vote/poll" => "\u{1F4CA}",          // Poll 📊
        // speak / apps
        "speak/list" | "app/speak" => "\u{1F5E3}\u{FE0F}", // RecordVoiceOver/Interpreter 🗣️
        "app/editor" => "\u{270F}\u{FE0F}",                // Edit ✏️
        "app/sort" => "\u{2195}\u{FE0F}",                  // LowPriority ↕️
        "app/vote" => "\u{1F5F3}\u{FE0F}",                 // HowToVote 🗳️
        "app/search" => "\u{1F50D}",                       // Search 🔍
        "app/screen" => "\u{1F4FA}",                       // ConnectedTv 📺
        "application/pdf" => "\u{1F4D5}",                  // PDF 📕
        "app/map" | "map/map" => "\u{1F5FA}\u{FE0F}",      // Map 🗺️
        "app/graph" => "\u{1F578}\u{FE0F}",                // Graph/web 🕸️
        "app/program" => "\u{1F5D3}\u{FE0F}",              // Programme 🗓️
        "app/profile" => "\u{1F464}",                      // Profile 👤
        "app/social" => "\u{1F98B}",                       // Social (Bluesky) 🦋
        "app/redirect" => "\u{21AA}\u{FE0F}",              // Redirect ↪️
        "app/cow" => "\u{1F404}",                          // Cow 🐄
        "app/parent" => "\u{1F9F9}",                       // Missing parent (cleanup) 🧹
        _ => mime_icon_by_prefix(mime_id),
    }
}

/// Fallback icons for the media / office families the React app matches by
/// substring (image/, audio/, video/, spreadsheet, presentation, document).
fn mime_icon_by_prefix(mime_id: &str) -> &'static str {
    if mime_id.contains("image/") {
        "\u{1F5BC}\u{FE0F}" // Image 🖼️
    } else if mime_id.contains("audio/") {
        "\u{1F3B5}" // MusicNote 🎵
    } else if mime_id.contains("video/") {
        "\u{1F3AC}" // Video 🎬
    } else if mime_id.contains("spreadsheet") {
        "\u{1F4D7}" // Excel 📗
    } else if mime_id.contains("presentation") {
        "\u{1F4D9}" // PowerPoint 📙
    } else if mime_id.contains("document") {
        "\u{1F4D8}" // Word 📘
    } else {
        "\u{2753}" // QuestionMark ❓
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
                div { class: "avatar", "\u{26A0}" }
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
