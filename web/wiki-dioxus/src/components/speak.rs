use dioxus::prelude::*;

use crate::graphql::{
    self, ChildNodeFields, Jsonb, NodeWithChildren, NodesInsertInput, NodesSetInput, Uuid,
};
use crate::i18n::t;
use crate::session::use_session;

/// Where a speaker list is shown. On the projector (`Screen`) view the admin and
/// join controls are hidden — it is read-only for the room.
#[derive(Clone, Copy, PartialEq)]
pub enum SpeakMode {
    /// The interactive app view: admin panel + join controls are shown.
    Full,
    /// The projector view: queue only, no controls.
    Screen,
}

/// SpeakApp — speaker queues for a context.
///
/// A context holds one or more hidden `speak/list` children ("talerliste"); each
/// list's queue is its `speak/speak` children. This mirrors the React SpeakApp
/// (`node.get("speakerlist")`), extended to render **every** list in the context
/// (#13), with an owner-only admin panel (#6), priority ordering, and a
/// current/next highlight (#14).
#[component]
pub fn SpeakApp(node: NodeWithChildren, mode: SpeakMode) -> Element {
    let session = use_session();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let screen = mode == SpeakMode::Screen;
    // The context owner may manage the list(s); Hasura still enforces the rest.
    let is_owner = user_id.is_some() && node.owner_id.as_ref().map(|o| o.0.clone()) == user_id;

    let context_id = node.context_id.clone().unwrap_or_else(|| node.id.clone());
    let lists: Vec<ChildNodeFields> = node
        .children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("speak/list"))
        .cloned()
        .collect();

    if lists.is_empty() {
        return rsx! {
            div { class: "card",
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"speak.emptyList\")}"
                    }
                }
            }
        };
    }

    rsx! {
        div { class: "stack stack-v",
            for list in lists {
                SpeakList {
                    key: "{list.id.0}",
                    list_id: list.id.0.clone(),
                    list_name: if list.name.is_empty() { node.name.clone() } else { list.name.clone() },
                    context_id: context_id.clone(),
                    is_owner,
                    screen,
                    current_user_id: user_id.clone(),
                }
            }
        }
    }
}

