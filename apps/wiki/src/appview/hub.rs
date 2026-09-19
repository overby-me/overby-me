//! The one socket a tab holds to the AppView's `/ws`, and the views riding it.
//!
//! The browser half of live updates: `watch.rs` keeps the books and asks the
//! questions, this owns the `WebSocket`. One connection per tab however many
//! views watch, opened by the first and closed after the last.
//!
//! A session here lasts a month and is sent as the first frame, so there is no
//! token to renew under a live socket. A different session (signing in or out)
//! is a different listener: the socket is closed, and comes back as them.

use super::watch::{payload, topics_of, Handle, Watches};
use super::wire::{Change, Shape, Wire};
use dioxus::core::{Runtime, RuntimeGuard};
use dioxus::prelude::*;
use serde_json::json;
use std::cell::RefCell;
use std::rc::Rc;
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;
use web_sys::{MessageEvent, WebSocket};

type Sink = Signal<Option<serde_json::Value>>;

/// A proxy drops a socket that says nothing for a minute.
const PING_EVERY_MS: i32 = 25_000;

#[derive(Default)]
struct HubState {
    ws: Option<WebSocket>,
    /// The AppView knows who is listening, so topics can be asked for.
    ready: bool,
    /// Who the socket was opened as.
    session: Option<String>,
    attempts: u32,
    reconnect: Option<i32>,
    ping: Option<i32>,
    watches: Watches<Sink>,
    runtime: Option<Rc<Runtime>>,
}

thread_local! {
    static HUB: Rc<RefCell<HubState>> = Rc::new(RefCell::new(HubState::default()));
}

pub(crate) struct Hub;

impl Hub {
    fn with<R>(f: impl FnOnce(&mut HubState) -> R) -> R {
        HUB.with(|hub| f(&mut hub.borrow_mut()))
    }

    /// Run `work` beside the renderer, not as a task of any scope: what it
    /// ends in is a write to a view's signal, which a task of the root scope
    /// is warned off making. The runtime is entered only where it is needed.
    fn spawn(work: impl std::future::Future<Output = ()> + 'static) {
        wasm_bindgen_futures::spawn_local(work);
    }

    fn token() -> Option<String> {
        let runtime = Self::with(|st| st.runtime.clone())?;
        let _guard = RuntimeGuard::new(runtime);
        crate::session::SESSION.peek().access_token.clone()
    }

    pub(crate) fn subscribe(wire: Wire, sink: Sink, runtime: Rc<Runtime>) -> Handle {
        let (handle, new) = Self::with(|st| {
            st.runtime.get_or_insert(runtime);
            st.watches.register(wire.clone(), sink)
        });
        if let Some(watch) = new {
            Self::spawn(async move {
                let topics = topics_of(Self::token().as_deref(), &wire.scope).await;
                let (fresh, ready) =
                    Self::with(|st| (st.watches.resolved(watch, topics), st.ready));
                match ready {
                    true => fresh.iter().for_each(|topic| Self::ask_for(topic)),
                    false => Self::ensure_connected(),
                }
            });
        }
        handle
    }

    pub(crate) fn unsubscribe(handle: &Handle) {
        let (dropped, idle) = Self::with(|st| {
            let dropped = st.watches.deregister(handle);
            (dropped, st.watches.is_empty())
        });
        for topic in dropped {
            Self::send(&json!({ "op": "unsub", "topic": topic }));
        }
        if idle {
            Self::with(|st| {
                Self::clear_timer(st.reconnect.take());
                st.attempts = 0;
                st.ready = false;
                st.ws.take()
            })
            .map(|ws| ws.close());
        }
    }

    /// Whoever is signed in changed: a socket opened as someone else goes.
    pub(crate) fn listen_as(session: Option<&str>) {
        let stale = Self::with(|st| st.ws.is_some() && st.session.as_deref() != session);
        if stale {
            // `on_close` brings it back, as whoever is signed in by then.
            Self::with(|st| st.ws.clone()).map(|ws| ws.close());
        }
    }

    fn clear_timer(handle: Option<i32>) {
        if let (Some(win), Some(id)) = (web_sys::window(), handle) {
            win.clear_timeout_with_handle(id);
        }
    }

    fn send(frame: &serde_json::Value) {
        if let Some(ws) = Self::with(|st| st.ws.clone()) {
            let _ = ws.send_with_str(&frame.to_string());
        }
    }

    fn ask_for(topic: &str) {
        Self::send(&json!({ "op": "sub", "topic": topic }));
    }

    fn ensure_connected() {
        let needed = Self::with(|st| st.ws.is_none() && st.reconnect.is_none());
        if needed {
            Self::connect();
        }
    }

