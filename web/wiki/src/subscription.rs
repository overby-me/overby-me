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
    let coalescer = use_hook(|| Rc::new(RefCell::new(Coalescer::default())));
    let timer: Rc<RefCell<Option<i32>>> = use_hook(|| Rc::new(RefCell::new(None)));
    let runtime = Runtime::current();

    use_effect({
        let coalescer = coalescer.clone();
        let timer = timer.clone();
        move || {
            // Reading the subscription signal ties this effect to each push.
            let pushed = sub.read().is_some();
            // The effect also runs once at mount, before anything has arrived.
            // Refreshing then re-fetches data the view has only just loaded —
            // two round trips per live view per device, for the same rows.
            if !pushed {
                return;
            }
            let now = js_sys::Date::now();
            let Some(delay) = coalescer.borrow_mut().on_push(now, js_sys::Math::random()) else {
                // A refresh is already pending; this push rides along with it.
                return;
            };
            let coalescer = coalescer.clone();
            let runtime = runtime.clone();
            let cb = Closure::once_into_js(move || {
                coalescer.borrow_mut().on_fire(js_sys::Date::now());
                let _guard = RuntimeGuard::new(runtime);
                refresh += 1;
            });
            if let Some(win) = web_sys::window() {
                if let Ok(id) = win.set_timeout_with_callback_and_timeout_and_arguments_0(
                    cb.unchecked_ref(),
                    delay as i32,
                ) {
                    if let Some(old) = timer.borrow_mut().replace(id) {
                        win.clear_timeout_with_handle(old);
                    }
                }
            }
        }
    });

    use_drop(move || {
        if let (Some(win), Some(id)) = (web_sys::window(), timer.borrow_mut().take()) {
            win.clear_timeout_with_handle(id);
        }
    });

    use_focus_refresh(refresh);
}

/// A burst of pushes becomes ONE refresh, and no two devices refresh together.
///
/// Every push used to re-run each query the view keyed on the counter. That is
/// fine for a document three people are editing and ruinous for a ballot: with
/// 500 delegates on one poll, each vote cast pushed to all 500, and each push
/// re-ran three queries — 750,000 requests for a single vote, all inside the
/// couple of minutes people are voting, and arriving in synchronised bursts
/// because the push reaches every device at the same instant.
///
/// So pushes inside a window fold into one refresh, and each refresh is spread
/// over a random slice of the window. The result a voter sees lags by at most
/// [`COALESCE_MS`] + [`SPREAD_MS`], which is not perceptible on a tally that
/// changes continuously — and the server meets a trickle instead of a stampede.
const COALESCE_MS: f64 = 1_500.0;

/// How wide the herd is spread. Jitter matters more than the window: 500 devices
/// refreshing 1.5 s apart but all at the same moment is still a burst of 500.
const SPREAD_MS: f64 = 1_000.0;

#[derive(Default)]
struct Coalescer {
    /// Earliest time a refresh may fire, so a settled view is not re-queried
    /// more often than the window allows.
    next_allowed_ms: f64,
    /// A refresh is already scheduled; further pushes need no timer of their own.
    scheduled: bool,
}

impl Coalescer {
    /// The delay to schedule a refresh at, or `None` if one is already pending.
    fn on_push(&mut self, now_ms: f64, rand: f64) -> Option<f64> {
        if self.scheduled {
            return None;
        }
        self.scheduled = true;
        let wait = (self.next_allowed_ms - now_ms).max(0.0);
        Some(wait + rand.clamp(0.0, 1.0) * SPREAD_MS)
    }

    fn on_fire(&mut self, now_ms: f64) {
        self.scheduled = false;
        self.next_allowed_ms = now_ms + COALESCE_MS;
    }
}

