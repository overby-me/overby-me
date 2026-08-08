//! The feedback app (`/?app=feedback`): browse feedback submissions. A home-
//! context owner sees ALL feedback (with the submitter) and may delete items; a
//! plain member sees only their own.
//!
//! That own-only limit is real, not just a filter on this list. The `nodes`
//! select rule grants a node to its owner, and to whoever owns or belongs to its
//! context. Feedback lives in the ROOT context, which has one member row, so a
//! member querying `wiki/feedback` directly gets back their own rows and nothing
//! else. (This note said the opposite until 2026-07-29, when the rule still
//! matched every authenticated user — worth knowing if you find older code that
//! assumed feedback was readable by anyone.)
//!
//! Feedback is composed from the user-menu dialog
//! ([`super::feedback::FeedbackDialog`]), which creates the `wiki/feedback`
//! nodes. Crashes arrive here too, as `kind = "crash"`, filed by the backend on
//! the app's behalf (`src/crash.rs`, `backend/src/feedback.rs`).

use dioxus::prelude::*;

use crate::components::widgets::Dialog;
use crate::graphql::{self, FeedbackItem};
use crate::i18n::{t, t_with};
use crate::model;
use crate::session::use_session;
use crate::snackbar::show_snackbar;

/// The reporter id the backend stores for someone with no account.
const ANONYMOUS: &str = "anonymous";

/// How many reporter chips a row shows before collapsing the rest into a count.
/// A crash that hit fifty people should say so without becoming fifty chips.
const MAX_REPORTER_CHIPS: usize = 6;

/// The filter chips: every kind that can appear, plus "all".
fn kind_filters() -> Vec<(String, String)> {
    [
        ("all", "member.filterAll"),
        ("crash", "feedback.crash"),
        ("bug", "feedback.bug"),
        ("feature", "feedback.feature"),
        ("other", "feedback.other"),
    ]
    .into_iter()
    .map(|(value, key)| (value.to_string(), t(key)))
    .collect()
}

/// The time-range choices, as (value, label).
fn since_options() -> Vec<(String, String)> {
    [
        ("any", "feedback.anyTime"),
        ("1", "feedback.lastDay"),
        ("7", "feedback.lastWeek"),
        ("30", "feedback.lastMonth"),
    ]
    .into_iter()
    .map(|(value, key)| (value.to_string(), t(key)))
    .collect()
}

/// The oldest timestamp a range admits, in epoch milliseconds; `None` for "any".
fn cutoff_ms(since: &str) -> Option<f64> {
    let days: f64 = since.parse().ok()?;
    // Server clock: the timestamps it is compared against are the database's.
    Some(crate::session::server_now_ms() - days * 24.0 * 60.0 * 60.0 * 1000.0)
}

fn matches_kind(item: &FeedbackItem, kind: &str) -> bool {
    // An auto-filed failure answers the "Bug" chip, matching how it is labelled.
    // Filtering to bugs and not being shown the ones the app reported itself is
    // the opposite of what that chip is for.
    kind == "all" || item.kind == kind || (kind == "bug" && item.kind == "error")
}

/// Free text against everything a person might search by: what was reported,
/// where it happened, which build, and who hit it.
fn matches_search(item: &FeedbackItem, needle: &str, people: &[model::Author]) -> bool {
    if needle.is_empty() {
        return true;
    }
    let named = |id: &String| {
        people
            .iter()
            .find(|p| p.user_id.as_ref() == Some(id))
            .map(|p| p.name.to_lowercase())
            .unwrap_or_default()
    };
    item.message.to_lowercase().contains(needle)
        || item.path.to_lowercase().contains(needle)
        || item.commit.to_lowercase().contains(needle)
        || item.owner_name.to_lowercase().contains(needle)
        || item.reporters.iter().any(|id| named(id).contains(needle))
}

/// Whether the report falls inside the range.
///
/// Measured from the LAST sighting, not the first. A crash that started weeks ago
/// and happened again this morning belongs in "last 24 hours" — the question the
/// range answers is what is going on now.
fn matches_since(item: &FeedbackItem, cutoff: Option<f64>) -> bool {
    let Some(cutoff) = cutoff else {
        return true;
    };
    let stamp = if item.last_seen.is_empty() {
        &item.created_at
    } else {
        &item.last_seen
    };
    let at = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(stamp)).get_time();
    at.is_nan() || at >= cutoff
}