    fn connect() {
        let url = super::appview_url()
            .replacen("https://", "wss://", 1)
            .replacen("http://", "ws://", 1);
        let Ok(ws) = WebSocket::new(&format!("{url}/ws")) else {
            Self::schedule_reconnect();
            return;
        };
        let session = Self::token();
        Self::with(|st| {
            st.ws = Some(ws.clone());
            st.session = session.clone();
        });

        let on_open = Closure::<dyn FnMut()>::new(move || match &session {
            Some(session) => Self::send(&json!({ "op": "auth", "session": session })),
            None => Self::listening(),
        });
        ws.set_onopen(Some(on_open.as_ref().unchecked_ref()));

        let on_message = Closure::<dyn FnMut(MessageEvent)>::new(move |e: MessageEvent| {
            let Some(text) = e.data().as_string() else {
                return;
            };
            let Ok(frame) = serde_json::from_str::<serde_json::Value>(&text) else {
                return;
            };
            match frame.get("op").and_then(|op| op.as_str()) {
                // A session the AppView does not know listens as nobody, which
                // is still listening: what is public stays live.
                Some("auth") => Self::listening(),
                Some("sub") => Self::answered(&frame),
                Some("unsub") if frame["revoked"] == true => {
                    Self::touched(frame["topic"].as_str().unwrap_or_default());
                }
                Some(_) => {}
                None => {
                    if let Ok(change) = serde_json::from_value::<Change>(frame) {
                        Self::changed(&change);
                    }
                }
            }
        });
        ws.set_onmessage(Some(on_message.as_ref().unchecked_ref()));

        // An error ends in a close as well, so this is the one place a lost
        // socket is answered from.
        let on_close = Closure::<dyn FnMut()>::new(move || {
            let wanted = Self::with(|st| {
                Self::clear_timer(st.ping.take());
                st.ready = false;
                st.ws = None;
                !st.watches.is_empty()
            });
            if wanted {
                Self::schedule_reconnect();
            }
        });
        ws.set_onclose(Some(on_close.as_ref().unchecked_ref()));

        // The socket owns these for its life: three small closures a connection.
        on_open.forget();
        on_message.forget();
        on_close.forget();
    }

    /// The AppView knows who this is: ask for every topic in use, and keep the
    /// socket talking.
    fn listening() {
        let topics = Self::with(|st| {
            st.ready = true;
            st.attempts = 0;
            st.watches.topics()
        });
        topics.iter().for_each(|topic| Self::ask_for(topic));
        Self::keep_alive();
    }

    fn keep_alive() {
        let beat = Closure::once_into_js(move || {
            let open = Self::with(|st| {
                st.ping = None;
                st.ready
            });
            if open {
                Self::send(&json!({ "op": "ping" }));
                Self::keep_alive();
            }
        });
        let timer = web_sys::window().and_then(|win| {
            win.set_timeout_with_callback_and_timeout_and_arguments_0(
                beat.unchecked_ref(),
                PING_EVERY_MS,
            )
            .ok()
        });
        Self::with(|st| st.ping = timer);
    }

    /// A topic was granted, or was not. A refusal is a context this reader may
    /// not watch, where every read says the same, so it is not worth a word.
    fn answered(frame: &serde_json::Value) {
        if frame["ok"] != true {
            return;
        }
        let topic = frame["topic"].as_str().unwrap_or_default();
        for grant in Self::with(|st| st.watches.granted(topic)) {
            let Some((wire, _, _)) = Self::with(|st| st.watches.parts(grant.watch)) else {
                continue;
            };
            // A view that only refetches expects the state on arrival, and
            // knows to ignore the first. A stream says nothing until something
            // happens, unless the socket was down: then something may have. A
            // board asks either way, for what was painted while it was loading.
            let rows = match (&wire.shape, grant.again) {
                (Shape::Touch | Shape::Cells { .. }, _) | (Shape::State { .. }, true) => Vec::new(),
                (Shape::Rows, true) => wire.rows_after_a_gap(),
                (Shape::Rows | Shape::State { .. }, false) => continue,
            };
            Self::hand_over(grant.watch, rows);
        }
    }

    fn changed(change: &Change) {
        for (watch, row) in Self::with(|st| st.watches.heard(change)) {
            Self::hand_over(watch, vec![row]);
        }
    }

    /// A topic was taken away: what its views show is no longer theirs to see,
    /// which a refetch finds out.
    fn touched(topic: &str) {
        for watch in Self::with(|st| st.watches.on_topic(topic)) {
            Self::hand_over(watch, Vec::new());
        }
    }

    fn hand_over(watch: u64, rows: Vec<serde_json::Value>) {
        Self::spawn(async move {
            let Some((wire, mut cursor, _)) = Self::with(|st| st.watches.parts(watch)) else {
                return;
            };
            let token = Self::token();
            let Some(pushed) = payload(token.as_deref(), &wire, rows, &mut cursor).await else {
                return;
            };
            // Asked for again: the view may have gone while the AppView answered.
            let (sinks, runtime) = Self::with(|st| {
                st.watches.moved_on(watch, cursor);
                let sinks = st.watches.parts(watch).map(|(_, _, sinks)| sinks);
                (sinks, st.runtime.clone())
            });
            let Some(runtime) = runtime else { return };
            let _guard = RuntimeGuard::new(runtime);
            for mut sink in sinks.into_iter().flatten() {
                sink.set(Some(pushed.clone()));
            }
        });
    }

    fn schedule_reconnect() {
        let delay = Self::with(|st| {
            let delay = crate::subscription::backoff_delay_ms(st.attempts, js_sys::Math::random());
            st.attempts = st.attempts.saturating_add(1);
            delay
        });
        let retry = Closure::once_into_js(move || {
            let wanted = Self::with(|st| {
                st.reconnect = None;
                !st.watches.is_empty()
            });
            if wanted {
                Self::connect();
            }
        });
        let timer = web_sys::window().and_then(|win| {
            win.set_timeout_with_callback_and_timeout_and_arguments_0(retry.unchecked_ref(), delay)
                .ok()
        });
        Self::with(|st| st.reconnect = timer);
    }
}
