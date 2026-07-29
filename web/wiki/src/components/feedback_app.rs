//! The feedback app (`/?app=feedback`): browse feedback submissions. A home-
//! context owner sees ALL feedback (with the submitter) and may delete items; a
//! plain member sees only their own. NOTE: the own-only filtering is COSMETIC —
//! the `nodes` select rule is open to any authenticated user, so a member could
//! still read others' feedback via a raw query. It's a low-sensitivity report
//! inbox, so this is a deliberate simplification (no restrictive select rule).
//! Feedback is composed from the user-menu dialog
//! ([`super::feedback::FeedbackDialog`]), which creates the `wiki/feedback` nodes.

use dioxus::prelude::*;

use crate::components::widgets::Dialog;
use crate::graphql::{self, FeedbackItem};
use crate::i18n::t;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// The material icon + label key for a feedback kind.
fn kind_glyph(kind: &str) -> (&'static str, &'static str) {
    match kind {
        "bug" => ("bug_report", "feedback.bug"),
        "feature" => ("lightbulb", "feedback.feature"),
        // Not offered in the dialog: a crash files itself (src/crash.rs), and
        // telling it apart from a bug someone sat down and wrote matters, since
        // one carries a stack and the other carries an account of what happened.
        "crash" => ("error", "feedback.crash"),
        _ => ("chat", "feedback.other"),
    }
}

#[component]
pub fn FeedbackApp() -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let my_id = session.read().user.as_ref().map(|u| u.id.clone());

    // Confirm-delete state: the id of the item awaiting confirmation.
    let mut confirm_delete = use_signal(|| None::<String>);

    // Whether the caller owns the home context (→ sees all feedback + can delete).
    let owner_token = session.read().access_token.clone();
    let owner_res = crate::use_data_resource!(|(owner_token)| async move {
        graphql::query_root_node(owner_token.as_deref())
            .await
            .ok()
            .flatten()
            .and_then(|n| n.is_context_owner)
            .unwrap_or(false)
    });
    let is_owner = (*owner_res.read()).unwrap_or(false);

    let feed_token = session.read().access_token.clone();
    let items_res = crate::use_data_resource!(|(feed_token)| async move {
        graphql::query_feedback(feed_token.as_deref())
            .await
            .unwrap_or_default()
    });
    let loading = items_res.read().is_none();
    let mut items = items_res.read().clone().unwrap_or_default();
    // Members see only their own (cosmetic — see the module doc); owners see all.
    if !is_owner {
        items.retain(|it| it.owner_id.is_some() && it.owner_id == my_id);
    }

    if !is_auth {
        return rsx! {
            div { class: "card",
                div { class: "empty-state",
                    div { class: "empty-state-orb empty-state-orb-sm",
                        span { class: "material-icons", "lock" }
                    }
                    p { class: "empty-state-body", "{t(\"node.documentUnavailable\")}" }
                }
            }
        };
    }

    let delete_item = move |id: String| {
        let token = session.read().access_token.clone();
        spawn(async move {
            match graphql::delete_node(token.as_deref(), &id).await {
                Ok(_) => crate::session::bump_data_version(),
                Err(e) => {
                    log::error!("delete feedback failed: {e}");
                    show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small", span { class: "material-icons", "feedback" } }
                h3 { class: "title-medium",
                    if is_owner { "{t(\"feedback.all\")}" } else { "{t(\"feedback.yours\")}" }
                }
            }
            div { class: "card-content",
                if loading {
                    super::widgets::Spinner {}
                } else if items.is_empty() {
                    div { class: "empty-state empty-state-sm",
                        div { class: "empty-state-orb empty-state-orb-sm",
                            span { class: "material-icons", "feedback" }
                        }
                        p { class: "empty-state-body",
                            if is_owner { "{t(\"feedback.empty\")}" } else { "{t(\"feedback.emptyMine\")}" }
                        }
                    }
                } else {
                    div { class: "list",
                        for item in items.iter() {
                            FeedbackRow {
                                key: "{item.id}",
                                item: item.clone(),
                                show_owner: is_owner,
                                can_delete: is_owner,
                                on_delete: move |id| confirm_delete.set(Some(id)),
                            }
                        }
                    }
                }
            }
        }

        // Delete confirmation.
        Dialog {
            open: confirm_delete.read().is_some(),
            on_dismiss: move |_| confirm_delete.set(None),
            headline: t("common.delete"),
            icon: "delete".to_string(),
            actions: rsx! {
                button {
                    class: "btn btn-outlined",
                    onclick: move |_| confirm_delete.set(None),
                    "{t(\"common.cancel\")}"
                }
                button {
                    class: "btn btn-primary",
                    onclick: move |_| {
                        if let Some(id) = confirm_delete.write().take() {
                            delete_item(id);
                        }
                    },
                    "{t(\"common.delete\")}"
                }
            },
            p { class: "body-medium", "{t(\"feedback.deleteConfirm\")}" }
        }
    }
}

/// One line of a crash report, as it should be shown.
///
/// A resolved stack is structured text, and the structure is worth keeping: the
/// point of reading one is to find the frame that belongs to THIS repo among the
/// standard library and dependency frames it is buried in. Everything else is
/// context.
#[derive(Debug, PartialEq)]
enum StackLine {
    /// The panic itself, which is the first line and the reason for the rest.
    Panic(String),
    /// A resolved frame: a function, and the file and line it came from. `app`
    /// marks a file in this repo, which is what someone is actually looking for
    /// (`trim_path` in the backend keeps the owning crate on everything else).
    Frame {
        function: String,
        location: String,
        app: bool,
    },
    /// A frame the backend could not resolve, left as the browser wrote it. Kept,
    /// because a gap in a stack is worth seeing, but it is noise.
    Raw(String),
    Plain(String),
}

/// Split a crash report into lines that can be styled.
fn parse_stack(message: &str) -> Vec<StackLine> {
    message
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("at ") {
                // `function (file:line)` — the location is the trailing
                // parenthesised group, and a function name may itself contain
                // parentheses (`call_mut<fn(Props) -> ...>`), so find the LAST.
                if let Some(open) = rest.rfind(" (") {
                    if rest.ends_with(')') {
                        let function = rest[..open].to_string();
                        let location = rest[open + 2..rest.len() - 1].to_string();
                        let app = location.starts_with("src/");
                        return StackLine::Frame {
                            function,
                            location,
                            app,
                        };
                    }
                }
                return StackLine::Frame {
                    function: rest.to_string(),
                    location: String::new(),
                    app: false,
                };
            }
            if trimmed.contains("wasm-function[") {
                return StackLine::Raw(trimmed.to_string());
            }
            if trimmed.starts_with("panicked at") {
                return StackLine::Panic(trimmed.to_string());
            }
            StackLine::Plain(line.to_string())
        })
        .collect()
}

