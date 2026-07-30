//! GraphQL subscriptions over the Hasura WebSocket endpoint
//! (the `graphql-transport-ws` protocol), so views update live instead of
//! polling.
//!
//! **One socket per client, not per subscription.** The protocol multiplexes by
//! design — every frame carries an `id` — and this app has 13 subscribing views,
//! so a delegate reading a motion with an open poll used to hold four to six
//! sockets. A congress of several hundred people made that a couple of thousand
//! connections against one Hasura, each with its own reconnect loop firing at
//! the same moment when the venue wifi dipped. Now it is one connection per
//! device, and a dip costs one reconnect.
//!
//! The hub owns the socket; hooks own only their place in its registry. That
//! registry is what survives a reconnect: on `connection_ack` every live
//! subscription is re-sent, and Hasura answers each with the current result, so
//! views recover without any bookkeeping of their own.

use std::cell::RefCell;
use std::collections::HashMap;
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

/// Subscribe to `query` and return a signal holding the latest `data` payload.
///
/// The hub connects on the first subscription and closes after the last one
/// goes, so a signed-out reader with no live views holds no socket at all.
pub fn use_graphql_subscription(query: String) -> Signal<Option<serde_json::Value>> {
    // Subscribing to the session hook keeps this component in the reactive
    // graph; the fresh token itself is re-read at each (re)connect below.
    let _session = use_session();
    let data = use_signal(|| None::<serde_json::Value>);

    let id = use_hook(|| Hub::subscribe(query.clone(), data, Runtime::current()));

    use_drop(move || {
        // Deregister BEFORE anything else: a frame already in flight for this id
        // must not find a signal whose scope is being torn down.
        Hub::unsubscribe(&id);
    });

    data
}

/// One registered subscription: what to ask for, and where to put the answers.
struct Sub {
    query: String,
    sink: Signal<Option<serde_json::Value>>,
}

/// The single connection, and every subscription riding on it.
#[derive(Default)]
struct HubState {
    ws: Option<WebSocket>,
    /// Set on `connection_ack`. Subscribe frames sent before that are dropped by
    /// the server, so they wait for it and go out together.
    acked: bool,
    /// Consecutive failed attempts since the last ack (drives the backoff);
    /// reset to 0 once a connection acks.
    attempts: u32,
    /// Handle of a scheduled reconnect, so it can be cancelled.
    timeout: Option<i32>,
    next_id: u64,
    subs: HashMap<String, Sub>,
    runtime: Option<Rc<Runtime>>,
}

thread_local! {
    static HUB: Rc<RefCell<HubState>> = Rc::new(RefCell::new(HubState::default()));
}

/// Namespace for the hub's operations; the state itself is thread-local, since
/// wasm is single-threaded and there is exactly one connection per document.
struct Hub;

impl Hub {
    fn with<R>(f: impl FnOnce(&mut HubState) -> R) -> R {
        HUB.with(|h| f(&mut h.borrow_mut()))
    }

    /// Register a subscription, connecting if this is the first one.
    fn subscribe(
        query: String,
        sink: Signal<Option<serde_json::Value>>,
        runtime: Rc<Runtime>,
    ) -> String {
        let (id, send_now) = Self::with(|st| {
            st.runtime.get_or_insert(runtime);
            st.next_id += 1;
            let id = st.next_id.to_string();
            st.subs.insert(
                id.clone(),
                Sub {
                    query: query.clone(),
                    sink,
                },
            );
            (id, st.acked)
        });
        if send_now {
            Self::send_subscribe(&id, &query);
        } else {
            Self::ensure_connected();
        }
        id
    }

    /// Deregister a subscription, and close the socket once none are left.
    fn unsubscribe(id: &str) {
        let (had, idle) = Self::with(|st| {
            let had = st.subs.remove(id).is_some();
            (had, st.subs.is_empty())
        });
        if had {
            Self::send(&json!({ "id": id, "type": "complete" }));
        }
        if idle {
            Self::with(|st| {
                if let (Some(win), Some(t)) = (web_sys::window(), st.timeout.take()) {
                    win.clear_timeout_with_handle(t);
                }
                st.acked = false;
                st.attempts = 0;
                st.ws.take()
            })
            .map(|ws| ws.close());
        }
    }

    fn send(msg: &serde_json::Value) {
        let ws = Self::with(|st| st.ws.clone());
        if let Some(ws) = ws {
            let _ = ws.send_with_str(&msg.to_string());
        }
    }

    fn send_subscribe(id: &str, query: &str) {
        Self::send(&json!({
            "id": id,
            "type": "subscribe",
            "payload": { "query": query },
        }));
    }

    /// Open the socket unless one is already open or a reconnect is pending.
    fn ensure_connected() {
        let needed = Self::with(|st| st.ws.is_none() && st.timeout.is_none());
        if needed {
            Self::connect();
        }
    }

