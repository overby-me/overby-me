//! Minimal GraphQL subscription client over the Hasura WebSocket endpoint
//! (the `graphql-transport-ws` protocol), so views can update live instead of
//! polling. Only what this app needs: connection_init with the bearer token,
//! one subscribe, and the latest `data` payload surfaced as a Dioxus signal.

use std::rc::Rc;

use dioxus::core::{Runtime, RuntimeGuard};
use dioxus::prelude::*;
use serde_json::json;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

use crate::nhost::graphql_url;
use crate::session::use_session;

/// Wire a subscription to a component's refresh counter: every pushed update
/// bumps `refresh`, so a `use_resource` keyed on it re-fetches live. This is the
/// common pattern — the payload itself is ignored; the query just needs to
/// cover the rows whose change should trigger a refresh. Also refreshes when the
/// window regains focus (#122), so a view recovers immediately if its socket was
/// dropped while the tab was in the background.
pub fn use_live(query: String, mut refresh: Signal<u32>) {
    let sub = use_graphql_subscription(query);
    use_effect(move || {
        // Reading the subscription signal ties this effect to each push.
        let _ = sub.read();
        refresh += 1;
    });
    use_focus_refresh(refresh);
}

/// Bump `refresh` whenever the window regains focus (and the tab is visible), so
/// data re-fetches on return to the app (#122). The listener is removed when the
/// component unmounts.
pub fn use_focus_refresh(mut refresh: Signal<u32>) {
    let runtime = Runtime::current();
    #[allow(clippy::type_complexity)]
    let handle: Option<(web_sys::Window, Rc<Closure<dyn FnMut()>>)> = use_hook(move || {
        let window = web_sys::window()?;
        let cb = Rc::new(Closure::<dyn FnMut()>::new(move || {
            // Skip if the tab is hidden (focus can fire on a background tab).
            let hidden = web_sys::window()
                .and_then(|w| w.document())
                .map(|d| d.hidden())
                .unwrap_or(false);
            if !hidden {
                let _guard = RuntimeGuard::new(runtime.clone());
                refresh += 1;
            }
        }));
        window
            .add_event_listener_with_callback("focus", (*cb).as_ref().unchecked_ref())
            .ok()?;
        Some((window, cb))
    });
    use_drop(move || {
        if let Some((window, cb)) = &handle {
            let _ = window
                .remove_event_listener_with_callback("focus", (**cb).as_ref().unchecked_ref());
        }
    });
}

/// Per-subscription connection state: the live socket, the retry counter, and
/// whether the owning component is still mounted. Deliberately per-socket (NO
/// shared connection manager): the rewrite's single multiplexed WebSocket lifts
/// this state machine wholesale, and multiplexing stays a rewrite concern.
struct SubState {
    ws: Option<WebSocket>,
    /// Consecutive failed attempts since the last `connection_ack` (drives the
    /// exponential backoff); reset to 0 once a connection acks.
    attempts: u32,
    /// Cleared on unmount so a straggling onclose/timeout never reconnects.
    alive: bool,
    /// Handle of a scheduled reconnect, so unmount can cancel it.
    timeout: Option<i32>,
}

/// Subscribe to `query` and return a signal holding the latest `data` payload.
/// The socket opens when the component mounts, RECONNECTS with capped
/// exponential backoff if it drops (server restart, venue wifi blip: without
/// this, a long-lived view like the projector silently freezes, because the
/// only other recovery path is the window-focus refresh and a projector tab
/// never refocuses), and closes for good when the component unmounts.
/// Resubscribing after a reconnect makes Hasura push the current result as the
/// first `next`, so dependent views refetch without any extra bookkeeping.
pub fn use_graphql_subscription(query: String) -> Signal<Option<serde_json::Value>> {
    // Subscribing to the session hook keeps this component in the reactive
    // graph; the fresh token itself is re-read at each (re)connect below.
    let _session = use_session();
    let data = use_signal(|| None::<serde_json::Value>);

    let state = use_hook(|| {
        let state = Rc::new(std::cell::RefCell::new(SubState {
            ws: None,
            attempts: 0,
            alive: true,
            timeout: None,
        }));
        connect(query.clone(), data, Runtime::current(), state.clone());
        state
    });

    use_drop({
        let state = state.clone();
        move || {
            let mut st = state.borrow_mut();
            st.alive = false;
            if let Some(id) = st.timeout.take() {
                if let Some(win) = web_sys::window() {
                    win.clear_timeout_with_handle(id);
                }
            }
            if let Some(ws) = st.ws.take() {
                let _ = ws.close();
            }
        }
    });

    data
}