/// One speaker list: its live queue plus (for the owner) an admin panel and a
/// join panel.
#[component]
fn SpeakList(
    list_id: String,
    list_name: String,
    context_id: Uuid,
    is_owner: bool,
    screen: bool,
    current_user_id: Option<String>,
) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();
    let mut refresh = use_signal(|| 0u32);
    let mut time_box = use_signal(|| 120i32);

    // Live updates: subscribe to this list's entries over the Hasura WebSocket
    // so entries added/removed by anyone appear at once.
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{list_id}\" }}, mimeId: {{ _eq: \"speak/speak\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    let access_token = session.read().access_token.clone();
    let list_for_query = list_id.clone();
    let state = use_resource(move || {
        let token = access_token.clone();
        let list = list_for_query.clone();
        let _ = refresh.read();
        async move {
            let n = graphql::query_node_by_id(token.as_deref(), &list)
                .await
                .ok()??;
            let speakers = sorted_speakers(&n.children);
            // The speaking time limit + last-start live on the list node's data.
            let time = n
                .data
                .as_ref()
                .and_then(|d| d.0.get("time"))
                .and_then(|t| t.as_f64())
                .unwrap_or(0.0);
            let updated_at = n
                .data
                .as_ref()
                .and_then(|d| d.0.get("updatedAt"))
                .and_then(|t| t.as_str())
                .unwrap_or("")
                .to_string();
            Some((n.mutable, time, updated_at, speakers))
        }
    });

    // Tick once a second so the countdown updates (cancelled on unmount).
    let mut tick = use_signal(|| 0u32);
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(1000).await;
            tick += 1;
        }
    });

    let st = state.read().clone().flatten();
    let mutable = st.as_ref().map(|s| s.0).unwrap_or(false);
    let speakers: Vec<ChildNodeFields> = st.as_ref().map(|s| s.3.clone()).unwrap_or_default();
    let remaining = st.as_ref().map_or(0, |(_, time, updated_at, _)| {
        let _ = tick.read();
        remaining_seconds(*time, updated_at)
    });
    let running = st.as_ref().map(|s| s.1 > 0.0).unwrap_or(false);
    let min_index = speakers.iter().map(|s| s.index).min().unwrap_or(0);
    let max_index = speakers.iter().map(|s| s.index).max().unwrap_or(0);
    let count = speakers.len();

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div {
                    class: if mutable { "avatar secondary" } else { "avatar" },
                    title: if mutable { "{t(\"speak.open\")}" } else { "{t(\"speak.close\")}" },
                    if mutable { "\u{1F513}" } else { "\u{1F512}" }
                }
                div {
                    h3 { class: "title-medium", "{list_name}" }
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"speak.speakerList\")}"
                    }
                }
                if remaining > 0 {
                    div { class: "flex-grow" }
                    div { class: "chip-timer", title: "{t(\"speak.talk\")}",
                        "\u{23F1}\u{FE0F} {time_string(remaining)}"
                    }
                }
            }

            if speakers.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        style: "color: var(--md-on-surface-variant);",
                        "{t(\"speak.emptyList\")}"
                    }
                }
            } else {
                div { class: "list",
                    for (i , speaker) in speakers.iter().enumerate() {
                        {
                            let speaker_id = speaker.id.0.clone();
                            let name = speaker.name.clone();
                            let (type_icon , type_key) = speak_type_meta(speaker_type(speaker));
                            let secondary = match i {
                                0 => format!("{type_icon} {}", t("speak.speakingNow")),
                                1 => format!("{type_icon} {}", t("speak.next")),
                                _ => format!("{type_icon} {}", t(type_key)),
                            };
                            let row_class = match i {
                                0 => "list-item speak-current",
                                1 => "list-item speak-next",
                                _ => "list-item",
                            };
                            let can_remove = is_auth
                                && !screen
                                && (is_owner
                                    || speaker.owner_id.as_ref().map(|o| o.0.clone())
                                        == current_user_id);
                            rsx! {
                                div { class: "{row_class}", key: "{speaker_id}",
                                    div { class: "avatar small secondary", "{i + 1}" }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{name}" }
                                        div { class: "list-item-secondary", "{secondary}" }
                                    }
                                    // Owner reorder (#7): pull a speaker to the
                                    // front or back of the queue.
                                    if is_owner && !screen {
                                        button {
                                            class: "btn-icon",
                                            title: "{t(\"speak.moveUp\")}",
                                            disabled: i == 0,
                                            onclick: {
                                                let token = session.read().access_token.clone();
                                                let id = speaker_id.clone();
                                                move |_| {
                                                    let token = token.clone();
                                                    let id = id.clone();
                                                    spawn(async move {
                                                        let set = NodesSetInput {
                                                            index: Some(min_index - 1),
                                                            ..Default::default()
                                                        };
                                                        let _ = graphql::update_node(token.as_deref(), &id, set).await;
                                                        refresh += 1;
                                                    });
                                                }
                                            },
                                            "\u{2B06}"
                                        }
                                        button {
                                            class: "btn-icon",
                                            title: "{t(\"speak.moveDown\")}",
                                            disabled: i + 1 == count,
                                            onclick: {
                                                let token = session.read().access_token.clone();
                                                let id = speaker_id.clone();
                                                move |_| {
                                                    let token = token.clone();
                                                    let id = id.clone();
                                                    spawn(async move {
                                                        let set = NodesSetInput {
                                                            index: Some(max_index + 1),
                                                            ..Default::default()
                                                        };
                                                        let _ = graphql::update_node(token.as_deref(), &id, set).await;
                                                        refresh += 1;
                                                    });
                                                }
                                            },
                                            "\u{2B07}"
                                        }
                                    }
                                    if can_remove {
                                        button {
                                            class: "btn-icon",
                                            title: "{t(\"speak.removeFromList\")}",
                                            onclick: {
                                                let token = session.read().access_token.clone();
                                                let id = speaker_id.clone();
                                                move |_| {
                                                    let token = token.clone();
                                                    let id = id.clone();
                                                    spawn(async move {
                                                        let _ = graphql::delete_node(token.as_deref(), &id).await;
                                                        refresh += 1;
                                                    });
                                                }
                                            },
                                            "\u{2715}"
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        // Owner admin panel (#6): open/close the list, clear it, run the timer.
        if is_owner && !screen {
            div { class: "card",
                div { class: "card-content",
                    div { class: "stack stack-h", style: "gap: 8px; flex-wrap: wrap; align-items: center;",
                        button {
                            class: "btn btn-secondary",
                            onclick: {
                                let token = session.read().access_token.clone();
                                let id = list_id.clone();
                                move |_| {
                                    let token = token.clone();
                                    let id = id.clone();
                                    spawn(async move {
                                        let set = NodesSetInput {
                                            mutable: Some(!mutable),
                                            ..Default::default()
                                        };
                                        let _ = graphql::update_node(token.as_deref(), &id, set).await;
                                        refresh += 1;
                                    });
                                }
                            },
                            if mutable { "\u{1F512} {t(\"speak.close\")}" } else { "\u{1F513} {t(\"speak.open\")}" }
                        }
                        button {
                            class: "btn btn-outlined",
                            disabled: count == 0,
                            onclick: {
                                let token = session.read().access_token.clone();
                                let ids: Vec<String> = speakers.iter().map(|s| s.id.0.clone()).collect();
                                move |_| {
                                    let token = token.clone();
                                    let ids = ids.clone();
                                    spawn(async move {
                                        for id in ids {
                                            let _ = graphql::delete_node(token.as_deref(), &id).await;
                                        }
                                        refresh += 1;
                                    });
                                }
                            },
                            "\u{1F5D1}\u{FE0F} {t(\"speak.clear\")}"
                        }
                        button {
                            class: "btn btn-primary",
                            onclick: {
                                let token = session.read().access_token.clone();
                                let id = list_id.clone();
                                move |_| {
                                    let token = token.clone();
                                    let id = id.clone();
                                    // Start sets the limit + a fresh timestamp; stop zeroes it.
                                    let secs = if running { 0 } else { *time_box.read() };
                                    move_timer(token, id, secs, refresh);
                                }
                            },
                            if running { "\u{23F9}\u{FE0F} {t(\"speak.stop\")}" } else { "\u{25B6}\u{FE0F} {t(\"speak.start\")}" }
                        }
                        div { class: "text-field", style: "margin: 0; width: 120px;",
                            label { "{t(\"speak.speakingTime\")}" }
                            input {
                                r#type: "number",
                                min: "0",
                                value: "{time_box}",
                                oninput: move |e| {
                                    if let Ok(v) = e.value().parse::<i32>() {
                                        time_box.set(v);
                                    }
                                },
                            }
                        }
                    }
                }
            }
        }

        // Join panel — insert a speak/speak entry under this list.
        if is_auth && !screen && mutable {
            div { class: "card",
                div { class: "card-header",
                    h3 { class: "title-medium", "{t(\"speak.joinSpeakerList\")}" }
                }
                div { class: "card-content",
                    div { class: "stack stack-v",
                        {
                            let speak_types = [
                                ("0", t("speak.talk")),
                                ("1", t("speak.question")),
                                ("2", t("speak.clarify")),
                                ("3", t("speak.procedure")),
                            ];
                            rsx! {
                                for (type_key , label) in speak_types {
                                    {
                                        let list = list_id.clone();
                                        let context_id = context_id.clone();
                                        let display_name = session
                                            .read()
                                            .user
                                            .as_ref()
                                            .map(|u| u.display_name.clone())
                                            .unwrap_or_default();
                                        let token = session.read().access_token.clone();
                                        rsx! {
                                            button {
                                                class: "btn btn-outlined",
                                                onclick: move |_| {
                                                    let name = display_name.clone();
                                                    let key = format!(
                                                        "{}-{}",
                                                        name.to_lowercase().replace(' ', "-"),
                                                        now_ms(),
                                                    );
                                                    let parent = list.clone();
                                                    let ctx = context_id.clone();
                                                    let token = token.clone();
                                                    let type_val = type_key.to_string();
                                                    spawn(async move {
                                                        let _ = graphql::insert_node(
                                                                token.as_deref(),
                                                                NodesInsertInput {
                                                                    name: Some(name),
                                                                    key: Some(key),
                                                                    mime_id: Some("speak/speak".to_string()),
                                                                    parent_id: Some(Uuid(parent)),
                                                                    context_id: Some(ctx),
                                                                    data: Some(
                                                                        Jsonb(serde_json::Value::String(type_val)),
                                                                    ),
                                                                    mutable: None,
                                                                    index: None,
                                                                },
                                                            )
                                                            .await;
                                                        refresh += 1;
                                                    });
                                                },
                                                "{label}"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
    }
}

/// Start/stop the speaking timer by writing `{ time, updatedAt }` to the list's
/// data (updatedAt is the moment the clock started, used by the countdown).
fn move_timer(token: Option<String>, list_id: String, secs: i32, mut refresh: Signal<u32>) {
    spawn(async move {
        let data = serde_json::json!({ "time": secs, "updatedAt": now_iso() });
        let set = NodesSetInput {
            data: Some(Jsonb(data)),
            ..Default::default()
        };
        let _ = graphql::update_node(token.as_deref(), &list_id, set).await;
        refresh += 1;
    });
}

/// The speak type stored on a `speak/speak` node's data: `"0"`..`"3"` (talk,
/// question, clarify, procedure). Higher numbers are procedural and jump the
/// queue, matching the React `order_by: [{ data: desc }, { createdAt: asc }]`.
fn speaker_type(node: &ChildNodeFields) -> i64 {
    node.data
        .as_ref()
        .and_then(|d| match &d.0 {
            serde_json::Value::String(s) => s.parse::<i64>().ok(),
            serde_json::Value::Number(n) => n.as_i64(),
            _ => None,
        })
        .unwrap_or(0)
}

/// Icon + i18n key for a speak type.
fn speak_type_meta(kind: i64) -> (&'static str, &'static str) {
    match kind {
        3 => ("\u{2696}\u{FE0F}", "speak.procedure"),
        2 => ("\u{1F4A1}", "speak.clarify"),
        1 => ("\u{2753}", "speak.question"),
        _ => ("\u{1F5E3}\u{FE0F}", "speak.talk"),
    }
}

/// Order a list's `speak/speak` children the way the queue is served: an owner
/// override (`index`) first, then procedural priority (type desc), then arrival
/// time (createdAt asc). Default index 0 keeps the priority/arrival order.
fn sorted_speakers(children: &[ChildNodeFields]) -> Vec<ChildNodeFields> {
    let mut out: Vec<ChildNodeFields> = children
        .iter()
        .filter(|c| c.mime_id.as_deref() == Some("speak/speak"))
        .cloned()
        .collect();
    out.sort_by(|a, b| {
        a.index
            .cmp(&b.index)
            .then_with(|| speaker_type(b).cmp(&speaker_type(a)))
            .then_with(|| {
                let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                a_ts.cmp(b_ts)
            })
    });
    out
}

/// Format seconds as `MM:SS` (matches the React `timeString`).
fn time_string(total: i64) -> String {
    let total = total.max(0);
    format!("{:02}:{:02}", total / 60, total % 60)
}

/// A millisecond timestamp for generating unique node keys.
fn now_ms() -> String {
    let window = web_sys::window().unwrap();
    let performance = window.performance().unwrap();
    format!("{:.0}", performance.now())
}

/// The current wall-clock time as an ISO 8601 string (for the timer start).
fn now_iso() -> String {
    String::from(js_sys::Date::new_0().to_iso_string())
}

/// Seconds left on the current speaker's turn: the `time` limit minus the wall
/// clock elapsed since the list was last updated. Uses JS `Date` for the clock.
fn remaining_seconds(time: f64, updated_at: &str) -> i64 {
    remaining_seconds_at(time, js_sys::Date::parse(updated_at), js_sys::Date::now())
}

/// Pure countdown maths (testable off-browser).
fn remaining_seconds_at(time: f64, updated_ms: f64, now_ms: f64) -> i64 {
    if time <= 0.0 || updated_ms.is_nan() {
        return 0;
    }
    let elapsed = (now_ms - updated_ms) / 1000.0;
    let rem = time - elapsed;
    if rem > 0.0 {
        rem.floor() as i64
    } else {
        0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::graphql::{Jsonb, Timestamptz, Uuid};

    fn speaker(id: &str, kind: &str, index: i32, created: &str) -> ChildNodeFields {
        ChildNodeFields {
            id: Uuid(id.to_string()),
            name: id.to_string(),
            key: id.to_string(),
            mime_id: Some("speak/speak".to_string()),
            mutable: false,
            index,
            created_at: Some(Timestamptz(created.to_string())),
            owner_id: None,
            data: Some(Jsonb(serde_json::Value::String(kind.to_string()))),
            mime: None,
        }
    }

    #[test]
    fn countdown_subtracts_elapsed_and_clamps() {
        // 60s limit, updated 10s ago -> 50 left.
        assert_eq!(remaining_seconds_at(60.0, 100_000.0, 110_000.0), 50);
        // Past the limit -> 0.
        assert_eq!(remaining_seconds_at(5.0, 100_000.0, 110_000.0), 0);
        // No limit set -> 0.
        assert_eq!(remaining_seconds_at(0.0, 100_000.0, 100_000.0), 0);
    }

    #[test]
    fn time_string_is_mm_ss() {
        assert_eq!(time_string(0), "00:00");
        assert_eq!(time_string(9), "00:09");
        assert_eq!(time_string(75), "01:15");
        assert_eq!(time_string(-5), "00:00");
    }

    #[test]
    fn priority_then_arrival_then_owner_override() {
        // A later procedure (type 3) jumps ahead of an earlier talk (type 0);
        // same-type entries keep arrival order.
        let list = vec![
            speaker("talk-early", "0", 0, "2024-01-01T10:00:00Z"),
            speaker("talk-late", "0", 0, "2024-01-01T10:05:00Z"),
            speaker("procedure", "3", 0, "2024-01-01T10:10:00Z"),
        ];
        let order: Vec<String> = sorted_speakers(&list)
            .iter()
            .map(|s| s.id.0.clone())
            .collect();
        assert_eq!(order, ["procedure", "talk-early", "talk-late"]);

        // A negative index override pulls a talk to the very front.
        let mut list2 = list.clone();
        list2[1].index = -1; // talk-late pinned to top
        let order2: Vec<String> = sorted_speakers(&list2)
            .iter()
            .map(|s| s.id.0.clone())
            .collect();
        assert_eq!(order2, ["talk-late", "procedure", "talk-early"]);
    }
}
