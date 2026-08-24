//! Browser side of the WebXR Wayland compositor.
//!
//! Holds the WebSocket session to the host, mirrors its window set into
//! floating divs, and paints frame damage onto each window's canvas.
//! Input capture comes next.

use std::collections::BTreeMap;

use dioxus::logger::tracing;
use dioxus::prelude::*;
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::Message;
use gloo_net::websocket::futures::WebSocket;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
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

#[derive(Clone, PartialEq)]
struct WindowInfo {
    title: String,
    app_id: String,
    x: i32,
    y: i32,
}

type Windows = BTreeMap<protocol::WindowId, WindowInfo>;

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let link = use_signal(|| Link::Connecting);
    let windows = use_signal(Windows::new);

    use_future(move || session_loop(link, windows));

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
        header {
            h1 { "webxr-compositor" }
            p { class: "status", id: "link-status", "{status}" }
        }
        main { id: "desk",
            for (id, info) in windows() {
                WindowView { key: "{id}", id, info }
            }
        }
    }
}

#[component]
fn WindowView(id: protocol::WindowId, info: WindowInfo) -> Element {
    let label = if info.title.is_empty() {
        info.app_id.clone()
    } else {
        info.title.clone()
    };
    rsx! {
        div {
            class: "window",
            style: "left: {info.x}px; top: {info.y}px;",
            div { class: "titlebar", span { class: "title", "{label}" } }
            canvas { class: "surface", id: "win-{id}" }
        }
    }
}

/// Reconnect forever: every attempt greets the host, then consumes messages
/// until the socket drops. A protocol mismatch parks the session instead,
/// since retrying cannot fix a stale page.
async fn session_loop(mut link: Signal<Link>, mut windows: Signal<Windows>) {
    loop {
        link.set(Link::Connecting);
        if let Some(mut ws) = open_socket()
            && greet(&mut ws).await.is_some()
        {
            consume(&mut ws, &mut link, &mut windows).await;
        }
        windows.write().clear();
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

async fn consume(ws: &mut WebSocket, link: &mut Signal<Link>, windows: &mut Signal<Windows>) {
    while let Some(Ok(message)) = ws.next().await {
        let Message::Bytes(bytes) = message else {
            continue;
        };
        match protocol::HostToClient::decode(&bytes) {
            Ok(msg) => {
                if apply(msg, link, windows).await.is_none() {
                    return;
                }
            }
            Err(error) => {
                tracing::warn!("undecodable host message: {error}");
            }
        }
    }
}

/// Returns None only for a protocol mismatch, which ends the session loop.
async fn apply(
    msg: protocol::HostToClient,
    link: &mut Signal<Link>,
    windows: &mut Signal<Windows>,
) -> Option<()> {
    match msg {
        protocol::HostToClient::Hello {
            version,
            host,
            output,
        } => {
            if version != protocol::VERSION {
                link.set(Link::Mismatch { theirs: version });
                return None;
            }
            link.set(Link::Connected { host, output });
        }
        protocol::HostToClient::WindowCreated { id, app_id, title } => {
            let count = i32::try_from(windows.read().len()).unwrap_or(0);
            windows.write().insert(
                id,
                WindowInfo {
                    title,
                    app_id,
                    x: 24 + (count % 8) * 48,
                    y: 24 + (count % 8) * 40,
                },
            );
        }
        protocol::HostToClient::WindowTitle { id, title } => {
            if let Some(info) = windows.write().get_mut(&id) {
                info.title = title;
            }
        }
        protocol::HostToClient::WindowClosed { id } => {
            windows.write().remove(&id);
        }
        protocol::HostToClient::Frame {
            id,
            size,
            damage,
            pixels,
        } => {
            draw_frame(id, size, damage, &pixels).await;
        }
    }
    Some(())
}

/// Paint damage onto the window's canvas. The canvas mounts a beat after
/// WindowCreated flips the signal, so the first frame may need to wait for
/// the DOM to catch up.
async fn draw_frame(
    id: protocol::WindowId,
    size: protocol::Size,
    damage: protocol::Rect,
    pixels: &[u8],
) {
    for _ in 0..120 {
        if let Some(canvas) = canvas_for(id) {
            if paint(&canvas, size, damage, pixels).is_none() {
                tracing::warn!("could not paint frame for window {id}");
            }
            return;
        }
        gloo_timers::future::TimeoutFuture::new(16).await;
    }
    tracing::warn!("no canvas appeared for window {id}");
}

fn canvas_for(id: protocol::WindowId) -> Option<HtmlCanvasElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(&format!("win-{id}"))?
        .dyn_into()
        .ok()
}

fn paint(
    canvas: &HtmlCanvasElement,
    size: protocol::Size,
    damage: protocol::Rect,
    pixels: &[u8],
) -> Option<()> {
    if canvas.width() != size.width || canvas.height() != size.height {
        canvas.set_width(size.width);
        canvas.set_height(size.height);
    }
    let context: CanvasRenderingContext2d = canvas.get_context("2d").ok()??.dyn_into().ok()?;
    let image =
        ImageData::new_with_u8_clamped_array_and_sh(Clamped(pixels), damage.width, damage.height)
            .ok()?;
    context
        .put_image_data(&image, f64::from(damage.x), f64::from(damage.y))
        .ok()
}
