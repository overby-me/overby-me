//! Browser side of the WebXR Wayland compositor.
//!
//! Holds the WebSocket session to the host, mirrors its window set into
//! floating divs, paints frame damage onto each window's canvas, and sends
//! pointer and keyboard input back through the wire protocol.

mod keymap;
mod xr;

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
    z: i32,
    width: u32,
    height: u32,
}

type Windows = BTreeMap<protocol::WindowId, WindowInfo>;

/// A menu, popover or tooltip overlay, anchored in its parent's surface.
#[derive(Clone, Copy, PartialEq)]
struct PopupInfo {
    parent: protocol::WindowId,
    x: i32,
    y: i32,
}

type Popups = BTreeMap<protocol::WindowId, PopupInfo>;

/// A drag in progress, anchored to where the pointer grabbed.
#[derive(Clone, Copy, PartialEq)]
enum DragOp {
    Move {
        id: protocol::WindowId,
        start_x: i32,
        start_y: i32,
        from_x: f64,
        from_y: f64,
    },
    Resize {
        id: protocol::WindowId,
        start_w: u32,
        start_h: u32,
        from_x: f64,
        from_y: f64,
    },
}

fn main() {
    dioxus::launch(App);
}

#[component]
fn App() -> Element {
    let link = use_signal(|| Link::Connecting);
    let mut windows = use_signal(Windows::new);
    let popups = use_signal(Popups::new);
    let focused: Signal<Option<protocol::WindowId>> = use_signal(|| None);
    let mut drag: Signal<Option<DragOp>> = use_signal(|| None);
    let cursor: Signal<String> = use_signal(|| "default".to_owned());

    let mut view3d = use_signal(|| false);

    let session = use_coroutine(move |rx| session_loop(rx, link, windows, popups, cursor));
    use_hook(move || install_key_listeners(session, focused));

    // The 3D render loop lives here so it survives XrView mounting and
    // unmounting; it idles cheaply while the flat view is active.
    use_future(move || async move {
        loop {
            gloo_timers::future::TimeoutFuture::new(33).await;
            if view3d() {
                render_3d(&windows, &popups);
            }
        }
    });

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
            button {
                class: "mode",
                id: "toggle-3d",
                onclick: move |_| {
                    let now = !view3d();
                    view3d.set(now);
                    if !now {
                        xr::drop_scene();
                    }
                },
                if view3d() { "flat view" } else { "3D view" }
            }
            if xr::xr_available() {
                button {
                    class: "mode",
                    id: "enter-vr",
                    onclick: move |_| {
                        view3d.set(true);
                        xr::enter_xr(session);
                    },
                    "enter VR"
                }
            }
        }
        if view3d() {
            XrView { windows, popups, focused }
        }
        main {
            id: "desk",
            class: if view3d() { "backstage" } else { "" },
            onmousedown: move |_| defocus(session, focused),
            onmousemove: move |e| {
                let Some(op) = drag() else { return };
                let p = e.client_coordinates();
                match op {
                    DragOp::Move { id, start_x, start_y, from_x, from_y } => {
                        if let Some(info) = windows.write().get_mut(&id) {
                            info.x = start_x + (p.x - from_x) as i32;
                            info.y = start_y + (p.y - from_y) as i32;
                        }
                    }
                    DragOp::Resize { id, start_w, start_h, from_x, from_y } => {
                        let width = (f64::from(start_w) + (p.x - from_x)).max(48.0) as u32;
                        let height = (f64::from(start_h) + (p.y - from_y)).max(48.0) as u32;
                        session.send(protocol::ClientToHost::Resize {
                            id,
                            size: protocol::Size { width, height },
                        });
                    }
                }
            },
            onmouseup: move |_| drag.set(None),
            for (id, info) in windows() {
                WindowView { key: "{id}", id, info, focused, windows, popups, drag, cursor }
            }
        }
    }
}