/// A node subscription that carries a CHANGE TOKEN instead of the rows.
///
/// `use_live` ignores the payload — it only needs to know that something under
/// `where_clause` changed. Hasura, meanwhile, pushes the whole result to every
/// subscriber whenever it differs, so selecting rows means a poll with 500 votes
/// shipped 23 KB to every device on every vote cast. Measured against
/// production: 23,011 bytes as rows, 101 bytes as `count` + `max(updatedAt)` —
/// which moves on exactly the same events, since an insert changes the count, an
/// edit changes the timestamp, and a delete in this wiki is an update that does
/// both.
///
/// Only for `nodes`. `relations` and `members` expose no timestamp to aggregate,
/// so a row EDITED in place (the chair moving the room's active node) would
/// leave count and max(id) untouched and never reach the projector.
pub fn nodes_changed(where_clause: &str) -> String {
    format!(
        "subscription {{ nodesAggregate(where: {{ {where_clause} }}) \
         {{ aggregate {{ count max {{ updatedAt }} }} }} }}"
    )
}

/// Refresh only when a streamed row belongs to YOU.
///
/// A change token can only say "something under this filter changed", so every
/// watcher of a shared scope wakes: one comment in a context refetched the post's
/// comment list AND every open thread's replies, on every device. A stream
/// carries the rows, so a watcher can compare `parentId` with its own and ignore
/// the rest.
///
/// The list is still re-fetched rather than merged. That keeps the server as the
/// single authority on order and contents — hand-applied deltas drift, and a
/// comment thread is the wrong place to discover that.
///
/// Every watcher of the same scope sends the SAME query, so the hub folds them
/// into one server-side subscription however many are on screen.
pub fn use_live_children(where_clause: String, mine: String, mut refresh: Signal<u32>) {
    let since = use_hook(|| {
        js_sys::Date::new_0()
            .to_iso_string()
            .as_string()
            .unwrap_or_default()
    });
    let stream =
        use_graphql_subscription(crate::graphql::nodes_stream(&where_clause, &since, "parentId"));
    use_effect(move || {
        let Some(payload) = stream.read().clone() else {
            return;
        };
        let touched = payload
            .get("nodes_stream")
            .and_then(|r| r.as_array())
            .map(|rows| {
                rows.iter()
                    .any(|r| r.get("parentId").and_then(|p| p.as_str()) == Some(mine.as_str()))
            })
            .unwrap_or(false);
        if touched {
            refresh += 1;
        }
    });
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

    let handle = use_hook(|| Hub::subscribe(query.clone(), data, Runtime::current()));

    use_drop(move || {
        // Deregister BEFORE anything else: a frame already in flight for this id
        // must not find a signal whose scope is being torn down.
        Hub::unsubscribe(&handle);
    });

    data
}

/// Which subscriptions exist on the socket, and who is listening to each.
///
/// Two components asking the same question cost ONE subscription. Hasura runs a
/// live query per registration and re-executes it on a timer, so a document with
/// forty comments — each reaction bar watching the same context — meant forty
/// live queries per device, and five hundred devices in a hall meant twenty
/// thousand against one server. Identical queries share, which is exactly what
/// makes a context-wide watch cheaper than a per-row one rather than the same.
///
/// Generic over the sink so the bookkeeping can be tested without a renderer.
struct Registry<S> {
    /// Server-side subscription id -> the query and everyone waiting on it.
    subs: HashMap<String, (String, Vec<(u64, S)>)>,
    /// Query text -> the id already carrying it.
    by_query: HashMap<String, String>,
    next_id: u64,
    next_sink: u64,
}

impl<S> Default for Registry<S> {
    fn default() -> Self {
        Registry {
            subs: HashMap::new(),
            by_query: HashMap::new(),
            next_id: 0,
            next_sink: 0,
        }
    }
}

/// One listener's place in the registry.
#[derive(Clone, Debug, PartialEq)]
struct Handle {
    id: String,
    sink: u64,
}

impl<S> Registry<S> {
    /// Add a listener. Returns its handle, and the query to SEND if this is the
    /// first listener for it — `None` means the socket already carries it.
    fn register(&mut self, query: String, sink: S) -> (Handle, Option<String>) {
        self.next_sink += 1;
        let sink_id = self.next_sink;
        if let Some(id) = self.by_query.get(&query).cloned() {
            if let Some((_, sinks)) = self.subs.get_mut(&id) {
                sinks.push((sink_id, sink));
                return (Handle { id, sink: sink_id }, None);
            }
        }
        self.next_id += 1;
        let id = self.next_id.to_string();
        self.subs
            .insert(id.clone(), (query.clone(), vec![(sink_id, sink)]));
        self.by_query.insert(query.clone(), id.clone());
        (Handle { id, sink: sink_id }, Some(query))
    }

