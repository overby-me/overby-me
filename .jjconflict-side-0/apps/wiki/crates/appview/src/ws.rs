//! The single multiplexed client WebSocket. Per the realtime decision
//! (`docs/atproto-stack-decisions.md`), the AppView pushes authoritative deltas
//! to clients over ONE `/ws` fed by an in-process `tokio::broadcast` channel.
//! This is the skeleton: it subscribes each socket to the broadcast and relays
//! deltas. The nested declarative subscription trees and the restrictor are a
//! later kickoff item; the transport and fan-out live here.

use axum::extract::State;
use axum::extract::ws::{Message, WebSocket, WebSocketUpgrade};
use axum::response::Response;

use crate::AppState;

/// Upgrade to a WebSocket and hand it to the per-socket relay loop.
pub async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> Response {
    ws.on_upgrade(move |socket| relay(socket, state))
}

/// Forward every broadcast delta to this client until it disconnects. Ping
/// frames are answered so the connection stays alive; client-sent frames are
/// ignored for now (the client protocol is a later item).
async fn relay(mut socket: WebSocket, state: AppState) {
    let mut rx = state.deltas.subscribe();
    loop {
        // Each frame we choose to send; None means close the socket.
        let outgoing = tokio::select! {
            // Authoritative delta to push down. A recv error means the client
            // lagged the broadcast buffer: drop it, and the reconnect-with-backoff
            // client (src/subscription.rs) reconnects and refetches.
            delta = rx.recv() => match delta {
                Ok(text) => Some(Message::Text(text.into())),
                Err(_) => None,
            },
            // Client frame: answer a ping, close on close/error/end-of-stream,
            // ignore anything else for now (the client protocol is a later item).
            incoming = socket.recv() => match incoming {
                Some(Ok(Message::Ping(p))) => Some(Message::Pong(p)),
                Some(Ok(Message::Close(_))) | None | Some(Err(_)) => None,
                Some(Ok(_)) => continue,
            },
        };
        match outgoing {
            Some(msg) => {
                if socket.send(msg).await.is_err() {
                    break;
                }
            }
            None => break,
        }
    }
}