/// A crash report, rendered as the stack it is.
#[component]
fn CrashStack(message: String) -> Element {
    rsx! {
        div { class: "feedback-stack",
            for (i , line) in parse_stack(&message).into_iter().enumerate() {
                match line {
                    StackLine::Panic(text) => rsx! {
                        div { key: "{i}", class: "stack-line stack-panic", "{text}" }
                    },
                    StackLine::Frame { function, location, app } => rsx! {
                        div {
                            key: "{i}",
                            class: if app { "stack-line stack-frame stack-app" } else { "stack-line stack-frame" },
                            span { class: "stack-fn", "{function}" }
                            if !location.is_empty() {
                                span { class: "stack-loc", " {location}" }
                            }
                        }
                    },
                    StackLine::Raw(text) => rsx! {
                        div { key: "{i}", class: "stack-line stack-raw", "{text}" }
                    },
                    StackLine::Plain(text) => rsx! {
                        div { key: "{i}", class: "stack-line", "{text}" }
                    },
                }
            }
        }
    }
}

/// One feedback submission row.
#[component]
fn FeedbackRow(
    item: FeedbackItem,
    show_owner: bool,
    can_delete: bool,
    on_delete: EventHandler<String>,
) -> Element {
    let (icon, label_key) = kind_glyph(&item.kind);
    let date: String = item.created_at.chars().take(10).collect();
    let screenshot = super::loader::use_file_object_url(item.image.clone().unwrap_or_default());

    rsx! {
        div { class: "feedback-item",
            div { class: "stack stack-h",
                // The chip carries the same glyph and names it, so an avatar
                // beside it was the icon twice over.
                span { class: "chip", span { class: "material-icons", "{icon}" }
                    span { class: "chip-label", "{t(label_key)}" }
                }
                div { class: "flex-grow" }
                if !date.is_empty() {
                    span { class: "body-small text-muted", "{date}" }
                }
                if can_delete {
                    button {
                        class: "btn-icon",
                        title: "{t(\"common.delete\")}",
                        onclick: {
                            let id = item.id.clone();
                            move |_| on_delete.call(id.clone())
                        },
                        span { class: "material-icons", "delete" }
                    }
                }
            }
            // A crash is a stack, not prose, and reads as one. Everything a
            // person typed stays prose.
            if item.kind == "crash" {
                CrashStack { message: item.message.clone() }
            } else {
                p { class: "body-medium text-preserve-breaks feedback-message", "{item.message}" }
            }
            if let Some(url) = screenshot {
                // The anchor carries the spacing, not the image: an inline
                // anchor's line box ignores its child's margin-top, so the
                // screenshot sat flush against the message.
                a {
                    class: "feedback-screenshot",
                    href: "{url}",
                    target: "_blank",
                    rel: "noopener",
                    img {
                        class: "zoomable",
                        src: "{url}",
                        alt: t("feedback.screenshot"),
                        loading: "lazy",
                    }
                }
            }
            div { class: "stack stack-h stack-wrap feedback-meta",
                if show_owner {
                    super::loader::UserPopover {
                        name: if item.owner_name.is_empty() { t("feedback.anonymous") } else { item.owner_name.clone() },
                        avatar_url: item.owner_avatar.clone(),
                        user_id: item.owner_id.clone(),
                        span { class: "chip",
                            span { class: "material-icons", "person" }
                            span { class: "chip-label",
                                "{t(\"feedback.submittedBy\")}: "
                                if item.owner_name.is_empty() { "{t(\"feedback.anonymous\")}" } else { "{item.owner_name}" }
                            }
                        }
                    }
                }
                if !item.path.is_empty() {
                    span { class: "body-small text-muted", "{item.path}" }
                }
                // Which build it came from. Absent on anything submitted before
                // builds recorded it, and on a build made outside the deploy
                // path, which reports `unknown` rather than guessing.
                if !item.commit.is_empty() && item.commit != "unknown" {
                    span { class: "body-small text-muted feedback-commit", "{item.commit}" }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_frame_in_this_repo_is_marked_as_such() {
        let parsed = parse_stack("    at render (src/components/folder.rs:224)");
        assert_eq!(
            parsed,
            vec![StackLine::Frame {
                function: "render".into(),
                location: "src/components/folder.rs:224".into(),
                app: true,
            }]
        );
    }

    #[test]
    fn a_dependency_frame_is_not() {
        // The backend keeps the owning crate on anything outside this repo, which
        // is exactly what tells the two apart.
        let parsed = parse_stack("    at new (alloc/src/boxed.rs:289)");
        assert!(matches!(&parsed[0], StackLine::Frame { app: false, .. }));
    }

    #[test]
    fn a_generic_function_keeps_its_own_parentheses() {
        // The reason the location is found from the END: a monomorphised name
        // contains parentheses of its own, and splitting on the first would cut
        // the function in half.
        let parsed =
            parse_stack("    at call_mut<fn(Props) -> Element> (core/src/ops/function.rs:166)");
        assert_eq!(
            parsed,
            vec![StackLine::Frame {
                function: "call_mut<fn(Props) -> Element>".into(),
                location: "core/src/ops/function.rs:166".into(),
                app: false,
            }]
        );
    }

    #[test]
    fn the_panic_and_unresolved_frames_are_recognised() {
        let parsed = parse_stack(
            "panicked at src/x.rs:1:1: boom\n\
             @https://radikal.wiki/assets/wiki-dioxus_bg-dxh1.wasm:wasm-function[5300]:0x36e775",
        );
        assert!(matches!(parsed[0], StackLine::Panic(_)));
        assert!(matches!(parsed[1], StackLine::Raw(_)));
    }
}
