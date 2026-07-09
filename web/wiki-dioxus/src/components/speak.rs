use dioxus::prelude::*;

use crate::graphql::{self, ChildNodeFields, Jsonb, NodeWithChildren, NodesInsertInput, Uuid};
use crate::i18n::t;
use crate::session::use_session;

/// SpeakApp — the speaker queue for a context.
///
/// A context holds a hidden `speak/list` child ("talerliste"); the queue itself
/// is that node's `speak/speak` children. This mirrors the React SpeakApp, which
/// reads `node.get("speakerlist")` and shows its entries.
#[component]
pub fn SpeakApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let is_auth = session.read().is_authenticated();

    // The speaker list is the context's (hidden) speak/list child.
    let list_id = node
        .children
        .iter()
        .find(|c| c.mime_id.as_deref() == Some("speak/list"))
        .map(|c| c.id.0.clone());
    let context_id = node.context_id.clone().unwrap_or_else(|| node.id.clone());

    // Re-fetch the queue after every join/remove by bumping this counter.
    let mut refresh = use_signal(|| 0u32);

    // Live updates: subscribe to the speaker list's entries over the Hasura
    // WebSocket, so entries added/removed by other participants appear at once.
    let sub_list = list_id
        .clone()
        .unwrap_or_else(|| "00000000-0000-0000-0000-000000000000".to_string());
    crate::subscription::use_live(
        format!(
            "subscription {{ nodes(where: {{ parentId: {{ _eq: \"{sub_list}\" }}, mimeId: {{ _eq: \"speak/speak\" }} }}) {{ id }} }}"
        ),
        refresh,
    );

    let access_token = session.read().access_token.clone();
    let list_for_query = list_id.clone();
    let queue = use_resource(move || {
        let token = access_token.clone();
        let list = list_for_query.clone();
        let _ = refresh.read();
        async move {
            let list = list?;
            let n = graphql::query_node_by_id(token.as_deref(), &list)
                .await
                .ok()??;
            let mut kids = n.children;
            // Queue order is arrival order (oldest first).
            kids.sort_by(|a, b| {
                let a_ts = a.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                let b_ts = b.created_at.as_ref().map(|t| t.0.as_str()).unwrap_or("");
                a_ts.cmp(b_ts)
            });
            // The speaker time limit lives on the list node's data.
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
            Some((time, updated_at, kids))
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

    let state = queue.read().clone().flatten();
    let speakers: Vec<ChildNodeFields> = state.as_ref().map(|s| s.2.clone()).unwrap_or_default();
    let remaining = state.as_ref().map_or(0, |(time, updated_at, _)| {
        let _ = tick.read();
        remaining_seconds(*time, updated_at)
    });

    rsx! {
        div { class: "grid grid-2",
            // Speaker list card
            div {
                div { class: "card",
                    div { class: "card-header",
                        div { class: "avatar secondary", "{super::loader::mime_icon(\"app/speak\")}" }
                        div {
                            h3 { class: "title-medium", "{node.name}" }
                            p {
                                class: "body-medium",
                                style: "color: var(--md-on-surface-variant);",
                                "{t(\"speak.speakerList\")}"
                            }
                        }
                        // Countdown for the current speaker's remaining time.
                        if remaining > 0 {
                            div { class: "flex-grow" }
                            div { class: "avatar secondary", title: "{t(\"speak.talk\")}",
                                "\u{23F1}\u{FE0F} {remaining}"
                            }
                        }
                    }
                    if list_id.is_none() {
                        div { class: "card-content",
                            p {
                                class: "body-medium",
                                style: "color: var(--md-on-surface-variant);",
                                "{t(\"speak.emptyList\")}"
                            }
                        }
                    } else if speakers.is_empty() {
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
                                div { class: "list-item", key: "{speaker.id.0}",
                                    div { class: "avatar small secondary", "{i + 1}" }
                                    div { class: "list-item-text",
                                        div { class: "list-item-primary", "{speaker.name}" }
                                    }
                                    if is_auth {
                                        {
                                            let speaker_id = speaker.id.0.clone();
                                            let token = session.read().access_token.clone();
                                            rsx! {
                                                button {
                                                    class: "btn-icon",
                                                    title: "{t(\"speak.removeFromList\")}",
                                                    onclick: move |_| {
                                                        let token = token.clone();
                                                        let id = speaker_id.clone();
                                                        spawn(async move {
                                                            let _ = graphql::delete_node(token.as_deref(), &id).await;
                                                            refresh += 1;
                                                        });
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
            }

            // Join panel — insert a speak/speak entry under the speaker list.
            div {
                if is_auth {
                    if let Some(list) = list_id.clone() {
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
                                                    let list = list.clone();
                                                    let context_id = context_id.clone();
                                                    let display_name = session.read().user.as_ref().map(|u| u.display_name.clone()).unwrap_or_default();
                                                    let token = session.read().access_token.clone();
                                                    rsx! {
                                                        button {
                                                            class: "btn btn-outlined",
                                                            onclick: move |_| {
                                                                let name = display_name.clone();
                                                                let key = format!("{}-{}", name.to_lowercase().replace(' ', "-"), now_ms());
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
                                                                            data: Some(Jsonb(serde_json::Value::String(type_val))),
                                                                            mutable: None,
                                                                            index: None,
                                                                        },
                                                                    ).await;
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
        }
    }
}

/// A millisecond timestamp for generating unique node keys.
fn now_ms() -> String {
    let window = web_sys::window().unwrap();
    let performance = window.performance().unwrap();
    format!("{:.0}", performance.now())
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
    use super::remaining_seconds_at;

    #[test]
    fn countdown_subtracts_elapsed_and_clamps() {
        // 60s limit, updated 10s ago -> 50 left.
        assert_eq!(remaining_seconds_at(60.0, 100_000.0, 110_000.0), 50);
        // Past the limit -> 0.
        assert_eq!(remaining_seconds_at(5.0, 100_000.0, 110_000.0), 0);
        // No limit set -> 0.
        assert_eq!(remaining_seconds_at(0.0, 100_000.0, 100_000.0), 0);
    }
}
