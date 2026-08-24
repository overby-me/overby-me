//! The Wayland side: a headless smithay compositor whose only output device
//! is the browser session hub.
//!
//! Runs on its own thread with a calloop event loop; the HTTP/WebSocket side
//! reaches it through the channel handed to [`run`].

use std::sync::Arc;
use std::time::Duration;

use smithay::input::keyboard::XkbConfig;
use smithay::input::{SeatHandler, SeatState};
use smithay::output::{Mode, Output, PhysicalProperties, Scale, Subpixel};
use smithay::reexports::calloop::channel::{Channel, Event};
use smithay::reexports::calloop::generic::Generic;
use smithay::reexports::calloop::{EventLoop, Interest, Mode as TriggerMode, PostAction};
use smithay::reexports::wayland_server::backend::ClientData;
use smithay::reexports::wayland_server::protocol::wl_buffer::WlBuffer;
use smithay::reexports::wayland_server::protocol::wl_seat::WlSeat;
use smithay::reexports::wayland_server::protocol::wl_surface::WlSurface;
use smithay::reexports::wayland_server::{Client, Display, DisplayHandle};
use smithay::utils::{Serial, Transform};
use smithay::wayland::buffer::BufferHandler;
use smithay::wayland::compositor::{CompositorClientState, CompositorHandler, CompositorState};
use smithay::wayland::output::{OutputHandler, OutputManagerState};
use smithay::wayland::selection::SelectionHandler;
use smithay::wayland::selection::data_device::{
    ClientDndGrabHandler, DataDeviceHandler, DataDeviceState, ServerDndGrabHandler,
};
use smithay::wayland::shell::xdg::{
    PopupSurface, PositionerState, ToplevelSurface, XdgShellHandler, XdgShellState,
};
use smithay::wayland::shm::{ShmHandler, ShmState};
use smithay::wayland::socket::ListeningSocketSource;
use smithay::{
    delegate_compositor, delegate_data_device, delegate_output, delegate_seat, delegate_shm,
    delegate_xdg_shell,
};
use webxr_compositor_protocol as protocol;

use crate::session::ClientId as BrowserId;

/// The mode advertised on the virtual output until browsers can size it.
const OUTPUT_SIZE: (i32, i32) = (1920, 1080);

pub struct State {
    display_handle: DisplayHandle,
    compositor_state: CompositorState,
    shm_state: ShmState,
    xdg_shell_state: XdgShellState,
    seat_state: SeatState<Self>,
    data_device_state: DataDeviceState,
}

/// Per-wayland-client data; smithay keeps surface state here.
#[derive(Default)]
struct ClientState {
    compositor_state: CompositorClientState,
}

impl ClientData for ClientState {}

pub fn run(events: Channel<(BrowserId, protocol::ClientToHost)>) {
    if let Err(error) = serve(events) {
        tracing::error!(%error, "the wayland thread died");
    }
}

fn serve(
    events: Channel<(BrowserId, protocol::ClientToHost)>,
) -> Result<(), Box<dyn std::error::Error>> {
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

    let mut seat = seat_state.new_wl_seat(&display_handle, "seat0");
    seat.add_keyboard(XkbConfig::default(), 200, 25)?;
    seat.add_pointer();

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
    let _output_global = output.create_global::<State>(&display_handle);
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

    let socket = match std::env::var("WEBXR_COMPOSITOR_WAYLAND_DISPLAY") {
        Ok(name) => ListeningSocketSource::with_name(&name)?,
        Err(_) => ListeningSocketSource::new_auto()?,
    };
    let socket_name = socket.socket_name().to_string_lossy().into_owned();
    tracing::info!(socket = %socket_name, "wayland socket ready");

    event_loop
        .handle()
        .insert_source(socket, |stream, _, state| {
            if let Err(error) = state
                .display_handle
                .insert_client(stream, Arc::new(ClientState::default()))
            {
                tracing::warn!(%error, "could not accept a wayland client");
            }
        })?;

    event_loop.handle().insert_source(
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

    event_loop
        .handle()
        .insert_source(events, |event, _, _state| {
            // Stands in for input routing until the seat is wired up.
            if let Event::Msg((client, msg)) = event {
                tracing::info!(client, ?msg, "browser event");
            }
        })?;

    event_loop.run(Some(Duration::from_millis(16)), &mut {
        State {
            display_handle,
            compositor_state,
            shm_state,
            xdg_shell_state,
            seat_state,
            data_device_state,
        }
    }, |state| {
        if let Err(error) = state.display_handle.flush_clients() {
            tracing::warn!(%error, "flushing wayland clients failed");
        }
    })?;
    Ok(())
}

impl CompositorHandler for State {
    fn compositor_state(&mut self) -> &mut CompositorState {
        &mut self.compositor_state
    }

    // Every client is inserted with a ClientState (see the socket source in
    // serve), so the data is always present and of that type.
    #[allow(clippy::unwrap_used)]
    fn client_compositor_state<'a>(&self, client: &'a Client) -> &'a CompositorClientState {
        &client.get_data::<ClientState>().unwrap().compositor_state
    }

    fn commit(&mut self, _surface: &WlSurface) {
        // The surface pipeline to the browser starts here, next milestone.
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
        surface.send_configure();
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
}

impl SelectionHandler for State {
    type SelectionUserData = ();
}

impl DataDeviceHandler for State {
    fn data_device_state(&self) -> &DataDeviceState {
        &self.data_device_state
    }
}

impl ClientDndGrabHandler for State {}
impl ServerDndGrabHandler for State {}

delegate_compositor!(State);
delegate_shm!(State);
delegate_seat!(State);
delegate_output!(State);
delegate_xdg_shell!(State);
delegate_data_device!(State);