/// Feed the scene the current window and popup sets and draw one frame.
fn render_3d(windows: &Signal<Windows>, popups: &Signal<Popups>) {
    let Some(canvas) = canvas_by_id("xr-canvas") else {
        return;
    };
    if !xr::init(&canvas) {
        return;
    }
    let width = canvas.client_width().unsigned_abs().max(1);
    let height = canvas.client_height().unsigned_abs().max(1);
    if canvas.width() != width || canvas.height() != height {
        canvas.set_width(width);
        canvas.set_height(height);
    }

    let window_quads: Vec<(protocol::WindowId, u32, u32)> = windows
        .peek()
        .iter()
        .filter(|(_, info)| info.width > 0)
        .map(|(id, info)| (*id, info.width, info.height))
        .collect();
    let popup_quads: Vec<(protocol::WindowId, protocol::WindowId, i32, i32, u32, u32)> = popups
        .peek()
        .iter()
        .filter_map(|(id, info)| {
            let canvas = canvas_by_id(&format!("win-{id}"))?;
            if canvas.width() == 0 {
                return None;
            }
            Some((
                *id,
                info.parent,
                info.x,
                info.y,
                canvas.width(),
                canvas.height(),
            ))
        })
        .collect();
    xr::render_frame(width, height, &window_quads, &popup_quads);
}

fn canvas_by_id(id: &str) -> Option<HtmlCanvasElement> {
    web_sys::window()?
        .document()?
        .get_element_by_id(id)?
        .dyn_into()
        .ok()
}

#[component]
fn XrView(
    windows: Signal<Windows>,
    popups: Signal<Popups>,
    focused: Signal<Option<protocol::WindowId>>,
) -> Element {
    let session = use_coroutine_handle::<protocol::ClientToHost>();
    let mut focused = focused;
    let mut grabbed: Signal<Option<protocol::WindowId>> = use_signal(|| None);
    let mut orbiting: Signal<Option<(f64, f64)>> = use_signal(|| None);

    let pick_from = move |x: f64, y: f64| -> Option<(protocol::WindowId, f64, f64)> {
        let canvas = canvas_by_id("xr-canvas")?;
        let w = f64::from(canvas.client_width().unsigned_abs().max(1));
        let h = f64::from(canvas.client_height().unsigned_abs().max(1));
        let ndc_x = (x / w * 2.0 - 1.0) as f32;
        let ndc_y = (1.0 - y / h * 2.0) as f32;
        xr::pick_at(ndc_x, ndc_y, (w / h) as f32)
    };

    rsx! {
        canvas {
            id: "xr-canvas",
            onmousedown: move |e| {
                let p = e.data().element_coordinates();
                if let Some((id, x, y)) = pick_from(p.x, p.y) {
                    grabbed.set(Some(id));
                    if *focused.peek() != Some(id) {
                        focused.set(Some(id));
                        session.send(protocol::ClientToHost::Focus { id: Some(id) });
                    }
                    session.send(protocol::ClientToHost::PointerMotion { id, x, y });
                    if let Some(button) = wire_button(&e) {
                        session.send(protocol::ClientToHost::PointerButton {
                            id,
                            button,
                            pressed: true,
                        });
                    }
                } else {
                    let p = e.client_coordinates();
                    orbiting.set(Some((p.x, p.y)));
                }
            },
            onmousemove: move |e| {
                if let Some((from_x, from_y)) = orbiting() {
                    let p = e.client_coordinates();
                    xr::orbit((p.x - from_x) as f32, (p.y - from_y) as f32);
                    orbiting.set(Some((p.x, p.y)));
                } else {
                    let p = e.data().element_coordinates();
                    if let Some((id, x, y)) = pick_from(p.x, p.y) {
                        session.send(protocol::ClientToHost::PointerMotion { id, x, y });
                    }
                }
            },
            onmouseup: move |e| {
                if let Some(id) = grabbed()
                    && let Some(button) = wire_button(&e)
                {
                    session.send(protocol::ClientToHost::PointerButton {
                        id,
                        button,
                        pressed: false,
                    });
                }
                grabbed.set(None);
                orbiting.set(None);
            },
            onwheel: move |e| {
                e.prevent_default();
                let p = e.data().element_coordinates();
                if let Some((id, _, _)) = pick_from(p.x, p.y) {
                    let (dx, dy) = wheel_delta(&e);
                    session.send(protocol::ClientToHost::PointerAxis { id, dx, dy });
                }
            },
        }
    }
}

