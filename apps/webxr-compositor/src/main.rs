//! Browser side of the WebXR Wayland compositor.
//!
//! Holds the WebSocket session to the host and shows its state; window
//! canvases and input capture come next.

use dioxus::logger::tracing;
use dioxus::prelude::*;
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::Message;
use gloo_net::websocket::futures::WebSocket;
use webxr_compositor_protocol as protocol;

const MAIN_CSS: Asset = asset!("/assets/main.css");

#[derive(Clone, PartialEq)]
enum Link {
    Connecting,
    Connected {
        host: String,
        output: protocol::Size,
    },
    Mismatch {
        theirs: u32,
    },
    Lost,
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let link = use_signal(|| Link::Connecting);

    use_future(move || session_loop(link));

    let status = match link() {
        Link::Connecting => "connecting to the host...".to_string(),
        Link::Connected { host, output } => format!(
            "connected: {host}, protocol v{}, output {}x{}",
            protocol::VERSION,
            output.width,
            output.height
        ),
        Link::Mismatch { theirs } => format!(
            "protocol mismatch: page speaks v{}, host speaks v{theirs}; reload the page",
            protocol::VERSION
        ),
        Link::Lost => "host link lost, retrying...".to_string(),
    };

    rsx! {
        document::Stylesheet { href: MAIN_CSS }
        main {
            h1 { "webxr-compositor" }
            p { class: "tagline", "Wayland apps in the browser: flat now, XR later." }
            p { class: "status", id: "link-status", "{status}" }
        }
    }
}

/// Reconnect forever: every attempt greets the host, then consumes messages
/// until the socket drops. A protocol mismatch parks the session instead,
/// since retrying cannot fix a stale page.
async fn session_loop(mut link: Signal<Link>) {
    loop {
        link.set(Link::Connecting);
        if let Some(mut ws) = open_socket()
            && greet(&mut ws).await.is_some()
        {
            consume(&mut ws, &mut link).await;
        }
        if matches!(link(), Link::Mismatch { .. }) {
            return;
        }
        link.set(Link::Lost);
        gloo_timers::future::TimeoutFuture::new(1_000).await;
    }
}

fn open_socket() -> Option<WebSocket> {
    let location = web_sys::window()?.location();
    let scheme = if location.protocol().ok()? == "https:" {
        "wss"
    } else {
        "ws"
    };
    let host = location.host().ok()?;
    WebSocket::open(&format!("{scheme}://{host}/ws")).ok()
}

async fn greet(ws: &mut WebSocket) -> Option<()> {
    let hello = protocol::ClientToHost::Hello {
        version: protocol::VERSION,
    }
    .encode()
    .ok()?;
    ws.send(Message::Bytes(hello)).await.ok()
}

async fn consume(ws: &mut WebSocket, link: &mut Signal<Link>) {
    while let Some(Ok(message)) = ws.next().await {
        let Message::Bytes(bytes) = message else {
            continue;
        };
        match protocol::HostToClient::decode(&bytes) {
            Ok(protocol::HostToClient::Hello {
                version,
                host,
                output,
            }) => {
                if version == protocol::VERSION {
                    link.set(Link::Connected { host, output });
                } else {
                    link.set(Link::Mismatch { theirs: version });
                    return;
                }
            }
            Ok(_) => {}
            Err(error) => {
                tracing::warn!("undecodable host message: {error}");
            }
        }
    }
}