    fn connect() {
        let ws_url = graphql_url()
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        let Ok(ws) = WebSocket::new_with_str(&ws_url, "graphql-transport-ws") else {
            // Could not even create the socket (e.g. offline): retry later.
            Self::schedule_reconnect();
            return;
        };
        Self::with(|st| st.ws = Some(ws.clone()));
        log::info!("subscription hub: connecting");

        // onopen -> connection_init. The bearer token is re-read from the session
        // at every (re)connect: a token captured at mount goes stale within
        // minutes (NHost tokens live ~15 min), so a reconnect hours in must not
        // reuse it.
        let on_open = {
            let ws = ws.clone();
            Closure::<dyn FnMut()>::new(move || {
                let token = Self::with(|st| st.runtime.clone()).map(|runtime| {
                    // The callback runs outside the Dioxus runtime; the guard
                    // makes reading the SESSION global legal here.
                    let _guard = RuntimeGuard::new(runtime);
                    crate::session::SESSION.peek().access_token.clone()
                });
                let payload = match token.flatten() {
                    Some(t) => json!({ "headers": { "Authorization": format!("Bearer {t}") } }),
                    None => json!({}),
                };
                let init = json!({ "type": "connection_init", "payload": payload });
                let _ = ws.send_with_str(&init.to_string());
            })
        };
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(txt) = e.data().as_string() else {
                return;
            };
            let Ok(msg) = serde_json::from_str::<serde_json::Value>(&txt) else {
                return;
            };
            let id = msg.get("id").and_then(|i| i.as_str()).unwrap_or_default();
            match msg.get("type").and_then(|t| t.as_str()) {
                Some("connection_ack") => {
                    // Every live subscription goes out now, including any that
                    // were registered while the socket was down. Hasura answers
                    // each with the current result, which is what makes a
                    // reconnect self-healing for the views.
                    let pending: Vec<(String, String)> = Self::with(|st| {
                        st.acked = true;
                        st.attempts = 0;
                        st.subs
                            .iter()
                            .map(|(id, s)| (id.clone(), s.query.clone()))
                            .collect()
                    });
                    for (id, query) in pending {
                        Self::send_subscribe(&id, &query);
                    }
                }
                // Keepalive: the server pings, we pong (graphql-transport-ws).
                Some("ping") => Self::send(&json!({ "type": "pong" })),
                Some("next") => {
                    let Some(d) = msg.get("payload").and_then(|p| p.get("data")) else {
                        return;
                    };
                    let target =
                        Self::with(|st| st.subs.get(id).map(|s| s.sink).zip(st.runtime.clone()));
                    if let Some((mut sink, runtime)) = target {
                        let _guard = RuntimeGuard::new(runtime);
                        sink.set(Some(d.clone()));
                    }
                }
                // A subscription the server refuses or ends. Rare, and previously
                // invisible: the view would simply stop updating with no trace.
                Some("error") => {
                    let detail = msg
                        .get("payload")
                        .map(|p| p.to_string())
                        .unwrap_or_default();
                    log::warn!("subscription {id} failed: {detail}");
                }
                _ => {}
            }
        });
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        // onclose -> reconnect while anything still wants data. Errors also end
        // in onclose, so scheduling only here avoids double reconnects.
        let on_close = Closure::<dyn FnMut()>::new(move || {
            let wanted = Self::with(|st| {
                st.acked = false;
                st.ws = None;
                !st.subs.is_empty()
            });
            if wanted {
                Self::schedule_reconnect();
            }
        });
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        // The socket owns these callbacks for its lifetime; leak them (three
        // small closures per connection, and there is now one connection).
        on_open.forget();
        on_message.forget();
        on_close.forget();
    }

    /// Schedule the next [`Self::connect`] with capped exponential backoff plus
    /// jitter, storing the handle so an idle hub can cancel it.
    fn schedule_reconnect() {
        let delay = Self::with(|st| {
            let d = backoff_delay_ms(st.attempts, js_sys::Math::random());
            st.attempts = st.attempts.saturating_add(1);
            d
        });
        let cb = Closure::once_into_js(move || {
            let wanted = Self::with(|st| {
                st.timeout = None;
                !st.subs.is_empty()
            });
            if wanted {
                Self::connect();
            }
        });
        if let Some(win) = web_sys::window() {
            if let Ok(id) =
                win.set_timeout_with_callback_and_timeout_and_arguments_0(cb.unchecked_ref(), delay)
            {
                Self::with(|st| st.timeout = Some(id));
            }
        }
    }
}

/// Backoff for the nth consecutive failure: 1s, 2s, 4s … capped at 30s, spread
/// by ±25%.
///
/// The jitter is the point at scale. Everyone in a hall shares one access point,
/// so a dip drops every device at the same instant; without spreading, they all
/// come back at the same instant too, and the server meets one synchronised
/// stampede after another. `rand` is a 0..1 sample, passed in so this is pure and
/// testable.
fn backoff_delay_ms(attempts: u32, rand: f64) -> i32 {
    let exp = attempts.min(5); // 2^5 * 1s = 32s pre-cap
    let base = ((1u32 << exp) * 1_000).min(30_000) as f64;
    let jitter = 0.75 + rand.clamp(0.0, 1.0) * 0.5;
    (base * jitter) as i32
}

#[cfg(test)]
mod tests {
    use super::backoff_delay_ms;

    #[test]
    fn backoff_doubles_then_caps() {
        // Mid-jitter (rand 0.5) is the base delay exactly.
        assert_eq!(backoff_delay_ms(0, 0.5), 1_000);
        assert_eq!(backoff_delay_ms(1, 0.5), 2_000);
        assert_eq!(backoff_delay_ms(2, 0.5), 4_000);
        assert_eq!(backoff_delay_ms(5, 0.5), 30_000);
        // Beyond the cap it stays there rather than growing without bound.
        assert_eq!(backoff_delay_ms(50, 0.5), 30_000);
    }

    #[test]
    fn jitter_spreads_the_stampede() {
        // A hall full of devices dropped at the same instant must not return at
        // the same instant: the window is ±25% and never zero.
        let lo = backoff_delay_ms(3, 0.0);
        let hi = backoff_delay_ms(3, 1.0);
        assert_eq!(lo, 6_000);
        assert_eq!(hi, 10_000);
        assert!(lo < hi, "jitter must actually spread the retries");
    }

    #[test]
    fn jitter_input_is_clamped() {
        // A bad sample cannot produce a negative or absurd delay.
        assert_eq!(backoff_delay_ms(0, -5.0), 750);
        assert_eq!(backoff_delay_ms(0, 5.0), 1_250);
    }
}