    /// Remove a listener. Returns the id to send `complete` for once the last
    /// listener for that query is gone.
    fn deregister(&mut self, handle: &Handle) -> Option<String> {
        let (query, empty) = {
            let (query, sinks) = self.subs.get_mut(&handle.id)?;
            sinks.retain(|(s, _)| *s != handle.sink);
            (query.clone(), sinks.is_empty())
        };
        if !empty {
            return None;
        }
        self.subs.remove(&handle.id);
        self.by_query.remove(&query);
        Some(handle.id.clone())
    }

    /// Every live (id, query), for re-sending after a reconnect.
    fn live(&self) -> Vec<(String, String)> {
        self.subs
            .iter()
            .map(|(id, (q, _))| (id.clone(), q.clone()))
            .collect()
    }

    fn is_empty(&self) -> bool {
        self.subs.is_empty()
    }
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
    subs: Registry<Signal<Option<serde_json::Value>>>,
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

    /// Register a listener, connecting if this is the first one. A query the
    /// socket already carries costs nothing but a place in its listener list.
    fn subscribe(
        query: String,
        sink: Signal<Option<serde_json::Value>>,
        runtime: Rc<Runtime>,
    ) -> Handle {
        let (handle, to_send, acked) = Self::with(|st| {
            st.runtime.get_or_insert(runtime);
            let (handle, to_send) = st.subs.register(query, sink);
            (handle, to_send, st.acked)
        });
        match (to_send, acked) {
            // New to the socket, and the socket is ready for it.
            (Some(query), true) => Self::send_subscribe(&handle.id, &query),
            // New, but the socket is not up yet: `connection_ack` sends it.
            (Some(_), false) => Self::ensure_connected(),
            // Already carried; the existing subscription feeds this sink too.
            (None, _) => {}
        }
        handle
    }

