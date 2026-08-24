//! The Wayland side: a headless smithay compositor whose only output device
//! is the browser session hub.
//!
//! Runs on its own thread with a calloop event loop; the HTTP/WebSocket side
//! reaches it through the channel handed to [`run`].

use std::sync::{Arc, PoisonError};
use std::time::{Duration, Instant};

use smithay::backend::input::{ButtonState, KeyState};
use smithay::input::keyboard::{FilterResult, Keycode, XkbConfig};
use smithay::input::pointer::{AxisFrame, ButtonEvent, MotionEvent};
use smithay::input::{Seat, SeatHandler, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::channel::{Channel, Event};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::timer::{TimeoutAction, Timer};
use smithay::reexports::calloop::{EventLoop, Interest, LoopHandle, Mode as TriggerMode, PostAction};
use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_callback::WlCallback;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_shm;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle};
use smithay::utils::{SERIAL_COUNTER, Serial, Transform};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{
    BufferAssignment, CompositorClientState, CompositorHandler, CompositorState, Damage,
    SurfaceAttributes, with_states,
};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::reexports::wayland_protocols::xdg::decoration::zv1::server::zxdg_toplevel_decoration_v1::Mode as DecorationMode;
use smithay::reexports::wayland_protocols::xdg::shell::server::xdg_toplevel;
use smithay::wayland::shell::xdg::decoration::{XdgDecorationHandler, XdgDecorationState};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, SurfaceCachedState, ToplevelSurface, XdgShellHandler,
    XdgShellState, XdgToplevelSurfaceData,
};
use smithay::wayland::shm::{BufferData, ShmHandler, ShmState, with_buffer_contents};
use smithay::wayland::socket::ListeningSocketSource;
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_decoration, delegate_xdg_shell,
};
use webxr_compositor_protocol as protocol;

use crate::session::{Hub, HubEvent};

/// The mode advertised on the virtual output until browsers can size it.
const OUTPUT_SIZE: (i32, i32) = (1920, 1080);

/// How often pending wl_callback frame acks are fired (roughly 60 Hz).
const FRAME_TICK: Duration = Duration::from_millis(16);

/// Frame callbacks are withheld while any browser has this much queued, so
/// clients stop rendering frames nobody can drain yet.
const INFLIGHT_BUDGET: u64 = 32 * 1024 * 1024;

/// Damage covering at least this fraction of the surface is sent as a full
/// frame; the bookkeeping is not worth it above this.
const FULL_FRAME_NUMERATOR: u64 = 4;
const FULL_FRAME_DENOMINATOR: u64 = 5;

pub struct State {
    display_handle: DisplayHandle,
    compositor_state: CompositorState,
    shm_state: ShmState,
    xdg_shell_state: XdgShellState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,
    seat: Seat<Self>,
    hub: Arc<Hub>,
    windows: Vec<Window>,
    next_window_id: protocol::WindowId,
    pending_callbacks: Vec<WlCallback>,
    started: Instant,
    /// bytes_sent at the last throughput log line.
    reported_bytes: u64,
}

/// One xdg toplevel and what the browsers know about it.
struct Window {
    id: protocol::WindowId,
    toplevel: ToplevelSurface,
    /// WindowCreated has been broadcast; happens at the first buffer commit.
    announced: bool,
    title: String,
    app_id: String,
    last_frame: Option<(protocol::Size, Vec<u8>)>,
}

/// Per-wayland-client data; smithay keeps surface state here.
#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientData for ClientState {}

pub fn run(hub: Arc<Hub>, events: Channel<HubEvent>) {
    if let Err(error) = serve(hub, events) {
        tracing::error!(%error, "the wayland thread died");
    }
}

