//! Minimal GraphQL subscription client over the Hasura WebSocket endpoint
//! (the `graphql-transport-ws` protocol), so views can update live instead of
//! polling. Only what this app needs: connection_init with the bearer token,
//! one subscribe, and the latest `data` payload surfaced as a Dioxus signal.

use dioxus::core::{Runtime, RuntimeGuard};
use dioxus::prelude::*;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

use crate::nhost::graphql_url;
use crate::session::use_session;

/// Subscribe to `query` and return a signal holding the latest `data` payload.
/// The socket is opened once for the component and closed when it unmounts.
pub fn use_graphql_subscription(query: String) -> Signal<Option<serde_json::Value>> {
    let session = use_session();
    let token = session.read().access_token.clone();
    let data = use_signal(|| None::<serde_json::Value>);

    let socket = use_hook(|| open_subscription(query.clone(), token.clone(), data));

    use_drop({
        let socket = socket.clone();
        move || {
            if let Some(ws) = &socket {
                let _ = ws.close();
            }
        }
    });

    data
}

/// Open the socket, wire the handshake/subscribe/data flow, and return it (so
/// the caller can close it). Returns None if the socket cannot be created.
fn open_subscription(
    query: String,
    token: Option<String>,
    mut data: Signal<Option<serde_json::Value>>,
) -> Option<WebSocket> {
    let ws_url = graphql_url()
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let ws = WebSocket::new_with_str(&ws_url, "graphql-transport-ws").ok()?;

    // The onmessage callback runs outside the Dioxus runtime; capture it so
    // signal writes are legal there (a RuntimeGuard restores the context).
    let runtime = Runtime::current();

    // onopen -> connection_init (with the bearer token in the payload headers).
    let on_open = {
        let ws = ws.clone();
        let token = token.clone();
        Closure::<dyn FnMut()>::new(move || {
            let payload = match &token {
                Some(t) => json!({ "headers": { "Authorization": format!("Bearer {t}") } }),
                None => json!({}),
            };
            let init = json!({ "type": "connection_init", "payload": payload });
            let _ = ws.send_with_str(&init.to_string());
        })
    };
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    // connection_ack -> subscribe; next -> push the data into the signal.
    let on_message = {
        let ws = ws.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(txt) = e.data().as_string() else {
                return;
            };
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(&txt) else {
                return;
            };
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("connection_ack") => {
                    let sub = json!({
                        "id": "1",
                        "type": "subscribe",
                        "payload": { "query": query },
                    });
                    let _ = ws.send_with_str(&sub.to_string());
                }
                // Keepalive: the server pings, we pong (graphql-transport-ws).
                Some("ping") => {
                    let _ = ws.send_with_str(&json!({ "type": "pong" }).to_string());
                }
                Some("next") => {
                    if let Some(d) = msg.get("payload").and_then(|p| p.get("data")) {
                        let _guard = RuntimeGuard::new(runtime.clone());
                        data.set(Some(d.clone()));
                    }
                }
                _ => {}
            }
        })
    };
    ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

    // The socket owns these callbacks for its lifetime; leak them (the component
    // opens at most one subscription and closes the socket on unmount).
    on_open.forget();
    on_message.forget();

    Some(ws)
}