    /// Drop a listener, ending the subscription once its last one goes, and
    /// closing the socket once no subscriptions are left.
    fn unsubscribe(handle: &Handle) {
        let (ended, idle) = Self::with(|st| {
            let ended = st.subs.deregister(handle);
            let idle = st.subs.is_empty();
            (ended, idle)
        });
        if let Some(id) = ended {
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
                        st.subs.live()
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
                    let target = Self::with(|st| {
                        let sinks: Vec<_> = st
                            .subs
                            .subs
                            .get(id)
                            .map(|(_, sinks)| sinks.iter().map(|(_, s)| *s).collect())
                            .unwrap_or_default();
                        st.runtime.clone().map(|rt| (sinks, rt))
                    });
                    if let Some((sinks, runtime)) = target {
                        let _guard = RuntimeGuard::new(runtime);
                        for mut sink in sinks {
                            sink.set(Some(d.clone()));
                        }
                    }
                }
                // A subscription the server refuses or ends. Rare, and previously
                // invisible: the view would simply stop updating with no trace.
                Some("error") => {
                    let detail = msg
                        .get("payload")
                        .map(|p| p.to_string())
                        .unwrap_or_default();
                    // `error`, not `warn`. A refused subscription is a view that
                    // has silently stopped updating, and these queries are built
                    // as strings with nothing to check them before the server
                    // does. One shipped double-wrapped and every reader of the
                    // feed lost live updates; it surfaced only because somebody
                    // pasted a warning nobody was filtering for.
                    log::error!("subscription {id} refused: {detail}");
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
    use super::{backoff_delay_ms, Coalescer, Registry, COALESCE_MS, SPREAD_MS};

    /// A burst of pushes costs one refresh, not one per push.
    ///
    /// This is the ballot case: 500 votes cast in two minutes, each pushing to
    /// every device on the poll. Without folding, each device ran three queries
    /// per vote.
    #[test]
    fn a_burst_of_pushes_schedules_exactly_one_refresh() {
        let mut c = Coalescer::default();
        let scheduled: Vec<f64> = (0..100)
            .filter_map(|i| c.on_push(1_000.0 + i as f64 * 5.0, 0.5))
            .collect();
        assert_eq!(scheduled.len(), 1, "a burst must fold into one refresh");
        // ...and once it fires, the view is live again for the next burst.
        c.on_fire(2_000.0);
        assert!(c.on_push(4_000.0, 0.5).is_some());
    }

    /// Two devices given different random draws refresh at different moments.
    ///
    /// The window alone does not help when a push reaches 500 devices at the
    /// same instant: they would simply burst 1.5 s later, together.
    #[test]
    fn refreshes_are_spread_across_the_herd() {
        let delay = |rand| {
            Coalescer::default()
                .on_push(0.0, rand)
                .expect("first push schedules")
        };
        assert_eq!(delay(0.0), 0.0);
        assert_eq!(delay(1.0), SPREAD_MS);
        assert!(
            delay(0.25) < delay(0.75),
            "the draw must actually spread devices"
        );
    }

    /// A refresh always arrives, and always within the promised window.
    #[test]
    fn a_refresh_is_never_delayed_beyond_the_window() {
        let mut c = Coalescer::default();
        c.on_fire(0.0); // a refresh just happened: the next one waits out the window
        let d = c
            .on_push(1.0, 1.0)
            .expect("a push after a fire still schedules");
        assert!(d <= COALESCE_MS + SPREAD_MS, "delay {d} exceeds the window");
        assert!(d >= COALESCE_MS - 1.0, "must respect the cooldown, got {d}");
    }

    /// The same question asked twice costs one subscription.
    ///
    /// Forty reaction bars on one document each watched the same context. Every
    /// registration is a live query Hasura re-runs on a timer, so sharing is the
    /// difference between one and forty per device — and between 500 and 20,000
    /// in a hall.
    #[test]
    fn identical_queries_share_one_subscription() {
        let mut r: Registry<u32> = Registry::default();
        let (h1, send1) = r.register("sub A".into(), 1);
        let (h2, send2) = r.register("sub A".into(), 2);
        assert_eq!(
            send1.as_deref(),
            Some("sub A"),
            "the first must go to the server"
        );
        assert_eq!(send2, None, "the second must ride along");
        assert_eq!(h1.id, h2.id, "both listen to the same subscription");
        assert_eq!(r.live().len(), 1);

        // Both sinks are fed by the one subscription.
        let sinks: Vec<u32> = r.subs[&h1.id].1.iter().map(|(_, s)| *s).collect();
        assert_eq!(sinks, vec![1, 2]);

        // A different question is its own subscription.
        let (_, send3) = r.register("sub B".into(), 3);
        assert_eq!(send3.as_deref(), Some("sub B"));
        assert_eq!(r.live().len(), 2);
    }

    /// A shared subscription ends only when its LAST listener goes.
    #[test]
    fn a_shared_subscription_outlives_its_first_listener() {
        let mut r: Registry<u32> = Registry::default();
        let (h1, _) = r.register("sub A".into(), 1);
        let (h2, _) = r.register("sub A".into(), 2);
        assert_eq!(
            r.deregister(&h1),
            None,
            "one listener leaving must not end it"
        );
        assert!(!r.is_empty());
        assert_eq!(r.deregister(&h2).as_deref(), Some(h1.id.as_str()));
        assert!(
            r.is_empty(),
            "the socket may close once nothing is listening"
        );
        // And the query is free to be registered afresh afterwards.
        let (_, send) = r.register("sub A".into(), 3);
        assert_eq!(send.as_deref(), Some("sub A"));
    }

    /// Deregistering twice (or an unknown handle) is harmless.
    #[test]
    fn deregistering_an_unknown_listener_is_a_no_op() {
        let mut r: Registry<u32> = Registry::default();
        let (h, _) = r.register("sub A".into(), 1);
        assert!(r.deregister(&h).is_some());
        assert_eq!(r.deregister(&h), None);
    }

    /// A bad random sample cannot produce a negative delay (setTimeout would
    /// fire immediately, re-creating the stampede it exists to prevent).
    #[test]
    fn a_bad_random_draw_is_clamped() {
        assert_eq!(Coalescer::default().on_push(0.0, -3.0), Some(0.0));
        assert_eq!(Coalescer::default().on_push(0.0, 9.0), Some(SPREAD_MS));
    }

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
