use dioxus::prelude::*;

use crate::graphql::{self, NodeWithChildren, NodesSetInput};
use crate::i18n::t;
use crate::session::use_session;

/// RedirectApp — a node that forwards to an external URL stored on its `data`
/// (#57). Reachable via `?app=redirect`. When a target is set it shows the link
/// and forwards after a short delay; the context owner can set/change it.
#[component]
pub fn RedirectApp(node: NodeWithChildren) -> Element {
    let session = use_session();
    let user_id = session.read().user.as_ref().map(|u| u.id.clone());
    let is_owner = user_id.is_some() && node.owner_id.as_ref().map(|o| o.0.clone()) == user_id;

    let target = redirect_url(node.data.as_ref().map(|d| &d.0));
    let node_id = node.id.0.clone();

    // Forward automatically once a valid target is known.
    {
        let target = target.clone();
        use_effect(move || {
            if let Some(url) = target.clone() {
                spawn(async move {
                    gloo_timers::future::TimeoutFuture::new(1500).await;
                    if let Some(w) = web_sys::window() {
                        let _ = w.location().set_href(&url);
                    }
                });
            }
        });
    }

    rsx! {
        div { class: "card",
            div { class: "card-header",
                div { class: "avatar", {super::loader::icon_el("app/redirect")} }
                h3 { class: "title-medium", "{node.name}" }
            }
            div { class: "card-content",
                match target {
                    Some(url) => rsx! {
                        // EXPERIMENT: a tonal forwarding banner with the morphing loader.
                        div { class: "tonal-banner",
                            div { class: "spinner", style: "width: 22px; height: 22px;" }
                            span { class: "body-medium", "{t(\"redirect.forwarding\")}" }
                        }
                        a {
                            class: "btn btn-primary",
                            href: "{url}",
                            target: "_blank",
                            rel: "noopener",
                            "{url}"
                        }
                    },
                    None => rsx! {
                        // EXPERIMENT: orb empty state for a redirect with no target set.
                        div { class: "empty-state empty-state-sm",
                            div { class: "empty-state-orb empty-state-orb-sm",
                                span { class: "material-icons", "link_off" }
                            }
                            p { class: "empty-state-body", "{t(\"redirect.noTarget\")}" }
                        }
                    },
                }
                if is_owner {
                    RedirectEdit { node_id, initial: node.data.as_ref().and_then(|d| redirect_url(Some(&d.0))).unwrap_or_default() }
                }
            }
        }
    }
}

/// Owner form to set the redirect target on the node's data.
#[component]
fn RedirectEdit(node_id: String, initial: String) -> Element {
    let session = use_session();
    let mut url = use_signal(|| initial.clone());
    let mut saving = use_signal(|| false);

    rsx! {
        div { class: "stack stack-h mt-2", style: "gap: 8px; align-items: flex-end;",
            div { class: "text-field", style: "flex-grow: 1; margin: 0;",
                label { "{t(\"redirect.targetUrl\")}" }
                input {
                    r#type: "url",
                    value: "{url}",
                    oninput: move |e| url.set(e.value()),
                }
            }
            button {
                class: "btn btn-primary",
                disabled: *saving.read(),
                onclick: {
                    let node_id = node_id.clone();
                    move |_| {
                        let node_id = node_id.clone();
                        let token = session.read().access_token.clone();
                        let value = url.read().trim().to_string();
                        saving.set(true);
                        spawn(async move {
                            let set = NodesSetInput {
                                data: Some(graphql::Jsonb(serde_json::json!({ "url": value }))),
                                ..Default::default()
                            };
                            let _ = graphql::update_node(token.as_deref(), &node_id, set).await;
                            if let Some(w) = web_sys::window() {
                                let _ = w.location().reload();
                            }
                        });
                    }
                },
                "{t(\"common.save\")}"
            }
        }
    }
}

/// Extract a redirect URL from a node's data: either the raw string, or the
/// `url` field of an object. Only `http(s)` targets are honoured.
fn redirect_url(data: Option<&serde_json::Value>) -> Option<String> {
    let raw = match data? {
        serde_json::Value::String(s) => s.clone(),
        serde_json::Value::Object(o) => o.get("url")?.as_str()?.to_string(),
        _ => return None,
    };
    let raw = raw.trim();
    if raw.starts_with("http://") || raw.starts_with("https://") {
        Some(raw.to_string())
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_url_reads_string_and_object_and_rejects_non_http() {
        assert_eq!(
            redirect_url(Some(&serde_json::json!("https://a.dk"))),
            Some("https://a.dk".to_string())
        );
        assert_eq!(
            redirect_url(Some(&serde_json::json!({ "url": "http://b.dk" }))),
            Some("http://b.dk".to_string())
        );
        assert_eq!(
            redirect_url(Some(&serde_json::json!("javascript:alert(1)"))),
            None
        );
        assert_eq!(redirect_url(Some(&serde_json::json!({}))), None);
        assert_eq!(redirect_url(None), None);
    }
}