fn serve(hub: Arc<Hub>, events: Channel<HubEvent>) -> Result<(), Box<dyn std::error::Error>> {
    let mut event_loop: EventLoop<State> = EventLoop::try_new()?;
    let display: Display<State> = Display::new()?;
    let display_handle = display.handle();

    let compositor_state = CompositorState::new::<State>(&display_handle);
    let shm_state = ShmState::new::<State>(&display_handle, Vec::new());
    let xdg_shell_state = XdgShellState::new::<State>(&display_handle);
    let mut seat_state = SeatState::new();
    let data_device_state = DataDeviceState::new::<State>(&display_handle);
    // Kept alive by the display; the binding exists to advertise xdg-output.
    let _output_manager_state = OutputManagerState::new_with_xdg_output::<State>(&display_handle);
    // The browser draws every frame, so decorations are always server-side.
    let _decoration_state = XdgDecorationState::new::<State>(&display_handle);

    let mut seat = seat_state.new_wl_seat(&display_handle, "seat0");
    seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    seat.add_pointer();

    advertise_output(&display_handle);
    insert_sources(event_loop.handle(), display, events)?;

    let mut state = State {
        display_handle,
        compositor_state,
        shm_state,
        xdg_shell_state,
        seat_state,
        data_device_state,
        seat,
        hub,
        windows: Vec::new(),
        next_window_id: 1,
        pending_callbacks: Vec::new(),
        started: Instant::now(),
        reported_bytes: 0,
    };

    event_loop.run(Some(FRAME_TICK), &mut state, |state| {
        if let Err(error) = state.display_handle.flush_clients() {
            tracing::warn!(%error, "flushing wayland clients failed");
        }
    })?;
    Ok(())
}

fn advertise_output(display_handle: &DisplayHandle) {
    let output = Output::new(
        "webxr-0".into(),
        PhysicalProperties {
            // Roughly a 27 inch panel; nothing physical exists to measure.
            size: (600, 340).into(),
            subpixel: Subpixel::Unknown,
            make: "webxr-compositor".into(),
            model: "browser".into(),
        },
    );
    let _output_global = output.create_global::<State>(display_handle);
    let mode = Mode {
        size: OUTPUT_SIZE.into(),
        refresh: 60_000,
    };
    output.change_current_state(
        Some(mode),
        Some(Transform::Normal),
        Some(Scale::Integer(1)),
        Some((0, 0).into()),
    );
    output.set_preferred(mode);
}

fn insert_sources(
    handle: LoopHandle<'static, State>,
    display: Display<State>,
    events: Channel<HubEvent>,
) -> Result<(), Box<dyn std::error::Error>> {
    let socket = match std::env::var("WEBXR_COMPOSITOR_WAYLAND_DISPLAY") {
        Ok(name) => ListeningSocketSource::with_name(&name)?,
        Err(_) => ListeningSocketSource::new_auto()?,
    };
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    tracing::info!(socket = %socket_name, "wayland socket ready");

    handle.insert_source(socket, |stream, _, state| {
        if let Err(error) = state
            .display_handle
            .insert_client(stream, Arc::new(ClientState::default()))
        {
            tracing::warn!(%error, "could not accept a wayland client");
        }
    })?;

    handle.insert_source(
        Generic::new(display, Interest::READ, TriggerMode::Level),
        |_, display, state| {
            // SAFETY: the display is not aliased; calloop owns it and hands
            // it out only inside this callback.
            let dispatched = unsafe { display.get_mut().dispatch_clients(state) };
            match dispatched {
                Ok(_) => Ok(PostAction::Continue),
                Err(error) => {
                    tracing::error!(%error, "wayland dispatch failed");
                    Err(error)
                }
            }
        },
    )?;

    handle.insert_source(events, |event, _, state| {
        if let Event::Msg(event) = event {
            state.on_hub_event(event);
        }
    })?;

    handle.insert_source(Timer::from_duration(FRAME_TICK), |_, _, state| {
        state.fire_frame_callbacks();
        TimeoutAction::ToDuration(FRAME_TICK)
    })?;

    let throughput_tick = Duration::from_secs(5);
    handle.insert_source(Timer::from_duration(throughput_tick), move |_, _, state| {
        let total = state.hub.bytes_sent();
        let delta = total - state.reported_bytes;
        state.reported_bytes = total;
        if delta > 0 {
            tracing::debug!(kib_per_s = delta / 5 / 1024, "browser payload rate");
        }
        TimeoutAction::ToDuration(throughput_tick)
    })?;

    Ok(())
}

impl State {
    fn on_hub_event(&mut self, event: HubEvent) {
        match event {
            HubEvent::Joined(client) => self.resync(client),
            HubEvent::Left(client) => tracing::debug!(client, "browser left"),
            HubEvent::Input(_client, msg) => self.on_input(msg),
        }
    }

