//! Browser side of the WebXR Wayland compositor.
//!
//! Holds the WebSocket session to the host, mirrors its window set into
//! floating divs, paints frame damage onto each window's canvas, and sends
//! pointer and keyboard input back through the wire protocol.

mod keymap;

use std::collections::BTreeMap;

use dioxus::logger::tracing;
use dioxus::prelude::*;
use futures_util::future::{Either, select};
use futures_util::stream::SplitSink;
use futures_util::{SinkExt, StreamExt};
use gloo_net::websocket::Message;
use gloo_net::websocket::futures::WebSocket;
use wasm_bindgen::closure::Closure;
use wasm_bindgen::{Clamped, JsCast};
use web_sys::{CanvasRenderingContext2d, HtmlCanvasElement, ImageData};
use webxr_compositor_protocol as protocol;

const MAIN_CSS: Asset = asset!("/assets/main.css");

const BTN_LEFT: u32 = 0x110;
const BTN_RIGHT: u32 = 0x111;
const BTN_MIDDLE: u32 = 0x112;
const BTN_SIDE: u32 = 0x113;
const BTN_EXTRA: u32 = 0x114;

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
    let focused: Signal<Option<protocol::WindowId>> = use_signal(|| None);

    let session = use_coroutine(move |rx| session_loop(rx, link, windows));
    use_hook(move || install_key_listeners(session, focused));

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
        main {
            id: "desk",
            onmousedown: move |_| defocus(session, focused),
            for (id, info) in windows() {
                WindowView { key: "{id}", id, info, focused }
            }
        }
    }
}

fn defocus(
    session: Coroutine<protocol::ClientToHost>,
    mut focused: Signal<Option<protocol::WindowId>>,
) {
    if focused.peek().is_some() {
        focused.set(None);
        session.send(protocol::ClientToHost::Focus { id: None });
    }
}

#[component]
fn WindowView(
    id: protocol::WindowId,
    info: WindowInfo,
    focused: Signal<Option<protocol::WindowId>>,
) -> Element {
    let session = use_coroutine_handle::<protocol::ClientToHost>();
    let mut focused = focused;
    let label = if info.title.is_empty() {
        info.app_id.clone()
    } else {
        info.title.clone()
    };
    let class = if focused() == Some(id) {
        "window focused"
    } else {
        "window"
    };
    rsx! {
        div {
            class: "{class}",
            style: "left: {info.x}px; top: {info.y}px;",
            onmousedown: move |e| {
                e.stop_propagation();
                if *focused.peek() != Some(id) {
                    focused.set(Some(id));
                    session.send(protocol::ClientToHost::Focus { id: Some(id) });
                }
            },
            div { class: "titlebar", span { class: "title", "{label}" } }
            canvas {
                class: "surface",
                id: "win-{id}",
                onmousemove: move |e| {
                    let p = e.data().element_coordinates();
                    session.send(protocol::ClientToHost::PointerMotion { id, x: p.x, y: p.y });
                },
                onmousedown: move |e| {
                    if let Some(button) = wire_button(&e) {
                        session.send(protocol::ClientToHost::PointerButton {
                            id,
                            button,
                            pressed: true,
                        });
                    }
                },
                onmouseup: move |e| {
                    if let Some(button) = wire_button(&e) {
                        session.send(protocol::ClientToHost::PointerButton {
                            id,
                            button,
                            pressed: false,
                        });
                    }
                },
                onwheel: move |e| {
                    e.prevent_default();
                    let (dx, dy) = wheel_delta(&e);
                    session.send(protocol::ClientToHost::PointerAxis { id, dx, dy });
                },
                oncontextmenu: move |e| e.prevent_default(),
            }
        }
    }
}

fn wire_button(e: &Event<MouseData>) -> Option<u32> {
    Some(match e.data().trigger_button()? {
        dioxus::html::input_data::MouseButton::Primary => BTN_LEFT,
        dioxus::html::input_data::MouseButton::Secondary => BTN_RIGHT,
        dioxus::html::input_data::MouseButton::Auxiliary => BTN_MIDDLE,
        dioxus::html::input_data::MouseButton::Fourth => BTN_SIDE,
        dioxus::html::input_data::MouseButton::Fifth => BTN_EXTRA,
        dioxus::html::input_data::MouseButton::Unknown => return None,
    })
}

