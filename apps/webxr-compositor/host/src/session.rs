//! WebSocket sessions between the host and browser frontends.

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;
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
pub struct Hub {
    clients: Mutex<BTreeMap<ClientId, mpsc::UnboundedSender<Bytes>>>,
    next_id: AtomicU64,
    events: calloop::channel::Sender<HubEvent>,
}

impl Hub {
    pub fn new(events: calloop::channel::Sender<HubEvent>) -> Self {
        Self {
            clients: Mutex::new(BTreeMap::new()),
            next_id: AtomicU64::new(1),
            events,
        }
    }

    /// Encode once, clone the refcounted bytes per client.
    pub fn broadcast(&self, msg: &protocol::HostToClient) {
        let Ok(bytes) = msg.encode() else {
            tracing::error!("dropped a message postcard could not encode");
            return;
        };
        let bytes = Bytes::from(bytes);
        let clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        for tx in clients.values() {
            // A closed queue only means that client is gone; session() cleans up.
            let _ = tx.send(bytes.clone());
        }
    }

    pub fn send_to(&self, id: ClientId, msg: &protocol::HostToClient) {
        let Ok(bytes) = msg.encode() else {
            tracing::error!("dropped a message postcard could not encode");
            return;
        };
        let clients = self.clients.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(tx) = clients.get(&id) {
            let _ = tx.send(Bytes::from(bytes));
        }
    }

    fn register(&self, tx: mpsc::UnboundedSender<Bytes>) -> ClientId {
        let id = self.next_id.fetch_add(1, Ordering::Relaxed);
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, tx);
        id
    }

    fn unregister(&self, id: ClientId) {
        self.clients
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
    }
}

pub async fn ws_handler(State(hub): State<Arc<Hub>>, ws: WebSocketUpgrade) -> Response {
    ws.on_upgrade(move |socket| session(hub, socket))
}

/// One browser tab: greet it, then pump hub broadcasts out and decoded
/// input events in, until either side hangs up.
async fn session(hub: Arc<Hub>, socket: WebSocket) {
    let (mut to_browser, mut from_browser) = socket.split();
    let (tx, mut rx) = mpsc::unbounded_channel();
    let id = hub.register(tx);
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
                    if to_browser.send(Message::Binary(bytes)).await.is_err() {
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