    fn on_input(&mut self, msg: protocol::ClientToHost) {
        match msg {
            protocol::ClientToHost::Hello { .. } => {}
            protocol::ClientToHost::Focus { id } => self.focus_window(id),
            protocol::ClientToHost::Key { code, pressed } => self.key(code, pressed),
            protocol::ClientToHost::PointerMotion { id, x, y } => self.pointer_motion(id, x, y),
            protocol::ClientToHost::PointerButton {
                id: _,
                button,
                pressed,
            } => self.pointer_button(button, pressed),
            protocol::ClientToHost::PointerAxis { id: _, dx, dy } => self.pointer_axis(dx, dy),
            protocol::ClientToHost::Close { id } => self.close_window(id),
            protocol::ClientToHost::Resize { id, size } => self.resize_window(id, size),
        }
    }

    fn surface_of(&self, id: protocol::WindowId) -> Option<WlSurface> {
        self.windows
            .iter()
            .find(|w| w.id == id)
            .map(|w| w.toplevel.wl_surface().clone())
    }

    fn timestamp(&self) -> u32 {
        u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX)
    }

    fn focus_window(&mut self, id: Option<protocol::WindowId>) {
        let surface = id.and_then(|id| self.surface_of(id));
        // Clients render themselves focused or not from the activated state.
        for window in &self.windows {
            let activated = Some(window.id) == id;
            let changed = window.toplevel.with_pending_state(|state| {
                if activated {
                    state.states.set(xdg_toplevel::State::Activated)
                } else {
                    state.states.unset(xdg_toplevel::State::Activated)
                }
            });
            if changed {
                window.toplevel.send_configure();
            }
        }
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        keyboard.set_focus(self, surface, SERIAL_COUNTER.next_serial());
    }

    /// Ask the client to lay itself out at the browser-chosen size, clamped
    /// to the limits the client itself declared.
    fn resize_window(&mut self, id: protocol::WindowId, size: protocol::Size) {
        let Some(window) = self.windows.iter().find(|w| w.id == id) else {
            return;
        };
        let (min, max) = with_states(window.toplevel.wl_surface(), |states| {
            let mut guard = states.cached_state.get::<SurfaceCachedState>();
            let cached = guard.current();
            (cached.min_size, cached.max_size)
        });
        let mut width = i32::try_from(size.width).unwrap_or(i32::MAX).max(1);
        let mut height = i32::try_from(size.height).unwrap_or(i32::MAX).max(1);
        if min.w > 0 {
            width = width.max(min.w);
        }
        if max.w > 0 {
            width = width.min(max.w);
        }
        if min.h > 0 {
            height = height.max(min.h);
        }
        if max.h > 0 {
            height = height.min(max.h);
        }
        window
            .toplevel
            .with_pending_state(|state| state.size = Some((width, height).into()));
        window.toplevel.send_configure();
    }

    fn key(&mut self, code: u32, pressed: bool) {
        let Some(keyboard) = self.seat.get_keyboard() else {
            return;
        };
        // The wire carries evdev codes; xkb keycodes sit 8 above them.
        let keycode = Keycode::new(code.saturating_add(8));
        let state = if pressed {
            KeyState::Pressed
        } else {
            KeyState::Released
        };
        let time = self.timestamp();
        keyboard.input::<(), _>(
            self,
            keycode,
            state,
            SERIAL_COUNTER.next_serial(),
            time,
            |_, _, _| FilterResult::Forward,
        );
    }

    /// The browser sends surface-local coordinates; every surface sits at
    /// the global origin, so they pass through unchanged.
    fn pointer_motion(&mut self, id: protocol::WindowId, x: f64, y: f64) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let focus = self.surface_of(id).map(|s| (s, (0.0, 0.0).into()));
        let time = self.timestamp();
        pointer.motion(
            self,
            focus,
            &MotionEvent {
                location: (x, y).into(),
                serial: SERIAL_COUNTER.next_serial(),
                time,
            },
        );
        pointer.frame(self);
    }

    fn pointer_button(&mut self, button: u32, pressed: bool) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let state = if pressed {
            ButtonState::Pressed
        } else {
            ButtonState::Released
        };
        let time = self.timestamp();
        pointer.button(
            self,
            &ButtonEvent {
                serial: SERIAL_COUNTER.next_serial(),
                time,
                button,
                state,
            },
        );
        pointer.frame(self);
    }

    fn pointer_axis(&mut self, dx: f64, dy: f64) {
        let Some(pointer) = self.seat.get_pointer() else {
            return;
        };
        let mut frame = AxisFrame::new(self.timestamp());
        if dx.abs() > f64::EPSILON {
            frame = frame.value(smithay::backend::input::Axis::Horizontal, dx);
        }
        if dy.abs() > f64::EPSILON {
            frame = frame.value(smithay::backend::input::Axis::Vertical, dy);
        }
        pointer.axis(self, frame);
        pointer.frame(self);
    }

    fn close_window(&mut self, id: protocol::WindowId) {
        if let Some(window) = self.windows.iter().find(|w| w.id == id) {
            window.toplevel.send_close();
        }
    }

    /// Bring a newly joined browser up to date with every mapped window.
    fn resync(&self, client: crate::session::ClientId) {
        for window in self.windows.iter().filter(|w| w.announced) {
            self.hub.send_to(
                client,
                &protocol::HostToClient::WindowCreated {
                    id: window.id,
                    app_id: window.app_id.clone(),
                    title: window.title.clone(),
                },
            );
            if let Some((size, pixels)) = &window.last_frame {
                self.hub.send_to(
                    client,
                    &protocol::HostToClient::Frame {
                        id: window.id,
                        size: *size,
                        damage: full_damage(*size),
                        pixels: pixels.clone(),
                    },
                );
            }
        }
    }

    fn fire_frame_callbacks(&mut self) {
        if self.pending_callbacks.is_empty() {
            return;
        }
        // Backpressure: a browser that cannot drain its queue must not keep
        // receiving fresh frames, so clients wait for their callbacks.
        if self.hub.max_inflight() > INFLIGHT_BUDGET {
            return;
        }
        let elapsed = u32::try_from(self.started.elapsed().as_millis()).unwrap_or(u32::MAX);
        for callback in self.pending_callbacks.drain(..) {
            callback.done(elapsed);
        }
    }

    /// The commit of a surface that is a toplevel: handles the initial
    /// configure dance, then turns attached shm buffers into browser frames.
    fn toplevel_commit(&mut self, index: usize, surface: &WlSurface) {
        if !initial_configure_sent(surface) {
            self.windows[index].toplevel.send_configure();
            return;
        }

        let (assignment, callbacks, damage, scale) = with_states(surface, |states| {
            let mut guard = states.cached_state.get::<SurfaceAttributes>();
            let attributes = guard.current();
            (
                attributes.buffer.take(),
                std::mem::take(&mut attributes.frame_callbacks),
                std::mem::take(&mut attributes.damage),
                attributes.buffer_scale,
            )
        });
        self.pending_callbacks.extend(callbacks);

        self.sync_meta(index, surface);

        match assignment {
            Some(BufferAssignment::NewBuffer(buffer)) => {
                match self.read_frame(index, &buffer, &damage, scale) {
                    Ok((size, rect, pixels)) => self.publish_frame(index, size, rect, pixels),
                    Err(error) => {
                        tracing::warn!(window = self.windows[index].id, error, "unreadable buffer");
                    }
                }
                buffer.release();
            }
            Some(BufferAssignment::Removed) | None => {}
        }
    }

    /// Decide how much of the committed buffer to copy: the damage bounding
    /// box when the previous frame is patchable, the whole buffer otherwise.
    fn read_frame(
        &self,
        index: usize,
        buffer: &WlBuffer,
        damage: &[Damage],
        scale: i32,
    ) -> Result<(protocol::Size, protocol::Rect, Vec<u8>), String> {
        with_buffer_contents(buffer, |ptr, len, data| {
            // SAFETY: with_buffer_contents guarantees ptr..ptr+len maps the
            // pool for the duration of this closure.
            let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
            let size = protocol::Size {
                width: data.width.unsigned_abs(),
                height: data.height.unsigned_abs(),
            };
            let rect = self.wanted_rect(index, size, damage, scale);
            let pixels = convert_rect(bytes, data, rect)?;
            Ok((size, rect, pixels))
        })
        .map_err(|error| format!("buffer access failed: {error:?}"))?
    }

    /// The full surface unless a smaller patch is provably enough: the
    /// stored frame must match the new size, the scale must be 1, and the
    /// client must have reported usable damage that is worth cropping.
    fn wanted_rect(
        &self,
        index: usize,
        size: protocol::Size,
        damage: &[Damage],
        scale: i32,
    ) -> protocol::Rect {
        let full = full_damage(size);
        let patchable = matches!(&self.windows[index].last_frame, Some((s, _)) if *s == size);
        if !patchable || scale != 1 || damage.is_empty() {
            return full;
        }

        let mut bounds: Option<(i32, i32, i32, i32)> = None;
        for entry in damage {
            // With scale 1 and no transform, surface and buffer coordinates
            // are the same space.
            let r = match entry {
                Damage::Surface(r) => (r.loc.x, r.loc.y, r.size.w, r.size.h),
                Damage::Buffer(r) => (r.loc.x, r.loc.y, r.size.w, r.size.h),
            };
            let x0 = r.0.max(0);
            let y0 = r.1.max(0);
            let x1 = r.0.saturating_add(r.2).min(i32::try_from(size.width).unwrap_or(i32::MAX));
            let y1 = r.1.saturating_add(r.3).min(i32::try_from(size.height).unwrap_or(i32::MAX));
            if x1 <= x0 || y1 <= y0 {
                continue;
            }
            bounds = Some(match bounds {
                None => (x0, y0, x1, y1),
                Some((bx0, by0, bx1, by1)) => (bx0.min(x0), by0.min(y0), bx1.max(x1), by1.max(y1)),
            });
        }
        let Some((x0, y0, x1, y1)) = bounds else {
            // Nothing visibly damaged; be conservative rather than clever.
            return full;
        };

        let rect = protocol::Rect {
            x: x0.unsigned_abs(),
            y: y0.unsigned_abs(),
            width: (x1 - x0).unsigned_abs(),
            height: (y1 - y0).unsigned_abs(),
        };
        let rect_area = u64::from(rect.width) * u64::from(rect.height);
        let full_area = u64::from(size.width) * u64::from(size.height);
        if rect_area * FULL_FRAME_DENOMINATOR >= full_area * FULL_FRAME_NUMERATOR {
            full
        } else {
            rect
        }
    }

    /// Keep title and app_id in sync with the xdg toplevel role state.
    fn sync_meta(&mut self, index: usize, surface: &WlSurface) {
        let (title, app_id) = with_states(surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .map(|data| {
                    let attributes = data.lock().unwrap_or_else(PoisonError::into_inner);
                    (
                        attributes.title.clone().unwrap_or_default(),
                        attributes.app_id.clone().unwrap_or_default(),
                    )
                })
                .unwrap_or_default()
        });

        let window = &mut self.windows[index];
        let title_changed = window.title != title;
        window.title = title;
        window.app_id = app_id;
        if window.announced && title_changed {
            self.hub.broadcast(&protocol::HostToClient::WindowTitle {
                id: window.id,
                title: window.title.clone(),
            });
        }
    }

    fn publish_frame(
        &mut self,
        index: usize,
        size: protocol::Size,
        rect: protocol::Rect,
        pixels: Vec<u8>,
    ) {
        let window = &mut self.windows[index];
        if !window.announced {
            window.announced = true;
            self.hub.broadcast(&protocol::HostToClient::WindowCreated {
                id: window.id,
                app_id: window.app_id.clone(),
                title: window.title.clone(),
            });
        }
        self.hub.broadcast(&protocol::HostToClient::Frame {
            id: window.id,
            size,
            damage: rect,
            pixels: pixels.clone(),
        });

        // Keep the stored frame current so a joining browser gets the whole
        // picture, not just the last patch.
        let window = &mut self.windows[index];
        if rect == full_damage(size) {
            window.last_frame = Some((size, pixels));
        } else if let Some((_, stored)) = &mut window.last_frame {
            let stride = size.width as usize * 4;
            let patch_stride = rect.width as usize * 4;
            for row in 0..rect.height as usize {
                let to = (rect.y as usize + row) * stride + rect.x as usize * 4;
                let from = row * patch_stride;
                stored[to..to + patch_stride]
                    .copy_from_slice(&pixels[from..from + patch_stride]);
            }
        }
    }
}

