//! WebSocket sessions between the host and browser frontends.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};


use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::extract::{Query, State};
use axum::response::{IntoResponse, Response};
use bytes::Bytes;
use futures_util::{SinkExt, StreamExt};
use smithay::reexports::calloop;
use tokio::sync::mpsc;
use webxr_compositor_protocol as protocol;

/// Advertised until a real Wayland output exists.
const OUTPUT: protocol::Size = protocol::Size {
    width: 1920,
    height: 1080,
};

pub type ClientId = u64;

/// What the compositor loop hears about browsers.
pub enum HubEvent {
    /// The client completed the hello exchange and wants the current state.
    Joined(ClientId),
    Left(ClientId),
    Input(ClientId, protocol::ClientToHost),
}

/// Fan-out point between one compositor and any number of browser sessions:
/// encoded frames go out to every client, input events come back tagged with
/// the client that sent them.
struct ClientQueue {
    tx: mpsc::UnboundedSender<Bytes>,
    /// Bytes queued but not yet written to the socket, for backpressure.
    inflight: Arc<AtomicU64>,
}

pub struct Hub {
    clients: Mutex<BTreeMap<ClientId, ClientQueue>>,
    next_id: AtomicU64,
    /// Total payload bytes ever broadcast, for throughput logging.
    bytes_sent: AtomicU64,
    /// When set, /ws requires this bearer in its query string.
    token: Mutex<Option<String>>,
    events: calloop::channel::Sender<HubEvent>,
}

impl Hub {
    pub fn new(events: calloop::channel::Sender<HubEvent>) -> Self {
        Self {
            clients: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            bytes_sent: AtomicU64::new(0),
            token: Mutex::new(None),
            events,
        }
    }

    pub fn set_token(&self, token: String) {
        *self.token.lock().unwrap_or_else(PoisonError::into_inner) = Some(token);
    }

    pub fn token(&self) -> Option<String> {
        self.token
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Constant-time enough: every byte is folded regardless of mismatches.
    fn token_matches(&self, presented: Option<&str>) -> bool {
        let Some(expected) = self.token() else {
            return true;
        };
        let Some(presented) = presented else {
            return false;
        };
        let expected = expected.as_bytes();
        let presented = presented.as_bytes();
        let mut diff = u8::from(expected.len() != presented.len());
        for (index, byte) in expected.iter().enumerate() {
            diff |= byte ^ presented.get(index).copied().unwrap_or(0);
        }
        diff == 0
    }

    /// Encode once, clone the refcounted bytes per client.
    pub fn broadcast(&self, msg: &protocol::HostToClient) {
        let Ok(bytes) = msg.encode() else {
            tracing::error!("dropped a message postcard could not encode");
            return;
        };
        let bytes = Bytes::from(bytes);
        let clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        for queue in clients.values() {
            queue
                .inflight
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            // A closed queue only means that client is gone; session() cleans up.
            let _ = queue.tx.send(bytes.clone());
        }
        self.bytes_sent
            .fetch_add(bytes.len() as u64, Ordering::Relaxed);
    }

    pub fn send_to(&self, id: ClientId, msg: &protocol::HostToClient) {
        let Ok(bytes) = msg.encode() else {
            tracing::error!("dropped a message postcard could not encode");
            return;
        };
        let clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(queue) = clients.get(&id) {
            queue
                .inflight
                .fetch_add(bytes.len() as u64, Ordering::Relaxed);
            let _ = queue.tx.send(Bytes::from(bytes));
        }
    }

    /// The largest unsent backlog across clients; the compositor holds frame
    /// callbacks while this is high so clients stop rendering into the void.
    pub fn max_inflight(&self) -> u64 {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .values()
            .map(|q| q.inflight.load(Ordering::Relaxed))
            .max()
            .unwrap_or(0)
    }

    pub fn bytes_sent(&self) -> u64 {
        self.bytes_sent.load(Ordering::Relaxed)
    }

    fn register(&self, tx: mpsc::UnboundedSender<Bytes>) -> (ClientId, Arc<AtomicU64>) {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        let inflight = Arc::new(AtomicU64::new(0));
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                id,
                ClientQueue {
                    tx,
                    inflight: Arc::clone(&inflight),
                },
            );
        (id, inflight)
    }

    fn unregister(&self, id: ClientId) {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }
}

pub async fn ws_handler(
    State(hub): State<Arc<Hub>>,
    Query(query): Query<std::collections::HashMap<String, String>>,
    ws: WebSocketUpgrade,
) -> Response {
    if !hub.token_matches(query.get("token").map(String::as_str)) {
        tracing::warn!("rejected a WebSocket connect with a bad token");
        return axum::http::StatusCode::FORBIDDEN.into_response();
    }
    ws.on_upgrade(move |socket| session(hub, socket))
}

/// One browser tab: greet it, then pump hub broadcasts out and decoded
/// input events in, until either side hangs up.
async fn session(hub: Arc<Hub>, socket: WebSocket) {
    let (mut to_browser, mut from_browser) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let (id, inflight) = hub.register(tx);
    tracing::info!(client = id, "browser connected");

    let hello = protocol::HostToClient::Hello {
        version: protocol::VERSION,
        host: format!("webxr-compositor-host {}", env!("CARGO_PKG_VERSION")),
        output: OUTPUT,
    };
    let greeted = match hello.encode() {
        Ok(bytes) => to_browser
            .send(Message::Binary(Bytes::from(bytes)))
            .await
            .is_ok(),
        Err(_) => false,
    };

    if greeted {
        // The compositor answers with the current window set and frames.
        let _ = hub.events.send(HubEvent::Joined(id));
        loop {
            tokio::select! {
                queued = rx.recv() => {
                    let Some(bytes) = queued else { break };
                    let len = bytes.len() as u64;
                    let sent = to_browser.send(Message::Binary(bytes)).await;
                    inflight.fetch_sub(len, Ordering::Relaxed);
                    if sent.is_err() {
                        break;
                    }
                }
                incoming = from_browser.next() => {
                    match incoming {
                        Some(Ok(Message::Binary(bytes))) => {
                            match protocol::ClientToHost::decode(&bytes) {
                                Ok(event) => {
                                    let _ = hub.events.send(HubEvent::Input(id, event));
                                }
                                Err(error) => tracing::warn!(
                                    client = id, %error, "undecodable message"
                                ),
                            }
                        }
                        Some(Ok(Message::Close(_))) | None => break,
                        Some(Ok(_)) => {}
                        Some(Err(error)) => {
                            tracing::debug!(client = id, %error, "socket error");
                            break;
                        }
                    }
                }
            }
        }
        let _ = hub.events.send(HubEvent::Left(id));
    }

    hub.unregister(id);
    tracing::info!(client = id, "browser disconnected");
}