/// Open the socket and wire the handshake/subscribe/data flow. On close (any
/// reason but unmount) schedules a reconnect via [`schedule_reconnect`].
fn connect(
    query: String,
    mut data: Signal<Option<serde_json::Value>>,
    runtime: Rc<Runtime>,
    state: Rc<std::cell::RefCell<SubState>>,
) {
    let ws_url = graphql_url()
        .replacen("https://", "wss://", 1)
        .replacen("http://", "ws://", 1);
    let Ok(ws) = WebSocket::new_with_str(&ws_url, "graphql-transport-ws") else {
        // Could not even create the socket (e.g. offline): retry later.
        schedule_reconnect(query, data, runtime, state);
        return;
    };
    state.borrow_mut().ws = Some(ws.clone());

    // onopen -> connection_init. The bearer token is re-read from the session
    // at every (re)connect: a token captured at mount goes stale within minutes
    // (NHost tokens live ~15 min), so a reconnect hours in must not reuse it.
    let on_open = {
        let ws = ws.clone();
        let runtime = runtime.clone();
        Closure::<dyn FnMut()>::new(move || {
            let token = {
                // The callback runs outside the Dioxus runtime; the guard makes
                // reading the SESSION global legal here.
                let _guard = RuntimeGuard::new(runtime.clone());
                crate::session::SESSION.peek().access_token.clone()
            };
            let payload = match &token {
                Some(t) => json!({ "headers": { "Authorization": format!("Bearer {t}") } }),
                None => json!({}),
            };
            let init = json!({ "type": "connection_init", "payload": payload });
            let _ = ws.send_with_str(&init.to_string());
        })
    };
    ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

    // connection_ack -> subscribe (and reset the backoff); next -> push the
    // data into the signal. The onmessage callback runs outside the Dioxus
    // runtime; a RuntimeGuard restores the context for signal writes.
    let on_message = {
        let ws = ws.clone();
        let query = query.clone();
        let runtime = runtime.clone();
        let state = state.clone();
        Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(txt) = e.data().as_string() else {
                return;
            };
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(&txt) else {
                return;
            };
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("connection_ack") => {
                    state.borrow_mut().attempts = 0;
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

    // onclose -> reconnect (unless the component unmounted). Errors also end in
    // onclose, so scheduling only here avoids double reconnects.
    let on_close = {
        let state = state.clone();
        Closure::<dyn FnMut()>::new(move || {
            if state.borrow().alive {
                schedule_reconnect(query.clone(), data, runtime.clone(), state.clone());
            }
        })
    };
    ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

    // The socket owns these callbacks for its lifetime; leak them (a few small
    // closures per (re)connect, bounded by reconnect frequency).
    on_open.forget();
    on_message.forget();
    on_close.forget();
}

/// Schedule the next [`connect`] attempt with capped exponential backoff
/// (1s, 2s, 4s, ... capped at 30s), storing the timeout handle so unmount can
/// cancel it.
fn schedule_reconnect(
    query: String,
    data: Signal<Option<serde_json::Value>>,
    runtime: Rc<Runtime>,
    state: Rc<std::cell::RefCell<SubState>>,
) {
    let delay_ms = {
        let mut st = state.borrow_mut();
        st.ws = None;
        let exp = st.attempts.min(5); // 2^5 * 1s = 32s pre-cap
        st.attempts = st.attempts.saturating_add(1);
        ((1u32 << exp) * 1_000).min(30_000) as i32
    };
    let cb = {
        let state = state.clone();
        Closure::once_into_js(move || {
            let pending = {
                let mut st = state.borrow_mut();
                st.timeout = None;
                st.alive
            };
            if pending {
                connect(query, data, runtime, state);
            }
        })
    };
    if let Some(win) = web_sys::window() {
        if let Ok(id) =
            win.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), delay_ms)
        {
            state.borrow_mut().timeout = Some(id);
        }
    }
}