fn full_damage(size: protocol::Size) -> protocol::Rect {
    protocol::Rect {
        x: 0,
        y: 0,
        width: size.width,
        height: size.height,
    }
}

fn initial_configure_sent(surface: &WlSurface) -> bool {
    with_states(surface, |states| {
        states
            .data_map
            .get::<XdgToplevelSurfaceData>()
            .map(|data| {
                data.lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .initial_configure_sent
            })
            .unwrap_or(true)
    })
}

/// Copy one rectangle of an shm buffer out as tightly packed RGBA8888.
fn convert_rect(bytes: &[u8], data: BufferData, rect: protocol::Rect) -> Result<Vec<u8>, String> {
    let width = usize::try_from(data.width).map_err(|_| "negative width".to_owned())?;
    let height = usize::try_from(data.height).map_err(|_| "negative height".to_owned())?;
    let stride = usize::try_from(data.stride).map_err(|_| "negative stride".to_owned())?;
    let offset = usize::try_from(data.offset).map_err(|_| "negative offset".to_owned())?;

    let opaque = match data.format {
        wl_shm::Format::Argb8888 => false,
        wl_shm::Format::Xrgb8888 => true,
        other => return Err(format!("unsupported shm format {other:?}")),
    };
    let end = offset
        .checked_add(stride.checked_mul(height).ok_or("size overflow")?)
        .ok_or("size overflow")?;
    if stride < width * 4 || bytes.len() < end {
        return Err("buffer smaller than its own geometry".to_owned());
    }
    let (rx, ry) = (rect.x as usize, rect.y as usize);
    let (rw, rh) = (rect.width as usize, rect.height as usize);
    if rx + rw > width || ry + rh > height {
        return Err("damage rect outside the buffer".to_owned());
    }

    let mut out = Vec::with_capacity(rw * rh * 4);
    for row in ry..ry + rh {
        let base = offset + row * stride + rx * 4;
        for pixel in bytes[base..base + rw * 4].chunks_exact(4) {
            // wl_shm formats are little-endian words: [B, G, R, A].
            out.extend_from_slice(&[
                pixel[2],
                pixel[1],
                pixel[0],
                if opaque { 0xFF } else { pixel[3] },
            ]);
        }
    }
    Ok(out)
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    // Every client is inserted with a ClientState (see the socket source in
    // insert_sources), so the data is always present and of that type.
    #[allow(clippy::unwrap_used)]
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, surface: &WlSurface) {
        if let Some(index) = self
            .windows
            .iter()
            .position(|w| w.toplevel.wl_surface() == surface)
        {
            self.toplevel_commit(index, surface);
        }
    }
}