fn raise(mut windows: Signal<Windows>, id: protocol::WindowId) {
    let top = windows.read().values().map(|w| w.z).max().unwrap_or(0);
    let already_top = windows.read().get(&id).map(|w| w.z) == Some(top);
    if already_top {
        return;
    }
    if let Some(info) = windows.write().get_mut(&id) {
        info.z = top + 1;
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
    windows: Signal<Windows>,
    popups: Signal<Popups>,
    drag: Signal<Option<DragOp>>,
    cursor: Signal<String>,
) -> Element {
    let session = use_coroutine_handle::<protocol::ClientToHost>();
    let mut focused = focused;
    let mut drag = drag;
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
    let (start_x, start_y) = (info.x, info.y);
    let (start_w, start_h) = (info.width, info.height);
    rsx! {
        div {
            class: "{class}",
            style: "left: {info.x}px; top: {info.y}px; z-index: {info.z};",
            onmousedown: move |e| {
                e.stop_propagation();
                raise(windows, id);
                if *focused.peek() != Some(id) {
                    focused.set(Some(id));
                    session.send(protocol::ClientToHost::Focus { id: Some(id) });
                }
                // The focus gesture doubles as consent to share the browser
                // clipboard with the focused client.
                push_browser_clipboard(session);
            },
            div {
                class: "titlebar",
                onmousedown: move |e| {
                    let p = e.client_coordinates();
                    drag.set(Some(DragOp::Move {
                        id,
                        start_x,
                        start_y,
                        from_x: p.x,
                        from_y: p.y,
                    }));
                },
                span { class: "title", "{label}" }
                button {
                    class: "close",
                    onmousedown: move |e| e.stop_propagation(),
                    onclick: move |_| session.send(protocol::ClientToHost::Close { id }),
                    "x"
                }
            }
            div {
                class: "resize-handle",
                onmousedown: move |e| {
                    e.stop_propagation();
                    let p = e.client_coordinates();
                    drag.set(Some(DragOp::Resize {
                        id,
                        start_w,
                        start_h,
                        from_x: p.x,
                        from_y: p.y,
                    }));
                },
            }
            div {
                class: "canvas-holder",
                canvas {
                    class: "surface",
                    id: "win-{id}",
                    style: "cursor: {cursor};",
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
                for (popup_id, popup) in popups().into_iter().filter(|(_, p)| p.parent == id) {
                    PopupView { key: "{popup_id}", id: popup_id, info: popup, popups, cursor }
                }
            }
        }
    }
}

#[component]
fn PopupView(
    id: protocol::WindowId,
    info: PopupInfo,
    popups: Signal<Popups>,
    cursor: Signal<String>,
) -> Element {
    let session = use_coroutine_handle::<protocol::ClientToHost>();
    rsx! {
        div {
            class: "popup",
            style: "left: {info.x}px; top: {info.y}px;",
            canvas {
                class: "surface",
                id: "win-{id}",
                style: "cursor: {cursor};",
                onmousemove: move |e| {
                    let p = e.data().element_coordinates();
                    session.send(protocol::ClientToHost::PointerMotion { id, x: p.x, y: p.y });
                },
                onmousedown: move |e| {
                    e.stop_propagation();
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
            for (child_id, child) in popups().into_iter().filter(|(_, p)| p.parent == id) {
                PopupView { key: "{child_id}", id: child_id, info: child, popups, cursor }
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
    mut popups: Signal<Popups>,
    mut cursor: Signal<String>,
) {
    loop {
        link.set(Link::Connecting);
        // Input queued while disconnected aims at a dead session; drop it.
        while rx.try_recv().is_ok() {}
        if let Some(ws) = open_socket() {
            run_session(
                ws,
                &mut rx,
                &mut link,
                &mut windows,
                &mut popups,
                &mut cursor,
            )
            .await;
        }
        windows.write().clear();
        popups.write().clear();
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
    popups: &mut Signal<Popups>,
    cursor: &mut Signal<String>,
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
                        if apply(msg, link, windows, popups, cursor).await.is_none() {
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
    popups: &mut Signal<Popups>,
    cursor: &mut Signal<String>,
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
            let top = windows.read().values().map(|w| w.z).max().unwrap_or(0);
            windows.write().insert(
                id,
                WindowInfo {
                    title,
                    app_id,
                    x: 24 + (count % 8) * 48,
                    y: 24 + (count % 8) * 40,
                    z: top + 1,
                    width: 0,
                    height: 0,
                },
            );
        }
        protocol::HostToClient::WindowTitle { id, title } => {
            if let Some(info) = windows.write().get_mut(&id) {
                info.title = title;
            }
        }
        protocol::HostToClient::WindowClosed { id } => {
            if windows.write().remove(&id).is_none() {
                popups.write().remove(&id);
            }
        }
        protocol::HostToClient::PopupCreated { id, parent, x, y } => {
            popups.write().insert(id, PopupInfo { parent, x, y });
        }
        protocol::HostToClient::Frame {
            id,
            size,
            damage,
            compressed,
            pixels,
        } => {
            let wire_len = pixels.len();
            let Some(pixels) = protocol::unwire_pixels(compressed, pixels) else {
                tracing::warn!("dropping an undecompressable frame for window {id}");
                return Some(());
            };
            let stale = windows
                .read()
                .get(&id)
                .is_some_and(|w| w.width != size.width || w.height != size.height);
            if stale && let Some(info) = windows.write().get_mut(&id) {
                info.width = size.width;
                info.height = size.height;
            }
            record_frame_stats(wire_len, pixels.len(), damage);
            draw_frame(id, size, damage, &pixels).await;
        }
        protocol::HostToClient::Clipboard { text } => {
            set_wxr_field("clip", &wasm_bindgen::JsValue::from_str(&text));
            if let Some(clipboard) = web_sys::window().map(|w| w.navigator().clipboard()) {
                // The promise resolves on its own; failure only means the
                // page lacks clipboard permission.
                let _ = clipboard.write_text(&text);
            }
        }
        protocol::HostToClient::Cursor { name } => {
            cursor.set(name);
        }
    }
    Some(())
}

/// Read the browser clipboard and hand it to the host, so the freshly
/// focused client can paste it. Needs a user gesture to be permitted, which
/// is why it rides on the focus click.
fn push_browser_clipboard(session: Coroutine<protocol::ClientToHost>) {
    let Some(clipboard) = web_sys::window().map(|w| w.navigator().clipboard()) else {
        return;
    };
    spawn(async move {
        if let Ok(js) = wasm_bindgen_futures::JsFuture::from(clipboard.read_text()).await
            && let Some(text) = js.as_string()
            && !text.is_empty()
        {
            session.send(protocol::ClientToHost::Clipboard { text });
        }
    });
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

/// The diagnostics object on `window.__wxr`, created on first use. The
/// browser tests read it to assert what the wire carried.
fn wxr_object() -> Option<js_sys::Object> {
    let window = web_sys::window()?;
    let key = wasm_bindgen::JsValue::from_str("__wxr");
    let existing = js_sys::Reflect::get(&window, &key)
        .ok()
        .filter(wasm_bindgen::JsValue::is_object);
    let obj = existing.unwrap_or_else(|| {
        let fresh: wasm_bindgen::JsValue = js_sys::Object::new().into();
        let _ = js_sys::Reflect::set(&window, &key, &fresh);
        fresh
    });
    obj.dyn_into().ok()
}

fn set_wxr_field(name: &str, value: &wasm_bindgen::JsValue) {
    if let Some(obj) = wxr_object() {
        let _ = js_sys::Reflect::set(&obj, &name.into(), value);
    }
}

/// Frame payload counters on `window.__wxr`, so the browser tests can
/// assert that damage frames stay small and compression pays.
fn record_frame_stats(wire: usize, raw: usize, damage: protocol::Rect) {
    let Some(obj) = wxr_object() else {
        return;
    };
    let get = |name: &str| {
        js_sys::Reflect::get(&obj, &name.into())
            .ok()
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0)
    };
    let wire = f64::from(u32::try_from(wire).unwrap_or(u32::MAX));
    let raw = f64::from(u32::try_from(raw).unwrap_or(u32::MAX));
    let _ = js_sys::Reflect::set(&obj, &"frames".into(), &(get("frames") + 1.0).into());
    let _ = js_sys::Reflect::set(&obj, &"bytes".into(), &(get("bytes") + wire).into());
    let _ = js_sys::Reflect::set(&obj, &"raw".into(), &(get("raw") + raw).into());
    let _ = js_sys::Reflect::set(&obj, &"lastW".into(), &f64::from(damage.width).into());
    let _ = js_sys::Reflect::set(&obj, &"lastH".into(), &f64::from(damage.height).into());
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