fn wheel_delta(e: &Event<WheelData>) -> (f64, f64) {
    match e.data().delta() {
        dioxus::html::geometry::WheelDelta::Pixels(v) => (v.x, v.y),
        dioxus::html::geometry::WheelDelta::Lines(v) => (v.x * 20.0, v.y * 20.0),
        dioxus::html::geometry::WheelDelta::Pages(v) => (v.x * 400.0, v.y * 400.0),
    }
}

/// Window-level listeners so keys arrive regardless of DOM focus; browser
/// defaults are suppressed only while a compositor window holds focus.
fn install_key_listeners(
    session: Coroutine<protocol::ClientToHost>,
    focused: Signal<Option<protocol::WindowId>>,
) {
    let Some(window) = web_sys::window() else {
        return;
    };
    for pressed in [true, false] {
        let handler =
            Closure::<dyn FnMut(web_sys::KeyboardEvent)>::new(move |e: web_sys::KeyboardEvent| {
                if focused.peek().is_none() || (pressed && e.repeat()) {
                    return;
                }
                if let Some(code) = keymap::evdev(&e.code()) {
                    e.prevent_default();
                    session.send(protocol::ClientToHost::Key { code, pressed });
                }
            });
        let kind = if pressed { "keydown" } else { "keyup" };
        if window
            .add_event_listener_with_callback(kind, handler.as_ref().unchecked_ref())
            .is_err()
        {
            tracing::warn!("could not install the {kind} listener");
        }
        handler.forget();
    }
}

/// Reconnect forever: every attempt greets the host, then pumps messages
/// both ways until the socket drops. A protocol mismatch parks the session
/// instead, since retrying cannot fix a stale page.
async fn session_loop(
    mut rx: UnboundedReceiver<protocol::ClientToHost>,
    mut link: Signal<Link>,
    mut windows: Signal<Windows>,
) {
    loop {
        link.set(Link::Connecting);
        // Input queued while disconnected aims at a dead session; drop it.
        while rx.try_recv().is_ok() {}
        if let Some(ws) = open_socket() {
            run_session(ws, &mut rx, &mut link, &mut windows).await;
        }
        windows.write().clear();
        if matches!(link(), Link::Mismatch { .. }) {
            return;
        }
        link.set(Link::Lost);
        gloo_timers::future::TimeoutFuture::new(1_000).await;
    }
}

async fn run_session(
    ws: WebSocket,
    rx: &mut UnboundedReceiver<protocol::ClientToHost>,
    link: &mut Signal<Link>,
    windows: &mut Signal<Windows>,
) {
    let (mut sink, mut stream) = ws.split();
    if send(
        &mut sink,
        &protocol::ClientToHost::Hello {
            version: protocol::VERSION,
        },
    )
    .await
    .is_none()
    {
        return;
    }

    loop {
        match select(stream.next(), rx.next()).await {
            Either::Left((Some(Ok(Message::Bytes(bytes))), _)) => {
                match protocol::HostToClient::decode(&bytes) {
                    Ok(msg) => {
                        if apply(msg, link, windows).await.is_none() {
                            return;
                        }
                    }
                    Err(error) => tracing::warn!("undecodable host message: {error}"),
                }
            }
            Either::Left((Some(Ok(Message::Text(_))), _)) => {}
            Either::Left((Some(Err(_)) | None, _)) => return,
            Either::Right((Some(outgoing), _)) => {
                if send(&mut sink, &outgoing).await.is_none() {
                    return;
                }
            }
            Either::Right((None, _)) => return,
        }
    }
}

async fn send(
    sink: &mut SplitSink<WebSocket, Message>,
    msg: &protocol::ClientToHost,
) -> Option<()> {
    let bytes = msg.encode().ok()?;
    sink.send(Message::Bytes(bytes)).await.ok()
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