impl BufferHandler for State {
    fn buffer_destroyed(&mut self, _buffer: &WlBuffer) {}
}

impl ShmHandler for State {
    fn shm_state(&self) -> &ShmState {
        &self.shm_state
    }
}

impl SeatHandler for State {
    type KeyboardFocus = WlSurface;
    type PointerFocus = WlSurface;
    type TouchFocus = WlSurface;

    fn seat_state(&mut self) -> &mut SeatState<Self> {
        &mut self.seat_state
    }
}

impl OutputHandler for State {}

impl XdgShellHandler for State {
    fn xdg_shell_state(&mut self) -> &mut XdgShellState {
        &mut self.xdg_shell_state
    }

    fn new_toplevel(&mut self, surface: ToplevelSurface) {
        let id = self.next_window_id;
        self.next_window_id += 1;
        tracing::info!(window = id, "new toplevel");
        self.windows.push(Window {
            id,
            toplevel: surface,
            announced: false,
            title: String::new(),
            app_id: String::new(),
            last_frame: None,
        });
    }

    fn new_popup(&mut self, _surface: PopupSurface, _positioner: PositionerState) {}

    fn grab(&mut self, _surface: PopupSurface, _seat: WlSeat, _serial: Serial) {}

    fn reposition_request(
        &mut self,
        _surface: PopupSurface,
        _positioner: PositionerState,
        _token: u32,
    ) {
    }