/// The material icon + label key for a feedback kind.
fn kind_glyph(kind: &str) -> (&'static str, &'static str) {
    match kind {
        // `error` is a failure the app noticed and could only describe to the
        // person as "something went wrong". It is filed automatically, and it is
        // stored under its own kind so repeats fold by digest — but it IS a bug,
        // and reading as "Other" put the app's own failures in the drawer for
        // things that fit nowhere.
        "bug" | "error" => ("bug_report", "feedback.bug"),
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
        graphql::query_feedback(feed_token.as_deref()).await
    });
    let loading = items_res.read().is_none();
    // The screen already distinguishes "nothing matching your filter" from
    // "nothing at all"; a failed fetch was falling into the second and telling
    // people nobody had reported anything.
    let load = items_res.read().clone();
    let failed = matches!(load, Some(Err(_)));
    if let Some(Err(e)) = &load {
        crate::errors::log_handled("feedback load failed", e);
    }
    let mut items = load.and_then(|r| r.ok()).unwrap_or_default();

    // Everyone named on any crash, resolved in ONE query for the whole list.
    // Per row it would be a request per crash, and per reporter it would be a
    // request per person.
    let reporter_ids: Vec<String> = {
        let mut ids: Vec<String> = items
            .iter()
            .flat_map(|it| it.reporters.iter().cloned())
            .collect();
        ids.sort();
        ids.dedup();
        ids
    };
    let people_token = session.read().access_token.clone();
    let people_key = reporter_ids.join(",");
    let people_res = crate::use_data_resource!(|(people_token, people_key)| async move {
        let ids: Vec<String> = people_key
            .split(',')
            .filter(|s| !s.is_empty())
            .map(str::to_string)
            .collect();
        graphql::query_users_by_ids(people_token.as_deref(), &ids).await
    });
    let people: Vec<model::Author> = people_res.read().clone().unwrap_or_default();
    // Members see only their own (cosmetic — see the module doc); owners see all.
    if !is_owner {
        items.retain(|it| it.owner_id.is_some() && it.owner_id == my_id);
    }

    // Filtering is client-side because the whole list is already here:
    // `query_feedback` fetches it unpaginated, feedback volume being low. If that
    // stops being true this moves to Hasura, the way the roster did.
    let kind = use_signal(|| "all".to_string());
    let search = use_signal(String::new);
    let since = use_signal(|| "any".to_string());
    let total_before = items.len();
    let needle = search.read().trim().to_lowercase();
    let cutoff = cutoff_ms(&since.read());
    items.retain(|it| {
        matches_kind(it, &kind.read())
            && matches_search(it, &needle, &people)
            && matches_since(it, cutoff)
    });
    let filtered = items.len() != total_before;

    if !is_auth {
        return rsx! {
            div { class: "card",
                div { class: "empty-state empty-state-sm",
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
                    crate::errors::log_handled("delete feedback failed", e);
                    show_snackbar(&t("error.somethingWentWrong"));
                }
            }
        });
    };

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar small", {super::loader::feedback_icon_el()} }
                h3 { class: "title-medium",
                    if is_owner { "{t(\"feedback.all\")}" } else { "{t(\"feedback.yours\")}" }
                }
                div { class: "flex-grow" }
                // Composing lives here now, rather than as a second user-menu row
                // that opened a dialog from anywhere.
                if crate::components::feedback::FEEDBACK_ENABLED {
                    button {
                        class: "btn btn-primary",
                        onclick: move |_| {
                            *crate::components::feedback::FEEDBACK_OPEN.write() = true;
                        },
                        span { class: "material-icons", "add" }
                        span { class: "feedback-compose-label", "{t(\"feedback.menu\")}" }
                    }
                }
            }
            div { class: "card-content",
                // The same toolbar the member roster uses, so the two screens
                // filter alike rather than merely resemble each other.
                if !loading && total_before > 0 {
                    super::widgets::FilterToolbar {
                        search,
                        filter: kind,
                        filters: kind_filters(),
                        search_placeholder: t("feedback.searchPlaceholder"),
                        // A bare `select`, which the stylesheet already dresses;
                        // a wrapper class here would style nothing.
                        trailing: rsx! {
                            select {
                                aria_label: t("feedback.timeRange"),
                                value: "{since}",
                                onchange: {
                                    let mut since = since;
                                    move |e: FormEvent| since.set(e.value())
                                },
                                for (value , label) in since_options() {
                                    option { key: "{value}", value: "{value}", "{label}" }
                                }
                            }
                        },
                    }
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
                            {super::loader::feedback_icon_el()}
                        }
                        p { class: "empty-state-body",
                            // Nothing matching and nothing at all are different
                            // situations: one is answered by changing the filter.
                            if filtered {
                                "{t(\"common.noResults\")}"
                            } else if is_owner {
                                "{t(\"feedback.empty\")}"
                            } else {
                                "{t(\"feedback.emptyMine\")}"
                            }
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
                                people: people.clone(),
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
    /// A frame in JavaScript. Always generated code — wasm-bindgen's glue and the
    /// bundler's runtime — since this app hand-writes no JavaScript beyond the
    /// service worker. Nothing here will ever name your code, so it is shown as
    /// the scaffolding it is.
    Js(String),
    Plain(String),
}

/// Whether `text` reads as `path/to/file.rs:123` rather than a function name.
///
/// The backend emits a bare location when it resolved a line but no name, and a
/// bare name when it resolved a name but no line. They arrive in the same shape,
/// so this is what tells them apart — otherwise a location was styled as if it
/// were a function.
fn looks_like_location(text: &str) -> bool {
    let Some((path, line)) = text.rsplit_once(':') else {
        return false;
    };
    !line.is_empty()
        && line.bytes().all(|b| b.is_ascii_digit())
        && path.contains('/')
        && !path.contains(' ')
}

/// Split a crash report into lines that can be styled.
fn parse_stack(message: &str) -> Vec<StackLine> {
    // A panic message can run to several lines, and only the first says
    // "panicked at" — the rest is what was being asserted. They belong with it,
    // so everything before the first frame reads as part of the panic.
    let mut seen_frame = false;
    message
        .lines()
        .map(|line| {
            let trimmed = line.trim_start();
            if let Some(rest) = trimmed.strip_prefix("at ") {
                seen_frame = true;
                // `function (file:line)` — the location is the trailing
                // parenthesised group, and a function name may itself contain
                // parentheses (`call_mut<fn(Props) -> ...>`), so find the LAST.
                if let Some(open) = rest.rfind(" (") {
                    if rest.ends_with(')') {
                        let location = rest[open + 2..rest.len() - 1].to_string();
                        // A JavaScript frame that reached here in Chrome's shape.
                        if location.contains("://") {
                            return StackLine::Js(trimmed.to_string());
                        }
                        let app = location.starts_with("src/");
                        return StackLine::Frame {
                            function: rest[..open].to_string(),
                            location,
                            app,
                        };
                    }
                }
                // One half only. Which half decides how it is coloured.
                if looks_like_location(rest) {
                    return StackLine::Frame {
                        function: String::new(),
                        app: rest.starts_with("src/"),
                        location: rest.to_string(),
                    };
                }
                return StackLine::Frame {
                    function: rest.to_string(),
                    location: String::new(),
                    app: false,
                };
            }
            if trimmed.contains("wasm-function[") {
                seen_frame = true;
                return StackLine::Raw(trimmed.to_string());
            }
            // Firefox writes `name@url:line:col`, and the bundler's own frames
            // arrive the same way.
            if trimmed.contains("://") && trimmed.contains('@') {
                seen_frame = true;
                return StackLine::Js(trimmed.to_string());
            }
            if !seen_frame {
                return StackLine::Panic(trimmed.to_string());
            }
            StackLine::Plain(line.to_string())
        })
        .collect()
}

/// Split an auto-filed failure into the sentence a person can read and the
/// payload a developer needs, pretty-printed.
///
/// These arrive as a line of prose followed by whatever the server said, which
/// for a GraphQL error is a single line of deeply nested JSON carrying the whole
/// generated SQL statement. Unreadable as a paragraph: no wrapping a browser
/// does to one 4 KB line makes the `message` field findable.
///
/// The payload is returned even when it does not parse — a report is clamped at
/// a maximum length, so a big one arrives with its JSON cut mid-string, and that
/// truncated tail is still worth reading in a monospace block rather than being
/// dropped or run together with the prose.
fn split_payload(message: &str) -> (String, Option<String>) {
    let Some(at) = message.find(['{', '[']) else {
        return (message.to_string(), None);
    };
    let (head, tail) = message.split_at(at);
    let pretty = serde_json::from_str::<serde_json::Value>(tail.trim())
        .ok()
        .and_then(|v| serde_json::to_string_pretty(&v).ok())
        .unwrap_or_else(|| tail.trim().to_string());
    (head.trim_end().to_string(), Some(pretty))
}

/// An auto-filed failure: the sentence, then its payload as a code block.
#[component]
fn ErrorDetail(message: String) -> Element {
    let (head, payload) = split_payload(&message);
    rsx! {
        if !head.is_empty() {
            p { class: "body-medium text-preserve-breaks feedback-message", "{head}" }
        }
        if let Some(payload) = payload {
            pre { class: "feedback-payload", "{payload}" }
        }
    }
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
                            if !function.is_empty() {
                                span { class: "stack-fn", "{function}" }
                            }
                            if !location.is_empty() {
                                span { class: "stack-loc",
                                    if function.is_empty() { "{location}" } else { " {location}" }
                                }
                            }
                        }
                    },
                    StackLine::Js(text) => rsx! {
                        div { key: "{i}", class: "stack-line stack-js", "{text}" }
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

/// A whole report, as text to paste into an editor, an issue, or an agent.
///
/// Deliberately English and deliberately markdown, whatever the reader's
/// language: this is not read in the app, it is pasted into something that
/// expects a bug report. The message goes in a fenced block so a stack survives
/// the paste, and everything else goes above it because "which build" and
/// "which page" are the questions that get asked first.
fn report_text(item: &FeedbackItem, reporters: &[(String, String, String)]) -> String {
    let kind = match item.kind.as_str() {
        "crash" => "Crash",
        "error" => "Failure",
        "bug" => "Bug report",
        "idea" => "Idea",
        _ => "Feedback",
    };
    let mut out = format!("# {kind} in the wiki\n\n");
    let mut fact = |label: &str, value: &str| {
        if !value.trim().is_empty() {
            out.push_str(&format!("- {label}: {}\n", value.trim()));
        }
    };
    fact("when", &item.created_at);
    // Only when it says something the line above does not: a report seen once
    // was last seen when it was made.
    if item.seen > 1 && item.last_seen != item.created_at {
        fact("last seen", &item.last_seen);
        fact("how often", &format!("{} times", item.seen));
    }
    fact("where", &item.path);
    fact("build", &item.commit);
    fact("browser", &item.user_agent);
    let names: Vec<&str> = reporters.iter().map(|(_, name, _)| name.as_str()).collect();
    if !names.is_empty() {
        fact("reported by", &names.join(", "));
    }
    fact("report id", &item.id);
    out.push_str("\n```\n");
    out.push_str(item.message.trim_end());
    out.push_str("\n```\n");
    out
}

/// One feedback submission row.
#[component]
fn FeedbackRow(
    item: FeedbackItem,
    show_owner: bool,
    can_delete: bool,
    /// Identities for every reporter across the whole list, resolved once by
    /// [`FeedbackApp`]. Not all of them belong to this row.
    people: Vec<model::Author>,
    on_delete: EventHandler<String>,
) -> Element {
    let (icon, label_key) = kind_glyph(&item.kind);
    // Date AND time, in the reader's own timezone. Slicing the first ten
    // characters off the ISO string was not just imprecise: the timestamp is
    // UTC and Denmark runs ahead of it, so anything sent in the first hour or
    // two after local midnight was still the previous day in UTC and was shown
    // as such. `full_datetime` converts and localises.
    let when = super::loader::full_datetime(&item.created_at);
    let when_ago = super::loader::relative_time(&item.created_at);
    // The date on the row is when it was FIRST seen; a folded crash also has a
    // most recent sighting, which is the one that says whether it is still going.
    let last_seen = super::loader::full_datetime(&item.last_seen);

    // (id, display name, avatar) per reporter. Someone the viewer may not read —
    // the `users` select rule wants a shared context — still gets a chip, because
    // "one more person" is worth knowing even without a name.
    let reporters: Vec<(String, String, String)> = item
        .reporters
        .iter()
        .map(|id| {
            if id == ANONYMOUS {
                return (id.clone(), t("feedback.anonymous"), String::new());
            }
            match people.iter().find(|p| p.user_id.as_deref() == Some(id)) {
                Some(p) => (id.clone(), p.name.clone(), p.avatar_url.clone()),
                None => (id.clone(), t("common.unknown"), String::new()),
            }
        })
        .collect();
    let screenshot = super::loader::use_file_object_url(item.image.clone().unwrap_or_default());

    rsx! {
        div { class: "feedback-item",
            // Wraps: a full timestamp is a good deal wider than a bare date, and
            // on a phone it would otherwise squeeze the kind chip.
            div { class: "stack stack-h stack-wrap",
                // The chip carries the same glyph and names it, so an avatar
                // beside it was the icon twice over.
                span { class: "chip", span { class: "material-icons", "{icon}" }
                    span { class: "chip-label", "{t(label_key)}" }
                }
                div { class: "flex-grow" }
                // How often, and to how many. Repeats fold into this row rather
                // than adding rows, so without this a crash hitting fifty people
                // is indistinguishable from one that happened once.
                if item.seen > 1 {
                    span {
                        class: "chip feedback-seen",
                        title: "{t_with(\"feedback.lastSeen\", &[(\"when\", &last_seen)])}",
                        span { class: "material-icons", "repeat" }
                        span { class: "chip-label",
                            if reporters.len() > 1 {
                                "{t_with(\"feedback.seenByPeople\", &[(\"count\", &item.seen.to_string()), (\"people\", &reporters.len().to_string())])}"
                            } else {
                                "{t_with(\"feedback.seenTimes\", &[(\"count\", &item.seen.to_string())])}"
                            }
                        }
                    }
                }
                if !when.is_empty() {
                    // Exact on the face of it, with "3 hours ago" a hover away —
                    // the reverse of the pattern elsewhere, because a report is
                    // read to work out what someone was doing at the time.
                    span { class: "body-small text-muted", title: "{when_ago}", "{when}" }
                }
                // A report is for pasting somewhere else — an editor, an issue,
                // a message to whoever owns the code, an agent — and selecting
                // forty wrapped monospace lines by hand on a phone is not that.
                // The whole report goes, not just the stack: what was on screen,
                // which build, which browser, how often and to how many. Those
                // are the first four questions anyone asked about a crash, and
                // the answers were all on this row and none of them in the copy.
                button {
                    class: "btn-icon",
                    title: "{t(\"feedback.copyReport\")}",
                    onclick: {
                        let text = report_text(&item, &reporters);
                        move |_| {
                            let text = text.clone();
                            spawn(async move {
                                // Awaited, not fired and forgotten: the
                                // clipboard can refuse (permissions, an
                                // insecure context) and saying "copied" when
                                // nothing was is worse than saying nothing.
                                let copied = match web_sys::window() {
                                    Some(win) => wasm_bindgen_futures::JsFuture::from(
                                        win.navigator().clipboard().write_text(&text),
                                    )
                                    .await
                                    .is_ok(),
                                    None => false,
                                };
                                show_snackbar(&t(if copied {
                                    "feedback.reportCopied"
                                } else {
                                    "error.somethingWentWrong"
                                }));
                            });
                        }
                    },
                    span { class: "material-icons", "content_copy" }
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
            // A crash is a stack and an auto-filed failure is a payload; neither
            // is prose, and neither reads as prose. Everything a person typed
            // stays prose.
            if item.kind == "crash" {
                CrashStack { message: item.message.clone() }
            } else if item.kind == "error" {
                ErrorDetail { message: item.message.clone() }
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
                // A crash names everyone who hit it, not just whoever hit it
                // first — that is the difference between "someone had a problem"
                // and "eleven people are having this problem". Typed feedback has
                // one author and keeps the single chip.
                if show_owner && !reporters.is_empty() {
                    for who in reporters.iter().take(MAX_REPORTER_CHIPS) {
                        super::loader::UserPopover {
                            key: "{who.0}",
                            name: who.1.clone(),
                            avatar_url: who.2.clone(),
                            user_id: (who.0 != ANONYMOUS).then(|| who.0.clone()),
                            span { class: "chip",
                                span { class: "material-icons", "person" }
                                span { class: "chip-label", "{who.1}" }
                            }
                        }
                    }
                    if reporters.len() > MAX_REPORTER_CHIPS {
                        span { class: "chip",
                            span { class: "chip-label",
                                "{t_with(\"feedback.andMore\", &[(\"count\", &(reporters.len() - MAX_REPORTER_CHIPS).to_string())])}"
                            }
                        }
                    }
                } else if show_owner {
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
    fn a_copied_report_carries_what_gets_asked_about_first() {
        let item = FeedbackItem {
            id: "abc-123".into(),
            kind: "crash".into(),
            message: "panicked at 'already borrowed'\n  at pdf.rs:602".into(),
            path: "/radikal_ungdom/landsmøde_2026".into(),
            commit: "46d43887".into(),
            user_agent: "Mozilla/5.0 (Android 15)".into(),
            seen: 12,
            created_at: "2026-08-01T09:12:00Z".into(),
            last_seen: "2026-08-05T14:32:00Z".into(),
            ..Default::default()
        };
        let who = vec![("u1".to_string(), "Marie".to_string(), String::new())];
        let text = report_text(&item, &who);
        for wanted in [
            "# Crash in the wiki",
            "- when: 2026-08-01T09:12:00Z",
            "- last seen: 2026-08-05T14:32:00Z",
            "- how often: 12 times",
            "- where: /radikal_ungdom/landsmøde_2026",
            "- build: 46d43887",
            "- browser: Mozilla/5.0 (Android 15)",
            "- reported by: Marie",
            "- report id: abc-123",
        ] {
            assert!(text.contains(wanted), "the report left out {wanted:?}:\n{text}");
        }
        // The stack survives the paste, which is what the fence is for.
        assert!(text.contains("```\npanicked at 'already borrowed'\n  at pdf.rs:602\n```"));
    }

    #[test]
    fn a_report_leaves_out_what_it_does_not_know() {
        // Anything sent before the app recorded builds and browsers, and a
        // report nobody has hit twice: an empty line reads as a missing fact,
        // and a missing fact is better left unsaid than said blankly.
        let item = FeedbackItem {
            kind: "idea".into(),
            message: "A page control for Word documents".into(),
            created_at: "2026-07-01T10:00:00Z".into(),
            last_seen: "2026-07-01T10:00:00Z".into(),
            seen: 1,
            ..Default::default()
        };
        let text = report_text(&item, &[]);
        assert!(text.starts_with("# Idea in the wiki"));
        for absent in ["- build:", "- browser:", "- where:", "- how often:", "- last seen:", "- reported by:"] {
            assert!(!text.contains(absent), "the report claimed {absent:?} it does not have:\n{text}");
        }
    }

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
    fn javascript_frames_are_recognised_in_both_engine_shapes() {
        // Firefox: name@url:line:col, including the bundler's own frames.
        let parsed = parse_stack(
            "panicked at src/x.rs:1:1: boom\n\
             Z/__wbg_new_1f236d63ba0c4784/<@https://radikal.wiki/assets/wiki-dioxus-dxh1.js:1:42935\n\
             dt@https://radikal.wiki/assets/wiki-dioxus-dxh1.js:1:64939",
        );
        assert!(matches!(parsed[1], StackLine::Js(_)));
        assert!(matches!(parsed[2], StackLine::Js(_)));
        // Chrome: at name (url:line:col).
        let chrome = parse_stack("    at Module.foo (https://radikal.wiki/assets/x.js:1:5)");
        assert!(matches!(chrome[0], StackLine::Js(_)));
    }

    #[test]
    fn a_frame_that_resolved_only_a_location_is_not_shown_as_a_function() {
        // The backend emits a bare location when it found a line but no name.
        let parsed = parse_stack("    at dioxus-core-0.7.9/src/any_props.rs:75");
        assert_eq!(
            parsed,
            vec![StackLine::Frame {
                function: String::new(),
                location: "dioxus-core-0.7.9/src/any_props.rs:75".into(),
                app: false,
            }]
        );
        // And a bare name, when it found the name but no line, still reads as one.
        let named = parse_stack("    at render_inner");
        assert!(matches!(&named[0], StackLine::Frame { location, .. } if location.is_empty()));
    }

    #[test]
    fn a_multi_line_panic_message_stays_with_the_panic() {
        // The message runs past the first line; only the first says "panicked at".
        let parsed = parse_stack(
            "panicked at src/components/error.rs:20:5:\n\
             Triggered test panic from /error\n\
             \u{20}   at render (src/components/folder.rs:224)",
        );
        assert!(matches!(parsed[0], StackLine::Panic(_)));
        assert!(matches!(parsed[1], StackLine::Panic(_)));
        assert!(matches!(parsed[2], StackLine::Frame { .. }));
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

    /// The reported shape: a sentence, then one line of nested JSON. The JSON
    /// becomes a tree, and the prose stays prose.
    #[test]
    fn a_graphql_failure_splits_into_prose_and_a_tree() {
        let msg = r#"graphql error (raw vars): [{"extensions":{"code":"unexpected","internal":{"error":{"message":"rate limited: retry_after_ms=54977","status_code":"P0001"}}},"message":"database query error"}]"#;
        let (head, payload) = split_payload(msg);
        assert_eq!(head, "graphql error (raw vars):");
        let payload = payload.expect("the JSON is the point of the report");
        assert!(payload.contains('\n'), "pretty-printed, not one line");
        // The two lines someone is actually looking for each end up on a line of
        // their own, indented to their depth rather than buried mid-string.
        assert!(payload
            .lines()
            .any(|l| l.trim() == "\"message\": \"rate limited: retry_after_ms=54977\","));
        assert!(payload
            .lines()
            .any(|l| l.trim() == "\"message\": \"database query error\""));
        assert!(
            payload.lines().count() > 10,
            "one line per field, not one line total"
        );
    }

    /// A report clamped at the storage limit arrives with its JSON cut in half.
    /// The tail is still what someone needs to read, so it is kept as-is rather
    /// than dropped or run together with the sentence.
    #[test]
    fn a_truncated_payload_is_kept_verbatim() {
        let (head, payload) = split_payload(r#"graphql error: [{"extensions":{"code":"unexp"#);
        assert_eq!(head, "graphql error:");
        assert_eq!(payload.as_deref(), Some(r#"[{"extensions":{"code":"unexp"#));
    }

    /// Something a person typed has no payload and must not grow a code block,
    /// even if they used a brace.
    #[test]
    fn prose_is_left_alone() {
        assert_eq!(
            split_payload("the vote button did nothing"),
            ("the vote button did nothing".to_string(), None)
        );
        let (head, payload) = split_payload("it printed {weird} at me");
        assert_eq!(head, "it printed");
        assert_eq!(payload.as_deref(), Some("{weird} at me"));
    }

    /// An auto-filed failure is a bug, and answers the Bug chip.
    #[test]
    fn an_auto_filed_failure_reads_as_a_bug() {
        assert_eq!(kind_glyph("error"), ("bug_report", "feedback.bug"));
        assert_eq!(kind_glyph("error"), kind_glyph("bug"));
        let item = FeedbackItem {
            kind: "error".into(),
            ..Default::default()
        };
        assert!(matches_kind(&item, "bug"), "the Bug filter must show it");
        assert!(matches_kind(&item, "all"));
        assert!(!matches_kind(&item, "feature"));
    }
}
