use dioxus::prelude::*;

use crate::graphql::NodeWithChildren;
use crate::i18n::t;
use crate::route::Route;

use super::loader::{icon_el, mime_icon, visible_sorted};

/// ProgramApp — an agenda / programme view of a context (#60). Renders the
/// context's ordered children as a numbered vertical timeline, each item linking
/// into its node.
#[component]
pub fn ProgramApp(node: NodeWithChildren, path: Vec<String>) -> Element {
    let items = visible_sorted(&node.children);

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {icon_el("app/program")} }
                h3 { class: "title-medium", "{node.name}" }
            }
            if items.is_empty() {
                div { class: "card-content",
                    p {
                        class: "body-medium",
                        class: "text-muted",
                        "{t(\"program.empty\")}"
                    }
                }
            } else {
                div { class: "program-timeline",
                    for (i , item) in items.iter().enumerate() {
                        {
                            let mut full = path.clone();
                            full.push(item.key.clone());
                            let icon = mime_icon(&super::loader::node_icon_mime_id(
                                item.mime_id.as_deref().unwrap_or(""),
                                item.data.as_ref().map(|d| &d.0),
                            ));
                            let time = program_time(item.data.as_ref().map(|d| &d.0));
                            rsx! {
                                Link {
                                    key: "{item.id.0}",
                                    to: Route::PathPage { segments: full, app: None },
                                    class: "program-item",
                                    div { class: "program-index", "{i + 1}" }
                                    div { class: "program-body",
                                        div { class: "list-item-primary",
                                            span {
                                                class: "material-icons",
                                                style: "vertical-align: middle; font-size: 18px; margin-right: 6px;",
                                                "{icon}"
                                            }
                                            "{item.name}"
                                        }
                                        if let Some(t) = time {
                                            div {
                                                class: "list-item-secondary",
                                                class: "text-muted",
                                                "{t}"
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

/// A human agenda time from a node's data, if present (`time`, `start`, or
/// `when` string fields).
fn program_time(data: Option<&serde_json::Value>) -> Option<String> {
    let obj = data?.as_object()?;
    for key in ["when", "start", "time"] {
        if let Some(v) = obj.get(key).and_then(|v| v.as_str()) {
            if !v.is_empty() {
                return Some(v.to_string());
            }
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn program_time_reads_known_fields() {
        assert_eq!(
            program_time(Some(&serde_json::json!({ "when": "10:00" }))),
            Some("10:00".to_string())
        );
        assert_eq!(
            program_time(Some(&serde_json::json!({ "start": "09:30" }))),
            Some("09:30".to_string())
        );
        assert_eq!(program_time(Some(&serde_json::json!({}))), None);
        assert_eq!(program_time(None), None);
    }
}