    fn toplevel_destroyed(&mut self, surface: ToplevelSurface) {
        if let Some(index) = self.windows.iter().position(|w| w.toplevel == surface) {
            let window = self.windows.remove(index);
            tracing::info!(window = window.id, "toplevel destroyed");
            if window.announced {
                self.hub
                    .broadcast(&protocol::HostToClient::WindowClosed { id: window.id });
            }
        }
    }
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl XdgDecorationHandler for State {
    fn new_decoration(&mut self, toplevel: ToplevelSurface) {
        self.force_server_side(&toplevel);
    }

    fn request_mode(&mut self, toplevel: ToplevelSurface, _mode: DecorationMode) {
        self.force_server_side(&toplevel);
    }

    fn unset_mode(&mut self, toplevel: ToplevelSurface) {
        self.force_server_side(&toplevel);
    }
}

impl State {
    /// The browser draws every frame, so client-side decorations are never
    /// wanted no matter what the client prefers.
    fn force_server_side(&self, toplevel: &ToplevelSurface) {
        toplevel.with_pending_state(|state| {
            state.decoration_mode = Some(DecorationMode::ServerSide);
        });
        if initial_configure_sent(toplevel.wl_surface()) {
            toplevel.send_configure();
        }
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_output!(State);
delegate_xdg_shell!(State);
delegate_xdg_decoration!(State);
delegate_data_device!(State);
